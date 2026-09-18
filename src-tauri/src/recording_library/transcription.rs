use std::{
    fs,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
};

use tauri::{AppHandle, Emitter, EventId, Listener, Manager, Runtime};

use crate::{
    app_runtime::{build_app_bootstrap, log_event, update_shell_snapshot},
    app_state::{derive_transcript_language_from_path, sanitize_recording_name},
    app_types::{
        transcript_language_key, RecentRecording, RecordingActionItem, RecordingBatchResult,
        whisper_vad_model_path, RecordingSegment, RecordingTranscript, SharedPersistedState,
    },
    runtime_assets::{detect_local_ffmpeg, refresh_whisper_detection_state},
    subtitles::segments_to_srt,
    transcription::{
        run_whisper_transcription, transcription_thread_count, SpeechEnvelope, SpeechRegion,
        WhisperSlotGuard, WhisperTranscriptionRequest, TRANSCRIPTION_CANCELLED,
    },
};

use super::{actions::auto_translate_after_transcription, update_recent_recording};

static OUTPUT_RENAME_LOCK: Mutex<()> = Mutex::new(());

fn selected_untranscribed_recordings<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
    language: &str,
    force: bool,
) -> Result<Vec<RecentRecording>, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not read the recording history.".to_string())?;
    let recordings = if file_paths.is_empty() {
        persisted
            .recent_recordings
            .iter()
            .filter(|recording| force || !recording.has_transcript_for_language(language))
            .cloned()
            .collect()
    } else {
        file_paths
            .iter()
            .filter_map(|file_path| {
                persisted
                    .recent_recordings
                    .iter()
                    .find(|recording| recording.file_path == *file_path)
                    .cloned()
            })
            .collect()
    };

    Ok(recordings)
}

/// Record what Silero VAD found, and warn when it found nothing it should have.
fn log_speech_regions<R: Runtime>(
    app: &AppHandle<R>,
    speech_regions: &[SpeechRegion],
    music_mode: bool,
) {
    // Music mode runs no VAD at all, so an empty list is the correct outcome there.
    if speech_regions.is_empty() && !music_mode {
        log_event(
            app,
            "WARN",
            "transcription.vad_regions_missing",
            serde_json::json!({
                "message": "VAD was enabled but no speech regions were parsed from whisper's \
                            output; segment ends fall back to whisper's own timings."
            }),
        );
        return;
    }

    let speech_ms: u64 = speech_regions
        .iter()
        .map(|region| region.end_ms.saturating_sub(region.start_ms))
        .sum();
    log_event(
        app,
        "INFO",
        "transcription.vad_regions",
        serde_json::json!({
            "regionCount": speech_regions.len(),
            "speechMs": speech_ms
        }),
    );
}

/// Everything a transcription pass needs resolved from settings and detection.
pub(crate) struct WhisperEngine {
    pub(crate) cli_path: PathBuf,
    pub(crate) model_path: PathBuf,
    pub(crate) vad_model_path: PathBuf,
    pub(crate) ffmpeg_path: PathBuf,
}

/// Everything transcription cannot run without, and whether each is there.
pub(crate) fn transcription_requirements(
    settings: &crate::app_types::AppSettings,
    whisper: &crate::app_types::WhisperDetection,
    ffmpeg: &crate::app_types::FfmpegDetection,
) -> Vec<crate::app_types::TranscriptionRequirement> {
    use crate::app_types::TranscriptionRequirement;
    use crate::asset_downloads::QueuedDownload;

    let mut whisper_downloads = Vec::new();
    if !whisper.cli_ready {
        whisper_downloads.push(QueuedDownload::WhisperRuntime {
            version: crate::app_state::sanitize_runtime_version(&settings.whisper.runtime_version),
        });
    }
    if !whisper.model_ready {
        whisper_downloads.push(QueuedDownload::WhisperModel);
    }

    vec![
        TranscriptionRequirement::new(
            "whisper",
            whisper.status == "ready",
            format!("Whisper is not ready yet: {}", whisper.message),
            whisper_downloads,
        ),
        TranscriptionRequirement::new(
            "ffmpeg",
            ffmpeg.executable_path.is_some(),
            "FFmpeg is required for transcription. Download it from Settings.".to_string(),
            vec![QueuedDownload::Ffmpeg { reinstall: false }],
        ),
        TranscriptionRequirement::new(
            "vad",
            settings.whisper.audio_type == "music"
                || whisper_vad_model_path(Path::new(&settings.asset_directory)).exists(),
            "The speech-detector (VAD) model has not been downloaded yet. Download it from Settings."
                .to_string(),
            if whisper.model_ready {
                vec![QueuedDownload::WhisperVadModel]
            } else {
                Vec::new()
            },
        ),
    ]
}

/// Every download this install still needs before it can transcribe, in the order they run.
pub(crate) fn missing_essential_downloads(
    settings: &crate::app_types::AppSettings,
    whisper: &crate::app_types::WhisperDetection,
    ffmpeg: &crate::app_types::FfmpegDetection,
) -> Vec<crate::asset_downloads::QueuedDownload> {
    transcription_requirements(settings, whisper, ffmpeg)
        .into_iter()
        .flat_map(|requirement| requirement.fixed_by)
        .collect()
}

/// Starts the downloads a fresh install still needs, from one press.
pub(crate) fn download_missing_essentials_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
    let settings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the current app settings.".to_string())?;
        persisted.settings.clone()
    };
    let whisper = refresh_whisper_detection_state(app).map_err(|error| error.to_string())?;
    let ffmpeg = detect_local_ffmpeg(&settings);

    let requests = missing_essential_downloads(&settings, &whisper, &ffmpeg);
    if requests.is_empty() {
        return Ok(());
    }

    let mut first_failure = None;
    let mut started = 0;
    for request in requests {
        match crate::asset_downloads::enqueue_download(app, request) {
            Ok(()) => started += 1,
            Err(error) => {
                first_failure.get_or_insert(error);
            }
        }
    }

    match first_failure {
        Some(error) if started == 0 => Err(error),
        _ => Ok(()),
    }
}

/// Resolve the engine, or say exactly what is missing.
pub(crate) fn resolve_whisper_engine<R: Runtime>(
    app: &AppHandle<R>,
    settings: &crate::app_types::AppSettings,
) -> Result<WhisperEngine, String> {
    let whisper_detection = refresh_whisper_detection_state(app).map_err(|error| error.to_string())?;
    let ffmpeg_detection = detect_local_ffmpeg(settings);

    if let Some(missing) = transcription_requirements(settings, &whisper_detection, &ffmpeg_detection)
        .into_iter()
        .find(|requirement| !requirement.ready)
    {
        return Err(missing.blocked_message);
    }

    let ffmpeg_path = ffmpeg_detection
        .executable_path
        .map(PathBuf::from)
        .ok_or_else(|| {
            "FFmpeg is required for transcription. Download it from Settings.".to_string()
        })?;
    let vad_model_path = whisper_vad_model_path(Path::new(&settings.asset_directory));

    let cli_path = whisper_detection
        .executable_path
        .map(PathBuf::from)
        .ok_or_else(|| {
            "Whisper reported itself ready but no CLI path came back. Re-run detection in Setup."
                .to_string()
        })?;
    let model_path = whisper_detection
        .model_path
        .map(PathBuf::from)
        .ok_or_else(|| {
            "Whisper reported itself ready but no model path came back. Re-run detection in Setup."
                .to_string()
        })?;

    Ok(WhisperEngine {
        cli_path,
        model_path,
        vad_model_path,
        ffmpeg_path,
    })
}

/// Whether the transcript for `language` carries per-sentence timings.
fn transcript_has_segments(transcripts: &[RecordingTranscript], language: &str) -> bool {
    transcripts
        .iter()
        .find(|transcript| transcript.language == language)
        .is_some_and(|transcript| transcript.segments_path.is_some())
}

fn apply_transcription_result_to_recording<R: Runtime>(
    app: &AppHandle<R>,
    original_file_path: &str,
    mut recording: RecentRecording,
    transcript_path: PathBuf,
    json_path: PathBuf,
    requested_language: &str,
    envelope: Option<&SpeechEnvelope>,
) -> Result<RecentRecording, String> {
    let language = transcript_language_key(requested_language);
    let audio_path = PathBuf::from(&recording.file_path);
    let already_transcribed =
        !recording.transcripts.is_empty() || recording.transcript_path.is_some();

    let is_mic_capture = recording.source.as_deref() == Some("recording");
    let preserve_audio_name = already_transcribed || !is_mic_capture;

    // First, because a new recording is named from this text.
    let cleaned = clean_transcript(&json_path, &transcript_path, recording.duration_ms, envelope);

    let final_transcript_path = if preserve_audio_name {
        store_additional_language_transcript(&audio_path, &transcript_path, &language).map_err(
            |error| {
                log_event(
                    app,
                    "ERROR",
                    "recording.store_additional_transcript_failed",
                    serde_json::json!({
                        "audioPath": recording.file_path,
                        "message": error
                    }),
                );
                error
            },
        )?
    } else {
        match rename_recording_outputs_from_transcript(
            &audio_path,
            &transcript_path,
            recording.created_at_ms,
        ) {
            Ok((renamed_audio_path, renamed_transcript)) => {
                recording.file_name = renamed_audio_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("recording.wav")
                    .to_string();
                recording.file_path = renamed_audio_path.display().to_string();
                recording.bytes_written = fs::metadata(&renamed_audio_path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(recording.bytes_written);
                renamed_transcript
            }
            Err(error) => {
                log_event(
                    app,
                    "ERROR",
                    "recording.rename_from_transcript_failed",
                    serde_json::json!({
                        "audioPath": recording.file_path,
                        "message": error
                    }),
                );
                store_additional_language_transcript(&audio_path, &transcript_path, &language)
                    .map_err(|store_error| {
                        log_event(
                            app,
                            "ERROR",
                            "recording.store_additional_transcript_failed",
                            serde_json::json!({
                                "audioPath": recording.file_path,
                                "message": store_error
                            }),
                        );
                        store_error
                    })?
            }
        }
    };

    recording.transcript_path = Some(final_transcript_path.display().to_string());
    recording.transcript_language =
        derive_transcript_language_from_path(&final_transcript_path, requested_language);

    let stored = cleaned
        .map(|segments| store_segments_sidecar(&recording.file_path, &language, &segments));
    let segments_path =
        match stored {
            Ok(Ok(sidecars)) => {
                if sidecars.subtitle_path.is_none() {
                    log_event(
                        app,
                        "WARN",
                        "recording.store_subtitle_failed",
                        serde_json::json!({
                            "audioPath": recording.file_path,
                            "message": "The segments were saved but the subtitle file could not be written."
                        }),
                    );
                }
                Some(sidecars.segments_path.display().to_string())
            }
            Err(reason) => {
                log_event(
                    app,
                    "WARN",
                    "recording.segments_skipped",
                    serde_json::json!({
                        "audioPath": recording.file_path,
                        "language": language,
                        "reason": reason.id(),
                        "message": reason.message()
                    }),
                );
                None
            }
            Ok(Err(error)) => {
                log_event(
                    app,
                    "ERROR",
                    "recording.store_segments_failed",
                    serde_json::json!({
                        "audioPath": recording.file_path,
                        "message": error
                    }),
                );
                None
            }
        };
    let _ = fs::remove_file(&json_path);

    recording
        .transcripts
        .retain(|transcript| transcript.language != language);
    recording.transcripts.push(RecordingTranscript {
        language,
        file_path: final_transcript_path.display().to_string(),
        detected_language: recording.transcript_language.clone(),
        segments_path,
    });

    let updated_recording = recording.clone();
    update_recent_recording(app, original_file_path, |recording| {
        *recording = updated_recording.clone();
    })?;

    Ok(recording)
}

fn store_additional_language_transcript(
    audio_path: &Path,
    transcript_path: &Path,
    language: &str,
) -> Result<PathBuf, String> {
    let _rename_guard = OUTPUT_RENAME_LOCK
        .lock()
        .map_err(|_| "Could not reserve a transcript output name.".to_string())?;
    let parent = audio_path
        .parent()
        .ok_or_else(|| "The saved recording path did not have a parent folder.".to_string())?;
    let stem = audio_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "The saved recording path did not have a file name.".to_string())?;
    let language_tag = sanitize_language_tag(language);
    let target = parent.join(format!("{stem}.{language_tag}.transcript.txt"));
    move_file(transcript_path, &target)?;
    Ok(target)
}

/// What a successful transcription left beside the audio.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct TranscriptSidecars {
    pub(crate) segments_path: PathBuf,
    pub(crate) subtitle_path: Option<PathBuf>,
}

/// Cleans whisper's segments and rewrites its transcript to match, even when nothing is left,
/// so no reader of the text sees what the cleaning dropped.
fn clean_transcript(
    json_path: &Path,
    transcript_path: &Path,
    duration_ms: u64,
    envelope: Option<&SpeechEnvelope>,
) -> Result<Vec<RecordingSegment>, SegmentsSkip> {
    let raw = parse_whisper_segments(json_path)?;
    let raw_len = raw.len();
    let segments = clean_segments(raw, duration_ms, CueTiming::TrimToSpeech(envelope));
    if segments.len() != raw_len {
        let cleaned_text = segments
            .iter()
            .map(|segment| segment.text.as_str())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let _ = fs::write(transcript_path, format!("{cleaned_text}\n"));
    }
    if segments.is_empty() {
        return Err(SegmentsSkip::CleaningRemovedEverything);
    }
    Ok(segments)
}

pub(crate) fn store_segments_sidecar(
    audio_file_path: &str,
    language: &str,
    segments: &[RecordingSegment],
) -> Result<TranscriptSidecars, String> {
    let _rename_guard = OUTPUT_RENAME_LOCK
        .lock()
        .map_err(|_| "Could not reserve a segments output name.".to_string())?;
    let audio_path = Path::new(audio_file_path);
    let parent = audio_path
        .parent()
        .ok_or_else(|| "The saved recording path did not have a parent folder.".to_string())?;
    let stem = audio_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "The saved recording path did not have a file name.".to_string())?;
    let language_tag = sanitize_language_tag(language);
    let target = parent.join(format!("{stem}.{language_tag}.segments.json"));
    let serialized =
        serde_json::to_string(&segments).map_err(|error| error.to_string())?;
    fs::write(&target, serialized).map_err(|error| error.to_string())?;

    // A subtitle file beside the audio, from the same cleaned segments.
    let subtitle_target = parent.join(format!("{stem}.{language_tag}.srt"));
    let subtitle_path = match fs::write(&subtitle_target, segments_to_srt(segments)) {
        Ok(()) => Some(subtitle_target),
        Err(_) => None,
    };

    Ok(TranscriptSidecars {
        segments_path: target,
        subtitle_path,
    })
}

fn is_whisper_hallucination(text: &str) -> bool {
    let normalized = text
        .trim()
        .trim_end_matches(|character| matches!(character, '。' | '.' | '!' | '！' | '\u{3000}' | ' '));
    const PHRASES: [&str; 6] = [
        "ご視聴ありがとうございました",
        "ご視聴ありがとうございます",
        "ご清聴ありがとうございました",
        "チャンネル登録お願いします",
        "Thank you for watching",
        "Thanks for watching",
    ];
    PHRASES
        .iter()
        .any(|phrase| normalized.eq_ignore_ascii_case(phrase))
}

/// How much room to leave either side of the speech, so a trim never shaves the attack off a
/// first syllable or the decay off a last one.
const CUE_EDGE_PAD_MS: u64 = 150;

/// Pull each cue in to the speech it actually holds, at both edges.
fn trim_cue_to_speech(
    start_ms: u64,
    end_ms: u64,
    envelope: Option<&SpeechEnvelope>,
    pad_ms: u64,
) -> (u64, u64) {
    match envelope {
        Some(envelope) => envelope.trim(start_ms, end_ms, pad_ms),
        None => (start_ms, end_ms),
    }
}

const CUE_TAIL_PAD_MS: u64 = 120;

/// Pull a cue's end back to the last Silero speech region inside it.
fn clamp_end_to_vad_regions(start_ms: u64, end_ms: u64, speech_regions: &[SpeechRegion]) -> u64 {
    let last_speech_end = speech_regions
        .iter()
        .filter(|region| region.start_ms < end_ms && region.end_ms > start_ms)
        .map(|region| region.end_ms)
        .max();

    match last_speech_end {
        Some(speech_end) => end_ms.min(speech_end.saturating_add(CUE_TAIL_PAD_MS)),
        None => end_ms,
    }
}

/// Which rule decides a cue's boundaries.
pub(crate) enum CueTiming<'a> {
    TrimToSpeech(Option<&'a SpeechEnvelope>),
    ClampToVadRegions(&'a [SpeechRegion]),
}

pub(crate) fn clean_segments(
    segments: Vec<RecordingSegment>,
    duration_ms: u64,
    timing: CueTiming<'_>,
) -> Vec<RecordingSegment> {
    const REPEAT_LIMIT: usize = 4;

    let bounded: Vec<RecordingSegment> = segments
        .into_iter()
        .filter(|segment| !is_whisper_hallucination(&segment.text))
        .filter_map(|mut segment| {
            if duration_ms > 0 {
                if segment.start_ms >= duration_ms {
                    return None;
                }
                if segment.end_ms > duration_ms {
                    segment.end_ms = duration_ms;
                }
            }
            Some(segment)
        })
        .map(|mut segment| {
            match timing {
                CueTiming::TrimToSpeech(envelope) => {
                    let (start_ms, end_ms) = trim_cue_to_speech(
                        segment.start_ms,
                        segment.end_ms,
                        envelope,
                        CUE_EDGE_PAD_MS,
                    );
                    segment.start_ms = start_ms;
                    segment.end_ms = end_ms;
                }
                CueTiming::ClampToVadRegions(regions) => {
                    segment.end_ms =
                        clamp_end_to_vad_regions(segment.start_ms, segment.end_ms, regions);
                }
            }
            segment
        })
        .collect();

    let mut cleaned: Vec<RecordingSegment> = Vec::with_capacity(bounded.len());
    let mut index = 0;
    while index < bounded.len() {
        let mut run_end = index + 1;
        while run_end < bounded.len() && bounded[run_end].text == bounded[index].text {
            run_end += 1;
        }
        if run_end - index >= REPEAT_LIMIT {
            let mut merged = bounded[index].clone();
            merged.end_ms = bounded[run_end - 1].end_ms;
            cleaned.push(merged);
        } else {
            cleaned.extend_from_slice(&bounded[index..run_end]);
        }
        index = run_end;
    }
    cleaned
}

pub(crate) fn parse_whisper_segments(
    json_path: &Path,
) -> Result<Vec<RecordingSegment>, SegmentsSkip> {
    let raw = crate::text_files::read_external_text(json_path)
        .map_err(|_| SegmentsSkip::JsonUnreadable)?;
    let parsed: WhisperJson =
        serde_json::from_str(&raw).map_err(|_| SegmentsSkip::JsonNotWhisperShaped)?;
    let segments: Vec<RecordingSegment> = parsed
        .transcription
        .into_iter()
        .map(|entry| RecordingSegment {
            text: entry.text.trim().to_string(),
            start_ms: entry.offsets.from,
            end_ms: entry.offsets.to,
        })
        .collect();
    if segments.is_empty() {
        return Err(SegmentsSkip::JsonHeldNoSegments);
    }
    Ok(segments)
}

/// Why no segments sidecar was written.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SegmentsSkip {
    JsonUnreadable,
    JsonNotWhisperShaped,
    JsonHeldNoSegments,
    CleaningRemovedEverything,
}

impl SegmentsSkip {
    pub(crate) fn id(self) -> &'static str {
        match self {
            SegmentsSkip::JsonUnreadable => "json_unreadable",
            SegmentsSkip::JsonNotWhisperShaped => "json_not_whisper_shaped",
            SegmentsSkip::JsonHeldNoSegments => "json_held_no_segments",
            SegmentsSkip::CleaningRemovedEverything => "cleaning_removed_everything",
        }
    }

    pub(crate) fn message(self) -> &'static str {
        match self {
            SegmentsSkip::JsonUnreadable => {
                "Whisper's timing file was missing or could not be opened."
            }
            SegmentsSkip::JsonNotWhisperShaped => {
                "Whisper's timing file was not in the expected format."
            }
            SegmentsSkip::JsonHeldNoSegments => "Whisper's timing file listed no segments.",
            SegmentsSkip::CleaningRemovedEverything => {
                "Every segment was filtered out as a hallucination or past the end of the audio."
            }
        }
    }
}

#[derive(serde::Deserialize)]
struct WhisperJson {
    #[serde(default)]
    transcription: Vec<WhisperJsonSegment>,
}

#[derive(serde::Deserialize)]
struct WhisperJsonSegment {
    #[serde(default)]
    text: String,
    offsets: WhisperJsonOffsets,
}

#[derive(serde::Deserialize)]
struct WhisperJsonOffsets {
    from: u64,
    to: u64,
}

fn sanitize_language_tag(language: &str) -> String {
    let sanitized: String = language
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "lang".into()
    } else {
        sanitized
    }
}

fn move_file(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        fs::remove_file(target).map_err(|error| error.to_string())?;
    }
    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(source, target).map_err(|error| error.to_string())?;
            let _ = fs::remove_file(source);
            Ok(())
        }
    }
}

/// Owns the batch transcription's `transcription-cancel` listener for the whole run.
pub(crate) struct CancelListener<R: Runtime> {
    app: AppHandle<R>,
    event_id: EventId,
    flag: Arc<AtomicBool>,
}

impl<R: Runtime> CancelListener<R> {
    pub(crate) fn register(app: &AppHandle<R>) -> Self {
        let flag = Arc::new(AtomicBool::new(false));
        let flag_for_listener = Arc::clone(&flag);
        let event_id = app.once("transcription-cancel", move |_| {
            flag_for_listener.store(true, Ordering::Relaxed);
        });

        Self {
            app: app.clone(),
            event_id,
            flag,
        }
    }

    fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    pub(crate) fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.flag)
    }
}

impl<R: Runtime> Drop for CancelListener<R> {
    fn drop(&mut self) {
        self.app.unlisten(self.event_id);
    }
}

pub(crate) fn transcribe_recordings_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
    force: bool,
) -> Result<RecordingBatchResult, String> {
    let settings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not inspect transcription settings.".to_string())?;
        persisted.settings.clone()
    };
    let language = transcript_language_key(&settings.whisper.language);

    let _whisper_slot = match WhisperSlotGuard::acquire(
        "Subtitles are being generated for a video. Wait for that to finish, or cancel it first.",
    ) {
        Ok(slot) => slot,
        Err(message) => {
            return Ok(RecordingBatchResult {
                status: "unavailable".into(),
                message,
                items: Vec::new(),
                bootstrap: build_app_bootstrap(app)?,
            });
        }
    };

    // The engine decodes with ffmpeg, then runs whisper-cli with its built-in Silero VAD, so
    // the managed Whisper runtime + ggml model, ffmpeg, and the VAD model must all be present.
    let engine = match resolve_whisper_engine(app, &settings) {
        Ok(engine) => engine,
        Err(message) => {
            return Ok(RecordingBatchResult {
                status: "unavailable".into(),
                message,
                items: Vec::new(),
                bootstrap: build_app_bootstrap(app)?,
            });
        }
    };
    let WhisperEngine {
        cli_path,
        model_path,
        vad_model_path,
        ffmpeg_path,
    } = engine;
    let recordings = selected_untranscribed_recordings(app, file_paths, &language, force)?;
    let total = recordings.len();
    let mut items = Vec::new();

    let cancel_listener = CancelListener::register(app);
    let thread_count = transcription_thread_count(&settings.whisper.cpu_usage);

    for (index, recording) in recordings.into_iter().enumerate() {
        if cancel_listener.is_cancelled() {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "cancelled".into(),
                message: "Transcription cancelled.".into(),
                note_id: recording.anki_note_id,
            });
            break;
        }

        if !force && recording.has_transcript_for_language(&language) {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "skipped".into(),
                message: format!("Already transcribed for {language}."),
                note_id: recording.anki_note_id,
            });
            continue;
        }

        let original_file_path = recording.file_path.clone();
        update_shell_snapshot(app, |shell| {
            shell.phase = "transcribing".into();
            shell.status_text = format!(
                "Transcribing {} of {}: {}",
                index + 1,
                total,
                recording.file_name
            );
            shell.started_at_ms = None;
            shell.current_recording_name = None;
            shell.last_output_path = Some(recording.file_path.clone());
        })?;

        let app_progress = app.clone();
        let app_segment = app.clone();
        let streaming_file_path = original_file_path.clone();
        let music_mode = settings.whisper.audio_type == "music";
        let result = run_whisper_transcription(
            &WhisperTranscriptionRequest {
                cli_path: cli_path.clone(),
                model_path: model_path.clone(),
                vad_model_path: vad_model_path.clone(),
                audio_path: PathBuf::from(&recording.file_path),
                language: settings.whisper.language.clone(),
                ffmpeg_path: ffmpeg_path.clone(),
                thread_count,
                music_mode,
                fast_decode: settings.whisper.decode_speed == "fast",
            },
            cancel_listener.flag(),
            move |percent| {
                let _ = app_progress.emit("transcription-progress", percent);
            },
            move |start_ms, end_ms, text| {
                if is_whisper_hallucination(&text) {
                    return;
                }
                let _ = app_segment.emit(
                    "transcription-segment",
                    serde_json::json!({
                        "filePath": streaming_file_path,
                        "startMs": start_ms,
                        "endMs": end_ms,
                        "text": text,
                    }),
                );
            },
        )
        .and_then(|result| {
            log_speech_regions(app, &result.speech_regions, music_mode);
            apply_transcription_result_to_recording(
                app,
                &original_file_path,
                recording.clone(),
                result.transcript_path,
                result.json_path,
                &settings.whisper.language,
                result.speech_envelope.as_ref(),
            )
        });

        match result {
            Ok(updated_recording) => {
                log_event(
                    app,
                    "INFO",
                    "transcription.saved",
                    serde_json::json!({
                        "audioPath": updated_recording.file_path,
                        "transcriptPath": updated_recording.transcript_path,
                        "hasSegments": transcript_has_segments(
                            &updated_recording.transcripts,
                            &transcript_language_key(&settings.whisper.language),
                        )
                    }),
                );

                let mut message =
                    "Transcript created. WAV audio was kept for transcription accuracy."
                        .to_string();

                if settings.features.translate_after_transcription {
                    if let Some(note) =
                        auto_translate_after_transcription(app, &updated_recording.file_path)
                    {
                        message = format!("{message} {note}");
                    }
                }

                items.push(RecordingActionItem {
                    file_path: updated_recording.file_path,
                    status: "success".into(),
                    message,
                    note_id: updated_recording.anki_note_id,
                });
            }
            Err(error) => {
                if error == TRANSCRIPTION_CANCELLED {
                    items.push(RecordingActionItem {
                        file_path: original_file_path,
                        status: "cancelled".into(),
                        message: "Transcription cancelled.".into(),
                        note_id: None,
                    });
                    break;
                }
                log_event(
                    app,
                    "ERROR",
                    "transcription.failed",
                    serde_json::json!({
                        "audioPath": original_file_path,
                        "message": error
                    }),
                );
                items.push(RecordingActionItem {
                    file_path: original_file_path,
                    status: "failed".into(),
                    message: error,
                    note_id: None,
                });
            }
        }
    }

    let success_count = items.iter().filter(|item| item.status == "success").count();
    let skipped_count = items.iter().filter(|item| item.status == "skipped").count();
    let failed_count = items.iter().filter(|item| item.status == "failed").count();
    let cancelled_count = items.iter().filter(|item| item.status == "cancelled").count();

    let message = if cancelled_count > 0 {
        format!("Transcription cancelled: {success_count} created before stopping.")
    } else {
        format!(
            "Transcription finished: {success_count} created, {skipped_count} skipped, {failed_count} failed."
        )
    };

    update_shell_snapshot(app, |shell| {
        shell.phase = "idle".into();
        shell.status_text = message.clone();
        shell.started_at_ms = None;
        shell.current_recording_name = None;
        shell.transition_count += 1;
    })?;

    Ok(RecordingBatchResult {
        status: if cancelled_count > 0 {
            "cancelled"
        } else if failed_count == 0 {
            "completed"
        } else {
            "partial"
        }
        .into(),
        message,
        items,
        bootstrap: build_app_bootstrap(app)?,
    })
}

fn derive_transcript_stem(transcript_path: &Path) -> Result<String, String> {
    let transcript = crate::text_files::read_external_text(transcript_path).map_err(|error| error.to_string())?;
    let collapsed = transcript.split_whitespace().collect::<Vec<_>>().join(" ");
    let shortened = collapsed.chars().take(10).collect::<String>();
    let sanitized = sanitize_recording_name(&shortened);
    if sanitized.is_empty() {
        return Err("The generated transcript title was empty.".into());
    }

    Ok(sanitized)
}

pub(crate) fn rename_recording_outputs_from_transcript(
    audio_path: &Path,
    transcript_path: &Path,
    recording_id: u64,
) -> Result<(PathBuf, PathBuf), String> {
    let _rename_guard = OUTPUT_RENAME_LOCK
        .lock()
        .map_err(|_| "Could not reserve unique recording output names.".to_string())?;
    let parent = audio_path
        .parent()
        .ok_or_else(|| "The saved recording path did not have a parent folder.".to_string())?;
    let new_stem = derive_transcript_stem(transcript_path)?;
    let timestamped_stem = format!("{new_stem}_{recording_id}");
    let (new_audio_path, new_transcript_path) =
        unique_recording_output_paths(parent, &timestamped_stem);

    fs::rename(audio_path, &new_audio_path).map_err(|error| error.to_string())?;
    if let Err(error) = fs::rename(transcript_path, &new_transcript_path) {
        let rollback_result = fs::rename(&new_audio_path, audio_path);
        return Err(match rollback_result {
            Ok(()) => error.to_string(),
            Err(rollback_error) => {
                format!("{error}. The audio rename also could not be rolled back: {rollback_error}")
            }
        });
    }

    Ok((new_audio_path, new_transcript_path))
}

fn unique_recording_output_paths(directory: &Path, file_stem: &str) -> (PathBuf, PathBuf) {
    let mut attempt = 0usize;
    loop {
        let candidate_stem = if attempt == 0 {
            file_stem.to_string()
        } else {
            format!("{file_stem}_{attempt}")
        };
        let audio_path = directory.join(format!("{candidate_stem}.wav"));
        let transcript_path = directory.join(format!("{candidate_stem}.transcript.txt"));

        if !audio_path.exists() && !transcript_path.exists() {
            return (audio_path, transcript_path);
        }

        attempt += 1;
    }
}

#[cfg(test)]
mod tests {
    /// The discriminating case for `hasSegments`, which cannot be forced through the UI:
    /// it needs one language WITH timings and another WITHOUT on the same recording, and
    /// whether a given language loses its timings depends on what whisper happens to emit.
    #[test]
    fn has_segments_answers_for_the_language_asked_not_for_any_language() {
        use super::transcript_has_segments;
        use crate::app_types::RecordingTranscript;

        let transcripts = vec![
            RecordingTranscript {
                language: "ja".into(),
                file_path: "clip.ja.transcript.txt".into(),
                detected_language: Some("ja".into()),
                segments_path: Some("clip.ja.segments.json".into()),
            },
            RecordingTranscript {
                language: "ko".into(),
                file_path: "clip.ko.transcript.txt".into(),
                detected_language: Some("ko".into()),
                segments_path: None,
            },
        ];

        assert!(
            transcript_has_segments(&transcripts, "ja"),
            "the language that has timings must report true",
        );
        assert!(
            !transcript_has_segments(&transcripts, "ko"),
            "the language WITHOUT timings must report false even though a sibling has them",
        );
        assert!(
            !transcript_has_segments(&transcripts, "fr"),
            "a language with no transcript at all has no timings",
        );

        // The shape this replaced. Kept as an assertion rather than a comment so the
        // difference is executable: `any()` calls the Korean transcript fine.
        let any_says = transcripts
            .iter()
            .any(|transcript| transcript.segments_path.is_some());
        assert!(
            any_says && !transcript_has_segments(&transcripts, "ko"),
            "this test only means something while the two answers disagree",
        );
    }

    /// The bug this file's lossy read exists for, in the shape it actually arrived in.
    #[test]
    fn a_truncated_character_in_whispers_json_still_yields_every_other_segment() {
        use super::{parse_whisper_segments, SegmentsSkip};
        use std::fs;

        let dir = tempfile::tempdir().unwrap();
        let json_path = dir.path().join("whisper-temp.json");

        let mut bytes = Vec::new();
        bytes.extend_from_slice(
            br#"{"transcription":[
                {"offsets":{"from":0,"to":900},"text":"first"},
                {"offsets":{"from":900,"to":1800},"text":""#,
        );
        bytes.extend_from_slice(&[0xEB, 0x86, 0x8D]); // a whole character
        bytes.extend_from_slice(&[0xEB, 0x86]); // and one cut short — the bug
        bytes.extend_from_slice(
            br#""},
                {"offsets":{"from":1800,"to":2700},"text":"third"}
            ]}"#,
        );
        fs::write(&json_path, &bytes).unwrap();

        assert!(
            fs::read_to_string(&json_path).is_err(),
            "the fixture must be invalid UTF-8, or this test proves nothing",
        );

        let segments = parse_whisper_segments(&json_path)
            .expect("a truncated character must not discard the file");
        assert_eq!(segments.len(), 3, "every segment survives");
        assert_eq!(segments[0].text, "first");
        assert_eq!(segments[2].text, "third");
        assert_eq!(segments[2].end_ms, 2700, "timings are intact");
        assert!(
            segments[1].text.contains('\u{FFFD}'),
            "only the truncated character is lost",
        );

        // And the reasons stay distinguishable, which is what makes the log useful.
        let absent = dir.path().join("not-here.json");
        assert_eq!(
            parse_whisper_segments(&absent).unwrap_err(),
            SegmentsSkip::JsonUnreadable,
        );
        let empty = dir.path().join("empty.json");
        fs::write(&empty, br#"{"transcription":[]}"#).unwrap();
        assert_eq!(
            parse_whisper_segments(&empty).unwrap_err(),
            SegmentsSkip::JsonHeldNoSegments,
        );
        let wrong = dir.path().join("wrong.json");
        fs::write(&wrong, b"not json at all").unwrap();
        assert_eq!(
            parse_whisper_segments(&wrong).unwrap_err(),
            SegmentsSkip::JsonNotWhisperShaped,
        );
    }

    use super::*;

    /// Settings whose only interesting field is the audio type and the asset directory.
    fn settings_for(audio_type: &str, asset_directory: &str) -> crate::app_types::AppSettings {
        let mut settings = crate::app_types::AppSettings::default();
        settings.whisper.audio_type = audio_type.into();
        settings.asset_directory = asset_directory.into();
        settings
    }

    fn whisper_detection(status: &str) -> crate::app_types::WhisperDetection {
        crate::app_types::WhisperDetection {
            status: status.into(),
            ..Default::default()
        }
    }

    fn ffmpeg_detection(present: bool) -> crate::app_types::FfmpegDetection {
        crate::app_types::FfmpegDetection {
            executable_path: present.then(|| "C:/ffmpeg.exe".to_string()),
            ..Default::default()
        }
    }

    /// FFmpeg is a requirement, full stop. The Setup checklist called it optional for as long as
    /// it decided for itself, so this is the assertion that matters most here.
    #[test]
    fn ffmpeg_is_a_transcription_requirement() {
        let requirements = transcription_requirements(
            &settings_for("speech", "C:/assets"),
            &whisper_detection("ready"),
            &ffmpeg_detection(false),
        );

        let ffmpeg = requirements
            .iter()
            .find(|requirement| requirement.id == "ffmpeg")
            .expect("ffmpeg is listed");
        assert!(!ffmpeg.ready, "absent ffmpeg must not read as satisfied");
    }

    /// The list is what the checklist marks required, so its ids are a contract with the
    /// frontend — `requirementLookup` in navigation.ts matches on exactly these.
    #[test]
    fn the_requirement_ids_are_the_three_the_checklist_matches_on() {
        let requirements = transcription_requirements(
            &settings_for("speech", "C:/assets"),
            &whisper_detection("ready"),
            &ffmpeg_detection(true),
        );
        let ids: Vec<&str> = requirements.iter().map(|entry| entry.id).collect();

        assert_eq!(ids, ["whisper", "ffmpeg", "vad"]);
    }

    /// Music mode runs no VAD, so the detector is not a requirement then — and the list keeps
    /// the same shape rather than dropping an entry, so a caller never has to ask why.
    #[test]
    fn the_speech_detector_is_only_required_outside_music_mode() {
        let speech = transcription_requirements(
            &settings_for("speech", "C:/assets-that-do-not-exist"),
            &whisper_detection("ready"),
            &ffmpeg_detection(true),
        );
        let music = transcription_requirements(
            &settings_for("music", "C:/assets-that-do-not-exist"),
            &whisper_detection("ready"),
            &ffmpeg_detection(true),
        );

        let vad = |list: &[crate::app_types::TranscriptionRequirement]| {
            list.iter().find(|entry| entry.id == "vad").unwrap().ready
        };
        assert!(!vad(&speech), "speech mode needs the detector");
        assert!(vad(&music), "music mode skips VAD entirely");
        assert_eq!(speech.len(), music.len(), "the list keeps one shape");
    }

    /// Every requirement carries the sentence transcription will show, because the engine
    /// returns the first unready entry's message verbatim. An empty one would surface as a
    /// blank error.
    #[test]
    fn every_requirement_can_explain_itself() {
        for requirement in transcription_requirements(
            &settings_for("speech", "C:/assets"),
            &whisper_detection("cliMissing"),
            &ffmpeg_detection(false),
        ) {
            assert!(
                !requirement.blocked_message.trim().is_empty(),
                "{} has nothing to say when it blocks",
                requirement.id
            );
        }
    }

    /// Order is what the engine reports on, since it returns the FIRST unready entry. Whisper
    /// before ffmpeg means a fresh install is told to install the runtime rather than a codec.
    #[test]
    fn whisper_is_reported_before_ffmpeg_when_both_are_missing() {
        let requirements = transcription_requirements(
            &settings_for("speech", "C:/assets"),
            &whisper_detection("cliMissing"),
            &ffmpeg_detection(false),
        );
        let first_missing = requirements
            .iter()
            .find(|requirement| !requirement.ready)
            .expect("both are missing");

        assert_eq!(first_missing.id, "whisper");
    }

    /// Detection with the two halves of "whisper is ready" set independently, which is what
    /// decides *which* downloads the whisper entry asks for.
    fn whisper_parts(
        status: &str,
        cli_ready: bool,
        model_ready: bool,
    ) -> crate::app_types::WhisperDetection {
        crate::app_types::WhisperDetection {
            status: status.into(),
            cli_ready,
            model_ready,
            ..Default::default()
        }
    }

    fn settings_pinned_to(runtime_version: &str) -> crate::app_types::AppSettings {
        let mut settings = settings_for("speech", "C:/assets-that-do-not-exist");
        settings.whisper.runtime_version = runtime_version.into();
        settings
    }

    /// The constructor's one rule. A requirement that is satisfied carries no fix, whatever the
    /// caller passed — so a consumer cannot start a download of something already installed by
    /// believing `fixed_by` over `ready`.
    #[test]
    fn a_satisfied_requirement_carries_no_download() {
        let ready = crate::app_types::TranscriptionRequirement::new(
            "ffmpeg",
            true,
            "unused".into(),
            vec![crate::asset_downloads::QueuedDownload::Ffmpeg { reinstall: false }],
        );
        let missing = crate::app_types::TranscriptionRequirement::new(
            "ffmpeg",
            false,
            "unused".into(),
            vec![crate::asset_downloads::QueuedDownload::Ffmpeg { reinstall: false }],
        );

        assert!(ready.fixed_by.is_empty(), "a satisfied entry offers no fix");
        assert_eq!(missing.fixed_by.len(), 1);
    }

    /// A fresh install asks for exactly three things, runtime first.
    #[test]
    fn a_fresh_install_asks_for_the_runtime_then_the_model_then_ffmpeg() {
        use crate::asset_downloads::QueuedDownload;

        let requests = missing_essential_downloads(
            &settings_pinned_to("v1.8.4"),
            &whisper_parts("cliMissing", false, false),
            &ffmpeg_detection(false),
        );

        assert_eq!(
            requests,
            vec![
                QueuedDownload::WhisperRuntime {
                    version: "v1.8.4".into()
                },
                QueuedDownload::WhisperModel,
                QueuedDownload::Ffmpeg { reinstall: false },
            ]
        );
    }

    /// The first-run press must never ask for a reinstall.
    #[test]
    fn the_essential_set_never_asks_for_a_reinstall() {
        use crate::asset_downloads::QueuedDownload;

        let requests = missing_essential_downloads(
            &settings_pinned_to("v1.8.4"),
            &whisper_parts("cliMissing", false, false),
            &ffmpeg_detection(false),
        );

        assert!(
            requests.contains(&QueuedDownload::Ffmpeg { reinstall: false }),
            "got {requests:?}"
        );
        assert!(
            !requests
                .iter()
                .any(|request| matches!(request, QueuedDownload::Ffmpeg { reinstall: true })),
            "the missing-asset list must only ever ask for a plain download"
        );
    }

    /// The version a user is pinned to, never the recommended constant.
    #[test]
    fn the_runtime_requested_is_the_one_the_settings_are_pinned_to() {
        use crate::asset_downloads::QueuedDownload;

        let requests = missing_essential_downloads(
            &settings_pinned_to("v1.7.1"),
            &whisper_parts("cliMissing", false, true),
            &ffmpeg_detection(true),
        );

        assert!(
            requests.contains(&QueuedDownload::WhisperRuntime {
                version: "v1.7.1".into()
            }),
            "a pinned runtime must not be replaced by the recommended one, got {requests:?}"
        );
        assert!(
            !requests.contains(&QueuedDownload::WhisperRuntime {
                version: crate::app_config::RECOMMENDED_WHISPER_RUNTIME_VERSION.into()
            }),
            "the recommended version must not be requested over the pinned one"
        );
        assert_ne!(
            crate::app_config::RECOMMENDED_WHISPER_RUNTIME_VERSION, "v1.7.1",
            "this test only proves anything while the pin differs from the recommendation"
        );
    }

    /// Only the missing half of whisper is fetched. The model is the larger of the two, and
    /// re-fetching it because the CLI went missing is the expensive way to be wrong.
    #[test]
    fn only_the_missing_half_of_whisper_is_requested() {
        use crate::asset_downloads::QueuedDownload;

        let model_only = missing_essential_downloads(
            &settings_pinned_to("v1.8.4"),
            &whisper_parts("modelMissing", true, false),
            &ffmpeg_detection(true),
        );

        assert_eq!(model_only, vec![QueuedDownload::WhisperModel]);
    }

    /// The detector on its own — the repair case, reachable by cancelling a model download
    /// between its two files. Requested only once the model is already there, which is exactly
    /// when the model download would no longer fetch it.
    #[test]
    fn the_speech_detector_is_requested_alone_once_the_model_is_installed() {
        use crate::asset_downloads::QueuedDownload;

        let requests = missing_essential_downloads(
            &settings_pinned_to("v1.8.4"),
            &whisper_parts("ready", true, true),
            &ffmpeg_detection(true),
        );

        assert_eq!(requests, vec![QueuedDownload::WhisperVadModel]);
    }

    /// Music mode needs no detector, so one press must not fetch one. This is the case a
    /// frontend-side copy of this mapping got wrong: detection's `vad_ready` has no music-mode
    /// clause, only the requirement list does.
    #[test]
    fn music_mode_asks_for_no_speech_detector() {
        let mut settings = settings_pinned_to("v1.8.4");
        settings.whisper.audio_type = "music".into();

        let requests = missing_essential_downloads(
            &settings,
            &whisper_parts("ready", true, true),
            &ffmpeg_detection(true),
        );

        assert!(
            requests.is_empty(),
            "music mode needs nothing downloaded here, got {requests:?}"
        );
    }

    /// Nothing missing means nothing requested, so the card cannot offer a download that would
    /// do nothing.
    #[test]
    fn an_install_that_needs_nothing_requests_nothing() {
        let directory = tempfile::tempdir().unwrap();
        fs::create_dir_all(directory.path().join("models")).unwrap();
        fs::write(
            crate::app_types::whisper_vad_model_path(directory.path()),
            b"detector",
        )
        .unwrap();
        let mut settings = settings_for("speech", directory.path().to_str().unwrap());
        settings.whisper.runtime_version = "v1.8.4".into();

        let requests = missing_essential_downloads(
            &settings,
            &whisper_parts("ready", true, true),
            &ffmpeg_detection(true),
        );

        assert!(requests.is_empty(), "got {requests:?}");
    }

    #[test]
    fn additional_language_transcript_keeps_audio_and_tags_language() {
        let dir = tempfile::tempdir().unwrap();
        let audio_path = dir.path().join("hola_100.wav");
        fs::write(&audio_path, b"audio").unwrap();

        let temp_transcript = dir.path().join("whisper-temp.txt");
        fs::write(&temp_transcript, "bonjour le monde").unwrap();

        let stored =
            store_additional_language_transcript(&audio_path, &temp_transcript, "fr").unwrap();

        // The audio file is untouched, and the transcript lands beside it tagged
        // with the language.
        assert!(audio_path.exists(), "audio must not be renamed or removed");
        assert_eq!(stored, dir.path().join("hola_100.fr.transcript.txt"));
        assert_eq!(fs::read_to_string(&stored).unwrap(), "bonjour le monde");
        assert!(!temp_transcript.exists(), "temp source should be moved");
    }

    #[test]
    fn retranscribing_same_language_overwrites_without_orphans() {
        let dir = tempfile::tempdir().unwrap();
        let audio_path = dir.path().join("hola_100.wav");
        fs::write(&audio_path, b"audio").unwrap();

        let first = dir.path().join("first-temp.txt");
        fs::write(&first, "old text").unwrap();
        let first_stored =
            store_additional_language_transcript(&audio_path, &first, "es").unwrap();

        let second = dir.path().join("second-temp.txt");
        fs::write(&second, "new text").unwrap();
        let second_stored =
            store_additional_language_transcript(&audio_path, &second, "es").unwrap();

        assert_eq!(first_stored, second_stored);
        assert_eq!(fs::read_to_string(&second_stored).unwrap(), "new text");
        let transcripts = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .map(|name| name.ends_with(".transcript.txt"))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(transcripts, 1);
    }

    #[test]
    fn segments_sidecar_parses_whisper_json_and_lands_beside_audio() {
        let dir = tempfile::tempdir().unwrap();
        let audio_path = dir.path().join("hola_100.wav");
        fs::write(&audio_path, b"audio").unwrap();

        // Whisper `--output-json` shape: transcription entries carry ms offsets.
        let json_path = dir.path().join("whisper-temp.json");
        fs::write(
            &json_path,
            r#"{
                "transcription": [
                    { "offsets": { "from": 0, "to": 2960 }, "text": " Bonjour le monde" },
                    { "offsets": { "from": 2960, "to": 5000 }, "text": " Comment ca va" }
                ]
            }"#,
        )
        .unwrap();

        let transcript_path = dir.path().join("hola_100.fr.transcript.txt");
        let segments = clean_transcript(&json_path, &transcript_path, 0, None)
            .expect("both segments survive cleaning");
        let stored =
            store_segments_sidecar(&audio_path.display().to_string(), "fr", &segments).unwrap();

        assert_eq!(
            stored.segments_path,
            dir.path().join("hola_100.fr.segments.json")
        );
        // The subtitle file rides along with every transcription, from the same segments.
        let subtitle_path = stored
            .subtitle_path
            .clone()
            .expect("a subtitle file is written beside the segments");
        assert_eq!(subtitle_path, dir.path().join("hola_100.fr.srt"));
        let srt = fs::read_to_string(&subtitle_path).unwrap();
        assert!(srt.starts_with("1
00:00:00,000 --> 00:00:02,960
Bonjour le monde
"), "{srt}");

        let stored = stored.segments_path;

        // Round-trip: the written sidecar deserializes back into clean segments
        // with trimmed text and the original ms offsets preserved.
        let raw = fs::read_to_string(&stored).unwrap();
        let segments: Vec<RecordingSegment> = serde_json::from_str(&raw).unwrap();
        assert_eq!(segments.len(), 2);
        assert_eq!(segments[0].text, "Bonjour le monde");
        assert_eq!(segments[0].start_ms, 0);
        assert_eq!(segments[0].end_ms, 2960);
        assert_eq!(segments[1].text, "Comment ca va");
        assert_eq!(segments[1].start_ms, 2960);
        assert_eq!(segments[1].end_ms, 5000);
    }

    #[test]
    fn missing_or_unparseable_json_yields_no_sidecar_without_error() {
        let dir = tempfile::tempdir().unwrap();
        let transcript_path = dir.path().join("hola_100.fr.transcript.txt");

        // A json path that was never written.
        let missing = dir.path().join("nope.json");
        assert_eq!(
            clean_transcript(&missing, &transcript_path, 0, None).expect_err("no segments"),
            SegmentsSkip::JsonUnreadable,
            "a missing json must say why, not just yield nothing",
        );

        // A json that is not whisper-shaped parses to no segments.
        let garbage = dir.path().join("garbage.json");
        fs::write(&garbage, "not json at all").unwrap();
        assert_eq!(
            clean_transcript(&garbage, &transcript_path, 0, None).expect_err("no segments"),
            SegmentsSkip::JsonNotWhisperShaped,
            "unparseable json must say why, not just yield nothing",
        );
    }

    fn envelope(sketch: &str) -> SpeechEnvelope {
        SpeechEnvelope::from_frames(sketch.chars().map(|frame| frame == '#').collect())
    }

    /// The case this replaced the VAD clamp for: a cue whose words start well after it does.
    /// Measured on a real recording, one five-second cue did not begin speaking until 3.69 s
    /// in and nothing trimmed it — the old clamp only ever moved the END, on the stated
    /// assumption that starts were already exact.
    #[test]
    fn a_late_start_is_pulled_in_to_the_speech() {
        let envelope = envelope("..................########........");

        let (start_ms, end_ms) = trim_cue_to_speech(0, 340, Some(&envelope), 50);

        assert_eq!(start_ms, 130, "speech starts at 180ms, less the 50ms pad");
        assert_eq!(end_ms, 310, "speech ends at 260ms, plus the 50ms pad");
    }

    /// A trim may only ever shrink. Were it able to grow, a cue would reach into its
    /// neighbour and two cards would share the same audio.
    #[test]
    fn a_trim_never_grows_a_cue() {
        let envelope = envelope("##########");

        let (start_ms, end_ms) = trim_cue_to_speech(0, 100, Some(&envelope), 5_000);

        assert_eq!((start_ms, end_ms), (0, 100), "the pad cannot exceed the cue");
    }

    /// A cue holding no detected speech keeps whisper's own timings. Collapsing it to nothing
    /// would turn a hallucination into a zero-length clip instead of an obviously wrong one.
    #[test]
    fn a_silent_cue_is_left_alone() {
        let envelope = envelope("....................");

        assert_eq!(trim_cue_to_speech(0, 200, Some(&envelope), 150), (0, 200));
    }

    /// No envelope at all — an unreadable WAV, or audio with no dynamic range to measure —
    /// must leave every timestamp exactly as whisper reported it.
    #[test]
    fn no_envelope_means_no_trimming() {
        assert_eq!(trim_cue_to_speech(1_000, 9_000, None, 150), (1_000, 9_000));
    }

    /// Interior silence stays. The cue spans two bursts and its text covers both, so pulling
    /// in to the first gap would drop words the sentence still claims to say.
    #[test]
    fn silence_inside_a_cue_survives() {
        let envelope = envelope("####..........####");

        let (start_ms, end_ms) = trim_cue_to_speech(0, 180, Some(&envelope), 0);

        assert_eq!((start_ms, end_ms), (0, 180), "both bursts are kept, gap and all");
    }

    /// The watch path's rule, pinned separately because it is deliberately NOT the library's.
    #[test]
    fn the_watch_path_still_clamps_to_vad_regions() {
        let segments = vec![RecordingSegment {
            text: "daijoubu".into(),
            start_ms: 30_060,
            end_ms: 36_100,
        }];
        let regions = [SpeechRegion {
            start_ms: 30_310,
            end_ms: 31_290,
        }];

        let cleaned = clean_segments(segments, 0, CueTiming::ClampToVadRegions(&regions));

        assert_eq!(cleaned[0].start_ms, 30_060, "the clamp never moves a start");
        assert_eq!(cleaned[0].end_ms, 31_410, "31.29s of speech plus the 120ms pad");
    }

    /// The two rules must actually differ. If they ever agree on this input, one of them has
    /// been quietly changed into the other.
    #[test]
    fn the_two_timing_rules_are_not_interchangeable() {
        let segment = || {
            vec![RecordingSegment {
                text: "daijoubu".into(),
                start_ms: 0,
                end_ms: 600,
            }]
        };
        let sketch = envelope("..................########..................................");
        let regions = [SpeechRegion {
            start_ms: 0,
            end_ms: 500,
        }];

        let trimmed = clean_segments(segment(), 0, CueTiming::TrimToSpeech(Some(&sketch)));
        let clamped = clean_segments(segment(), 0, CueTiming::ClampToVadRegions(&regions));

        assert_ne!(
            (trimmed[0].start_ms, trimmed[0].end_ms),
            (clamped[0].start_ms, clamped[0].end_ms)
        );
    }

    /// End to end through the real `CUE_EDGE_PAD_MS`, so the pad the app actually ships is
    /// the one under test. Speech runs 180ms..260ms inside a 600ms cue, leaving dead air at
    /// both ends for the trim to remove.
    #[test]
    fn clean_segments_trims_cues_to_their_speech() {
        let segments = vec![RecordingSegment {
            text: "daijoubu".into(),
            start_ms: 0,
            end_ms: 600,
        }];

        let sketch =
            envelope("..................########..................................");
        let cleaned = clean_segments(segments, 0, CueTiming::TrimToSpeech(Some(&sketch)));

        assert_eq!(cleaned.len(), 1);
        assert_eq!(cleaned[0].start_ms, 30, "speech at 180ms, less the 150ms pad");
        assert_eq!(cleaned[0].end_ms, 410, "speech to 260ms, plus the 150ms pad");
    }

    #[test]
    fn clean_segments_collapses_a_runaway_repetition_run() {
        let seg = |text: &str, from: u64, to: u64| RecordingSegment {
            text: text.into(),
            start_ms: from,
            end_ms: to,
        };
        let mut segments = vec![seg("intro", 0, 1000)];
        // Ten identical segments — whisper looping one line over an instrumental.
        for i in 0..10 {
            segments.push(seg("ループ", 1000 + i * 1000, 2000 + i * 1000));
        }

        let cleaned = clean_segments(segments, 0, CueTiming::TrimToSpeech(None));

        // The loop collapses to a single segment spanning the whole run.
        assert_eq!(cleaned.len(), 2);
        assert_eq!(cleaned[0].text, "intro");
        assert_eq!(cleaned[1].text, "ループ");
        assert_eq!(cleaned[1].start_ms, 1000);
        assert_eq!(cleaned[1].end_ms, 11000);
    }

    #[test]
    fn clean_segments_keeps_a_short_legitimate_repeat() {
        // A genuine 3× repeat (below the limit) must be preserved, not collapsed.
        let seg = |from: u64, to: u64| RecordingSegment {
            text: "リフレイン".into(),
            start_ms: from,
            end_ms: to,
        };
        let segments = vec![seg(0, 1000), seg(1000, 2000), seg(2000, 3000)];

        let cleaned = clean_segments(segments, 0, CueTiming::TrimToSpeech(None));

        assert_eq!(cleaned.len(), 3);
    }

    #[test]
    fn clean_segments_drops_a_lone_hallucination_phrase() {
        let seg = |text: &str, from: u64, to: u64| RecordingSegment {
            text: text.into(),
            start_ms: from,
            end_ms: to,
        };
        let segments = vec![
            seg("本物の台詞です", 0, 3000),
            seg("ご視聴ありがとうございました", 3000, 6000), // stock hallucination -> drop
            seg("ご視聴ありがとうございました。", 6000, 9000), // trailing 。 variant -> drop
            seg("Thanks for watching", 9000, 12000),          // english variant -> drop
            seg("ありがとうございました", 12000, 15000), // generic thanks -> KEEP (may be real)
        ];

        let cleaned = clean_segments(segments, 0, CueTiming::TrimToSpeech(None));

        assert_eq!(cleaned.len(), 2, "only the real line and the generic thanks survive");
        assert_eq!(cleaned[0].text, "本物の台詞です");
        assert_eq!(cleaned[1].text, "ありがとうございました");
    }

    #[test]
    fn clean_segments_drops_and_clamps_out_of_bounds_tails() {
        let seg = |text: &str, from: u64, to: u64| RecordingSegment {
            text: text.into(),
            start_ms: from,
            end_ms: to,
        };
        let segments = vec![
            seg("in bounds", 0, 5000),
            seg("overshoots the end", 5000, 9000), // ends past duration -> clamp
            seg("starts past the end", 9000, 12000), // starts past duration -> drop
        ];

        let cleaned = clean_segments(segments, 8000, CueTiming::TrimToSpeech(None));

        assert_eq!(cleaned.len(), 2);
        assert_eq!(cleaned[1].text, "overshoots the end");
        assert_eq!(cleaned[1].end_ms, 8000, "an overshooting end is clamped");
    }

    #[test]
    fn cleaning_rewrites_the_transcript_when_it_collapses_a_loop() {
        let dir = tempfile::tempdir().unwrap();
        let transcript_path = dir.path().join("song_1.ja.transcript.txt");
        // The raw whisper .txt still holds the looped junk.
        fs::write(&transcript_path, "ループ\nループ\nループ\nループ\nループ\nループ\n").unwrap();

        let entries = (0..6)
            .map(|i| {
                let from = 1000 * i;
                let to = 1000 * (i + 1);
                format!(r#"{{ "offsets": {{ "from": {from}, "to": {to} }}, "text": "ループ" }}"#)
            })
            .collect::<Vec<_>>()
            .join(",");
        let json_path = dir.path().join("whisper-temp.json");
        fs::write(&json_path, format!(r#"{{ "transcription": [ {entries} ] }}"#)).unwrap();

        let segments = clean_transcript(&json_path, &transcript_path, 60_000, None)
            .expect("the loop leaves one segment");
        assert_eq!(segments.len(), 1, "the six-segment loop collapses to one");
        // The transcript .txt was rewritten from the cleaned segment, dropping the loop.
        assert_eq!(fs::read_to_string(&transcript_path).unwrap().trim(), "ループ");
    }

    #[test]
    fn a_transcript_of_nothing_but_a_stock_phrase_is_saved_empty() {
        let dir = tempfile::tempdir().unwrap();
        let transcript_path = dir.path().join("whisper-temp.txt");
        fs::write(&transcript_path, " ご視聴ありがとうございました\n").unwrap();
        let json_path = dir.path().join("whisper-temp.json");
        fs::write(
            &json_path,
            r#"{ "transcription": [
                { "offsets": { "from": 0, "to": 2000 }, "text": " ご視聴ありがとうございました" }
            ] }"#,
        )
        .unwrap();

        assert_eq!(
            clean_transcript(&json_path, &transcript_path, 0, None).expect_err("nothing is left"),
            SegmentsSkip::CleaningRemovedEverything
        );
        assert_eq!(fs::read_to_string(&transcript_path).unwrap().trim(), "");
    }

    #[test]
    fn language_tags_are_filename_safe() {
        assert_eq!(sanitize_language_tag("es"), "es");
        assert_eq!(sanitize_language_tag("zh-hans"), "zh-hans");
        assert_eq!(sanitize_language_tag("a/b\\c"), "a_b_c");
        assert_eq!(sanitize_language_tag(""), "lang");
    }
}
