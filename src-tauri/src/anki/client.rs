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

pub(in crate::anki) fn anki_connect_request(
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

    check_anki_connect_error(&response)?;

    Ok(response
        .get("result")
        .cloned()
        .unwrap_or(serde_json::Value::Null))
}

/// Fails the request when the reply reports an error.
///
/// AnkiConnect answers with `error` as a string or null. Anything else is a reply this
/// app cannot read, and waving it through would let the `result` beside it — null in
/// every error response — be taken for a real answer by whatever reads it next. Kept
/// pure so each shape can be asserted without Anki running.
fn check_anki_connect_error(response: &serde_json::Value) -> Result<(), String> {
    match response.get("error") {
        None | Some(serde_json::Value::Null) => Ok(()),
        Some(serde_json::Value::String(error)) if error.is_empty() => Ok(()),
        Some(serde_json::Value::String(error)) => Err(error.clone()),
        Some(_) => Err(
            "Anki reported something this app could not read. Restart Anki and try again."
                .to_string(),
        ),
    }
}

pub(super) fn anki_note_exists(note_id: i64) -> Result<bool, String> {
    Ok(anki_note_snapshot(note_id, None)?.exists)
}

/// Reads one note out of a `notesInfo` reply.
///
/// Kept pure so the two answers that look alike can be told apart in a test without
/// Anki running, because confusing them costs the user data.
///
/// A reply that is not a LIST of notes is a read that did not happen. Reporting that
/// as `exists: false` would be a failure wearing an absent note's clothes — and the
/// caller prunes push history from `state.json` on exactly that answer, so one
/// unreadable reply would delete the record of every card ever mined from a
/// recording. A real list that simply does not contain the id is the opposite: solid
/// evidence the note was deleted in Anki, which is what this check exists to find.
fn note_snapshot_from_result(
    result: &serde_json::Value,
    note_id: i64,
    field_name: Option<&str>,
) -> Result<AnkiNoteSnapshot, String> {
    let Some(notes) = result.as_array() else {
        return Err("Anki's note list could not be read — its API may have changed.".to_string());
    };

    let Some(note) = notes.iter().find(|note| {
        note.get("noteId")
            .and_then(|value| value.as_i64())
            .is_some_and(|candidate| candidate == note_id)
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
    note_snapshot_from_result(&result, note_id, field_name)
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

/// Every note id matching an Anki search query.
pub(super) fn anki_find_notes(query: &str) -> Result<Vec<i64>, String> {
    Ok(json_i64_array(anki_connect_request(
        "findNotes",
        serde_json::json!({ "query": query }),
    )?))
}

/// Fetches whole notes — fields, tags, and all — for `note_ids`.
///
/// Deliberately takes a slice rather than one id: a note carries every field it
/// has, so the response is heavy per note and the only way to read many notes
/// affordably is to ask for them in one round trip. Callers with a large id list
/// must still chunk it; see `NOTES_INFO_BATCH_SIZE`.
pub(super) fn anki_notes_info(note_ids: &[i64]) -> Result<serde_json::Value, String> {
    anki_connect_request("notesInfo", serde_json::json!({ "notes": note_ids }))
}

fn json_i64_array(value: serde_json::Value) -> Vec<i64> {
    value
        .as_array()
        .map(|items| items.iter().filter_map(serde_json::Value::as_i64).collect())
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::{check_anki_connect_error, note_snapshot_from_result};

    #[test]
    fn an_error_field_this_app_cannot_read_fails_the_request() {
        // Clean replies.
        assert!(check_anki_connect_error(&serde_json::json!({ "result": [] })).is_ok());
        assert!(
            check_anki_connect_error(&serde_json::json!({ "result": [], "error": null })).is_ok()
        );
        assert!(check_anki_connect_error(&serde_json::json!({ "error": "" })).is_ok());

        // A reported error is passed through as Anki wrote it.
        assert_eq!(
            check_anki_connect_error(&serde_json::json!({ "error": "deck not found" })),
            Err("deck not found".to_string())
        );

        // An error shaped in a way this app does not understand must still FAIL. Waving
        // it through leaves `result` — null in every error reply — to be read as an
        // answer, which is how an unreadable response became "the note is gone".
        for unreadable in [
            serde_json::json!({ "result": null, "error": { "code": 1 } }),
            serde_json::json!({ "result": null, "error": 42 }),
            serde_json::json!({ "result": null, "error": ["nope"] }),
        ] {
            assert!(
                check_anki_connect_error(&unreadable).is_err(),
                "an unreadable error must not be waved through: {unreadable}"
            );
        }
    }

    fn notes(ids: &[i64]) -> serde_json::Value {
        serde_json::Value::Array(
            ids.iter()
                .map(|id| serde_json::json!({ "noteId": id, "fields": {} }))
                .collect(),
        )
    }

    #[test]
    fn a_reply_that_is_not_a_list_of_notes_is_an_error_not_an_absent_note() {
        // Each of these is a read that did not happen. Answering `exists: false` would
        // make the caller prune this note's push history from disk, so the whole point
        // is that none of them may resolve to Ok.
        for unreadable in [
            serde_json::Value::Null,
            serde_json::json!({}),
            serde_json::json!({ "notes": [] }),
            serde_json::json!("nope"),
            serde_json::json!(0),
        ] {
            assert!(
                note_snapshot_from_result(&unreadable, 1, None).is_err(),
                "an unreadable reply must not read as an absent note: {unreadable}"
            );
        }
    }

    #[test]
    fn a_real_list_without_the_id_is_a_note_that_was_deleted() {
        // The opposite case, and the one this check exists for: Anki answered properly
        // and the note is not in it. That IS evidence, and pruning is correct.
        let snapshot = note_snapshot_from_result(&notes(&[7, 8]), 1, None)
            .expect("a real list is readable");
        assert!(!snapshot.exists);

        // An empty list is still a list: Anki said it holds none of the ids asked for.
        let snapshot = note_snapshot_from_result(&notes(&[]), 1, None)
            .expect("an empty list is readable");
        assert!(!snapshot.exists);
    }

    #[test]
    fn a_present_note_carries_the_requested_field() {
        let result = serde_json::json!([
            { "noteId": 1, "fields": { "Sentence": { "value": "ある日" } } }
        ]);
        let snapshot = note_snapshot_from_result(&result, 1, Some("Sentence"))
            .expect("readable");
        assert!(snapshot.exists);
        assert_eq!(snapshot.field_value.as_deref(), Some("ある日"));

        // A field that is not on the note leaves the value absent without denying the
        // note itself — those are different facts.
        let snapshot = note_snapshot_from_result(&result, 1, Some("Missing"))
            .expect("readable");
        assert!(snapshot.exists);
        assert_eq!(snapshot.field_value, None);
    }
}
