use std::{fs, path::PathBuf, sync::atomic::Ordering};

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_runtime::{log_event, update_shell_snapshot},
    app_state::{derive_transcript_language_from_path, write_persisted_data},
    app_types::{
        transcript_language_key, ActiveRecording, RecentRecording, RecordingTranscript,
        SharedPersistedState,
    },
    recording_library::{auto_translate_after_transcription, rename_recording_outputs_from_transcript},
    runtime_assets::refresh_whisper_detection_state,
    transcription::{run_whisper_transcription, WhisperTranscriptionRequest},
};

fn insert_recent_recording<R: Runtime>(
    app: &AppHandle<R>,
    recent_recording: RecentRecording,
) -> Result<(), String> {
    let persisted_snapshot = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update the recording history.".to_string())?;
        persisted.recent_recordings.insert(0, recent_recording);
        persisted.clone()
    };

    write_persisted_data(app, &persisted_snapshot)
}

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
            let mut recent_recording = RecentRecording {
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

            if settings.features.transcription {
                let whisper_detection = refresh_whisper_detection_state(&app)?;

                if whisper_detection.status == "ready" {
                    update_shell_snapshot(&app, |shell| {
                        shell.phase = "transcribing".into();
                        shell.status_text = format!(
                            "Saved {}. Running Whisper transcription...",
                            recent_recording.file_name
                        );
                        shell.started_at_ms = None;
                        shell.current_recording_name = None;
                        shell.last_output_path = Some(recent_recording.file_path.clone());
                    })?;

                    match run_whisper_transcription(&WhisperTranscriptionRequest {
                        cli_path: PathBuf::from(
                            whisper_detection
                                .executable_path
                                .clone()
                                .unwrap_or_default(),
                        ),
                        model_path: PathBuf::from(
                            whisper_detection.model_path.clone().unwrap_or_default(),
                        ),
                        audio_path: PathBuf::from(&recent_recording.file_path),
                        language: settings.whisper.language.clone(),
                    }) {
                        Ok(result) => {
                            let mut transcript_path = result.transcript_path;
                            let mut audio_path = PathBuf::from(&recent_recording.file_path);

                            match rename_recording_outputs_from_transcript(
                                &audio_path,
                                &transcript_path,
                                recent_recording.created_at_ms,
                            ) {
                                Ok((renamed_audio_path, renamed_transcript_path)) => {
                                    audio_path = renamed_audio_path;
                                    transcript_path = renamed_transcript_path;
                                }
                                Err(error) => {
                                    log_event(
                                        &app,
                                        "ERROR",
                                        "recording.rename_from_transcript_failed",
                                        serde_json::json!({
                                            "audioPath": recent_recording.file_path,
                                            "message": error
                                        }),
                                    );
                                }
                            }

                            recent_recording.file_name = audio_path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("recording.wav")
                                .to_string();
                            recent_recording.file_path = audio_path.display().to_string();
                            recent_recording.transcript_path =
                                Some(transcript_path.display().to_string());
                            recent_recording.transcript_language =
                                derive_transcript_language_from_path(
                                    &transcript_path,
                                    &settings.whisper.language,
                                );
                            recent_recording.transcripts.push(RecordingTranscript {
                                language: transcript_language_key(&settings.whisper.language),
                                file_path: transcript_path.display().to_string(),
                                detected_language: recent_recording.transcript_language.clone(),
                            });
                            recent_recording.bytes_written = fs::metadata(&audio_path)
                                .map(|metadata| metadata.len())
                                .unwrap_or(recent_recording.bytes_written);

                            insert_recent_recording(&app, recent_recording.clone())?;

                            log_event(
                                &app,
                                "INFO",
                                "transcription.saved",
                                serde_json::json!({
                                    "audioPath": recent_recording.file_path,
                                    "transcriptPath": recent_recording.transcript_path
                                }),
                            );

                            update_shell_snapshot(&app, |shell| {
                                shell.phase = "idle".into();
                                shell.status_text =
                                    format!("Saved {} and transcript.", recent_recording.file_name);
                                shell.started_at_ms = None;
                                shell.current_recording_name = None;
                                shell.last_output_path = Some(recent_recording.file_path.clone());
                                shell.last_transcript_path =
                                    recent_recording.transcript_path.clone();
                                shell.transition_count += 1;
                            })?;

                            // Deliberately after the snapshot returns to "idle".
                            // Translation blocks for as long as the browser takes,
                            // and both start_recording and stop_recording refuse to
                            // run unless the phase is idle — translating first would
                            // lock the user out of recording for the whole wait.
                            if settings.features.translate_after_transcription {
                                if let Some(note) = auto_translate_after_transcription(
                                    &app,
                                    &recent_recording.file_path,
                                ) {
                                    let file_name = recent_recording.file_name.clone();
                                    update_shell_snapshot(&app, |shell| {
                                        shell.status_text =
                                            format!("Saved {file_name} and transcript. {note}");
                                    })?;
                                }
                            }

                            return Ok(());
                        }
                        Err(error) => {
                            log_event(
                                &app,
                                "ERROR",
                                "transcription.failed",
                                serde_json::json!({
                                    "audioPath": recent_recording.file_path,
                                    "message": error
                                }),
                            );
                            insert_recent_recording(&app, recent_recording.clone())?;
                            update_shell_snapshot(&app, |shell| {
                                shell.phase = "idle".into();
                                shell.status_text = format!(
                                    "Saved {}. Whisper transcription failed: {}",
                                    recent_recording.file_name, error
                                );
                                shell.started_at_ms = None;
                                shell.current_recording_name = None;
                                shell.last_output_path = Some(recent_recording.file_path.clone());
                                shell.last_transcript_path = None;
                                shell.transition_count += 1;
                            })?;
                            return Ok(());
                        }
                    }
                }

                insert_recent_recording(&app, recent_recording.clone())?;
                update_shell_snapshot(&app, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = format!(
                        "Saved {}. Whisper is not ready yet: {}",
                        recent_recording.file_name, whisper_detection.message
                    );
                    shell.started_at_ms = None;
                    shell.current_recording_name = None;
                    shell.last_output_path = Some(recent_recording.file_path.clone());
                    shell.last_transcript_path = None;
                    shell.transition_count += 1;
                })?;
                return Ok(());
            }

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
