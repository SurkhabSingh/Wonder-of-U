use std::collections::HashMap;

use serde::Serialize;

use super::measured::Measured;
use super::store::{Sample, MIN_CONTENT_TOKENS};

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
        .items_with_words()
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

    for item in later.items_with_words() {
        let Some((fingerprint, content, known)) = before.get(item.key.as_str()) else {
            items_added += 1;
            continue;
        };
        // The same text counted differently means the counting changed, not what is known.
        if *fingerprint != item.fingerprint || *content != item.content_tokens {
            items_changed += 1;
            continue;
        }
        items_compared += 1;
        earlier_content += content;
        earlier_known += known;
        later_content += item.content_tokens;
        later_known += item.known_tokens;
    }

    if earlier_content < MIN_CONTENT_TOKENS || later_content < MIN_CONTENT_TOKENS {
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

pub(super) fn round_tenth(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// The newest word list against the latest earlier one it can be compared with, one list's
/// last reading to the next list's first, as the reading line pairs them. Dated from that
/// list's first reading, where the line draws it.
pub(crate) fn latest_comparison(samples: &[Sample]) -> Option<Comparison> {
    let versions = super::level::versions(samples);
    let (latest, earlier) = versions.split_last()?;
    earlier.iter().rev().find_map(|version| {
        compare(version.last, latest.first).map(|comparison| Comparison {
            earlier_taken_at_ms: version.first.taken_at_ms,
            ..comparison
        })
    })
}

/// What a reading is measured against: the settings as they stand, and the list itself.
pub(crate) struct Vocabulary {
    pub(crate) settings: crate::app_types::KnownWordsBuild,
    pub(crate) list: Option<super::level::WordList>,
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
    pub(crate) levels: super::level::Levels,
    pub(crate) readings: usize,
    pub(crate) first_run_day: Option<super::day::DayKey>,
    pub(crate) damaged_rows: usize,
    pub(crate) newer_rows: usize,
    pub(crate) store_readable: bool,
    /// Set while writes to the store are failing, so the page can say what is not being kept.
    pub(crate) write_failure: Option<super::health::WriteFailure>,
}

pub(crate) fn load_progress_inner<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
) -> Result<ProgressReport, String> {
    use tauri::Manager;

    let settings = {
        let persisted_state = app.state::<crate::app_types::SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the app settings.".to_string())?;
        crate::app_types::KnownWordsBuild::from_anki_settings(&persisted.settings.anki)
    };
    let list = app
        .state::<crate::app_types::KnownWordsState>()
        .0
        .lock()
        .ok()
        .and_then(|guard| {
            guard.as_ref().map(|index| super::level::WordList {
                built_at_ms: index.built_at_ms,
                build: index.build.clone(),
                words: u32::try_from(index.words.len()).unwrap_or(u32::MAX),
            })
        });
    let vocabulary = Vocabulary { settings, list };
    let (path, cards_file) = {
        let paths = app.state::<crate::app_types::AppPathsState>();
        (paths.progress_file.clone(), paths.mined_cards_file.clone())
    };

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

    match super::store::remember_evidence(&path, &evidence.horizon()) {
        Ok(true) => super::health::record(app, &Ok(())),
        Ok(false) => {}
        Err(reason) => {
            crate::app_runtime::log_event(
                app,
                "WARN",
                "progress.horizon_unavailable",
                serde_json::json!({ "message": reason }),
            );
            super::health::record(app, &Err(reason));
        }
    }
    let cards = super::cards::load(&cards_file).unwrap_or_else(|reason| {
        crate::app_runtime::log_event(
            app,
            "WARN",
            "progress.cards_unreadable",
            serde_json::json!({ "message": reason }),
        );
        None
    });
    let sources = super::calendar::Sources {
        library: &evidence,
        cards: cards.as_ref(),
    };
    Ok(assemble(
        super::store::load(&path),
        &sources,
        library,
        &vocabulary,
        &today,
        super::health::current(),
        crate::app_runtime::now_ms(),
    ))
}

/// Every choice about what the page is told, with nothing to read or lock, so each state of
/// the store can be checked without the app running.
fn assemble(
    loaded: super::store::Loaded,
    sources: &super::calendar::Sources,
    library: super::library::LibraryReport,
    vocabulary: &Vocabulary,
    today: &super::day::DayKey,
    write_failure: Option<super::health::WriteFailure>,
    now_ms: u64,
) -> ProgressReport {
    let list = vocabulary.list.as_ref();
    match loaded {
        super::store::Loaded::Present { header, store } => ProgressReport {
            coverage_percent: coverage_from(&store.samples, &vocabulary.settings),
            immersion: immersion_from(
                super::streak::summarise(&store.days, today, &header.first_run_day),
                write_failure.is_some(),
                now_ms,
            ),
            calendar: super::calendar::build(
                &store.days,
                sources,
                &floor_from(&header.evidence_from, sources.library),
                Some(&header.first_run_day),
                today,
            ),
            comparison: latest_comparison(&store.samples),
            levels: super::level::levels(&store.samples, list),
            today: today.clone(),
            library,
            readings: super::level::distinct_readings(&store.samples),
            first_run_day: Some(header.first_run_day),
            damaged_rows: store.damaged,
            newer_rows: store.newer,
            store_readable: true,
            write_failure,
        },
        super::store::Loaded::Missing => ProgressReport {
            coverage_percent: Measured::unavailable(
                "Nothing has been measured yet. Refresh your word list to take a first reading.",
            ),
            immersion: Measured::unavailable(
                "Time has not been counted yet. It starts adding up as you listen and watch here.",
            ),
            calendar: super::calendar::uncounted(sources, &sources.library.horizon(), today),
            comparison: None,
            levels: super::level::levels(&[], list),
            today: today.clone(),
            library,
            readings: 0,
            first_run_day: None,
            damaged_rows: 0,
            newer_rows: 0,
            store_readable: true,
            write_failure,
        },
        super::store::Loaded::Unreadable(_) => ProgressReport {
            coverage_percent: Measured::unavailable(
                "Your progress history could not be opened, so there is nothing to show. It is not lost — nothing has been written over it.",
            ),
            immersion: Measured::unavailable(
                "Your progress history could not be opened, so the time you have put in cannot be shown. It is not lost — nothing has been written over it.",
            ),
            calendar: super::calendar::uncounted(sources, &sources.library.horizon(), today),
            comparison: None,
            levels: super::level::levels(&[], list),
            today: today.clone(),
            library,
            readings: 0,
            first_run_day: None,
            damaged_rows: 0,
            newer_rows: 0,
            store_readable: false,
            write_failure,
        },
    }
}

/// What the store holds is real, but time since saving stopped is not in it.
fn immersion_from(
    report: super::streak::ImmersionReport,
    unsaved: bool,
    now_ms: u64,
) -> Measured<super::streak::ImmersionReport> {
    if unsaved {
        Measured::partial(
            report,
            now_ms,
            "Time played since saving stopped is not counted here yet.",
        )
    } else {
        Measured::known(report, now_ms)
    }
}

/// The earlier of what the header kept and what the library shows now. The header alone
/// would drop the months before the store if the write that records them had failed.
fn floor_from(
    kept: &super::evidence::EvidenceFrom,
    evidence: &super::evidence::LibraryEvidence,
) -> super::evidence::EvidenceFrom {
    let mut floor = kept.clone();
    floor.merge_earlier(&evidence.horizon());
    floor
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

    fn store_with_days(first_run: &str, days: &[(&str, u64)]) -> crate::progress::store::Loaded {
        let mut ledger = crate::progress::ledger::Ledger::default();
        for (on, ms) in days {
            ledger.credit(
                &serde_json::from_str(&format!("\"{on}\"")).expect("a day"),
                crate::progress::credit::ImmersionSource::Watching,
                crate::progress::credit::Credit {
                    credited_ms: *ms,
                    unmeasured_ms: 0,
                },
                false,
            );
        }
        crate::progress::store::Loaded::Present {
            header: crate::progress::store::Header {
                v: 1,
                first_run_day: serde_json::from_str(&format!("\"{first_run}\"")).expect("a day"),
                rollover_hour: 4,
                history_lost_at_ms: None,
                evidence_from: Default::default(),
            },
            store: crate::progress::store::ProgressStore {
                days: ledger,
                ..Default::default()
            },
        }
    }

    fn library_on(stamps: &[u64]) -> crate::progress::evidence::LibraryEvidence {
        let videos: Vec<crate::app_types::WatchedVideo> = stamps
            .iter()
            .map(|added_at_ms| crate::app_types::WatchedVideo {
                added_at_ms: *added_at_ms,
                ..Default::default()
            })
            .collect();
        crate::progress::evidence::collect(&[], &videos, &day())
    }

    fn assembled(
        loaded: crate::progress::store::Loaded,
        evidence: &crate::progress::evidence::LibraryEvidence,
        write_failure: Option<crate::progress::health::WriteFailure>,
    ) -> ProgressReport {
        assembled_with_cards(loaded, evidence, None, write_failure)
    }

    fn assembled_with_cards(
        loaded: crate::progress::store::Loaded,
        evidence: &crate::progress::evidence::LibraryEvidence,
        cards: Option<&crate::progress::cards::CardHistory>,
        write_failure: Option<crate::progress::health::WriteFailure>,
    ) -> ProgressReport {
        assemble(
            loaded,
            &crate::progress::calendar::Sources {
                library: evidence,
                cards,
            },
            crate::progress::library::summarise(&[]),
            &Vocabulary {
                settings: build("Kaishi"),
                list: None,
            },
            &day(),
            write_failure,
            1,
        )
    }

    fn failing() -> Option<crate::progress::health::WriteFailure> {
        Some(crate::progress::health::WriteFailure {
            since_ms: 5,
            reason: "Access is denied.".to_string(),
        })
    }

    #[test]
    fn a_report_while_saving_fails_says_so_and_qualifies_the_time() {
        let report = assembled(store_with_days("2026-09-08", &[]), &library_on(&[]), failing());
        assert_eq!(report.write_failure, failing());
        let immersion = serde_json::to_value(&report.immersion).expect("serialise");
        assert_eq!(immersion["status"], serde_json::json!("partial"));
        assert!(immersion["value"].is_object(), "what was saved is still shown");
    }

    #[test]
    fn a_report_while_saving_works_carries_no_notice() {
        let report = assembled(store_with_days("2026-09-08", &[]), &library_on(&[]), None);
        assert_eq!(report.write_failure, None);
        let immersion = serde_json::to_value(&report.immersion).expect("serialise");
        assert_eq!(immersion["status"], serde_json::json!("known"));
    }

    /// Said beside a store that cannot be opened too: saving is failing there as well.
    #[test]
    fn an_unreadable_store_counts_no_day_and_still_passes_the_failure_on() {
        let report = assembled(
            crate::progress::store::Loaded::Unreadable("broken".to_string()),
            &library_on(&[1_775_000_000_000]),
            failing(),
        );
        assert!(!report.store_readable);
        assert_eq!(report.calendar.counted_from, None);
        assert!(report.calendar.days.iter().all(|entry| entry.combined_ms.is_none()));
        assert_eq!(report.write_failure, failing());
    }

    #[test]
    fn a_missing_store_is_readable_but_has_nothing_counted() {
        let report = assembled(crate::progress::store::Loaded::Missing, &library_on(&[]), None);
        assert!(report.store_readable);
        assert_eq!(report.calendar.counted_from, None);
        assert_eq!(report.first_run_day, None);
    }

    /// The header never got the library's floor, because that write failed. The months
    /// before the store must still be drawn from what the library shows.
    #[test]
    fn the_calendar_reaches_the_library_even_when_the_header_never_saved_it() {
        let library = library_on(&[1_775_000_000_000]);
        let earliest = library.earliest().cloned().expect("a date");
        let report = assembled(store_with_days("2026-09-08", &[]), &library, failing());
        assert_eq!(report.calendar.first_day, Some(earliest));
    }

    #[test]
    fn a_counted_day_in_the_store_reaches_the_calendar() {
        let report = assembled(
            store_with_days("2026-09-08", &[("2026-09-09", 60_000)]),
            &library_on(&[]),
            None,
        );
        let counted = report
            .calendar
            .days
            .iter()
            .find(|entry| entry.day.as_str() == "2026-09-09")
            .expect("the day is drawn");
        assert_eq!(counted.combined_ms, Some(60_000));
    }

    fn one_card_on(day: &str) -> crate::progress::cards::CardHistory {
        let made = 1_789_041_600_000;
        let history = crate::progress::cards::tally(
            &[(made, crate::progress::cards::CardKind::Word)],
            1,
            serde_json::from_str(&format!("\"{day}\"")).expect("a day"),
            None,
        );
        assert!(history.from.is_some(), "the note falls before the answer");
        history
    }

    #[test]
    fn every_state_of_the_store_passes_the_card_history_on() {
        let cards = one_card_on("2026-09-10");
        for loaded in [
            store_with_days("2026-09-08", &[]),
            crate::progress::store::Loaded::Missing,
            crate::progress::store::Loaded::Unreadable("broken".to_string()),
        ] {
            let report = assembled_with_cards(loaded, &library_on(&[]), Some(&cards), None);
            let counted = report.calendar.cards.expect("Anki's answer is carried");
            assert_eq!(counted.from, cards.from);
            assert!(report.calendar.days.iter().any(|entry| entry.cards.is_some()));
        }
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
            calendar: crate::progress::calendar::uncounted(
                &crate::progress::calendar::Sources {
                    library: &evidence,
                    cards: None,
                },
                &Default::default(),
                &day(),
            ),
            today: day(),
            library: crate::progress::library::summarise(&[]),
            comparison: None,
            levels: Default::default(),
            readings: 0,
            first_run_day: Some(day()),
            damaged_rows: 0,
            newer_rows: 0,
            store_readable: true,
            write_failure: None,
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
                "levels",
                "library",
                "newerRows",
                "readings",
                "storeReadable",
                "today",
                "writeFailure",
            ]
        );
    }

    #[test]
    fn a_fixed_corpus_shows_the_word_list_growing() {
        let earlier = sample(1, "Kaishi", vec![item("a", "f1", 1_000, 500)]);
        let later = sample(2, "Kaishi", vec![item("a", "f1", 1_000, 600)]);
        let comparison = compare(&earlier, &later).expect("comparable");
        assert_eq!(comparison.earlier_percent, 50.0);
        assert_eq!(comparison.later_percent, 60.0);
        assert_eq!(comparison.delta_points, 10.0);
        assert_eq!(comparison.items_compared, 1);
    }

    #[test]
    fn material_added_since_the_earlier_sample_is_held_out_of_the_change() {
        let earlier = sample(1, "Kaishi", vec![item("a", "f1", 1_000, 500)]);
        let later = sample(
            2,
            "Kaishi",
            vec![item("a", "f1", 1_000, 500), item("b", "f9", 1_000, 1_000)],
        );
        let comparison = compare(&earlier, &later).expect("comparable");
        assert_eq!(comparison.delta_points, 0.0, "the new easy item must not count");
        assert_eq!(comparison.items_compared, 1);
        assert_eq!(comparison.items_added, 1);
    }

    #[test]
    fn a_document_whose_text_changed_is_not_compared_against_its_own_past() {
        let earlier = sample(1, "Kaishi", vec![item("a", "old", 1_000, 500)]);
        let later = sample(2, "Kaishi", vec![item("a", "new", 1_000, 900)]);
        assert_eq!(compare(&earlier, &later), None, "nothing left to compare");

        let mixed_earlier = sample(
            1,
            "Kaishi",
            vec![item("a", "old", 1_000, 500), item("b", "same", 1_000, 500)],
        );
        let mixed_later = sample(
            2,
            "Kaishi",
            vec![item("a", "new", 1_000, 990), item("b", "same", 1_000, 600)],
        );
        let comparison = compare(&mixed_earlier, &mixed_later).expect("comparable");
        assert_eq!(comparison.items_changed, 1);
        assert_eq!(comparison.items_compared, 1);
        assert_eq!(comparison.delta_points, 10.0, "only the unchanged item counts");
    }

    #[test]
    fn samples_from_different_vocabulary_settings_are_not_comparable() {
        let earlier = sample(1, "Kaishi", vec![item("a", "f1", 1_000, 500)]);
        let later = sample(2, "Lapis", vec![item("a", "f1", 1_000, 600)]);
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
    fn a_comparison_needs_two_hundred_words_both_readings_had() {
        let earlier = sample(
            1,
            "Kaishi",
            vec![item("a", "f1", 199, 100), item("gone", "g", 5_000, 10)],
        );
        let later = sample(
            2,
            "Kaishi",
            vec![item("a", "f1", 199, 150), item("new", "n", 5_000, 4_000)],
        );
        assert_eq!(compare(&earlier, &later), None, "199 words in common is too few");

        let earlier = sample(1, "Kaishi", vec![item("a", "f1", 200, 100)]);
        let later = sample(2, "Kaishi", vec![item("a", "f1", 200, 150)]);
        let comparison = compare(&earlier, &later).expect("two hundred is enough");
        assert_eq!(comparison.delta_points, 25.0);
    }

    #[test]
    fn the_same_text_counted_differently_is_not_compared() {
        let earlier = sample(
            1,
            "Kaishi",
            vec![item("a", "f1", 1_000, 500), item("b", "f2", 1_000, 500)],
        );
        let later = sample(
            2,
            "Kaishi",
            vec![item("a", "f1", 1_100, 800), item("b", "f2", 1_000, 520)],
        );
        let comparison = compare(&earlier, &later).expect("comparable");
        assert_eq!(
            (comparison.items_compared, comparison.items_changed),
            (1, 1)
        );
        assert_eq!(comparison.delta_points, 2.0, "only the transcript counted alike");
    }

    #[test]
    fn a_transcript_with_no_words_is_not_counted_in_a_comparison() {
        let earlier = sample(
            1,
            "Kaishi",
            vec![
                item("a", "f1", 1_000, 500),
                item("quiet", "q", 0, 0),
                item("retried", "r1", 0, 0),
            ],
        );
        let later = sample(
            2,
            "Kaishi",
            vec![
                item("a", "f1", 1_000, 600),
                item("quiet", "q", 0, 0),
                item("retried", "r2", 0, 0),
                item("silence", "s", 0, 0),
            ],
        );
        let comparison = compare(&earlier, &later).expect("comparable");
        assert_eq!(
            (
                comparison.items_compared,
                comparison.items_added,
                comparison.items_changed
            ),
            (1, 0, 0)
        );
    }

    #[test]
    fn the_comparison_reaches_past_an_incomparable_neighbour() {
        let samples = vec![
            sample(1, "Kaishi", vec![item("a", "f1", 1_000, 500)]),
            sample(2, "Lapis", vec![item("a", "f1", 1_000, 100)]),
            sample(3, "Kaishi", vec![item("a", "f1", 1_000, 700)]),
        ];
        let comparison = latest_comparison(&samples).expect("comparable with the first");
        assert_eq!(comparison.earlier_taken_at_ms, 1);
        assert_eq!(comparison.delta_points, 20.0);
    }

    #[test]
    fn a_repeated_reading_is_counted_once_on_the_page() {
        let loaded = match store_with_days("2026-09-08", &[]) {
            crate::progress::store::Loaded::Present { header, mut store } => {
                let repeat = sample(1, "Kaishi", vec![item("a", "f1", 1_000, 500)]);
                store.samples = vec![repeat.clone(), repeat];
                crate::progress::store::Loaded::Present { header, store }
            }
            other => other,
        };
        let report = assembled(loaded, &library_on(&[]), None);
        assert_eq!(report.readings, 1);
    }

    fn listed(taken_at_ms: u64, list: u64, items: Vec<SampleItem>) -> Sample {
        Sample::new(taken_at_ms, day(), build("Kaishi"), list, 0, items, 1_000)
    }

    #[test]
    fn the_comparison_starts_from_the_last_reading_of_the_word_list_before() {
        let samples = vec![
            listed(1, 10, vec![item("a", "f1", 1_000, 500)]),
            listed(2, 10, vec![item("a", "f1", 1_000, 500), item("b", "f2", 1_000, 400)]),
            listed(3, 20, vec![item("a", "f1", 1_000, 550), item("b", "f2", 1_000, 400)]),
        ];
        let comparison = latest_comparison(&samples).expect("comparable");
        assert_eq!(
            comparison.items_compared, 2,
            "measured from the list's last reading"
        );
        assert_eq!(
            comparison.earlier_taken_at_ms, 1,
            "dated where the line draws that list"
        );
    }

    #[test]
    fn the_comparison_ends_at_the_first_reading_of_the_newest_word_list() {
        let samples = vec![
            listed(1, 10, vec![item("a", "f1", 1_000, 500)]),
            listed(2, 20, vec![item("a", "f1", 1_000, 550)]),
            listed(3, 20, vec![item("a", "f1", 1_000, 550), item("c", "f3", 1_000, 900)]),
        ];
        let comparison = latest_comparison(&samples).expect("comparable");
        assert_eq!(comparison.later_taken_at_ms, 2);
        assert_eq!(comparison.items_added, 0, "the line's step, not the newest library");
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
        let samples = vec![sample(500, "Kaishi", vec![item("a", "f1", 1_000, 420)])];
        let value = serde_json::to_value(coverage_from(&samples, &build("Lapis")))
            .expect("serialize");
        assert_eq!(value["value"], serde_json::json!(42.0));
        assert_eq!(value["status"], serde_json::json!("stale"));
        assert_eq!(value["asOfMs"], serde_json::json!(500));
    }

    #[test]
    fn coverage_that_missed_a_document_says_so_and_keeps_its_value() {
        let mut samples = vec![sample(500, "Kaishi", vec![item("a", "f1", 1_000, 420)])];
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
