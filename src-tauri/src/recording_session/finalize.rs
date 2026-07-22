use std::sync::atomic::Ordering;

use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::{
    app_runtime::{log_event, update_shell_snapshot},
    app_types::{ActiveRecording, RecentRecording, SharedPersistedState},
    recording_library::insert_recent_recording,
};

pub(super) fn finalize_recording_pipeline<R: Runtime>(
    app: AppHandle<R>,
    active: ActiveRecording,
) -> Result<(), String> {
    active.stop_signal.store(true, Ordering::SeqCst);
    let result = active
        .worker
        .join()
        .map_err(|_| "The recording worker thread panicked.".to_string())?;

    match result {
        Ok(capture) => {
            let recent_recording = RecentRecording {
                file_name: capture
                    .output_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("recording.wav")
                    .to_string(),
                file_path: capture.output_path.display().to_string(),
                transcript_path: None,
                transcript_language: None,
                transcripts: Vec::new(),
                translation_path: None,
                anki_note_id: None,
                anki_deck_name: None,
                anki_note_type: None,
                anki_pushes: Vec::new(),
                furigana_applied: false,
                audio_deleted: false,
                duration_ms: capture.duration_ms,
                bytes_written: capture.bytes_written,
                created_at_ms: capture.created_at_ms,
                source: Some("recording".into()),
                source_url: None,
                title: None,
            };

            log_event(
                &app,
                "INFO",
                "recording.saved",
                serde_json::json!({
                    "filePath": recent_recording.file_path,
                    "displayName": capture.display_name,
                    "durationMs": recent_recording.duration_ms,
                    "bytesWritten": recent_recording.bytes_written
                }),
            );

            let settings = {
                let persisted_state = app.state::<SharedPersistedState>();
                let persisted = persisted_state
                    .0
                    .lock()
                    .map_err(|_| "Could not inspect transcription settings.".to_string())?;
                persisted.settings.clone()
            };

            // Save the recording untranscribed and return to idle immediately so the
            // app stays usable — finalize no longer blocks on Whisper. When
            // transcription is enabled we hand the file to the frontend's
            // non-blocking transcription queue via an event; that queue path
            // (recording_library::transcription) renames the mic capture on its
            // first transcript, stores segments, and translates after transcription
            // exactly as a manual transcribe does.
            insert_recent_recording(&app, recent_recording.clone())?;
            update_shell_snapshot(&app, |shell| {
                shell.phase = "idle".into();
                shell.status_text = format!("Saved {}", recent_recording.file_name);
                shell.started_at_ms = None;
                shell.current_recording_name = None;
                shell.last_output_path = Some(recent_recording.file_path.clone());
                shell.last_transcript_path = None;
                shell.transition_count += 1;
            })?;

            if settings.features.transcription {
                let _ = app.emit(
                    "recording-transcribe-request",
                    serde_json::json!({
                        "filePath": recent_recording.file_path,
                        "title": recent_recording.file_name
                    }),
                );
            }
        }
        Err(error) => {
            log_event(
                &app,
                "ERROR",
                "recording.failed",
                serde_json::json!({ "message": error }),
            );
            update_shell_snapshot(&app, |shell| {
                shell.phase = "error".into();
                shell.status_text = error.clone();
                shell.started_at_ms = None;
                shell.current_recording_name = None;
                shell.last_transcript_path = None;
            })?;
            return Err(error);
        }
    }

    Ok(())
}
