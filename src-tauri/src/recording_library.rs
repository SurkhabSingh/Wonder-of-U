use tauri::{AppHandle, Manager, Runtime};

mod actions;
mod conversion;
mod texts;
mod transcription;

pub(crate) use actions::{
    auto_translate_after_transcription, delete_recording_inner, delete_recordings_inner,
    play_recording_inner, translate_recordings_inner,
};
pub(crate) use conversion::convert_recordings_to_mp3_inner;
pub(crate) use texts::read_recording_texts_inner;
pub(crate) use transcription::{
    rename_recording_outputs_from_transcript, transcribe_recordings_inner,
};

use crate::{
    app_runtime::emit_app_snapshot,
    app_state::write_persisted_data,
    app_types::{RecentRecording, SharedPersistedState},
};

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
