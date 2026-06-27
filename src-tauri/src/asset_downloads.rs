use std::{
    fs,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Manager, Runtime};

mod control;
mod transfer;

pub(crate) use control::{
    cancel_whisper_model_download_inner, toggle_whisper_model_download_pause_inner,
};

use transfer::{
    download_file_to_path_with_progress, ensure_directory_exists, extract_zip_archive_to_directory,
    reset_model_download_control, update_model_download_snapshot,
};

use crate::{
    app_config::{
        RECOMMENDED_FFMPEG_RUNTIME_FILE, RECOMMENDED_FFMPEG_RUNTIME_URL,
        RECOMMENDED_WHISPER_RUNTIME_FILE, RECOMMENDED_WHISPER_RUNTIME_VERSION,
    },
    app_runtime::{log_event, update_shell_snapshot},
    app_state::{sanitize_runtime_version, write_persisted_data},
    app_types::{
        whisper_model_spec, ModelDownloadControlState, SharedPersistedState, SharedShellState,
    },
    runtime_assets::{
        app_managed_runtime_directory, collect_managed_ffmpeg_candidates,
        collect_managed_whisper_cli_candidates, managed_ffmpeg_install_directory,
        refresh_whisper_detection_state, verify_ffmpeg_binary,
    },
    transcription::{verify_whisper_cli, verify_whisper_model},
};

fn clear_managed_whisper_override<R: Runtime>(
    app: &AppHandle<R>,
    asset_kind: &str,
) -> Result<(), String> {
    let persisted_snapshot = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update the managed Whisper settings.".to_string())?;

        match asset_kind {
            "runtime" => persisted.settings.whisper.cli_path.clear(),
            "model" => persisted.settings.whisper.model_path.clear(),
            _ => {}
        }

        persisted.clone()
    };

    write_persisted_data(app, &persisted_snapshot)
}

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

fn recommended_ffmpeg_archive_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
    let asset_directory = PathBuf::from(&persisted.settings.asset_directory);
    let downloads_directory = asset_directory.join("downloads");
    drop(persisted);

    ensure_directory_exists(&downloads_directory)?;
    Ok(downloads_directory.join(RECOMMENDED_FFMPEG_RUNTIME_FILE))
}

fn recommended_ffmpeg_install_directory<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
    let asset_directory = PathBuf::from(&persisted.settings.asset_directory);
    drop(persisted);

    let install_directory = managed_ffmpeg_install_directory(&asset_directory);
    ensure_directory_exists(&install_directory)?;
    Ok(install_directory)
}

fn find_existing_managed_ffmpeg_path(asset_directory: &Path) -> Option<PathBuf> {
    collect_managed_ffmpeg_candidates(asset_directory)
        .into_iter()
        .find(|candidate| candidate.exists())
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

    {
        let control_state = app.state::<ModelDownloadControlState>();
        let mut control = control_state
            .control
            .lock()
            .map_err(|_| "Could not initialize the model download control state.".to_string())?;
        if control.active {
            return Err("A model download is already in progress.".into());
        }
        control.active = true;
        control.paused = false;
        control.cancel_requested = false;
    }

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
                clear_managed_whisper_override(&app_handle, "model")?;
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

    Ok(())
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

    {
        let control_state = app.state::<ModelDownloadControlState>();
        let mut control = control_state
            .control
            .lock()
            .map_err(|_| "Could not initialize the download control state.".to_string())?;
        if control.active {
            return Err("Another download is already in progress.".into());
        }
        control.active = true;
        control.paused = false;
        control.cancel_requested = false;
    }

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

    Ok(())
}

pub(crate) fn download_recommended_ffmpeg_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
    {
        let shell_state = app.state::<SharedShellState>();
        let shell = shell_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the shell state.".to_string())?;
        if shell.phase != "idle" && shell.phase != "error" {
            return Err("Finish the current task before downloading FFmpeg.".into());
        }
    }

    {
        let control_state = app.state::<ModelDownloadControlState>();
        let mut control = control_state
            .control
            .lock()
            .map_err(|_| "Could not initialize the download control state.".to_string())?;
        if control.active {
            return Err("Another download is already in progress.".into());
        }
        control.active = true;
        control.paused = false;
        control.cancel_requested = false;
    }

    let archive_path = recommended_ffmpeg_archive_path(app)?;
    let install_directory = recommended_ffmpeg_install_directory(app)?;
    let app_handle = app.clone();

    update_shell_snapshot(app, |shell| {
        shell.phase = "downloading-model".into();
        shell.status_text = format!("Downloading FFmpeg to {}...", install_directory.display());
        shell.started_at_ms = None;
        shell.current_recording_name = None;
    })?;
    update_model_download_snapshot(app, |snapshot| {
        snapshot.kind = Some("ffmpeg".into());
        snapshot.status = "starting".into();
        snapshot.message = "Preparing the FFmpeg download...".into();
        snapshot.downloaded_bytes = 0;
        snapshot.total_bytes = None;
        snapshot.progress_percent = None;
        snapshot.target_path = Some(archive_path.display().to_string());
    })?;

    std::thread::Builder::new()
        .name("ffmpeg-download".into())
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

                let ffmpeg_path = if let Some(existing_path) =
                    find_existing_managed_ffmpeg_path(&asset_directory)
                {
                    verify_ffmpeg_binary(&existing_path)?;
                    existing_path
                } else {
                    download_file_to_path_with_progress(
                        &app_handle,
                        RECOMMENDED_FFMPEG_RUNTIME_URL,
                        &archive_path,
                        "ffmpeg",
                        "FFmpeg",
                    )?;

                    extract_zip_archive_to_directory(&archive_path, &install_directory)?;
                    find_existing_managed_ffmpeg_path(&asset_directory)
                        .ok_or_else(|| "FFmpeg downloaded, but ffmpeg.exe was not found.".to_string())?
                };

                verify_ffmpeg_binary(&ffmpeg_path)?;
                update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some("ffmpeg".into());
                    snapshot.status = "completed".into();
                    snapshot.message =
                        "FFmpeg downloaded. MP3 compression is now enabled.".into();
                    snapshot.downloaded_bytes =
                        snapshot.total_bytes.unwrap_or(snapshot.downloaded_bytes);
                    snapshot.progress_percent = Some(100.0);
                    snapshot.target_path = Some(ffmpeg_path.display().to_string());
                })?;
                reset_model_download_control(&app_handle)?;

                update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = format!(
                        "FFmpeg is ready at {}. Future transcribed recordings will be compressed to MP3.",
                        ffmpeg_path.display()
                    );
                    shell.started_at_ms = None;
                })?;

                log_event(
                    &app_handle,
                    "INFO",
                    "ffmpeg.downloaded",
                    serde_json::json!({
                        "archivePath": archive_path.display().to_string(),
                        "ffmpegPath": ffmpeg_path.display().to_string()
                    }),
                );

                let _ = fs::remove_file(&archive_path);
                Ok(())
            })();

            if let Err(error) = download_result {
                let cancelled = error.ends_with("download cancelled.");
                let _ = update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some("ffmpeg".into());
                    if cancelled {
                        snapshot.status = "cancelled".into();
                        snapshot.message = "FFmpeg download cancelled.".into();
                    } else {
                        snapshot.status = "failed".into();
                        snapshot.message = format!("FFmpeg download failed: {error}");
                    }
                });
                let _ = reset_model_download_control(&app_handle);
                let _ = update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = if cancelled {
                        "FFmpeg download cancelled.".into()
                    } else {
                        format!("FFmpeg download failed: {error}")
                    };
                    shell.started_at_ms = None;
                });
                log_event(
                    &app_handle,
                    "ERROR",
                    "ffmpeg.download_failed",
                    serde_json::json!({ "message": error }),
                );
            }
        })
        .map_err(|error| error.to_string())?;

    Ok(())
}
