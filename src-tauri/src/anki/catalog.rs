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
                note_type: selected_note_type,
                fields: None,
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

    let mut decks = json_string_array(
        anki_connect_request("deckNames", serde_json::json!({}))?,
        "deck list",
    )?;
    let mut note_types = json_string_array(
        anki_connect_request("modelNames", serde_json::json!({}))?,
        "note type list",
    )?;
    decks.sort();
    note_types.sort();

    // Only ask for the fields of a note type Anki has just said it HAS.
    let fields = if note_types.iter().any(|name| name == &selected_note_type) {
        Some(json_string_array(
            anki_connect_request(
                "modelFieldNames",
                serde_json::json!({ "modelName": &selected_note_type }),
            )?,
            "field list",
        )?)
    } else {
        None
    };

    Ok(AnkiCatalog {
        status: "ready".into(),
        message: "AnkiConnect is ready.".into(),
        version,
        decks,
        note_types,
        note_type: selected_note_type,
        fields,
    })
}

#[cfg(test)]
mod tests {
    use crate::app_types::AnkiCatalog;

    fn catalog(note_type: &str, fields: Option<Vec<String>>) -> serde_json::Value {
        serde_json::to_value(AnkiCatalog {
            status: "ready".into(),
            message: "AnkiConnect is ready.".into(),
            version: Some(6),
            decks: vec!["wonder of u".into()],
            note_types: vec!["Wonder of U Listening".into(), "Lapis".into()],
            note_type: note_type.into(),
            fields,
        })
        .expect("the catalog must serialize")
    }

    /// The catalog crosses into TypeScript, and `noteType` is what stops a warning
    /// describing a note type other than the one it names. A rename on this side is silent
    /// on the other: the frontend reads `undefined`, every comparison against it fails, and
    /// the warnings just stop appearing — which looks exactly like nothing being wrong.
    #[test]
    fn the_catalog_reaches_the_frontend_under_the_names_it_is_read_by() {
        let ready = catalog("Lapis", Some(vec!["Expression".into(), "Sentence".into()]));
        assert_eq!(ready["noteType"], serde_json::json!("Lapis"));
        assert_eq!(ready["fields"], serde_json::json!(["Expression", "Sentence"]));
        assert_eq!(
            ready["noteTypes"],
            serde_json::json!(["Wonder of U Listening", "Lapis"])
        );
    }

    /// A note type Anki does not have must arrive as null rather than an empty list. The
    /// empty list is what a note type whose fields do not match the mapping looks like, and
    /// the two ask the user for opposite things: re-choose the note type, or fix the fields.
    #[test]
    fn a_note_type_anki_does_not_have_arrives_as_null() {
        assert_eq!(
            catalog("Wonder of U Listening", None)["fields"],
            serde_json::Value::Null
        );
        assert_eq!(
            catalog("Wonder of U Listening", Some(Vec::new()))["fields"],
            serde_json::json!([])
        );
    }
}
