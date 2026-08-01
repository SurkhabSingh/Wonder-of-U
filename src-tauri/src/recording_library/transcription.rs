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
        RecordingSegment, RecordingTranscript, SharedPersistedState, WHISPER_VAD_MODEL_FILE,
    },
    runtime_assets::{detect_local_ffmpeg, refresh_whisper_detection_state},
    subtitles::segments_to_srt,
    transcription::{
        run_whisper_transcription, transcription_thread_count, SpeechRegion, VadTimeline,
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

/// Say so when word-level timings did not arrive.
///
/// Cue boundaries are placed on word edges from `--dtw` token timestamps, and `--dtw` does not
/// fail — it prints a line to stderr, disables itself, and returns ordinary timestamps. Without
/// this the only symptom would be cues quietly reverting to whisper's own boundaries, which is
/// exactly the silent-miss shape that hid the furigana bug for eleven days.
///
/// Music mode runs no VAD, so an empty timeline is the correct outcome there and says nothing
/// about DTW.
fn log_word_timings<R: Runtime>(app: &AppHandle<R>, timeline: &VadTimeline, music_mode: bool) {
    if music_mode || !timeline.is_empty() {
        return;
    }
    log_event(
        app,
        "WARN",
        "transcription.word_timings_missing",
        serde_json::json!({
            "message": "No VAD timeline was parsed, so token timestamps could not be placed                         and cue boundaries fall back to whisper's own segment bounds."
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
    if whisper_detection.status != "ready" {
        return Err(format!(
            "Whisper is not ready yet: {}",
            whisper_detection.message
        ));
    }

    let ffmpeg_path = detect_local_ffmpeg(settings)
        .executable_path
        .map(PathBuf::from)
        .ok_or_else(|| {
            "FFmpeg is required for transcription. Download it from Settings.".to_string()
        })?;

    let vad_model_path = Path::new(&settings.asset_directory)
        .join("models")
        .join(WHISPER_VAD_MODEL_FILE);
    // Only speech mode uses the VAD model; music mode skips VAD entirely, so don't require the
    // VAD model to be present when the user has chosen Music.
    if settings.whisper.audio_type != "music" && !vad_model_path.exists() {
        return Err(
            "The speech-detector (VAD) model has not been downloaded yet. Download it from Settings."
                .to_string(),
        );
    }

    Ok(WhisperEngine {
        cli_path: PathBuf::from(whisper_detection.executable_path.unwrap_or_default()),
        model_path: PathBuf::from(whisper_detection.model_path.unwrap_or_default()),
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
    vad_timeline: &VadTimeline,
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
            vad_timeline,
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
    vad_timeline: &VadTimeline,
) -> Result<Option<TranscriptSidecars>, String> {
    let parsed = match parse_whisper_output(json_path) {
        Some(segments) if !segments.is_empty() => segments,
        _ => return Ok(None),
    };

    // Word positions first, while the list still matches whisper's own indexing one to one.
    // `t_dtw` is stated against the compressed timeline whisper decoded, so it means nothing
    // until the VAD log puts it back on the recording's.
    let first_word_ms: Vec<Option<u64>> = parsed
        .iter()
        .map(|entry| {
            entry
                .first_token_dtw_ms
                .and_then(|dtw_ms| vad_timeline.to_original_ms(dtw_ms))
        })
        .collect();
    let mut raw: Vec<RecordingSegment> = parsed.into_iter().map(|entry| entry.segment).collect();
    retime_cues_from_tokens(&mut raw, &first_word_ms);

    // Repair whisper's runaway repetition and out-of-bounds tails before persisting. The cue
    // bounds are already final by here, which is what `AsGiven` says.
    let raw_len = raw.len();
    let segments = clean_segments(raw, duration_ms, CueTiming::AsGiven);
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

/// Re-time each cue from the position of its own first word.
///
/// Whisper's cues are butt-joined — `end of N == start of N+1`, exactly, every cue — so one
/// boundary serves as both the end of a sentence and the start of the next. When that shared
/// point lands inside a word, the earlier sentence loses it and the later one inherits it.
/// That is a single defect wearing three faces: a sentence that starts mid-word, one that ends
/// mid-word, and one that opens with the tail of its predecessor.
///
/// It is invisible while there are pauses to put boundaries in, and constant once there are
/// not. Measured on a 550 s recording, the first half holds 94 pauses over 150 ms and the
/// second holds 18; minute 5-6 has none at all. In that second half 68% of cues carry a
/// boundary error over 200 ms, against 45% in the first. The boundaries did not get worse —
/// the silence ran out, so whisper had nowhere to put them but inside speech.
///
/// Which is why nothing that looks for silence can fix this: not the VAD clamp, not a
/// waveform-energy trim, not alass. `--dtw` aligns each token to the audio by dynamic time
/// warping, so it can place a cut between two words with no pause between them. On the case
/// this was built for, cue `345220` began 340 ms before its own first word and DTW showed the
/// 340 ms was `なんだ`, the previous sentence's ending.
///
/// A cue therefore starts where its own first word does, and ends where the NEXT cue's first
/// word does. Cues stay contiguous, but every seam now falls on a word edge, so no cut can
/// land inside a word.
///
/// Not fixed here: a cue whose successor begins seconds later still ends seconds after its own
/// last word. `t_dtw` gives one timestamp per token — a start, not a span — so there is no
/// measurement of where a final word ends. Trailing dead air needs its own answer.
fn retime_cues_from_tokens(
    segments: &mut [RecordingSegment],
    first_word_ms: &[Option<u64>],
) {
    debug_assert_eq!(segments.len(), first_word_ms.len());

    for index in 0..segments.len() {
        // Without a word position for this cue, leave it exactly as whisper reported it. The
        // same is true of a start that would run past the cue's own end, which would mean the
        // mapping and the decode disagree about this segment.
        let Some(start_ms) = first_word_ms[index] else {
            continue;
        };
        if start_ms >= segments[index].end_ms {
            continue;
        }
        segments[index].start_ms = start_ms;
    }

    // Ends come second and read the already-corrected starts, so a cue ends exactly where its
    // successor begins and the pair cannot disagree about the seam between them. The last cue
    // keeps whisper's end: there is no following word to take one from.
    for index in 0..segments.len().saturating_sub(1) {
        let next_start_ms = segments[index + 1].start_ms;
        if next_start_ms > segments[index].start_ms {
            segments[index].end_ms = next_start_ms;
        }
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
    /// Take the cue bounds as given. What the recording library uses, because
    /// `retime_cues_from_tokens` has already placed them on word edges — that needs each cue's
    /// neighbours, so it runs over the whole list before this per-cue pass.
    AsGiven,
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
                CueTiming::AsGiven => {}
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
    Some(
        parse_whisper_output(json_path)?
            .into_iter()
            .map(|entry| entry.segment)
            .collect(),
    )
}

/// One segment plus, when whisper was asked for them, the position of its first word.
pub(crate) struct ParsedSegment {
    pub(crate) segment: RecordingSegment,
    /// The DTW timestamp of the first real token, on whisper's COMPRESSED timeline. `None`
    /// when DTW produced nothing for this segment.
    pub(crate) first_token_dtw_ms: Option<u64>,
}

/// Reads whisper's json, keeping the per-token DTW timestamps when they are there.
pub(crate) fn parse_whisper_output(json_path: &Path) -> Option<Vec<ParsedSegment>> {
    let raw = fs::read_to_string(json_path).ok()?;
    let parsed: WhisperJson = serde_json::from_str(&raw).ok()?;
    Some(
        parsed
            .transcription
            .into_iter()
            .map(|entry| ParsedSegment {
                first_token_dtw_ms: first_token_dtw_ms(&entry),
                segment: RecordingSegment {
                    text: entry.text.trim().to_string(),
                    start_ms: entry.offsets.from,
                    end_ms: entry.offsets.to,
                },
            })
            .collect(),
    )
}

/// whisper reports `t_dtw` in centiseconds; everything here is milliseconds.
const DTW_UNIT_MS: u64 = 10;

/// The first token of `entry` that is real text rather than a control marker.
///
/// `[_BEG_]` and `[_TT_nnn]` carry no audio and sit at both ends of a segment, so taking the
/// literal first token would place a sentence at a marker's timestamp rather than at its own
/// first word.
fn first_token_dtw_ms(entry: &WhisperJsonSegment) -> Option<u64> {
    entry
        .tokens
        .iter()
        .find(|token| !token.text.trim_start().starts_with("[_"))
        .and_then(|token| token.t_dtw)
        .filter(|t_dtw| *t_dtw >= 0)
        .map(|t_dtw| t_dtw as u64 * DTW_UNIT_MS)
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
    /// Absent unless whisper was run with `--output-json-full`, and carrying `t_dtw` only when
    /// it was also run with `--dtw`. A container-level default keeps an older json readable.
    #[serde(default)]
    tokens: Vec<WhisperJsonToken>,
}

#[derive(serde::Deserialize)]
struct WhisperJsonOffsets {
    from: u64,
    to: u64,
}

#[derive(serde::Deserialize)]
struct WhisperJsonToken {
    #[serde(default)]
    text: String,
    /// Where dynamic time warping placed this token against the audio, in centiseconds on the
    /// compressed timeline. Distinct from `offsets`, which carries the decoder's own estimate —
    /// the two disagree by up to a second, and this is the one measured against the waveform.
    #[serde(default)]
    t_dtw: Option<i64>,
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
                dtw_preset: crate::app_types::whisper_model_spec(&settings.whisper.model_choice)
                    .dtw_preset
                    .to_string(),
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
            log_word_timings(app, &result.vad_timeline, music_mode);
            apply_transcription_result_to_recording(
                app,
                &original_file_path,
                recording.clone(),
                result.transcript_path,
                result.json_path,
                &settings.whisper.language,
                &result.vad_timeline,
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
            &VadTimeline::default(),
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
            &VadTimeline::default(),
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
            &VadTimeline::default(),
        )
        .unwrap();
        assert!(result.is_none(), "unparseable json must not produce a sidecar");

        // No sidecar file was left behind for the language.
        assert!(!dir.path().join("hola_100.fr.segments.json").exists());
    }

    fn cue(text: &str, start_ms: u64, end_ms: u64) -> RecordingSegment {
        RecordingSegment {
            text: text.into(),
            start_ms,
            end_ms,
        }
    }

    /// The measured case this was built for. Cue `345220` began 340 ms before its own first
    /// word, and DTW showed the 340 ms held `なんだ` — the previous sentence's ending. One
    /// misplaced seam, and it broke both cues: the earlier one lost its ending, the later one
    /// inherited it.
    #[test]
    fn a_seam_moves_to_the_word_edge_and_fixes_both_cues() {
        let mut segments = vec![
            cue("sou nan da", 344_600, 345_220),
            cue("jaa issho ni iku?", 345_220, 346_200),
        ];

        retime_cues_from_tokens(&mut segments, &[Some(344_620), Some(345_560)]);

        assert_eq!(segments[0].start_ms, 344_620);
        assert_eq!(segments[0].end_ms, 345_560, "keeps its own ending");
        assert_eq!(segments[1].start_ms, 345_560, "no longer opens on the previous word");
    }

    /// Cues stay contiguous: whatever one gives up, its neighbour takes. A gap here would be
    /// audio belonging to no sentence at all.
    #[test]
    fn cues_remain_contiguous_after_retiming() {
        let mut segments = vec![
            cue("one", 0, 1_000),
            cue("two", 1_000, 2_000),
            cue("three", 2_000, 3_000),
        ];

        retime_cues_from_tokens(&mut segments, &[Some(100), Some(1_200), Some(2_050)]);

        assert_eq!(segments[0].end_ms, segments[1].start_ms);
        assert_eq!(segments[1].end_ms, segments[2].start_ms);
        assert_eq!(segments[2].end_ms, 3_000, "the last cue keeps whisper's end");
    }

    /// No DTW for a cue — an older json, or a run where `--dtw` disabled itself — must leave
    /// that cue exactly as whisper reported it rather than guess at a position.
    #[test]
    fn a_cue_without_a_word_position_is_left_alone() {
        let mut segments = vec![cue("one", 500, 1_500), cue("two", 1_500, 2_500)];

        retime_cues_from_tokens(&mut segments, &[None, None]);

        assert_eq!((segments[0].start_ms, segments[0].end_ms), (500, 1_500));
        assert_eq!((segments[1].start_ms, segments[1].end_ms), (1_500, 2_500));
    }

    /// A word position at or past the cue's own end means the mapping and the decode disagree
    /// about this segment. Applying it would invert the cue; whisper's own bounds stand.
    #[test]
    fn an_impossible_word_position_is_refused() {
        let mut segments = vec![cue("one", 500, 1_500), cue("two", 1_500, 2_500)];

        retime_cues_from_tokens(&mut segments, &[Some(9_000), Some(1_600)]);

        assert_eq!(segments[0].start_ms, 500, "refused, so the cue is untouched");
        assert_eq!(segments[1].start_ms, 1_600);
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
        // Ends at 300, so the clamp pulls the cue's end to 420 (300 + the 120 ms pad) while
        // `AsGiven` leaves it at 600. If these ever agree, one rule has become the other.
        let regions = [SpeechRegion {
            start_ms: 0,
            end_ms: 300,
        }];

        let as_given = clean_segments(segment(), 0, CueTiming::AsGiven);
        let clamped = clean_segments(segment(), 0, CueTiming::ClampToVadRegions(&regions));

        assert_ne!(
            (as_given[0].start_ms, as_given[0].end_ms),
            (clamped[0].start_ms, clamped[0].end_ms)
        );
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

        let cleaned = clean_segments(segments, 0, CueTiming::AsGiven);

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

        let cleaned = clean_segments(segments, 0, CueTiming::AsGiven);

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

        let cleaned = clean_segments(segments, 0, CueTiming::AsGiven);

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

        let cleaned = clean_segments(segments, 8000, CueTiming::AsGiven);

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
            &VadTimeline::default(),
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
