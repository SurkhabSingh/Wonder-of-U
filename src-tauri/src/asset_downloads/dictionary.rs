use std::{
    fs,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_config::{IPADIC_DICTIONARY_FILE, IPADIC_DICTIONARY_URL},
    app_runtime::{log_event, update_shell_snapshot},
    app_types::{SharedPersistedState, SharedShellState},
    runtime_assets::{find_managed_dictionary_root, managed_dictionary_install_directory},
    tokenizer::dictionary_loads,
};

use super::transfer::{
    download_file_to_path_with_progress, ensure_directory_exists, extract_zip_archive_to_directory,
    reset_model_download_control, update_model_download_snapshot,
    verify_managed_directory_or_remove, DownloadSlotGuard, PartialInstallGuard,
};

fn dictionary_archive_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
    let asset_directory = PathBuf::from(&persisted.settings.asset_directory);
    let downloads_directory = asset_directory.join("downloads");
    drop(persisted);

    ensure_directory_exists(&downloads_directory)?;
    Ok(downloads_directory.join(IPADIC_DICTIONARY_FILE))
}

fn configured_asset_directory<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
    let asset_directory = PathBuf::from(&persisted.settings.asset_directory);
    drop(persisted);
    Ok(asset_directory)
}

/// Proves the extracted dictionary is one lindera can actually read.
///
/// The load parses every component lindera needs, so it answers the only question
/// worth asking about a dictionary directory — far more than checking that the
/// files exist. It costs a one-off ~57MB read on the download thread, which is why
/// it happens here once and never in detection.
fn verify_extracted_dictionary(
    install_directory: &Path,
    asset_directory: &Path,
) -> Result<PathBuf, String> {
    let dictionary_path = find_managed_dictionary_root(asset_directory).ok_or_else(|| {
        format!(
            "The dictionary was downloaded, but no dictionary was found under {}.",
            install_directory.display()
        )
    })?;
    dictionary_loads(&dictionary_path)?;
    Ok(dictionary_path)
}

/// Downloads the pinned IPADIC dictionary into `<asset_dir>/lindera-ipadic/<version>/`.
///
/// Shaped like the FFmpeg download — a zip fetched to the downloads folder and
/// unpacked — and it shares the `ModelDownloadControlState` slot with the other
/// asset downloads, so only one runs at a time and Cancel works.
pub(crate) fn download_recommended_dictionary_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
    {
        let shell_state = app.state::<SharedShellState>();
        let shell = shell_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the shell state.".to_string())?;
        if shell.phase != "idle" && shell.phase != "error" {
            return Err(
                "Finish the current task before downloading the Japanese dictionary.".into(),
            );
        }
    }

    let download_slot =
        DownloadSlotGuard::acquire(app, "Another download is already in progress.")?;

    let archive_path = dictionary_archive_path(app)?;
    let asset_directory = configured_asset_directory(app)?;
    let install_directory = managed_dictionary_install_directory(&asset_directory);
    ensure_directory_exists(&install_directory)?;
    let app_handle = app.clone();

    update_shell_snapshot(app, |shell| {
        shell.phase = "downloading-model".into();
        shell.status_text = format!(
            "Downloading the Japanese dictionary to {}...",
            install_directory.display()
        );
        shell.started_at_ms = None;
        shell.current_recording_name = None;
    })?;
    update_model_download_snapshot(app, |snapshot| {
        snapshot.kind = Some("dictionary".into());
        snapshot.status = "starting".into();
        snapshot.message = "Preparing the Japanese dictionary download...".into();
        snapshot.downloaded_bytes = 0;
        snapshot.total_bytes = None;
        snapshot.progress_percent = None;
        snapshot.target_path = Some(archive_path.display().to_string());
    })?;

    std::thread::Builder::new()
        .name("dictionary-download".into())
        .spawn(move || {
            let download_result = (|| -> Result<(), String> {
                download_file_to_path_with_progress(
                    &app_handle,
                    IPADIC_DICTIONARY_URL,
                    &archive_path,
                    "dictionary",
                    "the Japanese dictionary",
                )?;

                // Armed across extraction only: an interrupted unpack writes
                // metadata.json long before the word list, and detection keys on
                // metadata.json. Once the archive is whole, the validation below
                // owns the cleanup instead.
                let mut install_guard = PartialInstallGuard::new(install_directory.clone());
                extract_zip_archive_to_directory(&archive_path, &install_directory)?;
                install_guard.disarm();

                let dictionary_path =
                    verify_managed_directory_or_remove(&install_directory, |_| {
                        verify_extracted_dictionary(&install_directory, &asset_directory)
                    })?;

                update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some("dictionary".into());
                    snapshot.status = "completed".into();
                    snapshot.message =
                        "The Japanese dictionary is ready. Sentences can be analysed word by word."
                            .into();
                    snapshot.downloaded_bytes =
                        snapshot.total_bytes.unwrap_or(snapshot.downloaded_bytes);
                    snapshot.progress_percent = Some(100.0);
                    snapshot.target_path = Some(dictionary_path.display().to_string());
                })?;
                reset_model_download_control(&app_handle)?;

                update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = format!(
                        "The Japanese dictionary is ready at {}.",
                        dictionary_path.display()
                    );
                    shell.started_at_ms = None;
                })?;

                log_event(
                    &app_handle,
                    "INFO",
                    "dictionary.downloaded",
                    serde_json::json!({
                        "archivePath": archive_path.display().to_string(),
                        "dictionaryPath": dictionary_path.display().to_string()
                    }),
                );

                let _ = fs::remove_file(&archive_path);
                Ok(())
            })();

            if let Err(error) = download_result {
                let cancelled = error.ends_with("download cancelled.");
                let _ = update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some("dictionary".into());
                    if cancelled {
                        snapshot.status = "cancelled".into();
                        snapshot.message = "Japanese dictionary download cancelled.".into();
                    } else {
                        snapshot.status = "failed".into();
                        snapshot.message = format!("Japanese dictionary download failed: {error}");
                    }
                });
                let _ = reset_model_download_control(&app_handle);
                let _ = update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = if cancelled {
                        "Japanese dictionary download cancelled.".into()
                    } else {
                        format!("Japanese dictionary download failed: {error}")
                    };
                    shell.started_at_ms = None;
                });
                log_event(
                    &app_handle,
                    "ERROR",
                    "dictionary.download_failed",
                    serde_json::json!({ "message": error }),
                );
            }
        })
        .map_err(|error| error.to_string())?;
    download_slot.disarm();

    Ok(())
}
