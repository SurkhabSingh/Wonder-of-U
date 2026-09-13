use std::{
    fs,
    path::{Path, PathBuf},
    sync::{atomic::AtomicBool, Arc, Mutex},
};

use tauri::{AppHandle, Emitter, Manager, Runtime};

mod finalize;

use finalize::finalize_recording_pipeline;

use crate::{
    app_runtime::{ensure_directory_exists, log_event, now_ms, update_shell_snapshot},
    app_state::{next_recording_stem, normalize_settings, unique_wav_path, write_persisted_data},
    app_types::{
        ActiveRecording, AppPathsState, RecorderState, SharedPersistedState, SharedShellState,
    },
    recording::capture_system_audio_loopback,
    recording_indicator::{signal_recording_indicator, IndicatorSignal},
};

/// Event carrying the live input level (a single `f32` peak, 0.0..=1.0) to the
/// recording meters — the main-window bar (`useRecordingLevel`) and the toast
/// overlay bar (`src/overlay/main.ts`). Broadcast, so both windows receive it.
const RECORDING_LEVEL_EVENT: &str = "recording-level";

static RECORDER_START_CLAIM: Mutex<bool> = Mutex::new(false);

static WAV_PATH_RESERVATION_LOCK: Mutex<()> = Mutex::new(());

/// Whether nothing owns the recorder phase.
pub(crate) fn recorder_phase_is_free(phase: &str) -> bool {
    phase == "idle" || phase == "error"
}

pub(crate) fn recording_refusal_for(phase: &str) -> &'static str {
    match phase {
        "downloading-model" => {
            "A download is in progress. Wait for it to finish before recording."
        }
        "transcribing" => "A transcription is running. Wait for it to finish before recording.",
        "recording" => "A recording is already running.",
        "saving" => "The last recording is still being saved.",
        _ => "The app is busy with another task.",
    }
}

/// Owns the recorder slot for the whole of `start_recording_inner`.
struct RecorderStartClaim;

impl RecorderStartClaim {
    fn acquire<R: Runtime>(app: &AppHandle<R>) -> Result<Self, String> {
        Self::acquire_with(|| {
            let recorder_state = app.state::<RecorderState>();
            let recorder = recorder_state
                .0
                .lock()
                .map_err(|_| "Could not inspect the recorder state.".to_string())?;
            Ok(recorder.is_some())
        })
    }


    fn acquire_with<F>(recording_is_active: F) -> Result<Self, String>
    where
        F: FnOnce() -> Result<bool, String>,
    {
        let mut claimed = RECORDER_START_CLAIM
            .lock()
            .map_err(|_| "Could not inspect the recorder state.".to_string())?;
        if *claimed || recording_is_active()? {
            return Err("A recording is already in progress.".into());
        }

        *claimed = true;
        Ok(Self)
    }
}

impl Drop for RecorderStartClaim {
    fn drop(&mut self) {
        if let Ok(mut claimed) = RECORDER_START_CLAIM.lock() {
            *claimed = false;
        }
    }
}

struct ReservedWavPath {
    path: PathBuf,
    armed: bool,
}

impl ReservedWavPath {
    fn disarm(mut self) {
        self.armed = false;
    }
}

impl Drop for ReservedWavPath {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Picks a free WAV name and creates it in the same breath.
fn reserve_wav_path(directory: &Path, file_stem: &str) -> Result<PathBuf, String> {
    let _reservation_guard = WAV_PATH_RESERVATION_LOCK
        .lock()
        .map_err(|_| "Could not reserve a recording output name.".to_string())?;
    let output_path = unique_wav_path(directory, file_stem);
    fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output_path)
        .map_err(|error| {
            format!(
                "Could not reserve the recording file {}: {error}",
                output_path.display()
            )
        })?;

    Ok(output_path)
}

pub(crate) fn start_recording_inner<R: Runtime>(
    app: &AppHandle<R>,
    requested_name: Option<String>,
) -> Result<(), String> {
    let _slot_claim = RecorderStartClaim::acquire(app)?;

    {
        let shell_state = app.state::<SharedShellState>();
        let shell = shell_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the shell state.".to_string())?;
        if !recorder_phase_is_free(&shell.phase) {
            return Err(recording_refusal_for(&shell.phase).to_string());
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
        let output_path = reserve_wav_path(&output_directory, &file_stem)?;
        let snapshot = persisted.clone();
        (output_path, file_stem, snapshot)
    };
    let reserved_output = ReservedWavPath {
        path: output_path.clone(),
        armed: true,
    };

    // Spawn the capture worker FIRST — before the durable state write and the shell snapshot emit. 
    let stop_signal = Arc::new(AtomicBool::new(false));
    let log_path = app.state::<AppPathsState>().inner().log_file.clone();
    let output_path_for_worker = output_path.clone();
    let display_name_for_worker = display_name.clone();
    let stop_signal_for_worker = stop_signal.clone();
    let app_for_level = app.clone();
    let worker = std::thread::Builder::new()
        .name("system-audio-recorder".into())
        .spawn(move || {
            capture_system_audio_loopback(
                output_path_for_worker,
                display_name_for_worker,
                stop_signal_for_worker,
                log_path,
                started_at_ms,
                move |level| {
                    let _ = app_for_level.emit(RECORDING_LEVEL_EVENT, level);
                },
            )
        })
        .map_err(|error| {
            let message = error.to_string();
            let _ = update_shell_snapshot(app, |shell| {
                shell.phase = "error".into();
                shell.status_text = message.clone();
                shell.started_at_ms = None;
                shell.current_recording_name = None;
                shell.transition_count += 1;
            });
            message
        })?;
    reserved_output.disarm();

    {
        let recorder_state = app.state::<RecorderState>();
        let mut recorder = recorder_state
            .0
            .lock()
            .map_err(|_| "Could not store the active recorder.".to_string())?;
        *recorder = Some(ActiveRecording {
            stop_signal,
            worker,
        });
    }

    if let Err(error) = write_persisted_data(app, &persisted_snapshot) {
        let message: String = error.into();
        log_event(
            app,
            "WARN",
            "recording.persist_failed",
            serde_json::json!({ "error": message }),
        );
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
        shell.last_transcript_path = None;
        shell.transition_count += 1;
    })?;

    signal_recording_indicator(app, IndicatorSignal::Recording);
    Ok(())
}

pub(crate) fn stop_recording_inner<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    {
        let shell_state = app.state::<SharedShellState>();
        let shell = shell_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the shell state.".to_string())?;
        if shell.phase == "saving" || shell.phase == "transcribing" {
            return Err("The previous recording is still being finalized.".into());
        }
    }

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
        shell.started_at_ms = None;
        shell.transition_count += 1;
    })?;

    let app_handle = app.clone();
    std::thread::Builder::new()
        .name("recording-finalizer".into())
        .spawn(move || {
            if let Err(error) = finalize_recording_pipeline(app_handle.clone(), active) {
                log_event(
                    &app_handle,
                    "ERROR",
                    "recording.finalize_failed",
                    serde_json::json!({ "message": error }),
                );
                let _ = update_shell_snapshot(&app_handle, |shell| {
                    shell.phase = "error".into();
                    shell.status_text = error;
                    shell.started_at_ms = None;
                    shell.current_recording_name = None;
                    shell.last_transcript_path = None;
                    shell.transition_count += 1;
                });
            }
        })
        .map_err(|error| {
            let message = error.to_string();
            let _ = update_shell_snapshot(app, |shell| {
                shell.phase = "error".into();
                shell.status_text = message.clone();
                shell.started_at_ms = None;
                shell.current_recording_name = None;
                shell.transition_count += 1;
            });
            message
        })?;

    signal_recording_indicator(app, IndicatorSignal::Saved);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{reserve_wav_path, RecorderStartClaim};

    #[test]
    fn reserving_a_wav_name_takes_it_from_the_next_start() {
        let temp_dir = tempfile::tempdir().unwrap();

        let first = reserve_wav_path(temp_dir.path(), "sample").unwrap();
        let second = reserve_wav_path(temp_dir.path(), "sample").unwrap();

        assert_eq!(first.file_name().unwrap(), "sample.wav");
        assert_eq!(second.file_name().unwrap(), "sample_1.wav");
        assert!(first.exists());
        assert!(second.exists());
    }

    #[test]
    fn a_second_start_cannot_claim_the_recorder_slot() {
        let claim = RecorderStartClaim::acquire_with(|| Ok(false)).unwrap();

        // The first start is still between its check and storing the handle, so
        // the second must be turned away rather than overwrite the first.
        assert!(RecorderStartClaim::acquire_with(|| Ok(false)).is_err());

        // A failed setup step between the claim and the spawn must not wedge
        // recording until the app restarts.
        drop(claim);
        assert!(RecorderStartClaim::acquire_with(|| Ok(false)).is_ok());
    }

    #[test]
    fn an_already_running_recording_blocks_a_new_start() {
        assert!(RecorderStartClaim::acquire_with(|| Ok(true)).is_err());
    }
}

#[cfg(test)]
mod refusal_message_tests {
    use super::recording_refusal_for;

    /// One sentence used to answer every phase, and it named the recorder.
    #[test]
    fn the_refusal_names_the_thing_that_is_actually_running() {
        assert!(recording_refusal_for("downloading-model").contains("download"));
        assert!(recording_refusal_for("transcribing").contains("transcription"));
        assert!(recording_refusal_for("recording").contains("recording"));
        assert!(recording_refusal_for("saving").contains("saved"));
    }

    /// A download is not a recording, and saying so was the whole defect.
    #[test]
    fn a_download_is_never_described_as_a_recording_task() {
        let message = recording_refusal_for("downloading-model");
        assert!(
            !message.contains("previous recording task"),
            "still describing the wrong subsystem: {message}"
        );
    }

    /// Every phase gets a sentence, and none of them is empty — the caller returns this
    /// verbatim as the error a user reads.
    #[test]
    fn every_refusal_says_something() {
        for phase in ["downloading-model", "transcribing", "recording", "saving", "whatever"] {
            let message = recording_refusal_for(phase);
            assert!(!message.trim().is_empty(), "{phase} has nothing to say");
            assert!(message.ends_with('.'), "{phase}: {message}");
        }
    }
}
