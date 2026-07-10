use std::{fs, path::PathBuf};

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_runtime::{build_app_bootstrap, update_shell_snapshot},
    app_types::{
        AnkiSettings, RecentRecording, RecordingActionItem, RecordingBatchResult,
        SharedPersistedState,
    },
    recording_library::{selected_recordings, update_recent_recording},
};

use super::{
    client::{anki_connect_request, anki_note_field_value, anki_offline_message},
    fields::{anki_media_file_name, preserve_anki_sound_tags, user_friendly_anki_error},
    furigana::{recording_transcript_supports_furigana, request_furigana_html},
    references::refresh_recording_anki_reference,
};

pub(crate) fn add_furigana_to_anki_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let (settings, transcription_language) = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the Anki settings.".to_string())?;
        (
            persisted.settings.anki.clone(),
            persisted.settings.whisper.language.clone(),
        )
    };

    if settings.fields.transcription.is_empty() {
        return Err(
            "Map the expression/transcript Anki field before updating existing cards.".into(),
        );
    }

    let recordings = selected_recordings(app, file_paths)?;
    let mut items = Vec::new();

    if let Err(error) = anki_connect_request("version", serde_json::json!({})) {
        let message = anki_offline_message(&error);
        update_shell_snapshot(app, |shell| {
            shell.status_text = message.clone();
            shell.transition_count += 1;
        })?;
        return Ok(RecordingBatchResult {
            status: "unavailable".into(),
            message,
            items: recordings
                .into_iter()
                .map(|recording| RecordingActionItem {
                    file_path: recording.file_path,
                    status: "failed".into(),
                    message: "Anki is currently offline.".into(),
                    note_id: recording.anki_note_id,
                })
                .collect(),
            bootstrap: build_app_bootstrap(app)?,
        });
    }

    for recording in recordings {
        let file_path = recording.file_path.clone();
        match add_furigana_to_single_anki_card(app, recording, &settings, &transcription_language) {
            Ok(note_id) => items.push(RecordingActionItem {
                file_path,
                status: "success".into(),
                message: format!("Updated Anki note {note_id} with furigana."),
                note_id: Some(note_id),
            }),
            Err((file_path, note_id, error)) => items.push(RecordingActionItem {
                file_path,
                status: "failed".into(),
                message: error,
                note_id,
            }),
        }
    }

    let success_count = items.iter().filter(|item| item.status == "success").count();
    let failed_count = items.iter().filter(|item| item.status == "failed").count();
    let message =
        format!("Furigana update finished: {success_count} updated, {failed_count} failed.");

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

fn add_furigana_to_single_anki_card<R: Runtime>(
    app: &AppHandle<R>,
    recording: RecentRecording,
    settings: &AnkiSettings,
    transcription_language: &str,
) -> Result<i64, (String, Option<i64>, String)> {
    let file_path = recording.file_path.clone();
    let mut recording = refresh_recording_anki_reference(app, recording).map_err(|error| {
        (
            file_path.clone(),
            None,
            format!("Could not verify the existing Anki card: {error}"),
        )
    })?;
    let matching_push = recording
        .anki_push_for_target(
            transcription_language,
            &settings.deck_name,
            &settings.note_type,
        )
        .cloned();
    let note_id = matching_push.as_ref().map(|push| push.note_id).or_else(|| {
        recording
            .anki_pushes
            .is_empty()
            .then_some(recording.anki_note_id)
            .flatten()
    });
    let Some(note_id) = note_id else {
        return Err((
            file_path,
            None,
            "Push this language to the configured Anki deck before adding furigana.".into(),
        ));
    };

    let transcript_variant = recording
        .transcript_for_language(transcription_language)
        .cloned();
    let transcript_path = transcript_variant
        .as_ref()
        .map(|transcript| transcript.file_path.as_str())
        .or_else(|| {
            recording
                .transcripts
                .is_empty()
                .then_some(recording.transcript_path.as_deref())
                .flatten()
        })
        .ok_or_else(|| {
            (
                file_path.clone(),
                Some(note_id),
                format!("Transcribe this recording for {transcription_language} before adding furigana."),
            )
        })?;
    let transcript = fs::read_to_string(transcript_path).map_err(|error| {
        (
            file_path.clone(),
            Some(note_id),
            format!("Could not read transcript: {error}"),
        )
    })?;
    if let Some(transcript_variant) = transcript_variant {
        recording.transcript_path = Some(transcript_variant.file_path);
        recording.transcript_language = transcript_variant.detected_language;
    }
    if !recording_transcript_supports_furigana(&recording, &transcript) {
        return Err((
            file_path,
            Some(note_id),
            "Add furigana is only available for Japanese transcripts.".into(),
        ));
    }
    let furigana_html = request_furigana_html(&transcript)
        .map_err(|error| (file_path.clone(), Some(note_id), error))?;
    let target_field = settings.fields.transcription.as_str();
    let existing_furigana_field_value =
        anki_note_field_value(note_id, target_field).map_err(|error| {
            (
                file_path.clone(),
                Some(note_id),
                format!(
                    "Could not read the existing expression field before adding furigana: {error}"
                ),
            )
        })?;
    let media_file_name = anki_media_file_name(&PathBuf::from(&recording.file_path));
    let fallback_sound_tag =
        if !settings.fields.audio.is_empty() && settings.fields.audio == target_field {
            Some(format!("[sound:{media_file_name}]"))
        } else {
            None
        };
    let furigana_html = preserve_anki_sound_tags(
        existing_furigana_field_value.as_deref(),
        &furigana_html,
        fallback_sound_tag.as_deref(),
    );

    let mut fields = serde_json::Map::new();
    fields.insert(
        target_field.to_string(),
        serde_json::Value::String(furigana_html),
    );
    anki_connect_request(
        "updateNoteFields",
        serde_json::json!({
            "note": {
                "id": note_id,
                "fields": fields
            }
        }),
    )
    .map_err(|error| {
        (
            file_path,
            Some(note_id),
            user_friendly_anki_error(&error, settings),
        )
    })?;

    update_recent_recording(app, &recording.file_path, |recording| {
        if let Some(push) = recording.anki_pushes.iter_mut().find(|push| {
            push.note_id == note_id
                && push.deck_name == settings.deck_name
                && push.note_type == settings.note_type
        }) {
            push.furigana_applied = true;
        }
        if recording.anki_note_id == Some(note_id) {
            recording.furigana_applied = true;
        }
    })
    .map_err(|error| {
        (
            recording.file_path.clone(),
            Some(note_id),
            format!("Furigana was added, but its local status could not be saved: {error}"),
        )
    })?;

    Ok(note_id)
}
