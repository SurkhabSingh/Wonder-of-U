//! alass — automatic subtitle synchronisation, as a managed binary.
//!
//! **Licence.** alass is GPL-3.0. It is invoked as a **separate process**, exactly as ffmpeg
//! already is, and none of its source is vendored or linked. That distinction is the whole
//! reason the design looks like this: linking it would put this app under GPL-3.0 too.
//! Do not "simplify" this into a library dependency.
//!
//! **Why only one file is extracted.** The official `alass-windows64.zip` is 26 MB and
//! unpacks to ~74 MB, because it carries its own complete ffmpeg — `alass-cli.exe` shells
//! out to ffmpeg to read the reference audio. This app already manages ffmpeg. The shipped
//! `alass.bat` shows how the binary finds it:
//!
//! ```text
//! set ALASS_FFMPEG_PATH=%~dp0ffmpeg\bin\ffmpeg.exe
//! set ALASS_FFPROBE_PATH=%~dp0ffmpeg\bin\ffprobe.exe
//! ```
//!
//! Two environment variables. So only `alass-cli.exe` (3.5 MB) is extracted and pointed at
//! the ffmpeg the rest of the app already uses — a twentieth of the disk, and one ffmpeg
//! version to reason about instead of two that can drift apart. Verified against the real
//! binary before this was written.

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

/// The only entry taken out of the release archive. Matched by suffix rather than by full
/// path so a future release that moves it within the zip still resolves.
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
///
/// Returns the entry path so the caller can read it; kept pure so the matching rule is
/// testable without downloading 26 MB.
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
///
/// Managed-only, unlike yt-dlp: there is no conventional system install of alass to probe
/// for on Windows, and spawning a guess on every snapshot emit would cost what the yt-dlp
/// probe cache exists to avoid.
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
///
/// Positional and order-dependent: `<reference> <incorrect> <output>`. The reference is the
/// **video**, because alass aligns against voice activity in the real audio — which is also
/// why it can fix drift that varies across an episode, where a constant `sub-delay` cannot.
///
/// Kept pure so the ordering is pinned by a test rather than by whoever edits it next.
pub(crate) fn alass_args(video: &str, incorrect_subtitles: &str, output: &str) -> Vec<String> {
    vec![
        // Without this, a subtitle that should start before zero is silently clamped to the
        // start of the file, which stacks several cues on top of each other. Letting the
        // negative timestamp through and rejecting it later is more honest than a file that
        // looks synchronised and is not.
        "--allow-negative-timestamps".into(),
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
        // The archive carries a whole ffmpeg; picking any of it would be 70 MB of a second
        // copy of something the app already manages.
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
                "--allow-negative-timestamps",
                "video.mkv",
                "wrong.srt",
                "fixed.srt"
            ]
        );
    }
}
