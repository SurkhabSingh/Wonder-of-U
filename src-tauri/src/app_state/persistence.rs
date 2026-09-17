use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_runtime::now_ms,
    app_types::{
        default_indicator_position, default_theme_preference, AnkiSettings, AppPathsState,
        AppSettings, FeatureSettings, PersistedData, ScannerSettings, TranslationSettings,
        WhisperSettings,
    },
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
        known_words_file: data_dir.join("known_words.txt"),
        progress_file: data_dir.join("progress.jsonl"),
        mined_cards_file: data_dir.join("mined_cards.json"),
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
        scanner: ScannerSettings::default(),
        jimaku_api_key: String::new(),
        theme: default_theme_preference(),
        indicator_position: default_indicator_position(),
        launch_at_login: false,
        start_minimized: false,
    })
}

fn first_run_state(settings: AppSettings) -> PersistedData {
    PersistedData {
        settings,
        ..PersistedData::default()
    }
}

/// Moves an unparseable state file aside instead of letting it be overwritten.
fn preserve_unparseable_state_file(paths: &AppPathsState, reason: &str) {
    let backup_path = paths
        .state_file
        .with_extension(format!("json.corrupt-{}", now_ms()));
    let rename_error = fs::rename(&paths.state_file, &backup_path)
        .err()
        .map(|error| error.to_string());

    crate::logging::write(
        &paths.log_file,
        "ERROR",
        "state.unreadable",
        serde_json::json!({
            "stateFile": paths.state_file.display().to_string(),
            "backupPath": backup_path.display().to_string(),
            "backupError": rename_error,
            "message": format!(
                "The saved library could not be parsed and was moved aside; starting from defaults. {reason}"
            )
        }),
    );
}

pub(crate) fn load_persisted_data<R: Runtime>(
    app: &AppHandle<R>,
    paths: &AppPathsState,
) -> Result<PersistedData, tauri::Error> {
    let defaults = default_settings(app, paths)?;

    let mut state = match fs::read_to_string(&paths.state_file) {
        Ok(raw) => match serde_json::from_str::<PersistedData>(&raw) {
            Ok(state) => state,
            Err(error) => {
                preserve_unparseable_state_file(paths, &error.to_string());
                first_run_state(defaults.clone())
            }
        },
        Err(error) if error.kind() == ErrorKind::NotFound => first_run_state(defaults.clone()),
        Err(error) => {
            crate::logging::write(
                &paths.log_file,
                "ERROR",
                "state.unreadable",
                serde_json::json!({
                    "stateFile": paths.state_file.display().to_string(),
                    "message": format!("The saved library could not be read; starting from defaults. {error}")
                }),
            );
            first_run_state(defaults.clone())
        }
    };

    state.settings = normalize_settings(app, paths, state.settings)?;
    reconcile_recording_history(&mut state);
    normalize_recent_recording_languages(&mut state.recent_recordings);
    if state.untitled_counter == 0 {
        state.untitled_counter = 1;
    }

    Ok(state)
}

/// Writes a JSON file atomically: temp file, flush, rename over the original.
pub(crate) fn write_file_atomically(path: &Path, contents: &str) -> Result<(), String> {
    let temp_path = match path.file_name() {
        Some(name) => {
            let mut temp_name = name.to_os_string();
            temp_name.push(".tmp");
            path.with_file_name(temp_name)
        }
        None => return Err(format!("{} is not a file path.", path.display())),
    };

    let result = (|| {
        let mut file = fs::File::create(&temp_path).map_err(|error| error.to_string())?;
        file.write_all(contents.as_bytes())
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        drop(file);
        fs::rename(&temp_path, path).map_err(|error| error.to_string())
    })();

    if result.is_err() {
        let _ = fs::remove_file(&temp_path);
    }

    result
}

pub(crate) fn write_persisted_data<R: Runtime>(
    app: &AppHandle<R>,
    state: &PersistedData,
) -> Result<(), String> {
    let paths = app.state::<AppPathsState>().inner().clone();
    let serialized = serde_json::to_string_pretty(state).map_err(|error| error.to_string())?;
    write_file_atomically(&paths.state_file, &serialized)
}

#[cfg(test)]
mod tests {
    use super::{preserve_unparseable_state_file, write_file_atomically};
    use crate::app_types::AppPathsState;
    use std::fs;

    fn paths_in(data_dir: &std::path::Path) -> AppPathsState {
        AppPathsState {
            state_file: data_dir.join("state.json"),
            known_words_file: data_dir.join("known_words.txt"),
            progress_file: data_dir.join("progress.jsonl"),
            mined_cards_file: data_dir.join("mined_cards.json"),
            log_file: data_dir.join("wonder-of-u.log"),
            data_dir: data_dir.to_path_buf(),
            assets_dir: data_dir.join("assets"),
        }
    }

    #[test]
    fn an_atomic_write_replaces_the_previous_state_and_leaves_no_temp_file() {
        let dir = tempfile::tempdir().unwrap();
        let state_file = dir.path().join("state.json");
        fs::write(&state_file, "{\"old\":true}").unwrap();

        write_file_atomically(&state_file, "{\"new\":true}").unwrap();

        assert_eq!(fs::read_to_string(&state_file).unwrap(), "{\"new\":true}");
        assert!(!dir.path().join("state.json.tmp").exists());
    }

    #[test]
    fn an_atomic_write_creates_the_state_file_on_a_first_run() {
        let dir = tempfile::tempdir().unwrap();
        let state_file = dir.path().join("state.json");

        write_file_atomically(&state_file, "{}").unwrap();

        assert_eq!(fs::read_to_string(&state_file).unwrap(), "{}");
    }

    #[test]
    fn a_failed_write_leaves_the_previous_state_untouched() {
        let dir = tempfile::tempdir().unwrap();
        let state_file = dir.path().join("state.json");
        fs::write(&state_file, "{\"old\":true}").unwrap();
        // The temp path is a directory, so creating the temp FILE fails: the real
        // state file must not have been touched on the way to that failure.
        fs::create_dir(dir.path().join("state.json.tmp")).unwrap();

        assert!(write_file_atomically(&state_file, "{\"new\":true}").is_err());
        assert_eq!(fs::read_to_string(&state_file).unwrap(), "{\"old\":true}");
    }

    #[test]
    fn an_unparseable_state_file_is_preserved_rather_than_discarded() {
        let dir = tempfile::tempdir().unwrap();
        let paths = paths_in(dir.path());
        fs::write(&paths.state_file, "{\"recentRecordings\": [tru").unwrap();

        preserve_unparseable_state_file(&paths, "expected value at line 1");

        // The original is out of the way of the defaults startup is about to write...
        assert!(!paths.state_file.exists());
        // ...but its bytes still exist under a timestamped name, and the loss is logged.
        let backup = fs::read_dir(dir.path())
            .unwrap()
            .flatten()
            .map(|entry| entry.file_name().to_string_lossy().to_string())
            .find(|name| name.starts_with("state.json.corrupt-"))
            .expect("the unparseable state file was not preserved");
        assert_eq!(
            fs::read_to_string(dir.path().join(backup)).unwrap(),
            "{\"recentRecordings\": [tru"
        );

        let log = fs::read_to_string(&paths.log_file).unwrap();
        assert!(log.contains("\"level\":\"ERROR\""));
        assert!(log.contains("state.unreadable"));
    }
}
