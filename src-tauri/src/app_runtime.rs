use std::{
    fs,
    path::Path,
    time::{SystemTime, UNIX_EPOCH},
};

use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::{
    app_config::APP_SNAPSHOT_EVENT,
    anki::known_words_snapshot_from_state,
    app_types::{
        AppBootstrap, AppPathsState, KnownWordsBuild, ModelDownloadState, SharedPersistedState,
        SharedShellState, ShellSnapshot, WhisperDetectionState,
    },
    runtime_assets::{
        detect_local_alass, detect_local_dictionary, detect_local_ffmpeg, detect_local_ytdlp,
    },
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

pub(crate) fn log_event<R: Runtime>(
    app: &AppHandle<R>,
    level: &str,
    event: &str,
    details: serde_json::Value,
) {
    let path = app.state::<AppPathsState>().inner().log_file.clone();
    crate::logging::write(&path, level, event, details);
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
    let alass_detection = detect_local_alass(&persisted.settings);
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
    let dictionary_detection = detect_local_dictionary(&persisted.settings);
    // Judged against the settings just read, rather than re-locking them: this runs
    // on every snapshot emit, and reading them twice is how the two halves of the
    // answer end up describing different moments.
    let known_words = known_words_snapshot_from_state(
        app,
        &KnownWordsBuild::from_anki_settings(&persisted.settings.anki),
    );
    let log_path = app
        .state::<AppPathsState>()
        .inner()
        .log_file
        .display()
        .to_string();

    // Asked of the same function transcription asks, with the detections just read — so the
    // checklist cannot describe a requirement the engine does not have, or miss one it does.
    let transcription_requirements = crate::recording_library::transcription::transcription_requirements(
        &persisted.settings,
        &whisper_detection,
        &ffmpeg_detection,
    );

    Ok(AppBootstrap {
        shell,
        settings: persisted.settings,
        recent_recordings: persisted.recent_recordings,
        watched_videos: persisted.watched_videos,
        whisper_detection,
        ffmpeg_detection,
        ytdlp_detection,
        alass_detection,
        model_download,
        dictionary_detection,
        known_words,
        transcription_requirements,
        log_path,
        logging_failure: crate::logging::failure(),
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

/// A plain-text block a user can paste into a bug report.
///
/// Deliberately not the log: this is the part a person will actually complete, and it answers
/// the questions every report starts with. The log file is the optional attachment beside it.
pub(crate) fn diagnostics_text<R: Runtime>(app: &AppHandle<R>) -> String {
    let bootstrap = build_app_bootstrap(app);
    let mut lines = vec![format!("Run: {}", crate::logging::run_id())];

    if let Some(fields) = crate::logging::environment().as_object() {
        for key in ["app", "os", "arch", "cpus", "webview"] {
            if let Some(value) = fields.get(key) {
                lines.push(format!("{key}: {}", value.to_string().trim_matches('"')));
            }
        }
    }

    match bootstrap {
        Ok(state) => {
            lines.push(format!(
                "Whisper: {} ({})",
                state.whisper_detection.status, state.settings.whisper.model_choice
            ));
            lines.push(format!("FFmpeg: {}", state.ffmpeg_detection.status));
            lines.push(format!("yt-dlp: {}", state.ytdlp_detection.status));
            lines.push(format!("alass: {}", state.alass_detection.status));
            lines.push(format!("Dictionary: {}", state.dictionary_detection.status));
            lines.push(format!(
                "Anki mapping: {}",
                if state.settings.anki.fields.transcription.is_empty() {
                    "not mapped"
                } else {
                    "mapped"
                }
            ));
            for requirement in &state.transcription_requirements {
                lines.push(format!(
                    "Requirement {}: {}",
                    requirement.id,
                    if requirement.ready { "ready" } else { "MISSING" }
                ));
            }
        }
        // Worth reporting rather than hiding: if this is what fails, it IS the bug.
        Err(error) => lines.push(format!("State could not be read: {error}")),
    }

    if let Some(reason) = crate::logging::failure() {
        lines.push(format!("Logging: UNAVAILABLE ({reason})"));
    }

    lines.join("\n")
}
