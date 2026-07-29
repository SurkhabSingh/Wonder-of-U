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

    // Only ask for the fields of a note type Anki has just said it HAS.
    //
    // `modelFieldNames` errors on a name it does not know, and that error used to take the
    // whole catalog down with it — so renaming or deleting the configured note type in Anki
    // emptied the deck AND note-type lists on this page, and the only control that could
    // have fixed it was the note-type picker that had just gone blank. The recovery path
    // was the thing that broke.
    //
    // Checked against the list rather than wrapped in a fallback: a lookup that cannot be
    // asked for a name that does not exist has no failure to handle.
    let fields = if note_types.iter().any(|name| name == &selected_note_type) {
        json_string_array(anki_connect_request(
            "modelFieldNames",
            serde_json::json!({ "modelName": selected_note_type }),
        )?)
    } else {
        Vec::new()
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
