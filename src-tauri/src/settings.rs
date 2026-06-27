use std::path::Path;

use tauri::{AppHandle, Manager, Runtime};
#[cfg(desktop)]
use tauri_plugin_autostart::ManagerExt;
#[cfg(target_os = "windows")]
use winreg::{
    enums::{HKEY_CURRENT_USER, KEY_SET_VALUE},
    RegKey,
};

use crate::{
    app_config::AUTOSTART_ARGUMENT,
    app_runtime::{emit_app_snapshot, ensure_directory_exists, log_event},
    app_state::{normalize_settings, write_persisted_data},
    app_types::{AppPathsState, AppSettings, SharedPersistedState},
    runtime_assets::{refresh_whisper_detection_state, whisper_detection_inputs_changed},
};

#[cfg(target_os = "windows")]
const WINDOWS_RUN_KEY: &str = "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Run";

#[cfg(target_os = "windows")]
fn windows_autostart_command(executable: &Path) -> String {
    format!("\"{}\" {AUTOSTART_ARGUMENT}", executable.display())
}

#[cfg(target_os = "windows")]
fn repair_windows_autostart_command<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let executable = std::env::current_exe().map_err(|error| error.to_string())?;
    let run_key = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey_with_flags(WINDOWS_RUN_KEY, KEY_SET_VALUE)
        .map_err(|error| error.to_string())?;
    run_key
        .set_value(
            &app.package_info().name,
            &windows_autostart_command(&executable),
        )
        .map_err(|error| error.to_string())
}

pub(crate) fn apply_launch_at_login_setting<R: Runtime>(
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
            #[cfg(target_os = "windows")]
            if let Err(error) = repair_windows_autostart_command(app) {
                let _ = autostart_manager.disable();
                return Err(format!(
                    "Windows startup registration could not be finalized. {error}"
                ));
            }
        } else if let Err(error) = autostart_manager.disable() {
            let message = error.to_string();
            if !is_autostart_not_found_error(&message) {
                return Err(message);
            }
        }

        match autostart_manager.is_enabled() {
            Ok(actual_state) => Ok(actual_state),
            Err(error) => {
                let message = error.to_string();
                if !enabled && is_autostart_not_found_error(&message) {
                    Ok(false)
                } else {
                    Err(message)
                }
            }
        }
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

pub(crate) fn save_settings_inner<R: Runtime>(
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

#[cfg(all(test, target_os = "windows"))]
mod tests {
    use super::windows_autostart_command;
    use std::path::Path;

    #[test]
    fn windows_autostart_command_quotes_executable_paths() {
        assert_eq!(
            windows_autostart_command(Path::new(
                r"C:\Program Files\Wonder of U\wonder_of_u_desktop.exe"
            )),
            r#""C:\Program Files\Wonder of U\wonder_of_u_desktop.exe" --autostart"#
        );
    }
}
