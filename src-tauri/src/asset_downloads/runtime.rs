use std::{
    fs,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_config::{RECOMMENDED_WHISPER_RUNTIME_FILE, RECOMMENDED_WHISPER_RUNTIME_VERSION},
    app_runtime::{log_event, update_shell_snapshot},
    app_state::{sanitize_runtime_version, write_persisted_data},
    app_types::{SharedPersistedState, SharedShellState},
    runtime_assets::{
        app_managed_runtime_directory, collect_managed_whisper_cli_candidates,
        refresh_whisper_detection_state,
    },
    transcription::verify_whisper_cli,
};

use super::transfer::{
    download_file_to_path_with_progress, ensure_directory_exists, extract_zip_archive_to_directory,
    reset_model_download_control, update_model_download_snapshot, DownloadSlotGuard,
};

fn activate_managed_runtime_version<R: Runtime>(
    app: &AppHandle<R>,
    runtime_version: &str,
) -> Result<(), String> {
    let normalized_version = sanitize_runtime_version(runtime_version);
    let persisted_snapshot = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update the managed Whisper runtime.".to_string())?;
        persisted.settings.whisper.runtime_version = normalized_version;
        persisted.settings.whisper.cli_path.clear();
        persisted.clone()
    };

    write_persisted_data(app, &persisted_snapshot)
}

fn runtime_download_url(runtime_version: &str) -> String {
    format!(
        "https://github.com/ggml-org/whisper.cpp/releases/download/{}/{}",
        sanitize_runtime_version(runtime_version),
        RECOMMENDED_WHISPER_RUNTIME_FILE
    )
}

fn recommended_runtime_archive_path<R: Runtime>(
    app: &AppHandle<R>,
    runtime_version: &str,
) -> Result<PathBuf, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
    let asset_directory = PathBuf::from(&persisted.settings.asset_directory);
    let runtime_directory = asset_directory.join("downloads");
    drop(persisted);

    ensure_directory_exists(&runtime_directory)?;
    Ok(runtime_directory.join(format!(
        "{}-{}",
        sanitize_runtime_version(runtime_version),
        RECOMMENDED_WHISPER_RUNTIME_FILE
    )))
}

fn recommended_runtime_install_directory<R: Runtime>(
    app: &AppHandle<R>,
    runtime_version: &str,
) -> Result<PathBuf, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
    let asset_directory = PathBuf::from(&persisted.settings.asset_directory);
    drop(persisted);

    let runtime_directory = app_managed_runtime_directory(&asset_directory, runtime_version);
    ensure_directory_exists(&runtime_directory)?;
    Ok(runtime_directory)
}

fn find_existing_managed_cli_path(
    asset_directory: &Path,
    runtime_version: &str,
) -> Option<PathBuf> {
    collect_managed_whisper_cli_candidates(asset_directory, runtime_version)
        .into_iter()
        .find(|candidate| candidate.exists())
}

pub(crate) fn download_recommended_whisper_runtime_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
    download_whisper_runtime_version_inner(app, RECOMMENDED_WHISPER_RUNTIME_VERSION)
}

pub(crate) fn download_whisper_runtime_version_inner<R: Runtime>(
    app: &AppHandle<R>,
    runtime_version: &str,
) -> Result<(), String> {
    let runtime_version = sanitize_runtime_version(runtime_version);
    {
        let shell_state = app.state::<SharedShellState>();
        let shell = shell_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the shell state.".to_string())?;
        if shell.phase != "idle" && shell.phase != "error" {
            return Err("Finish the current task before downloading the Whisper runtime.".into());
        }
    }

    let download_slot =
        DownloadSlotGuard::acquire(app, "Another download is already in progress.")?;

    let archive_path = recommended_runtime_archive_path(app, &runtime_version)?;
    let install_directory = recommended_runtime_install_directory(app, &runtime_version)?;
    let download_url = runtime_download_url(&runtime_version);
    let app_handle = app.clone();

    update_shell_snapshot(app, |shell| {
        shell.phase = "downloading-model".into();
        shell.status_text = format!(
            "Downloading Whisper runtime {} to {}...",
            runtime_version,
            install_directory.display()
        );
        shell.started_at_ms = None;
        shell.current_recording_name = None;
    })?;
    update_model_download_snapshot(app, |snapshot| {
        snapshot.kind = Some("runtime".into());
        snapshot.status = "starting".into();
        snapshot.message = "Preparing the Whisper runtime download...".into();
        snapshot.downloaded_bytes = 0;
        snapshot.total_bytes = None;
        snapshot.progress_percent = None;
        snapshot.target_path = Some(archive_path.display().to_string());
    })?;

    std::thread::Builder::new()
        .name("whisper-runtime-download".into())
        .spawn(move || {
            let download_result = (|| -> Result<(), String> {
                let asset_directory = {
                    let persisted_state = app_handle.state::<SharedPersistedState>();
                    let persisted = persisted_state
                        .0
                        .lock()
                        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
                    PathBuf::from(&persisted.settings.asset_directory)
                };

                let cli_path = if let Some(existing_cli_path) =
                    find_existing_managed_cli_path(&asset_directory, &runtime_version)
                {
                    verify_whisper_cli(&existing_cli_path)?;
                    existing_cli_path
                } else {
                    download_file_to_path_with_progress(
                        &app_handle,
                        &download_url,
                        &archive_path,
                        "runtime",
                        &format!("Whisper runtime {runtime_version}"),
                    )?;

                    extract_zip_archive_to_directory(&archive_path, &install_directory)?;
                    find_existing_managed_cli_path(&asset_directory, &runtime_version).ok_or_else(
                        || "The runtime downloaded, but whisper-cli.exe was not found.".to_string(),
                    )?
                };
                verify_whisper_cli(&cli_path)?;
                activate_managed_runtime_version(&app_handle, &runtime_version)?;

                let detection = refresh_whisper_detection_state(&app_handle)?;
                update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some("runtime".into());
                    snapshot.status = "completed".into();
                    snapshot.message = format!(
                        "Whisper runtime {} downloaded and activated.",
                        runtime_version
                    );
                    snapshot.downloaded_bytes =
                        snapshot.total_bytes.unwrap_or(snapshot.downloaded_bytes);
                    snapshot.progress_percent = Some(100.0);
                    snapshot.target_path = Some(cli_path.display().to_string());
                })?;
                reset_model_download_control(&app_handle)?;

                update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = if detection.status == "ready" {
                        format!(
                            "Whisper runtime {} is ready at {}",
                            runtime_version,
                            cli_path.display()
                        )
                    } else {
                        format!(
                            "Runtime downloaded, but Whisper still needs setup: {}",
                            detection.message
                        )
                    };
                    shell.started_at_ms = None;
                })?;

                log_event(
                    &app_handle,
                    "INFO",
                    "whisper.runtime_downloaded",
                    serde_json::json!({
                        "runtimeArchivePath": archive_path.display().to_string(),
                        "cliPath": cli_path.display().to_string(),
                        "runtimeVersion": runtime_version
                    }),
                );

                let _ = fs::remove_file(&archive_path);
                Ok(())
            })();

            if let Err(error) = download_result {
                let cancelled = error.ends_with("download cancelled.");
                let _ = update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some("runtime".into());
                    if cancelled {
                        snapshot.status = "cancelled".into();
                        snapshot.message = "Runtime download cancelled.".into();
                    } else {
                        snapshot.status = "failed".into();
                        snapshot.message = format!("Runtime download failed: {error}");
                    }
                });
                let _ = reset_model_download_control(&app_handle);
                let _ = update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = if cancelled {
                        "Whisper runtime download cancelled.".into()
                    } else {
                        format!("Whisper runtime download failed: {error}")
                    };
                    shell.started_at_ms = None;
                });
                log_event(
                    &app_handle,
                    "ERROR",
                    "whisper.runtime_download_failed",
                    serde_json::json!({ "message": error }),
                );
            }
        })
        .map_err(|error| error.to_string())?;
    download_slot.disarm();

    Ok(())
}
