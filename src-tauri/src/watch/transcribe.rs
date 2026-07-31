//! Generating subtitles for a watch session from the video's own audio.
//!
//! Deliberately does NOT go through the recording library. `recording_library::import` strips
//! the video track and registers a library entry, so importing a film to subtitle it would
//! leave an audio recording the user never asked for. `decode_to_wav_16k` already passes
//! `-map 0:a:0 -vn`, so an MKV can be handed to the transcription engine directly.
//!
//! The output sits beside the video and is offered to alass exactly like a downloaded
//! subtitle, so the whole chain — transcribe, time, realign — can be exercised end to end.

use std::{fs, path::{Path, PathBuf}};

use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::{
    app_runtime::log_event,
    app_types::{transcript_language_key, SharedPersistedState},
    recording_library::transcription::{
        clean_segments, parse_whisper_segments, resolve_whisper_engine, CancelListener,
        WhisperEngine,
    },
    subtitles::segments_to_srt,
    transcription::{
        run_whisper_transcription, transcription_thread_count, WhisperSlotGuard,
        WhisperTranscriptionRequest,
    },
    watch::subtitles::ffprobe_path_for,
};

#[derive(Debug, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GeneratedSubtitles {
    pub(crate) path: String,
    pub(crate) cue_count: usize,
    pub(crate) language: String,
}

/// `{video stem}.{lang}.whisper.srt`, beside the video.
///
/// Beside the video because that is where mpv, alass and the user all already look — the same
/// choice `jimaku_download` and alass's `.synced.` output make. The `.whisper.` marker says the
/// file was machine-generated, so it is never confused with a downloaded subtitle.
///
/// Deterministic, so regenerating overwrites instead of littering the folder with variants.
pub(crate) fn generated_subtitle_path(video_path: &Path, language: &str) -> PathBuf {
    let stem = video_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("video");
    video_path.with_file_name(format!("{stem}.{language}.whisper.srt"))
}

pub(crate) fn generate_watch_subtitles_inner<R: Runtime>(
    app: &AppHandle<R>,
    video_path: &Path,
) -> Result<GeneratedSubtitles, String> {
    if !video_path.exists() {
        return Err(format!("The video is no longer at {}", video_path.display()));
    }

    // One whisper pass at a time, and this is what actually enforces it. The previous version
    // read `shell.phase`, which this path never writes — so it excluded a recording or a
    // download but not the library's own transcription, the one run it shares a cancel event
    // with. Held for the whole pass and released on drop.
    let _whisper_slot = WhisperSlotGuard::acquire(
        "A transcription is already running. Wait for it to finish, or cancel it first.",
    )?;

    let settings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the current app settings.".to_string())?;
        persisted.settings.clone()
    };
    let language = transcript_language_key(&settings.whisper.language);

    let WhisperEngine {
        cli_path,
        model_path,
        vad_model_path,
        ffmpeg_path,
    } = resolve_whisper_engine(app, &settings)?;

    // A real cancel flag, not a placeholder. This listens to the same global
    // `transcription-cancel` event the library batch uses, which is safe because the slot above
    // genuinely means only one of the two can be running: one event, one owner. (Tauri's `once`
    // is "fires once then unregisters", not "only one listener" — with both registered, one
    // emit would set both flags and kill both runs.) Registered before the pass so a Cancel
    // pressed at any point during it lands, and unregistered on drop.
    let cancel_listener = CancelListener::register(app);
    let app_progress = app.clone();
    let result = run_whisper_transcription(
        &WhisperTranscriptionRequest {
            cli_path,
            model_path,
            vad_model_path,
            audio_path: video_path.to_path_buf(),
            language: settings.whisper.language.clone(),
            ffmpeg_path: ffmpeg_path.clone(),
            thread_count: transcription_thread_count(&settings.whisper.cpu_usage),
            music_mode: settings.whisper.audio_type == "music",
            fast_decode: settings.whisper.decode_speed == "fast",
        },
        cancel_listener.flag(),
        // Reuses the transcription progress channel, so the existing bar reports this too —
        // a 24-minute film is minutes of work and reads as a hang without one.
        move |percent| {
            let _ = app_progress.emit("transcription-progress", percent);
        },
        |_start_ms, _end_ms, _text| {},
    )?;

    // Without a duration the out-of-bounds pass is skipped, and a hallucinated tail past the
    // end of the film would survive into the subtitle file.
    let duration_ms = crate::recording_library::import::probe_duration_ms(
        Some(&ffprobe_path_for(&ffmpeg_path.display().to_string()).display().to_string()),
        video_path,
    );

    let raw = parse_whisper_segments(&result.json_path)
        .ok_or_else(|| "Whisper produced no readable segments for this video.".to_string())?;
    let segments = clean_segments(raw, duration_ms, result.speech_envelope.as_ref());
    if segments.is_empty() {
        return Err("No speech was found in this video.".into());
    }

    let target = generated_subtitle_path(video_path, &language);
    fs::write(&target, segments_to_srt(&segments)).map_err(|error| {
        format!(
            "The subtitle file could not be written to {}: {error}",
            target.display()
        )
    })?;

    // whisper's temp outputs are ours to clean up; nothing else refers to them.
    let _ = fs::remove_file(&result.transcript_path);
    let _ = fs::remove_file(&result.json_path);

    log_event(
        app,
        "INFO",
        "watch.subtitles_generated",
        serde_json::json!({
            "videoPath": video_path.display().to_string(),
            "subtitlePath": target.display().to_string(),
            "cueCount": segments.len(),
            "speechRegions": result.speech_regions.len()
        }),
    );

    Ok(GeneratedSubtitles {
        path: target.display().to_string(),
        cue_count: segments.len(),
        language,
    })
}

#[cfg(test)]
mod tests {
    use super::generated_subtitle_path;
    use std::path::Path;

    #[test]
    fn the_subtitle_sits_beside_the_video_and_says_it_was_generated() {
        assert_eq!(
            generated_subtitle_path(Path::new(r"C:\anime\ep01.mkv"), "ja"),
            Path::new(r"C:\anime\ep01.ja.whisper.srt")
        );
    }

    /// Regenerating overwrites rather than littering the folder, and a second language gets
    /// its own file instead of clobbering the first.
    #[test]
    fn the_name_is_deterministic_per_language() {
        let video = Path::new("/v/show.mp4");
        assert_eq!(
            generated_subtitle_path(video, "ja"),
            generated_subtitle_path(video, "ja")
        );
        assert_ne!(
            generated_subtitle_path(video, "ja"),
            generated_subtitle_path(video, "en")
        );
    }

    /// A video whose name already contains dots must not lose part of it.
    #[test]
    fn a_dotted_video_name_survives() {
        assert_eq!(
            generated_subtitle_path(Path::new("/v/Show.S01E02.1080p.mkv"), "ja"),
            Path::new("/v/Show.S01E02.1080p.ja.whisper.srt")
        );
    }
}
