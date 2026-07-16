use std::path::PathBuf;

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_runtime::{log_event, update_shell_snapshot},
    app_state::write_persisted_data,
    app_types::{whisper_model_spec, SharedPersistedState, SharedShellState},
    runtime_assets::refresh_whisper_detection_state,
    transcription::verify_whisper_model,
};

use super::transfer::{
    download_file_to_path_with_progress, ensure_directory_exists, reset_model_download_control,
    update_model_download_snapshot, DownloadSlotGuard,
};

fn clear_managed_model_override<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let persisted_snapshot = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update the managed Whisper settings.".to_string())?;
        persisted.settings.whisper.model_path.clear();
        persisted.clone()
    };

    write_persisted_data(app, &persisted_snapshot)
}

fn recommended_model_target_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
    let asset_directory = PathBuf::from(&persisted.settings.asset_directory);
    let model_choice = whisper_model_spec(&persisted.settings.whisper.model_choice);
    let models_directory = asset_directory.join("models");
    drop(persisted);

    ensure_directory_exists(&models_directory)?;
    Ok(models_directory.join(model_choice.file_name))
}

pub(crate) fn download_recommended_whisper_model_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
    {
        let shell_state = app.state::<SharedShellState>();
        let shell = shell_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the shell state.".to_string())?;
        if shell.phase != "idle" && shell.phase != "error" {
            return Err("Finish the current task before downloading the Whisper model.".into());
        }
    }

    let download_slot =
        DownloadSlotGuard::acquire(app, "A model download is already in progress.")?;

    let target_path = recommended_model_target_path(app)?;
    let model_spec = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the current app settings.".to_string())?;
        *whisper_model_spec(&persisted.settings.whisper.model_choice)
    };
    let app_handle = app.clone();

    update_shell_snapshot(app, |shell| {
        shell.phase = "downloading-model".into();
        shell.status_text = format!(
            "Downloading the {} Whisper model to {}...",
            model_spec.label,
            target_path.display()
        );
        shell.started_at_ms = None;
        shell.current_recording_name = None;
    })?;
    update_model_download_snapshot(app, |snapshot| {
        snapshot.kind = Some("model".into());
        snapshot.status = "starting".into();
        snapshot.message = format!("Preparing the {} model download...", model_spec.label);
        snapshot.downloaded_bytes = 0;
        snapshot.total_bytes = None;
        snapshot.progress_percent = None;
        snapshot.target_path = Some(target_path.display().to_string());
    })?;

    std::thread::Builder::new()
        .name("whisper-model-download".into())
        .spawn(move || {
            let download_result = (|| -> Result<(), String> {
                if !target_path.exists() {
                    download_file_to_path_with_progress(
                        &app_handle,
                        model_spec.download_url,
                        &target_path,
                        "model",
                        &format!("the {} Whisper model", model_spec.label),
                    )?;
                }
                verify_whisper_model(&target_path)?;
                clear_managed_model_override(&app_handle)?;
                let detection = refresh_whisper_detection_state(&app_handle)?;
                update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some("model".into());
                    snapshot.status = "completed".into();
                    snapshot.message =
                        format!("{} model downloaded successfully.", model_spec.label);
                    snapshot.downloaded_bytes =
                        snapshot.total_bytes.unwrap_or(snapshot.downloaded_bytes);
                    snapshot.progress_percent = Some(100.0);
                    snapshot.target_path = Some(target_path.display().to_string());
                })?;
                reset_model_download_control(&app_handle)?;

                update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = if detection.status == "ready" {
                        format!(
                            "{} model is ready at {}",
                            model_spec.label,
                            target_path.display()
                        )
                    } else {
                        format!(
                            "Model downloaded, but Whisper still needs setup: {}",
                            detection.message
                        )
                    };
                    shell.started_at_ms = None;
                })?;

                log_event(
                    &app_handle,
                    "INFO",
                    "whisper.model_downloaded",
                    serde_json::json!({
                        "targetPath": target_path.display().to_string(),
                        "modelChoice": model_spec.id
                    }),
                );
                Ok(())
            })();

            if let Err(error) = download_result {
                let cancelled = error.ends_with("download cancelled.");
                let _ = update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some("model".into());
                    if cancelled {
                        snapshot.status = "cancelled".into();
                        snapshot.message = "Model download cancelled.".into();
                    } else {
                        snapshot.status = "failed".into();
                        snapshot.message = format!("Model download failed: {error}");
                    }
                });
                let _ = reset_model_download_control(&app_handle);
                let _ = update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = if cancelled {
                        "Whisper model download cancelled.".into()
                    } else {
                        format!("Whisper model download failed: {error}")
                    };
                    shell.started_at_ms = None;
                });
                log_event(
                    &app_handle,
                    "ERROR",
                    "whisper.model_download_failed",
                    serde_json::json!({ "message": error }),
                );
            }
        })
        .map_err(|error| error.to_string())?;
    download_slot.disarm();

    Ok(())
}
