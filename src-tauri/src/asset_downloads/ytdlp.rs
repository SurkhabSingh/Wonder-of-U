use std::path::PathBuf;

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_config::YTDLP_RELEASE_DOWNLOAD_URL,
    app_runtime::{log_event, update_shell_snapshot},
    app_types::{ModelDownloadControlState, SharedPersistedState, SharedShellState},
    runtime_assets::{managed_ytdlp_install_directory, verify_ytdlp_binary},
};

use super::transfer::{
    download_file_to_path_with_progress, ensure_directory_exists, reset_model_download_control,
    update_model_download_snapshot,
};

/// Downloads the latest yt-dlp release into `<asset_dir>/yt-dlp/yt-dlp.exe`.
///
/// Unlike the FFmpeg download this is a bare `.exe` (no zip to extract) and the
/// binary is always overwritten so a re-download refreshes it. The transfer runs
/// on a named OS thread and shares the `ModelDownloadControlState` slot with the
/// other asset downloads, so only one runs at a time and Cancel works.
pub(crate) fn download_recommended_ytdlp_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
    {
        let shell_state = app.state::<SharedShellState>();
        let shell = shell_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the shell state.".to_string())?;
        if shell.phase != "idle" && shell.phase != "error" {
            return Err("Finish the current task before downloading yt-dlp.".into());
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

    let install_directory = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the current app settings.".to_string())?;
        let asset_directory = PathBuf::from(&persisted.settings.asset_directory);
        drop(persisted);
        managed_ytdlp_install_directory(&asset_directory)
    };
    ensure_directory_exists(&install_directory)?;
    let target_path = install_directory.join("yt-dlp.exe");
    let app_handle = app.clone();

    update_shell_snapshot(app, |shell| {
        shell.phase = "downloading-model".into();
        shell.status_text = format!("Downloading yt-dlp to {}...", target_path.display());
        shell.started_at_ms = None;
        shell.current_recording_name = None;
    })?;
    update_model_download_snapshot(app, |snapshot| {
        snapshot.kind = Some("ytdlp".into());
        snapshot.status = "starting".into();
        snapshot.message = "Preparing the yt-dlp download...".into();
        snapshot.downloaded_bytes = 0;
        snapshot.total_bytes = None;
        snapshot.progress_percent = None;
        snapshot.target_path = Some(target_path.display().to_string());
    })?;

    std::thread::Builder::new()
        .name("ytdlp-download".into())
        .spawn(move || {
            let download_result = (|| -> Result<PathBuf, String> {
                // Always overwrite: a re-download is how the user refreshes yt-dlp.
                download_file_to_path_with_progress(
                    &app_handle,
                    YTDLP_RELEASE_DOWNLOAD_URL,
                    &target_path,
                    "ytdlp",
                    "yt-dlp",
                )?;

                verify_ytdlp_binary(&target_path)?;
                update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some("ytdlp".into());
                    snapshot.status = "completed".into();
                    snapshot.message = "yt-dlp downloaded. YouTube import is now enabled.".into();
                    snapshot.downloaded_bytes =
                        snapshot.total_bytes.unwrap_or(snapshot.downloaded_bytes);
                    snapshot.progress_percent = Some(100.0);
                    snapshot.target_path = Some(target_path.display().to_string());
                })?;
                reset_model_download_control(&app_handle)?;

                update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = format!(
                        "yt-dlp is ready at {}. You can import audio from YouTube.",
                        target_path.display()
                    );
                    shell.started_at_ms = None;
                })?;

                log_event(
                    &app_handle,
                    "INFO",
                    "ytdlp.downloaded",
                    serde_json::json!({ "ytdlpPath": target_path.display().to_string() }),
                );

                Ok(target_path.clone())
            })();

            if let Err(error) = download_result {
                let cancelled = error.ends_with("download cancelled.");
                let _ = update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some("ytdlp".into());
                    if cancelled {
                        snapshot.status = "cancelled".into();
                        snapshot.message = "yt-dlp download cancelled.".into();
                    } else {
                        snapshot.status = "failed".into();
                        snapshot.message = format!("yt-dlp download failed: {error}");
                    }
                });
                let _ = reset_model_download_control(&app_handle);
                let _ = update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = if cancelled {
                        "yt-dlp download cancelled.".into()
                    } else {
                        format!("yt-dlp download failed: {error}")
                    };
                    shell.started_at_ms = None;
                });
                log_event(
                    &app_handle,
                    "ERROR",
                    "ytdlp.download_failed",
                    serde_json::json!({ "message": error }),
                );
            }
        })
        .map_err(|error| error.to_string())?;

    Ok(())
}
