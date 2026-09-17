use std::collections::HashSet;
use std::sync::Mutex;

use tauri::{AppHandle, Emitter, Manager, Runtime};

use crate::app_config::PROGRESS_EVENT;
use crate::progress::cards::{self, CardKind};
use crate::progress::measured::Measured;

use super::{
    client::{anki_connect_health_check, anki_find_notes},
    tags,
};

enum Unanswered {
    NotOpen,
    Failed(String),
}

// Held from the first search to the saved file, so two counts cannot save out of order.
static COUNTING: Mutex<()> = Mutex::new(());

/// How many notes in the open collection came from this app, counted by tag — the only
/// identity a user cannot change. Its own command so the report opens without Anki.
pub(crate) fn count_mined_cards_inner<R: Runtime>(app: &AppHandle<R>) -> Measured<usize> {
    let path = app
        .state::<crate::app_types::AppPathsState>()
        .mined_cards_file
        .clone();
    let _counting = COUNTING.lock();
    let now_ms = crate::app_runtime::now_ms();
    let unanswered = match read_mined_notes() {
        Ok(notes) => {
            match cards::keep(&path, &notes, now_ms, crate::progress::day::today()) {
                Ok(_) => {
                    let _ = app.emit(PROGRESS_EVENT, ());
                }
                Err(reason) => crate::app_runtime::log_event(
                    app,
                    "WARN",
                    "progress.cards_unsaved",
                    serde_json::json!({ "message": reason }),
                ),
            }
            return Measured::known(notes.len(), now_ms);
        }
        Err(unanswered) => unanswered,
    };

    let kept = cards::load(&path).unwrap_or_else(|reason| {
        crate::app_runtime::log_event(
            app,
            "WARN",
            "progress.cards_unreadable",
            serde_json::json!({ "message": reason }),
        );
        None
    });
    from_last_answer(kept, unanswered)
}

fn from_last_answer(kept: Option<cards::CardHistory>, unanswered: Unanswered) -> Measured<usize> {
    match (kept, unanswered) {
        (Some(history), Unanswered::NotOpen) => Measured::stale(
            history.total(),
            history.read_at_ms,
            "Anki is not open, so this is the count from the last time it answered.",
        ),
        (Some(history), Unanswered::Failed(reason)) => Measured::stale(
            history.total(),
            history.read_at_ms,
            format!("This is the count from the last time Anki answered. {reason}"),
        ),
        (None, Unanswered::NotOpen) => Measured::unavailable(
            "Anki is not open, so the cards in your collection cannot be counted.",
        ),
        (None, Unanswered::Failed(reason)) => Measured::unavailable(reason),
    }
}

/// The whole set is read before the kinds, so a note made between the searches is left out
/// until the next count rather than filed as unsorted.
fn read_mined_notes() -> Result<Vec<(i64, CardKind)>, Unanswered> {
    if anki_connect_health_check().is_err() {
        return Err(Unanswered::NotOpen);
    }
    let search = |tag: &str| anki_find_notes(&format!("tag:{tag}")).map_err(Unanswered::Failed);
    let all = search(tags::MINED)?;
    let word = search(tags::MINED_WORD)?;
    let line = search(tags::MINED_LINE)?;
    let transcript = search(tags::MINED_TRANSCRIPT)?;
    Ok(sort_by_kind(all, &word, &line, &transcript))
}

fn sort_by_kind(
    all: Vec<i64>,
    word: &[i64],
    line: &[i64],
    transcript: &[i64],
) -> Vec<(i64, CardKind)> {
    let word: HashSet<i64> = word.iter().copied().collect();
    let line: HashSet<i64> = line.iter().copied().collect();
    let transcript: HashSet<i64> = transcript.iter().copied().collect();
    all.into_iter()
        .map(|note_id| {
            let kind = if word.contains(&note_id) {
                CardKind::Word
            } else if line.contains(&note_id) {
                CardKind::Line
            } else if transcript.contains(&note_id) {
                CardKind::Transcript
            } else {
                CardKind::Unsorted
            };
            (note_id, kind)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn each_note_is_filed_once_under_the_kind_its_tag_names() {
        let sorted = sort_by_kind(vec![1, 2, 3, 4], &[1, 2], &[2], &[3]);
        assert_eq!(
            sorted,
            [
                (1, CardKind::Word),
                (2, CardKind::Word),
                (3, CardKind::Transcript),
                (4, CardKind::Unsorted),
            ]
        );
    }

    fn history_read_at(read_at_ms: u64) -> cards::CardHistory {
        let read_on = serde_json::from_str("\"2026-09-15\"").expect("a day key is a string");
        let notes = [(1_789_041_600_000, CardKind::Word), (0, CardKind::Line)];
        cards::tally(&notes, read_at_ms, read_on, None)
    }

    #[test]
    fn a_closed_anki_serves_the_last_answer_dated_as_such() {
        let wire = serde_json::to_value(from_last_answer(
            Some(history_read_at(42)),
            Unanswered::NotOpen,
        ))
        .expect("serialises");
        assert_eq!(wire["status"], serde_json::json!("stale"));
        assert_eq!(wire["value"], serde_json::json!(2));
        assert_eq!(wire["asOfMs"], serde_json::json!(42));
    }

    #[test]
    fn a_failed_answer_keeps_its_reason_beside_the_last_count() {
        let wire = serde_json::to_value(from_last_answer(
            Some(history_read_at(42)),
            Unanswered::Failed("collection is not open".to_string()),
        ))
        .expect("serialises");
        assert_eq!(wire["status"], serde_json::json!("stale"));
        assert!(wire["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("collection is not open")));
    }

    #[test]
    fn without_a_saved_answer_there_is_no_number() {
        for unanswered in [
            Unanswered::NotOpen,
            Unanswered::Failed("refused".to_string()),
        ] {
            let wire =
                serde_json::to_value(from_last_answer(None, unanswered)).expect("serialises");
            assert_eq!(wire["status"], serde_json::json!("unavailable"));
            assert_eq!(wire["value"], serde_json::Value::Null);
        }
    }

    #[test]
    fn a_kind_search_never_adds_a_note_the_whole_search_did_not_find() {
        let sorted = sort_by_kind(vec![7], &[8], &[7, 9], &[]);
        assert_eq!(sorted, [(7, CardKind::Line)]);
    }
}
