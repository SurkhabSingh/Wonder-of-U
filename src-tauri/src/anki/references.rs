use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_runtime::emit_app_snapshot,
    app_state::write_persisted_data,
    app_types::{RecentRecording, SharedPersistedState},
    recording_library::update_recent_recording,
};

use super::client::anki_note_exists;

fn clear_recording_anki_reference<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
) -> Result<(), String> {
    update_recent_recording(app, file_path, |recording| {
        recording.anki_note_id = None;
        recording.anki_deck_name = None;
        recording.anki_note_type = None;
    })
}

pub(super) fn refresh_recording_anki_reference<R: Runtime>(
    app: &AppHandle<R>,
    mut recording: RecentRecording,
) -> Result<RecentRecording, String> {
    let Some(note_id) = recording.anki_note_id else {
        return Ok(recording);
    };

    if anki_note_exists(note_id)? {
        return Ok(recording);
    }

    clear_recording_anki_reference(app, &recording.file_path)?;
    recording.anki_note_id = None;
    recording.anki_deck_name = None;
    recording.anki_note_type = None;
    Ok(recording)
}

pub(super) fn refresh_recent_anki_note_references<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
    let recordings_with_notes = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not inspect pushed Anki cards.".to_string())?;
        persisted
            .recent_recordings
            .iter()
            .filter_map(|recording| {
                recording
                    .anki_note_id
                    .map(|note_id| (recording.file_path.clone(), note_id))
            })
            .collect::<Vec<_>>()
    };

    let mut missing_file_paths = Vec::new();
    for (file_path, note_id) in recordings_with_notes {
        if !anki_note_exists(note_id)? {
            missing_file_paths.push(file_path);
        }
    }

    if missing_file_paths.is_empty() {
        return Ok(());
    }

    let persisted_snapshot = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update pushed Anki card status.".to_string())?;
        for recording in &mut persisted.recent_recordings {
            if missing_file_paths
                .iter()
                .any(|file_path| file_path == &recording.file_path)
            {
                recording.anki_note_id = None;
                recording.anki_deck_name = None;
                recording.anki_note_type = None;
            }
        }
        persisted.clone()
    };

    write_persisted_data(app, &persisted_snapshot)?;
    emit_app_snapshot(app);
    Ok(())
}
