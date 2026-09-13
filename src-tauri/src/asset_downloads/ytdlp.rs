use std::path::{Path, PathBuf};

use tauri::{AppHandle, Runtime};

use crate::{
    app_config::YTDLP_RELEASE_DOWNLOAD_URL,
    runtime_assets::{managed_ytdlp_install_directory, verify_ytdlp_binary},
};

use super::asset::AssetKind;
use super::envelope::{AssetDownloadPlan, Installed};
use super::transfer::{
    asset_directory, ensure_directory_exists, verify_managed_binary_or_remove,
};

/// Where yt-dlp is installed: `<asset_dir>/yt-dlp/yt-dlp.exe`.
fn ytdlp_target_path(asset_directory: &Path) -> PathBuf {
    managed_ytdlp_install_directory(asset_directory).join("yt-dlp.exe")
}

/// Downloads the latest yt-dlp release into `<asset_dir>/yt-dlp/yt-dlp.exe`.
pub(super) fn ytdlp_plan<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<AssetDownloadPlan<R>, String> {
    let asset_directory = asset_directory(app)?;
    let install_directory = managed_ytdlp_install_directory(&asset_directory);
    ensure_directory_exists(&install_directory)?;
    let target_path = ytdlp_target_path(&asset_directory);

    let installed_path = target_path.clone();

    Ok(AssetDownloadPlan {
        kind: AssetKind::Ytdlp,
        slot_busy_message: "Another download is already in progress.".into(),
        shell_start_text: format!("Downloading yt-dlp to {}...", target_path.display()),
        starting_message: "Preparing the yt-dlp download...".into(),
        starting_target_path: target_path,
        cancelled_message: "yt-dlp download cancelled.".into(),
        cancelled_shell_text: "yt-dlp download cancelled.".into(),
        failed_message_prefix: "yt-dlp download failed".into(),
        failed_shell_prefix: "yt-dlp download failed".into(),
        success_log_event: "ytdlp.downloaded",
        failure_log_event: "ytdlp.download_failed",
        install: Box::new(move |context| {
            context.fetch(YTDLP_RELEASE_DOWNLOAD_URL, &installed_path, "yt-dlp")?;
            verify_managed_binary_or_remove(&installed_path, verify_ytdlp_binary)?;

            Ok(Installed {
                completed_message: "yt-dlp downloaded. You can now import from a link."
                    .into(),
                shell_success_text: format!(
                    "yt-dlp is ready at {}. You can import audio from a link.",
                    installed_path.display()
                ),
                log_details: serde_json::json!({
                    "ytdlpPath": installed_path.display().to_string()
                }),
                target_path: installed_path,
            })
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::ytdlp_target_path;
    use std::path::Path;

    /// yt-dlp installs as a bare binary under its own directory — no archive, and nothing in
    /// `downloads/`. Pinning the layout is what makes the next five conversions safe to
    /// compare against: each asset differs here and nowhere else.
    #[test]
    fn yt_dlp_installs_as_a_bare_binary_under_the_asset_directory() {
        let target = ytdlp_target_path(Path::new("C:/assets"));

        assert!(target.ends_with("yt-dlp/yt-dlp.exe"), "got {target:?}");
        assert!(target.starts_with("C:/assets"), "got {target:?}");
        // Not staged through `downloads/` the way the archive-based assets are.
        assert!(
            !target.components().any(|part| part.as_os_str() == "downloads"),
            "yt-dlp should install directly, not via downloads/: {target:?}"
        );
    }
}
