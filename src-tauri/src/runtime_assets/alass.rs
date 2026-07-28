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

use super::versions::{probe_version, VersionCache};
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
/// See `FFMPEG_VERSION` — same reasoning, same trust window.
static ALASS_VERSION: VersionCache = VersionCache::new();

pub(crate) fn detect_local_alass(settings: &AppSettings) -> AlassDetection {
    let asset_directory = PathBuf::from(&settings.asset_directory);
    if let Some(path) = collect_managed_alass_candidates(&asset_directory)
        .into_iter()
        .find(|candidate| managed_binary_is_present(candidate))
    {
        return AlassDetection {
            status: "ready".into(),
            executable_path: Some(path.display().to_string()),
            version: ALASS_VERSION.probe(|| probe_version(&path, "--version")),
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
/// **`--split-penalty` is deliberately left at alass's default.** Raising it to 1000 makes
/// every line shift by one offset, which is *exact* for a uniformly desynced file — but it
/// was measured to be catastrophic on the case alass actually exists for. Against a
/// reference desynced by a VARYING amount (+2s / +6s / +11s):
///   penalty 1000: 20.1s, 39.1s, 64.1s   (wrong by tens of seconds)
///   penalty 7:     5.0s, 20.0s, 40.1s   (near exact)
/// Tuning for the easy case would have broken the hard one, so the default stands.
///
/// Kept pure so the ordering is pinned by a test rather than by whoever edits it next.
pub(crate) fn alass_args(video: &str, incorrect_subtitles: &str, output: &str) -> Vec<String> {
    vec![
        // The one flag that matters, and the reason the first version of this synced badly.
        //
        // By default alass tries to detect a framerate difference between the reference and
        // the subtitles and rescales accordingly. That is meaningful for frame-based formats
        // and actively harmful for the time-based ones this app deals in (.srt/.ass/.vtt),
        // where it invents a ratio — 25/23.976 was observed on material where nothing of the
        // sort applied — and stretches the timings. Because it is a SCALE error the damage
        // grows with the timestamp, so subtitles look nearly right at the start of an episode
        // and drift steadily worse.
        //
        // Measured against a reference with real speech at 5s / 20s / 40s, uniformly
        // desynced by +5s:
        //   with guessing:    +49ms, +64ms, +84ms   (drifting)
        //   without guessing: +85ms, +85ms, +85ms   (constant, correctable)
        "--disable-fps-guessing".into(),
        // alass trades accuracy for speed by default. Measured on a reference with real
        // speech, uniformly desynced by +5s:
        //   default:  85ms residual on a 60s clip, exact on a 24-minute one
        //   `-O 0`:   exact on both, and on the varying-drift case it also corrected the
        //             last cue that the default left 85ms out
        // The cost was unmeasurable here — 4.2s either way on 24 minutes — but that
        // material is 24 identical repetitions and so unusually easy. If a real episode
        // ever syncs slowly, this is the knob that bought the accuracy.
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
