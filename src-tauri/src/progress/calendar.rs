use serde::Serialize;

use super::day::DayKey;
use super::evidence::{EvidenceFrom, LibraryEvidence};
use super::ledger::Ledger;

const GRID_DAYS: usize = 371;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CalendarDay {
    pub(crate) day: DayKey,
    /// `None` before the store existed. No number means no colour step to pick, so a
    /// measured zero over an uncounted day is impossible rather than guarded against.
    pub(crate) combined_ms: Option<u64>,
    pub(crate) made_card: Option<bool>,
    pub(crate) added_material: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CalendarSpan {
    pub(crate) first_day: Option<DayKey>,
    /// `None` when the store could not be read. Then no day was counted, today included, and
    /// no branch can supply a horizon that would draw one as a measured zero.
    pub(crate) counted_from: Option<DayKey>,
    pub(crate) days: Vec<CalendarDay>,
    pub(crate) dropped_evidence: u32,
}

/// `horizon` is the persisted evidence floor, not the live library's: deleting every old
/// recording empties a day's mark but must not re-label the months before it as uncounted.
pub(crate) fn build(
    ledger: &Ledger,
    evidence: &LibraryEvidence,
    horizon: &EvidenceFrom,
    counted_from: Option<&DayKey>,
    today: &DayKey,
) -> CalendarSpan {
    let window_start = step_back(today, GRID_DAYS - 1);
    let earliest = [horizon.library.as_ref(), counted_from]
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
            let totals = ledger.get(&day);
            days.push(CalendarDay {
                combined_ms: counted.then(|| totals.map_or(0, |totals| totals.combined_ms())),
                made_card: counted.then(|| totals.is_some_and(|totals| totals.mined)),
                added_material: evidence.days.contains(&day),
                day: day.clone(),
            });
            cursor = day.previous();
        }
        days.reverse();
    }

    CalendarSpan {
        first_day,
        counted_from: counted_from.cloned(),
        days,
        dropped_evidence: evidence.dropped,
    }
}

/// For a store that could not be read: only the library can speak, and nothing was counted.
pub(crate) fn uncounted(
    evidence: &LibraryEvidence,
    horizon: &EvidenceFrom,
    today: &DayKey,
) -> CalendarSpan {
    build(&Ledger::default(), evidence, horizon, None, today)
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
    use super::*;
    use super::super::credit::{Credit, ImmersionSource};
    use super::super::evidence::collect;
    use crate::app_types::WatchedVideo;

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
        let span = build(
            &watched(&[("2026-09-14", 600_000)]),
            &evidence_on(&[1_775_000_000_000], &today),
            &evidence_on(&[1_775_000_000_000], &today).horizon(),
            Some(&counted_from),
            &today,
        );
        assert_eq!(span.counted_from, Some(counted_from.clone()), "the horizon is carried");
        let before = counted_from.previous().expect("a real date");
        assert_eq!(day_in(&span, before.as_str()).combined_ms, None);
        assert_eq!(day_in(&span, before.as_str()).made_card, None);
        assert_eq!(day_in(&span, counted_from.as_str()).combined_ms, Some(0));
        assert_eq!(day_in(&span, "2026-09-14").combined_ms, Some(600_000));
    }

    /// The tiles say unavailable; the grid must not say "counted, nothing played" beside them.
    #[test]
    fn a_store_nobody_could_read_counts_no_day_not_even_today() {
        let today = key("2026-09-16");
        let evidence = evidence_on(&[1_775_000_000_000], &today);
        let span = uncounted(&evidence, &evidence.horizon(), &today);
        assert_eq!(span.counted_from, None);
        assert!(!span.days.is_empty(), "the library still speaks");
        assert!(span.days.iter().all(|entry| entry.combined_ms.is_none()));
        assert!(span.days.iter().all(|entry| entry.made_card.is_none()));
        assert_eq!(day_in(&span, "2026-09-16").combined_ms, None);
    }

    #[test]
    fn nothing_to_read_and_nothing_counted_draws_nothing() {
        let today = key("2026-09-16");
        let span = uncounted(&evidence_on(&[], &today), &EvidenceFrom::default(), &today);
        assert_eq!(span.first_day, None);
        assert!(span.days.is_empty());
    }

    #[test]
    fn a_counted_day_with_nothing_on_it_is_a_measured_zero() {
        let today = key("2026-09-15");
        let span = build(
            &watched(&[]),
            &evidence_on(&[], &today),
            &EvidenceFrom::default(),
            Some(&key("2026-09-14")),
            &today,
        );
        assert_eq!(day_in(&span, "2026-09-14").combined_ms, Some(0));
        assert_eq!(day_in(&span, "2026-09-14").made_card, Some(false));
    }

    #[test]
    fn the_span_starts_at_the_evidence_when_it_predates_the_store() {
        let today = key("2026-09-15");
        let evidence = evidence_on(&[1_775_000_000_000], &today);
        let earliest = evidence.earliest().cloned().expect("a date");
        let span = build(
            &watched(&[]),
            &evidence,
            &evidence.horizon(),
            Some(&key("2026-09-13")),
            &today,
        );
        assert_eq!(span.first_day, Some(earliest.clone()));
        assert!(earliest < key("2026-09-13"));
        assert_eq!(day_in(&span, earliest.as_str()).combined_ms, None);
        assert!(day_in(&span, earliest.as_str()).added_material);
    }

    /// The days lose their marks; the months before them must not become months nobody
    /// knew about.
    #[test]
    fn deleting_the_old_recordings_does_not_move_the_span_forward() {
        let today = key("2026-09-15");
        let horizon = evidence_on(&[1_775_000_000_000], &today).horizon();
        let emptied = evidence_on(&[], &today);

        let span = build(&watched(&[]), &emptied, &horizon, Some(&key("2026-09-13")), &today);
        assert_eq!(span.first_day, horizon.library);
        let earliest = horizon.library.clone().expect("a date");
        assert!(!day_in(&span, earliest.as_str()).added_material, "the mark is gone");
        assert_eq!(
            day_in(&span, earliest.as_str()).combined_ms,
            None,
            "still uncounted, not a zero"
        );
    }

    #[test]
    fn the_span_ends_on_today_and_runs_oldest_first() {
        let today = key("2026-09-15");
        let span = build(
            &watched(&[]),
            &evidence_on(&[], &today),
            &EvidenceFrom::default(),
            Some(&key("2026-09-13")),
            &today,
        );
        assert_eq!(span.days.first().map(|entry| &entry.day), span.first_day.as_ref());
        assert_eq!(span.days.last().map(|entry| entry.day.clone()), Some(today));
        assert_eq!(span.days.len(), 3);
    }

    #[test]
    fn the_span_never_exceeds_the_grid() {
        let today = key("2026-09-15");
        let span = build(
            &watched(&[]),
            &evidence_on(&[1_000_000_000_000], &today),
            &evidence_on(&[1_000_000_000_000], &today).horizon(),
            Some(&key("2026-09-13")),
            &today,
        );
        assert_eq!(span.days.len(), GRID_DAYS);
    }

    #[test]
    fn the_span_reaches_the_frontend_under_the_names_it_is_read_by() {
        let today = key("2026-09-15");
        let span = build(
            &watched(&[("2026-09-15", 60_000)]),
            &evidence_on(&[], &today),
            &EvidenceFrom::default(),
            Some(&today),
            &today,
        );
        let wire = serde_json::to_value(&span).expect("serialises");
        let object = wire.as_object().expect("an object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["countedFrom", "days", "droppedEvidence", "firstDay"]);

        let mut day_keys: Vec<&str> = object["days"][0]
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        day_keys.sort_unstable();
        assert_eq!(
            day_keys,
            ["addedMaterial", "combinedMs", "day", "madeCard"]
        );
    }

    #[test]
    fn a_date_nobody_could_read_is_counted_rather_than_drawn() {
        let today = key("2026-09-15");
        let span = build(
            &watched(&[]),
            &evidence_on(&[0, 0], &today),
            &EvidenceFrom::default(),
            Some(&key("2026-09-15")),
            &today,
        );
        assert_eq!(span.dropped_evidence, 2);
        assert!(!day_in(&span, "2026-09-15").added_material);
    }
}
