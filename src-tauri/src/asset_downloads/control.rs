use tauri::{AppHandle, Manager, Runtime};

use crate::app_types::{ModelDownloadControlState, ModelDownloadState};

use super::asset::{paused_message, AssetKind};
use super::transfer::update_model_download_snapshot;

/// Statuses that mean the download is over, whatever happens next.
///
/// Pause and cancel check `control.active` and *then* write the snapshot, and those are two
/// different locks taken at two different moments. A download that finishes in the gap resets
/// the control slot and writes its own final status — and the pause write, already past its
/// check, would land on top of it. The card then showed "Paused the alass download." at 100%
/// with live Resume and Cancel buttons over a download that had already finished and released
/// the slot, so pressing either could only answer "There is no active model download".
///
/// Rather than widen the locking, the rule is that a terminal status is final: the closure
/// below runs while the snapshot lock is held, and the worker's completed/failed/cancelled
/// write takes that same lock, so whichever arrives second sees the other and the pause simply
/// declines. Silently — a pause that arrives after the thing it would pause has finished is
/// not a failure worth telling anyone about.
fn is_terminal(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled" | "idle")
}

/// What to call the download currently in the slot.
///
/// Both commands here act on whichever download is running — they take no argument — so the
/// only way to name it is to read the snapshot. That lookup used to be a `match` on a bare
/// string, written twice with *different* arms: one returned "Runtime"/"Model", the other
/// "runtime"/"model", and neither listed alass or dictionary, so cancelling either of those
/// announced "Cancelling the model download". Going through `AssetKind` makes a missing arm
/// a compile error instead of a wrong sentence.
///
/// The fallback is for the genuinely empty slot only. Callers have already checked
/// `control.active`, so in practice a kind is always present by the time this runs.
fn active_download_label<R: Runtime>(app: &AppHandle<R>) -> Result<&'static str, String> {
    let kind = app
        .state::<ModelDownloadState>()
        .0
        .lock()
        .map_err(|_| "Could not inspect the current download state.".to_string())?
        .kind;
    Ok(kind.map_or("asset", AssetKind::label))
}

pub(crate) fn toggle_whisper_model_download_pause_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
    let control_state = app.state::<ModelDownloadControlState>();
    let mut control = control_state
        .control
        .lock()
        .map_err(|_| "Could not inspect the model download control state.".to_string())?;

    if !control.active {
        return Err("There is no active model download to pause or resume.".into());
    }

    control.paused = !control.paused;
    let is_paused = control.paused;
    drop(control);
    control_state.condvar.notify_all();

    let download_label = active_download_label(app)?;

    update_model_download_snapshot(app, |snapshot| {
        if is_terminal(&snapshot.status) {
            return;
        }
        snapshot.status = if is_paused {
            "paused".into()
        } else {
            "downloading".into()
        };
        // Both phrasings put the name mid-sentence, which is why one label can serve them.
        // The previous wording started the paused message with the name and so needed a
        // second, lowercased copy for the resumed one — the copy that read "ffmpeg".
        //
        // The paused sentence goes through  because the download thread
        // writes it too, the moment it notices; two spellings of it made the card flicker
        // between them.
        snapshot.message = if is_paused {
            paused_message(download_label)
        } else {
            format!("Resuming the {download_label} download...")
        };
    })?;

    Ok(())
}

pub(crate) fn cancel_whisper_model_download_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
    let control_state = app.state::<ModelDownloadControlState>();
    let mut control = control_state
        .control
        .lock()
        .map_err(|_| "Could not inspect the model download control state.".to_string())?;

    if !control.active {
        return Err("There is no active model download to cancel.".into());
    }

    control.cancel_requested = true;
    control.paused = false;
    drop(control);
    control_state.condvar.notify_all();

    let download_label = active_download_label(app)?;

    update_model_download_snapshot(app, |snapshot| {
        // Same race as pause: a cancel pressed as the download finishes must not reopen a
        // finished card into a "cancelling" one that never resolves.
        if is_terminal(&snapshot.status) {
            return;
        }
        snapshot.status = "cancelling".into();
        snapshot.message = format!("Cancelling the {download_label} download...");
    })?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::is_terminal;

    /// The four statuses a worker leaves behind when it is done with the slot. A pause or
    /// cancel landing on any of them is the race described above.
    #[test]
    fn a_finished_download_is_terminal() {
        assert!(is_terminal("completed"));
        assert!(is_terminal("failed"));
        assert!(is_terminal("cancelled"));
        assert!(is_terminal("idle"));
    }

    /// The in-flight statuses must stay writable, or Pause and Resume would do nothing at
    /// all — a fix worse than the bug.
    #[test]
    fn a_download_still_running_is_not_terminal() {
        assert!(!is_terminal("starting"));
        assert!(!is_terminal("downloading"));
        assert!(!is_terminal("paused"));
        assert!(!is_terminal("cancelling"));
    }
}
