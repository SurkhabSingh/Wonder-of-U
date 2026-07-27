use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use crate::app_types::{AppSettings, MpvDetection};

use super::path_probe::PathProbeCache;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Whether `mpv` on PATH runs. Same reasoning as the yt-dlp probe: a PATH binary can
/// only be confirmed by spawning it, and detection runs on every app-snapshot emit, so
/// the result is cached.
static PATH_MPV_PROBE: PathProbeCache = PathProbeCache::new();

/// The directory the app downloads mpv into: `<asset_dir>/mpv`.
pub(crate) fn managed_mpv_install_directory(asset_directory: &Path) -> PathBuf {
    asset_directory.join("mpv")
}

fn push_mpv_candidate(candidates: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !candidates.iter().any(|existing| existing == &candidate) {
        candidates.push(candidate);
    }
}

/// Unlike yt-dlp, mpv ships as an ARCHIVE that unpacks to a folder tree, so the binary
/// can land either directly in the install directory or one level down depending on how
/// the archive was rooted. Both shapes are checked rather than assuming one.
pub(crate) fn collect_managed_mpv_candidates(asset_directory: &Path) -> Vec<PathBuf> {
    let install_directory = managed_mpv_install_directory(asset_directory);
    let mut candidates = Vec::new();
    push_mpv_candidate(&mut candidates, install_directory.join("mpv.exe"));
    push_mpv_candidate(&mut candidates, install_directory.join("mpv"));
    push_mpv_candidate(&mut candidates, install_directory.join("mpv").join("mpv.exe"));
    candidates
}

/// Where a user-installed mpv usually lives on Windows. Checked before falling back to
/// a bare `mpv` on PATH, because the common installers (winget, the official zip) do not
/// always put it there.
#[cfg(target_os = "windows")]
fn system_mpv_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    for variable in ["ProgramFiles", "ProgramFiles(x86)", "LOCALAPPDATA"] {
        if let Ok(root) = std::env::var(variable) {
            candidates.push(PathBuf::from(&root).join("mpv").join("mpv.exe"));
            candidates.push(
                PathBuf::from(&root)
                    .join("Programs")
                    .join("mpv")
                    .join("mpv.exe"),
            );
        }
    }
    candidates
}

#[cfg(not(target_os = "windows"))]
fn system_mpv_candidates() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/bin/mpv"),
        PathBuf::from("/usr/local/bin/mpv"),
        PathBuf::from("/opt/homebrew/bin/mpv"),
    ]
}

fn hide_command_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

/// A managed binary is trusted by existence — it lives where the app installed it and
/// was verified at download time — so a non-empty regular file there is ready without
/// spawning it. The non-empty check still rejects a truncated download.
pub(crate) fn managed_binary_is_present(candidate: &Path) -> bool {
    candidate
        .metadata()
        .map(|metadata| metadata.is_file() && metadata.len() > 0)
        .unwrap_or(false)
}

pub(crate) fn verify_mpv_binary(executable_path: &Path) -> Result<(), String> {
    let mut command = Command::new(executable_path);
    hide_command_window(&mut command);
    // `--no-config` so a user's mpv.conf cannot make a version probe fail (or, worse,
    // make it hang waiting on something).
    let output = command
        .arg("--no-config")
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

/// Finds mpv, preferring one the user already installed.
///
/// This is the opposite order to yt-dlp, deliberately. mpv is a video player people
/// configure — scripts, shaders, key bindings — and someone who has tuned theirs would
/// be annoyed to find the app quietly using a stock copy instead. The managed download
/// is the fallback for someone who has no mpv at all.
pub(crate) fn detect_local_mpv(settings: &AppSettings) -> MpvDetection {
    for candidate in system_mpv_candidates() {
        if managed_binary_is_present(&candidate) && verify_mpv_binary(&candidate).is_ok() {
            return MpvDetection {
                status: "ready".into(),
                executable_path: Some(candidate.display().to_string()),
                managed: false,
                message: "Your own mpv install will be used for Watch & Mine.".into(),
            };
        }
    }

    let path_candidate = PathBuf::from("mpv");
    if PATH_MPV_PROBE.binary_is_available(|| verify_mpv_binary(&path_candidate).is_ok()) {
        return MpvDetection {
            status: "ready".into(),
            executable_path: Some("mpv".into()),
            managed: false,
            message: "System mpv is available for Watch & Mine.".into(),
        };
    }

    let asset_directory = PathBuf::from(&settings.asset_directory);
    if let Some(managed_path) = collect_managed_mpv_candidates(&asset_directory)
        .into_iter()
        .find(|candidate| managed_binary_is_present(candidate))
    {
        return MpvDetection {
            status: "ready".into(),
            executable_path: Some(managed_path.display().to_string()),
            managed: true,
            message: "App-managed mpv is ready for Watch & Mine.".into(),
        };
    }

    MpvDetection::default()
}

#[cfg(test)]
mod tests {
    use super::{
        collect_managed_mpv_candidates, managed_binary_is_present, managed_mpv_install_directory,
    };
    use std::path::PathBuf;

    #[test]
    fn managed_binary_is_present_requires_a_non_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!managed_binary_is_present(dir.path()));
        assert!(!managed_binary_is_present(&dir.path().join("absent.exe")));
        // A zero-byte file is a truncated download, not a binary.
        let empty = dir.path().join("empty.exe");
        std::fs::write(&empty, b"").unwrap();
        assert!(!managed_binary_is_present(&empty));
        let real = dir.path().join("mpv.exe");
        std::fs::write(&real, b"MZ...").unwrap();
        assert!(managed_binary_is_present(&real));
    }

    #[test]
    fn managed_install_directory_is_the_mpv_subfolder() {
        let root = PathBuf::from("C:\\assets");
        assert_eq!(
            managed_mpv_install_directory(&root),
            PathBuf::from("C:\\assets\\mpv")
        );
    }

    #[test]
    fn managed_candidates_cover_the_nested_archive_layout() {
        let root = PathBuf::from("C:\\assets");
        let candidates = collect_managed_mpv_candidates(&root);
        assert!(candidates.contains(&PathBuf::from("C:\\assets\\mpv\\mpv.exe")));
        // mpv ships as an archive, so the binary may sit one level deeper than yt-dlp's.
        assert!(candidates.contains(&PathBuf::from("C:\\assets\\mpv\\mpv\\mpv.exe")));
        assert_eq!(candidates.len(), 3);
    }
}
