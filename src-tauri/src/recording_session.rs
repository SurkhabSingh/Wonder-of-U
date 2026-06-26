use std::{
    fs,
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_runtime::{ensure_directory_exists, log_event, now_ms, update_shell_snapshot},
    app_state::{
        derive_transcript_language_from_path, next_recording_stem, normalize_settings,
        unique_wav_path, write_persisted_data,
    },
    app_types::{
        ActiveRecording, AppPathsState, RecentRecording, RecorderState, SharedPersistedState,
        SharedShellState,
    },
    recording::capture_system_audio_loopback,
    recording_library::rename_recording_outputs_from_transcript,
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

pub(crate) fn start_recording_inner<R: Runtime>(
    app: &AppHandle<R>,
    requested_name: Option<String>,
) -> Result<(), String> {
    {
        let shell_state = app.state::<SharedShellState>();
        let shell = shell_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the shell state.".to_string())?;
        if shell.phase != "idle" && shell.phase != "error" {
            return Err("The app is still busy with the previous recording task.".into());
        }
    }

    {
        let recorder_state = app.state::<RecorderState>();
        let recorder = recorder_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the recorder state.".to_string())?;
        if recorder.is_some() {
            return Err("A recording is already in progress.".into());
        }
    }

    let started_at_ms = now_ms();
    let (output_path, display_name, persisted_snapshot) = {
        let paths = app.state::<AppPathsState>().inner().clone();
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not prepare the recording state.".to_string())?;
        persisted.settings = normalize_settings(app, &paths, persisted.settings.clone())
            .map_err(|error| error.to_string())?;

        let output_directory = PathBuf::from(&persisted.settings.output_directory);
        ensure_directory_exists(&output_directory)?;

        let file_stem = next_recording_stem(&mut persisted, requested_name.as_deref());
        let output_path = unique_wav_path(&output_directory, &file_stem);
        let snapshot = persisted.clone();
        (output_path, file_stem, snapshot)
    };

    write_persisted_data(app, &persisted_snapshot)?;
    update_shell_snapshot(app, |shell| {
        shell.phase = "recording".into();
        shell.status_text = format!("Starting system audio capture to {}", output_path.display());
        shell.started_at_ms = Some(started_at_ms);
        shell.current_recording_name = Some(display_name.clone());
        shell.last_output_path = None;
        shell.last_transcript_path = None;
        shell.transition_count += 1;
    })?;

    let stop_signal = Arc::new(AtomicBool::new(false));
    let log_path = app.state::<AppPathsState>().inner().log_file.clone();
    let output_path_for_worker = output_path.clone();
    let display_name_for_worker = display_name.clone();
    let stop_signal_for_worker = stop_signal.clone();
    let worker = std::thread::Builder::new()
        .name("system-audio-recorder".into())
        .spawn(move || {
            capture_system_audio_loopback(
                output_path_for_worker,
                display_name_for_worker,
                stop_signal_for_worker,
                log_path,
                started_at_ms,
            )
        })
        .map_err(|error| {
            let message = error.to_string();
            let _ = update_shell_snapshot(app, |shell| {
                shell.phase = "error".into();
                shell.status_text = message.clone();
                shell.started_at_ms = None;
                shell.current_recording_name = None;
                shell.transition_count += 1;
            });
            message
        })?;

    {
        let recorder_state = app.state::<RecorderState>();
        let mut recorder = recorder_state
            .0
            .lock()
            .map_err(|_| "Could not store the active recorder.".to_string())?;
        *recorder = Some(ActiveRecording {
            stop_signal,
            worker,
        });
    }

    log_event(
        app,
        "INFO",
        "recording.start_requested",
        serde_json::json!({
            "outputPath": output_path.display().to_string(),
            "displayName": display_name
        }),
    );

    update_shell_snapshot(app, |shell| {
        shell.phase = "recording".into();
        shell.status_text = format!("Recording system audio to {}", output_path.display());
        shell.started_at_ms = Some(started_at_ms);
        shell.current_recording_name = Some(display_name.clone());
        shell.last_output_path = None;
        shell.last_transcript_path = None;
        shell.transition_count += 1;
    })?;
    Ok(())
}

fn finalize_recording_pipeline<R: Runtime>(
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
                translation_path: None,
                anki_note_id: None,
                anki_deck_name: None,
                anki_note_type: None,
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

pub(crate) fn stop_recording_inner<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    {
        let shell_state = app.state::<SharedShellState>();
        let shell = shell_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the shell state.".to_string())?;
        if shell.phase == "saving" || shell.phase == "transcribing" {
            return Err("The previous recording is still being finalized.".into());
        }
    }

    let active = {
        let recorder_state = app.state::<RecorderState>();
        let mut recorder = recorder_state
            .0
            .lock()
            .map_err(|_| "Could not access the recorder state.".to_string())?;
        recorder
            .take()
            .ok_or_else(|| "No recording is currently running.".to_string())?
    };

    update_shell_snapshot(app, |shell| {
        shell.phase = "saving".into();
        shell.status_text = "Stopping capture and saving the WAV file...".into();
        shell.started_at_ms = None;
        shell.transition_count += 1;
    })?;

    let app_handle = app.clone();
    std::thread::Builder::new()
        .name("recording-finalizer".into())
        .spawn(move || {
            if let Err(error) = finalize_recording_pipeline(app_handle.clone(), active) {
                log_event(
                    &app_handle,
                    "ERROR",
                    "recording.finalize_failed",
                    serde_json::json!({ "message": error }),
                );
                let _ = update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "error".into();
                    shell.status_text = error;
                    shell.started_at_ms = None;
                    shell.current_recording_name = None;
                    shell.last_transcript_path = None;
                    shell.transition_count += 1;
                });
            }
        })
        .map_err(|error| error.to_string())?;

    Ok(())
}
