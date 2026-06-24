mod app_state;
mod app_types;
mod recording;
mod transcription;

use app_state::*;
use app_types::*;

use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use recording::capture_system_audio_loopback;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    webview::PageLoadEvent,
    AppHandle, Emitter, Manager, Runtime, WindowEvent,
};
#[cfg(desktop)]
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use transcription::{
    run_whisper_transcription, verify_whisper_cli, verify_whisper_model,
    WhisperTranscriptionRequest,
};
#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE},
    System::Threading::CreateMutexW,
    UI::WindowsAndMessaging::{
        FindWindowW, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
    },
};
use zip::ZipArchive;

const START_SHORTCUT: &str = "Ctrl+Alt+R";
const STOP_SHORTCUT: &str = "Ctrl+Alt+S";
const SHOW_SHORTCUT: &str = "Ctrl+Alt+W";
const APP_TITLE: &str = "Wonder of U";
const APP_SNAPSHOT_EVENT: &str = "app://snapshot-changed";
const START_SHORTCUT_CANDIDATES: [&str; 3] = [START_SHORTCUT, "Ctrl+Alt+Shift+R", "Ctrl+Alt+F8"];
const STOP_SHORTCUT_CANDIDATES: [&str; 3] = [STOP_SHORTCUT, "Ctrl+Alt+Shift+S", "Ctrl+Alt+F9"];
const SHOW_SHORTCUT_CANDIDATES: [&str; 3] = [SHOW_SHORTCUT, "Ctrl+Alt+Shift+W", "Ctrl+Alt+F10"];
const RECENT_RECORDINGS_LIMIT: usize = 10;
const WHISPER_RELEASES_API_URL: &str =
    "https://api.github.com/repos/ggml-org/whisper.cpp/releases/latest";
const RECOMMENDED_WHISPER_RUNTIME_VERSION: &str = "v1.8.4";
const RECOMMENDED_WHISPER_RUNTIME_FILE: &str = "whisper-bin-x64.zip";
const RECOMMENDED_FFMPEG_RUNTIME_FILE: &str = "ffmpeg-master-latest-win64-gpl-shared.zip";
const RECOMMENDED_FFMPEG_RUNTIME_URL: &str = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl-shared.zip";
const ANKI_CONNECT_TIMEOUT: Duration = Duration::from_millis(1500);
const ANKI_LOOKUP_FURIGANA_URL: &str = "http://127.0.0.1:8766/furigana";
const ANKI_LOOKUP_TIMEOUT: Duration = Duration::from_millis(2500);
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

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

fn log_event<R: Runtime>(app: &AppHandle<R>, level: &str, event: &str, details: serde_json::Value) {
    let path = app.state::<AppPathsState>().inner().log_file.clone();
    append_structured_log(&path, level, event, details);
}

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent("Wonder of U Desktop/0.1.0")
        .build()
        .map_err(|error| error.to_string())
}

fn update_check_http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent("Wonder of U Desktop/0.1.0")
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|error| error.to_string())
}

fn update_model_download_snapshot<R: Runtime, F>(
    app: &AppHandle<R>,
    update: F,
) -> Result<(), String>
where
    F: FnOnce(&mut ModelDownloadSnapshot),
{
    let download_state = app.state::<ModelDownloadState>();
    let mut snapshot = download_state
        .0
        .lock()
        .map_err(|_| "Could not update the model download state.".to_string())?;
    update(&mut snapshot);
    drop(snapshot);
    emit_app_snapshot(app);
    Ok(())
}

fn reset_model_download_control<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let control_state = app.state::<ModelDownloadControlState>();
    let mut control = control_state
        .control
        .lock()
        .map_err(|_| "Could not reset the model download control state.".to_string())?;
    control.active = false;
    control.paused = false;
    control.cancel_requested = false;
    control_state.condvar.notify_all();
    Ok(())
}

fn ensure_directory_exists(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| error.to_string())
}

fn push_whisper_candidate(candidates: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !candidates.iter().any(|existing| existing == &candidate) {
        candidates.push(candidate);
    }
}

fn add_cli_candidates_from_directory(
    candidates: &mut Vec<PathBuf>,
    directory: &Path,
    remaining_depth: usize,
) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(file_type) = entry.file_type() else {
            continue;
        };

        if file_type.is_dir() {
            if remaining_depth > 0 {
                add_cli_candidates_from_directory(candidates, &path, remaining_depth - 1);
            }
            continue;
        }

        let is_cli = path
            .file_name()
            .and_then(|value| value.to_str())
            .map(|value| {
                value.eq_ignore_ascii_case("whisper-cli.exe")
                    || value.eq_ignore_ascii_case("whisper-cli")
            })
            .unwrap_or(false);

        if is_cli {
            push_whisper_candidate(candidates, path);
        }
    }
}

fn add_model_candidates_from_directory(candidates: &mut Vec<PathBuf>, directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let is_model = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.eq_ignore_ascii_case("bin"))
            .unwrap_or(false);

        if is_model {
            push_whisper_candidate(candidates, path);
        }
    }
}

fn push_whisper_model_directory(candidates: &mut Vec<PathBuf>, directory: PathBuf) {
    add_model_candidates_from_directory(candidates, &directory);
}

fn managed_runtime_root(asset_directory: &Path) -> PathBuf {
    asset_directory.join("whisper-runtime")
}

fn app_managed_runtime_directory(asset_directory: &Path, runtime_version: &str) -> PathBuf {
    managed_runtime_root(asset_directory).join(sanitize_runtime_version(runtime_version))
}

fn collect_managed_whisper_cli_candidates(
    asset_directory: &Path,
    runtime_version: &str,
) -> Vec<PathBuf> {
    let executable_names = ["whisper-cli.exe", "whisper-cli"];
    let mut candidates = Vec::new();
    let runtime_directory = app_managed_runtime_directory(asset_directory, runtime_version);

    for executable_name in executable_names {
        push_whisper_candidate(&mut candidates, runtime_directory.join(executable_name));
        push_whisper_candidate(
            &mut candidates,
            runtime_directory.join("bin").join(executable_name),
        );
        push_whisper_candidate(
            &mut candidates,
            runtime_directory.join("Release").join(executable_name),
        );
        push_whisper_candidate(
            &mut candidates,
            runtime_directory
                .join("bin")
                .join("Release")
                .join(executable_name),
        );
        push_whisper_candidate(&mut candidates, asset_directory.join(executable_name));
    }

    add_cli_candidates_from_directory(&mut candidates, &runtime_directory, 4);

    candidates
}

fn collect_installed_runtime_versions(asset_directory: &Path) -> Vec<String> {
    let runtime_root = managed_runtime_root(asset_directory);
    let Ok(entries) = fs::read_dir(runtime_root) else {
        return Vec::new();
    };

    let mut versions = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            if !path.is_dir() {
                return None;
            }

            let version = entry.file_name().to_string_lossy().to_string();
            collect_managed_whisper_cli_candidates(asset_directory, &version)
                .into_iter()
                .any(|candidate| candidate.exists())
                .then_some(version)
        })
        .collect::<Vec<_>>();

    versions.sort();
    versions.dedup();
    versions
}

fn managed_ffmpeg_root(asset_directory: &Path) -> PathBuf {
    asset_directory.join("ffmpeg-runtime")
}

fn managed_ffmpeg_install_directory(asset_directory: &Path) -> PathBuf {
    managed_ffmpeg_root(asset_directory).join("latest")
}

fn push_ffmpeg_candidate(candidates: &mut Vec<PathBuf>, candidate: PathBuf) {
    if !candidates.iter().any(|existing| existing == &candidate) {
        candidates.push(candidate);
    }
}

fn push_ffmpeg_candidates_from_directory(candidates: &mut Vec<PathBuf>, directory: &Path) {
    if !directory.exists() {
        return;
    }

    push_ffmpeg_candidate(candidates, directory.join("ffmpeg.exe"));
    push_ffmpeg_candidate(candidates, directory.join("ffmpeg"));
    push_ffmpeg_candidate(candidates, directory.join("bin").join("ffmpeg.exe"));
    push_ffmpeg_candidate(candidates, directory.join("bin").join("ffmpeg"));

    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            push_ffmpeg_candidates_from_directory(candidates, &path);
        } else if path
            .file_name()
            .and_then(|value| value.to_str())
            .map(|value| value.eq_ignore_ascii_case("ffmpeg.exe") || value == "ffmpeg")
            .unwrap_or(false)
        {
            push_ffmpeg_candidate(candidates, path);
        }
    }
}

fn collect_managed_ffmpeg_candidates(asset_directory: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    push_ffmpeg_candidates_from_directory(
        &mut candidates,
        &managed_ffmpeg_install_directory(asset_directory),
    );
    candidates
}

fn verify_ffmpeg_binary(executable_path: &Path) -> Result<(), String> {
    let mut command = Command::new(executable_path);
    hide_command_window(&mut command);
    let output = command
        .arg("-version")
        .output()
        .map_err(|error| error.to_string())?;

    if output.status.success() {
        return Ok(());
    }

    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
    Err(if stderr.is_empty() { stdout } else { stderr })
}

fn detect_local_ffmpeg(settings: &AppSettings) -> FfmpegDetection {
    let asset_directory = PathBuf::from(&settings.asset_directory);
    if let Some(managed_path) = collect_managed_ffmpeg_candidates(&asset_directory)
        .into_iter()
        .find(|candidate| candidate.exists() && verify_ffmpeg_binary(candidate).is_ok())
    {
        return FfmpegDetection {
            status: "ready".into(),
            executable_path: Some(managed_path.display().to_string()),
            managed: true,
            message:
                "App-managed FFmpeg is ready. Transcribed WAV recordings can be manually converted to MP3."
                    .into(),
        };
    }

    let path_candidate = PathBuf::from("ffmpeg");
    if verify_ffmpeg_binary(&path_candidate).is_ok() {
        return FfmpegDetection {
            status: "ready".into(),
            executable_path: Some("ffmpeg".into()),
            managed: false,
            message:
                "System FFmpeg is available. Transcribed WAV recordings can be manually converted to MP3."
                    .into(),
        };
    }

    FfmpegDetection::default()
}

fn collect_managed_whisper_model_candidates(
    asset_directory: &Path,
    executable_path: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    push_whisper_model_directory(&mut candidates, asset_directory.join("models"));
    for runtime_version in collect_installed_runtime_versions(asset_directory) {
        let runtime_directory = app_managed_runtime_directory(asset_directory, &runtime_version);
        push_whisper_model_directory(&mut candidates, runtime_directory.clone());
        push_whisper_model_directory(&mut candidates, runtime_directory.join("models"));
    }

    if let Some(cli_path) = executable_path {
        if let Some(bin_directory) = cli_path.parent() {
            push_whisper_model_directory(&mut candidates, bin_directory.join("models"));

            if let Some(runtime_directory) = bin_directory.parent() {
                push_whisper_model_directory(&mut candidates, runtime_directory.join("models"));

                if let Some(root_directory) = runtime_directory.parent() {
                    push_whisper_model_directory(&mut candidates, root_directory.join("models"));
                }
            }
        }
    }

    candidates
}

fn find_existing_managed_model_path(
    asset_directory: &Path,
    model_choice: &str,
    executable_path: Option<&Path>,
) -> Option<PathBuf> {
    let expected_file_name = whisper_model_spec(model_choice).file_name;
    collect_managed_whisper_model_candidates(asset_directory, executable_path)
        .into_iter()
        .find(|candidate| {
            candidate.exists()
                && candidate
                    .file_name()
                    .and_then(|value| value.to_str())
                    .map(|value| value.eq_ignore_ascii_case(expected_file_name))
                    .unwrap_or(false)
        })
}

fn all_managed_model_paths(asset_directory: &Path) -> Vec<PathBuf> {
    let models_directory = asset_directory.join("models");
    WHISPER_MODEL_SPECS
        .iter()
        .map(|spec| models_directory.join(spec.file_name))
        .collect()
}

fn validate_manual_path(manual_path: &str) -> Option<PathBuf> {
    let trimmed = manual_path.trim();
    if trimmed.is_empty() {
        return None;
    }

    let candidate = PathBuf::from(trimmed);
    candidate.exists().then_some(candidate)
}

fn detect_local_whisper<R: Runtime>(app: &AppHandle<R>) -> Result<WhisperDetection, String> {
    let settings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the current settings.".to_string())?;
        persisted.settings.clone()
    };

    let manual_cli_override_present = !settings.whisper.cli_path.trim().is_empty();
    let manual_model_override_present = !settings.whisper.model_path.trim().is_empty();
    let manual_cli_path = validate_manual_path(&settings.whisper.cli_path);
    let manual_model_path = validate_manual_path(&settings.whisper.model_path);
    let model_choice = whisper_model_spec(&settings.whisper.model_choice).id;
    let runtime_version = sanitize_runtime_version(&settings.whisper.runtime_version);
    let asset_directory = PathBuf::from(&settings.asset_directory);
    let available_runtime_versions = collect_installed_runtime_versions(&asset_directory);

    let (executable_path, source) = if let Some(path) = manual_cli_path {
        (Some(path), Some("manual".to_string()))
    } else {
        (
            collect_managed_whisper_cli_candidates(&asset_directory, &runtime_version)
                .into_iter()
                .find(|candidate| candidate.exists()),
            Some("managed".to_string()),
        )
    };

    let executable_path = executable_path.filter(|path| path.exists());
    let source = executable_path.as_ref().map(|_| source).unwrap_or(None);

    let (model_path, model_source) = if let Some(path) = manual_model_path {
        (Some(path), Some("manual".to_string()))
    } else {
        (
            find_existing_managed_model_path(
                &asset_directory,
                model_choice,
                executable_path.as_deref(),
            ),
            Some("managed".to_string()),
        )
    };
    let model_path = model_path.filter(|path| path.exists());
    let model_source = model_path.as_ref().map(|_| model_source).unwrap_or(None);

    let cli_error = executable_path
        .as_deref()
        .and_then(|path| verify_whisper_cli(path).err());
    let model_error = model_path
        .as_deref()
        .and_then(|path| verify_whisper_model(path).err());

    let (status, message) = match (
        executable_path.as_ref(),
        model_path.as_ref(),
        cli_error.as_ref(),
        model_error.as_ref(),
    ) {
        (Some(_), Some(_), None, None) => (
            "ready".to_string(),
            "Whisper is ready for offline transcription.".to_string(),
        ),
        (Some(_), Some(_), Some(error), _) => (
            "invalid".to_string(),
            format!("The selected whisper-cli path failed validation: {error}"),
        ),
        (Some(_), Some(_), _, Some(error)) => (
            "invalid".to_string(),
            format!("The selected Whisper model failed validation: {error}"),
        ),
        (None, _, _, _) if manual_cli_override_present => (
            "cliMissing".to_string(),
            "The manual whisper-cli path was not found. Fix the path or download the recommended runtime."
                .to_string(),
        ),
        (None, _, _, _) => (
            "cliMissing".to_string(),
            "Whisper CLI is missing. Add a manual path or download the recommended runtime."
                .to_string(),
        ),
        (Some(_), None, _, _) if manual_model_override_present => (
            "modelMissing".to_string(),
            "The manual Whisper model path was not found. Fix the path or download the selected model."
                .to_string(),
        ),
        (Some(_), None, _, _) => (
            "modelMissing".to_string(),
            "Whisper CLI is ready, but no usable ggml model file is configured yet.".to_string(),
        ),
    };

    let cli_ready = executable_path.is_some() && cli_error.is_none();
    let model_ready = model_path.is_some() && model_error.is_none();
    let cli_managed = matches!(source.as_deref(), Some("managed"));
    let model_managed = matches!(model_source.as_deref(), Some("managed"));

    Ok(WhisperDetection {
        status,
        executable_path: executable_path.map(|path| path.display().to_string()),
        model_path: model_path.map(|path| path.display().to_string()),
        source,
        model_source,
        runtime_version,
        available_runtime_versions,
        cli_ready,
        model_ready,
        cli_managed,
        model_managed,
        message,
    })
}

fn refresh_whisper_detection_state<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<WhisperDetection, String> {
    let detection = detect_local_whisper(app)?;
    let detection_state = app.state::<WhisperDetectionState>();
    let mut stored_detection = detection_state
        .0
        .lock()
        .map_err(|_| "Could not update the Whisper readiness state.".to_string())?;
    *stored_detection = detection.clone();
    drop(stored_detection);

    log_event(
        app,
        "INFO",
        "whisper.ready_state",
        serde_json::json!({
            "status": detection.status,
            "source": detection.source,
            "executablePath": detection.executable_path,
            "modelPath": detection.model_path
        }),
    );

    emit_app_snapshot(app);
    Ok(detection)
}

fn check_whisper_runtime_update_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<WhisperAssetUpdateResult, String> {
    let detection = refresh_whisper_detection_state(app)?;
    if !detection.cli_ready {
        return Ok(WhisperAssetUpdateResult {
            kind: "runtime".into(),
            status: "unavailable".into(),
            message: "Install or point the app to whisper-cli before checking for runtime updates."
                .into(),
            current_version: None,
            latest_version: None,
        });
    }

    if !detection.cli_managed {
        return Ok(WhisperAssetUpdateResult {
            kind: "runtime".into(),
            status: "manual".into(),
            message: "Update checks are only available for the app-managed Whisper runtime.".into(),
            current_version: detection.executable_path,
            latest_version: None,
        });
    }

    let response = update_check_http_client()?
        .get(WHISPER_RELEASES_API_URL)
        .send()
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let payload = response.text().map_err(|error| error.to_string())?;
    let latest_tag = serde_json::from_str::<serde_json::Value>(&payload)
        .ok()
        .and_then(|value| {
            value
                .get("tag_name")
                .and_then(|tag| tag.as_str())
                .map(str::to_string)
        })
        .ok_or_else(|| "Could not read the latest whisper.cpp release tag.".to_string())?;

    let current_version = sanitize_runtime_version(&detection.runtime_version);
    let latest_version = sanitize_runtime_version(&latest_tag);
    let latest_installed = detection
        .available_runtime_versions
        .iter()
        .any(|version| sanitize_runtime_version(version) == latest_version);
    let update_available = latest_version != current_version;
    Ok(WhisperAssetUpdateResult {
        kind: "runtime".into(),
        status: if !update_available {
            "current".into()
        } else if latest_installed {
            "installed".into()
        } else {
            "available".into()
        },
        message: if !update_available {
            "Your app-managed Whisper runtime is up to date.".into()
        } else if latest_installed {
            format!(
                "Whisper runtime {} is already downloaded. Select it from Active runtime to use it.",
                latest_version
            )
        } else {
            format!(
                "A newer whisper.cpp runtime is available: {}.",
                latest_version
            )
        },
        current_version: Some(current_version),
        latest_version: Some(latest_version),
    })
}

fn check_whisper_model_update_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<WhisperAssetUpdateResult, String> {
    let detection = refresh_whisper_detection_state(app)?;
    if !detection.model_ready {
        return Ok(WhisperAssetUpdateResult {
            kind: "model".into(),
            status: "unavailable".into(),
            message: "Install or point the app to a Whisper model before checking for updates."
                .into(),
            current_version: None,
            latest_version: None,
        });
    }

    if !detection.model_managed {
        return Ok(WhisperAssetUpdateResult {
            kind: "model".into(),
            status: "manual".into(),
            message: "Update checks are only available for the app-managed Whisper model.".into(),
            current_version: detection.model_path,
            latest_version: None,
        });
    }

    let settings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the current app settings.".to_string())?;
        persisted.settings.clone()
    };
    let model_spec = whisper_model_spec(&settings.whisper.model_choice);
    let local_model_path = detection
        .model_path
        .as_ref()
        .map(PathBuf::from)
        .ok_or_else(|| "The app-managed model path could not be resolved.".to_string())?;
    let local_size = fs::metadata(&local_model_path)
        .map_err(|error| error.to_string())?
        .len();

    let response = update_check_http_client()?
        .head(model_spec.download_url)
        .send()
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let remote_size = response
        .headers()
        .get("content-length")
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok());

    let (status, message) = match remote_size {
        Some(size) if size != local_size => (
            "available".to_string(),
            format!(
                "A newer {} model build may be available for download.",
                model_spec.label
            ),
        ),
        Some(_) => (
            "current".to_string(),
            format!("Your {} model appears to be up to date.", model_spec.label),
        ),
        None => (
            "unknown".to_string(),
            "The remote model size could not be verified right now.".into(),
        ),
    };

    Ok(WhisperAssetUpdateResult {
        kind: "model".into(),
        status,
        message,
        current_version: Some(format!("{} ({})", model_spec.label, model_spec.file_name)),
        latest_version: remote_size.map(|size| format!("{} bytes", size)),
    })
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

fn whisper_detection_inputs_changed(previous: &AppSettings, next: &AppSettings) -> bool {
    previous.asset_directory != next.asset_directory
        || previous.whisper.cli_path != next.whisper.cli_path
        || previous.whisper.runtime_version != next.whisper.runtime_version
        || previous.whisper.model_path != next.whisper.model_path
        || previous.whisper.model_choice != next.whisper.model_choice
}

fn build_app_bootstrap<R: Runtime>(app: &AppHandle<R>) -> Result<AppBootstrap, String> {
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

fn emit_app_snapshot<R: Runtime>(app: &AppHandle<R>) {
    if let Ok(snapshot) = build_app_bootstrap(app) {
        let _ = app.emit(APP_SNAPSHOT_EVENT, &snapshot);
    }
}

fn update_shell_snapshot<R: Runtime, F>(app: &AppHandle<R>, update: F) -> Result<(), String>
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
        persisted
            .recent_recordings
            .truncate(RECENT_RECORDINGS_LIMIT);

        persisted.clone()
    };

    write_persisted_data(app, &persisted_snapshot)
}

fn hide_command_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

fn anki_offline_message(error: &str) -> String {
    format!(
        "Anki is currently offline. Start Anki and make sure AnkiConnect is installed, then try again. {error}"
    )
}

fn anki_connect_request(
    action: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(ANKI_CONNECT_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;
    let payload = serde_json::json!({
        "action": action,
        "version": 6,
        "params": params
    });

    let response = client
        .post("http://127.0.0.1:8765")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(payload.to_string())
        .send()
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let response = response.text().map_err(|error| error.to_string())?;
    let response =
        serde_json::from_str::<serde_json::Value>(&response).map_err(|error| error.to_string())?;

    if let Some(error) = response.get("error").and_then(|value| value.as_str()) {
        if !error.is_empty() {
            return Err(error.to_string());
        }
    }

    Ok(response
        .get("result")
        .cloned()
        .unwrap_or(serde_json::Value::Null))
}

fn anki_note_exists(note_id: i64) -> Result<bool, String> {
    let result = anki_connect_request(
        "notesInfo",
        serde_json::json!({
            "notes": [note_id]
        }),
    )?;

    let Some(notes) = result.as_array() else {
        return Ok(false);
    };

    Ok(notes.iter().any(|note| {
        note.get("noteId")
            .and_then(|value| value.as_i64())
            .is_some_and(|candidate| candidate == note_id)
    }))
}

fn anki_note_field_value(note_id: i64, field_name: &str) -> Result<Option<String>, String> {
    let result = anki_connect_request(
        "notesInfo",
        serde_json::json!({
            "notes": [note_id]
        }),
    )?;

    let Some(note) = result.as_array().and_then(|notes| {
        notes.iter().find(|note| {
            note.get("noteId")
                .and_then(|value| value.as_i64())
                .is_some_and(|candidate| candidate == note_id)
        })
    }) else {
        return Ok(None);
    };

    Ok(note
        .get("fields")
        .and_then(|fields| fields.as_object())
        .and_then(|fields| fields.get(field_name))
        .and_then(|field| field.get("value"))
        .and_then(|value| value.as_str())
        .map(ToString::to_string))
}

fn clear_recording_anki_reference<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
) -> Result<(), String> {
    update_recent_recording(app, file_path, |recording| {
        recording.anki_note_id = None;
        recording.anki_deck_name = None;
        recording.anki_note_type = None;
    })
}

fn refresh_recording_anki_reference<R: Runtime>(
    app: &AppHandle<R>,
    mut recording: RecentRecording,
) -> Result<RecentRecording, String> {
    let Some(note_id) = recording.anki_note_id else {
        return Ok(recording);
    };

    if anki_note_exists(note_id)? {
        return Ok(recording);
    }

    clear_recording_anki_reference(app, &recording.file_path)?;
    recording.anki_note_id = None;
    recording.anki_deck_name = None;
    recording.anki_note_type = None;
    Ok(recording)
}

fn refresh_recent_anki_note_references<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let recordings_with_notes = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not inspect pushed Anki cards.".to_string())?;
        persisted
            .recent_recordings
            .iter()
            .filter_map(|recording| {
                recording
                    .anki_note_id
                    .map(|note_id| (recording.file_path.clone(), note_id))
            })
            .collect::<Vec<_>>()
    };

    let mut missing_file_paths = Vec::new();
    for (file_path, note_id) in recordings_with_notes {
        if !anki_note_exists(note_id)? {
            missing_file_paths.push(file_path);
        }
    }

    if missing_file_paths.is_empty() {
        return Ok(());
    }

    let persisted_snapshot = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update pushed Anki card status.".to_string())?;
        for recording in &mut persisted.recent_recordings {
            if missing_file_paths
                .iter()
                .any(|file_path| file_path == &recording.file_path)
            {
                recording.anki_note_id = None;
                recording.anki_deck_name = None;
                recording.anki_note_type = None;
            }
        }
        persisted.clone()
    };

    write_persisted_data(app, &persisted_snapshot)?;
    emit_app_snapshot(app);
    Ok(())
}

fn request_furigana_html(text: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(ANKI_LOOKUP_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;

    let response_text = client
        .post(ANKI_LOOKUP_FURIGANA_URL)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::json!({ "text": text }).to_string())
        .send()
        .map_err(|error| {
            format!(
                "Anki Lookup add-on is not running or did not respond. Open Anki with the Wonder of U/Anki Lookup add-on installed, then try again. {error}"
            )
        })?
        .error_for_status()
        .map_err(|error| {
            format!("Anki Lookup add-on rejected the furigana request. {error}")
        })?
        .text()
        .map_err(|error| format!("Anki Lookup add-on response could not be read. {error}"))?;
    let response = serde_json::from_str::<FuriganaBridgeResponse>(&response_text)
        .map_err(|error| format!("Anki Lookup add-on returned invalid furigana data. {error}"))?;

    if response.ok {
        response
            .furigana_html
            .ok_or_else(|| "Anki Lookup add-on did not return furigana HTML.".to_string())
    } else {
        Err(response
            .error
            .unwrap_or_else(|| "Anki Lookup add-on could not create furigana.".into()))
    }
}

fn json_string_array(value: serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

fn load_anki_catalog_inner<R: Runtime>(
    app: &AppHandle<R>,
    note_type: Option<String>,
) -> Result<AnkiCatalog, String> {
    let configured_note_type = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the Anki settings.".to_string())?;
        persisted.settings.anki.note_type.clone()
    };
    let selected_note_type = note_type.unwrap_or(configured_note_type).trim().to_string();

    let version = match anki_connect_request("version", serde_json::json!({})) {
        Ok(value) => value.as_i64(),
        Err(error) => {
            return Ok(AnkiCatalog {
                status: "offline".into(),
                message: anki_offline_message(&error),
                version: None,
                decks: Vec::new(),
                note_types: Vec::new(),
                fields: Vec::new(),
            });
        }
    };

    if let Err(error) = refresh_recent_anki_note_references(app) {
        log_event(
            app,
            "WARN",
            "anki.note_reference_refresh_failed",
            serde_json::json!({ "message": error }),
        );
    }

    let mut decks = json_string_array(anki_connect_request("deckNames", serde_json::json!({}))?);
    let mut note_types =
        json_string_array(anki_connect_request("modelNames", serde_json::json!({}))?);
    decks.sort();
    note_types.sort();

    let fields = if selected_note_type.is_empty() {
        Vec::new()
    } else {
        json_string_array(anki_connect_request(
            "modelFieldNames",
            serde_json::json!({ "modelName": selected_note_type }),
        )?)
    };

    Ok(AnkiCatalog {
        status: "ready".into(),
        message: "AnkiConnect is ready.".into(),
        version,
        decks,
        note_types,
        fields,
    })
}

fn find_recent_recording<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
) -> Result<RecentRecording, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not read the recording history.".to_string())?;
    persisted
        .recent_recordings
        .iter()
        .find(|recording| recording.file_path == file_path)
        .cloned()
        .ok_or_else(|| "The selected recording is no longer in the recent list.".to_string())
}

fn update_recent_recording<R: Runtime, F>(
    app: &AppHandle<R>,
    file_path: &str,
    update: F,
) -> Result<(), String>
where
    F: FnOnce(&mut RecentRecording),
{
    let persisted_snapshot = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update the recording history.".to_string())?;
        let recording = persisted
            .recent_recordings
            .iter_mut()
            .find(|recording| recording.file_path == file_path)
            .ok_or_else(|| "The selected recording is no longer in the recent list.".to_string())?;
        update(recording);
        persisted.clone()
    };

    write_persisted_data(app, &persisted_snapshot)?;
    emit_app_snapshot(app);
    Ok(())
}

fn delete_local_audio_after_anki_push<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
) -> Result<(), String> {
    let audio_path = PathBuf::from(file_path);
    match fs::remove_file(&audio_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Anki card was created, but local audio cleanup failed: {error}"
            ));
        }
    }

    update_recent_recording(app, file_path, |recording| {
        recording.audio_deleted = true;
        recording.bytes_written = 0;
    })
}

fn anki_media_file_name(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .map(sanitize_recording_name)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "recording".into())
        .replace(' ', "_");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("wav");
    format!("wonder_of_u_{stem}.{extension}")
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\n', "<br>")
}

fn user_friendly_anki_error(error: &str, settings: &AnkiSettings) -> String {
    let normalized = error.to_lowercase();
    if normalized.contains("duplicate") {
        return format!(
            "This transcript already exists in the '{}' deck. Wonder of U did not create a duplicate card.",
            settings.deck_name
        );
    }

    if normalized.contains("model") && normalized.contains("not") && normalized.contains("found") {
        return format!(
            "Anki could not find the '{}' note type. Refresh Anki mapping and choose an available note type.",
            settings.note_type
        );
    }

    if normalized.contains("deck") && normalized.contains("not") && normalized.contains("found") {
        return format!(
            "Anki could not find the '{}' deck. Refresh Anki mapping and choose an available deck.",
            settings.deck_name
        );
    }

    if normalized.contains("field") {
        return "Anki rejected one of the mapped fields. Refresh Anki mapping and check that every selected field still exists on the note type.".into();
    }

    format!("Anki could not create the card. {error}")
}

fn push_single_recording_to_anki<R: Runtime>(
    app: &AppHandle<R>,
    recording: &RecentRecording,
    settings: &AnkiSettings,
    auto_add_furigana_after_push: bool,
) -> Result<AnkiPushOutcome, String> {
    if settings.deck_name.is_empty() {
        return Err("Choose an Anki deck before pushing recordings.".into());
    }
    if settings.note_type.is_empty() {
        return Err("Choose an Anki note type before pushing recordings.".into());
    }
    if settings.fields.transcription.is_empty() {
        return Err("Map an Anki field for the transcript before pushing recordings.".into());
    }

    let transcript_path = recording
        .transcript_path
        .as_deref()
        .ok_or_else(|| "This recording does not have a transcript yet.".to_string())?;
    let transcript = fs::read_to_string(transcript_path)
        .map_err(|error| format!("Could not read transcript: {error}"))?;
    let audio_path = PathBuf::from(&recording.file_path);
    if !audio_path.exists() {
        return Err("The audio file is missing from disk.".into());
    }

    let media_file_name = anki_media_file_name(&audio_path);
    anki_connect_request(
        "storeMediaFile",
        serde_json::json!({
            "filename": media_file_name,
            "path": recording.file_path
        }),
    )?;

    let mut fields = serde_json::Map::new();
    fields.insert(
        settings.fields.transcription.clone(),
        serde_json::Value::String(html_escape(&transcript)),
    );
    prepend_anki_field_value(
        &mut fields,
        &settings.fields.audio,
        format!("[sound:{media_file_name}]"),
    );
    if !settings.fields.source_path.is_empty() {
        fields.insert(
            settings.fields.source_path.clone(),
            serde_json::Value::String(html_escape(&recording.file_path)),
        );
    }
    if !settings.fields.created_at.is_empty() {
        fields.insert(
            settings.fields.created_at.clone(),
            serde_json::Value::String(recording.created_at_ms.to_string()),
        );
    }
    if !settings.fields.translation.is_empty() {
        if let Some(translation_path) = recording.translation_path.as_deref() {
            if let Ok(translation) = fs::read_to_string(translation_path) {
                fields.insert(
                    settings.fields.translation.clone(),
                    serde_json::Value::String(html_escape(&translation)),
                );
            }
        }
    }

    let furigana_message = if auto_add_furigana_after_push {
        maybe_insert_automatic_furigana_field(
            recording,
            settings,
            &transcript,
            &media_file_name,
            &mut fields,
        )
    } else {
        None
    };

    let note_id = anki_connect_request(
        "addNote",
        serde_json::json!({
            "note": {
                "deckName": settings.deck_name.clone(),
                "modelName": settings.note_type.clone(),
                "fields": fields,
                "options": {
                    "allowDuplicate": false,
                    "duplicateScope": "deck",
                    "duplicateScopeOptions": {
                        "deckName": settings.deck_name.clone(),
                        "checkChildren": false,
                        "checkAllModels": false
                    }
                },
                "tags": ["wonder-of-u"]
            }
        }),
    )
    .map_err(|error| user_friendly_anki_error(&error, settings))?
    .as_i64()
    .ok_or_else(|| "AnkiConnect did not return a note id.".to_string())?;

    update_recent_recording(app, &recording.file_path, |recording| {
        recording.anki_note_id = Some(note_id);
        recording.anki_deck_name = Some(settings.deck_name.clone());
        recording.anki_note_type = Some(settings.note_type.clone());
    })?;

    Ok(AnkiPushOutcome {
        note_id,
        furigana_message,
    })
}

fn maybe_insert_automatic_furigana_field(
    recording: &RecentRecording,
    settings: &AnkiSettings,
    transcript: &str,
    media_file_name: &str,
    fields: &mut serde_json::Map<String, serde_json::Value>,
) -> Option<String> {
    if !recording_transcript_supports_furigana(recording, transcript) {
        return None;
    }

    match request_furigana_html(transcript) {
        Ok(furigana_html) => {
            let target_field = settings.fields.transcription.as_str();
            let existing_value = fields.get(target_field).and_then(|value| value.as_str());
            let fallback_sound_tag =
                if !settings.fields.audio.is_empty() && settings.fields.audio == target_field {
                    Some(format!("[sound:{media_file_name}]"))
                } else {
                    None
                };
            let furigana_html = preserve_anki_sound_tags(
                existing_value,
                &furigana_html,
                fallback_sound_tag.as_deref(),
            );
            fields.insert(
                target_field.to_string(),
                serde_json::Value::String(furigana_html),
            );
            Some("Furigana was added automatically.".into())
        }
        Err(error) => Some(format!("Furigana was skipped because {error}")),
    }
}

fn prepend_anki_field_value(
    fields: &mut serde_json::Map<String, serde_json::Value>,
    field_name: &str,
    value: String,
) {
    if field_name.is_empty() {
        return;
    }

    let next_value = fields
        .get(field_name)
        .and_then(|existing| existing.as_str())
        .map(|existing| join_anki_field_parts(&value, existing))
        .unwrap_or(value);
    fields.insert(
        field_name.to_string(),
        serde_json::Value::String(next_value),
    );
}

fn join_anki_field_parts(first: &str, second: &str) -> String {
    let first = first.trim();
    let second = second.trim();
    match (first.is_empty(), second.is_empty()) {
        (true, true) => String::new(),
        (true, false) => second.to_string(),
        (false, true) => first.to_string(),
        (false, false) => format!("{first}<br>{second}"),
    }
}

fn preserve_anki_sound_tags(
    existing_value: Option<&str>,
    new_value: &str,
    fallback_sound_tag: Option<&str>,
) -> String {
    let mut sound_tags = existing_value
        .map(extract_anki_sound_tags)
        .unwrap_or_default();

    if let Some(fallback_sound_tag) = fallback_sound_tag {
        if !new_value.contains(fallback_sound_tag)
            && !sound_tags.iter().any(|tag| tag == fallback_sound_tag)
        {
            sound_tags.push(fallback_sound_tag.to_string());
        }
    }

    let sound_prefix = sound_tags.join(" ");
    join_anki_field_parts(&sound_prefix, new_value)
}

fn extract_anki_sound_tags(value: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut remaining = value;
    while let Some(start) = remaining.find("[sound:") {
        let candidate = &remaining[start..];
        let Some(end) = candidate.find(']') else {
            break;
        };
        let tag = candidate[..=end].to_string();
        if !tags.contains(&tag) {
            tags.push(tag);
        }
        remaining = &candidate[end + 1..];
    }
    tags
}

fn recording_pushed_to_anki_target(recording: &RecentRecording, settings: &AnkiSettings) -> bool {
    recording.anki_note_id.is_some()
        && recording.anki_deck_name.as_deref() == Some(settings.deck_name.as_str())
        && recording.anki_note_type.as_deref() == Some(settings.note_type.as_str())
}

fn selected_recordings<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
) -> Result<Vec<RecentRecording>, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not read the recording history.".to_string())?;
    let recordings = if file_paths.is_empty() {
        persisted
            .recent_recordings
            .iter()
            .filter(|recording| recording.transcript_path.is_some())
            .cloned()
            .collect()
    } else {
        file_paths
            .iter()
            .filter_map(|file_path| {
                persisted
                    .recent_recordings
                    .iter()
                    .find(|recording| recording.file_path == *file_path)
                    .cloned()
            })
            .collect()
    };

    Ok(recordings)
}

fn selected_untranscribed_recordings<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
) -> Result<Vec<RecentRecording>, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not read the recording history.".to_string())?;
    let recordings = if file_paths.is_empty() {
        persisted
            .recent_recordings
            .iter()
            .filter(|recording| recording.transcript_path.is_none())
            .cloned()
            .collect()
    } else {
        file_paths
            .iter()
            .filter_map(|file_path| {
                persisted
                    .recent_recordings
                    .iter()
                    .find(|recording| recording.file_path == *file_path)
                    .cloned()
            })
            .collect()
    };

    Ok(recordings)
}

fn apply_transcription_result_to_recording<R: Runtime>(
    app: &AppHandle<R>,
    original_file_path: &str,
    mut recording: RecentRecording,
    transcript_path: PathBuf,
    requested_language: &str,
) -> Result<RecentRecording, String> {
    let mut audio_path = PathBuf::from(&recording.file_path);
    let mut final_transcript_path = transcript_path;

    match rename_recording_outputs_from_transcript(&audio_path, &final_transcript_path) {
        Ok((renamed_audio_path, renamed_transcript_path)) => {
            audio_path = renamed_audio_path;
            final_transcript_path = renamed_transcript_path;
        }
        Err(error) => {
            log_event(
                app,
                "ERROR",
                "recording.rename_from_transcript_failed",
                serde_json::json!({
                    "audioPath": recording.file_path,
                    "message": error
                }),
            );
        }
    }

    recording.file_name = audio_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("recording.wav")
        .to_string();
    recording.file_path = audio_path.display().to_string();
    recording.transcript_path = Some(final_transcript_path.display().to_string());
    recording.transcript_language =
        derive_transcript_language_from_path(&final_transcript_path, requested_language);
    recording.bytes_written = fs::metadata(&audio_path)
        .map(|metadata| metadata.len())
        .unwrap_or(recording.bytes_written);

    let updated_recording = recording.clone();
    update_recent_recording(app, original_file_path, |recording| {
        *recording = updated_recording.clone();
    })?;

    Ok(recording)
}

fn transcribe_recordings_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let settings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not inspect transcription settings.".to_string())?;
        persisted.settings.clone()
    };
    let whisper_detection = refresh_whisper_detection_state(app)?;
    if whisper_detection.status != "ready" {
        return Ok(RecordingBatchResult {
            status: "unavailable".into(),
            message: format!("Whisper is not ready yet: {}", whisper_detection.message),
            items: Vec::new(),
            bootstrap: build_app_bootstrap(app)?,
        });
    }

    let cli_path = PathBuf::from(
        whisper_detection
            .executable_path
            .clone()
            .unwrap_or_default(),
    );
    let model_path = PathBuf::from(whisper_detection.model_path.clone().unwrap_or_default());
    let recordings = selected_untranscribed_recordings(app, file_paths)?;
    let total = recordings.len();
    let mut items = Vec::new();

    for (index, recording) in recordings.into_iter().enumerate() {
        if recording.transcript_path.is_some() {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "skipped".into(),
                message: "Already transcribed.".into(),
                note_id: recording.anki_note_id,
            });
            continue;
        }

        let original_file_path = recording.file_path.clone();
        update_shell_snapshot(app, |shell| {
            shell.phase = "transcribing".into();
            shell.status_text = format!(
                "Transcribing {} of {}: {}",
                index + 1,
                total,
                recording.file_name
            );
            shell.started_at_ms = None;
            shell.current_recording_name = None;
            shell.last_output_path = Some(recording.file_path.clone());
        })?;

        let result = run_whisper_transcription(&WhisperTranscriptionRequest {
            cli_path: cli_path.clone(),
            model_path: model_path.clone(),
            audio_path: PathBuf::from(&recording.file_path),
            language: settings.whisper.language.clone(),
        })
        .and_then(|result| {
            apply_transcription_result_to_recording(
                app,
                &original_file_path,
                recording.clone(),
                result.transcript_path,
                &settings.whisper.language,
            )
        });

        match result {
            Ok(updated_recording) => {
                log_event(
                    app,
                    "INFO",
                    "transcription.saved",
                    serde_json::json!({
                        "audioPath": updated_recording.file_path,
                        "transcriptPath": updated_recording.transcript_path
                    }),
                );
                items.push(RecordingActionItem {
                    file_path: updated_recording.file_path,
                    status: "success".into(),
                    message: "Transcript created. WAV audio was kept for transcription accuracy."
                        .into(),
                    note_id: updated_recording.anki_note_id,
                });
            }
            Err(error) => {
                log_event(
                    app,
                    "ERROR",
                    "transcription.failed",
                    serde_json::json!({
                        "audioPath": original_file_path,
                        "message": error
                    }),
                );
                items.push(RecordingActionItem {
                    file_path: original_file_path,
                    status: "failed".into(),
                    message: error,
                    note_id: None,
                });
            }
        }
    }

    let success_count = items.iter().filter(|item| item.status == "success").count();
    let skipped_count = items.iter().filter(|item| item.status == "skipped").count();
    let failed_count = items.iter().filter(|item| item.status == "failed").count();
    let message = format!(
        "Transcription finished: {success_count} created, {skipped_count} skipped, {failed_count} failed."
    );

    update_shell_snapshot(app, |shell| {
        shell.phase = "idle".into();
        shell.status_text = message.clone();
        shell.started_at_ms = None;
        shell.current_recording_name = None;
        shell.transition_count += 1;
    })?;

    Ok(RecordingBatchResult {
        status: if failed_count == 0 {
            "completed"
        } else {
            "partial"
        }
        .into(),
        message,
        items,
        bootstrap: build_app_bootstrap(app)?,
    })
}

fn push_recordings_to_anki_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let (settings, delete_local_audio_after_push, auto_add_furigana_after_push) = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the Anki settings.".to_string())?;
        (
            persisted.settings.anki.clone(),
            persisted
                .settings
                .features
                .delete_local_audio_after_anki_push,
            persisted
                .settings
                .features
                .auto_add_furigana_after_anki_push,
        )
    };
    push_recordings_to_anki_with_settings_inner(
        app,
        file_paths,
        settings,
        delete_local_audio_after_push,
        auto_add_furigana_after_push,
    )
}

fn push_recordings_to_anki_deck_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
    deck_name: String,
) -> Result<RecordingBatchResult, String> {
    let deck_name = deck_name.trim().to_string();
    if deck_name.is_empty() {
        return Err("Choose an Anki deck before pushing recordings.".into());
    }

    let (mut settings, delete_local_audio_after_push, auto_add_furigana_after_push) = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the Anki settings.".to_string())?;
        (
            persisted.settings.anki.clone(),
            persisted
                .settings
                .features
                .delete_local_audio_after_anki_push,
            persisted
                .settings
                .features
                .auto_add_furigana_after_anki_push,
        )
    };
    settings.deck_name = deck_name;
    push_recordings_to_anki_with_settings_inner(
        app,
        file_paths,
        settings,
        delete_local_audio_after_push,
        auto_add_furigana_after_push,
    )
}

fn push_recordings_to_anki_with_settings_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
    settings: AnkiSettings,
    delete_local_audio_after_push: bool,
    auto_add_furigana_after_push: bool,
) -> Result<RecordingBatchResult, String> {
    let recordings = selected_recordings(app, file_paths)?;
    let mut items = Vec::new();

    if let Err(error) = anki_connect_request("version", serde_json::json!({})) {
        let message = anki_offline_message(&error);
        update_shell_snapshot(app, |shell| {
            shell.status_text = message.clone();
            shell.transition_count += 1;
        })?;
        return Ok(RecordingBatchResult {
            status: "unavailable".into(),
            message,
            items: recordings
                .into_iter()
                .map(|recording| RecordingActionItem {
                    file_path: recording.file_path,
                    status: "failed".into(),
                    message: "Anki is currently offline.".into(),
                    note_id: recording.anki_note_id,
                })
                .collect(),
            bootstrap: build_app_bootstrap(app)?,
        });
    }

    for recording in recordings {
        let original_file_path = recording.file_path.clone();
        let original_note_id = recording.anki_note_id;
        let recording = match refresh_recording_anki_reference(app, recording) {
            Ok(recording) => recording,
            Err(error) => {
                items.push(RecordingActionItem {
                    file_path: original_file_path,
                    status: "failed".into(),
                    message: format!("Could not verify the existing Anki card: {error}"),
                    note_id: original_note_id,
                });
                continue;
            }
        };

        if recording_pushed_to_anki_target(&recording, &settings) {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "skipped".into(),
                message: format!("Already pushed to {}.", settings.deck_name),
                note_id: recording.anki_note_id,
            });
            continue;
        }

        match push_single_recording_to_anki(
            app,
            &recording,
            &settings,
            auto_add_furigana_after_push,
        ) {
            Ok(outcome) => {
                let note_id = outcome.note_id;
                let mut message = format!("Created Anki note {note_id}.");
                if let Some(furigana_message) = outcome.furigana_message {
                    message.push(' ');
                    message.push_str(&furigana_message);
                }
                if delete_local_audio_after_push {
                    match delete_local_audio_after_anki_push(app, &recording.file_path) {
                        Ok(()) => {
                            message.push_str(" Local audio was deleted after Anki copied it.");
                        }
                        Err(error) => {
                            message.push_str(&format!(" {error}"));
                        }
                    }
                }
                items.push(RecordingActionItem {
                    file_path: recording.file_path,
                    status: "success".into(),
                    message,
                    note_id: Some(note_id),
                });
            }
            Err(error) => items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "failed".into(),
                message: error,
                note_id: None,
            }),
        }
    }

    let success_count = items.iter().filter(|item| item.status == "success").count();
    let skipped_count = items.iter().filter(|item| item.status == "skipped").count();
    let failed_count = items.iter().filter(|item| item.status == "failed").count();

    update_shell_snapshot(app, |shell| {
        shell.status_text = format!(
            "Anki push finished: {success_count} created, {skipped_count} skipped, {failed_count} failed."
        );
        shell.transition_count += 1;
    })?;

    Ok(RecordingBatchResult {
        status: if failed_count == 0 { "completed" } else { "partial" }.into(),
        message: format!(
            "Anki push finished: {success_count} created, {skipped_count} skipped, {failed_count} failed."
        ),
        items,
        bootstrap: build_app_bootstrap(app)?,
    })
}

fn add_furigana_to_anki_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let settings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the Anki settings.".to_string())?;
        persisted.settings.anki.clone()
    };

    if settings.fields.transcription.is_empty() {
        return Err(
            "Map the expression/transcript Anki field before updating existing cards.".into(),
        );
    }

    let recordings = selected_recordings(app, file_paths)?;
    let mut items = Vec::new();

    if let Err(error) = anki_connect_request("version", serde_json::json!({})) {
        let message = anki_offline_message(&error);
        update_shell_snapshot(app, |shell| {
            shell.status_text = message.clone();
            shell.transition_count += 1;
        })?;
        return Ok(RecordingBatchResult {
            status: "unavailable".into(),
            message,
            items: recordings
                .into_iter()
                .map(|recording| RecordingActionItem {
                    file_path: recording.file_path,
                    status: "failed".into(),
                    message: "Anki is currently offline.".into(),
                    note_id: recording.anki_note_id,
                })
                .collect(),
            bootstrap: build_app_bootstrap(app)?,
        });
    }

    for recording in recordings {
        let file_path = recording.file_path.clone();
        match add_furigana_to_single_anki_card(app, recording, &settings) {
            Ok(note_id) => items.push(RecordingActionItem {
                file_path,
                status: "success".into(),
                message: format!("Updated Anki note {note_id} with furigana."),
                note_id: Some(note_id),
            }),
            Err((file_path, note_id, error)) => items.push(RecordingActionItem {
                file_path,
                status: "failed".into(),
                message: error,
                note_id,
            }),
        }
    }

    let success_count = items.iter().filter(|item| item.status == "success").count();
    let failed_count = items.iter().filter(|item| item.status == "failed").count();
    let message =
        format!("Furigana update finished: {success_count} updated, {failed_count} failed.");

    update_shell_snapshot(app, |shell| {
        shell.status_text = message.clone();
        shell.transition_count += 1;
    })?;

    Ok(RecordingBatchResult {
        status: if failed_count == 0 {
            "completed"
        } else {
            "partial"
        }
        .into(),
        message,
        items,
        bootstrap: build_app_bootstrap(app)?,
    })
}

fn add_furigana_to_single_anki_card<R: Runtime>(
    app: &AppHandle<R>,
    recording: RecentRecording,
    settings: &AnkiSettings,
) -> Result<i64, (String, Option<i64>, String)> {
    let file_path = recording.file_path.clone();
    let note_id = recording.anki_note_id;

    let Some(note_id) = note_id else {
        return Err((
            file_path,
            None,
            "Push this recording to Anki before adding furigana.".into(),
        ));
    };

    let recording = refresh_recording_anki_reference(app, recording).map_err(|error| {
        (
            file_path.clone(),
            Some(note_id),
            format!("Could not verify the existing Anki card: {error}"),
        )
    })?;
    if recording.anki_note_id.is_none() {
        return Err((
            file_path,
            Some(note_id),
            "The Anki card was deleted. Push this recording again before adding furigana.".into(),
        ));
    }

    let transcript_path = recording.transcript_path.as_deref().ok_or_else(|| {
        (
            file_path.clone(),
            Some(note_id),
            "Transcribe this recording before adding furigana.".into(),
        )
    })?;
    let transcript = fs::read_to_string(transcript_path).map_err(|error| {
        (
            file_path.clone(),
            Some(note_id),
            format!("Could not read transcript: {error}"),
        )
    })?;
    if !recording_transcript_supports_furigana(&recording, &transcript) {
        return Err((
            file_path,
            Some(note_id),
            "Add furigana is only available for Japanese transcripts.".into(),
        ));
    }
    let furigana_html = request_furigana_html(&transcript)
        .map_err(|error| (file_path.clone(), Some(note_id), error))?;
    let target_field = settings.fields.transcription.as_str();
    let existing_furigana_field_value =
        anki_note_field_value(note_id, target_field).map_err(|error| {
            (
                file_path.clone(),
                Some(note_id),
                format!(
                    "Could not read the existing expression field before adding furigana: {error}"
                ),
            )
        })?;
    let media_file_name = anki_media_file_name(&PathBuf::from(&recording.file_path));
    let fallback_sound_tag =
        if !settings.fields.audio.is_empty() && settings.fields.audio == target_field {
            Some(format!("[sound:{media_file_name}]"))
        } else {
            None
        };
    let furigana_html = preserve_anki_sound_tags(
        existing_furigana_field_value.as_deref(),
        &furigana_html,
        fallback_sound_tag.as_deref(),
    );

    let mut fields = serde_json::Map::new();
    fields.insert(
        target_field.to_string(),
        serde_json::Value::String(furigana_html),
    );
    anki_connect_request(
        "updateNoteFields",
        serde_json::json!({
            "note": {
                "id": note_id,
                "fields": fields
            }
        }),
    )
    .map_err(|error| {
        (
            file_path,
            Some(note_id),
            user_friendly_anki_error(&error, settings),
        )
    })?;

    Ok(note_id)
}

fn recording_transcript_supports_furigana(recording: &RecentRecording, transcript: &str) -> bool {
    if is_japanese_transcript_language(recording.transcript_language.as_deref()) {
        return true;
    }

    if recording.transcript_language.is_some() {
        return false;
    }

    transcript_looks_japanese(transcript)
}

fn convert_recordings_to_mp3_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let recordings = selected_recordings(app, file_paths)?;
    let mut items = Vec::new();

    for recording in recordings {
        let original_file_path = recording.file_path.clone();
        let audio_path = PathBuf::from(&recording.file_path);
        let is_wav = audio_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|extension| extension.eq_ignore_ascii_case("wav"))
            .unwrap_or(false);

        if recording.audio_deleted {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "skipped".into(),
                message: "Local audio was already deleted after Anki copied it.".into(),
                note_id: recording.anki_note_id,
            });
            continue;
        }

        if recording.transcript_path.is_none() {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "skipped".into(),
                message: "Transcribe this recording before converting it to MP3.".into(),
                note_id: recording.anki_note_id,
            });
            continue;
        }

        if !is_wav {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "skipped".into(),
                message: "This recording is already MP3 or is not a WAV file.".into(),
                note_id: recording.anki_note_id,
            });
            continue;
        }

        let converted_path = compress_transcribed_audio_if_possible(app, &audio_path);
        let converted_to_mp3 = converted_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|extension| extension.eq_ignore_ascii_case("mp3"))
            .unwrap_or(false);

        if !converted_to_mp3 {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "failed".into(),
                message: "MP3 conversion did not complete. The WAV file was kept.".into(),
                note_id: recording.anki_note_id,
            });
            continue;
        }

        let mut updated_recording = recording.clone();
        updated_recording.file_name = converted_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("recording.mp3")
            .to_string();
        updated_recording.file_path = converted_path.display().to_string();
        updated_recording.bytes_written = fs::metadata(&converted_path)
            .map(|metadata| metadata.len())
            .unwrap_or(updated_recording.bytes_written);

        update_recent_recording(app, &original_file_path, |recording| {
            *recording = updated_recording.clone();
        })?;

        items.push(RecordingActionItem {
            file_path: updated_recording.file_path,
            status: "success".into(),
            message: "Recording converted to MP3.".into(),
            note_id: updated_recording.anki_note_id,
        });
    }

    let success_count = items.iter().filter(|item| item.status == "success").count();
    let skipped_count = items.iter().filter(|item| item.status == "skipped").count();
    let failed_count = items.iter().filter(|item| item.status == "failed").count();

    update_shell_snapshot(app, |shell| {
        shell.status_text = format!(
            "MP3 conversion finished: {success_count} converted, {skipped_count} skipped, {failed_count} failed."
        );
        shell.transition_count += 1;
    })?;

    Ok(RecordingBatchResult {
        status: if failed_count == 0 { "completed" } else { "partial" }.into(),
        message: format!(
            "MP3 conversion finished: {success_count} converted, {skipped_count} skipped, {failed_count} failed."
        ),
        items,
        bootstrap: build_app_bootstrap(app)?,
    })
}

fn delete_recording_inner<R: Runtime>(app: &AppHandle<R>, file_path: &str) -> Result<(), String> {
    let removed_recording = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update the recording history.".to_string())?;
        let index = persisted
            .recent_recordings
            .iter()
            .position(|recording| recording.file_path == file_path)
            .ok_or_else(|| "The selected recording is no longer in the recent list.".to_string())?;
        let removed = persisted.recent_recordings.remove(index);
        let snapshot = persisted.clone();
        drop(persisted);
        write_persisted_data(app, &snapshot)?;
        removed
    };

    for path in [
        Some(removed_recording.file_path.as_str()),
        removed_recording.transcript_path.as_deref(),
        removed_recording.translation_path.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Could not delete {path}: {error}")),
        }
    }

    log_event(
        app,
        "INFO",
        "recording.deleted",
        serde_json::json!({ "filePath": file_path }),
    );
    emit_app_snapshot(app);
    Ok(())
}

fn delete_recordings_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let mut items = Vec::new();

    for file_path in file_paths {
        match delete_recording_inner(app, &file_path) {
            Ok(()) => items.push(RecordingActionItem {
                file_path,
                status: "success".into(),
                message: "Deleted recording files.".into(),
                note_id: None,
            }),
            Err(error) => items.push(RecordingActionItem {
                file_path,
                status: "failed".into(),
                message: error,
                note_id: None,
            }),
        }
    }

    let success_count = items.iter().filter(|item| item.status == "success").count();
    let failed_count = items.iter().filter(|item| item.status == "failed").count();
    let message = format!("Delete finished: {success_count} deleted, {failed_count} failed.");

    update_shell_snapshot(app, |shell| {
        shell.status_text = message.clone();
        shell.transition_count += 1;
    })?;

    Ok(RecordingBatchResult {
        status: if failed_count == 0 {
            "completed"
        } else {
            "partial"
        }
        .into(),
        message,
        items,
        bootstrap: build_app_bootstrap(app)?,
    })
}

fn play_recording_inner<R: Runtime>(app: &AppHandle<R>, file_path: &str) -> Result<(), String> {
    let recording = find_recent_recording(app, file_path)?;
    let path = PathBuf::from(&recording.file_path);
    if !path.exists() {
        return Err("The audio file is missing from disk.".into());
    }

    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("cmd");
        command.creation_flags(CREATE_NO_WINDOW);
        command.arg("/C").arg("start").arg("").arg(&path);
        command.spawn().map_err(|error| error.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|error| error.to_string())?;
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn translate_recordings_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let recordings = selected_recordings(app, file_paths)?;
    let mut items = Vec::new();

    for recording in recordings {
        if recording.translation_path.is_some() {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "skipped".into(),
                message: "Already translated.".into(),
                note_id: recording.anki_note_id,
            });
        } else if recording.transcript_path.is_none() {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "failed".into(),
                message: "No transcript available to translate.".into(),
                note_id: recording.anki_note_id,
            });
        } else {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "failed".into(),
                message: "Translation provider is not configured yet. This will be wired through the translation/extension bridge phase.".into(),
                note_id: recording.anki_note_id,
            });
        }
    }

    let skipped_count = items.iter().filter(|item| item.status == "skipped").count();
    let failed_count = items.iter().filter(|item| item.status == "failed").count();

    Ok(RecordingBatchResult {
        status: if failed_count == 0 {
            "completed"
        } else {
            "unavailable"
        }
        .into(),
        message: format!(
            "Translation request finished: {skipped_count} skipped, {failed_count} unavailable."
        ),
        items,
        bootstrap: build_app_bootstrap(app)?,
    })
}

fn clear_managed_whisper_override<R: Runtime>(
    app: &AppHandle<R>,
    asset_kind: &str,
) -> Result<(), String> {
    let persisted_snapshot = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update the managed Whisper settings.".to_string())?;

        match asset_kind {
            "runtime" => persisted.settings.whisper.cli_path.clear(),
            "model" => persisted.settings.whisper.model_path.clear(),
            _ => {}
        }

        persisted.clone()
    };

    write_persisted_data(app, &persisted_snapshot)
}

fn activate_managed_runtime_version<R: Runtime>(
    app: &AppHandle<R>,
    runtime_version: &str,
) -> Result<(), String> {
    let normalized_version = sanitize_runtime_version(runtime_version);
    let persisted_snapshot = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update the managed Whisper runtime.".to_string())?;
        persisted.settings.whisper.runtime_version = normalized_version;
        persisted.settings.whisper.cli_path.clear();
        persisted.clone()
    };

    write_persisted_data(app, &persisted_snapshot)
}

fn unique_path_with_suffix(directory: &Path, file_stem: &str, suffix: &str) -> PathBuf {
    let sanitized_stem = if file_stem.is_empty() {
        "recording".to_string()
    } else {
        file_stem.to_string()
    };

    let mut attempt = 0usize;
    loop {
        let candidate = if attempt == 0 {
            directory.join(format!("{sanitized_stem}{suffix}"))
        } else {
            directory.join(format!("{sanitized_stem}_{attempt}{suffix}"))
        };

        if !candidate.exists() {
            return candidate;
        }

        attempt += 1;
    }
}

fn derive_transcript_stem(transcript_path: &Path) -> Result<String, String> {
    let transcript = fs::read_to_string(transcript_path).map_err(|error| error.to_string())?;
    let collapsed = transcript.split_whitespace().collect::<Vec<_>>().join(" ");
    let shortened = collapsed.chars().take(10).collect::<String>();
    let sanitized = sanitize_recording_name(&shortened);
    if sanitized.is_empty() {
        return Err("The generated transcript title was empty.".into());
    }

    Ok(sanitized)
}

fn rename_recording_outputs_from_transcript(
    audio_path: &Path,
    transcript_path: &Path,
) -> Result<(PathBuf, PathBuf), String> {
    let parent = audio_path
        .parent()
        .ok_or_else(|| "The saved recording path did not have a parent folder.".to_string())?;
    let new_stem = derive_transcript_stem(transcript_path)?;
    let new_audio_path = unique_path_with_suffix(parent, &new_stem, ".wav");
    let new_transcript_path = unique_path_with_suffix(parent, &new_stem, ".transcript.txt");

    fs::rename(audio_path, &new_audio_path).map_err(|error| error.to_string())?;
    fs::rename(transcript_path, &new_transcript_path).map_err(|error| error.to_string())?;

    Ok((new_audio_path, new_transcript_path))
}

fn compress_transcribed_audio_if_possible<R: Runtime>(
    app: &AppHandle<R>,
    audio_path: &Path,
) -> PathBuf {
    if audio_path
        .extension()
        .and_then(|value| value.to_str())
        .map(|extension| !extension.eq_ignore_ascii_case("wav"))
        .unwrap_or(true)
    {
        return audio_path.to_path_buf();
    }

    let parent = audio_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = audio_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("recording");
    let mp3_path = unique_path_with_suffix(parent, stem, ".mp3");

    let settings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = match persisted_state.0.lock() {
            Ok(persisted) => persisted,
            Err(_) => return audio_path.to_path_buf(),
        };
        persisted.settings.clone()
    };
    let ffmpeg_detection = detect_local_ffmpeg(&settings);
    let executable_path = ffmpeg_detection
        .executable_path
        .clone()
        .unwrap_or_else(|| "ffmpeg".into());

    let mut command = Command::new(&executable_path);
    hide_command_window(&mut command);
    if let Some(ffmpeg_directory) = Path::new(&executable_path).parent() {
        command.current_dir(ffmpeg_directory);
    }
    command
        .arg("-y")
        .arg("-nostdin")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg(audio_path)
        .arg("-map")
        .arg("0:a:0")
        .arg("-vn")
        .arg("-codec:a")
        .arg("libmp3lame")
        .arg("-b:a")
        .arg("128k")
        .arg(&mp3_path);

    let output = match command.output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            log_event(
                app,
                "INFO",
                "audio.compression_skipped",
                serde_json::json!({
                    "audioPath": audio_path,
                    "ffmpegStatus": ffmpeg_detection.status,
                    "message": "FFmpeg was not found. Keeping the WAV recording."
                }),
            );
            return audio_path.to_path_buf();
        }
        Err(error) => {
            log_event(
                app,
                "WARN",
                "audio.compression_failed",
                serde_json::json!({
                    "audioPath": audio_path,
                    "executablePath": executable_path,
                    "message": error.to_string()
                }),
            );
            return audio_path.to_path_buf();
        }
    };

    let mp3_ready = output.status.success()
        && fs::metadata(&mp3_path)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false);

    if !mp3_ready {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let details = [stderr, stdout]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let _ = fs::remove_file(&mp3_path);
        log_event(
            app,
            "WARN",
            "audio.compression_failed",
            serde_json::json!({
                "audioPath": audio_path,
                "targetPath": mp3_path,
                "executablePath": executable_path,
                "statusCode": output.status.code(),
                "message": if details.is_empty() {
                    "ffmpeg did not produce a valid MP3 file.".to_string()
                } else {
                    details
                }
            }),
        );
        return audio_path.to_path_buf();
    }

    match fs::remove_file(audio_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            log_event(
                app,
                "WARN",
                "audio.source_delete_failed",
                serde_json::json!({
                    "audioPath": audio_path,
                    "targetPath": mp3_path,
                    "message": error.to_string()
                }),
            );
        }
    }

    log_event(
        app,
        "INFO",
        "audio.compressed",
        serde_json::json!({
            "sourcePath": audio_path,
            "targetPath": mp3_path
        }),
    );
    mp3_path
}

fn runtime_download_url(runtime_version: &str) -> String {
    format!(
        "https://github.com/ggml-org/whisper.cpp/releases/download/{}/{}",
        sanitize_runtime_version(runtime_version),
        RECOMMENDED_WHISPER_RUNTIME_FILE
    )
}

fn recommended_runtime_archive_path<R: Runtime>(
    app: &AppHandle<R>,
    runtime_version: &str,
) -> Result<PathBuf, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
    let asset_directory = PathBuf::from(&persisted.settings.asset_directory);
    let runtime_directory = asset_directory.join("downloads");
    drop(persisted);

    ensure_directory_exists(&runtime_directory)?;
    Ok(runtime_directory.join(format!(
        "{}-{}",
        sanitize_runtime_version(runtime_version),
        RECOMMENDED_WHISPER_RUNTIME_FILE
    )))
}

fn recommended_runtime_install_directory<R: Runtime>(
    app: &AppHandle<R>,
    runtime_version: &str,
) -> Result<PathBuf, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
    let asset_directory = PathBuf::from(&persisted.settings.asset_directory);
    drop(persisted);

    let runtime_directory = app_managed_runtime_directory(&asset_directory, runtime_version);
    ensure_directory_exists(&runtime_directory)?;
    Ok(runtime_directory)
}

fn find_existing_managed_cli_path(
    asset_directory: &Path,
    runtime_version: &str,
) -> Option<PathBuf> {
    collect_managed_whisper_cli_candidates(asset_directory, runtime_version)
        .into_iter()
        .find(|candidate| candidate.exists())
}

fn recommended_model_target_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
    let asset_directory = PathBuf::from(&persisted.settings.asset_directory);
    let model_choice = whisper_model_spec(&persisted.settings.whisper.model_choice);
    let models_directory = asset_directory.join("models");
    drop(persisted);

    ensure_directory_exists(&models_directory)?;
    Ok(models_directory.join(model_choice.file_name))
}

fn recommended_ffmpeg_archive_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
    let asset_directory = PathBuf::from(&persisted.settings.asset_directory);
    let downloads_directory = asset_directory.join("downloads");
    drop(persisted);

    ensure_directory_exists(&downloads_directory)?;
    Ok(downloads_directory.join(RECOMMENDED_FFMPEG_RUNTIME_FILE))
}

fn recommended_ffmpeg_install_directory<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
    let asset_directory = PathBuf::from(&persisted.settings.asset_directory);
    drop(persisted);

    let install_directory = managed_ffmpeg_install_directory(&asset_directory);
    ensure_directory_exists(&install_directory)?;
    Ok(install_directory)
}

fn find_existing_managed_ffmpeg_path(asset_directory: &Path) -> Option<PathBuf> {
    collect_managed_ffmpeg_candidates(asset_directory)
        .into_iter()
        .find(|candidate| candidate.exists())
}

fn remove_directory_contents(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let entry_path = entry.path();
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            fs::remove_dir_all(&entry_path).map_err(|error| error.to_string())?;
        } else {
            fs::remove_file(&entry_path).map_err(|error| error.to_string())?;
        }
    }

    Ok(())
}

fn extract_zip_archive_to_directory(
    archive_path: &Path,
    target_directory: &Path,
) -> Result<(), String> {
    ensure_directory_exists(target_directory)?;
    remove_directory_contents(target_directory)?;

    let archive_file = fs::File::open(archive_path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(archive_file).map_err(|error| error.to_string())?;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let Some(relative_path) = entry.enclosed_name() else {
            continue;
        };

        let output_path = target_directory.join(relative_path);
        if entry.is_dir() {
            ensure_directory_exists(&output_path)?;
            continue;
        }

        if let Some(parent) = output_path.parent() {
            ensure_directory_exists(parent)?;
        }

        let mut output_file = fs::File::create(&output_path).map_err(|error| error.to_string())?;
        std::io::copy(&mut entry, &mut output_file).map_err(|error| error.to_string())?;
    }

    Ok(())
}

fn download_file_to_path_with_progress<R: Runtime>(
    app: &AppHandle<R>,
    url: &str,
    target_path: &Path,
    kind: &str,
    label: &str,
) -> Result<(), String> {
    let client = http_client()?;
    let mut response = client.get(url).send().map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("Download failed with status {}", response.status()));
    }

    let total_bytes = response.content_length();
    let temp_path = target_path.with_extension("part");
    let mut file = std::fs::File::create(&temp_path).map_err(|error| error.to_string())?;
    let mut buffer = [0u8; 64 * 1024];
    let mut downloaded_bytes = 0u64;

    update_model_download_snapshot(app, |snapshot| {
        snapshot.kind = Some(kind.to_string());
        snapshot.status = "downloading".into();
        snapshot.message = format!("Downloading {label}...");
        snapshot.downloaded_bytes = 0;
        snapshot.total_bytes = total_bytes;
        snapshot.progress_percent = total_bytes.map(|_| 0.0);
        snapshot.target_path = Some(target_path.display().to_string());
    })?;

    loop {
        {
            let control_state = app.state::<ModelDownloadControlState>();
            let mut control = control_state
                .control
                .lock()
                .map_err(|_| "Could not inspect the model download state.".to_string())?;

            while control.active && control.paused && !control.cancel_requested {
                drop(control);
                update_model_download_snapshot(app, |snapshot| {
                    snapshot.kind = Some(kind.to_string());
                    snapshot.status = "paused".into();
                    snapshot.message = format!("{label} download paused.");
                })?;
                control =
                    control_state
                        .condvar
                        .wait(control_state.control.lock().map_err(|_| {
                            "Could not resume the model download state.".to_string()
                        })?)
                        .map_err(|_| "Could not resume the model download state.".to_string())?;
            }

            if control.cancel_requested {
                drop(control);
                let _ = fs::remove_file(&temp_path);
                update_model_download_snapshot(app, |snapshot| {
                    snapshot.kind = Some(kind.to_string());
                    snapshot.status = "cancelled".into();
                    snapshot.message = format!("{label} download cancelled.");
                })?;
                reset_model_download_control(app)?;
                return Err(format!("{label} download cancelled."));
            }
        }

        let read_bytes = response
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read_bytes == 0 {
            break;
        }

        file.write_all(&buffer[..read_bytes])
            .map_err(|error| error.to_string())?;
        downloaded_bytes = downloaded_bytes.saturating_add(read_bytes as u64);

        update_model_download_snapshot(app, |snapshot| {
            snapshot.kind = Some(kind.to_string());
            snapshot.status = "downloading".into();
            snapshot.message = format!("Downloading {label}...");
            snapshot.downloaded_bytes = downloaded_bytes;
            snapshot.total_bytes = total_bytes;
            snapshot.progress_percent = total_bytes.map(|total| {
                if total == 0 {
                    0.0
                } else {
                    (downloaded_bytes as f64 / total as f64) * 100.0
                }
            });
            snapshot.target_path = Some(target_path.display().to_string());
        })?;
    }

    fs::rename(&temp_path, target_path).map_err(|error| error.to_string())?;
    Ok(())
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

fn download_recommended_whisper_model_inner<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    {
        let shell_state = app.state::<SharedShellState>();
        let shell = shell_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the shell state.".to_string())?;
        if shell.phase != "idle" && shell.phase != "error" {
            return Err("Finish the current task before downloading the Whisper model.".into());
        }
    }

    {
        let control_state = app.state::<ModelDownloadControlState>();
        let mut control = control_state
            .control
            .lock()
            .map_err(|_| "Could not initialize the model download control state.".to_string())?;
        if control.active {
            return Err("A model download is already in progress.".into());
        }
        control.active = true;
        control.paused = false;
        control.cancel_requested = false;
    }

    let target_path = recommended_model_target_path(app)?;
    let model_spec = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the current app settings.".to_string())?;
        *whisper_model_spec(&persisted.settings.whisper.model_choice)
    };
    let app_handle = app.clone();

    update_shell_snapshot(app, |shell| {
        shell.phase = "downloading-model".into();
        shell.status_text = format!(
            "Downloading the {} Whisper model to {}...",
            model_spec.label,
            target_path.display()
        );
        shell.started_at_ms = None;
        shell.current_recording_name = None;
    })?;
    update_model_download_snapshot(app, |snapshot| {
        snapshot.kind = Some("model".into());
        snapshot.status = "starting".into();
        snapshot.message = format!("Preparing the {} model download...", model_spec.label);
        snapshot.downloaded_bytes = 0;
        snapshot.total_bytes = None;
        snapshot.progress_percent = None;
        snapshot.target_path = Some(target_path.display().to_string());
    })?;

    std::thread::Builder::new()
        .name("whisper-model-download".into())
        .spawn(move || {
            let download_result = (|| -> Result<(), String> {
                if !target_path.exists() {
                    download_file_to_path_with_progress(
                        &app_handle,
                        model_spec.download_url,
                        &target_path,
                        "model",
                        &format!("the {} Whisper model", model_spec.label),
                    )?;
                }
                verify_whisper_model(&target_path)?;
                clear_managed_whisper_override(&app_handle, "model")?;
                let detection = refresh_whisper_detection_state(&app_handle)?;
                update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some("model".into());
                    snapshot.status = "completed".into();
                    snapshot.message =
                        format!("{} model downloaded successfully.", model_spec.label);
                    snapshot.downloaded_bytes =
                        snapshot.total_bytes.unwrap_or(snapshot.downloaded_bytes);
                    snapshot.progress_percent = Some(100.0);
                    snapshot.target_path = Some(target_path.display().to_string());
                })?;
                reset_model_download_control(&app_handle)?;

                update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = if detection.status == "ready" {
                        format!(
                            "{} model is ready at {}",
                            model_spec.label,
                            target_path.display()
                        )
                    } else {
                        format!(
                            "Model downloaded, but Whisper still needs setup: {}",
                            detection.message
                        )
                    };
                    shell.started_at_ms = None;
                })?;

                log_event(
                    &app_handle,
                    "INFO",
                    "whisper.model_downloaded",
                    serde_json::json!({
                        "targetPath": target_path.display().to_string(),
                        "modelChoice": model_spec.id
                    }),
                );
                Ok(())
            })();

            if let Err(error) = download_result {
                let cancelled = error.ends_with("download cancelled.");
                let _ = update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some("model".into());
                    if cancelled {
                        snapshot.status = "cancelled".into();
                        snapshot.message = "Model download cancelled.".into();
                    } else {
                        snapshot.status = "failed".into();
                        snapshot.message = format!("Model download failed: {error}");
                    }
                });
                let _ = reset_model_download_control(&app_handle);
                let _ = update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = if cancelled {
                        "Whisper model download cancelled.".into()
                    } else {
                        format!("Whisper model download failed: {error}")
                    };
                    shell.started_at_ms = None;
                });
                log_event(
                    &app_handle,
                    "ERROR",
                    "whisper.model_download_failed",
                    serde_json::json!({ "message": error }),
                );
            }
        })
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn download_recommended_whisper_runtime_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
    download_whisper_runtime_version_inner(app, RECOMMENDED_WHISPER_RUNTIME_VERSION)
}

fn download_whisper_runtime_version_inner<R: Runtime>(
    app: &AppHandle<R>,
    runtime_version: &str,
) -> Result<(), String> {
    let runtime_version = sanitize_runtime_version(runtime_version);
    {
        let shell_state = app.state::<SharedShellState>();
        let shell = shell_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the shell state.".to_string())?;
        if shell.phase != "idle" && shell.phase != "error" {
            return Err("Finish the current task before downloading the Whisper runtime.".into());
        }
    }

    {
        let control_state = app.state::<ModelDownloadControlState>();
        let mut control = control_state
            .control
            .lock()
            .map_err(|_| "Could not initialize the download control state.".to_string())?;
        if control.active {
            return Err("Another download is already in progress.".into());
        }
        control.active = true;
        control.paused = false;
        control.cancel_requested = false;
    }

    let archive_path = recommended_runtime_archive_path(app, &runtime_version)?;
    let install_directory = recommended_runtime_install_directory(app, &runtime_version)?;
    let download_url = runtime_download_url(&runtime_version);
    let app_handle = app.clone();

    update_shell_snapshot(app, |shell| {
        shell.phase = "downloading-model".into();
        shell.status_text = format!(
            "Downloading Whisper runtime {} to {}...",
            runtime_version,
            install_directory.display()
        );
        shell.started_at_ms = None;
        shell.current_recording_name = None;
    })?;
    update_model_download_snapshot(app, |snapshot| {
        snapshot.kind = Some("runtime".into());
        snapshot.status = "starting".into();
        snapshot.message = "Preparing the Whisper runtime download...".into();
        snapshot.downloaded_bytes = 0;
        snapshot.total_bytes = None;
        snapshot.progress_percent = None;
        snapshot.target_path = Some(archive_path.display().to_string());
    })?;

    std::thread::Builder::new()
        .name("whisper-runtime-download".into())
        .spawn(move || {
            let download_result = (|| -> Result<(), String> {
                let asset_directory = {
                    let persisted_state = app_handle.state::<SharedPersistedState>();
                    let persisted = persisted_state
                        .0
                        .lock()
                        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
                    PathBuf::from(&persisted.settings.asset_directory)
                };

                let cli_path = if let Some(existing_cli_path) =
                    find_existing_managed_cli_path(&asset_directory, &runtime_version)
                {
                    verify_whisper_cli(&existing_cli_path)?;
                    existing_cli_path
                } else {
                    download_file_to_path_with_progress(
                        &app_handle,
                        &download_url,
                        &archive_path,
                        "runtime",
                        &format!("Whisper runtime {runtime_version}"),
                    )?;

                    extract_zip_archive_to_directory(&archive_path, &install_directory)?;
                    find_existing_managed_cli_path(&asset_directory, &runtime_version).ok_or_else(
                        || "The runtime downloaded, but whisper-cli.exe was not found.".to_string(),
                    )?
                };
                verify_whisper_cli(&cli_path)?;
                activate_managed_runtime_version(&app_handle, &runtime_version)?;

                let detection = refresh_whisper_detection_state(&app_handle)?;
                update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some("runtime".into());
                    snapshot.status = "completed".into();
                    snapshot.message = format!(
                        "Whisper runtime {} downloaded and activated.",
                        runtime_version
                    );
                    snapshot.downloaded_bytes =
                        snapshot.total_bytes.unwrap_or(snapshot.downloaded_bytes);
                    snapshot.progress_percent = Some(100.0);
                    snapshot.target_path = Some(cli_path.display().to_string());
                })?;
                reset_model_download_control(&app_handle)?;

                update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = if detection.status == "ready" {
                        format!(
                            "Whisper runtime {} is ready at {}",
                            runtime_version,
                            cli_path.display()
                        )
                    } else {
                        format!(
                            "Runtime downloaded, but Whisper still needs setup: {}",
                            detection.message
                        )
                    };
                    shell.started_at_ms = None;
                })?;

                log_event(
                    &app_handle,
                    "INFO",
                    "whisper.runtime_downloaded",
                    serde_json::json!({
                        "runtimeArchivePath": archive_path.display().to_string(),
                        "cliPath": cli_path.display().to_string(),
                        "runtimeVersion": runtime_version
                    }),
                );

                let _ = fs::remove_file(&archive_path);
                Ok(())
            })();

            if let Err(error) = download_result {
                let cancelled = error.ends_with("download cancelled.");
                let _ = update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some("runtime".into());
                    if cancelled {
                        snapshot.status = "cancelled".into();
                        snapshot.message = "Runtime download cancelled.".into();
                    } else {
                        snapshot.status = "failed".into();
                        snapshot.message = format!("Runtime download failed: {error}");
                    }
                });
                let _ = reset_model_download_control(&app_handle);
                let _ = update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = if cancelled {
                        "Whisper runtime download cancelled.".into()
                    } else {
                        format!("Whisper runtime download failed: {error}")
                    };
                    shell.started_at_ms = None;
                });
                log_event(
                    &app_handle,
                    "ERROR",
                    "whisper.runtime_download_failed",
                    serde_json::json!({ "message": error }),
                );
            }
        })
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn download_recommended_ffmpeg_inner<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    {
        let shell_state = app.state::<SharedShellState>();
        let shell = shell_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the shell state.".to_string())?;
        if shell.phase != "idle" && shell.phase != "error" {
            return Err("Finish the current task before downloading FFmpeg.".into());
        }
    }

    {
        let control_state = app.state::<ModelDownloadControlState>();
        let mut control = control_state
            .control
            .lock()
            .map_err(|_| "Could not initialize the download control state.".to_string())?;
        if control.active {
            return Err("Another download is already in progress.".into());
        }
        control.active = true;
        control.paused = false;
        control.cancel_requested = false;
    }

    let archive_path = recommended_ffmpeg_archive_path(app)?;
    let install_directory = recommended_ffmpeg_install_directory(app)?;
    let app_handle = app.clone();

    update_shell_snapshot(app, |shell| {
        shell.phase = "downloading-model".into();
        shell.status_text = format!("Downloading FFmpeg to {}...", install_directory.display());
        shell.started_at_ms = None;
        shell.current_recording_name = None;
    })?;
    update_model_download_snapshot(app, |snapshot| {
        snapshot.kind = Some("ffmpeg".into());
        snapshot.status = "starting".into();
        snapshot.message = "Preparing the FFmpeg download...".into();
        snapshot.downloaded_bytes = 0;
        snapshot.total_bytes = None;
        snapshot.progress_percent = None;
        snapshot.target_path = Some(archive_path.display().to_string());
    })?;

    std::thread::Builder::new()
        .name("ffmpeg-download".into())
        .spawn(move || {
            let download_result = (|| -> Result<(), String> {
                let asset_directory = {
                    let persisted_state = app_handle.state::<SharedPersistedState>();
                    let persisted = persisted_state
                        .0
                        .lock()
                        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
                    PathBuf::from(&persisted.settings.asset_directory)
                };

                let ffmpeg_path = if let Some(existing_path) =
                    find_existing_managed_ffmpeg_path(&asset_directory)
                {
                    verify_ffmpeg_binary(&existing_path)?;
                    existing_path
                } else {
                    download_file_to_path_with_progress(
                        &app_handle,
                        RECOMMENDED_FFMPEG_RUNTIME_URL,
                        &archive_path,
                        "ffmpeg",
                        "FFmpeg",
                    )?;

                    extract_zip_archive_to_directory(&archive_path, &install_directory)?;
                    find_existing_managed_ffmpeg_path(&asset_directory)
                        .ok_or_else(|| "FFmpeg downloaded, but ffmpeg.exe was not found.".to_string())?
                };

                verify_ffmpeg_binary(&ffmpeg_path)?;
                update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some("ffmpeg".into());
                    snapshot.status = "completed".into();
                    snapshot.message =
                        "FFmpeg downloaded. MP3 compression is now enabled.".into();
                    snapshot.downloaded_bytes =
                        snapshot.total_bytes.unwrap_or(snapshot.downloaded_bytes);
                    snapshot.progress_percent = Some(100.0);
                    snapshot.target_path = Some(ffmpeg_path.display().to_string());
                })?;
                reset_model_download_control(&app_handle)?;

                update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = format!(
                        "FFmpeg is ready at {}. Future transcribed recordings will be compressed to MP3.",
                        ffmpeg_path.display()
                    );
                    shell.started_at_ms = None;
                })?;

                log_event(
                    &app_handle,
                    "INFO",
                    "ffmpeg.downloaded",
                    serde_json::json!({
                        "archivePath": archive_path.display().to_string(),
                        "ffmpegPath": ffmpeg_path.display().to_string()
                    }),
                );

                let _ = fs::remove_file(&archive_path);
                Ok(())
            })();

            if let Err(error) = download_result {
                let cancelled = error.ends_with("download cancelled.");
                let _ = update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some("ffmpeg".into());
                    if cancelled {
                        snapshot.status = "cancelled".into();
                        snapshot.message = "FFmpeg download cancelled.".into();
                    } else {
                        snapshot.status = "failed".into();
                        snapshot.message = format!("FFmpeg download failed: {error}");
                    }
                });
                let _ = reset_model_download_control(&app_handle);
                let _ = update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = if cancelled {
                        "FFmpeg download cancelled.".into()
                    } else {
                        format!("FFmpeg download failed: {error}")
                    };
                    shell.started_at_ms = None;
                });
                log_event(
                    &app_handle,
                    "ERROR",
                    "ffmpeg.download_failed",
                    serde_json::json!({ "message": error }),
                );
            }
        })
        .map_err(|error| error.to_string())?;

    Ok(())
}

fn toggle_whisper_model_download_pause_inner<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
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

fn cancel_whisper_model_download_inner<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
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
            _ => "model",
        }
    };

    update_model_download_snapshot(app, |snapshot| {
        snapshot.status = "cancelling".into();
        snapshot.message = format!("Cancelling the {download_label} download...");
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
        recording_pushed_to_anki_target, recording_transcript_supports_furigana,
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
