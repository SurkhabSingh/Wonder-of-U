use std::{fs, path::PathBuf, process::Command};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Manager, Runtime};

mod conversion;
mod transcription;

pub(crate) use conversion::convert_recordings_to_mp3_inner;
pub(crate) use transcription::{
    rename_recording_outputs_from_transcript, transcribe_recordings_inner,
};

use crate::{
    app_runtime::{build_app_bootstrap, emit_app_snapshot, log_event, update_shell_snapshot},
    app_state::write_persisted_data,
    app_types::{RecentRecording, RecordingActionItem, RecordingBatchResult, SharedPersistedState},
};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn find_recent_recording<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
) -> Result<RecentRecording, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not read the recording history.".to_string())?;
    persisted
        .recent_recordings
        .iter()
        .find(|recording| recording.file_path == file_path)
        .cloned()
        .ok_or_else(|| "The selected recording is no longer in the recent list.".to_string())
}

pub(crate) fn update_recent_recording<R: Runtime, F>(
    app: &AppHandle<R>,
    file_path: &str,
    update: F,
) -> Result<(), String>
where
    F: FnOnce(&mut RecentRecording),
{
    let persisted_snapshot = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update the recording history.".to_string())?;
        let recording = persisted
            .recent_recordings
            .iter_mut()
            .find(|recording| recording.file_path == file_path)
            .ok_or_else(|| "The selected recording is no longer in the recent list.".to_string())?;
        update(recording);
        persisted.clone()
    };

    write_persisted_data(app, &persisted_snapshot)?;
    emit_app_snapshot(app);
    Ok(())
}

pub(crate) fn selected_recordings<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
) -> Result<Vec<RecentRecording>, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not read the recording history.".to_string())?;
    let recordings = if file_paths.is_empty() {
        persisted
            .recent_recordings
            .iter()
            .filter(|recording| recording.transcript_path.is_some())
            .cloned()
            .collect()
    } else {
        file_paths
            .iter()
            .filter_map(|file_path| {
                persisted
                    .recent_recordings
                    .iter()
                    .find(|recording| recording.file_path == *file_path)
                    .cloned()
            })
            .collect()
    };

    Ok(recordings)
}

pub(crate) fn delete_recording_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
) -> Result<(), String> {
    let removed_recording = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update the recording history.".to_string())?;
        let index = persisted
            .recent_recordings
            .iter()
            .position(|recording| recording.file_path == file_path)
            .ok_or_else(|| "The selected recording is no longer in the recent list.".to_string())?;
        let removed = persisted.recent_recordings.remove(index);
        let snapshot = persisted.clone();
        drop(persisted);
        write_persisted_data(app, &snapshot)?;
        removed
    };

    for path in [
        Some(removed_recording.file_path.as_str()),
        removed_recording.transcript_path.as_deref(),
        removed_recording.translation_path.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Could not delete {path}: {error}")),
        }
    }

    log_event(
        app,
        "INFO",
        "recording.deleted",
        serde_json::json!({ "filePath": file_path }),
    );
    emit_app_snapshot(app);
    Ok(())
}

pub(crate) fn delete_recordings_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let mut items = Vec::new();

    for file_path in file_paths {
        match delete_recording_inner(app, &file_path) {
            Ok(()) => items.push(RecordingActionItem {
                file_path,
                status: "success".into(),
                message: "Deleted recording files.".into(),
                note_id: None,
            }),
            Err(error) => items.push(RecordingActionItem {
                file_path,
                status: "failed".into(),
                message: error,
                note_id: None,
            }),
        }
    }

    let success_count = items.iter().filter(|item| item.status == "success").count();
    let failed_count = items.iter().filter(|item| item.status == "failed").count();
    let message = format!("Delete finished: {success_count} deleted, {failed_count} failed.");

    update_shell_snapshot(app, |shell| {
        shell.status_text = message.clone();
        shell.transition_count += 1;
    })?;

    Ok(RecordingBatchResult {
        status: if failed_count == 0 {
            "completed"
        } else {
            "partial"
        }
        .into(),
        message,
        items,
        bootstrap: build_app_bootstrap(app)?,
    })
}

pub(crate) fn play_recording_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
) -> Result<(), String> {
    let recording = find_recent_recording(app, file_path)?;
    let path = PathBuf::from(&recording.file_path);
    if !path.exists() {
        return Err("The audio file is missing from disk.".into());
    }

    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("cmd");
        command.creation_flags(CREATE_NO_WINDOW);
        command.arg("/C").arg("start").arg("").arg(&path);
        command.spawn().map_err(|error| error.to_string())?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|error| error.to_string())?;
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

pub(crate) fn translate_recordings_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let recordings = selected_recordings(app, file_paths)?;
    let mut items = Vec::new();

    for recording in recordings {
        if recording.translation_path.is_some() {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "skipped".into(),
                message: "Already translated.".into(),
                note_id: recording.anki_note_id,
            });
        } else if recording.transcript_path.is_none() {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "failed".into(),
                message: "No transcript available to translate.".into(),
                note_id: recording.anki_note_id,
            });
        } else {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "failed".into(),
                message: "Translation provider is not configured yet. This will be wired through the translation/extension bridge phase.".into(),
                note_id: recording.anki_note_id,
            });
        }
    }

    let skipped_count = items.iter().filter(|item| item.status == "skipped").count();
    let failed_count = items.iter().filter(|item| item.status == "failed").count();

    Ok(RecordingBatchResult {
        status: if failed_count == 0 {
            "completed"
        } else {
            "unavailable"
        }
        .into(),
        message: format!(
            "Translation request finished: {skipped_count} skipped, {failed_count} unavailable."
        ),
        items,
        bootstrap: build_app_bootstrap(app)?,
    })
}
