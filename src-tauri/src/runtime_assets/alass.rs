use std::{
    path::{Path, PathBuf},
    process::Command,
};

use crate::app_types::{AlassDetection, AppSettings};

use super::ytdlp::managed_binary_is_present;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const ALASS_ARCHIVE_ENTRY: &str = "bin/alass-cli.exe";

/// The directory the app downloads alass into: `<asset_dir>/alass`.
pub(crate) fn managed_alass_install_directory(asset_directory: &Path) -> PathBuf {
    asset_directory.join("alass")
}

/// Where the extracted binary lands. Flat, like yt-dlp: one file, no archive layout to
/// preserve, because everything else in the archive is deliberately discarded.
pub(crate) fn collect_managed_alass_candidates(asset_directory: &Path) -> Vec<PathBuf> {
    let install_directory = managed_alass_install_directory(asset_directory);
    vec![
        install_directory.join("alass-cli.exe"),
        install_directory.join("alass-cli"),
    ]
}

/// Picks `alass-cli.exe` out of the release archive.
pub(crate) fn alass_archive_entry(names: &[String]) -> Option<String> {
    names
        .iter()
        .find(|name| {
            let normalized = name.replace('\\', "/");
            normalized.ends_with(ALASS_ARCHIVE_ENTRY)
        })
        .cloned()
}

fn hide_command_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

pub(crate) fn verify_alass_binary(executable_path: &Path) -> Result<(), String> {
    let mut command = Command::new(executable_path);
    hide_command_window(&mut command);
    let output = command
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

/// Whether the managed alass binary is installed.
pub(crate) fn detect_local_alass(settings: &AppSettings) -> AlassDetection {
    let asset_directory = PathBuf::from(&settings.asset_directory);
    if let Some(path) = collect_managed_alass_candidates(&asset_directory)
        .into_iter()
        .find(|candidate| managed_binary_is_present(candidate))
    {
        return AlassDetection {
            status: "ready".into(),
            executable_path: Some(path.display().to_string()),
            message: "alass is ready. Out-of-sync subtitles can be aligned automatically."
                .into(),
        };
    }
    AlassDetection::default()
}

/// Builds the alass argument list.
pub(crate) fn alass_args(video: &str, incorrect_subtitles: &str, output: &str) -> Vec<String> {
    vec![
        "--disable-fps-guessing".into(),
        "--speed-optimization".into(),
        "0".into(),
        video.to_string(),
        incorrect_subtitles.to_string(),
        output.to_string(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cli_is_found_wherever_the_archive_puts_it() {
        let names = vec![
            "alass-windows64/".to_string(),
            "alass-windows64/bin/LICENSE.txt".to_string(),
            "alass-windows64/bin/alass-cli.exe".to_string(),
            "alass-windows64/ffmpeg/bin/ffmpeg.exe".to_string(),
        ];
        assert_eq!(
            alass_archive_entry(&names).as_deref(),
            Some("alass-windows64/bin/alass-cli.exe")
        );
    }

    #[test]
    fn the_bundled_ffmpeg_is_never_mistaken_for_the_cli() {
        let names = vec![
            "alass-windows64/ffmpeg/bin/ffmpeg.exe".to_string(),
            "alass-windows64/ffmpeg/bin/ffprobe.exe".to_string(),
        ];
        assert!(alass_archive_entry(&names).is_none());
    }

    #[test]
    fn backslash_archives_still_match() {
        let names = vec!["alass-windows64\\bin\\alass-cli.exe".to_string()];
        assert!(alass_archive_entry(&names).is_some());
    }

    #[test]
    fn the_video_is_the_reference_and_the_output_is_last() {
        // Order is the CLI's contract: <reference> <incorrect> <output>. Getting it wrong
        // would overwrite the input with a partially-written file.
        let args = alass_args("video.mkv", "wrong.srt", "fixed.srt");
        assert_eq!(
            args,
            vec![
                "--disable-fps-guessing",
                "--speed-optimization",
                "0",
                "video.mkv",
                "wrong.srt",
                "fixed.srt"
            ]
        );
    }

    /// Framerate guessing rescales the timings, so its error compounds with the timestamp —
    /// the difference between "slightly late" and "unusable by the end of the episode".
    /// It stays off; this fails if anyone drops the flag.
    #[test]
    fn framerate_guessing_is_always_disabled() {
        assert!(alass_args("a.mkv", "b.srt", "c.srt")
            .iter()
            .any(|arg| arg == "--disable-fps-guessing"));
    }
}
