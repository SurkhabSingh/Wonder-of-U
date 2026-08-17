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
            // `force` re-runs even recordings that already have this language, so a
            // segments sidecar can be backfilled onto an existing transcript.
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
///
/// The regions are scraped from whisper's stderr, which is an INFO log rather than an API, and
/// the runtime is user-upgradable — so a rename in a future build would take the cue-end clamp
/// with it. Everything degrades to whisper's own ends in that case, which is exactly the old
/// behaviour and therefore invisible. This WARN is the only thing that would say so out loud.
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
///
/// **One list, two readers.** Transcription refuses on the first unready entry; the Setup
/// checklist marks exactly these required. They used to be separate opinions, and they
/// disagreed: the checklist called FFmpeg optional — "Install FFmpeg for optional MP3
/// conversion" — while this function would not proceed without it. A new user finished every
/// required step, pressed Transcribe, and was sent to fetch a 193 MB "optional" component.
///
/// Ordered as the user should fix them: no model is worth fetching before the runtime that
/// reads it. The messages live beside the conditions so the sentence a user sees and the check
/// that produced it cannot drift apart.
pub(crate) fn transcription_requirements(
    settings: &crate::app_types::AppSettings,
    whisper: &crate::app_types::WhisperDetection,
    ffmpeg: &crate::app_types::FfmpegDetection,
) -> Vec<crate::app_types::TranscriptionRequirement> {
    use crate::app_types::TranscriptionRequirement;

    vec![
        // Whisper's readiness already means "the CLI runs AND a model resolves", so this is one
        // entry rather than two — splitting it would restate detection's rule, badly.
        TranscriptionRequirement {
            id: "whisper",
            ready: whisper.status == "ready",
            blocked_message: format!("Whisper is not ready yet: {}", whisper.message),
        },
        TranscriptionRequirement {
            id: "ffmpeg",
            ready: ffmpeg.executable_path.is_some(),
            blocked_message: "FFmpeg is required for transcription. Download it from Settings."
                .to_string(),
        },
        // Only speech mode uses the VAD model; music mode skips VAD entirely, so it is not a
        // requirement at all when the user has chosen Music. Reported as satisfied rather than
        // omitted, so the list keeps one shape whatever the setting.
        TranscriptionRequirement {
            id: "vad",
            ready: settings.whisper.audio_type == "music"
                || whisper_vad_model_path(Path::new(&settings.asset_directory)).exists(),
            blocked_message:
                "The speech-detector (VAD) model has not been downloaded yet. Download it from Settings."
                    .to_string(),
        },
    ]
}

/// Resolve the engine, or say exactly what is missing.
///
/// The engine decodes with ffmpeg, then runs whisper-cli with its built-in Silero VAD, so the
/// managed runtime + ggml model, ffmpeg, and the VAD model must all be present.
///
/// Shared so the library batch and the watch-session generator cannot drift into two different
/// answers to "is Whisper ready" — the batch reports these as an `unavailable` result, the
/// generator as a plain error, but the checks and their wording are one thing.
pub(crate) fn resolve_whisper_engine<R: Runtime>(
    app: &AppHandle<R>,
    settings: &crate::app_types::AppSettings,
) -> Result<WhisperEngine, String> {
    let whisper_detection = refresh_whisper_detection_state(app).map_err(|error| error.to_string())?;
    let ffmpeg_detection = detect_local_ffmpeg(settings);

    // The three checks that used to sit here inline now live in one list, because the Setup
    // checklist has to describe the same three and was getting one of them wrong.
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

    // Detection only reports "ready" when both of these are present, so this is the invariant
    // restated rather than a new check — but restated where it is USED. `unwrap_or_default()`
    // stood here before, and an empty PathBuf is not a missing value to anything downstream:
    // whisper would have been spawned with "" and the user told the CLI produced no stdout.
    // If that invariant ever moves, this says which piece is gone instead of guessing.
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
    // A recording that already carries a transcript is being transcribed into an
    // additional language. Keep the audio file and its name untouched so the
    // recording's identity (and any current selection or pending Anki push that
    // references `file_path`) stays valid, and store this language's transcript
    // beside the audio under a language-tagged name.
    let already_transcribed =
        !recording.transcripts.is_empty() || recording.transcript_path.is_some();

    // Only a fresh mic capture (which has no meaningful title) gets a name derived
    // from its transcript; every imported/downloaded recording keeps its original.
    // Imported/downloaded media carries a meaningful title (a podcast/video name), so
    // for those we only place the transcript beside the untouched audio, exactly like
    // an additional language. Source values: "recording" (mic), "import", "youtube";
    // legacy recordings with no source fall on the preserve side too.
    let is_mic_capture = recording.source.as_deref() == Some("recording");
    let preserve_audio_name = already_transcribed || !is_mic_capture;

    // A transcript is only ever persisted from *inside* the recording's own folder — the
    // transcript viewer sandboxes its reads there, so a path left in the temp dir would read
    // back as "missing". Every branch below therefore ends at a beside-the-audio path or fails
    // the transcription outright; the raw `transcript_path` (a temp file) is never stored.
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
        // First transcript for this recording: derive friendly file names from the
        // transcript text and rename both the audio and transcript to match.
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
                // The friendly rename failed. Do NOT fall back to the temp path — place the
                // transcript beside the (un-renamed) audio so it stays inside the sandbox and is
                // readable; only a failure of that safe placement fails the transcription.
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

    // Parse whisper's per-segment json and drop a clean `{stem}.{lang}.segments.json`
    // sidecar beside the audio so playback can jump per sentence. `recording.file_path`
    // is now the final audio path (renamed for the first transcript, unchanged for
    // additional languages), so its stem is exactly the one to mirror. A missing or
    // unparseable json leaves `segments_path` None and never fails transcription.
    let segments_path =
        match store_segments_sidecar(
            &recording.file_path,
            &json_path,
            &language,
            &final_transcript_path,
            recording.duration_ms,
            envelope,
        ) {
            Ok(sidecars) => sidecars.map(|sidecars| {
                if sidecars.subtitle_path.is_none() {
                    log_event(
                        app,
                        "WARN",
                        "recording.store_subtitle_failed",
                        serde_json::json!({
                            "audioPath": recording.file_path,
                            "message": "The segments were saved but the subtitle file could                                         not be written."
                        }),
                    );
                }
                sidecars.segments_path.display().to_string()
            }),
            Err(error) => {
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

/// Move a freshly generated transcript for an additional language next to the
/// audio file without renaming the audio. Returns the stored transcript path.
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
    // Deterministic per-language name so re-transcribing the same language
    // overwrites its previous transcript instead of leaving orphans behind.
    let target = parent.join(format!("{stem}.{language_tag}.transcript.txt"));
    move_file(transcript_path, &target)?;
    Ok(target)
}

/// Parse whisper's `--output-json` sidecar into the clean segment array and write
/// `{stem}.{lang}.segments.json` beside the audio, mirroring how the transcript
/// sidecar is named/placed. Returns `Ok(Some(path))` on success, `Ok(None)` when
/// the json is absent or carries no parseable segments (a normal, non-fatal case),
/// and `Err` only when the sidecar itself could not be written.
/// What a successful transcription left beside the audio.
#[derive(Debug)]
pub(crate) struct TranscriptSidecars {
    /// `{stem}.{lang}.segments.json`, the per-sentence timings the viewer and miner read.
    pub(crate) segments_path: PathBuf,
    /// `{stem}.{lang}.srt`, for mpv and alass. `None` when only this file could not be
    /// written, which never fails the transcription.
    pub(crate) subtitle_path: Option<PathBuf>,
}

pub(crate) fn store_segments_sidecar(
    audio_file_path: &str,
    json_path: &Path,
    language: &str,
    transcript_path: &Path,
    duration_ms: u64,
    envelope: Option<&SpeechEnvelope>,
) -> Result<Option<TranscriptSidecars>, String> {
    let raw = match parse_whisper_segments(json_path) {
        Some(segments) if !segments.is_empty() => segments,
        _ => return Ok(None),
    };

    // Repair whisper's runaway repetition and out-of-bounds tails before persisting.
    let raw_len = raw.len();
    let segments = clean_segments(raw, duration_ms, CueTiming::TrimToSpeech(envelope));
    if segments.is_empty() {
        return Ok(None);
    }

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
    // Deterministic per-language name so re-transcribing the same language
    // overwrites its previous segments sidecar instead of leaving orphans behind.
    let target = parent.join(format!("{stem}.{language_tag}.segments.json"));
    let serialized =
        serde_json::to_string(&segments).map_err(|error| error.to_string())?;
    fs::write(&target, serialized).map_err(|error| error.to_string())?;

    // If cleaning removed segments (a repetition loop, or a hallucinated tail past the
    // audio end), the whisper `.txt` still holds that junk — rewrite it from the cleaned
    // segments so the displayed transcript, translation input, and whole-recording Anki
    // push all match the sidecar. Left untouched when nothing was removed, so a normal
    // transcript keeps whisper's exact text. Best-effort: a failed rewrite is ignored.
    if segments.len() != raw_len {
        let cleaned_text = segments
            .iter()
            .map(|segment| segment.text.as_str())
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let _ = fs::write(transcript_path, format!("{cleaned_text}\n"));
    }

    // A subtitle file beside the audio, from the same cleaned segments.
    //
    // Written for every transcription because it costs nothing and is the only form the
    // rest of the world reads: mpv can load it with `--sub-file`, and alass can be pointed
    // at it. Deterministically named like the sidecar, so a re-transcribe overwrites rather
    // than accumulating.
    //
    // Best-effort on purpose. Timestamps and mining depend on the sidecar above; a subtitle
    // file that could not be written must not fail a transcription that otherwise succeeded.
    let subtitle_target = parent.join(format!("{stem}.{language_tag}.srt"));
    let subtitle_path = match fs::write(&subtitle_target, segments_to_srt(&segments)) {
        Ok(()) => Some(subtitle_target),
        // Reported to the caller rather than returned as an error: the segments above are
        // what timestamps and mining depend on, and they are already written. But it is not
        // swallowed either, or a missing subtitle file would look like a feature that never
        // ran rather than one that failed.
        Err(_) => None,
    };

    Ok(Some(TranscriptSidecars {
        segments_path: target,
        subtitle_path,
    }))
}

/// Repairs two whisper failure modes on non-vocal / trailing audio before the segments
/// are persisted:
/// 1. **Runaway repetition** — a run of `REPEAT_LIMIT`+ consecutive segments with the
///    identical trimmed text (whisper looping one line over an instrumental) collapses
///    to a single segment spanning the whole run.
/// 2. **Out-of-bounds tails** — a segment starting at/after the audio end is dropped, and
///    an overshooting `end_ms` is clamped to the duration (kills a trailing hallucination
///    such as a "thanks for watching" line placed past the real end).
///
/// `duration_ms == 0` (unknown duration) skips the bounds pass but keeps the dedup pass.
/// True when a segment's text is *only* a notorious Whisper hallucination — the phrases it
/// emits over music/silence that a VAD region flagged as speech (a lone "thanks for
/// watching", a subscribe plug). Matched whole (after trimming trailing punctuation) so a
/// real sentence that merely contains such words is never dropped.
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
///
/// 150 ms rather than the 120 ms the old VAD clamp used: this one moves cue *starts* too, and
/// the cost of being generous is a little dead air while the cost of being tight is a lost
/// syllable.
const CUE_EDGE_PAD_MS: u64 = 150;

/// Pull each cue in to the speech it actually holds, at both edges.
///
/// Replaces a clamp that pulled the END back to the last Silero speech region inside the cue.
/// That was wrong twice over. It trusted the VAD, which on Japanese ends a region before a
/// devoiced sentence ending (です -> "des") and so cut real syllables; and it never touched the
/// START, on the stated assumption that "starts are already exact", which measurement does not
/// support — one recording carried 12.8 s of leading silence, including a five-second cue whose
/// words did not begin until 3.7 s in.
///
/// Measured over that recording, against the audio's own speech energy:
///
/// ```text
///                                 cues   cuts speech   dead air   leading silence
///   VAD clamp (what this replaces) 153        20         74.8 s       12.8 s
///   no clamp at all                153        11         93.3 s       12.8 s
///   this                           153        11         72.1 s        2.6 s
/// ```
///
/// Better than both on every count, and it keeps all 153 cues and all of their text — a
/// tempting alternative, raising Silero's own speech padding to 300 ms, scored well on one
/// sentence but merged the file down to 136 cues and dropped words out of them.
///
/// Interior silence is deliberately left alone. It is the larger share by far, but removing it
/// would mean splitting the cue, and the text cannot be split with it without word timings — a
/// sentence that no longer matches its own audio is worse than a long one.
fn trim_cue_to_speech(
    start_ms: u64,
    end_ms: u64,
    envelope: Option<&SpeechEnvelope>,
    pad_ms: u64,
) -> (u64, u64) {
    // No envelope (unreadable WAV, or audio with no dynamic range to measure): leave whisper's
    // own timings exactly as they are, which is what every caller did before this existed.
    match envelope {
        Some(envelope) => envelope.trim(start_ms, end_ms, pad_ms),
        None => (start_ms, end_ms),
    }
}

/// How much room the VAD clamp leaves after the last speech region. Silero's own
/// `--vad-speech-pad-ms` is 30, tight enough that a region edge lands on the tail of a vowel.
const CUE_TAIL_PAD_MS: u64 = 120;

/// Pull a cue's end back to the last Silero speech region inside it.
///
/// This is the older rule, and it is kept for exactly one caller: generating subtitles for a
/// watch session. It is not better — it cuts devoiced Japanese sentence endings, which is why
/// the recording library no longer uses it. It is here because that path is verified working
/// and the user asked for it frozen, and because the alternatives are both worse for it: the
/// waveform trim moves cue STARTS, which is the one thing that would disturb subtitle sync,
/// and dropping the rule entirely leaves each cue running to the next one, which bloats every
/// card mined from a video.
///
/// Do not "unify" these two by deleting this. The divergence is the decision.
fn clamp_end_to_vad_regions(start_ms: u64, end_ms: u64, speech_regions: &[SpeechRegion]) -> u64 {
    // Every region is scanned rather than stopping at the first past the cue: the log arrives
    // in order today, but nothing here should depend on that.
    let last_speech_end = speech_regions
        .iter()
        .filter(|region| region.start_ms < end_ms && region.end_ms > start_ms)
        .map(|region| region.end_ms)
        .max();

    // No regions at all (music mode, or a runtime that stopped printing them), or a cue
    // covering no speech: leave whisper's own end exactly as it was.
    match last_speech_end {
        Some(speech_end) => end_ms.min(speech_end.saturating_add(CUE_TAIL_PAD_MS)),
        None => end_ms,
    }
}

/// Which rule decides a cue's boundaries.
///
/// An enum rather than an `Option` or a bool so the two callers have to say which they mean,
/// and so the reason they differ has somewhere to live. They are NOT interchangeable: see
/// `clamp_end_to_vad_regions` for why the watch path keeps the older one.
pub(crate) enum CueTiming<'a> {
    /// Pull both edges in to the speech in the waveform. What the recording library uses,
    /// because a mined card is listened to and a few hundred ms of dead air is audible.
    TrimToSpeech(Option<&'a SpeechEnvelope>),
    /// Pull the end back to the last VAD region. What a watch session's generated subtitles
    /// use, frozen deliberately.
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
        // Drop a segment that is only a stock Whisper hallucination phrase (emitted on a
        // non-speech stretch the VAD flagged as speech).
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
        // Trimmed after the bounds pass so a cue already cut to the file's end is measured
        // against the speech inside its real span, and before the repeat collapse so a
        // collapsed run inherits trimmed members rather than whisper's padded ones.
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
            // Keep one segment spanning the whole repeated run.
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

/// Read whisper's json and convert its `transcription[].offsets.{from,to}` (ms)
/// plus text into the clean segment array. Returns `None` for a missing or
/// unparseable file so the caller can degrade to no segments.
pub(crate) fn parse_whisper_segments(json_path: &Path) -> Option<Vec<RecordingSegment>> {
    let raw = fs::read_to_string(json_path).ok()?;
    let parsed: WhisperJson = serde_json::from_str(&raw).ok()?;
    let segments = parsed
        .transcription
        .into_iter()
        .map(|entry| RecordingSegment {
            text: entry.text.trim().to_string(),
            start_ms: entry.offsets.from,
            end_ms: entry.offsets.to,
        })
        .collect();
    Some(segments)
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

/// Move a file, tolerating a pre-existing destination (Windows `rename` fails on
/// an existing target) and cross-device moves (temp dir on a different volume
/// than the output directory) by falling back to copy + delete.
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
///
/// Registered before the recordings loop so a Cancel clicked at any point during the batch —
/// including while a long whisper-cli pass is mid-flight — reaches the flag. Mirrors the
/// yt-dlp import's `CancelListener`: `Drop` unregisters the `once` handler so a batch that
/// finished normally never leaves it (and the `Arc` it pins) registered for the session.
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

    // Claim the one whisper slot before doing anything. This batch used to check nothing at
    // all, so starting the video library's subtitle generation and then pressing Transcribe
    // ran two whisper-cli processes at once — each asking for the full thread count, and both
    // listening to the one cancel event. Reported as `unavailable` rather than an error,
    // because that is what the queue already knows how to show.
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

    // Register the Cancel listener before the loop so a Cancel clicked at any point during
    // the batch — including mid-pass — reaches the flag; `Drop` unregisters it on return.
    let cancel_listener = CancelListener::register(app);
    // The CPU-usage preference does not change during a batch, so resolve the whisper-cli
    // thread count once and reuse it for every recording.
    let thread_count = transcription_thread_count(&settings.whisper.cpu_usage);

    for (index, recording) in recordings.into_iter().enumerate() {
        // A Cancel that arrived (during a previous item's pass, or before this one started)
        // stops the batch here: mark this item cancelled and leave the rest unprocessed.
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
        // Named rather than inlined twice: the region warning must fire on exactly the runs
        // that enabled VAD, and two copies of one condition is how that drifts apart.
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
            // Each sentence as whisper decodes it, so the viewer fills in live instead of
            // staying blank behind a progress bar. The file path rides along because the
            // queue runs recordings back to back and only the viewed one should render.
            //
            // Hallucinations are filtered here for the same reason `clean_segments` drops
            // them from the saved transcript: a stock "thanks for watching" would otherwise
            // flash on screen and then vanish when the run finished. The run-collapse half
            // of that filter needs the whole list, so a runaway repetition can still appear
            // live and be collapsed at the end.
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
                        "transcriptPath": updated_recording.transcript_path
                    }),
                );

                let mut message =
                    "Transcript created. WAV audio was kept for transcription accuracy."
                        .to_string();

                // The audio is renamed when its first transcript lands, so the
                // updated path is the one that still resolves in history.
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
                // A cancellation surfaces as this exact error from the engine: treat it as a
                // clean stop (not a failure), mark the item, and leave the rest unprocessed.
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
    // Counted, because it was not. A cancelled item landed in none of the three counters, so a
    // run the user stopped reported "0 created, 0 skipped, 0 failed" — indistinguishable from a
    // batch that ran and had nothing to do — and, with no failures, a status of "completed".
    // This string is also what the shell writes into `status_text`, which the frontend shows in
    // a toast, so an unnamed cancel became a green success notification.
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
        // A stop the user asked for is its own outcome, not a completion and not a failure.
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
    let transcript = fs::read_to_string(transcript_path).map_err(|error| error.to_string())?;
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
        // Only one transcript file exists for the language.
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
        let stored = store_segments_sidecar(
            &audio_path.display().to_string(),
            &json_path,
            "fr",
            &transcript_path,
            0,
            None,
        )
        .unwrap()
        .expect("a parseable json must yield a sidecar path");

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
        let audio_path = dir.path().join("hola_100.wav");
        fs::write(&audio_path, b"audio").unwrap();

        let transcript_path = dir.path().join("hola_100.fr.transcript.txt");

        // A json path that was never written.
        let missing = dir.path().join("nope.json");
        let result = store_segments_sidecar(
            &audio_path.display().to_string(),
            &missing,
            "fr",
            &transcript_path,
            0,
            None,
        )
        .unwrap();
        assert!(result.is_none(), "a missing json must not produce a sidecar");

        // A json that is not whisper-shaped parses to no segments.
        let garbage = dir.path().join("garbage.json");
        fs::write(&garbage, "not json at all").unwrap();
        let result = store_segments_sidecar(
            &audio_path.display().to_string(),
            &garbage,
            "fr",
            &transcript_path,
            0,
            None,
        )
        .unwrap();
        assert!(result.is_none(), "unparseable json must not produce a sidecar");

        // No sidecar file was left behind for the language.
        assert!(!dir.path().join("hola_100.fr.segments.json").exists());
    }

    /// Builds an envelope from a voiced/silent sketch: each char is one 10 ms frame,
    /// `#` voiced and `.` silent. Keeps these tests about the rule rather than about WAV
    /// parsing, which `transcription.rs` covers separately.
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
    ///
    /// Generating subtitles keeps the old VAD clamp: it is verified working and frozen on
    /// request. This test exists so that "why are there two of these" is answered by a failing
    /// test rather than by someone deleting one of them.
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
    fn store_segments_sidecar_rewrites_transcript_when_it_collapses_a_loop() {
        let dir = tempfile::tempdir().unwrap();
        let audio_path = dir.path().join("song_1.wav");
        fs::write(&audio_path, b"audio").unwrap();
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

        let stored = store_segments_sidecar(
            &audio_path.display().to_string(),
            &json_path,
            "ja",
            &transcript_path,
            60_000,
            None,
        )
        .unwrap()
        .expect("a parseable json must yield a sidecar path");

        let segments: Vec<RecordingSegment> =
            serde_json::from_str(&fs::read_to_string(&stored.segments_path).unwrap()).unwrap();
        assert_eq!(segments.len(), 1, "the six-segment loop collapses to one");
        // The transcript .txt was rewritten from the cleaned segment, dropping the loop.
        assert_eq!(fs::read_to_string(&transcript_path).unwrap().trim(), "ループ");
    }

    #[test]
    fn language_tags_are_filename_safe() {
        assert_eq!(sanitize_language_tag("es"), "es");
        assert_eq!(sanitize_language_tag("zh-hans"), "zh-hans");
        assert_eq!(sanitize_language_tag("a/b\\c"), "a_b_c");
        assert_eq!(sanitize_language_tag(""), "lang");
    }
}
