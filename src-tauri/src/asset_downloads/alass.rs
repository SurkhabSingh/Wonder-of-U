use std::{
    fs,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Runtime};

use crate::{
    app_config::ALASS_RELEASE_DOWNLOAD_URL,
    runtime_assets::{alass_archive_entry, managed_alass_install_directory, verify_alass_binary},
};

use super::asset::AssetKind;
use super::envelope::{run_asset_download, AssetDownloadPlan, Installed};
use super::transfer::{
    asset_directory, ensure_directory_exists, extract_zip_entry_to_path,
    verify_managed_binary_or_remove,
};

/// Where alass puts its two files.
///
/// **The archive sits in the install directory, not `downloads/`** — alass is the only asset
/// that does this. It is deliberate: the zip is deleted the moment extraction has been
/// attempted, so it never lives long enough for the staging directory to buy anything, and
/// keeping it beside its own install is one less place a stale 26 MB file can hide.
struct AlassPaths {
    archive: PathBuf,
    target: PathBuf,
}

fn alass_paths(asset_directory: &Path) -> AlassPaths {
    let install_directory = managed_alass_install_directory(asset_directory);
    AlassPaths {
        archive: install_directory.join("alass-windows64.zip"),
        target: install_directory.join("alass-cli.exe"),
    }
}

/// Downloads alass into `<asset_dir>/alass/alass-cli.exe`.
///
/// The release is a 26 MB zip that unpacks to ~74 MB, almost all of it a second copy of
/// ffmpeg. Only `alass-cli.exe` is kept — see `runtime_assets/alass.rs` for why that is
/// sufficient and how the binary is pointed at the ffmpeg the app already manages.
pub(crate) fn download_recommended_alass_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
    let asset_directory = asset_directory(app)?;
    let install_directory = managed_alass_install_directory(&asset_directory);
    ensure_directory_exists(&install_directory)?;
    let paths = alass_paths(&asset_directory);

    // The card and the shell point at the binary being installed, not the archive being
    // fetched. Three assets do it this way and three point at the archive; the envelope must
    // not decide which, so it is stated here.
    let shown_path = paths.target.clone();

    run_asset_download(
        app,
        AssetDownloadPlan {
            kind: AssetKind::Alass,
            thread_name: "alass-download",
            shell_busy_message: "Finish the current task before downloading alass.".into(),
            slot_busy_message: "Another download is already in progress.".into(),
            shell_start_text: format!("Downloading alass to {}...", shown_path.display()),
            starting_message: "Preparing the alass download...".into(),
            starting_target_path: shown_path,
            cancelled_message: "alass download cancelled.".into(),
            cancelled_shell_text: "alass download cancelled.".into(),
            failed_message_prefix: "alass download failed".into(),
            failed_shell_prefix: "alass download failed".into(),
            success_log_event: "alass.downloaded",
            failure_log_event: "alass.download_failed",
            install: Box::new(move |context| {
                context.fetch(ALASS_RELEASE_DOWNLOAD_URL, &paths.archive, "alass")?;

                let extracted =
                    extract_zip_entry_to_path(&paths.archive, &paths.target, |names| {
                        alass_archive_entry(names)
                    });
                // The archive is 26 MB of mostly-discarded ffmpeg; it is never worth keeping,
                // and it is removed whether or not the extraction succeeded. Hence the
                // deliberate order: remove first, THEN propagate.
                let _ = fs::remove_file(&paths.archive);
                extracted?;

                verify_managed_binary_or_remove(&paths.target, verify_alass_binary)?;

                Ok(Installed {
                    completed_message:
                        "alass downloaded. Subtitles can now be synced automatically.".into(),
                    shell_success_text: format!(
                        "alass is ready at {}. Out-of-sync subtitles can be aligned automatically.",
                        paths.target.display()
                    ),
                    log_details: serde_json::json!({
                        "alassPath": paths.target.display().to_string()
                    }),
                    // Same path the card started on — only the dictionary discovers a new one.
                    target_path: paths.target,
                })
            }),
        },
    )
}

#[cfg(test)]
mod tests {
    use super::alass_paths;
    use std::path::Path;

    /// alass is the one asset that stages its archive inside its own install directory. If a
    /// conversion quietly moved it to `downloads/` like the others, nothing would fail to
    /// compile and nothing would misbehave — the zip would simply start surviving in a
    /// different place. This is the only thing that would notice.
    #[test]
    fn the_archive_is_staged_beside_the_binary_not_in_downloads() {
        let paths = alass_paths(Path::new("C:/assets"));

        assert_eq!(
            paths.archive.parent(),
            paths.target.parent(),
            "the archive and the binary share a directory"
        );
        assert!(
            !paths
                .archive
                .components()
                .any(|part| part.as_os_str() == "downloads"),
            "alass stages in its own directory: {:?}",
            paths.archive
        );
    }

    /// The extracted binary is what detection looks for, so its name is load-bearing.
    #[test]
    fn the_installed_binary_is_the_cli_not_the_archive() {
        let paths = alass_paths(Path::new("C:/assets"));

        assert!(paths.target.ends_with("alass/alass-cli.exe"), "{:?}", paths.target);
        assert!(paths.archive.ends_with("alass/alass-windows64.zip"), "{:?}", paths.archive);
    }
}
