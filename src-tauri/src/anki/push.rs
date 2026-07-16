use std::{fs, path::PathBuf};

use tauri::{AppHandle, Manager, Runtime};

use super::{
    client::{anki_connect_request, anki_offline_message},
    fields::{
        anki_media_file_name, html_escape, prepend_anki_field_value,
        recording_pushed_to_anki_target, user_friendly_anki_error,
    },
    furigana::{
        insert_furigana_field, recording_transcript_supports_furigana, request_furigana_html,
    },
    references::refresh_recording_anki_reference,
};
use crate::{
    app_runtime::{build_app_bootstrap, update_shell_snapshot},
    app_types::{
        transcript_language_key, AnkiSettings, RecentRecording, RecordingActionItem,
        RecordingAnkiPush, RecordingBatchResult, SharedPersistedState,
    },
    recording_library::{selected_recordings, update_recent_recording},
};

struct AnkiPushOutcome {
    note_id: i64,
    furigana_message: Option<String>,
}

fn delete_local_audio_after_anki_push<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
) -> Result<(), String> {
    let audio_path = PathBuf::from(file_path);
    match fs::remove_file(&audio_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            return Err(format!(
                "Anki card was created, but local audio cleanup failed: {error}"
            ));
        }
    }

    update_recent_recording(app, file_path, |recording| {
        recording.audio_deleted = true;
        recording.bytes_written = 0;
    })
}

fn push_single_recording_to_anki<R: Runtime>(
    app: &AppHandle<R>,
    recording: &RecentRecording,
    settings: &AnkiSettings,
    transcription_language: &str,
    auto_add_furigana_after_push: bool,
) -> Result<AnkiPushOutcome, String> {
    if settings.deck_name.is_empty() {
        return Err("Choose an Anki deck before pushing recordings.".into());
    }
    if settings.note_type.is_empty() {
        return Err("Choose an Anki note type before pushing recordings.".into());
    }
    if settings.fields.transcription.is_empty() {
        return Err("Map an Anki field for the transcript before pushing recordings.".into());
    }

    let transcript_variant = recording.transcript_for_language(transcription_language);
    let transcript_path = transcript_variant
        .map(|transcript| transcript.file_path.as_str())
        .or_else(|| {
            recording
                .transcripts
                .is_empty()
                .then_some(recording.transcript_path.as_deref())
                .flatten()
        })
        .ok_or_else(|| {
            format!("This recording has not been transcribed for {transcription_language} yet.")
        })?;
    let transcript = fs::read_to_string(transcript_path)
        .map_err(|error| format!("Could not read transcript: {error}"))?;
    let audio_path = PathBuf::from(&recording.file_path);
    if !audio_path.exists() {
        return Err("The audio file is missing from disk.".into());
    }

    let media_file_name = anki_media_file_name(&audio_path);
    anki_connect_request(
        "storeMediaFile",
        serde_json::json!({
            "filename": media_file_name,
            "path": recording.file_path
        }),
    )?;

    let mut fields = serde_json::Map::new();
    fields.insert(
        settings.fields.transcription.clone(),
        serde_json::Value::String(html_escape(&transcript)),
    );
    prepend_anki_field_value(
        &mut fields,
        &settings.fields.audio,
        format!("[sound:{media_file_name}]"),
    );
    if !settings.fields.source_path.is_empty() {
        fields.insert(
            settings.fields.source_path.clone(),
            serde_json::Value::String(html_escape(&recording.file_path)),
        );
    }
    if !settings.fields.created_at.is_empty() {
        fields.insert(
            settings.fields.created_at.clone(),
            serde_json::Value::String(recording.created_at_ms.to_string()),
        );
    }
    if !settings.fields.translation.is_empty() {
        if let Some(translation_path) = recording.translation_path.as_deref() {
            if let Ok(translation) = fs::read_to_string(translation_path) {
                fields.insert(
                    settings.fields.translation.clone(),
                    serde_json::Value::String(html_escape(&translation)),
                );
            }
        }
    }

    let mut transcript_recording = recording.clone();
    if let Some(transcript) = transcript_variant {
        transcript_recording.transcript_path = Some(transcript.file_path.clone());
        transcript_recording.transcript_language = transcript.detected_language.clone();
    }
    let (furigana_message, furigana_applied) = if auto_add_furigana_after_push {
        maybe_insert_automatic_furigana_field(
            &transcript_recording,
            settings,
            &transcript,
            &media_file_name,
            &mut fields,
        )
    } else {
        (None, false)
    };

    let note_id = anki_connect_request(
        "addNote",
        serde_json::json!({
            "note": {
                "deckName": settings.deck_name.clone(),
                "modelName": settings.note_type.clone(),
                "fields": fields,
                "options": {
                    "allowDuplicate": false,
                    "duplicateScope": "deck",
                    "duplicateScopeOptions": {
                        "deckName": settings.deck_name.clone(),
                        "checkChildren": false,
                        "checkAllModels": false
                    }
                },
                "tags": ["wonder-of-u"]
            }
        }),
    )
    .map_err(|error| user_friendly_anki_error(&error, settings))?
    .as_i64()
    .ok_or_else(|| "AnkiConnect did not return a note id.".to_string())?;

    update_recent_recording(app, &recording.file_path, |recording| {
        let language = transcript_language_key(transcription_language);
        recording.anki_pushes.retain(|push| {
            push.language != language
                || push.deck_name != settings.deck_name
                || push.note_type != settings.note_type
        });
        recording.anki_pushes.push(RecordingAnkiPush {
            language,
            deck_name: settings.deck_name.clone(),
            note_type: settings.note_type.clone(),
            note_id,
            furigana_applied,
        });
        recording.anki_note_id = Some(note_id);
        recording.anki_deck_name = Some(settings.deck_name.clone());
        recording.anki_note_type = Some(settings.note_type.clone());
        recording.furigana_applied = furigana_applied;
    })?;

    Ok(AnkiPushOutcome {
        note_id,
        furigana_message,
    })
}

fn maybe_insert_automatic_furigana_field(
    recording: &RecentRecording,
    settings: &AnkiSettings,
    transcript: &str,
    media_file_name: &str,
    fields: &mut serde_json::Map<String, serde_json::Value>,
) -> (Option<String>, bool) {
    if !recording_transcript_supports_furigana(recording, transcript) {
        return (None, false);
    }

    match request_furigana_html(transcript) {
        Ok(furigana_html) => {
            insert_furigana_field(settings, &furigana_html, media_file_name, fields);
            (Some("Furigana was added automatically.".into()), true)
        }
        Err(error) => (Some(format!("Furigana was skipped because {error}")), false),
    }
}

pub(crate) fn push_recordings_to_anki_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let (
        settings,
        transcription_language,
        delete_local_audio_after_push,
        auto_add_furigana_after_push,
    ) = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the Anki settings.".to_string())?;
        (
            persisted.settings.anki.clone(),
            persisted.settings.whisper.language.clone(),
            persisted
                .settings
                .features
                .delete_local_audio_after_anki_push,
            persisted
                .settings
                .features
                .auto_add_furigana_after_anki_push,
        )
    };
    push_recordings_to_anki_with_settings_inner(
        app,
        file_paths,
        settings,
        transcription_language,
        delete_local_audio_after_push,
        auto_add_furigana_after_push,
    )
}

pub(crate) fn push_recordings_to_anki_deck_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
    deck_name: String,
) -> Result<RecordingBatchResult, String> {
    let deck_name = deck_name.trim().to_string();
    if deck_name.is_empty() {
        return Err("Choose an Anki deck before pushing recordings.".into());
    }

    let (
        mut settings,
        transcription_language,
        delete_local_audio_after_push,
        auto_add_furigana_after_push,
    ) = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the Anki settings.".to_string())?;
        (
            persisted.settings.anki.clone(),
            persisted.settings.whisper.language.clone(),
            persisted
                .settings
                .features
                .delete_local_audio_after_anki_push,
            persisted
                .settings
                .features
                .auto_add_furigana_after_anki_push,
        )
    };
    settings.deck_name = deck_name;
    push_recordings_to_anki_with_settings_inner(
        app,
        file_paths,
        settings,
        transcription_language,
        delete_local_audio_after_push,
        auto_add_furigana_after_push,
    )
}

fn push_recordings_to_anki_with_settings_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
    settings: AnkiSettings,
    transcription_language: String,
    delete_local_audio_after_push: bool,
    auto_add_furigana_after_push: bool,
) -> Result<RecordingBatchResult, String> {
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
        let original_file_path = recording.file_path.clone();
        let original_note_id = recording.anki_note_id;
        let recording = match refresh_recording_anki_reference(app, recording) {
            Ok(recording) => recording,
            Err(error) => {
                items.push(RecordingActionItem {
                    file_path: original_file_path,
                    status: "failed".into(),
                    message: format!("Could not verify the existing Anki card: {error}"),
                    note_id: original_note_id,
                });
                continue;
            }
        };

        if recording_pushed_to_anki_target(&recording, &settings, &transcription_language) {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "skipped".into(),
                message: format!("Already pushed to {}.", settings.deck_name),
                note_id: recording.anki_note_id,
            });
            continue;
        }

        match push_single_recording_to_anki(
            app,
            &recording,
            &settings,
            &transcription_language,
            auto_add_furigana_after_push,
        ) {
            Ok(outcome) => {
                let note_id = outcome.note_id;
                let mut message = format!("Created Anki note {note_id}.");
                if let Some(furigana_message) = outcome.furigana_message {
                    message.push(' ');
                    message.push_str(&furigana_message);
                }
                if delete_local_audio_after_push {
                    match delete_local_audio_after_anki_push(app, &recording.file_path) {
                        Ok(()) => {
                            message.push_str(" Local audio was deleted after Anki copied it.");
                        }
                        Err(error) => {
                            message.push_str(&format!(" {error}"));
                        }
                    }
                }
                items.push(RecordingActionItem {
                    file_path: recording.file_path,
                    status: "success".into(),
                    message,
                    note_id: Some(note_id),
                });
            }
            Err(error) => items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "failed".into(),
                message: error,
                note_id: None,
            }),
        }
    }

    let success_count = items.iter().filter(|item| item.status == "success").count();
    let skipped_count = items.iter().filter(|item| item.status == "skipped").count();
    let failed_count = items.iter().filter(|item| item.status == "failed").count();

    update_shell_snapshot(app, |shell| {
        shell.status_text = format!(
            "Anki push finished: {success_count} created, {skipped_count} skipped, {failed_count} failed."
        );
        shell.transition_count += 1;
    })?;

    Ok(RecordingBatchResult {
        status: if failed_count == 0 { "completed" } else { "partial" }.into(),
        message: format!(
            "Anki push finished: {success_count} created, {skipped_count} skipped, {failed_count} failed."
        ),
        items,
        bootstrap: build_app_bootstrap(app)?,
    })
}
