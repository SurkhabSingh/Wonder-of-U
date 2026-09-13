use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_state::write_persisted_data,
    app_types::{
        whisper_model_spec, whisper_vad_model_path, SharedPersistedState, WhisperModelSpec,
        WHISPER_VAD_MODEL_URL,
    },
    runtime_assets::refresh_whisper_detection_state,
    transcription::verify_whisper_model,
};

use super::asset::AssetKind;
use super::envelope::{AssetDownloadPlan, Installed};
use super::transfer::{asset_directory, ensure_directory_exists};

fn clear_managed_model_override<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let persisted_snapshot = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update the managed Whisper settings.".to_string())?;
        persisted.settings.whisper.model_path.clear();
        persisted.clone()
    };

    write_persisted_data(app, &persisted_snapshot)
}

/// Which model the user has chosen, read in the same lock as the asset directory.
fn chosen_model<R: Runtime>(app: &AppHandle<R>) -> Result<WhisperModelSpec, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
    Ok(*whisper_model_spec(&persisted.settings.whisper.model_choice))
}

/// The two files one model download provisions.
struct ModelPaths {
    model: PathBuf,
    vad: PathBuf,
}

fn model_paths(asset_directory: &Path, model_file_name: &str) -> ModelPaths {
    ModelPaths {
        model: asset_directory.join("models").join(model_file_name),
        vad: whisper_vad_model_path(asset_directory),
    }
}

pub(super) fn whisper_model_plan<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<AssetDownloadPlan<R>, String> {
    let asset_directory = asset_directory(app)?;
    let model_spec = chosen_model(app)?;
    let paths = model_paths(&asset_directory, model_spec.file_name);
    ensure_directory_exists(
        paths
            .model
            .parent()
            .ok_or_else(|| "The models directory has no parent.".to_string())?,
    )?;

    let shell_start_text = format!(
        "Downloading the {} Whisper model to {}...",
        model_spec.label,
        paths.model.display()
    );
    let starting_target_path = paths.model.clone();

    Ok(AssetDownloadPlan {
        kind: AssetKind::Model,
        slot_busy_message: "A model download is already in progress.".into(),
        shell_start_text,
        starting_message: format!("Preparing the {} model download...", model_spec.label),
        starting_target_path,
        cancelled_message: "Model download cancelled.".into(),
        cancelled_shell_text: "Whisper model download cancelled.".into(),
        failed_message_prefix: "Model download failed".into(),
        failed_shell_prefix: "Whisper model download failed".into(),
        success_log_event: "whisper.model_downloaded",
        failure_log_event: "whisper.model_download_failed",
        install: Box::new(move |context| {
            if !paths.model.exists() {
                context.fetch(
                    model_spec.download_url,
                    &paths.model,
                    &format!("the {} Whisper model", model_spec.label),
                )?;
            }
            if !paths.vad.exists() {
                context.fetch(
                    WHISPER_VAD_MODEL_URL,
                    &paths.vad,
                    "the speech-detector (VAD) model",
                )?;
            }

            verify_whisper_model(&paths.model)?;
            clear_managed_model_override(context.app())?;
            let detection = refresh_whisper_detection_state(context.app())?;

            Ok(Installed {
                completed_message: format!(
                    "{} model downloaded successfully.",
                    model_spec.label
                ),
                shell_success_text: if detection.status == "ready" {
                    format!("{} model is ready at {}", model_spec.label, paths.model.display())
                } else {
                    format!(
                        "Model downloaded, but Whisper still needs setup: {}",
                        detection.message
                    )
                },
                log_details: serde_json::json!({
                    "targetPath": paths.model.display().to_string(),
                    "modelChoice": model_spec.id
                }),
                target_path: paths.model,
            })
        }),
    })
}

/// Fetches **only** the speech-detector model, never the transcription model.
pub(super) fn whisper_vad_model_plan<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<AssetDownloadPlan<R>, String> {
    let asset_directory = asset_directory(app)?;
    let vad_path = whisper_vad_model_path(&asset_directory);
    ensure_directory_exists(
        vad_path
            .parent()
            .ok_or_else(|| "The models directory has no parent.".to_string())?,
    )?;

    let shell_start_text = format!(
        "Downloading the speech detector to {}...",
        vad_path.display()
    );
    let starting_target_path = vad_path.clone();

    Ok(AssetDownloadPlan {
        kind: AssetKind::Model,
        slot_busy_message: "A model download is already in progress.".into(),
        shell_start_text,
        starting_message: "Preparing the speech-detector download...".into(),
        starting_target_path,
        cancelled_message: "Speech-detector download cancelled.".into(),
        cancelled_shell_text: "Speech-detector download cancelled.".into(),
        failed_message_prefix: "Speech-detector download failed".into(),
        failed_shell_prefix: "Speech-detector download failed".into(),
        success_log_event: "whisper.vad_model_downloaded",
        failure_log_event: "whisper.vad_model_download_failed",
        install: Box::new(move |context| {
            if !vad_path.exists() {
                context.fetch(
                    WHISPER_VAD_MODEL_URL,
                    &vad_path,
                    "the speech-detector (VAD) model",
                )?;
            }
            let detection = refresh_whisper_detection_state(context.app())?;

            Ok(Installed {
                completed_message: "The speech detector is ready. Transcription can run again."
                    .into(),
                shell_success_text: if detection.status == "ready" {
                    "The speech detector is ready. Transcription can run again.".to_string()
                } else {
                    format!(
                        "Speech detector downloaded, but Whisper still needs setup: {}",
                        detection.message
                    )
                },
                log_details: serde_json::json!({
                    "vadModelPath": vad_path.display().to_string()
                }),
                target_path: vad_path,
            })
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::model_paths;
    use crate::app_types::{whisper_vad_model_path, WHISPER_MODEL_SPECS, WHISPER_VAD_MODEL_FILE};
    use std::path::Path;

    /// One download provisions two files, and both belong in `models/` — the VAD model is
    /// resolved relative to the chosen model, so putting the model elsewhere would strand it.
    #[test]
    fn the_vad_model_lands_beside_the_transcription_model() {
        let paths = model_paths(Path::new("C:/assets"), "ggml-small.bin");

        assert_eq!(paths.model.parent(), paths.vad.parent());
        assert!(paths.vad.ends_with(WHISPER_VAD_MODEL_FILE), "{:?}", paths.vad);
        assert!(
            paths
                .model
                .components()
                .any(|part| part.as_os_str() == "models"),
            "{:?}",
            paths.model
        );
    }

    /// The full download and the repair must agree on where the detector goes, or the repair
    /// would write a file the gate is not looking for and offer itself again forever. They
    /// agree because they call the same function — this fails the moment one stops.
    #[test]
    fn the_download_and_the_gate_resolve_the_same_detector_path() {
        let asset_directory = Path::new("C:/assets");

        assert_eq!(
            model_paths(asset_directory, "ggml-small.bin").vad,
            whisper_vad_model_path(asset_directory)
        );
    }

    /// Every model in the catalogue gets its own file, so switching choice downloads rather
    /// than silently reusing whatever was there.
    #[test]
    fn each_model_choice_has_its_own_file() {
        let mut seen: Vec<_> = WHISPER_MODEL_SPECS
            .iter()
            .map(|spec| model_paths(Path::new("C:/assets"), spec.file_name).model)
            .collect();
        let total = seen.len();
        seen.sort();
        seen.dedup();

        assert_eq!(seen.len(), total, "two model choices share a path");
    }
}
