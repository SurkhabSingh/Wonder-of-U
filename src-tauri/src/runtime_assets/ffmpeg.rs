use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use crate::app_types::{AppSettings, FfmpegDetection};

use super::ytdlp::managed_binary_is_present;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn managed_ffmpeg_root(asset_directory: &Path) -> PathBuf {
    asset_directory.join("ffmpeg-runtime")
}

pub(crate) fn managed_ffmpeg_install_directory(asset_directory: &Path) -> PathBuf {
    managed_ffmpeg_root(asset_directory).join("latest")
}

fn push_ffmpeg_candidate(candidates: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !candidates.iter().any(|existing| existing == &candidate) {
        candidates.push(candidate);
    }
}

fn push_ffmpeg_candidates_from_directory(candidates: &mut Vec<PathBuf>, directory: &Path) {
    if !directory.exists() {
        return;
    }

    push_ffmpeg_candidate(candidates, directory.join("ffmpeg.exe"));
    push_ffmpeg_candidate(candidates, directory.join("ffmpeg"));
    push_ffmpeg_candidate(candidates, directory.join("bin").join("ffmpeg.exe"));
    push_ffmpeg_candidate(candidates, directory.join("bin").join("ffmpeg"));

    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            push_ffmpeg_candidates_from_directory(candidates, &path);
        } else if path
            .file_name()
            .and_then(|value| value.to_str())
            .map(|value| value.eq_ignore_ascii_case("ffmpeg.exe") || value == "ffmpeg")
            .unwrap_or(false)
        {
            push_ffmpeg_candidate(candidates, path);
        }
    }
}

pub(crate) fn collect_managed_ffmpeg_candidates(asset_directory: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    push_ffmpeg_candidates_from_directory(
        &mut candidates,
        &managed_ffmpeg_install_directory(asset_directory),
    );
    candidates
}

fn hide_command_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

pub(crate) fn verify_ffmpeg_binary(executable_path: &Path) -> Result<(), String> {
    let mut command = Command::new(executable_path);
    hide_command_window(&mut command);
    let output = command
        .arg("-version")
        .output()
        .map_err(|error| error.to_string())?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Err(if stderr.is_empty() { stdout } else { stderr })
}

pub(crate) fn detect_local_ffmpeg(settings: &AppSettings) -> FfmpegDetection {
    let asset_directory = PathBuf::from(&settings.asset_directory);
    // Managed binary: trust its presence (a non-empty file the app installed and
    // verified at download time) instead of spawning `ffmpeg -version` on every
    // app-snapshot emit — see `managed_binary_is_present`.
    if let Some(managed_path) = collect_managed_ffmpeg_candidates(&asset_directory)
        .into_iter()
        .find(|candidate| managed_binary_is_present(candidate))
    {
        return FfmpegDetection {
            status: "ready".into(),
            executable_path: Some(managed_path.display().to_string()),
            managed: true,
            message:
                "App-managed FFmpeg is ready. Transcribed WAV recordings can be manually converted to MP3."
                    .into(),
        };
    }

    let path_candidate = PathBuf::from("ffmpeg");
    if verify_ffmpeg_binary(&path_candidate).is_ok() {
        return FfmpegDetection {
            status: "ready".into(),
            executable_path: Some("ffmpeg".into()),
            managed: false,
            message:
                "System FFmpeg is available. Transcribed WAV recordings can be manually converted to MP3."
                    .into(),
        };
    }

    FfmpegDetection::default()
}

#[cfg(test)]
mod tests {
    use super::{collect_managed_ffmpeg_candidates, managed_ffmpeg_install_directory};

    #[test]
    fn managed_ffmpeg_candidates_include_nested_binaries_without_duplicates() {
        let temp_dir = tempfile::tempdir().unwrap();
        let install_directory = managed_ffmpeg_install_directory(temp_dir.path());
        let nested_directory = install_directory.join("archive").join("bin");
        std::fs::create_dir_all(&nested_directory).unwrap();
        let nested_binary = nested_directory.join("ffmpeg.exe");
        std::fs::write(&nested_binary, b"test").unwrap();

        let candidates = collect_managed_ffmpeg_candidates(temp_dir.path());
        assert!(candidates.contains(&nested_binary));
        assert_eq!(
            candidates
                .iter()
                .filter(|candidate| *candidate == &nested_binary)
                .count(),
            1
        );
    }
}
