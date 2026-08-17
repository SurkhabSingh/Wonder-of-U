use std::{
    fs,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Runtime};

use crate::{
    app_config::{RECOMMENDED_FFMPEG_RUNTIME_FILE, RECOMMENDED_FFMPEG_RUNTIME_URL},
    runtime_assets::{
        collect_managed_ffmpeg_candidates, managed_ffmpeg_install_directory, verify_ffmpeg_binary,
    },
};

use super::asset::AssetKind;
use super::envelope::{AssetDownloadPlan, Installed};
use super::transfer::{
    asset_directory, ensure_directory_exists, extract_zip_archive_to_directory,
    first_runnable_binary, verify_managed_binary_or_remove,
};

/// Where ffmpeg's archive is staged and where it unpacks to.
///
/// Staged through the shared `downloads/` directory and removed only on success, the same way
/// the dictionary does it — and the opposite of alass.
struct FfmpegPaths {
    archive: PathBuf,
    install: PathBuf,
}

fn ffmpeg_paths(asset_directory: &Path) -> FfmpegPaths {
    FfmpegPaths {
        archive: asset_directory
            .join("downloads")
            .join(RECOMMENDED_FFMPEG_RUNTIME_FILE),
        install: managed_ffmpeg_install_directory(asset_directory),
    }
}

fn find_existing_managed_ffmpeg_path(asset_directory: &Path) -> Option<PathBuf> {
    collect_managed_ffmpeg_candidates(asset_directory)
        .into_iter()
        .find(|candidate| candidate.exists())
}

/// As with the whisper runtime: a managed ffmpeg that will not run is not one we have.
///
/// This path was less badly off than whisper's — the broken binary was removed, so a
/// second click did download a working one — but the first click reported "FFmpeg
/// download failed" without having attempted a download, and the recovery depended on
/// the user trying the same button twice after being told it failed.
fn find_runnable_managed_ffmpeg_path(asset_directory: &Path) -> Option<PathBuf> {
    first_runnable_binary(
        collect_managed_ffmpeg_candidates(asset_directory),
        verify_ffmpeg_binary,
    )
}

pub(super) fn ffmpeg_plan<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<AssetDownloadPlan<R>, String> {
    let asset_directory = asset_directory(app)?;
    let paths = ffmpeg_paths(&asset_directory);
    ensure_directory_exists(
        paths
            .archive
            .parent()
            .ok_or_else(|| "The downloads directory has no parent.".to_string())?,
    )?;
    ensure_directory_exists(&paths.install)?;

    let shell_start_text = format!("Downloading FFmpeg to {}...", paths.install.display());
    // Names the archive while fetching; the finished card names the binary that was found.
    let starting_target_path = paths.archive.clone();

    Ok(AssetDownloadPlan {
        kind: AssetKind::Ffmpeg,
        slot_busy_message: "Another download is already in progress.".into(),
        shell_start_text,
        starting_message: "Preparing the FFmpeg download...".into(),
        starting_target_path,
        cancelled_message: "FFmpeg download cancelled.".into(),
        cancelled_shell_text: "FFmpeg download cancelled.".into(),
        failed_message_prefix: "FFmpeg download failed".into(),
        failed_shell_prefix: "FFmpeg download failed".into(),
        success_log_event: "ffmpeg.downloaded",
        failure_log_event: "ffmpeg.download_failed",
        install: Box::new(move |context| {
            // Skip-if-runnable, and note the test is *runnable*, not *present*. This is
            // deliberate and must survive: the Settings button is hidden entirely while
            // detection reports ready, so the only way to reach this with a working ffmpeg
            // installed is a re-download of something detection cannot see.
            let ffmpeg_path = match find_runnable_managed_ffmpeg_path(&asset_directory) {
                // Already run by the search, so nothing to check again here.
                Some(existing_path) => existing_path,
                None => {
                    context.fetch(
                        RECOMMENDED_FFMPEG_RUNTIME_URL,
                        &paths.archive,
                        "FFmpeg",
                    )?;

                    extract_zip_archive_to_directory(&paths.archive, &paths.install)?;
                    let downloaded_path = find_existing_managed_ffmpeg_path(&asset_directory)
                        .ok_or_else(|| {
                            "FFmpeg downloaded, but ffmpeg.exe was not found.".to_string()
                        })?;
                    // Detection trusts ffmpeg.exe by existence, so one that no longer runs
                    // has to go rather than keep reporting ready.
                    verify_managed_binary_or_remove(&downloaded_path, verify_ffmpeg_binary)?;
                    downloaded_path
                }
            };

            let log_details = serde_json::json!({
                "archivePath": paths.archive.display().to_string(),
                "ffmpegPath": ffmpeg_path.display().to_string()
            });
            // Success only, like the dictionary. A no-op on the skip path, where no
            // archive was ever fetched.
            let _ = fs::remove_file(&paths.archive);

            Ok(Installed {
                completed_message: "FFmpeg downloaded. MP3 compression is now enabled."
                    .into(),
                shell_success_text: format!(
                    "FFmpeg is ready at {}. Future transcribed recordings will be compressed to MP3.",
                    ffmpeg_path.display()
                ),
                target_path: ffmpeg_path,
                log_details,
            })
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::ffmpeg_paths;
    use std::path::Path;

    /// ffmpeg stages through `downloads/` and installs somewhere else entirely. Both halves
    /// matter: extraction clears the install directory, so pointing it at `downloads/` would
    /// wipe the staging area mid-download.
    #[test]
    fn the_archive_and_the_install_directory_are_different_places() {
        let paths = ffmpeg_paths(Path::new("C:/assets"));

        assert!(
            paths
                .archive
                .components()
                .any(|part| part.as_os_str() == "downloads"),
            "expected downloads/ staging: {:?}",
            paths.archive
        );
        assert_ne!(paths.archive.parent(), Some(paths.install.as_path()));
        assert!(
            !paths
                .install
                .components()
                .any(|part| part.as_os_str() == "downloads"),
            "the install directory is wiped on extract and must not be downloads/: {:?}",
            paths.install
        );
    }
}
