use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use tauri::{Manager, Runtime};

use crate::{
    anki::{hide_command_window, slice_ffmpeg_args, ClipPadding},
    app_types::SharedPersistedState,
    runtime_assets::detect_local_ffmpeg,
};

/// Scratch space for previews, and ours alone.
fn preview_temp_dir() -> Result<PathBuf, String> {
    let directory = std::env::temp_dir().join("wonder-of-u-preview");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create a temporary folder for playback: {error}"))?;
    Ok(directory)
}

/// Counter behind the preview filename.
static PREVIEW_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Deletes every file in the preview directory except the one just written.
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
