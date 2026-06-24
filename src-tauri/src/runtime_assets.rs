use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_state::sanitize_runtime_version,
    app_types::{
        whisper_model_spec, AppSettings, FfmpegDetection, SharedPersistedState,
        WhisperAssetUpdateResult, WhisperDetection, WhisperDetectionState, WHISPER_MODEL_SPECS,
    },
    emit_app_snapshot, log_event,
    transcription::{verify_whisper_cli, verify_whisper_model},
};

const WHISPER_RELEASES_API_URL: &str =
    "https://api.github.com/repos/ggml-org/whisper.cpp/releases/latest";
#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn update_check_http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent("Wonder of U Desktop/0.1.0")
        .connect_timeout(Duration::from_secs(5))
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|error| error.to_string())
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

pub(crate) fn app_managed_runtime_directory(
    asset_directory: &Path,
    runtime_version: &str,
) -> PathBuf {
    managed_runtime_root(asset_directory).join(sanitize_runtime_version(runtime_version))
}

pub(crate) fn collect_managed_whisper_cli_candidates(
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

pub(crate) fn managed_ffmpeg_install_directory(asset_directory: &Path) -> PathBuf {
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

pub(crate) fn collect_managed_ffmpeg_candidates(asset_directory: &Path) -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    push_ffmpeg_candidates_from_directory(
        &mut candidates,
        &managed_ffmpeg_install_directory(asset_directory),
    );
    candidates
}

fn hide_command_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

pub(crate) fn verify_ffmpeg_binary(executable_path: &Path) -> Result<(), String> {
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

pub(crate) fn detect_local_ffmpeg(settings: &AppSettings) -> FfmpegDetection {
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

pub(crate) fn all_managed_model_paths(asset_directory: &Path) -> Vec<PathBuf> {
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

pub(crate) fn refresh_whisper_detection_state<R: Runtime>(
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

pub(crate) fn whisper_detection_inputs_changed(previous: &AppSettings, next: &AppSettings) -> bool {
    previous.asset_directory != next.asset_directory
        || previous.whisper.cli_path != next.whisper.cli_path
        || previous.whisper.runtime_version != next.whisper.runtime_version
        || previous.whisper.model_path != next.whisper.model_path
        || previous.whisper.model_choice != next.whisper.model_choice
}
