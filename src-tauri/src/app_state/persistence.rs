use std::{fs, path::PathBuf};

use tauri::{AppHandle, Manager, Runtime};

use crate::app_types::{
    default_theme_preference, AnkiSettings, AppPathsState, AppSettings, FeatureSettings,
    PersistedData, TranslationSettings, WhisperSettings,
};

use super::{
    history::normalize_recent_recording_languages, normalize_settings, reconcile_recording_history,
};

pub(crate) fn build_app_paths<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<AppPathsState, tauri::Error> {
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

pub(super) fn default_output_directory<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<PathBuf, tauri::Error> {
    let base = app
        .path()
        .document_dir()
        .or_else(|_| app.path().download_dir())
        .or_else(|_| app.path().home_dir())?;

    Ok(base.join("Wonder of U Recordings"))
}

pub(super) fn default_asset_directory(paths: &AppPathsState) -> PathBuf {
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
        anki: AnkiSettings::default(),
        features: FeatureSettings::default(),
        translation: TranslationSettings::default(),
        theme: default_theme_preference(),
        launch_at_login: false,
        start_minimized: false,
    })
}

pub(crate) fn load_persisted_data<R: Runtime>(
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
    reconcile_recording_history(&mut state);
    normalize_recent_recording_languages(&mut state.recent_recordings);
    if state.untitled_counter == 0 {
        state.untitled_counter = 1;
    }

    Ok(state)
}

pub(crate) fn write_persisted_data<R: Runtime>(
    app: &AppHandle<R>,
    state: &PersistedData,
) -> Result<(), String> {
    let paths = app.state::<AppPathsState>().inner().clone();
    let serialized = serde_json::to_string_pretty(state).map_err(|error| error.to_string())?;
    fs::write(&paths.state_file, serialized).map_err(|error| error.to_string())
}
