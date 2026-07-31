//! Cutting the clip the transcript viewer plays when you click a sentence.
//!
//! Playback used to seek the original file with `audio.currentTime`. For an MP3 the WebView
//! seeks through the file's Xing TOC — 100 entries for the whole recording, linearly
//! interpolated between them. On a constant-bitrate file that is exact. On a variable-bitrate
//! one it is an estimate, and on a real 550 s VBR recording from the library the estimate is
//! wrong by up to a second, in both directions, erratically:
//!
//! ```text
//!   seek to    lands at    error
//!     8.08 s     8.40 s    +320 ms   first syllable gone
//!   293.34 s   292.32 s   -1020 ms   plays the previous sentence
//!   494.46 s   493.58 s    -876 ms
//! ```
//!
//! That is why the same sentence sounded wrong in the viewer and perfect on the card: a mined
//! clip is cut by ffmpeg, which parses frames and lands sample-exact (measured: 0.0 ms error at
//! every position tested). So playback stops seeking and plays an ffmpeg cut instead — the same
//! cut, from the same window, that a card would get.
//!
//! Nothing here is shared with the miner beyond two pure functions. An earlier attempt at this
//! wrote previews into the mining temp directory using the miner's own naming, and mined cards
//! broke; the cause was never established. Sharing a directory with files that delete
//! themselves, and a name generator that hands out the first free name, is enough of a hazard
//! that this keeps its own of both — not as caution, but so the two paths have nothing to
//! collide over.

use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
};

use tauri::{Manager, Runtime};

use crate::{
    anki::{hide_command_window, slice_ffmpeg_args, ClipPadding},
    app_types::SharedPersistedState,
    runtime_assets::detect_local_ffmpeg,
};

/// Scratch space for previews, and ours alone.
///
/// Deliberately NOT the miner's `wonder-of-u` directory. That one is shared with files whose
/// lifetime is a mine, and whose names come from a generator that returns the first unused one —
/// so a preview left sitting there changes which name the next mine is given. A separate
/// directory means a preview cannot be seen by the mining path at all.
fn preview_temp_dir() -> Result<PathBuf, String> {
    let directory = std::env::temp_dir().join("wonder-of-u-preview");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create a temporary folder for playback: {error}"))?;
    Ok(directory)
}

/// Counter behind the preview filename.
///
/// A fixed name would be served from the WebView's cache on the second play — same URL, stale
/// bytes, and the wrong sentence heard. Each cut gets its own name so each gets its own URL.
static PREVIEW_COUNTER: AtomicU64 = AtomicU64::new(0);

/// The preview currently on disk, so the next one can remove it.
static CURRENT_PREVIEW: Mutex<Option<PathBuf>> = Mutex::new(None);

/// Deletes every file in the preview directory except the one just written.
///
/// Only one preview is ever playable, so anything else in here is finished with — including
/// whatever a crash or a force-quit left behind, which is why this sweeps the directory rather
/// than only unlinking the path it remembers. Safe precisely because the directory is ours: no
/// other part of the app writes here.
fn sweep_previews_except(keep: &Path) {
    let Ok(directory) = preview_temp_dir() else {
        return;
    };
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path != keep {
            let _ = fs::remove_file(path);
        }
    }
}

/// Cuts `[start_ms, end_ms]` (plus the miner's padding) out of `file_path` and returns the
/// clip's path for the frontend to play.
///
/// The padding comes from the same `clipPaddingMs` setting a mine uses, so what is heard here
/// and what lands on the card are the same window by construction rather than by agreement.
pub(crate) fn preview_segment_clip_inner<R: Runtime>(
    app: &tauri::AppHandle<R>,
    file_path: String,
    start_ms: u64,
    end_ms: u64,
) -> Result<String, String> {
    let audio_path = PathBuf::from(&file_path);
    if !audio_path.exists() {
        return Err(format!(
            "The audio is no longer at {}. It was moved or renamed.",
            audio_path.display()
        ));
    }

    let settings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the current app settings.".to_string())?;
        persisted.settings.clone()
    };

    let ffmpeg_path = detect_local_ffmpeg(&settings)
        .executable_path
        .map(PathBuf::from)
        .ok_or_else(|| "FFmpeg is required to play a sentence; install it in Setup.".to_string())?;

    let directory = preview_temp_dir()?;
    let sequence = PREVIEW_COUNTER.fetch_add(1, Ordering::Relaxed);
    let clip = directory.join(format!("segment-{sequence}.mp3"));

    let mut command = Command::new(&ffmpeg_path);
    hide_command_window(&mut command);
    if let Some(ffmpeg_directory) = ffmpeg_path.parent() {
        command.current_dir(ffmpeg_directory);
    }
    command.args(slice_ffmpeg_args(
        start_ms,
        end_ms,
        ClipPadding::symmetric(settings.anki.clip_padding_ms),
        &audio_path.display().to_string(),
        &clip.display().to_string(),
    ));

    let output = command.output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "FFmpeg is required to play a sentence; install it in Setup.".to_string()
        } else {
            format!("FFmpeg could not cut the sentence: {error}")
        }
    })?;

    // ffmpeg can exit 0 having written nothing, so the file is checked rather than the status.
    let clip_ready = output.status.success()
        && fs::metadata(&clip)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false);
    if !clip_ready {
        let _ = fs::remove_file(&clip);
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "The sentence could not be prepared for playback.".to_string()
        } else {
            format!("The sentence could not be prepared for playback: {stderr}")
        });
    }

    // After the new clip exists, never before: a failed cut must not leave playback with
    // nothing to fall back on.
    sweep_previews_except(&clip);
    if let Ok(mut current) = CURRENT_PREVIEW.lock() {
        *current = Some(clip.clone());
    }

    Ok(clip.display().to_string())
}

/// The directory the asset protocol must be allowed to serve previews from.
pub(crate) fn preview_scope_dir() -> Option<PathBuf> {
    preview_temp_dir().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The one property the previous attempt broke: a preview must never be written where the
    /// miner keeps its scratch files, because that directory's names are handed out by "first
    /// free name wins" and its files delete themselves.
    #[test]
    fn previews_live_apart_from_the_miners_scratch_files() {
        let previews = preview_temp_dir().expect("preview directory");
        let mining = std::env::temp_dir().join("wonder-of-u");

        assert_ne!(previews, mining);
        assert!(!previews.starts_with(&mining));
        assert!(!mining.starts_with(&previews));
    }

    /// A repeated play of the same sentence must not reuse a URL, or the WebView serves the
    /// previous bytes from cache.
    #[test]
    fn every_preview_gets_its_own_name() {
        let first = PREVIEW_COUNTER.fetch_add(1, Ordering::Relaxed);
        let second = PREVIEW_COUNTER.fetch_add(1, Ordering::Relaxed);

        assert_ne!(first, second);
    }

    /// The sweep keeps the live clip and removes the leftovers, including ones this process
    /// never wrote — a crash mid-session is the case that matters.
    #[test]
    fn the_sweep_keeps_only_the_live_clip() {
        let directory = preview_temp_dir().expect("preview directory");
        let keep = directory.join("segment-keep-me.mp3");
        let stale = directory.join("segment-stale.mp3");
        fs::write(&keep, b"live").expect("write live clip");
        fs::write(&stale, b"stale").expect("write stale clip");

        sweep_previews_except(&keep);

        assert!(keep.exists(), "the clip being played must survive");
        assert!(!stale.exists(), "everything else is finished with");
        let _ = fs::remove_file(&keep);
    }
}
