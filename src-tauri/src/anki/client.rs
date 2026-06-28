use std::time::Duration;

const ANKI_CONNECT_TIMEOUT: Duration = Duration::from_millis(250);
const ANKI_HEALTH_CHECK_TIMEOUT: Duration = Duration::from_millis(750);
const ANKI_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

pub(super) struct AnkiNoteSnapshot {
    pub(super) exists: bool,
    pub(super) field_value: Option<String>,
}

pub(super) fn anki_offline_message(error: &str) -> String {
    format!(
        "Anki is currently offline. Start Anki and make sure AnkiConnect is installed, then try again. {error}"
    )
}

pub(super) fn anki_connect_request(
    action: &str,
    params: serde_json::Value,
) -> Result<serde_json::Value, String> {
    anki_connect_request_with_timeout(action, params, ANKI_REQUEST_TIMEOUT)
}

pub(super) fn anki_connect_health_check() -> Result<serde_json::Value, String> {
    anki_connect_request_with_timeout("version", serde_json::json!({}), ANKI_HEALTH_CHECK_TIMEOUT)
}

fn anki_connect_request_with_timeout(
    action: &str,
    params: serde_json::Value,
    request_timeout: Duration,
) -> Result<serde_json::Value, String> {
    let client = reqwest::blocking::Client::builder()
        .connect_timeout(ANKI_CONNECT_TIMEOUT)
        .timeout(request_timeout)
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

pub(super) fn anki_note_exists(note_id: i64) -> Result<bool, String> {
    Ok(anki_note_snapshot(note_id, None)?.exists)
}

pub(super) fn anki_note_snapshot(
    note_id: i64,
    field_name: Option<&str>,
) -> Result<AnkiNoteSnapshot, String> {
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
        return Ok(AnkiNoteSnapshot {
            exists: false,
            field_value: None,
        });
    };

    let field_value = field_name
        .and_then(|field_name| note.get("fields")?.as_object()?.get(field_name))
        .and_then(|field| field.get("value"))
        .and_then(|value| value.as_str())
        .map(ToString::to_string);

    Ok(AnkiNoteSnapshot {
        exists: true,
        field_value,
    })
}

pub(super) fn anki_note_field_value(
    note_id: i64,
    field_name: &str,
) -> Result<Option<String>, String> {
    Ok(anki_note_snapshot(note_id, Some(field_name))?.field_value)
}

pub(super) fn json_string_array(value: serde_json::Value) -> Vec<String> {
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
