use std::path::{Path, PathBuf};

use tauri::{AppHandle, Manager, Runtime};

mod actions;
mod conversion;
mod import;
mod texts;
mod transcription;

pub(crate) use actions::{
    delete_recording_inner, delete_recordings_inner, play_recording_inner, playback_path,
    translate_recordings_inner,
};
pub(crate) use conversion::convert_recordings_to_mp3_inner;
pub(crate) use import::{import_media_inner, import_youtube_inner};
pub(crate) use texts::{parse_translation_language, read_recording_texts_inner};
pub(crate) use transcription::transcribe_recordings_inner;
// Production code renames via `transcribe_recordings_inner`; only the lib tests
// reach the rename helper through the module facade, so gate the re-export to the
// test build to keep the non-test build free of an unused-import warning.
#[cfg(test)]
pub(crate) use transcription::rename_recording_outputs_from_transcript;

use crate::{
    app_runtime::emit_app_snapshot,
    app_state::write_persisted_data,
    app_types::{RecentRecording, SharedPersistedState},
};

/// Returns a path inside `directory` that does not exist yet, appending `_1`,
/// `_2`, ... to the stem until it is free. The "does not exist" guarantee is what
/// keeps an import from ever writing onto its own source file.
pub(crate) fn unique_path_with_suffix(
    directory: &Path,
    file_stem: &str,
    suffix: &str,
) -> PathBuf {
    let sanitized_stem = if file_stem.is_empty() {
        "recording".to_string()
    } else {
        file_stem.to_string()
    };

    let mut attempt = 0usize;
    loop {
        let candidate = if attempt == 0 {
            directory.join(format!("{sanitized_stem}{suffix}"))
        } else {
            directory.join(format!("{sanitized_stem}_{attempt}{suffix}"))
        };

        if !candidate.exists() {
            return candidate;
        }

        attempt += 1;
    }
}

/// Pushes a freshly created recording to the front of the persisted history and
/// flushes it to disk. The lock is released before the write.
pub(crate) fn insert_recent_recording<R: Runtime>(
    app: &AppHandle<R>,
    recent_recording: RecentRecording,
) -> Result<(), String> {
    let persisted_snapshot = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update the recording history.".to_string())?;
        persisted.recent_recordings.insert(0, recent_recording);
        persisted.clone()
    };

    write_persisted_data(app, &persisted_snapshot)
}

pub(crate) fn find_recent_recording<R: Runtime>(
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
