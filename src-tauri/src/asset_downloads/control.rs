use tauri::{AppHandle, Manager, Runtime};

use crate::app_types::{ModelDownloadControlState, ModelDownloadState};

use super::asset::{paused_message, AssetKind};
use super::transfer::update_model_download_snapshot;

/// Statuses that mean the download is over, whatever happens next.
fn is_terminal(status: &str) -> bool {
    matches!(status, "completed" | "failed" | "cancelled" | "idle")
}

/// What to call the download currently in the slot.
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
        snapshot.message = if is_paused {
            paused_message(download_label)
        } else {
            format!("Resuming the {download_label} download...")
        };
    })?;

    Ok(())
}

/// Cancels the running download, which also ends the queue.
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
