use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::{
    app_config::APP_SNAPSHOT_EVENT,
    app_types::{
        AppBootstrap, AppPathsState, ModelDownloadState, SharedPersistedState, SharedShellState,
        ShellSnapshot, WhisperDetectionState,
    },
    runtime_assets::{detect_local_dictionary, detect_local_ffmpeg, detect_local_ytdlp},
};

pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

pub(crate) fn ensure_directory_exists(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| error.to_string())
}

pub(crate) fn append_structured_log(
    path: &Path,
    level: &str,
    event: &str,
    details: serde_json::Value,
) {
    let payload = serde_json::json!({
        "tsMs": now_ms(),
        "level": level,
        "event": event,
        "details": details
    });

    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{payload}");
    }
}

pub(crate) fn log_event<R: Runtime>(
    app: &AppHandle<R>,
    level: &str,
    event: &str,
    details: serde_json::Value,
) {
    let path = app.state::<AppPathsState>().inner().log_file.clone();
    append_structured_log(&path, level, event, details);
}

pub(crate) fn build_app_bootstrap<R: Runtime>(app: &AppHandle<R>) -> Result<AppBootstrap, String> {
    let shell = app
        .state::<SharedShellState>()
        .0
        .lock()
        .map_err(|_| "Could not read the shell state.".to_string())?
        .clone();
    let persisted = app
        .state::<SharedPersistedState>()
        .0
        .lock()
        .map_err(|_| "Could not read the app settings.".to_string())?
        .clone();
    let ffmpeg_detection = detect_local_ffmpeg(&persisted.settings);
    let ytdlp_detection = detect_local_ytdlp(&persisted.settings);
    let dictionary_detection = detect_local_dictionary(&persisted.settings);
    let whisper_detection = app
        .state::<WhisperDetectionState>()
        .0
        .lock()
        .map_err(|_| "Could not read the Whisper readiness state.".to_string())?
        .clone();
    let model_download = app
        .state::<ModelDownloadState>()
        .0
        .lock()
        .map_err(|_| "Could not read the model download state.".to_string())?
        .clone();
    let log_path = app
        .state::<AppPathsState>()
        .inner()
        .log_file
        .display()
        .to_string();

    Ok(AppBootstrap {
        shell,
        settings: persisted.settings,
        recent_recordings: persisted.recent_recordings,
        whisper_detection,
        ffmpeg_detection,
        ytdlp_detection,
        dictionary_detection,
        model_download,
        log_path,
    })
}

pub(crate) fn emit_app_snapshot<R: Runtime>(app: &AppHandle<R>) {
    if let Ok(snapshot) = build_app_bootstrap(app) {
        let _ = app.emit(APP_SNAPSHOT_EVENT, &snapshot);
    }
}

pub(crate) fn update_shell_snapshot<R: Runtime, F>(
    app: &AppHandle<R>,
    update: F,
) -> Result<(), String>
where
    F: FnOnce(&mut ShellSnapshot),
{
    let shell_state = app.state::<SharedShellState>();
    let mut shell = shell_state
        .0
        .lock()
        .map_err(|_| "Could not update the shell state.".to_string())?;
    update(&mut shell);
    drop(shell);
    emit_app_snapshot(app);
    Ok(())
}

pub(crate) fn setup_error(message: impl Into<String>) -> tauri::Error {
    let boxed_error: Box<dyn std::error::Error> = Box::new(std::io::Error::other(message.into()));
    tauri::Error::Setup(boxed_error.into())
}
