use tauri::{AppHandle, Manager, Runtime};

use crate::app_types::{ModelDownloadControlState, ModelDownloadState};

use super::transfer::update_model_download_snapshot;

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

    let download_label = {
        let snapshot = app
            .state::<ModelDownloadState>()
            .0
            .lock()
            .map_err(|_| "Could not inspect the current download state.".to_string())?
            .clone();
        match snapshot.kind.as_deref() {
            Some("runtime") => "Runtime",
            Some("ffmpeg") => "FFmpeg",
            Some("ytdlp") => "yt-dlp",
            _ => "Model",
        }
    };

    let resumed_label = download_label.to_ascii_lowercase();

    update_model_download_snapshot(app, |snapshot| {
        snapshot.status = if is_paused {
            "paused".into()
        } else {
            "downloading".into()
        };
        snapshot.message = if is_paused {
            format!("{download_label} download paused.")
        } else {
            format!("Resuming the {resumed_label} download...")
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

    let download_label = {
        let snapshot = app
            .state::<ModelDownloadState>()
            .0
            .lock()
            .map_err(|_| "Could not inspect the current download state.".to_string())?
            .clone();
        match snapshot.kind.as_deref() {
            Some("runtime") => "runtime",
            Some("ffmpeg") => "FFmpeg",
            Some("ytdlp") => "yt-dlp",
            _ => "model",
        }
    };

    update_model_download_snapshot(app, |snapshot| {
        snapshot.status = "cancelling".into();
        snapshot.message = format!("Cancelling the {download_label} download...");
    })?;

    Ok(())
}
