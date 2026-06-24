use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::Deserialize;
use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_state::{
        is_japanese_transcript_language, sanitize_recording_name, transcript_looks_japanese,
        write_persisted_data,
    },
    app_types::{
        AnkiCatalog, AnkiSettings, RecentRecording, RecordingActionItem, RecordingBatchResult,
        SharedPersistedState,
    },
    build_app_bootstrap, emit_app_snapshot, log_event, selected_recordings,
    update_recent_recording, update_shell_snapshot,
};

const ANKI_CONNECT_TIMEOUT: Duration = Duration::from_millis(1500);
const ANKI_LOOKUP_FURIGANA_URL: &str = "http://127.0.0.1:8766/furigana";
const ANKI_LOOKUP_TIMEOUT: Duration = Duration::from_millis(2500);

struct AnkiPushOutcome {
    note_id: i64,
    furigana_message: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FuriganaBridgeResponse {
    ok: bool,
    #[serde(default)]
    furigana_html: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

fn anki_offline_message(error: &str) -> String {
    format!(
        "Anki is currently offline. Start Anki and make sure AnkiConnect is installed, then try again. {error}"
    )
}

fn anki_connect_request(
    action: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(ANKI_CONNECT_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;
    let payload = serde_json::json!({
        "action": action,
        "version": 6,
        "params": params
    });

    let response = client
        .post("http://127.0.0.1:8765")
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(payload.to_string())
        .send()
        .map_err(|error| error.to_string())?
        .error_for_status()
        .map_err(|error| error.to_string())?;
    let response = response.text().map_err(|error| error.to_string())?;
    let response =
        serde_json::from_str::<serde_json::Value>(&response).map_err(|error| error.to_string())?;

    if let Some(error) = response.get("error").and_then(|value| value.as_str()) {
        if !error.is_empty() {
            return Err(error.to_string());
        }
    }

    Ok(response
        .get("result")
        .cloned()
        .unwrap_or(serde_json::Value::Null))
}

fn anki_note_exists(note_id: i64) -> Result<bool, String> {
    let result = anki_connect_request(
        "notesInfo",
        serde_json::json!({
            "notes": [note_id]
        }),
    )?;

    let Some(notes) = result.as_array() else {
        return Ok(false);
    };

    Ok(notes.iter().any(|note| {
        note.get("noteId")
            .and_then(|value| value.as_i64())
            .is_some_and(|candidate| candidate == note_id)
    }))
}

fn anki_note_field_value(note_id: i64, field_name: &str) -> Result<Option<String>, String> {
    let result = anki_connect_request(
        "notesInfo",
        serde_json::json!({
            "notes": [note_id]
        }),
    )?;

    let Some(note) = result.as_array().and_then(|notes| {
        notes.iter().find(|note| {
            note.get("noteId")
                .and_then(|value| value.as_i64())
                .is_some_and(|candidate| candidate == note_id)
        })
    }) else {
        return Ok(None);
    };

    Ok(note
        .get("fields")
        .and_then(|fields| fields.as_object())
        .and_then(|fields| fields.get(field_name))
        .and_then(|field| field.get("value"))
        .and_then(|value| value.as_str())
        .map(ToString::to_string))
}

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

fn refresh_recording_anki_reference<R: Runtime>(
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

fn refresh_recent_anki_note_references<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
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

fn request_furigana_html(text: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(ANKI_LOOKUP_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;

    let response_text = client
        .post(ANKI_LOOKUP_FURIGANA_URL)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::json!({ "text": text }).to_string())
        .send()
        .map_err(|error| {
            format!(
                "Anki Lookup add-on is not running or did not respond. Open Anki with the Wonder of U/Anki Lookup add-on installed, then try again. {error}"
            )
        })?
        .error_for_status()
        .map_err(|error| {
            format!("Anki Lookup add-on rejected the furigana request. {error}")
        })?
        .text()
        .map_err(|error| format!("Anki Lookup add-on response could not be read. {error}"))?;
    let response = serde_json::from_str::<FuriganaBridgeResponse>(&response_text)
        .map_err(|error| format!("Anki Lookup add-on returned invalid furigana data. {error}"))?;

    if response.ok {
        response
            .furigana_html
            .ok_or_else(|| "Anki Lookup add-on did not return furigana HTML.".to_string())
    } else {
        Err(response
            .error
            .unwrap_or_else(|| "Anki Lookup add-on could not create furigana.".into()))
    }
}

fn json_string_array(value: serde_json::Value) -> Vec<String> {
    value
        .as_array()
        .map(|items| {
            items
                .iter()
                .filter_map(|item| item.as_str().map(str::to_string))
                .collect::<Vec<_>>()
        })
        .unwrap_or_default()
}

pub(crate) fn load_anki_catalog_inner<R: Runtime>(
    app: &AppHandle<R>,
    note_type: Option<String>,
) -> Result<AnkiCatalog, String> {
    let configured_note_type = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the Anki settings.".to_string())?;
        persisted.settings.anki.note_type.clone()
    };
    let selected_note_type = note_type.unwrap_or(configured_note_type).trim().to_string();

    let version = match anki_connect_request("version", serde_json::json!({})) {
        Ok(value) => value.as_i64(),
        Err(error) => {
            return Ok(AnkiCatalog {
                status: "offline".into(),
                message: anki_offline_message(&error),
                version: None,
                decks: Vec::new(),
                note_types: Vec::new(),
                fields: Vec::new(),
            });
        }
    };

    if let Err(error) = refresh_recent_anki_note_references(app) {
        log_event(
            app,
            "WARN",
            "anki.note_reference_refresh_failed",
            serde_json::json!({ "message": error }),
        );
    }

    let mut decks = json_string_array(anki_connect_request("deckNames", serde_json::json!({}))?);
    let mut note_types =
        json_string_array(anki_connect_request("modelNames", serde_json::json!({}))?);
    decks.sort();
    note_types.sort();

    let fields = if selected_note_type.is_empty() {
        Vec::new()
    } else {
        json_string_array(anki_connect_request(
            "modelFieldNames",
            serde_json::json!({ "modelName": selected_note_type }),
        )?)
    };

    Ok(AnkiCatalog {
        status: "ready".into(),
        message: "AnkiConnect is ready.".into(),
        version,
        decks,
        note_types,
        fields,
    })
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

fn anki_media_file_name(path: &Path) -> String {
    let stem = path
        .file_stem()
        .and_then(|value| value.to_str())
        .map(sanitize_recording_name)
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "recording".into())
        .replace(' ', "_");
    let extension = path
        .extension()
        .and_then(|value| value.to_str())
        .unwrap_or("wav");
    format!("wonder_of_u_{stem}.{extension}")
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\n', "<br>")
}

fn user_friendly_anki_error(error: &str, settings: &AnkiSettings) -> String {
    let normalized = error.to_lowercase();
    if normalized.contains("duplicate") {
        return format!(
            "This transcript already exists in the '{}' deck. Wonder of U did not create a duplicate card.",
            settings.deck_name
        );
    }

    if normalized.contains("model") && normalized.contains("not") && normalized.contains("found") {
        return format!(
            "Anki could not find the '{}' note type. Refresh Anki mapping and choose an available note type.",
            settings.note_type
        );
    }

    if normalized.contains("deck") && normalized.contains("not") && normalized.contains("found") {
        return format!(
            "Anki could not find the '{}' deck. Refresh Anki mapping and choose an available deck.",
            settings.deck_name
        );
    }

    if normalized.contains("field") {
        return "Anki rejected one of the mapped fields. Refresh Anki mapping and check that every selected field still exists on the note type.".into();
    }

    format!("Anki could not create the card. {error}")
}

fn push_single_recording_to_anki<R: Runtime>(
    app: &AppHandle<R>,
    recording: &RecentRecording,
    settings: &AnkiSettings,
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

    let transcript_path = recording
        .transcript_path
        .as_deref()
        .ok_or_else(|| "This recording does not have a transcript yet.".to_string())?;
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

    let furigana_message = if auto_add_furigana_after_push {
        maybe_insert_automatic_furigana_field(
            recording,
            settings,
            &transcript,
            &media_file_name,
            &mut fields,
        )
    } else {
        None
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
        recording.anki_note_id = Some(note_id);
        recording.anki_deck_name = Some(settings.deck_name.clone());
        recording.anki_note_type = Some(settings.note_type.clone());
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
) -> Option<String> {
    if !recording_transcript_supports_furigana(recording, transcript) {
        return None;
    }

    match request_furigana_html(transcript) {
        Ok(furigana_html) => {
            let target_field = settings.fields.transcription.as_str();
            let existing_value = fields.get(target_field).and_then(|value| value.as_str());
            let fallback_sound_tag =
                if !settings.fields.audio.is_empty() && settings.fields.audio == target_field {
                    Some(format!("[sound:{media_file_name}]"))
                } else {
                    None
                };
            let furigana_html = preserve_anki_sound_tags(
                existing_value,
                &furigana_html,
                fallback_sound_tag.as_deref(),
            );
            fields.insert(
                target_field.to_string(),
                serde_json::Value::String(furigana_html),
            );
            Some("Furigana was added automatically.".into())
        }
        Err(error) => Some(format!("Furigana was skipped because {error}")),
    }
}

fn prepend_anki_field_value(
    fields: &mut serde_json::Map<String, serde_json::Value>,
    field_name: &str,
    value: String,
) {
    if field_name.is_empty() {
        return;
    }

    let next_value = fields
        .get(field_name)
        .and_then(|existing| existing.as_str())
        .map(|existing| join_anki_field_parts(&value, existing))
        .unwrap_or(value);
    fields.insert(
        field_name.to_string(),
        serde_json::Value::String(next_value),
    );
}

pub(crate) fn join_anki_field_parts(first: &str, second: &str) -> String {
    let first = first.trim();
    let second = second.trim();
    match (first.is_empty(), second.is_empty()) {
        (true, true) => String::new(),
        (true, false) => second.to_string(),
        (false, true) => first.to_string(),
        (false, false) => format!("{first}<br>{second}"),
    }
}

pub(crate) fn preserve_anki_sound_tags(
    existing_value: Option<&str>,
    new_value: &str,
    fallback_sound_tag: Option<&str>,
) -> String {
    let mut sound_tags = existing_value
        .map(extract_anki_sound_tags)
        .unwrap_or_default();

    if let Some(fallback_sound_tag) = fallback_sound_tag {
        if !new_value.contains(fallback_sound_tag)
            && !sound_tags.iter().any(|tag| tag == fallback_sound_tag)
        {
            sound_tags.push(fallback_sound_tag.to_string());
        }
    }

    let sound_prefix = sound_tags.join(" ");
    join_anki_field_parts(&sound_prefix, new_value)
}

fn extract_anki_sound_tags(value: &str) -> Vec<String> {
    let mut tags = Vec::new();
    let mut remaining = value;
    while let Some(start) = remaining.find("[sound:") {
        let candidate = &remaining[start..];
        let Some(end) = candidate.find(']') else {
            break;
        };
        let tag = candidate[..=end].to_string();
        if !tags.contains(&tag) {
            tags.push(tag);
        }
        remaining = &candidate[end + 1..];
    }
    tags
}

pub(crate) fn recording_pushed_to_anki_target(
    recording: &RecentRecording,
    settings: &AnkiSettings,
) -> bool {
    recording.anki_note_id.is_some()
        && recording.anki_deck_name.as_deref() == Some(settings.deck_name.as_str())
        && recording.anki_note_type.as_deref() == Some(settings.note_type.as_str())
}

pub(crate) fn push_recordings_to_anki_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let (settings, delete_local_audio_after_push, auto_add_furigana_after_push) = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the Anki settings.".to_string())?;
        (
            persisted.settings.anki.clone(),
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

    let (mut settings, delete_local_audio_after_push, auto_add_furigana_after_push) = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the Anki settings.".to_string())?;
        (
            persisted.settings.anki.clone(),
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
        delete_local_audio_after_push,
        auto_add_furigana_after_push,
    )
}

fn push_recordings_to_anki_with_settings_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
    settings: AnkiSettings,
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

        if recording_pushed_to_anki_target(&recording, &settings) {
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

pub(crate) fn add_furigana_to_anki_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let settings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the Anki settings.".to_string())?;
        persisted.settings.anki.clone()
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
        match add_furigana_to_single_anki_card(app, recording, &settings) {
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
) -> Result<i64, (String, Option<i64>, String)> {
    let file_path = recording.file_path.clone();
    let note_id = recording.anki_note_id;

    let Some(note_id) = note_id else {
        return Err((
            file_path,
            None,
            "Push this recording to Anki before adding furigana.".into(),
        ));
    };

    let recording = refresh_recording_anki_reference(app, recording).map_err(|error| {
        (
            file_path.clone(),
            Some(note_id),
            format!("Could not verify the existing Anki card: {error}"),
        )
    })?;
    if recording.anki_note_id.is_none() {
        return Err((
            file_path,
            Some(note_id),
            "The Anki card was deleted. Push this recording again before adding furigana.".into(),
        ));
    }

    let transcript_path = recording.transcript_path.as_deref().ok_or_else(|| {
        (
            file_path.clone(),
            Some(note_id),
            "Transcribe this recording before adding furigana.".into(),
        )
    })?;
    let transcript = fs::read_to_string(transcript_path).map_err(|error| {
        (
            file_path.clone(),
            Some(note_id),
            format!("Could not read transcript: {error}"),
        )
    })?;
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

    Ok(note_id)
}

pub(crate) fn recording_transcript_supports_furigana(
    recording: &RecentRecording,
    transcript: &str,
) -> bool {
    if is_japanese_transcript_language(recording.transcript_language.as_deref()) {
        return true;
    }

    if recording.transcript_language.is_some() {
        return false;
    }

    transcript_looks_japanese(transcript)
}
