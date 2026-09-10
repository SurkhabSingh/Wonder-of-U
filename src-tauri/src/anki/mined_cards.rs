use crate::progress::measured::Measured;

use super::{
    client::{anki_connect_health_check, anki_connect_request, json_i64_array},
    tags,
};

/// How many cards in the open collection came from this app.
///
/// Counted by TAG, which is the only stable identity. Deck and note type are settings the
/// user changes, and a count keyed on either reports zero for exactly the people who use
/// the app most — on this machine every tagged note sits on a note type the app did not
/// create.
///
/// Its own command rather than part of the progress report, for two reasons the audit was
/// clear about: the report is read from local files and must open at the same speed whether
/// Anki is running or not, and this is the one number that cannot be answered without it.
///
/// A closed Anki is an ordinary state, not a failure — the same shape `load_anki_catalog`
/// uses. It comes back as a number that says it is not available, never as a zero.
pub(crate) fn count_mined_cards_inner(now_ms: u64) -> Measured<usize> {
    if anki_connect_health_check().is_err() {
        return Measured::unavailable(
            "Anki is not open, so the cards in your collection cannot be counted.",
        );
    }

    // The identifying tag, not one of the kinds under it: this counts everything the app
    // has made, whichever path made it. Every card carries this one alongside its kind.
    let query = format!("tag:{}", tags::MINED);
    let reply = match anki_connect_request("findNotes", serde_json::json!({ "query": query })) {
        Ok(reply) => reply,
        Err(reason) => return Measured::unavailable(reason),
    };

    match json_i64_array(reply, "note id list") {
        // Notes, not cards. One note can make several cards, and saying "cards" would
        // overstate a number the user can check against Anki's own browser in one click.
        Ok(ids) => Measured::known(ids.len(), now_ms),
        Err(reason) => Measured::unavailable(reason),
    }
}
