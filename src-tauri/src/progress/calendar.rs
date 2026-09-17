use serde::Serialize;

use super::cards::{CardCounts, CardHistory};
use super::day::DayKey;
use super::evidence::{EvidenceFrom, LibraryEvidence, MaterialCounts};
use super::ledger::Ledger;

const GRID_DAYS: usize = 371;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CalendarDay {
    pub(crate) day: DayKey,
    /// `None` before the store existed. No number means no colour step to pick, so a
    /// measured zero over an uncounted day is impossible rather than guarded against.
    pub(crate) combined_ms: Option<u64>,
    pub(crate) cards: Option<CardCounts>,
    pub(crate) material: Option<MaterialCounts>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CardsCounted {
    pub(crate) from: Option<DayKey>,
    pub(crate) on: DayKey,
    pub(crate) at_ms: u64,
    pub(crate) undated: u32,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CalendarSpan {
    pub(crate) first_day: Option<DayKey>,
    /// `None` when the store could not be read. Then no day was counted, today included, and
    /// no branch can supply a horizon that would draw one as a measured zero.
    pub(crate) counted_from: Option<DayKey>,
    pub(crate) material_from: Option<DayKey>,
    pub(crate) cards: Option<CardsCounted>,
    pub(crate) days: Vec<CalendarDay>,
    pub(crate) dropped_evidence: u32,
}

pub(crate) struct Sources<'a> {
    pub(crate) library: &'a LibraryEvidence,
    pub(crate) cards: Option<&'a CardHistory>,
}

/// `horizon` is the persisted library floor, not the live library's: deleting every old
/// recording lowers a day's count but must not re-label the months before it as unknown.
pub(crate) fn build(
    ledger: &Ledger,
    sources: &Sources,
    horizon: &EvidenceFrom,
    counted_from: Option<&DayKey>,
    today: &DayKey,
) -> CalendarSpan {
    let window_start = step_back(today, GRID_DAYS - 1);
    let cards_from = sources.cards.and_then(|history| history.from.as_ref());
    let earliest = [horizon.library.as_ref(), counted_from, cards_from]
        .into_iter()
        .flatten()
        .min()
        .cloned();
    let first_day = match (earliest, window_start) {
        (Some(day), Some(window)) => Some(day.max(window)),
        (day, None) => day,
        (None, _) => None,
    };

    let mut days = Vec::new();
    if let Some(first) = first_day.clone() {
        let mut cursor = Some(today.clone());
        while let Some(day) = cursor {
            if day < first {
                break;
            }
            let counted = counted_from.is_some_and(|from| day >= *from);
            let stocked = horizon.library.as_ref().is_some_and(|from| day >= *from);
            days.push(CalendarDay {
                combined_ms: counted
                    .then(|| ledger.get(&day).map_or(0, |totals| totals.combined_ms())),
                cards: sources.cards.and_then(|history| history.on(&day)),
                material: stocked
                    .then(|| sources.library.days.get(&day).copied().unwrap_or_default()),
                day: day.clone(),
            });
            cursor = day.previous();
        }
        days.reverse();
    }

    CalendarSpan {
        first_day,
        counted_from: counted_from.cloned(),
        material_from: horizon.library.clone(),
        cards: sources.cards.map(|history| CardsCounted {
            from: history.from.clone(),
            on: history.read_on.clone(),
            at_ms: history.read_at_ms,
            undated: history.undated,
        }),
        days,
        dropped_evidence: sources.library.dropped,
    }
}

/// For a store that could not be read: the library and Anki can speak, and nothing was counted.
pub(crate) fn uncounted(sources: &Sources, horizon: &EvidenceFrom, today: &DayKey) -> CalendarSpan {
    build(&Ledger::default(), sources, horizon, None, today)
}

fn step_back(from: &DayKey, by: usize) -> Option<DayKey> {
    let mut day = from.clone();
    for _ in 0..by {
        day = day.previous()?;
    }
    Some(day)
}

#[cfg(test)]
mod tests {
    use super::super::cards::{tally, CardKind};
    use super::super::credit::{Credit, ImmersionSource};
    use super::super::evidence::collect;
    use super::*;
    use crate::app_types::WatchedVideo;

    const APRIL_FIRST: u64 = 1_775_000_000_000;
    const NOON_SEPT_TENTH: i64 = 1_789_041_600_000;

    fn key(day: &str) -> DayKey {
        serde_json::from_str(&format!("\"{day}\"")).expect("a day key is a string")
    }

    fn watched(minutes: &[(&str, u64)]) -> Ledger {
        let mut ledger = Ledger::default();
        for (day, ms) in minutes {
            ledger.credit(
                &key(day),
                ImmersionSource::Watching,
                Credit {
                    credited_ms: *ms,
                    unmeasured_ms: 0,
                },
                false,
            );
        }
        ledger
    }

    fn evidence_on(stamps: &[u64], today: &DayKey) -> LibraryEvidence {
        let videos: Vec<WatchedVideo> = stamps
            .iter()
            .map(|added_at_ms| WatchedVideo {
                added_at_ms: *added_at_ms,
                ..WatchedVideo::default()
            })
            .collect();
        collect(&[], &videos, today)
    }

    fn library(evidence: &LibraryEvidence) -> Sources<'_> {
        Sources {
            library: evidence,
            cards: None,
        }
    }

    fn day_in(span: &CalendarSpan, day: &str) -> CalendarDay {
        span.days
            .iter()
            .find(|entry| entry.day == key(day))
            .cloned()
            .unwrap_or_else(|| panic!("{day} is not in the span"))
    }

    #[test]
    fn the_day_before_the_store_carries_no_number_at_all() {
        let today = key("2026-09-15");
        let counted_from = key("2026-09-13");
        let evidence = evidence_on(&[APRIL_FIRST], &today);
        let span = build(
            &watched(&[("2026-09-14", 600_000)]),
            &library(&evidence),
            &evidence.horizon(),
            Some(&counted_from),
            &today,
        );
        assert_eq!(
            span.counted_from,
            Some(counted_from.clone()),
            "the horizon is carried"
        );
        let before = counted_from.previous().expect("a real date");
        assert_eq!(day_in(&span, before.as_str()).combined_ms, None);
        assert_eq!(day_in(&span, counted_from.as_str()).combined_ms, Some(0));
        assert_eq!(day_in(&span, "2026-09-14").combined_ms, Some(600_000));
    }

    /// The tiles say unavailable; the grid must not say "counted, nothing played" beside them.
    #[test]
    fn a_store_nobody_could_read_counts_no_day_not_even_today() {
        let today = key("2026-09-16");
        let evidence = evidence_on(&[APRIL_FIRST], &today);
        let span = uncounted(&library(&evidence), &evidence.horizon(), &today);
        assert_eq!(span.counted_from, None);
        assert!(!span.days.is_empty(), "the library still speaks");
        assert!(span.days.iter().all(|entry| entry.combined_ms.is_none()));
        assert_eq!(day_in(&span, "2026-09-16").combined_ms, None);
        assert!(day_in(&span, "2026-09-16").material.is_some());
    }

    #[test]
    fn nothing_to_read_and_nothing_counted_draws_nothing() {
        let today = key("2026-09-16");
        let evidence = evidence_on(&[], &today);
        let span = uncounted(&library(&evidence), &EvidenceFrom::default(), &today);
        assert_eq!(span.first_day, None);
        assert!(span.days.is_empty());
    }

    #[test]
    fn a_counted_day_with_nothing_on_it_is_a_measured_zero() {
        let today = key("2026-09-15");
        let evidence = evidence_on(&[], &today);
        let span = build(
            &watched(&[]),
            &library(&evidence),
            &EvidenceFrom::default(),
            Some(&key("2026-09-14")),
            &today,
        );
        assert_eq!(day_in(&span, "2026-09-14").combined_ms, Some(0));
        assert_eq!(
            day_in(&span, "2026-09-14").material,
            None,
            "no library to count"
        );
    }

    #[test]
    fn the_span_starts_at_the_evidence_when_it_predates_the_store() {
        let today = key("2026-09-15");
        let evidence = evidence_on(&[APRIL_FIRST, APRIL_FIRST + 1], &today);
        let earliest = evidence.earliest().cloned().expect("a date");
        let span = build(
            &watched(&[]),
            &library(&evidence),
            &evidence.horizon(),
            Some(&key("2026-09-13")),
            &today,
        );
        assert_eq!(span.first_day, Some(earliest.clone()));
        assert_eq!(span.material_from, Some(earliest.clone()));
        assert!(earliest < key("2026-09-13"));
        let first = day_in(&span, earliest.as_str());
        assert_eq!(first.combined_ms, None);
        assert_eq!(first.material.map(|counts| counts.videos), Some(2));
    }

    #[test]
    fn material_is_counted_from_the_library_floor_and_never_before_it() {
        let today = key("2026-09-15");
        let evidence = evidence_on(&[], &today);
        let floor = EvidenceFrom {
            library: Some(key("2026-09-10")),
        };
        let span = build(
            &watched(&[]),
            &library(&evidence),
            &floor,
            Some(&key("2026-09-08")),
            &today,
        );
        assert_eq!(day_in(&span, "2026-09-09").material, None);
        assert_eq!(
            day_in(&span, "2026-09-10").material,
            Some(MaterialCounts::default())
        );
        assert_eq!(
            day_in(&span, "2026-09-15").material,
            Some(MaterialCounts::default())
        );
    }

    /// The days lose their counts; the months before them must not become months nobody
    /// knew about.
    #[test]
    fn deleting_the_old_recordings_does_not_move_the_span_forward() {
        let today = key("2026-09-15");
        let horizon = evidence_on(&[APRIL_FIRST], &today).horizon();
        let emptied = evidence_on(&[], &today);

        let span = build(
            &watched(&[]),
            &library(&emptied),
            &horizon,
            Some(&key("2026-09-13")),
            &today,
        );
        assert_eq!(span.first_day, horizon.library);
        let earliest = horizon.library.clone().expect("a date");
        let first = day_in(&span, earliest.as_str());
        assert_eq!(
            first.material,
            Some(MaterialCounts::default()),
            "counted, now zero"
        );
        assert_eq!(first.combined_ms, None, "still uncounted, not a zero");
    }

    #[test]
    fn a_day_carries_what_anki_counted_and_nothing_outside_its_answer() {
        let today = key("2026-09-15");
        let made = key("2026-09-10");
        let history = tally(
            &[
                (NOON_SEPT_TENTH, CardKind::Word),
                (NOON_SEPT_TENTH + 1, CardKind::Line),
            ],
            1,
            key("2026-09-13"),
            None,
        );
        let evidence = evidence_on(&[], &today);
        let sources = Sources {
            library: &evidence,
            cards: Some(&history),
        };
        let span = build(
            &watched(&[]),
            &sources,
            &EvidenceFrom::default(),
            Some(&today),
            &today,
        );

        assert_eq!(
            span.first_day,
            Some(made.clone()),
            "the first card opens the span"
        );
        assert_eq!(
            day_in(&span, made.as_str())
                .cards
                .map(|counts| counts.total()),
            Some(2)
        );
        assert_eq!(
            day_in(&span, "2026-09-12").cards,
            Some(CardCounts::default())
        );
        assert_eq!(
            day_in(&span, "2026-09-14").cards,
            None,
            "after the last answer"
        );
        let counted = span.cards.expect("Anki has answered");
        assert_eq!(
            (counted.from, counted.on, counted.at_ms),
            (Some(made), key("2026-09-13"), 1)
        );
    }

    #[test]
    fn without_an_answer_from_anki_no_day_has_cards() {
        let today = key("2026-09-15");
        let evidence = evidence_on(&[APRIL_FIRST], &today);
        let span = build(
            &watched(&[]),
            &library(&evidence),
            &evidence.horizon(),
            Some(&key("2026-09-13")),
            &today,
        );
        assert_eq!(span.cards, None);
        assert!(span.days.iter().all(|entry| entry.cards.is_none()));
    }

    #[test]
    fn the_span_ends_on_today_and_runs_oldest_first() {
        let today = key("2026-09-15");
        let evidence = evidence_on(&[], &today);
        let span = build(
            &watched(&[]),
            &library(&evidence),
            &EvidenceFrom::default(),
            Some(&key("2026-09-13")),
            &today,
        );
        assert_eq!(
            span.days.first().map(|entry| &entry.day),
            span.first_day.as_ref()
        );
        assert_eq!(span.days.last().map(|entry| entry.day.clone()), Some(today));
        assert_eq!(span.days.len(), 3);
    }

    #[test]
    fn the_span_never_exceeds_the_grid() {
        let today = key("2026-09-15");
        let evidence = evidence_on(&[1_000_000_000_000], &today);
        let span = build(
            &watched(&[]),
            &library(&evidence),
            &evidence.horizon(),
            Some(&key("2026-09-13")),
            &today,
        );
        assert_eq!(span.days.len(), GRID_DAYS);
    }

    fn sorted_keys(value: &serde_json::Value) -> Vec<String> {
        let mut keys: Vec<String> = value
            .as_object()
            .expect("an object")
            .keys()
            .cloned()
            .collect();
        keys.sort_unstable();
        keys
    }

    #[test]
    fn the_span_reaches_the_frontend_under_the_names_it_is_read_by() {
        let today = key("2026-09-15");
        let evidence = evidence_on(&[], &today);
        let history = tally(&[], 1, today.clone(), None);
        let sources = Sources {
            library: &evidence,
            cards: Some(&history),
        };
        let span = build(
            &watched(&[("2026-09-15", 60_000)]),
            &sources,
            &EvidenceFrom::default(),
            Some(&today),
            &today,
        );
        let wire = serde_json::to_value(&span).expect("serialises");
        assert_eq!(
            sorted_keys(&wire),
            [
                "cards",
                "countedFrom",
                "days",
                "droppedEvidence",
                "firstDay",
                "materialFrom"
            ]
        );
        assert_eq!(
            sorted_keys(&wire["cards"]),
            ["atMs", "from", "on", "undated"]
        );
        assert_eq!(
            sorted_keys(&wire["days"][0]),
            ["cards", "combinedMs", "day", "material"]
        );
    }

    #[test]
    fn a_date_nobody_could_read_is_counted_rather_than_drawn() {
        let today = key("2026-09-15");
        let evidence = evidence_on(&[0, 0], &today);
        let span = build(
            &watched(&[]),
            &library(&evidence),
            &EvidenceFrom::default(),
            Some(&key("2026-09-15")),
            &today,
        );
        assert_eq!(span.dropped_evidence, 2);
        assert_eq!(day_in(&span, "2026-09-15").material, None);
    }
}
