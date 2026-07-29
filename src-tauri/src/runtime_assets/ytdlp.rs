use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use crate::app_types::{AppSettings, YtdlpDetection};

use super::path_probe::PathProbeCache;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Whether `yt-dlp` on PATH runs. Unlike a managed binary it can only be confirmed by
/// spawning it, so the result is cached to keep that ~1s off every snapshot emit —
/// see `PathProbeCache`.
static PATH_YTDLP_PROBE: PathProbeCache = PathProbeCache::new();

/// The directory the app downloads yt-dlp into: `<asset_dir>/yt-dlp`.
pub(crate) fn managed_ytdlp_install_directory(asset_directory: &Path) -> PathBuf {
    asset_directory.join("yt-dlp")
}

fn push_ytdlp_candidate(candidates: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !candidates.iter().any(|existing| existing == &candidate) {
        candidates.push(candidate);
    }
}

/// The app-managed yt-dlp binary is always a bare `yt-dlp.exe` (or `yt-dlp`)
/// landed directly in the install directory — there is no archive to unpack, so
/// the candidate list is short and flat.
pub(crate) fn collect_managed_ytdlp_candidates(asset_directory: &Path) -> Vec<PathBuf> {
    let install_directory = managed_ytdlp_install_directory(asset_directory);
    let mut candidates = Vec::new();
    push_ytdlp_candidate(&mut candidates, install_directory.join("yt-dlp.exe"));
    push_ytdlp_candidate(&mut candidates, install_directory.join("yt-dlp"));
    candidates
}

fn hide_command_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

/// A managed binary is trusted by existence: it lives at a path the app installed to
/// and verified at download time, so a non-empty regular file there is treated as
/// ready WITHOUT spawning it. This matters because `detect_local_ytdlp` runs on every
/// app-snapshot emit (via `build_app_bootstrap`), and spawning the frozen-Python
/// `yt-dlp --version` costs ~1s a call — those per-emit spawns are what dragged the
/// YouTube completion path out for seconds and stalled the import queue. The
/// non-empty check still rejects a truncated/partial download.
pub(crate) fn managed_binary_is_present(candidate: &Path) -> bool {
    candidate
        .metadata()
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
}

pub(crate) fn verify_ytdlp_binary(executable_path: &Path) -> Result<(), String> {
    let mut command = Command::new(executable_path);
    hide_command_window(&mut command);
    // --ignore-config: a stray yt-dlp.conf must not influence even a version probe.
    let output = command
        .arg("--ignore-config")
        .arg("--version")
        .output()
        .map_err(|error| error.to_string())?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Err(if stderr.is_empty() { stdout } else { stderr })
}

pub(crate) fn detect_local_ytdlp(settings: &AppSettings) -> YtdlpDetection {
    let asset_directory = PathBuf::from(&settings.asset_directory);
    // Managed binary: trust its presence (see `managed_binary_is_present`) — no spawn.
    if let Some(managed_path) = collect_managed_ytdlp_candidates(&asset_directory)
        .into_iter()
        .find(|candidate| managed_binary_is_present(candidate))
    {
        return YtdlpDetection {
            status: "ready".into(),
            executable_path: Some(managed_path.display().to_string()),
            managed: true,
            message: "App-managed yt-dlp is ready. You can import audio from YouTube.".into(),
        };
    }

    // A PATH-discovered binary is at an unknown location, so it still needs a
    // `--version` probe to confirm it exists and actually runs — cached, because this
    // runs on every emit too.
    let path_candidate = PathBuf::from("yt-dlp");
    if PATH_YTDLP_PROBE.binary_is_available(|| verify_ytdlp_binary(&path_candidate).is_ok()) {
        return YtdlpDetection {
            status: "ready".into(),
            executable_path: Some("yt-dlp".into()),
            managed: false,
            message: "System yt-dlp is available. You can import audio from YouTube.".into(),
        };
    }

    YtdlpDetection::default()
}

#[cfg(test)]
mod tests {
    use super::{
        collect_managed_ytdlp_candidates, managed_binary_is_present,
        managed_ytdlp_install_directory,
    };
    use std::path::PathBuf;

    #[test]
    fn managed_binary_is_present_requires_a_non_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        // A directory is not a binary.
        assert!(!managed_binary_is_present(dir.path()));
        // A missing path is not present.
        assert!(!managed_binary_is_present(&dir.path().join("absent.exe")));
        // A zero-byte file (a truncated download) is rejected.
        let empty = dir.path().join("empty.exe");
        std::fs::write(&empty, b"").unwrap();
        assert!(!managed_binary_is_present(&empty));
        // A non-empty file at a managed path is trusted without spawning it.
        let real = dir.path().join("yt-dlp.exe");
        std::fs::write(&real, b"MZ...").unwrap();
        assert!(managed_binary_is_present(&real));
    }

    #[test]
    fn managed_install_directory_is_the_yt_dlp_subfolder() {
        let root = PathBuf::from("C:\\assets");
        assert_eq!(
            managed_ytdlp_install_directory(&root),
            PathBuf::from("C:\\assets\\yt-dlp")
        );
    }

    #[test]
    fn managed_candidates_cover_both_exe_and_bare_names_without_duplicates() {
        let root = PathBuf::from("C:\\assets");
        let candidates = collect_managed_ytdlp_candidates(&root);
        assert!(candidates.contains(&PathBuf::from("C:\\assets\\yt-dlp\\yt-dlp.exe")));
        assert!(candidates.contains(&PathBuf::from("C:\\assets\\yt-dlp\\yt-dlp")));
        assert_eq!(candidates.len(), 2);
    }
}
