//! Realigning an out-of-sync subtitle file against the video's own audio.
//!
//! Two tools, for two different faults. When a file is off by a constant, mpv's `sub-delay`
//! fixes it instantly and reversibly, with nothing written to disk — see
//! `set_watch_subtitle_delay`. When the drift *varies* across the episode (a different
//! release, missing ad breaks, a 25 vs 23.976 fps mismatch) no single offset works, and that
//! is what alass is for: it aligns against voice activity in the real audio.
//!
//! The corrected file is written **beside the original with a new name**, never over it. A
//! sync can be wrong — the reference audio may be music, or the wrong track — and the
//! original is the only way back.

use std::{
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use crate::app_types::AppSettings;
use crate::runtime_assets::{
    alass_args, collect_managed_alass_candidates, detect_local_ffmpeg, managed_binary_is_present,
};

use super::subtitles::ffprobe_path_for;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn hide_command_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

/// Where a synced file lands: `<stem>.synced.<ext>` beside the original.
///
/// Deliberately derived rather than a temp file — the user picked this subtitle from disk,
/// and the corrected version is something they will want to keep and re-open.
pub(crate) fn synced_subtitle_path(subtitle_path: &Path) -> PathBuf {
    let extension = subtitle_path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("srt");
    let stem = subtitle_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("subtitles");
    // Re-syncing an already-synced file would otherwise pile up `.synced.synced.srt`.
    let stem = stem.strip_suffix(".synced").unwrap_or(stem);
    subtitle_path.with_file_name(format!("{stem}.synced.{extension}"))
}

fn managed_alass_path(settings: &AppSettings) -> Option<PathBuf> {
    let asset_directory = PathBuf::from(&settings.asset_directory);
    collect_managed_alass_candidates(&asset_directory)
        .into_iter()
        .find(|candidate| managed_binary_is_present(candidate))
}

/// What alass reported doing, alongside where it wrote.
///
/// The summary is surfaced rather than swallowed because a sync can succeed and still be
/// wrong, and "shifted block of 3 subtitles by -0:00:05.000" is the difference between a
/// user who can see what happened and one staring at subtitles that are still off.
pub(crate) struct SyncOutcome {
    pub(crate) output_path: PathBuf,
    pub(crate) summary: String,
}

/// Pulls the human-readable lines out of alass's output, dropping its progress bar.
fn summarize_alass_output(stdout: &str, stderr: &str) -> String {
    let lines = stdout
        .lines()
        .chain(stderr.lines())
        .map(str::trim)
        .filter(|line| line.starts_with("shifted block") || line.starts_with("warn:"))
        .collect::<Vec<_>>();
    lines.join(" · ")
}

/// Aligns `subtitle_path` to `video_path`, returning where the corrected file was written.
pub(crate) fn sync_subtitles_with_alass(
    settings: &AppSettings,
    video_path: &Path,
    subtitle_path: &Path,
) -> Result<SyncOutcome, String> {
    if !video_path.exists() {
        return Err(format!("The video is no longer at {}", video_path.display()));
    }
    if !subtitle_path.exists() {
        return Err(format!(
            "The subtitle file is no longer at {}",
            subtitle_path.display()
        ));
    }

    let alass_path = managed_alass_path(settings)
        .ok_or_else(|| "alass is not installed yet; download it in Setup.".to_string())?;

    // alass shells out to ffmpeg to read the reference audio and finds it through these two
    // variables — the same mechanism the release's own `alass.bat` uses. Pointing them at
    // the app's managed ffmpeg is what lets us ship the 3.5 MB binary alone instead of the
    // ~70 MB of ffmpeg the archive carries.
    let ffmpeg = detect_local_ffmpeg(settings)
        .executable_path
        .ok_or_else(|| "FFmpeg is required to sync subtitles; install it in Setup.".to_string())?;
    let ffprobe = ffprobe_path_for(&ffmpeg);

    let output_path = synced_subtitle_path(subtitle_path);
    let args = alass_args(
        &video_path.display().to_string(),
        &subtitle_path.display().to_string(),
        &output_path.display().to_string(),
    );

    let mut command = Command::new(&alass_path);
    hide_command_window(&mut command);
    let output = command
        .env("ALASS_FFMPEG_PATH", &ffmpeg)
        .env("ALASS_FFPROBE_PATH", &ffprobe)
        .args(&args)
        .output()
        .map_err(|error| format!("alass could not be started: {error}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let detail = if stderr.is_empty() { stdout } else { stderr };
        return Err(if detail.is_empty() {
            "alass could not align these subtitles.".into()
        } else {
            format!("alass could not align these subtitles. {detail}")
        });
    }

    // alass exits 0 having written nothing when it cannot find enough speech to align
    // against, so success is confirmed by the file rather than by the status code.
    if !output_path.exists() {
        return Err("alass finished without writing a synced file — the video may have no speech to align against.".into());
    }

    let summary = summarize_alass_output(
        &String::from_utf8_lossy(&output.stdout),
        &String::from_utf8_lossy(&output.stderr),
    );
    Ok(SyncOutcome {
        output_path,
        summary,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_summary_keeps_the_shifts_and_drops_the_progress_bar() {
        let stdout = "synchronizing 'a.srt' to reference file 'v.mkv'...
             1 / 3 [====>----] 33.33 % 61728.40/s 0s
             shifted block of 3 subtitles with length 0:00:35.000 by -0:00:05.000
";
        let stderr = "warn: some subtitles now have negative timings
";
        let summary = summarize_alass_output(stdout, stderr);
        assert!(summary.contains("shifted block of 3 subtitles"));
        assert!(summary.contains("warn: some subtitles"));
        assert!(!summary.contains("%"));
        assert!(!summary.contains("synchronizing"));
    }

    #[test]
    fn the_synced_file_sits_beside_the_original() {
        // Never over it: a sync can align against the wrong thing, and the original is the
        // only way back.
        let path = synced_subtitle_path(Path::new(r"C:\anime\ep01.ja.srt"));
        assert_eq!(path, PathBuf::from(r"C:\anime\ep01.ja.synced.srt"));
    }

    #[test]
    fn re_syncing_does_not_stack_suffixes() {
        let once = synced_subtitle_path(Path::new("ep01.srt"));
        let twice = synced_subtitle_path(&once);
        assert_eq!(once, twice);
    }

    #[test]
    fn an_unusual_extension_is_preserved() {
        assert_eq!(
            synced_subtitle_path(Path::new("show.ass")),
            PathBuf::from("show.synced.ass")
        );
        // No extension at all still produces something openable.
        assert_eq!(
            synced_subtitle_path(Path::new("subs")),
            PathBuf::from("subs.synced.srt")
        );
    }
}
