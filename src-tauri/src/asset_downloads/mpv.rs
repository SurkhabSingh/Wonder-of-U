use std::{
    fs,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Runtime};

use crate::{
    app_config::{MPV_RELEASE_FILE, MPV_RELEASE_SHA256, MPV_RELEASE_URL, MPV_SKIPPED_ENTRY},
    runtime_assets::{collect_managed_mpv_candidates, managed_mpv_install_directory, verify_mpv_binary},
};

use super::asset::AssetKind;
use super::envelope::{AssetDownloadPlan, Installed};
use super::transfer::{
    asset_directory, ensure_directory_exists, extract_zip_archive_except, first_runnable_binary,
    verify_managed_binary_or_remove, verify_sha256,
};

/// Where the archive is staged and where it unpacks to, following ffmpeg: staged in the shared
/// `downloads/` folder and removed only once the install has succeeded.
struct MpvPaths {
    archive: PathBuf,
    install: PathBuf,
}

fn mpv_paths(asset_directory: &Path) -> MpvPaths {
    MpvPaths {
        archive: asset_directory.join("downloads").join(MPV_RELEASE_FILE),
        install: managed_mpv_install_directory(asset_directory),
    }
}

fn find_runnable_managed_mpv(asset_directory: &Path) -> Option<PathBuf> {
    first_runnable_binary(
        collect_managed_mpv_candidates(asset_directory),
        verify_mpv_binary,
    )
}

/// `reinstall` fetches a fresh copy even when a working one is installed.
///
/// The ordinary download skips in that case, which is right for "I have none" and wrong for
/// "replace the one I have" — the second would report a download it never made.
pub(super) fn mpv_plan<R: Runtime>(
    app: &AppHandle<R>,
    reinstall: bool,
) -> Result<AssetDownloadPlan<R>, String> {
    let asset_directory = asset_directory(app)?;
    let paths = mpv_paths(&asset_directory);
    ensure_directory_exists(
        paths
            .archive
            .parent()
            .ok_or_else(|| "The downloads directory has no parent.".to_string())?,
    )?;
    ensure_directory_exists(&paths.install)?;

    let shell_start_text = format!("Downloading mpv to {}...", paths.install.display());
    let starting_target_path = paths.archive.clone();

    Ok(AssetDownloadPlan {
        kind: AssetKind::Mpv,
        slot_busy_message: "Another download is already in progress.".into(),
        shell_start_text,
        starting_message: "Preparing the mpv download...".into(),
        starting_target_path,
        cancelled_message: "mpv download cancelled.".into(),
        cancelled_shell_text: "mpv download cancelled.".into(),
        failed_message_prefix: "mpv download failed".into(),
        failed_shell_prefix: "mpv download failed".into(),
        success_log_event: "mpv.downloaded",
        failure_log_event: "mpv.download_failed",
        install: Box::new(move |context| {
            // Skip when one is already runnable, as ffmpeg does — but only when the request
            // is not a reinstall. A reinstall that skipped would return the success envelope
            // below having fetched nothing, which is what the Settings button offering exactly
            // that would have done.
            let installed = if reinstall {
                None
            } else {
                find_runnable_managed_mpv(&asset_directory)
            };
            let mpv_path = match installed {
                Some(existing_path) => existing_path,
                None => {
                    context.fetch(MPV_RELEASE_URL, &paths.archive, "mpv")?;

                    // Before unpacking, not after: a transfer that ends early still leaves a
                    // file, and every other asset treats that file existing as proof it worked.
                    verify_sha256(&paths.archive, MPV_RELEASE_SHA256)?;

                    // The debug symbols are four fifths of the archive and of no use to anyone
                    // running the player.
                    extract_zip_archive_except(&paths.archive, &paths.install, |name| {
                        name.eq_ignore_ascii_case(MPV_SKIPPED_ENTRY)
                    })?;

                    let downloaded_path = collect_managed_mpv_candidates(&asset_directory)
                        .into_iter()
                        .find(|candidate: &PathBuf| candidate.exists())
                        .ok_or_else(|| "mpv downloaded, but mpv.exe was not found.".to_string())?;
                    // Detection trusts a managed binary by its presence, so one that will not
                    // run has to go rather than keep reporting ready.
                    verify_managed_binary_or_remove(&downloaded_path, verify_mpv_binary)?;
                    downloaded_path
                }
            };

            let log_details = serde_json::json!({
                "archivePath": paths.archive.display().to_string(),
                "mpvPath": mpv_path.display().to_string()
            });
            // Only on success, and a no-op on the skip path where nothing was fetched.
            let _ = fs::remove_file(&paths.archive);

            Ok(Installed {
                completed_message: "mpv downloaded. Watch & Mine is ready.".into(),
                shell_success_text: format!(
                    "mpv is ready at {}. You can watch a video and mine lines as you go.",
                    mpv_path.display()
                ),
                target_path: mpv_path,
                log_details,
            })
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::mpv_paths;
    use crate::app_config::{MPV_RELEASE_FILE, MPV_RELEASE_SHA256, MPV_RELEASE_URL};
    use std::path::Path;

    /// Staging and installing are different places: extraction empties the install directory,
    /// so pointing it at the staging folder would delete the archive mid-install.
    #[test]
    fn the_archive_and_the_install_directory_are_different_places() {
        let paths = mpv_paths(Path::new("C:/assets"));

        assert!(
            paths
                .archive
                .components()
                .any(|part| part.as_os_str() == "downloads"),
            "expected downloads/ staging: {:?}",
            paths.archive
        );
        assert_ne!(paths.archive.parent(), Some(paths.install.as_path()));
    }

    /// The URL has to name the archive that gets staged, or mpv is fetched under one name and
    /// looked for under another.
    #[test]
    fn the_url_ends_with_the_file_it_stages() {
        assert!(
            MPV_RELEASE_URL.ends_with(MPV_RELEASE_FILE),
            "{MPV_RELEASE_URL} does not end with {MPV_RELEASE_FILE}"
        );
    }

    /// A pinned release, not a rolling one. Nightly builds carry a date and a build hash in the
    /// filename and are deleted after about a month, so a saved URL stops resolving.
    #[test]
    fn the_release_is_pinned_to_a_version() {
        assert!(
            MPV_RELEASE_URL.contains("/releases/download/v"),
            "not a versioned release asset: {MPV_RELEASE_URL}"
        );
        assert!(
            !MPV_RELEASE_URL.contains("/latest/"),
            "a floating URL cannot be checked against a fixed digest"
        );
    }

    /// The digest is what the checksum is compared against, so its shape has to be right or the
    /// comparison silently never matches.
    #[test]
    fn the_digest_is_a_sha256() {
        assert_eq!(MPV_RELEASE_SHA256.len(), 64, "{MPV_RELEASE_SHA256}");
        assert!(MPV_RELEASE_SHA256.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(
            MPV_RELEASE_SHA256.to_ascii_lowercase(),
            MPV_RELEASE_SHA256,
            "compared case-insensitively, but kept lowercase so the constant reads consistently"
        );
    }
}
