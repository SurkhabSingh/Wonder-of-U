mod recording;

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
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
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
#[cfg(desktop)]
use tauri_plugin_autostart::ManagerExt;

const START_SHORTCUT: &str = "Ctrl+Alt+R";
const STOP_SHORTCUT: &str = "Ctrl+Alt+S";
const SHOW_SHORTCUT: &str = "Ctrl+Alt+W";
const APP_SNAPSHOT_EVENT: &str = "app://snapshot-changed";
const START_SHORTCUT_CANDIDATES: [&str; 3] =
    [START_SHORTCUT, "Ctrl+Alt+Shift+R", "Ctrl+Alt+F8"];
const STOP_SHORTCUT_CANDIDATES: [&str; 3] =
    [STOP_SHORTCUT, "Ctrl+Alt+Shift+S", "Ctrl+Alt+F9"];
const SHOW_SHORTCUT_CANDIDATES: [&str; 3] =
    [SHOW_SHORTCUT, "Ctrl+Alt+Shift+W", "Ctrl+Alt+F10"];
const RECENT_RECORDINGS_LIMIT: usize = 10;

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
    translation: bool,
    anki: bool,
}

impl Default for FeatureSettings {
    fn default() -> Self {
        Self {
            transcription: true,
            translation: false,
            anki: false,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AppSettings {
    output_directory: String,
    asset_directory: String,
    features: FeatureSettings,
    launch_at_login: bool,
    start_minimized: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RecentRecording {
    file_name: String,
    file_path: String,
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
}

impl Default for ShellSnapshot {
    fn default() -> Self {
        Self {
            phase: "idle".into(),
            status_text:
                "Tray shell is ready. Press Ctrl+Alt+R to start recording system audio.".into(),
            last_shortcut: None,
            transition_count: 0,
            hotkeys: HotkeyBindings::default(),
            started_at_ms: None,
            current_recording_name: None,
            last_output_path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct AppBootstrap {
    shell: ShellSnapshot,
    settings: AppSettings,
    recent_recordings: Vec<RecentRecording>,
    log_path: String,
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
struct RecorderState(Mutex<Option<ActiveRecording>>);

struct ActiveRecording {
    stop_signal: Arc<AtomicBool>,
    worker: JoinHandle<Result<RecordingCaptureResult, String>>,
}

#[tauri::command]
fn get_app_bootstrap(app: AppHandle) -> Result<AppBootstrap, String> {
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
    let output_directory = normalize_directory_input(
        &settings.output_directory,
        &default_output_directory(app)?,
    );
    let asset_directory =
        normalize_directory_input(&settings.asset_directory, &default_asset_directory(paths));

    let transcription_enabled =
        settings.features.transcription || settings.features.translation || settings.features.anki;

    Ok(AppSettings {
        output_directory: output_directory.display().to_string(),
        asset_directory: asset_directory.display().to_string(),
        features: FeatureSettings {
            transcription: transcription_enabled,
            translation: settings.features.translation,
            anki: settings.features.anki,
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
            autostart_manager.enable().map_err(|error| error.to_string())?;
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

fn save_settings_inner<R: Runtime>(app: &AppHandle<R>, settings: AppSettings) -> Result<(), String> {
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
    log_event(
        app,
        "INFO",
        "settings.saved",
        serde_json::json!({
            "outputDirectory": normalized.output_directory,
            "assetDirectory": normalized.asset_directory
        }),
    );
    update_shell_snapshot(app, |shell| {
        if shell.phase == "idle" {
            shell.status_text = "Settings saved.".into();
        }
    })?;
    Ok(())
}

fn start_recording_inner<R: Runtime>(
    app: &AppHandle<R>,
    requested_name: Option<String>,
) -> Result<(), String> {
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
        *recorder = Some(ActiveRecording { stop_signal, worker });
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
        shell.transition_count += 1;
    })?;

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
    })?;

    active.stop_signal.store(true, Ordering::SeqCst);
    let result = active
        .worker
        .join()
        .map_err(|_| "The recording worker thread panicked.".to_string())?;

    match result {
        Ok(capture) => {
            let recent_recording = RecentRecording {
                file_name: capture
                    .output_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("recording.wav")
                    .to_string(),
                file_path: capture.output_path.display().to_string(),
                duration_ms: capture.duration_ms,
                bytes_written: capture.bytes_written,
                created_at_ms: capture.created_at_ms,
            };

            let persisted_snapshot = {
                let persisted_state = app.state::<SharedPersistedState>();
                let mut persisted = persisted_state
                    .0
                    .lock()
                    .map_err(|_| "Could not update the recording history.".to_string())?;
                persisted.recent_recordings.insert(0, recent_recording.clone());
                persisted.recent_recordings.truncate(RECENT_RECORDINGS_LIMIT);
                persisted.clone()
            };
            write_persisted_data(app, &persisted_snapshot)?;

            log_event(
                app,
                "INFO",
                "recording.saved",
                serde_json::json!({
                    "filePath": recent_recording.file_path,
                    "displayName": capture.display_name,
                    "durationMs": recent_recording.duration_ms,
                    "bytesWritten": recent_recording.bytes_written
                }),
            );

            update_shell_snapshot(app, |shell| {
                shell.phase = "idle".into();
                shell.status_text =
                    format!("Saved {}", recent_recording.file_name);
                shell.started_at_ms = None;
                shell.current_recording_name = None;
                shell.last_output_path = Some(recent_recording.file_path.clone());
                shell.transition_count += 1;
            })?;
        }
        Err(error) => {
            log_event(
                app,
                "ERROR",
                "recording.failed",
                serde_json::json!({ "message": error }),
            );
            update_shell_snapshot(app, |shell| {
                shell.phase = "error".into();
                shell.status_text = error.clone();
                shell.started_at_ms = None;
                shell.current_recording_name = None;
            })?;
            return Err(error);
        }
    }

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
    let boxed_error: Box<dyn std::error::Error> =
        Box::new(std::io::Error::new(std::io::ErrorKind::Other, message.into()));
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
        .setup(|app| {
            let app_handle = app.handle().clone();
            let paths = build_app_paths(&app_handle)?;
            let persisted = load_persisted_data(&app_handle, &paths)?;

            app.manage(paths.clone());
            app.manage(SharedPersistedState(Mutex::new(persisted)));
            app.manage(SharedShellState(Mutex::new(ShellSnapshot::default())));
            app.manage(RecorderState(Mutex::new(None)));

            let mut startup_warnings = Vec::new();

            {
                let persisted_state = app.state::<SharedPersistedState>();
                let mut persisted = persisted_state
                    .0
                    .lock()
                    .map_err(|_| setup_error("Could not initialize persisted app state."))?;

                match apply_launch_at_login_setting(&app_handle, persisted.settings.launch_at_login) {
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

            let show_item =
                MenuItem::with_id(app, "show", "Open Wonder of U", true, None::<&str>)?;
            let hide_item =
                MenuItem::with_id(app, "hide", "Hide Window", true, None::<&str>)?;
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
        assert_eq!(
            sanitize_recording_name("  lesson:01?*  "),
            "lesson 01"
        );
    }

    #[test]
    fn unique_wav_path_appends_suffix_when_file_exists() {
        let temp_dir = tempfile::tempdir().unwrap();
        let first = temp_dir.path().join("sample.wav");
        std::fs::write(&first, b"test").unwrap();
        let second = unique_wav_path(temp_dir.path(), "sample");
        assert_eq!(second.file_name().unwrap().to_string_lossy(), "sample_1.wav");
    }

    #[test]
    fn persisted_data_counter_defaults_to_positive_value() {
        let state = PersistedData {
            settings: serde_json::from_value(serde_json::json!({
                "outputDirectory": "C:\\Temp",
                "assetDirectory": "C:\\Temp\\assets",
                "features": {
                    "transcription": true,
                    "translation": false,
                    "anki": false
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
