use std::sync::{Condvar, Mutex};

use tauri::{App, Manager};

use crate::{
    asset_downloads::DownloadQueue,
    app_runtime::setup_error,
    app_state::{build_app_paths, load_persisted_data, write_persisted_data},
    anki::restore_known_words_index,
    app_types::{
        KnownWordsState, ModelDownloadControl, ModelDownloadControlState, ModelDownloadQueueState, ModelDownloadSnapshot,
        ModelDownloadState, RecorderState, SharedPersistedState, SharedShellState, ShellSnapshot,
        WhisperDetection, WhisperDetectionState,
    },
    runtime_assets::refresh_whisper_detection_state,
    settings::apply_launch_at_login_setting,
};

pub(crate) fn initialize_app_state(app: &mut App) -> Result<Vec<String>, tauri::Error> {
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
    // The queue of waiting downloads. Empty at startup on purpose: a queue is something
    // you are watching, not a background job that survives a restart.
    app.manage(ModelDownloadQueueState(Mutex::new(DownloadQueue::default())));
    app.manage(ModelDownloadControlState {
        control: Mutex::new(ModelDownloadControl::default()),
        condvar: Condvar::new(),
    });
    app.manage(RecorderState(Mutex::new(None)));
    app.manage(KnownWordsState(Mutex::new(None)));
    app.manage(crate::translation_bridge::TranslationBridge::new());

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

    // The hook goes in before anything else can panic, so a crash during the rest of startup
    // is still recorded.
    crate::logging::install_panic_hook(paths.log_file.clone());

    let mut startup = crate::logging::environment();
    if let Some(fields) = startup.as_object_mut() {
        fields.insert(
            "message".into(),
            serde_json::json!("Wonder of U started."),
        );
        fields.insert(
            "dataDir".into(),
            serde_json::json!(paths.data_dir.display().to_string()),
        );
        fields.insert(
            "stateFile".into(),
            serde_json::json!(paths.state_file.display().to_string()),
        );
    }
    // Before the first line, so no recording name can be written unredacted.
    if let Ok(persisted) = app_handle.state::<SharedPersistedState>().0.lock() {
        crate::logging::set_recordings_directory(&persisted.settings.output_directory);
    }
    crate::logging::write(&paths.log_file, "INFO", "app.startup", startup);

    if let Err(error) = refresh_whisper_detection_state(&app_handle) {
        startup_warnings.push(format!(
            "Whisper readiness could not be initialized cleanly. {error}"
        ));
    }

    // After the settings are managed, since it judges the saved list against them,
    // and deliberately not a warning on failure: a missing or unreadable word list
    // is a Refresh away from fixed and says so in its own snapshot. It must never
    // be a reason the app opens complaining.
    restore_known_words_index(&app_handle);

    Ok(startup_warnings)
}
