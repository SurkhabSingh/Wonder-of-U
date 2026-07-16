use std::{
    fs,
    io::{ErrorKind, Write},
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_runtime::{append_structured_log, now_ms},
    app_types::{
        default_theme_preference, AnkiSettings, AppPathsState, AppSettings, FeatureSettings,
        PersistedData, TranslationSettings, WhisperSettings,
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

fn first_run_state(settings: AppSettings) -> PersistedData {
    PersistedData {
        settings,
        recent_recordings: Vec::new(),
        untitled_counter: 1,
    }
}

/// Moves an unparseable state file aside instead of letting it be overwritten.
///
/// Falling back to defaults is not a recovery: startup writes the defaults straight
/// back over `state.json`, so the library and every setting are gone for good the
/// moment the app opens. It also compounds — the reset restores the DEFAULT
/// `output_directory`, so `reconcile_recording_history` then sweeps the wrong
/// folder and recovers nothing from the real one. Keeping the bytes under a
/// timestamped name is what makes the library recoverable by hand.
///
/// Best-effort by design: a rename that fails must not stop the app from starting,
/// but it is logged at ERROR either way so the loss is never silent.
fn preserve_unparseable_state_file(paths: &AppPathsState, reason: &str) {
    let backup_path = paths
        .state_file
        .with_extension(format!("json.corrupt-{}", now_ms()));
    let rename_error = fs::rename(&paths.state_file, &backup_path)
        .err()
        .map(|error| error.to_string());

    append_structured_log(
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

    // Note the deliberate split: no file at all is a genuine first run and defaults
    // are the right answer silently, but a file that is present and unreadable is a
    // problem the user has to be told about and be able to recover from.
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
            // Present but unreadable (locked, permissions). The contents are fine, so
            // there is nothing to move aside — but this session still starts with an
            // empty library and will persist it, so say so.
            append_structured_log(
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

/// Writes the state file atomically: temp file, flush, rename over the original.
///
/// This is the only writer of the only copy of the library, and it runs on every
/// history mutation. A plain `fs::write` truncates the real file first, so a crash
/// or power loss between the truncate and the flush leaves well-formed-looking but
/// truncated JSON — which the loader cannot parse, and the whole library is gone.
/// Same temp+rename shape `asset_downloads::transfer` uses for downloads.
///
/// The `sync_all` is load-bearing, not belt and braces. The rename is atomic with
/// respect to the directory entry only; without the flush it can commit while the
/// temp file's bytes are still in the page cache, and a power loss then leaves the
/// state file's NEW name over the OLD file's unwritten contents — exactly the
/// truncated-JSON case the temp file exists to prevent. Windows' MoveFileEx (what
/// `fs::rename` uses) does not flush the source for us. The parent directory is
/// not synced: there is no portable handle for that on Windows, and NTFS journals
/// the rename itself.
fn write_state_file_atomically(state_file: &Path, serialized: &str) -> Result<(), String> {
    let temp_path = state_file.with_extension("json.tmp");

    let result = (|| {
        let mut file = fs::File::create(&temp_path).map_err(|error| error.to_string())?;
        file.write_all(serialized.as_bytes())
            .map_err(|error| error.to_string())?;
        file.sync_all().map_err(|error| error.to_string())?;
        drop(file);
        fs::rename(&temp_path, state_file).map_err(|error| error.to_string())
    })();

    if result.is_err() {
        // A stranded temp file would be retried into on the next write anyway, but
        // leaving a half-written state.json.tmp beside the real one is confusing to
        // anyone recovering by hand.
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
    write_state_file_atomically(&paths.state_file, &serialized)
}

#[cfg(test)]
mod tests {
    use super::{preserve_unparseable_state_file, write_state_file_atomically};
    use crate::app_types::AppPathsState;
    use std::fs;

    fn paths_in(data_dir: &std::path::Path) -> AppPathsState {
        AppPathsState {
            state_file: data_dir.join("state.json"),
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

        write_state_file_atomically(&state_file, "{\"new\":true}").unwrap();

        assert_eq!(fs::read_to_string(&state_file).unwrap(), "{\"new\":true}");
        assert!(!dir.path().join("state.json.tmp").exists());
    }

    #[test]
    fn an_atomic_write_creates_the_state_file_on_a_first_run() {
        let dir = tempfile::tempdir().unwrap();
        let state_file = dir.path().join("state.json");

        write_state_file_atomically(&state_file, "{}").unwrap();

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

        assert!(write_state_file_atomically(&state_file, "{\"new\":true}").is_err());
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
