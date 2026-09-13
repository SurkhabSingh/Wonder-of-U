use crate::progress::measured::Measured;

use super::{
    client::{anki_connect_health_check, anki_connect_request, json_i64_array},
    tags,
};

/// How many notes in the open collection came from this app, counted by tag — the only
/// identity a user cannot change. Its own command so the report opens without Anki.
pub(crate) fn count_mined_cards_inner(now_ms: u64) -> Measured<usize> {
    if anki_connect_health_check().is_err() {
        return Measured::unavailable(
            "Anki is not open, so the cards in your collection cannot be counted.",
        );
    }

    let query = format!("tag:{}", tags::MINED);
    let reply = match anki_connect_request("findNotes", serde_json::json!({ "query": query })) {
        Ok(reply) => reply,
        Err(reason) => return Measured::unavailable(reason),
    };

    match json_i64_array(reply, "note id list") {
        Ok(ids) => Measured::known(ids.len(), now_ms),
        Err(reason) => Measured::unavailable(reason),
    }
}
