use crate::progress::measured::Measured;

use super::client::{anki_connect_health_check, anki_connect_request, json_i64_array};

/// The tag both mining paths write. Named here so the count and the writers cannot drift.
const MINED_TAG: &str = "wonder-of-u";

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

    let query = format!("tag:{MINED_TAG}");
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

#[cfg(test)]
mod tests {
    use super::MINED_TAG;

    /// The tag is the identity of everything this app has ever made. If it drifts from what
    /// the two mining paths write, the count silently becomes zero for a healthy collection
    /// — which is a wrong answer wearing the clothes of an empty one.
    #[test]
    fn the_counted_tag_is_the_one_both_mining_paths_write() {
        let mine = include_str!("mine.rs");
        let push = include_str!("push.rs");
        let written = format!("\"tags\": [\"{MINED_TAG}\"]");
        assert!(
            mine.contains(&written),
            "anki/mine.rs must write {written}"
        );
        assert!(
            push.contains(&written),
            "anki/push.rs must write {written}"
        );
    }
}
