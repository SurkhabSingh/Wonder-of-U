use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_state::write_persisted_data,
    app_types::{
        whisper_model_spec, SharedPersistedState, WhisperModelSpec, WHISPER_VAD_MODEL_FILE,
        WHISPER_VAD_MODEL_URL,
    },
    runtime_assets::refresh_whisper_detection_state,
    transcription::verify_whisper_model,
};

use super::asset::AssetKind;
use super::envelope::{run_asset_download, AssetDownloadPlan, Installed};
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
///
/// One read rather than two: both answers describe where the download is going, and taking
/// them a moment apart is how the path and the model it is named for end up disagreeing.
fn chosen_model<R: Runtime>(app: &AppHandle<R>) -> Result<WhisperModelSpec, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
    Ok(*whisper_model_spec(&persisted.settings.whisper.model_choice))
}

/// The two files one model download provisions.
///
/// The VAD model is not an extra: whisper.cpp's segmentation needs it, and it lives beside the
/// transcription model so a single download leaves the engine usable. It is under a megabyte
/// against the model's hundreds, which is why it is fetched silently rather than announced.
struct ModelPaths {
    model: PathBuf,
    vad: PathBuf,
}

fn model_paths(asset_directory: &Path, model_file_name: &str) -> ModelPaths {
    let models_directory = asset_directory.join("models");
    ModelPaths {
        model: models_directory.join(model_file_name),
        vad: models_directory.join(WHISPER_VAD_MODEL_FILE),
    }
}

pub(crate) fn download_recommended_whisper_model_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
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

    run_asset_download(
        app,
        AssetDownloadPlan {
            kind: AssetKind::Model,
            thread_name: "whisper-model-download",
            shell_busy_message: "Finish the current task before downloading the Whisper model."
                .into(),
            // Deliberately not the "Another download..." the other five use. Preserved rather
            // than unified, because unifying it would reword a message nobody asked to change.
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
                // TWO transfers under one slot, which is the reason phase G is a closure and
                // not a `download_url` field: the shape of the work differs here, not just its
                // parameters. Both report as `AssetKind::Model`, so the card shows one download
                // that happens to fetch two files.
                //
                // Skip-if-EXISTS, not skip-if-runnable as ffmpeg and the runtime use. A model
                // is data, not an executable — there is nothing to run to prove it — so its
                // only cheap test is presence.
                if !paths.model.exists() {
                    context.fetch(
                        model_spec.download_url,
                        &paths.model,
                        &format!("the {} Whisper model", model_spec.label),
                    )?;
                }
                // The engine also needs whisper.cpp's built-in Silero VAD model (tiny). Fetch
                // it into the same models directory so one download provisions both.
                if !paths.vad.exists() {
                    context.fetch(
                        WHISPER_VAD_MODEL_URL,
                        &paths.vad,
                        "the speech-detector (VAD) model",
                    )?;
                }

                // Bare, and NOT wrapped in `verify_managed_binary_or_remove` the way every
                // other asset's verification is — so a model that fails this check is left on
                // disk, and detection, which tests existence, keeps reporting it ready.
                // Preserved exactly as it was: changing it would delete a user's model file,
                // which is a decision to take on its own rather than inside a refactor.
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
        },
    )
}

#[cfg(test)]
mod tests {
    use super::model_paths;
    use crate::app_types::{WHISPER_MODEL_SPECS, WHISPER_VAD_MODEL_FILE};
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
