use std::collections::HashMap;

use serde::Serialize;

use super::measured::Measured;
use super::store::Sample;

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Comparison {
    pub(crate) delta_points: f64,
    pub(crate) earlier_percent: f64,
    pub(crate) later_percent: f64,
    pub(crate) items_compared: usize,
    pub(crate) items_added: usize,
    pub(crate) items_changed: usize,
    pub(crate) earlier_taken_at_ms: u64,
    pub(crate) later_taken_at_ms: u64,
}

pub(crate) fn compare(earlier: &Sample, later: &Sample) -> Option<Comparison> {
    if !earlier.build.matches(&later.build) {
        return None;
    }

    let before: HashMap<&str, (&str, u32, u32)> = earlier
        .items
        .iter()
        .map(|item| {
            (
                item.key.as_str(),
                (
                    item.fingerprint.as_str(),
                    item.content_tokens,
                    item.known_tokens,
                ),
            )
        })
        .collect();

    let mut earlier_content = 0_u32;
    let mut earlier_known = 0_u32;
    let mut later_content = 0_u32;
    let mut later_known = 0_u32;
    let mut items_compared = 0_usize;
    let mut items_added = 0_usize;
    let mut items_changed = 0_usize;

    for item in &later.items {
        let Some((fingerprint, content, known)) = before.get(item.key.as_str()) else {
            items_added += 1;
            continue;
        };
        if *fingerprint != item.fingerprint {
            items_changed += 1;
            continue;
        }
        items_compared += 1;
        earlier_content += content;
        earlier_known += known;
        later_content += item.content_tokens;
        later_known += item.known_tokens;
    }

    if items_compared == 0 || earlier_content == 0 || later_content == 0 {
        return None;
    }

    let earlier_percent = percent(earlier_known, earlier_content);
    let later_percent = percent(later_known, later_content);
    Some(Comparison {
        // Both operands are at one decimal; their difference is not. 50.2 - 50.0 is
        // 0.20000000000000284, and that is what would reach the screen.
        delta_points: round_tenth(later_percent - earlier_percent),
        earlier_percent,
        later_percent,
        items_compared,
        items_added,
        items_changed,
        earlier_taken_at_ms: earlier.taken_at_ms,
        later_taken_at_ms: later.taken_at_ms,
    })
}

pub(crate) fn percent(known: u32, content: u32) -> f64 {
    if content == 0 {
        return 0.0;
    }
    let rounded = round_tenth(f64::from(known) * 100.0 / f64::from(content));
    if known < content && rounded >= 100.0 {
        99.9
    } else {
        rounded
    }
}

fn round_tenth(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

pub(crate) fn latest_comparison(samples: &[Sample]) -> Option<Comparison> {
    let latest = samples.last()?;
    samples
        .iter()
        .rev()
        .skip(1)
        .find_map(|earlier| compare(earlier, latest))
}

/// Everything the Progress page is told, from local files only: a closed Anki cannot turn
/// a measurement that was taken into a number that is missing.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProgressReport {
    pub(crate) coverage_percent: Measured<f64>,
    pub(crate) immersion: Measured<super::streak::ImmersionReport>,
    pub(crate) calendar: super::calendar::CalendarSpan,
    pub(crate) today: super::day::DayKey,
    pub(crate) library: super::library::LibraryReport,
    pub(crate) comparison: Option<Comparison>,
    pub(crate) readings: usize,
    pub(crate) first_run_day: Option<super::day::DayKey>,
    pub(crate) damaged_rows: usize,
    pub(crate) newer_rows: usize,
    pub(crate) store_readable: bool,
}

pub(crate) fn load_progress_inner<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<ProgressReport, String> {
    use tauri::Manager;

    let current_build = {
        let persisted_state = app.state::<crate::app_types::SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the app settings.".to_string())?;
        crate::app_types::KnownWordsBuild::from_anki_settings(&persisted.settings.anki)
    };
    let path = app
        .state::<crate::app_types::AppPathsState>()
        .progress_file
        .clone();

    let today = super::day::today();
    let (library, evidence) = {
        let persisted_state = app.state::<crate::app_types::SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the recording history.".to_string())?;
        let library = super::library::summarise(&persisted.recent_recordings);
        let evidence = super::evidence::collect(
            &persisted.recent_recordings,
            &persisted.watched_videos,
            &today,
        );
        (library, evidence)
    };

    // Persisted before the store is read, so the calendar draws the floor the header keeps
    // rather than whatever the library happens to hold today.
    if let Err(reason) = super::store::remember_evidence(&path, &evidence.horizon()) {
        crate::app_runtime::log_event(
            app,
            "WARN",
            "progress.horizon_unavailable",
            serde_json::json!({ "message": reason }),
        );
    }

    match super::store::load(&path) {
        super::store::Loaded::Present { header, store } => Ok(ProgressReport {
            coverage_percent: coverage_from(&store.samples, &current_build),
            immersion: Measured::known(
                super::streak::summarise(&store.days, &today, &header.first_run_day),
                crate::app_runtime::now_ms(),
            ),
            calendar: super::calendar::build(
                &store.days,
                &evidence,
                &header.evidence_from,
                Some(&header.first_run_day),
                &today,
            ),
            comparison: latest_comparison(&store.samples),
            today: today.clone(),
            library,
            readings: store.samples.len(),
            first_run_day: Some(header.first_run_day),
            damaged_rows: store.damaged,
            newer_rows: store.newer,
            store_readable: true,
        }),
        super::store::Loaded::Missing => Ok(ProgressReport {
            coverage_percent: Measured::unavailable(
                "Nothing has been measured yet. Refresh your word list to take a first reading.",
            ),
            immersion: Measured::unavailable(
                "Time has not been counted yet. It starts adding up as you listen and watch here.",
            ),
            calendar: super::calendar::uncounted(&evidence, &evidence.horizon(), &today),
            comparison: None,
            today: today.clone(),
            library,
            readings: 0,
            first_run_day: None,
            damaged_rows: 0,
            newer_rows: 0,
            store_readable: true,
        }),
        super::store::Loaded::Unreadable(_) => Ok(ProgressReport {
            coverage_percent: Measured::unavailable(
                "Your progress history could not be opened, so there is nothing to show. It is not lost — nothing has been written over it.",
            ),
            immersion: Measured::unavailable(
                "Your progress history could not be opened, so the time you have put in cannot be shown. It is not lost — nothing has been written over it.",
            ),
            calendar: super::calendar::uncounted(&evidence, &evidence.horizon(), &today),
            comparison: None,
            today: today.clone(),
            library,
            readings: 0,
            first_run_day: None,
            damaged_rows: 0,
            newer_rows: 0,
            store_readable: false,
        }),
    }
}

pub(crate) fn coverage_from(
    samples: &[Sample],
    current_build: &crate::app_types::KnownWordsBuild,
) -> Measured<f64> {
    let Some(latest) = samples.last() else {
        return Measured::unavailable(
            "Nothing has been measured yet. Refresh your word list to take a first reading.",
        );
    };
    let (content, known) = latest.totals();
    if content == 0 {
        return Measured::unavailable(
            "The last reading found no Japanese text to measure.",
        );
    }
    let value = percent(known, content);

    if !latest.build.matches(current_build) {
        return Measured::stale(
            value,
            latest.taken_at_ms,
            "Your vocabulary settings changed after this reading. Refresh your word list for a current one.",
        );
    }
    if latest.unread_items > 0 {
        return Measured::partial(
            value,
            latest.taken_at_ms,
            format!(
                "{} transcript{} could not be read, so this covers less than your whole library.",
                latest.unread_items,
                if latest.unread_items == 1 { "" } else { "s" }
            ),
        );
    }
    Measured::known(value, latest.taken_at_ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_types::{KnownWordsBuild, VocabularySource};
    use crate::progress::store::{Sample, SampleItem};

    fn build(note_type: &str) -> KnownWordsBuild {
        KnownWordsBuild {
            sources: vec![VocabularySource {
                note_type: note_type.to_string(),
                field: "Word".to_string(),
            }],
            mature_after_days: 21,
        }
    }

    fn day() -> crate::progress::day::DayKey {
        serde_json::from_str("\"2026-09-10\"").expect("a day key is a string")
    }

    fn item(key: &str, fingerprint: &str, content: u32, known: u32) -> SampleItem {
        SampleItem {
            key: key.to_string(),
            fingerprint: fingerprint.to_string(),
            content_tokens: content,
            known_tokens: known,
        }
    }

    fn sample(taken_at_ms: u64, note_type: &str, items: Vec<SampleItem>) -> Sample {
        Sample::new(taken_at_ms, day(), build(note_type), 1, 0, items, 1_000)
    }

    /// The names ProgressPage reads straight off the report. A rename here blanks the page
    /// beside a healthy status, so the list is spelled out rather than trusted.
    #[test]
    fn the_report_reaches_the_frontend_under_the_names_it_is_read_by() {
        let ledger = crate::progress::ledger::Ledger::default();
        let evidence = crate::progress::evidence::LibraryEvidence::default();
        let report = ProgressReport {
            coverage_percent: Measured::known(58.4, 1),
            immersion: Measured::known(
                crate::progress::streak::summarise(&ledger, &day(), &day()),
                1,
            ),
            calendar: crate::progress::calendar::uncounted(&evidence, &Default::default(), &day()),
            today: day(),
            library: crate::progress::library::summarise(&[]),
            comparison: None,
            readings: 0,
            first_run_day: Some(day()),
            damaged_rows: 0,
            newer_rows: 0,
            store_readable: true,
        };
        let wire = serde_json::to_value(&report).expect("serialises");
        let mut keys: Vec<&str> = wire
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            [
                "calendar",
                "comparison",
                "coveragePercent",
                "damagedRows",
                "firstRunDay",
                "immersion",
                "library",
                "newerRows",
                "readings",
                "storeReadable",
                "today",
            ]
        );
    }

    #[test]
    fn a_fixed_corpus_shows_the_word_list_growing() {
        let earlier = sample(1, "Kaishi", vec![item("a", "f1", 100, 50)]);
        let later = sample(2, "Kaishi", vec![item("a", "f1", 100, 60)]);
        let comparison = compare(&earlier, &later).expect("comparable");
        assert_eq!(comparison.earlier_percent, 50.0);
        assert_eq!(comparison.later_percent, 60.0);
        assert_eq!(comparison.delta_points, 10.0);
        assert_eq!(comparison.items_compared, 1);
    }

    #[test]
    fn material_added_since_the_earlier_sample_is_held_out_of_the_change() {
        let earlier = sample(1, "Kaishi", vec![item("a", "f1", 100, 50)]);
        let later = sample(
            2,
            "Kaishi",
            vec![item("a", "f1", 100, 50), item("b", "f9", 100, 100)],
        );
        let comparison = compare(&earlier, &later).expect("comparable");
        assert_eq!(comparison.delta_points, 0.0, "the new easy item must not count");
        assert_eq!(comparison.items_compared, 1);
        assert_eq!(comparison.items_added, 1);
    }

    #[test]
    fn a_document_whose_text_changed_is_not_compared_against_its_own_past() {
        let earlier = sample(1, "Kaishi", vec![item("a", "old", 100, 50)]);
        let later = sample(2, "Kaishi", vec![item("a", "new", 100, 90)]);
        assert_eq!(compare(&earlier, &later), None, "nothing left to compare");

        let mixed_earlier = sample(
            1,
            "Kaishi",
            vec![item("a", "old", 100, 50), item("b", "same", 100, 50)],
        );
        let mixed_later = sample(
            2,
            "Kaishi",
            vec![item("a", "new", 100, 99), item("b", "same", 100, 60)],
        );
        let comparison = compare(&mixed_earlier, &mixed_later).expect("comparable");
        assert_eq!(comparison.items_changed, 1);
        assert_eq!(comparison.items_compared, 1);
        assert_eq!(comparison.delta_points, 10.0, "only the unchanged item counts");
    }

    #[test]
    fn samples_from_different_vocabulary_settings_are_not_comparable() {
        let earlier = sample(1, "Kaishi", vec![item("a", "f1", 100, 50)]);
        let later = sample(2, "Lapis", vec![item("a", "f1", 100, 60)]);
        assert_eq!(compare(&earlier, &later), None);
    }

    #[test]
    fn the_change_is_pooled_across_documents() {
        let earlier = sample(
            1,
            "Kaishi",
            vec![item("short", "f1", 2, 0), item("long", "f2", 998, 500)],
        );
        let later = sample(
            2,
            "Kaishi",
            vec![item("short", "f1", 2, 2), item("long", "f2", 998, 500)],
        );
        let comparison = compare(&earlier, &later).expect("comparable");
        // Averaging the two documents would call this a 50-point jump. Pooled it is 0.2.
        assert_eq!(comparison.delta_points, 0.2);
    }

    #[test]
    fn the_comparison_reaches_past_an_incomparable_neighbour() {
        let samples = vec![
            sample(1, "Kaishi", vec![item("a", "f1", 100, 50)]),
            sample(2, "Lapis", vec![item("a", "f1", 100, 10)]),
            sample(3, "Kaishi", vec![item("a", "f1", 100, 70)]),
        ];
        let comparison = latest_comparison(&samples).expect("comparable with the first");
        assert_eq!(comparison.earlier_taken_at_ms, 1);
        assert_eq!(comparison.delta_points, 20.0);
    }

    #[test]
    fn a_share_only_reads_as_whole_when_nothing_is_left() {
        assert_eq!(percent(9_999, 10_000), 99.9);
        assert_eq!(percent(10_000, 10_000), 100.0);
        assert_eq!(percent(1, 3), 33.3);
        assert_eq!(percent(0, 10), 0.0);
        assert_eq!(percent(0, 0), 0.0);
    }

    #[test]
    fn coverage_says_when_there_is_nothing_measured_rather_than_zero() {
        let value = serde_json::to_value(coverage_from(&[], &build("Kaishi")))
            .expect("serialize");
        assert_eq!(value["value"], serde_json::Value::Null);
        assert_eq!(value["status"], serde_json::json!("unavailable"));
    }

    #[test]
    fn coverage_measured_under_other_settings_is_shown_and_dated_as_stale() {
        let samples = vec![sample(500, "Kaishi", vec![item("a", "f1", 100, 42)])];
        let value = serde_json::to_value(coverage_from(&samples, &build("Lapis")))
            .expect("serialize");
        assert_eq!(value["value"], serde_json::json!(42.0));
        assert_eq!(value["status"], serde_json::json!("stale"));
        assert_eq!(value["asOfMs"], serde_json::json!(500));
    }

    #[test]
    fn coverage_that_missed_a_document_says_so_and_keeps_its_value() {
        let mut samples = vec![sample(500, "Kaishi", vec![item("a", "f1", 100, 42)])];
        samples[0].unread_items = 2;
        let value = serde_json::to_value(coverage_from(&samples, &build("Kaishi")))
            .expect("serialize");
        assert_eq!(value["value"], serde_json::json!(42.0));
        assert_eq!(value["status"], serde_json::json!("partial"));
        assert!(
            value["reason"].as_str().unwrap_or_default().contains("2 transcripts"),
            "{value}"
        );
    }
}
