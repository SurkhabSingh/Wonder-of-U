use std::{
    fs,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_config::RECOMMENDED_WHISPER_RUNTIME_FILE,
    app_state::{sanitize_runtime_version, write_persisted_data},
    app_types::SharedPersistedState,
    runtime_assets::{
        app_managed_runtime_directory, collect_managed_whisper_cli_candidates,
        refresh_whisper_detection_state,
    },
    transcription::verify_whisper_cli,
};

use super::asset::AssetKind;
use super::envelope::{AssetDownloadPlan, Installed};
use super::transfer::{
    asset_directory, ensure_directory_exists, extract_zip_archive_to_directory,
    first_runnable_binary, verify_managed_binary_or_remove,
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

fn find_existing_managed_cli_path(
    asset_directory: &Path,
    runtime_version: &str,
) -> Option<PathBuf> {
    collect_managed_whisper_cli_candidates(asset_directory, runtime_version)
        .into_iter()
        .find(|candidate| candidate.exists())
}

/// A managed whisper-cli that will not run does not count as one we have.
///
/// The download skipped itself whenever the file merely existed, and existence is a
/// weak claim for an executable: antivirus can quarantine one of the DLLs beside it,
/// an extraction can be cut short, a disk can fill. Verification did run — and then
/// returned the error, leaving the file exactly where it was. So every retry found it
/// again, failed again, and there was no way out through the interface; the one action
/// that would have replaced the file was the action being refused.
///
/// Failing candidates are removed, because detection tests existence too
/// (`detect_local_whisper`) and would otherwise keep reporting the runtime ready while
/// nothing could transcribe. If the download that follows also fails, "not installed"
/// is the truthful state to be left in.
fn find_runnable_managed_cli_path(
    asset_directory: &Path,
    runtime_version: &str,
) -> Option<PathBuf> {
    first_runnable_binary(
        collect_managed_whisper_cli_candidates(asset_directory, runtime_version),
        verify_whisper_cli,
    )
}

/// Where a given runtime version stages its archive and unpacks to.
///
/// Version-scoped on both halves, which is why the skip-if-runnable check below can never
/// suppress a download of a *different* version: each lives in its own directory and is
/// searched by its own name.
struct RuntimePaths {
    archive: PathBuf,
    install: PathBuf,
}

fn runtime_paths(asset_directory: &Path, runtime_version: &str) -> RuntimePaths {
    RuntimePaths {
        archive: asset_directory.join("downloads").join(format!(
            "{}-{}",
            sanitize_runtime_version(runtime_version),
            RECOMMENDED_WHISPER_RUNTIME_FILE
        )),
        install: app_managed_runtime_directory(asset_directory, runtime_version),
    }
}

pub(super) fn whisper_runtime_plan<R: Runtime>(
    app: &AppHandle<R>,
    runtime_version: &str,
) -> Result<AssetDownloadPlan<R>, String> {
    let runtime_version = sanitize_runtime_version(runtime_version);
    let asset_directory = asset_directory(app)?;
    let paths = runtime_paths(&asset_directory, &runtime_version);
    ensure_directory_exists(
        paths
            .archive
            .parent()
            .ok_or_else(|| "The downloads directory has no parent.".to_string())?,
    )?;
    ensure_directory_exists(&paths.install)?;

    let download_url = runtime_download_url(&runtime_version);
    let shell_start_text = format!(
        "Downloading Whisper runtime {} to {}...",
        runtime_version,
        paths.install.display()
    );
    let starting_target_path = paths.archive.clone();

    Ok(AssetDownloadPlan {
        kind: AssetKind::Runtime,
        slot_busy_message: "Another download is already in progress.".into(),
        shell_start_text,
        starting_message: "Preparing the Whisper runtime download...".into(),
        starting_target_path,
        // The snapshot and the shell genuinely disagree here — "Runtime" against "Whisper
        // runtime" — which is why the plan carries four strings rather than two.
        cancelled_message: "Runtime download cancelled.".into(),
        cancelled_shell_text: "Whisper runtime download cancelled.".into(),
        failed_message_prefix: "Runtime download failed".into(),
        failed_shell_prefix: "Whisper runtime download failed".into(),
        success_log_event: "whisper.runtime_downloaded",
        failure_log_event: "whisper.runtime_download_failed",
        install: Box::new(move |context| {
            let cli_path = match find_runnable_managed_cli_path(&asset_directory, &runtime_version) {
                // Already run by the search, so nothing to check again here.
                Some(existing_cli_path) => existing_cli_path,
                None => {
                    context.fetch(
                        &download_url,
                        &paths.archive,
                        &format!("Whisper runtime {runtime_version}"),
                    )?;

                    extract_zip_archive_to_directory(&paths.archive, &paths.install)?;
                    let downloaded_cli_path =
                        find_existing_managed_cli_path(&asset_directory, &runtime_version)
                            .ok_or_else(|| {
                                "The runtime downloaded, but whisper-cli.exe was not found."
                                    .to_string()
                            })?;
                    // A fresh download that cannot run is reported, not kept: leaving it
                    // would have detection call the runtime ready on the next launch.
                    verify_managed_binary_or_remove(&downloaded_cli_path, verify_whisper_cli)?;
                    downloaded_cli_path
                }
            };

            // The two steps that make this more than a file fetch: point the settings at
            // the version just installed, then re-read readiness so the sentence below can
            // tell the truth about it.
            activate_managed_runtime_version(context.app(), &runtime_version)?;
            let detection = refresh_whisper_detection_state(context.app())?;

            let log_details = serde_json::json!({
                "runtimeArchivePath": paths.archive.display().to_string(),
                "cliPath": cli_path.display().to_string(),
                "runtimeVersion": runtime_version
            });
            let _ = fs::remove_file(&paths.archive);

            Ok(Installed {
                completed_message: format!(
                    "Whisper runtime {} downloaded and activated.",
                    runtime_version
                ),
                // This is what `Installed` returning sentences buys: a fetch can succeed
                // and Whisper still not be usable, and only the install knows that.
                shell_success_text: if detection.status == "ready" {
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
                },
                target_path: cli_path,
                log_details,
            })
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::runtime_paths;
    use std::path::Path;

    /// Both halves are version-scoped, which is what lets two runtimes coexist and what stops
    /// the skip-if-runnable check from suppressing a download of a different version.
    #[test]
    fn each_runtime_version_stages_and_installs_under_its_own_name() {
        let older = runtime_paths(Path::new("C:/assets"), "v1.8.4");
        let newer = runtime_paths(Path::new("C:/assets"), "v1.9.1");

        assert_ne!(older.archive, newer.archive);
        assert_ne!(older.install, newer.install);
        assert!(
            older.install.components().any(|p| p.as_os_str() == "v1.8.4"),
            "{:?}",
            older.install
        );
        assert!(
            older
                .archive
                .components()
                .any(|part| part.as_os_str() == "downloads"),
            "expected downloads/ staging: {:?}",
            older.archive
        );
    }
}
