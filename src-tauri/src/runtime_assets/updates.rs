use std::{
    fs,
    path::PathBuf,
    process::Command,
    time::Duration,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_config::YTDLP_RELEASES_API_URL,
    app_state::sanitize_runtime_version,
    app_types::{whisper_model_spec, SharedPersistedState, WhisperAssetUpdateResult},
    runtime_assets::detect_local_ytdlp,
};

use super::refresh_whisper_detection_state;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const WHISPER_RELEASES_API_URL: &str =
    "https://api.github.com/repos/ggml-org/whisper.cpp/releases/latest";

fn update_check_http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent("Wonder of U Desktop/0.1.0")
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|error| error.to_string())
}

pub(crate) fn check_whisper_runtime_update_inner<R: Runtime>(
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

/// Reads the installed yt-dlp version by running `<path> --version` and trimming
/// its single-line output. Returns `None` when the binary cannot be run.
fn installed_ytdlp_version(executable_path: &str) -> Option<String> {
    let mut command = Command::new(executable_path);
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let output = command
        .arg("--ignore-config")
        .arg("--version")
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!version.is_empty()).then_some(version)
}

pub(crate) fn check_ytdlp_update_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<WhisperAssetUpdateResult, String> {
    let settings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the current app settings.".to_string())?;
        persisted.settings.clone()
    };

    let detection = detect_local_ytdlp(&settings);
    let installed_version = detection
        .executable_path
        .as_deref()
        .and_then(installed_ytdlp_version);

    let response = update_check_http_client()?
        .get(YTDLP_RELEASES_API_URL)
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
        .ok_or_else(|| "Could not read the latest yt-dlp release tag.".to_string())?;

    let (status, message) = match installed_version.as_deref() {
        None => (
            "installed".to_string(),
            format!("yt-dlp is not installed yet. The latest release is {latest_tag}."),
        ),
        Some(current) if current == latest_tag => (
            "current".to_string(),
            "Your yt-dlp is up to date.".to_string(),
        ),
        Some(_) => (
            "available".to_string(),
            format!("A newer yt-dlp release is available: {latest_tag}."),
        ),
    };

    Ok(WhisperAssetUpdateResult {
        kind: "ytdlp".into(),
        status,
        message,
        current_version: installed_version,
        latest_version: Some(latest_tag),
    })
}

pub(crate) fn check_whisper_model_update_inner<R: Runtime>(
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
