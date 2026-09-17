use std::collections::BTreeMap;
use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::app_state::write_file_atomically;

use super::day::DayKey;
use super::evidence::usable_day;

const HISTORY_VERSION: u32 = 1;

// Replacements share one temp path, so a second writer would truncate the first.
static WRITE: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CardKind {
    Word,
    Line,
    Transcript,
    Unsorted,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CardCounts {
    pub(crate) word: u32,
    pub(crate) line: u32,
    pub(crate) transcript: u32,
    pub(crate) unsorted: u32,
}

impl CardCounts {
    fn add(&mut self, kind: CardKind) {
        let slot = match kind {
            CardKind::Word => &mut self.word,
            CardKind::Line => &mut self.line,
            CardKind::Transcript => &mut self.transcript,
            CardKind::Unsorted => &mut self.unsorted,
        };
        *slot = slot.saturating_add(1);
    }

    pub(crate) fn total(&self) -> u32 {
        self.word
            .saturating_add(self.line)
            .saturating_add(self.transcript)
            .saturating_add(self.unsorted)
    }
}

/// Anki's last answer, by day. Replaced whole on every answer, so a deleted note leaves
/// nothing behind; only `from` carries over, and only ever earlier.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CardHistory {
    pub(crate) v: u32,
    pub(crate) read_at_ms: u64,
    pub(crate) read_on: DayKey,
    pub(crate) from: Option<DayKey>,
    pub(crate) days: BTreeMap<DayKey, CardCounts>,
    pub(crate) undated: u32,
}

impl CardHistory {
    pub(crate) fn total(&self) -> usize {
        let dated: u64 = self
            .days
            .values()
            .map(|counts| u64::from(counts.total()))
            .sum();
        usize::try_from(dated + u64::from(self.undated)).unwrap_or(usize::MAX)
    }

    pub(crate) fn on(&self, day: &DayKey) -> Option<CardCounts> {
        let from = self.from.as_ref()?;
        if day < from || *day > self.read_on {
            return None;
        }
        Some(self.days.get(day).copied().unwrap_or_default())
    }
}

pub(crate) fn tally(
    notes: &[(i64, CardKind)],
    read_at_ms: u64,
    read_on: DayKey,
    kept_from: Option<DayKey>,
) -> CardHistory {
    let mut days: BTreeMap<DayKey, CardCounts> = BTreeMap::new();
    let mut undated = 0_u32;
    for (note_id, kind) in notes {
        // A note id is the millisecond the note was made.
        let made_on = u64::try_from(*note_id)
            .ok()
            .and_then(|stamp| usable_day(stamp, &read_on));
        match made_on {
            Some(day) => days.entry(day).or_default().add(*kind),
            None => undated = undated.saturating_add(1),
        }
    }
    let from = [kept_from, days.keys().next().cloned()]
        .into_iter()
        .flatten()
        .min();
    CardHistory {
        v: HISTORY_VERSION,
        read_at_ms,
        read_on,
        from,
        days,
        undated,
    }
}

/// `Ok(None)` when Anki has never answered, or answered into a file this build cannot read.
pub(crate) fn load(path: &Path) -> Result<Option<CardHistory>, String> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let history: CardHistory =
        serde_json::from_str(&contents).map_err(|error| error.to_string())?;
    Ok((history.v == HISTORY_VERSION).then_some(history))
}

pub(crate) fn keep(
    path: &Path,
    notes: &[(i64, CardKind)],
    read_at_ms: u64,
    read_on: DayKey,
) -> Result<CardHistory, String> {
    let _guard = WRITE.lock();
    let kept_from = load(path).ok().flatten().and_then(|previous| previous.from);
    let history = tally(notes, read_at_ms, read_on, kept_from);
    let contents = serde_json::to_string(&history).map_err(|error| error.to_string())?;
    write_file_atomically(path, &contents)?;
    Ok(history)
}

#[cfg(test)]
mod tests {
    use super::super::day::day_key_for_ms;
    use super::*;

    const SEPT_TENTH: i64 = 1_789_041_600_000;
    const HOUR: i64 = 3_600_000;
    const DAY: i64 = 24 * HOUR;

    fn key(day: &str) -> DayKey {
        serde_json::from_str(&format!("\"{day}\"")).expect("a day key is a string")
    }

    fn day_of(note_id: i64) -> DayKey {
        day_key_for_ms(u64::try_from(note_id).expect("positive")).expect("a real date")
    }

    fn read_on() -> DayKey {
        day_of(SEPT_TENTH + 5 * DAY)
    }

    #[test]
    fn a_note_is_counted_on_the_day_it_was_made_under_its_kind() {
        let notes = [
            (SEPT_TENTH, CardKind::Word),
            (SEPT_TENTH + 1, CardKind::Word),
            (SEPT_TENTH + 2, CardKind::Transcript),
            (SEPT_TENTH + 2 * DAY, CardKind::Unsorted),
        ];
        let history = tally(&notes, 1, read_on(), None);
        let first = history.on(&day_of(SEPT_TENTH)).expect("a counted day");
        assert_eq!((first.word, first.line, first.transcript), (2, 0, 1));
        assert_eq!(first.total(), 3);
        let later = history
            .on(&day_of(SEPT_TENTH + 2 * DAY))
            .expect("a counted day");
        assert_eq!(later.unsorted, 1);
    }

    #[test]
    fn every_note_is_counted_once_even_without_a_usable_date() {
        let notes = [
            (SEPT_TENTH, CardKind::Line),
            (0, CardKind::Line),
            (-5, CardKind::Word),
            (SEPT_TENTH + 400 * DAY, CardKind::Word),
        ];
        let history = tally(&notes, 1, read_on(), None);
        assert_eq!(history.undated, 3);
        assert_eq!(history.total(), notes.len());
    }

    #[test]
    fn a_day_anki_did_not_speak_for_has_no_number() {
        let history = tally(&[(SEPT_TENTH + DAY, CardKind::Word)], 1, read_on(), None);
        let first = day_of(SEPT_TENTH + DAY);
        assert_eq!(history.on(&first.previous().expect("a date")), None);
        assert_eq!(
            history.on(&day_of(SEPT_TENTH + 6 * DAY)),
            None,
            "after the answer"
        );
        assert_eq!(
            history.on(&day_of(SEPT_TENTH + 3 * DAY)),
            Some(CardCounts::default()),
            "a day in between is a counted zero"
        );
        assert_eq!(history.on(&read_on()), Some(CardCounts::default()));
    }

    #[test]
    fn a_collection_without_cards_speaks_for_no_day() {
        let history = tally(&[], 1, read_on(), None);
        assert_eq!(history.from, None);
        assert_eq!(history.on(&read_on()), None);
        assert_eq!(history.total(), 0);
    }

    #[test]
    fn the_start_only_ever_moves_earlier() {
        let notes = [(SEPT_TENTH + 2 * DAY, CardKind::Word)];
        let earlier = key("2026-04-06");
        let kept = tally(&notes, 1, read_on(), Some(earlier.clone()));
        assert_eq!(kept.from, Some(earlier.clone()));
        assert_eq!(kept.on(&earlier), Some(CardCounts::default()));

        let later = day_of(SEPT_TENTH + 4 * DAY);
        let moved = tally(&notes, 1, read_on(), Some(later));
        assert_eq!(moved.from, Some(day_of(SEPT_TENTH + 2 * DAY)));

        let emptied = tally(&[], 1, read_on(), Some(earlier.clone()));
        assert_eq!(emptied.from, Some(earlier));
    }

    #[test]
    fn a_new_answer_replaces_every_day_but_keeps_the_start() {
        let dir = tempfile::tempdir().expect("a temp directory");
        let path = dir.path().join("mined_cards.json");
        keep(&path, &[(SEPT_TENTH, CardKind::Word)], 1, read_on()).expect("first answer");
        let answer = [(SEPT_TENTH + 3 * DAY, CardKind::Line)];
        keep(&path, &answer, 2, read_on()).expect("second answer");

        let history = load(&path).expect("readable").expect("present");
        assert_eq!(history.read_at_ms, 2);
        assert_eq!(
            history.days.len(),
            1,
            "the deleted note left nothing behind"
        );
        assert_eq!(history.from, Some(day_of(SEPT_TENTH)));
        assert_eq!(history.on(&day_of(SEPT_TENTH)), Some(CardCounts::default()));
    }

    #[test]
    fn only_a_missing_file_means_anki_never_answered() {
        let dir = tempfile::tempdir().expect("a temp directory");
        let path = dir.path().join("mined_cards.json");
        assert_eq!(load(&path), Ok(None));
        fs::write(&path, "not json").expect("write");
        assert!(load(&path).is_err());
        let newer = serde_json::json!({
            "v": 2, "readAtMs": 1, "readOn": "2026-09-15", "from": null, "days": {}, "undated": 0
        });
        fs::write(&path, newer.to_string()).expect("write");
        assert_eq!(load(&path), Ok(None));
    }

    #[test]
    fn the_counts_reach_the_page_under_the_names_it_reads() {
        let wire = serde_json::to_value(CardCounts::default()).expect("serialises");
        let mut keys: Vec<&str> = wire
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["line", "transcript", "unsorted", "word"]);
    }
}
