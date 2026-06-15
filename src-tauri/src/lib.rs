mod recording;
mod transcription;

use std::{
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Condvar, Mutex,
    },
    thread::JoinHandle,
    time::{SystemTime, UNIX_EPOCH},
};

use recording::{capture_system_audio_loopback, RecordingCaptureResult};
use serde::{Deserialize, Serialize};
use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, Runtime, WindowEvent,
};
#[cfg(desktop)]
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
use tauri_plugin_notification::NotificationExt;
use transcription::{
    run_whisper_transcription, verify_whisper_cli, verify_whisper_model,
    WhisperTranscriptionRequest,
};
use zip::ZipArchive;

const START_SHORTCUT: &str = "Ctrl+Alt+R";
const STOP_SHORTCUT: &str = "Ctrl+Alt+S";
const SHOW_SHORTCUT: &str = "Ctrl+Alt+W";
const APP_SNAPSHOT_EVENT: &str = "app://snapshot-changed";
const START_SHORTCUT_CANDIDATES: [&str; 3] = [START_SHORTCUT, "Ctrl+Alt+Shift+R", "Ctrl+Alt+F8"];
const STOP_SHORTCUT_CANDIDATES: [&str; 3] = [STOP_SHORTCUT, "Ctrl+Alt+Shift+S", "Ctrl+Alt+F9"];
const SHOW_SHORTCUT_CANDIDATES: [&str; 3] = [SHOW_SHORTCUT, "Ctrl+Alt+Shift+W", "Ctrl+Alt+F10"];
const RECENT_RECORDINGS_LIMIT: usize = 10;
const WHISPER_RELEASES_API_URL: &str =
    "https://api.github.com/repos/ggml-org/whisper.cpp/releases/latest";
const RECOMMENDED_WHISPER_RUNTIME_VERSION: &str = "v1.8.4";
const RECOMMENDED_WHISPER_RUNTIME_FILE: &str = "whisper-bin-x64.zip";
const RECOMMENDED_WHISPER_RUNTIME_URL: &str =
    "https://github.com/ggml-org/whisper.cpp/releases/download/v1.8.4/whisper-bin-x64.zip";

#[derive(Copy, Clone)]
struct WhisperModelSpec {
    id: &'static str,
    label: &'static str,
    file_name: &'static str,
    download_url: &'static str,
}

const WHISPER_MODEL_SPECS: [WhisperModelSpec; 5] = [
    WhisperModelSpec {
        id: "tiny",
        label: "Tiny",
        file_name: "ggml-tiny.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-tiny.bin",
    },
    WhisperModelSpec {
        id: "base",
        label: "Base",
        file_name: "ggml-base.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-base.bin",
    },
    WhisperModelSpec {
        id: "small",
        label: "Small",
        file_name: "ggml-small.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-small.bin",
    },
    WhisperModelSpec {
        id: "medium",
        label: "Medium",
        file_name: "ggml-medium.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-medium.bin",
    },
    WhisperModelSpec {
        id: "large-v3",
        label: "Large v3",
        file_name: "ggml-large-v3.bin",
        download_url: "https://huggingface.co/ggerganov/whisper.cpp/resolve/main/ggml-large-v3.bin",
    },
];

fn default_whisper_model_id() -> &'static str {
    "small"
}

fn default_whisper_model_choice() -> String {
    default_whisper_model_id().to_string()
}

fn whisper_model_spec(model_id: &str) -> &'static WhisperModelSpec {
    WHISPER_MODEL_SPECS
        .iter()
        .find(|spec| spec.id == model_id)
        .unwrap_or(&WHISPER_MODEL_SPECS[2])
}

#[derive(Copy, Clone)]
enum HotkeyAction {
    Start,
    Stop,
    ShowWindow,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FeatureSettings {
    transcription: bool,
}

impl Default for FeatureSettings {
    fn default() -> Self {
        Self {
            transcription: true,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WhisperSettings {
    cli_path: String,
    model_path: String,
    #[serde(default = "default_whisper_model_choice")]
    model_choice: String,
    language: String,
}

impl Default for WhisperSettings {
    fn default() -> Self {
        Self {
            cli_path: String::new(),
            model_path: String::new(),
            model_choice: default_whisper_model_id().into(),
            language: "auto".into(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSettings {
    output_directory: String,
    asset_directory: String,
    #[serde(default)]
    whisper: WhisperSettings,
    #[serde(default)]
    features: FeatureSettings,
    #[serde(default)]
    launch_at_login: bool,
    #[serde(default)]
    start_minimized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecentRecording {
    file_name: String,
    file_path: String,
    #[serde(default)]
    transcript_path: Option<String>,
    duration_ms: u64,
    bytes_written: u64,
    created_at_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct PersistedData {
    settings: AppSettings,
    recent_recordings: Vec<RecentRecording>,
    untitled_counter: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct HotkeyBindings {
    start: String,
    stop: String,
    show_window: String,
}

impl Default for HotkeyBindings {
    fn default() -> Self {
        Self {
            start: START_SHORTCUT.to_string(),
            stop: STOP_SHORTCUT.to_string(),
            show_window: SHOW_SHORTCUT.to_string(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ShellSnapshot {
    phase: String,
    status_text: String,
    last_shortcut: Option<String>,
    transition_count: u32,
    hotkeys: HotkeyBindings,
    started_at_ms: Option<u64>,
    current_recording_name: Option<String>,
    last_output_path: Option<String>,
    last_transcript_path: Option<String>,
}

impl Default for ShellSnapshot {
    fn default() -> Self {
        Self {
            phase: "idle".into(),
            status_text: "Tray shell is ready. Press Ctrl+Alt+R to start recording system audio."
                .into(),
            last_shortcut: None,
            transition_count: 0,
            hotkeys: HotkeyBindings::default(),
            started_at_ms: None,
            current_recording_name: None,
            last_output_path: None,
            last_transcript_path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppBootstrap {
    shell: ShellSnapshot,
    settings: AppSettings,
    recent_recordings: Vec<RecentRecording>,
    whisper_detection: WhisperDetection,
    model_download: ModelDownloadSnapshot,
    log_path: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WhisperDetection {
    status: String,
    executable_path: Option<String>,
    model_path: Option<String>,
    source: Option<String>,
    model_source: Option<String>,
    cli_ready: bool,
    model_ready: bool,
    cli_managed: bool,
    model_managed: bool,
    message: String,
}

impl Default for WhisperDetection {
    fn default() -> Self {
        Self {
            status: "notFound".into(),
            executable_path: None,
            model_path: None,
            source: None,
            model_source: None,
            cli_ready: false,
            model_ready: false,
            cli_managed: false,
            model_managed: false,
            message:
                "Add or download whisper-cli and a Whisper model to enable offline transcription."
                    .into(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct WhisperAssetUpdateResult {
    kind: String,
    status: String,
    message: String,
    current_version: Option<String>,
    latest_version: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct ModelDownloadSnapshot {
    kind: Option<String>,
    status: String,
    message: String,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    progress_percent: Option<f64>,
    target_path: Option<String>,
}

impl Default for ModelDownloadSnapshot {
    fn default() -> Self {
        Self {
            kind: None,
            status: "idle".into(),
            message: "No download in progress.".into(),
            downloaded_bytes: 0,
            total_bytes: None,
            progress_percent: None,
            target_path: None,
        }
    }
}

#[derive(Clone)]
struct AppPathsState {
    data_dir: PathBuf,
    state_file: PathBuf,
    log_file: PathBuf,
    assets_dir: PathBuf,
}

struct SharedShellState(Mutex<ShellSnapshot>);
struct SharedPersistedState(Mutex<PersistedData>);
struct WhisperDetectionState(Mutex<WhisperDetection>);
struct ModelDownloadState(Mutex<ModelDownloadSnapshot>);
struct ModelDownloadControlState {
    control: Mutex<ModelDownloadControl>,
    condvar: Condvar,
}
struct RecorderState(Mutex<Option<ActiveRecording>>);

#[derive(Default)]
struct ModelDownloadControl {
    active: bool,
    paused: bool,
    cancel_requested: bool,
}

struct ActiveRecording {
    stop_signal: Arc<AtomicBool>,
    worker: JoinHandle<Result<RecordingCaptureResult, String>>,
}

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
fn check_whisper_runtime_update(app: AppHandle) -> Result<WhisperAssetUpdateResult, String> {
    check_whisper_runtime_update_inner(&app)
}

#[tauri::command]
fn check_whisper_model_update(app: AppHandle) -> Result<WhisperAssetUpdateResult, String> {
    check_whisper_model_update_inner(&app)
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
fn save_settings(app: AppHandle, settings: AppSettings) -> Result<AppBootstrap, String> {
    save_settings_inner(&app, settings)?;
    build_app_bootstrap(&app)
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

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn build_app_paths<R: Runtime>(app: &AppHandle<R>) -> Result<AppPathsState, tauri::Error> {
    let data_dir = app.path().app_local_data_dir()?;
    let log_dir = app.path().app_log_dir()?;
    let assets_dir = data_dir.join("assets");

    fs::create_dir_all(&data_dir)?;
    fs::create_dir_all(&log_dir)?;
    fs::create_dir_all(&assets_dir)?;

    Ok(AppPathsState {
        state_file: data_dir.join("state.json"),
        log_file: log_dir.join("wonder-of-u.log"),
        data_dir,
        assets_dir,
    })
}

fn default_output_directory<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, tauri::Error> {
    let base = app
        .path()
        .document_dir()
        .or_else(|_| app.path().download_dir())
        .or_else(|_| app.path().home_dir())?;

    Ok(base.join("Wonder of U Recordings"))
}

fn default_asset_directory(paths: &AppPathsState) -> PathBuf {
    paths.assets_dir.clone()
}

fn default_settings<R: Runtime>(
    app: &AppHandle<R>,
    paths: &AppPathsState,
) -> Result<AppSettings, tauri::Error> {
    Ok(AppSettings {
        output_directory: default_output_directory(app)?.display().to_string(),
        asset_directory: default_asset_directory(paths).display().to_string(),
        whisper: WhisperSettings::default(),
        features: FeatureSettings::default(),
        launch_at_login: false,
        start_minimized: false,
    })
}

fn load_persisted_data<R: Runtime>(
    app: &AppHandle<R>,
    paths: &AppPathsState,
) -> Result<PersistedData, tauri::Error> {
    let defaults = default_settings(app, paths)?;

    let mut state = match fs::read_to_string(&paths.state_file) {
        Ok(raw) => serde_json::from_str::<PersistedData>(&raw).unwrap_or(PersistedData {
            settings: defaults.clone(),
            recent_recordings: Vec::new(),
            untitled_counter: 1,
        }),
        Err(_) => PersistedData {
            settings: defaults.clone(),
            recent_recordings: Vec::new(),
            untitled_counter: 1,
        },
    };

    state.settings = normalize_settings(app, paths, state.settings)?;
    state.recent_recordings.truncate(RECENT_RECORDINGS_LIMIT);
    if state.untitled_counter == 0 {
        state.untitled_counter = 1;
    }

    Ok(state)
}

fn normalize_settings<R: Runtime>(
    app: &AppHandle<R>,
    paths: &AppPathsState,
    settings: AppSettings,
) -> Result<AppSettings, tauri::Error> {
    let output_directory =
        normalize_directory_input(&settings.output_directory, &default_output_directory(app)?);
    let asset_directory =
        normalize_directory_input(&settings.asset_directory, &default_asset_directory(paths));
    let language = settings.whisper.language.trim();
    let model_choice = whisper_model_spec(settings.whisper.model_choice.trim()).id;
    let managed_cli_candidates = collect_managed_whisper_cli_candidates(&asset_directory);
    let managed_model_candidates = all_managed_model_paths(&asset_directory);
    let cli_path = settings.whisper.cli_path.trim();
    let model_path = settings.whisper.model_path.trim();

    let normalized_cli_path = if cli_path.is_empty() {
        String::new()
    } else {
        let candidate = PathBuf::from(cli_path);
        if managed_cli_candidates
            .iter()
            .any(|managed| managed == &candidate)
        {
            String::new()
        } else {
            cli_path.to_string()
        }
    };
    let normalized_model_path = if model_path.is_empty() {
        String::new()
    } else {
        let candidate = PathBuf::from(model_path);
        if managed_model_candidates
            .iter()
            .any(|managed| managed == &candidate)
        {
            String::new()
        } else {
            model_path.to_string()
        }
    };

    Ok(AppSettings {
        output_directory: output_directory.display().to_string(),
        asset_directory: asset_directory.display().to_string(),
        whisper: WhisperSettings {
            cli_path: normalized_cli_path,
            model_path: normalized_model_path,
            model_choice: model_choice.to_string(),
            language: if language.is_empty() {
                "auto".into()
            } else {
                language.to_string()
            },
        },
        features: FeatureSettings {
            transcription: settings.features.transcription,
        },
        launch_at_login: settings.launch_at_login,
        start_minimized: settings.start_minimized,
    })
}

fn normalize_directory_input(input: &str, fallback: &Path) -> PathBuf {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return fallback.to_path_buf();
    }

    let candidate = PathBuf::from(trimmed);
    if candidate.is_absolute() {
        candidate
    } else {
        fallback.join(candidate)
    }
}

fn sanitize_recording_name(name: &str) -> String {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return String::new();
    }

    trimmed
        .chars()
        .map(|character| match character {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => ' ',
            c if c.is_control() => ' ',
            c => c,
        })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .trim_end_matches('.')
        .trim()
        .to_string()
}

fn next_recording_stem(state: &mut PersistedData, requested_name: Option<&str>) -> String {
    let requested = requested_name
        .map(sanitize_recording_name)
        .unwrap_or_default();

    if !requested.is_empty() {
        return requested;
    }

    let stem = format!("recording_{}", state.untitled_counter.max(1));
    state.untitled_counter = state.untitled_counter.max(1) + 1;
    stem
}

fn unique_wav_path(directory: &Path, file_stem: &str) -> PathBuf {
    let sanitized_stem = if file_stem.is_empty() {
        "recording".to_string()
    } else {
        file_stem.to_string()
    };

    let mut attempt = 0usize;
    loop {
        let candidate = if attempt == 0 {
            directory.join(format!("{sanitized_stem}.wav"))
        } else {
            directory.join(format!("{sanitized_stem}_{attempt}.wav"))
        };

        if !candidate.exists() {
            return candidate;
        }

        attempt += 1;
    }
}

fn write_persisted_data<R: Runtime>(
    app: &AppHandle<R>,
    state: &PersistedData,
) -> Result<(), String> {
    let paths = app.state::<AppPathsState>().inner().clone();
    let serialized = serde_json::to_string_pretty(state).map_err(|error| error.to_string())?;
    fs::write(&paths.state_file, serialized).map_err(|error| error.to_string())
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

fn show_native_notification<R: Runtime>(app: &AppHandle<R>, title: &str, body: &str) {
    let _ = app.notification().builder().title(title).body(body).show();
}

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent("Wonder of U Desktop/0.1.0")
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

fn app_managed_runtime_directory(asset_directory: &Path) -> PathBuf {
    asset_directory
        .join("whisper-runtime")
        .join(RECOMMENDED_WHISPER_RUNTIME_VERSION)
}

fn collect_managed_whisper_cli_candidates(asset_directory: &Path) -> Vec<PathBuf> {
    let executable_names = ["whisper-cli.exe", "whisper-cli"];
    let mut candidates = Vec::new();
    let runtime_directory = app_managed_runtime_directory(asset_directory);

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

fn collect_managed_whisper_model_candidates(
    asset_directory: &Path,
    executable_path: Option<&Path>,
) -> Vec<PathBuf> {
    let mut candidates = Vec::new();

    push_whisper_model_directory(&mut candidates, asset_directory.join("models"));
    push_whisper_model_directory(
        &mut candidates,
        app_managed_runtime_directory(asset_directory),
    );
    push_whisper_model_directory(
        &mut candidates,
        app_managed_runtime_directory(asset_directory).join("models"),
    );

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
    let asset_directory = PathBuf::from(&settings.asset_directory);

    let (executable_path, source) = if let Some(path) = manual_cli_path {
        (Some(path), Some("manual".to_string()))
    } else {
        (
            collect_managed_whisper_cli_candidates(&asset_directory)
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

    let response = http_client()?
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

    let update_available = latest_tag != RECOMMENDED_WHISPER_RUNTIME_VERSION;
    Ok(WhisperAssetUpdateResult {
        kind: "runtime".into(),
        status: if update_available {
            "available".into()
        } else {
            "current".into()
        },
        message: if update_available {
            format!("A newer whisper.cpp runtime is available: {}.", latest_tag)
        } else {
            "Your app-managed Whisper runtime is up to date.".into()
        },
        current_version: Some(RECOMMENDED_WHISPER_RUNTIME_VERSION.into()),
        latest_version: Some(latest_tag),
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

    let response = http_client()?
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
        } else {
            autostart_manager
                .disable()
                .map_err(|error| error.to_string())?;
        }

        return autostart_manager
            .is_enabled()
            .map_err(|error| error.to_string());
    }

    #[cfg(not(desktop))]
    {
        Ok(enabled)
    }
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
    let mut normalized =
        normalize_settings(app, &paths, settings).map_err(|error| error.to_string())?;
    ensure_directory_exists(Path::new(&normalized.output_directory))?;
    ensure_directory_exists(Path::new(&normalized.asset_directory))?;
    normalized.launch_at_login = apply_launch_at_login_setting(app, normalized.launch_at_login)?;

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
    let _ = refresh_whisper_detection_state(app)?;
    log_event(
        app,
        "INFO",
        "settings.saved",
        serde_json::json!({
            "outputDirectory": normalized.output_directory,
            "assetDirectory": normalized.asset_directory,
            "whisperCliPath": normalized.whisper.cli_path,
            "whisperModelPath": normalized.whisper.model_path,
            "whisperModelChoice": normalized.whisper.model_choice,
            "whisperLanguage": normalized.whisper.language
        }),
    );
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

fn recommended_runtime_archive_path<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
    let asset_directory = PathBuf::from(&persisted.settings.asset_directory);
    let runtime_directory = asset_directory.join("downloads");
    drop(persisted);

    ensure_directory_exists(&runtime_directory)?;
    Ok(runtime_directory.join(RECOMMENDED_WHISPER_RUNTIME_FILE))
}

fn recommended_runtime_install_directory<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<PathBuf, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
    let asset_directory = PathBuf::from(&persisted.settings.asset_directory);
    drop(persisted);

    let runtime_directory = app_managed_runtime_directory(&asset_directory);
    ensure_directory_exists(&runtime_directory)?;
    Ok(runtime_directory)
}

fn find_existing_managed_cli_path(asset_directory: &Path) -> Option<PathBuf> {
    collect_managed_whisper_cli_candidates(asset_directory)
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
        .map_err(|error| error.to_string())?;

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
    show_native_notification(
        app,
        "Recording started",
        &format!("Capturing system audio as {}.", display_name),
    );

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

    let archive_path = recommended_runtime_archive_path(app)?;
    let install_directory = recommended_runtime_install_directory(app)?;
    let app_handle = app.clone();

    update_shell_snapshot(app, |shell| {
        shell.phase = "downloading-model".into();
        shell.status_text = format!(
            "Downloading the recommended Whisper runtime to {}...",
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
                    find_existing_managed_cli_path(&asset_directory)
                {
                    verify_whisper_cli(&existing_cli_path)?;
                    existing_cli_path
                } else {
                    download_file_to_path_with_progress(
                        &app_handle,
                        RECOMMENDED_WHISPER_RUNTIME_URL,
                        &archive_path,
                        "runtime",
                        "the recommended Whisper runtime",
                    )?;

                    extract_zip_archive_to_directory(&archive_path, &install_directory)?;
                    find_existing_managed_cli_path(&asset_directory).ok_or_else(|| {
                        "The runtime downloaded, but whisper-cli.exe was not found.".to_string()
                    })?
                };
                verify_whisper_cli(&cli_path)?;
                clear_managed_whisper_override(&app_handle, "runtime")?;

                let detection = refresh_whisper_detection_state(&app_handle)?;
                update_model_download_snapshot(&app_handle, |snapshot| {
                    snapshot.kind = Some("runtime".into());
                    snapshot.status = "completed".into();
                    snapshot.message =
                        "Recommended Whisper runtime downloaded successfully.".into();
                    snapshot.downloaded_bytes =
                        snapshot.total_bytes.unwrap_or(snapshot.downloaded_bytes);
                    snapshot.progress_percent = Some(100.0);
                    snapshot.target_path = Some(cli_path.display().to_string());
                })?;
                reset_model_download_control(&app_handle)?;

                update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "idle".into();
                    shell.status_text = if detection.status == "ready" {
                        format!("Whisper runtime is ready at {}", cli_path.display())
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
                        "cliPath": cli_path.display().to_string()
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
                            show_native_notification(
                                &app,
                                "Recording finished",
                                &format!(
                                    "Saved {} and its transcript.",
                                    recent_recording.file_name
                                ),
                            );
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
                            show_native_notification(
                                &app,
                                "Recording finished",
                                &format!("Saved {}.", recent_recording.file_name),
                            );
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
                show_native_notification(
                    &app,
                    "Recording finished",
                    &format!("Saved {}.", recent_recording.file_name),
                );
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
            show_native_notification(
                &app,
                "Recording finished",
                &format!("Saved {}.", recent_recording.file_name),
            );
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

fn setup_error(message: impl Into<String>) -> tauri::Error {
    let boxed_error: Box<dyn std::error::Error> = Box::new(std::io::Error::new(
        std::io::ErrorKind::Other,
        message.into(),
    ));
    tauri::Error::Setup(boxed_error.into())
}

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
    tauri::Builder::default()
        .plugin(
            tauri_plugin_autostart::Builder::new()
                .args(["--autostart"])
                .build(),
        )
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_notification::init())
        .setup(|app| {
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

            TrayIconBuilder::new()
                .tooltip("Wonder of U")
                .menu(&menu)
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

            if start_minimized {
                let _ = hide_main_window_inner(&app_handle);
            }

            emit_app_snapshot(&app.handle());
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_app_bootstrap,
            download_recommended_whisper_model,
            download_recommended_whisper_runtime,
            check_whisper_runtime_update,
            check_whisper_model_update,
            toggle_whisper_model_download_pause,
            cancel_whisper_model_download,
            save_settings,
            start_recording,
            stop_recording,
            show_main_window,
            hide_main_window
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(test)]
mod tests {
    use super::{sanitize_recording_name, unique_wav_path, PersistedData};
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
        assert!(Path::new("C:\\Temp").is_absolute());
    }
}
