mod anki;
mod app_state;
mod app_types;
mod asset_downloads;
mod recording;
mod recording_library;
mod runtime_assets;
mod transcription;

use anki::*;
use app_state::*;
use app_types::*;
use asset_downloads::*;
use recording_library::*;
use runtime_assets::*;

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use recording::capture_system_audio_loopback;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    webview::PageLoadEvent,
    AppHandle, Emitter, Manager, Runtime, WindowEvent,
};
#[cfg(desktop)]
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use transcription::{run_whisper_transcription, WhisperTranscriptionRequest};
#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE},
    System::Threading::CreateMutexW,
    UI::WindowsAndMessaging::{
        FindWindowW, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
    },
};
const START_SHORTCUT: &str = "Ctrl+Alt+R";
const STOP_SHORTCUT: &str = "Ctrl+Alt+S";
const SHOW_SHORTCUT: &str = "Ctrl+Alt+W";
const APP_TITLE: &str = "Wonder of U";
const APP_SNAPSHOT_EVENT: &str = "app://snapshot-changed";
const START_SHORTCUT_CANDIDATES: [&str; 3] = [START_SHORTCUT, "Ctrl+Alt+Shift+R", "Ctrl+Alt+F8"];
const STOP_SHORTCUT_CANDIDATES: [&str; 3] = [STOP_SHORTCUT, "Ctrl+Alt+Shift+S", "Ctrl+Alt+F9"];
const SHOW_SHORTCUT_CANDIDATES: [&str; 3] = [SHOW_SHORTCUT, "Ctrl+Alt+Shift+W", "Ctrl+Alt+F10"];
const RECOMMENDED_WHISPER_RUNTIME_VERSION: &str = "v1.8.4";
const RECOMMENDED_WHISPER_RUNTIME_FILE: &str = "whisper-bin-x64.zip";
const RECOMMENDED_FFMPEG_RUNTIME_FILE: &str = "ffmpeg-master-latest-win64-gpl-shared.zip";
const RECOMMENDED_FFMPEG_RUNTIME_URL: &str = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl-shared.zip";
#[tauri::command]
fn get_app_bootstrap(app: AppHandle) -> Result<AppBootstrap, String> {
    build_app_bootstrap(&app)
}

#[tauri::command]
fn download_recommended_whisper_model(app: AppHandle) -> Result<AppBootstrap, String> {
    download_recommended_whisper_model_inner(&app)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
fn download_recommended_whisper_runtime(app: AppHandle) -> Result<AppBootstrap, String> {
    download_recommended_whisper_runtime_inner(&app)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
fn download_whisper_runtime_version(
    app: AppHandle,
    runtime_version: String,
) -> Result<AppBootstrap, String> {
    download_whisper_runtime_version_inner(&app, &runtime_version)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
fn download_recommended_ffmpeg(app: AppHandle) -> Result<AppBootstrap, String> {
    download_recommended_ffmpeg_inner(&app)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
async fn check_whisper_runtime_update(app: AppHandle) -> Result<WhisperAssetUpdateResult, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        check_whisper_runtime_update_inner(&app_for_blocking)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn check_whisper_model_update(app: AppHandle) -> Result<WhisperAssetUpdateResult, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        check_whisper_model_update_inner(&app_for_blocking)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn toggle_whisper_model_download_pause(app: AppHandle) -> Result<AppBootstrap, String> {
    toggle_whisper_model_download_pause_inner(&app)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
fn cancel_whisper_model_download(app: AppHandle) -> Result<AppBootstrap, String> {
    cancel_whisper_model_download_inner(&app)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
async fn save_settings(app: AppHandle, settings: AppSettings) -> Result<AppBootstrap, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        save_settings_inner(&app_for_blocking, settings)?;
        build_app_bootstrap(&app_for_blocking)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn start_recording(app: AppHandle, requested_name: Option<String>) -> Result<AppBootstrap, String> {
    start_recording_inner(&app, requested_name)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
fn stop_recording(app: AppHandle) -> Result<AppBootstrap, String> {
    stop_recording_inner(&app)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
fn show_main_window(app: AppHandle) -> Result<(), String> {
    show_main_window_inner(&app).map_err(|error| error.to_string())
}

#[tauri::command]
fn hide_main_window(app: AppHandle) -> Result<(), String> {
    hide_main_window_inner(&app).map_err(|error| error.to_string())
}

#[tauri::command]
async fn load_anki_catalog(
    app: AppHandle,
    note_type: Option<String>,
) -> Result<AnkiCatalog, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        load_anki_catalog_inner(&app_for_blocking, note_type)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
fn play_recording(app: AppHandle, file_path: String) -> Result<(), String> {
    play_recording_inner(&app, &file_path)
}

#[tauri::command]
fn delete_recording(app: AppHandle, file_path: String) -> Result<AppBootstrap, String> {
    delete_recording_inner(&app, &file_path)?;
    build_app_bootstrap(&app)
}

#[tauri::command]
fn delete_recordings(
    app: AppHandle,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    delete_recordings_inner(&app, file_paths)
}

#[tauri::command]
async fn push_recordings_to_anki(
    app: AppHandle,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        push_recordings_to_anki_inner(&app_for_blocking, file_paths)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn push_recordings_to_anki_deck(
    app: AppHandle,
    file_paths: Vec<String>,
    deck_name: String,
) -> Result<RecordingBatchResult, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        push_recordings_to_anki_deck_inner(&app_for_blocking, file_paths, deck_name)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn translate_recordings(
    app: AppHandle,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        translate_recordings_inner(&app_for_blocking, file_paths)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn add_furigana_to_anki(
    app: AppHandle,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        add_furigana_to_anki_inner(&app_for_blocking, file_paths)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn transcribe_recordings(
    app: AppHandle,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        transcribe_recordings_inner(&app_for_blocking, file_paths)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
async fn convert_recordings_to_mp3(
    app: AppHandle,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let app_for_blocking = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        convert_recordings_to_mp3_inner(&app_for_blocking, file_paths)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn append_structured_log(path: &Path, level: &str, event: &str, details: serde_json::Value) {
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

fn ensure_directory_exists(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| error.to_string())
}

fn apply_launch_at_login_setting<R: Runtime>(
    app: &AppHandle<R>,
    enabled: bool,
) -> Result<bool, String> {
    #[cfg(desktop)]
    {
        let autostart_manager = app.autolaunch();
        if enabled {
            autostart_manager
                .enable()
                .map_err(|error| error.to_string())?;
        } else if let Err(error) = autostart_manager.disable() {
            let message = error.to_string();
            if !is_autostart_not_found_error(&message) {
                return Err(message);
            }
        }

        return match autostart_manager.is_enabled() {
            Ok(actual_state) => Ok(actual_state),
            Err(error) => {
                let message = error.to_string();
                if !enabled && is_autostart_not_found_error(&message) {
                    Ok(false)
                } else {
                    Err(message)
                }
            }
        };
    }

    #[cfg(not(desktop))]
    {
        Ok(enabled)
    }
}

fn is_autostart_not_found_error(message: &str) -> bool {
    let normalized = message.to_ascii_lowercase();
    normalized.contains("os error 2")
        || normalized.contains("the system cannot find the file")
        || normalized.contains("cannot find the file specified")
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

fn save_settings_inner<R: Runtime>(
    app: &AppHandle<R>,
    settings: AppSettings,
) -> Result<(), String> {
    let paths = app.state::<AppPathsState>().inner().clone();
    let previous_settings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read current app settings.".to_string())?;
        persisted.settings.clone()
    };
    let mut normalized =
        normalize_settings(app, &paths, settings).map_err(|error| error.to_string())?;
    ensure_directory_exists(Path::new(&normalized.output_directory))?;
    ensure_directory_exists(Path::new(&normalized.asset_directory))?;

    let launch_at_login_changed = normalized.launch_at_login != previous_settings.launch_at_login;
    let refresh_whisper_detection =
        whisper_detection_inputs_changed(&previous_settings, &normalized);

    normalized.launch_at_login = if launch_at_login_changed {
        apply_launch_at_login_setting(app, normalized.launch_at_login)?
    } else {
        previous_settings.launch_at_login
    };

    let snapshot = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update app settings.".to_string())?;
        persisted.settings = normalized.clone();
        persisted.clone()
    };

    write_persisted_data(app, &snapshot)?;
    log_event(
        app,
        "INFO",
        "settings.saved",
        serde_json::json!({
            "outputDirectory": normalized.output_directory,
            "assetDirectory": normalized.asset_directory,
            "whisperCliPath": normalized.whisper.cli_path,
            "whisperRuntimeVersion": normalized.whisper.runtime_version,
            "whisperModelPath": normalized.whisper.model_path,
            "whisperModelChoice": normalized.whisper.model_choice,
            "whisperLanguage": normalized.whisper.language,
            "ankiDeckName": normalized.anki.deck_name,
            "ankiNoteType": normalized.anki.note_type
        }),
    );

    if refresh_whisper_detection {
        let _ = refresh_whisper_detection_state(app)?;
    } else {
        emit_app_snapshot(app);
    }

    Ok(())
}

fn insert_recent_recording<R: Runtime>(
    app: &AppHandle<R>,
    recent_recording: RecentRecording,
) -> Result<(), String> {
    let persisted_snapshot = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update the recording history.".to_string())?;
        persisted.recent_recordings.insert(0, recent_recording);

        persisted.clone()
    };

    write_persisted_data(app, &persisted_snapshot)
}

fn start_recording_inner<R: Runtime>(
    app: &AppHandle<R>,
    requested_name: Option<String>,
) -> Result<(), String> {
    {
        let shell_state = app.state::<SharedShellState>();
        let shell = shell_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the shell state.".to_string())?;
        if shell.phase != "idle" && shell.phase != "error" {
            return Err("The app is still busy with the previous recording task.".into());
        }
    }

    {
        let recorder_state = app.state::<RecorderState>();
        let recorder = recorder_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the recorder state.".to_string())?;
        if recorder.is_some() {
            return Err("A recording is already in progress.".into());
        }
    }

    let started_at_ms = now_ms();
    let (output_path, display_name, persisted_snapshot) = {
        let paths = app.state::<AppPathsState>().inner().clone();
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not prepare the recording state.".to_string())?;
        persisted.settings = normalize_settings(app, &paths, persisted.settings.clone())
            .map_err(|error| error.to_string())?;

        let output_directory = PathBuf::from(&persisted.settings.output_directory);
        ensure_directory_exists(&output_directory)?;

        let file_stem = next_recording_stem(&mut persisted, requested_name.as_deref());
        let output_path = unique_wav_path(&output_directory, &file_stem);
        let snapshot = persisted.clone();
        (output_path, file_stem, snapshot)
    };

    write_persisted_data(app, &persisted_snapshot)?;
    update_shell_snapshot(app, |shell| {
        shell.phase = "recording".into();
        shell.status_text = format!("Starting system audio capture to {}", output_path.display());
        shell.started_at_ms = Some(started_at_ms);
        shell.current_recording_name = Some(display_name.clone());
        shell.last_output_path = None;
        shell.last_transcript_path = None;
        shell.transition_count += 1;
    })?;

    let stop_signal = Arc::new(AtomicBool::new(false));
    let log_path = app.state::<AppPathsState>().inner().log_file.clone();
    let output_path_for_worker = output_path.clone();
    let display_name_for_worker = display_name.clone();
    let stop_signal_for_worker = stop_signal.clone();
    let worker = std::thread::Builder::new()
        .name("system-audio-recorder".into())
        .spawn(move || {
            capture_system_audio_loopback(
                output_path_for_worker,
                display_name_for_worker,
                stop_signal_for_worker,
                log_path,
                started_at_ms,
            )
        })
        .map_err(|error| {
            let message = error.to_string();
            let _ = update_shell_snapshot(app, |shell| {
                shell.phase = "error".into();
                shell.status_text = message.clone();
                shell.started_at_ms = None;
                shell.current_recording_name = None;
                shell.transition_count += 1;
            });
            message
        })?;

    {
        let recorder_state = app.state::<RecorderState>();
        let mut recorder = recorder_state
            .0
            .lock()
            .map_err(|_| "Could not store the active recorder.".to_string())?;
        *recorder = Some(ActiveRecording {
            stop_signal,
            worker,
        });
    }

    log_event(
        app,
        "INFO",
        "recording.start_requested",
        serde_json::json!({
            "outputPath": output_path.display().to_string(),
            "displayName": display_name
        }),
    );

    update_shell_snapshot(app, |shell| {
        shell.phase = "recording".into();
        shell.status_text = format!("Recording system audio to {}", output_path.display());
        shell.started_at_ms = Some(started_at_ms);
        shell.current_recording_name = Some(display_name.clone());
        shell.last_output_path = None;
        shell.last_transcript_path = None;
        shell.transition_count += 1;
    })?;
    Ok(())
}

fn finalize_recording_pipeline<R: Runtime>(
    app: AppHandle<R>,
    active: ActiveRecording,
) -> Result<(), String> {
    active.stop_signal.store(true, Ordering::SeqCst);
    let result = active
        .worker
        .join()
        .map_err(|_| "The recording worker thread panicked.".to_string())?;

    match result {
        Ok(capture) => {
            let mut recent_recording = RecentRecording {
                file_name: capture
                    .output_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("recording.wav")
                    .to_string(),
                file_path: capture.output_path.display().to_string(),
                transcript_path: None,
                transcript_language: None,
                translation_path: None,
                anki_note_id: None,
                anki_deck_name: None,
                anki_note_type: None,
                audio_deleted: false,
                duration_ms: capture.duration_ms,
                bytes_written: capture.bytes_written,
                created_at_ms: capture.created_at_ms,
            };

            log_event(
                &app,
                "INFO",
                "recording.saved",
                serde_json::json!({
                    "filePath": recent_recording.file_path,
                    "displayName": capture.display_name,
                    "durationMs": recent_recording.duration_ms,
                    "bytesWritten": recent_recording.bytes_written
                }),
            );

            let settings = {
                let persisted_state = app.state::<SharedPersistedState>();
                let persisted = persisted_state
                    .0
                    .lock()
                    .map_err(|_| "Could not inspect transcription settings.".to_string())?;
                persisted.settings.clone()
            };

            if settings.features.transcription {
                let whisper_detection = refresh_whisper_detection_state(&app)?;

                if whisper_detection.status == "ready" {
                    update_shell_snapshot(&app, |shell| {
                        shell.phase = "transcribing".into();
                        shell.status_text = format!(
                            "Saved {}. Running Whisper transcription...",
                            recent_recording.file_name
                        );
                        shell.started_at_ms = None;
                        shell.current_recording_name = None;
                        shell.last_output_path = Some(recent_recording.file_path.clone());
                    })?;

                    match run_whisper_transcription(&WhisperTranscriptionRequest {
                        cli_path: PathBuf::from(
                            whisper_detection
                                .executable_path
                                .clone()
                                .unwrap_or_default(),
                        ),
                        model_path: PathBuf::from(
                            whisper_detection.model_path.clone().unwrap_or_default(),
                        ),
                        audio_path: PathBuf::from(&recent_recording.file_path),
                        language: settings.whisper.language.clone(),
                    }) {
                        Ok(result) => {
                            let mut transcript_path = result.transcript_path;
                            let mut audio_path = PathBuf::from(&recent_recording.file_path);

                            match rename_recording_outputs_from_transcript(
                                &audio_path,
                                &transcript_path,
                                recent_recording.created_at_ms,
                            ) {
                                Ok((renamed_audio_path, renamed_transcript_path)) => {
                                    audio_path = renamed_audio_path;
                                    transcript_path = renamed_transcript_path;
                                }
                                Err(error) => {
                                    log_event(
                                        &app,
                                        "ERROR",
                                        "recording.rename_from_transcript_failed",
                                        serde_json::json!({
                                            "audioPath": recent_recording.file_path,
                                            "message": error
                                        }),
                                    );
                                }
                            }

                            recent_recording.file_name = audio_path
                                .file_name()
                                .and_then(|name| name.to_str())
                                .unwrap_or("recording.wav")
                                .to_string();
                            recent_recording.file_path = audio_path.display().to_string();
                            recent_recording.transcript_path =
                                Some(transcript_path.display().to_string());
                            recent_recording.transcript_language =
                                derive_transcript_language_from_path(
                                    &transcript_path,
                                    &settings.whisper.language,
                                );
                            recent_recording.bytes_written = fs::metadata(&audio_path)
                                .map(|metadata| metadata.len())
                                .unwrap_or(recent_recording.bytes_written);

                            insert_recent_recording(&app, recent_recording.clone())?;

                            log_event(
                                &app,
                                "INFO",
                                "transcription.saved",
                                serde_json::json!({
                                    "audioPath": recent_recording.file_path,
                                    "transcriptPath": recent_recording.transcript_path
                                }),
                            );

                            update_shell_snapshot(&app, |shell| {
                                shell.phase = "idle".into();
                                shell.status_text =
                                    format!("Saved {} and transcript.", recent_recording.file_name);
                                shell.started_at_ms = None;
                                shell.current_recording_name = None;
                                shell.last_output_path = Some(recent_recording.file_path.clone());
                                shell.last_transcript_path =
                                    recent_recording.transcript_path.clone();
                                shell.transition_count += 1;
                            })?;
                            return Ok(());
                        }
                        Err(error) => {
                            log_event(
                                &app,
                                "ERROR",
                                "transcription.failed",
                                serde_json::json!({
                                    "audioPath": recent_recording.file_path,
                                    "message": error
                                }),
                            );
                            insert_recent_recording(&app, recent_recording.clone())?;
                            update_shell_snapshot(&app, |shell| {
                                shell.phase = "idle".into();
                                shell.status_text = format!(
                                    "Saved {}. Whisper transcription failed: {}",
                                    recent_recording.file_name, error
                                );
                                shell.started_at_ms = None;
                                shell.current_recording_name = None;
                                shell.last_output_path = Some(recent_recording.file_path.clone());
                                shell.last_transcript_path = None;
                                shell.transition_count += 1;
                            })?;
                            return Ok(());
                        }
                    }
                }

                insert_recent_recording(&app, recent_recording.clone())?;
                update_shell_snapshot(&app, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = format!(
                        "Saved {}. Whisper is not ready yet: {}",
                        recent_recording.file_name, whisper_detection.message
                    );
                    shell.started_at_ms = None;
                    shell.current_recording_name = None;
                    shell.last_output_path = Some(recent_recording.file_path.clone());
                    shell.last_transcript_path = None;
                    shell.transition_count += 1;
                })?;
                return Ok(());
            }

            insert_recent_recording(&app, recent_recording.clone())?;
            update_shell_snapshot(&app, |shell| {
                shell.phase = "idle".into();
                shell.status_text = format!("Saved {}", recent_recording.file_name);
                shell.started_at_ms = None;
                shell.current_recording_name = None;
                shell.last_output_path = Some(recent_recording.file_path.clone());
                shell.last_transcript_path = None;
                shell.transition_count += 1;
            })?;
        }
        Err(error) => {
            log_event(
                &app,
                "ERROR",
                "recording.failed",
                serde_json::json!({ "message": error }),
            );
            update_shell_snapshot(&app, |shell| {
                shell.phase = "error".into();
                shell.status_text = error.clone();
                shell.started_at_ms = None;
                shell.current_recording_name = None;
                shell.last_transcript_path = None;
            })?;
            return Err(error);
        }
    }

    Ok(())
}

fn stop_recording_inner<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    {
        let shell_state = app.state::<SharedShellState>();
        let shell = shell_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the shell state.".to_string())?;
        if shell.phase == "saving" || shell.phase == "transcribing" {
            return Err("The previous recording is still being finalized.".into());
        }
    }

    let active = {
        let recorder_state = app.state::<RecorderState>();
        let mut recorder = recorder_state
            .0
            .lock()
            .map_err(|_| "Could not access the recorder state.".to_string())?;
        recorder
            .take()
            .ok_or_else(|| "No recording is currently running.".to_string())?
    };

    update_shell_snapshot(app, |shell| {
        shell.phase = "saving".into();
        shell.status_text = "Stopping capture and saving the WAV file...".into();
        shell.started_at_ms = None;
        shell.transition_count += 1;
    })?;

    let app_handle = app.clone();
    std::thread::Builder::new()
        .name("recording-finalizer".into())
        .spawn(move || {
            if let Err(error) = finalize_recording_pipeline(app_handle.clone(), active) {
                log_event(
                    &app_handle,
                    "ERROR",
                    "recording.finalize_failed",
                    serde_json::json!({ "message": error }),
                );
                let _ = update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "error".into();
                    shell.status_text = error;
                    shell.started_at_ms = None;
                    shell.current_recording_name = None;
                    shell.last_transcript_path = None;
                    shell.transition_count += 1;
                });
            }
        })
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn show_main_window_inner<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        window.show()?;
        window.unminimize()?;
        window.set_focus()?;
    }

    Ok(())
}

fn hide_main_window_inner<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        window.hide()?;
    }

    Ok(())
}

fn resolve_startup_visibility<R: Runtime>(
    app: &AppHandle<R>,
    startup_visibility: &StartupVisibility,
) {
    if !startup_visibility.initialized.load(Ordering::Acquire)
        || !startup_visibility.page_loaded.load(Ordering::Acquire)
        || startup_visibility.resolved.swap(true, Ordering::AcqRel)
    {
        return;
    }

    if !startup_visibility.start_minimized.load(Ordering::Acquire) {
        let _ = show_main_window_inner(app);
    }
}

fn setup_error(message: impl Into<String>) -> tauri::Error {
    let boxed_error: Box<dyn std::error::Error> = Box::new(std::io::Error::new(
        std::io::ErrorKind::Other,
        message.into(),
    ));
    tauri::Error::Setup(boxed_error.into())
}

#[cfg(target_os = "windows")]
struct SingleInstanceGuard {
    handle: HANDLE,
}

#[cfg(target_os = "windows")]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
fn current_launch_should_focus_existing_instance() -> bool {
    !std::env::args().any(|argument| argument == "--autostart")
}

#[cfg(target_os = "windows")]
fn focus_existing_instance_window() {
    let window_title = wide_null(APP_TITLE);

    unsafe {
        let window = FindWindowW(std::ptr::null(), window_title.as_ptr());
        if window.is_null() {
            return;
        }

        if IsIconic(window) != 0 {
            ShowWindow(window, SW_RESTORE);
        } else {
            ShowWindow(window, SW_SHOW);
        }

        SetForegroundWindow(window);
    }
}

#[cfg(target_os = "windows")]
fn acquire_single_instance_or_exit() -> Option<SingleInstanceGuard> {
    let mutex_name = wide_null("Local\\com.wonderofu.desktop.single-instance");
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
    if handle.is_null() {
        return None;
    }

    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        if current_launch_should_focus_existing_instance() {
            focus_existing_instance_window();
        }

        unsafe {
            CloseHandle(handle);
        }
        std::process::exit(0);
    }

    Some(SingleInstanceGuard { handle })
}

#[cfg(not(target_os = "windows"))]
fn acquire_single_instance_or_exit() {}

fn handle_shortcut<R: Runtime>(app: &AppHandle<R>, action: HotkeyAction, shortcut: &str) {
    let _ = update_shell_snapshot(app, |shell| {
        shell.last_shortcut = Some(shortcut.to_string());
    });

    let action_result = match action {
        HotkeyAction::Start => start_recording_inner(app, None),
        HotkeyAction::Stop => stop_recording_inner(app),
        HotkeyAction::ShowWindow => show_main_window_inner(app).map_err(|error| error.to_string()),
    };

    if let Err(error) = action_result {
        log_event(
            app,
            "ERROR",
            "hotkey.failed",
            serde_json::json!({
                "shortcut": shortcut,
                "message": error
            }),
        );
        let _ = update_shell_snapshot(app, |shell| {
            shell.phase = "error".into();
            shell.status_text = error.clone();
            shell.started_at_ms = None;
            shell.current_recording_name = None;
        });
    }
}

fn register_hotkey<R: Runtime>(
    app: &AppHandle<R>,
    action: HotkeyAction,
    label: &str,
    candidates: &[&'static str],
) -> Result<(String, Option<String>), String> {
    let global_shortcut = app.global_shortcut();
    let mut last_error = None;

    for candidate in candidates {
        let registered_shortcut = *candidate;
        match global_shortcut.on_shortcut(registered_shortcut, move |app, _shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }

            handle_shortcut(app, action, registered_shortcut);
        }) {
            Ok(()) => {
                let warning = if registered_shortcut == candidates[0] {
                    None
                } else {
                    Some(format!(
                        "{label} hotkey moved to {registered_shortcut} because {primary} was unavailable.",
                        primary = candidates[0]
                    ))
                };

                return Ok((registered_shortcut.to_string(), warning));
            }
            Err(error) => last_error = Some(error.to_string()),
        }
    }

    Ok((
        "Unavailable".into(),
        Some(format!(
            "{label} hotkey could not be registered. Tried: {}. {}",
            candidates.join(", "),
            last_error.unwrap_or_else(|| "The operating system rejected every candidate.".into())
        )),
    ))
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let _single_instance_guard = acquire_single_instance_or_exit();
    let startup_visibility = Arc::new(StartupVisibility::default());
    let setup_visibility = Arc::clone(&startup_visibility);
    let page_load_visibility = Arc::clone(&startup_visibility);

    tauri::Builder::default()
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--autostart"])
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(move |app| {
            let app_handle = app.handle().clone();
            let paths = build_app_paths(&app_handle)?;
            let persisted = load_persisted_data(&app_handle, &paths)?;

            app.manage(paths.clone());
            app.manage(SharedPersistedState(Mutex::new(persisted)));
            app.manage(SharedShellState(Mutex::new(ShellSnapshot::default())));
            app.manage(WhisperDetectionState(Mutex::new(
                WhisperDetection::default(),
            )));
            app.manage(ModelDownloadState(Mutex::new(
                ModelDownloadSnapshot::default(),
            )));
            app.manage(ModelDownloadControlState {
                control: Mutex::new(ModelDownloadControl::default()),
                condvar: Condvar::new(),
            });
            app.manage(RecorderState(Mutex::new(None)));

            let mut startup_warnings = Vec::new();

            {
                let persisted_state = app.state::<SharedPersistedState>();
                let mut persisted = persisted_state
                    .0
                    .lock()
                    .map_err(|_| setup_error("Could not initialize persisted app state."))?;

                match apply_launch_at_login_setting(&app_handle, persisted.settings.launch_at_login)
                {
                    Ok(actual_state) => {
                        persisted.settings.launch_at_login = actual_state;
                    }
                    Err(error) => {
                        persisted.settings.launch_at_login = false;
                        startup_warnings.push(format!(
                            "Launch-at-login could not be synchronized. {error}"
                        ));
                    }
                }

                let snapshot = persisted.clone();
                drop(persisted);
                write_persisted_data(&app_handle, &snapshot).map_err(setup_error)?;
            }

            append_structured_log(
                &paths.log_file,
                "INFO",
                "app.startup",
                serde_json::json!({
                    "dataDir": paths.data_dir.display().to_string(),
                    "stateFile": paths.state_file.display().to_string()
                }),
            );

            if let Err(error) = refresh_whisper_detection_state(&app_handle) {
                startup_warnings.push(format!(
                    "Whisper readiness could not be initialized cleanly. {error}"
                ));
            }

            let show_item = MenuItem::with_id(app, "show", "Open Wonder of U", true, None::<&str>)?;
            let hide_item = MenuItem::with_id(app, "hide", "Hide Window", true, None::<&str>)?;
            let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show_item, &hide_item, &quit_item])?;

            let mut tray_builder = TrayIconBuilder::new().tooltip(APP_TITLE).menu(&menu);
            if let Some(icon) = app.default_window_icon().cloned() {
                tray_builder = tray_builder.icon(icon);
            }

            tray_builder
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => {
                        let _ = show_main_window_inner(app);
                    }
                    "hide" => {
                        let _ = hide_main_window_inner(app);
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        let _ = show_main_window_inner(tray.app_handle());
                    }
                })
                .build(app)?;

            if let Some(window) = app.get_webview_window("main") {
                let app_handle = app.handle().clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = hide_main_window_inner(&app_handle);
                    }
                });
            }

            let (start_binding, start_warning) = register_hotkey(
                &app.handle(),
                HotkeyAction::Start,
                "Start",
                &START_SHORTCUT_CANDIDATES,
            )
            .map_err(setup_error)?;
            let (stop_binding, stop_warning) = register_hotkey(
                &app.handle(),
                HotkeyAction::Stop,
                "Stop",
                &STOP_SHORTCUT_CANDIDATES,
            )
            .map_err(setup_error)?;
            let (show_binding, show_warning) = register_hotkey(
                &app.handle(),
                HotkeyAction::ShowWindow,
                "Show window",
                &SHOW_SHORTCUT_CANDIDATES,
            )
            .map_err(setup_error)?;

            let mut warnings = Vec::new();
            if let Some(warning) = start_warning {
                warnings.push(warning);
            }
            if let Some(warning) = stop_warning {
                warnings.push(warning);
            }
            if let Some(warning) = show_warning {
                warnings.push(warning);
            }
            warnings.extend(startup_warnings);

            {
                let shell_state = app.state::<SharedShellState>();
                let mut shell = shell_state
                    .0
                    .lock()
                    .map_err(|_| setup_error("Could not initialize shell state."))?;
                shell.hotkeys.start = start_binding;
                shell.hotkeys.stop = stop_binding;
                shell.hotkeys.show_window = show_binding;
                if !warnings.is_empty() {
                    shell.status_text = format!(
                        "Tray shell is ready with fallback hotkeys. {}",
                        warnings.join(" ")
                    );
                }
            }

            let start_minimized = {
                let persisted_state = app.state::<SharedPersistedState>();
                let persisted = persisted_state
                    .0
                    .lock()
                    .map_err(|_| setup_error("Could not read minimized startup preference."))?;
                persisted.settings.start_minimized
            };

            setup_visibility
                .start_minimized
                .store(start_minimized, Ordering::Release);
            setup_visibility.initialized.store(true, Ordering::Release);
            resolve_startup_visibility(&app_handle, &setup_visibility);

            emit_app_snapshot(&app.handle());
            Ok(())
        })
        .on_page_load(move |webview, payload| {
            if webview.label() != "main"
                || payload.event() != PageLoadEvent::Finished
                || payload.url().scheme() == "about"
            {
                return;
            }

            page_load_visibility
                .page_loaded
                .store(true, Ordering::Release);
            resolve_startup_visibility(webview.window().app_handle(), &page_load_visibility);
        })
        .invoke_handler(tauri::generate_handler![
            get_app_bootstrap,
            download_recommended_whisper_model,
            download_recommended_whisper_runtime,
            download_whisper_runtime_version,
            download_recommended_ffmpeg,
            check_whisper_runtime_update,
            check_whisper_model_update,
            toggle_whisper_model_download_pause,
            cancel_whisper_model_download,
            save_settings,
            start_recording,
            stop_recording,
            load_anki_catalog,
            play_recording,
            delete_recording,
            delete_recordings,
            push_recordings_to_anki,
            push_recordings_to_anki_deck,
            add_furigana_to_anki,
            translate_recordings,
            transcribe_recordings,
            convert_recordings_to_mp3,
            show_main_window,
            hide_main_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{
        join_anki_field_parts, normalize_theme_preference, preserve_anki_sound_tags,
        reconcile_recording_history, recording_pushed_to_anki_target,
        recording_transcript_supports_furigana, rename_recording_outputs_from_transcript,
        sanitize_recording_name, unique_wav_path, AnkiSettings, PersistedData, RecentRecording,
    };
    use std::path::Path;

    #[test]
    fn sanitize_recording_name_removes_windows_invalid_chars() {
        assert_eq!(sanitize_recording_name("  lesson:01?*  "), "lesson 01");
    }

    #[test]
    fn unique_wav_path_appends_suffix_when_file_exists() {
        let temp_dir = tempfile::tempdir().unwrap();
        let first = temp_dir.path().join("sample.wav");
        std::fs::write(&first, b"test").unwrap();
        let second = unique_wav_path(temp_dir.path(), "sample");
        assert_eq!(
            second.file_name().unwrap().to_string_lossy(),
            "sample_1.wav"
        );
    }

    #[test]
    fn transcript_renames_use_a_matched_timestamped_pair() {
        let temp_dir = tempfile::tempdir().unwrap();
        let audio_path = temp_dir.path().join("recording.wav");
        let transcript_path = temp_dir.path().join("temporary.txt");
        std::fs::write(&audio_path, b"audio").unwrap();
        std::fs::write(&transcript_path, "same audio").unwrap();

        let (renamed_audio, renamed_transcript) =
            rename_recording_outputs_from_transcript(&audio_path, &transcript_path, 12345).unwrap();

        assert_eq!(
            renamed_audio.file_name().unwrap().to_string_lossy(),
            "same audio_12345.wav"
        );
        assert_eq!(
            renamed_transcript.file_name().unwrap().to_string_lossy(),
            "same audio_12345.transcript.txt"
        );
    }

    #[test]
    fn repeated_transcripts_never_reuse_an_existing_output_pair() {
        let temp_dir = tempfile::tempdir().unwrap();

        let first_audio = temp_dir.path().join("recording_a.wav");
        let first_transcript = temp_dir.path().join("temporary_a.txt");
        std::fs::write(&first_audio, b"audio").unwrap();
        std::fs::write(&first_transcript, "same audio").unwrap();
        let first_pair =
            rename_recording_outputs_from_transcript(&first_audio, &first_transcript, 12345)
                .unwrap();

        let second_audio = temp_dir.path().join("recording_b.wav");
        let second_transcript = temp_dir.path().join("temporary_b.txt");
        std::fs::write(&second_audio, b"audio").unwrap();
        std::fs::write(&second_transcript, "same audio").unwrap();
        let second_pair =
            rename_recording_outputs_from_transcript(&second_audio, &second_transcript, 12345)
                .unwrap();

        assert_ne!(first_pair, second_pair);
        assert_eq!(
            second_pair.0.file_name().unwrap().to_string_lossy(),
            "same audio_12345_1.wav"
        );
        assert_eq!(
            second_pair.1.file_name().unwrap().to_string_lossy(),
            "same audio_12345_1.transcript.txt"
        );
    }

    #[test]
    fn theme_preference_accepts_known_values_and_rejects_unknown_values() {
        assert_eq!(normalize_theme_preference("light"), "light");
        assert_eq!(normalize_theme_preference(" dark "), "dark");
        assert_eq!(normalize_theme_preference("sepia"), "system");
    }

    #[test]
    fn persisted_data_counter_defaults_to_positive_value() {
        let state = PersistedData {
            settings: serde_json::from_value(serde_json::json!({
                "outputDirectory": "C:\\Temp",
                "assetDirectory": "C:\\Temp\\assets",
                "whisper": {
                    "cliPath": "",
                    "modelPath": "",
                    "language": "auto"
                },
                "features": {
                    "transcription": true
                },
                "launchAtLogin": false,
                "startMinimized": false
            }))
            .unwrap(),
            recent_recordings: Vec::new(),
            untitled_counter: 0,
        };

        assert_eq!(state.untitled_counter, 0);
        assert_eq!(state.settings.theme, "system");
        assert!(Path::new("C:\\Temp").is_absolute());
    }

    #[test]
    fn recording_history_recovers_untracked_audio_without_dropping_existing_entries() {
        let temp_dir = tempfile::tempdir().unwrap();
        let recovered_audio = temp_dir.path().join("recovered.wav");
        let recovered_transcript = temp_dir.path().join("recovered.transcript.txt");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 16_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&recovered_audio, spec).unwrap();
        for _ in 0..16_000 {
            writer.write_sample(0i16).unwrap();
        }
        writer.finalize().unwrap();
        std::fs::write(&recovered_transcript, "recovered transcript").unwrap();

        let existing = RecentRecording {
            file_name: "existing.wav".into(),
            file_path: temp_dir.path().join("existing.wav").display().to_string(),
            transcript_path: None,
            transcript_language: None,
            translation_path: None,
            anki_note_id: Some(42),
            anki_deck_name: Some("Japanese".into()),
            anki_note_type: Some("Mining".into()),
            audio_deleted: true,
            duration_ms: 123,
            bytes_written: 0,
            created_at_ms: 1,
        };
        let mut state = PersistedData {
            settings: serde_json::from_value(serde_json::json!({
                "outputDirectory": temp_dir.path().display().to_string(),
                "assetDirectory": temp_dir.path().join("assets").display().to_string(),
                "whisper": {
                    "cliPath": "",
                    "modelPath": "",
                    "language": "auto"
                },
                "features": {
                    "transcription": true
                },
                "launchAtLogin": false,
                "startMinimized": false
            }))
            .unwrap(),
            recent_recordings: vec![existing],
            untitled_counter: 1,
        };

        reconcile_recording_history(&mut state);

        assert_eq!(state.recent_recordings.len(), 2);
        let preserved = state
            .recent_recordings
            .iter()
            .find(|recording| recording.anki_note_id == Some(42))
            .unwrap();
        assert!(preserved.audio_deleted);

        let recovered = state
            .recent_recordings
            .iter()
            .find(|recording| recording.file_name == "recovered.wav")
            .unwrap();
        let recovered_transcript_path = recovered_transcript.display().to_string();
        assert_eq!(
            recovered.transcript_path.as_deref(),
            Some(recovered_transcript_path.as_str())
        );
        assert_eq!(recovered.duration_ms, 1000);
        assert!(recovered.bytes_written > 0);
    }

    #[test]
    fn anki_target_match_requires_same_deck_and_note_type() {
        let settings = AnkiSettings {
            deck_name: "Japanese".into(),
            note_type: "Mining".into(),
            ..Default::default()
        };
        let mut recording = RecentRecording {
            file_name: "sample.wav".into(),
            file_path: "C:\\Temp\\sample.wav".into(),
            transcript_path: Some("C:\\Temp\\sample.transcript.txt".into()),
            transcript_language: Some("ja".into()),
            translation_path: None,
            anki_note_id: Some(42),
            anki_deck_name: Some("Japanese".into()),
            anki_note_type: Some("Mining".into()),
            audio_deleted: false,
            duration_ms: 1,
            bytes_written: 1,
            created_at_ms: 1,
        };

        assert!(recording_pushed_to_anki_target(&recording, &settings));

        recording.anki_deck_name = Some("Other".into());
        assert!(!recording_pushed_to_anki_target(&recording, &settings));

        recording.anki_deck_name = Some("Japanese".into());
        recording.anki_note_type = Some("Basic".into());
        assert!(!recording_pushed_to_anki_target(&recording, &settings));

        recording.anki_note_type = Some("Mining".into());
        recording.anki_note_id = None;
        assert!(!recording_pushed_to_anki_target(&recording, &settings));
    }

    #[test]
    fn furigana_requires_japanese_transcript_language() {
        let mut recording = RecentRecording {
            file_name: "sample.wav".into(),
            file_path: "C:\\Temp\\sample.wav".into(),
            transcript_path: Some("C:\\Temp\\sample.transcript.txt".into()),
            transcript_language: Some("en".into()),
            translation_path: None,
            anki_note_id: Some(42),
            anki_deck_name: Some("Japanese".into()),
            anki_note_type: Some("Mining".into()),
            audio_deleted: false,
            duration_ms: 1,
            bytes_written: 1,
            created_at_ms: 1,
        };

        assert!(!recording_transcript_supports_furigana(
            &recording,
            "日本語を食べる"
        ));

        recording.transcript_language = Some("ja".into());
        assert!(recording_transcript_supports_furigana(
            &recording,
            "plain text"
        ));

        recording.transcript_language = None;
        assert!(recording_transcript_supports_furigana(
            &recording,
            "日本語を食べる"
        ));
        assert!(!recording_transcript_supports_furigana(
            &recording,
            "plain text"
        ));
    }

    #[test]
    fn anki_field_parts_join_without_erasing_audio() {
        assert_eq!(
            join_anki_field_parts("[sound:sample.wav]", "transcript"),
            "[sound:sample.wav]<br>transcript"
        );
        assert_eq!(join_anki_field_parts("", "transcript"), "transcript");
        assert_eq!(
            join_anki_field_parts("[sound:sample.wav]", ""),
            "[sound:sample.wav]"
        );
    }

    #[test]
    fn furigana_replacement_preserves_sound_tags() {
        let result = preserve_anki_sound_tags(
            Some("[sound:sample.wav]<br>old text"),
            "<ruby>text<rt>reading</rt></ruby>",
            None,
        );
        assert_eq!(
            result,
            "[sound:sample.wav]<br><ruby>text<rt>reading</rt></ruby>"
        );
    }

    #[test]
    fn furigana_replacement_uses_fallback_sound_tag() {
        let result = preserve_anki_sound_tags(
            Some("old text"),
            "<ruby>text<rt>reading</rt></ruby>",
            Some("[sound:sample.wav]"),
        );
        assert_eq!(
            result,
            "[sound:sample.wav]<br><ruby>text<rt>reading</rt></ruby>"
        );
    }
}
