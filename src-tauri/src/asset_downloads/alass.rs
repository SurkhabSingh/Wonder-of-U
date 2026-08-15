use std::{fs, path::PathBuf};

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_config::ALASS_RELEASE_DOWNLOAD_URL,
    app_runtime::{log_event, update_shell_snapshot},
    app_types::{SharedPersistedState, SharedShellState},
    runtime_assets::{alass_archive_entry, managed_alass_install_directory, verify_alass_binary},
};

use super::asset::AssetKind;
use super::transfer::{
    download_file_to_path_with_progress, ensure_directory_exists, extract_zip_entry_to_path,
    reset_model_download_control, update_model_download_snapshot, verify_managed_binary_or_remove,
    DownloadSlotGuard,
};

/// Downloads alass into `<asset_dir>/alass/alass-cli.exe`.
///
/// The release is a 26 MB zip that unpacks to ~74 MB, almost all of it a second copy of
/// ffmpeg. Only `alass-cli.exe` is kept — see `runtime_assets/alass.rs` for why that is
/// sufficient and how the binary is pointed at the ffmpeg the app already manages.
pub(crate) fn download_recommended_alass_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
    {
        let shell_state = app.state::<SharedShellState>();
        let shell = shell_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the shell state.".to_string())?;
        if shell.phase != "idle" && shell.phase != "error" {
            return Err("Finish the current task before downloading alass.".into());
        }
    }

    let download_slot =
        DownloadSlotGuard::acquire(app, "Another download is already in progress.")?;

    let install_directory = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the current app settings.".to_string())?;
        let asset_directory = PathBuf::from(&persisted.settings.asset_directory);
        drop(persisted);
        managed_alass_install_directory(&asset_directory)
    };
    ensure_directory_exists(&install_directory)?;
    let archive_path = install_directory.join("alass-windows64.zip");
    let target_path = install_directory.join("alass-cli.exe");
    let app_handle = app.clone();

    update_shell_snapshot(app, |shell| {
        shell.phase = "downloading-model".into();
        shell.status_text = format!("Downloading alass to {}...", target_path.display());
        shell.started_at_ms = None;
        shell.current_recording_name = None;
    })?;
    update_model_download_snapshot(app, |snapshot| {
        snapshot.kind = Some(AssetKind::Alass);
        snapshot.status = "starting".into();
        snapshot.message = "Preparing the alass download...".into();
        snapshot.downloaded_bytes = 0;
        snapshot.total_bytes = None;
        snapshot.progress_percent = None;
        snapshot.target_path = Some(target_path.display().to_string());
    })?;

    std::thread::Builder::new()
        .name("alass-download".into())
        .spawn(move || {
            let download_result = (|| -> Result<PathBuf, String> {
                download_file_to_path_with_progress(
                    &app_handle,
                    ALASS_RELEASE_DOWNLOAD_URL,
                    &archive_path,
                    AssetKind::Alass,
                    "alass",
                )?;

                let extracted =
                    extract_zip_entry_to_path(&archive_path, &target_path, |names| {
                        alass_archive_entry(names)
                    });
                // The archive is 26 MB of mostly-discarded ffmpeg; it is never worth keeping,
                // and it is removed whether or not the extraction succeeded.
                let _ = fs::remove_file(&archive_path);
                extracted?;

                verify_managed_binary_or_remove(&target_path, verify_alass_binary)?;
                update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some(AssetKind::Alass);
                    snapshot.status = "completed".into();
                    snapshot.message =
                        "alass downloaded. Subtitles can now be synced automatically.".into();
                    snapshot.downloaded_bytes =
                        snapshot.total_bytes.unwrap_or(snapshot.downloaded_bytes);
                    snapshot.progress_percent = Some(100.0);
                    snapshot.target_path = Some(target_path.display().to_string());
                })?;
                reset_model_download_control(&app_handle)?;

                update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = format!(
                        "alass is ready at {}. Out-of-sync subtitles can be aligned automatically.",
                        target_path.display()
                    );
                    shell.started_at_ms = None;
                })?;

                log_event(
                    &app_handle,
                    "INFO",
                    "alass.downloaded",
                    serde_json::json!({ "alassPath": target_path.display().to_string() }),
                );

                Ok(target_path.clone())
            })();

            if let Err(error) = download_result {
                let cancelled = error.ends_with("download cancelled.");
                let _ = update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some(AssetKind::Alass);
                    if cancelled {
                        snapshot.status = "cancelled".into();
                        snapshot.message = "alass download cancelled.".into();
                    } else {
                        snapshot.status = "failed".into();
                        snapshot.message = format!("alass download failed: {error}");
                    }
                });
                let _ = reset_model_download_control(&app_handle);
                let _ = update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = if cancelled {
                        "alass download cancelled.".into()
                    } else {
                        format!("alass download failed: {error}")
                    };
                    shell.started_at_ms = None;
                });
                log_event(
                    &app_handle,
                    "ERROR",
                    "alass.download_failed",
                    serde_json::json!({ "message": error }),
                );
            }

            drop(download_slot);
        })
        .map_err(|error| error.to_string())?;

    Ok(())
}
