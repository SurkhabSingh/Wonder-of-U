use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_runtime::emit_app_snapshot,
    app_state::write_persisted_data,
    app_types::{RecentRecording, SharedPersistedState},
    recording_library::update_recent_recording,
};

use super::client::{anki_note_exists, anki_note_snapshot};

fn field_contains_furigana(value: Option<&str>) -> bool {
    value.is_some_and(|value| {
        let normalized = value.to_ascii_lowercase();
        normalized.contains("<ruby") && normalized.contains("<rt")
    })
}

fn clear_recording_anki_reference<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
) -> Result<(), String> {
    update_recent_recording(app, file_path, |recording| {
        recording.anki_note_id = None;
        recording.anki_deck_name = None;
        recording.anki_note_type = None;
        recording.furigana_applied = false;
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
    recording.furigana_applied = false;
    Ok(recording)
}

pub(super) fn refresh_recent_anki_note_references<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<(), String> {
    let (recordings_with_notes, target_field) = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not inspect pushed Anki cards.".to_string())?;
        let recordings = persisted
            .recent_recordings
            .iter()
            .filter_map(|recording| {
                recording.anki_note_id.map(|note_id| {
                    (
                        recording.file_path.clone(),
                        note_id,
                        recording.furigana_applied,
                    )
                })
            })
            .collect::<Vec<_>>();
        (
            recordings,
            persisted.settings.anki.fields.transcription.clone(),
        )
    };

    let mut updates = Vec::new();
    for (file_path, note_id, current_furigana_applied) in recordings_with_notes {
        let snapshot = anki_note_snapshot(
            note_id,
            (!target_field.is_empty()).then_some(target_field.as_str()),
        )?;
        let furigana_applied = if target_field.is_empty() {
            current_furigana_applied
        } else {
            field_contains_furigana(snapshot.field_value.as_deref())
        };

        if !snapshot.exists || furigana_applied != current_furigana_applied {
            updates.push((file_path, snapshot.exists, furigana_applied));
        }
    }

    if updates.is_empty() {
        return Ok(());
    }

    let persisted_snapshot = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update pushed Anki card status.".to_string())?;
        for recording in &mut persisted.recent_recordings {
            if let Some((_, note_exists, furigana_applied)) = updates
                .iter()
                .find(|(file_path, _, _)| file_path == &recording.file_path)
            {
                if *note_exists {
                    recording.furigana_applied = *furigana_applied;
                } else {
                    recording.anki_note_id = None;
                    recording.anki_deck_name = None;
                    recording.anki_note_type = None;
                    recording.furigana_applied = false;
                }
            }
        }
        persisted.clone()
    };

    write_persisted_data(app, &persisted_snapshot)?;
    emit_app_snapshot(app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::field_contains_furigana;

    #[test]
    fn furigana_detection_requires_ruby_and_reading_markup() {
        assert!(field_contains_furigana(Some(
            "<ruby>日本語<rt>にほんご</rt></ruby>"
        )));
        assert!(!field_contains_furigana(Some("<ruby>日本語</ruby>")));
        assert!(!field_contains_furigana(Some("日本語")));
        assert!(!field_contains_furigana(None));
    }
}
