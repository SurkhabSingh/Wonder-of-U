use std::collections::BTreeMap;

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_runtime::log_event,
    app_types::{AnkiCatalog, SharedPersistedState},
};

use super::{
    client::{
        anki_connect_health_check, anki_connect_request, anki_offline_message, json_string_array,
    },
    references::refresh_recent_anki_note_references,
};

/// Both note-type arguments are overrides for the settings on disk, and exist for
/// the same reason: the picker calls this the moment the user changes a note type,
/// before the debounced settings save has necessarily landed. Passing the choice
/// through explicitly is what stops the field list from lagging one selection
/// behind. `None` means "whatever is persisted".
///
/// `vocabulary_note_type` is the note type a vocabulary row was just pointed at,
/// so its fields are in the map even before that row is saved.
pub(crate) fn load_anki_catalog_inner<R: Runtime>(
    app: &AppHandle<R>,
    note_type: Option<String>,
    vocabulary_note_type: Option<String>,
) -> Result<AnkiCatalog, String> {
    let (configured_note_type, mut vocabulary_note_types) = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the Anki settings.".to_string())?;
        let note_types = persisted
            .settings
            .anki
            .vocabulary_sources
            .iter()
            .map(|source| source.note_type.clone())
            .collect::<Vec<_>>();
        (persisted.settings.anki.note_type.clone(), note_types)
    };
    let selected_note_type = note_type.unwrap_or(configured_note_type).trim().to_string();
    // The row being edited is not on disk yet, so add its note type to whatever is
    // persisted rather than replacing it — the other rows still need their fields.
    if let Some(pending) = vocabulary_note_type {
        vocabulary_note_types.push(pending);
    }

    let version = match anki_connect_health_check() {
        Ok(value) => value.as_i64(),
        Err(error) => {
            return Ok(AnkiCatalog {
                status: "offline".into(),
                message: anki_offline_message(&error),
                version: None,
                decks: Vec::new(),
                note_types: Vec::new(),
                fields: Vec::new(),
                vocabulary_field_map: BTreeMap::new(),
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

    let fields = model_field_names(&selected_note_type)?;
    let vocabulary_field_map =
        vocabulary_field_map(&selected_note_type, &fields, &vocabulary_note_types)?;

    Ok(AnkiCatalog {
        status: "ready".into(),
        message: "AnkiConnect is ready.".into(),
        version,
        decks,
        note_types,
        fields,
        vocabulary_field_map,
    })
}

/// Fields for each distinct vocabulary note type, one `modelFieldNames` call per
/// distinct type. Eager rather than lazy-per-row on purpose: the distinct count is
/// tiny (a learner's vocabulary spans two or three note types, not dozens), so the
/// cost is a couple of localhost round trips, and doing it here means every row's
/// dropdown is populated the moment the catalog lands — no per-row fetch to
/// sequence, and no frontend map to accumulate and risk dropping on the next
/// catalog refresh. The push note type's fields are already in hand, so a
/// vocabulary row that happens to reuse it costs nothing.
fn vocabulary_field_map(
    push_note_type: &str,
    push_fields: &[String],
    vocabulary_note_types: &[String],
) -> Result<BTreeMap<String, Vec<String>>, String> {
    let mut map = BTreeMap::new();
    for note_type in vocabulary_note_types {
        let note_type = note_type.trim();
        if note_type.is_empty() || map.contains_key(note_type) {
            continue;
        }
        let fields = if note_type == push_note_type {
            push_fields.to_vec()
        } else {
            model_field_names(note_type)?
        };
        map.insert(note_type.to_string(), fields);
    }
    Ok(map)
}

fn model_field_names(note_type: &str) -> Result<Vec<String>, String> {
    if note_type.is_empty() {
        return Ok(Vec::new());
    }

    Ok(json_string_array(anki_connect_request(
        "modelFieldNames",
        serde_json::json!({ "modelName": note_type }),
    )?))
}
