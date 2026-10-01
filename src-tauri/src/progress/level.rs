use serde::Serialize;

use crate::app_types::KnownWordsBuild;

use super::day::{day_key_for_ms, DayKey};
use super::report::{compare, percent, round_tenth};
use super::store::Sample;
use super::vocabulary::WordHistory;

/// The word list as it stands, for a count no reading has recorded yet.
pub(crate) struct WordList {
    pub(crate) built_at_ms: u64,
    pub(crate) build: KnownWordsBuild,
    pub(crate) words: u32,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) enum NotCompared {
    SettingsChanged,
    /// Fewer than `MIN_CONTENT_TOKENS` words were in both readings, unchanged.
    TooLittleInCommon,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReadingPoint {
    pub(crate) at_ms: u64,
    pub(crate) gained: f64,
    pub(crate) step: Option<f64>,
    pub(crate) compared: usize,
    pub(crate) coverage: f64,
    pub(crate) not_compared: Option<NotCompared>,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WordsPoint {
    pub(crate) at_ms: u64,
    pub(crate) words: u32,
    pub(crate) settings_changed: bool,
}

/// A day the review log spoke for, before the first word list this app measured.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BackfillPoint {
    pub(crate) at_ms: i64,
    pub(crate) day: DayKey,
    pub(crate) words: u32,
}

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Levels {
    pub(crate) reading: Vec<ReadingPoint>,
    pub(crate) words: Vec<WordsPoint>,
    /// What Anki's log says the list held before the first measured count, oldest first.
    pub(crate) backfill: Vec<BackfillPoint>,
}

pub(crate) struct Version<'a> {
    pub(crate) first: &'a Sample,
    pub(crate) last: &'a Sample,
}

/// A reading that measured exactly what the one before it did, whatever rebuilt the word list
/// in between, so it adds a repeat and nothing else.
fn repeats(earlier: &Sample, later: &Sample) -> bool {
    earlier.build.matches(&later.build) && earlier.items_with_words().eq(later.items_with_words())
}

/// Readings grouped by the word list they were taken with, oldest first; a repeat joins the
/// group before it.
pub(crate) fn versions(samples: &[Sample]) -> Vec<Version<'_>> {
    let mut versions: Vec<Version<'_>> = Vec::new();
    for sample in samples {
        match versions.last_mut() {
            Some(version)
                if version.last.same_word_list(sample) || repeats(version.last, sample) =>
            {
                version.last = sample
            }
            _ => versions.push(Version {
                first: sample,
                last: sample,
            }),
        }
    }
    versions
}

/// Each step compares one word list's last reading with the next list's first, over the
/// transcripts both had unchanged, so material added in between cannot move the line.
pub(crate) fn levels(
    samples: &[Sample],
    list: Option<&WordList>,
    history: Option<&WordHistory>,
) -> Levels {
    let versions = versions(samples);
    let mut reading = Vec::new();
    let mut gained = 0.0;
    for (at, version) in versions.iter().enumerate() {
        let before = at.checked_sub(1).map(|earlier| &versions[earlier]);
        let step = before.and_then(|before| compare(before.last, version.first));
        let not_compared = match (before, &step) {
            (Some(before), None) if !before.last.build.matches(&version.first.build) => {
                Some(NotCompared::SettingsChanged)
            }
            (Some(_), None) => Some(NotCompared::TooLittleInCommon),
            _ => None,
        };
        if let Some(step) = &step {
            gained = round_tenth(gained + step.delta_points);
        }
        let (content, known) = version.first.totals();
        reading.push(ReadingPoint {
            at_ms: version.first.taken_at_ms,
            gained,
            step: step.as_ref().map(|step| step.delta_points),
            compared: step.as_ref().map_or(0, |step| step.items_compared),
            coverage: percent(known, content),
            not_compared,
        });
    }

    // A count belongs to a word list, so this groups by list alone; `held` then folds the
    // counts that did not move.
    let mut words = Vec::new();
    let mut previous: Option<&KnownWordsBuild> = None;
    let mut first_build: Option<&KnownWordsBuild> = None;
    let mut listed_at: Vec<u64> = Vec::new();
    for group in word_lists(samples) {
        let first = &group[0];
        let is_current = list.is_some_and(|list| list.built_at_ms == first.index_built_at_ms);
        let count = group
            .iter()
            .find_map(|sample| sample.known_words)
            .or_else(|| list.filter(|_| is_current).map(|list| list.words));
        if let Some(count) = count {
            words.push(word_point(first.taken_at_ms, count, &first.build, previous));
            previous = Some(&first.build);
            first_build = first_build.or(previous);
            listed_at.push(first.index_built_at_ms);
        }
    }
    // The list itself stands in for the reading it has not had yet.
    if let Some(list) = list {
        if !samples
            .iter()
            .any(|sample| sample.index_built_at_ms == list.built_at_ms)
        {
            words.push(word_point(
                list.built_at_ms,
                list.words,
                &list.build,
                previous,
            ));
            first_build = first_build.or(Some(&list.build));
            listed_at.push(list.built_at_ms);
        }
    }
    let mut words = held(words);

    // The replay stops where the app's own counts start: when the first counted list was built,
    // which can be days before the reading that drew it.
    let measured_from = listed_at.iter().min().copied();
    let backfill: Vec<BackfillPoint> = history
        .map_or(&[][..], |history| &history.days)
        .iter()
        .filter_map(|day| {
            let at_ms = day.day.ends_at_ms()?;
            measured_from
                .is_none_or(|measured| at_ms < i64::try_from(measured).unwrap_or(i64::MAX))
                .then(|| BackfillPoint {
                    at_ms,
                    day: day.day.clone(),
                    words: day.words,
                })
        })
        .collect();
    // The replay stands for the list before the first count, so a first count taken under
    // other settings changed what was measured, not how many words were known.
    if let (Some(history), Some(first), Some(build)) = (history, words.first_mut(), first_build) {
        first.settings_changed = !backfill.is_empty() && !history.build.matches(build);
    }

    Levels {
        reading,
        words,
        backfill,
    }
}

fn word_lists(samples: &[Sample]) -> Vec<&[Sample]> {
    let mut groups = Vec::new();
    let mut start = 0;
    for at in 1..=samples.len() {
        if at == samples.len() || !samples[at - 1].same_word_list(&samples[at]) {
            groups.push(&samples[start..at]);
            start = at;
        }
    }
    groups
}

fn unchanged(earlier: &WordsPoint, later: &WordsPoint) -> bool {
    earlier.words == later.words && !later.settings_changed
}

fn same_day(earlier: &WordsPoint, later: &WordsPoint) -> bool {
    let day = day_key_for_ms(earlier.at_ms);
    day.is_some() && day == day_key_for_ms(later.at_ms)
}

/// A run of counts that did not move keeps its first and, from a later day, its last: when the
/// count was reached and how long it held. Each one between would add a dot and nothing else.
fn held(points: Vec<WordsPoint>) -> Vec<WordsPoint> {
    let mut kept: Vec<WordsPoint> = Vec::with_capacity(points.len());
    for point in points {
        match kept.as_mut_slice() {
            [.., before, last] if unchanged(before, last) && unchanged(last, &point) => {
                *last = point;
            }
            [.., first] if unchanged(first, &point) && same_day(first, &point) => {}
            _ => kept.push(point),
        }
    }
    kept
}

fn word_point(
    at_ms: u64,
    words: u32,
    build: &KnownWordsBuild,
    previous: Option<&KnownWordsBuild>,
) -> WordsPoint {
    WordsPoint {
        at_ms,
        words,
        settings_changed: previous.is_some_and(|earlier| !earlier.matches(build)),
    }
}

pub(crate) fn distinct_readings(samples: &[Sample]) -> usize {
    samples
        .iter()
        .enumerate()
        .filter(|(at, sample)| {
            at.checked_sub(1)
                .map(|earlier| &samples[earlier])
                .is_none_or(|earlier| !repeats(earlier, sample))
        })
        .count()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_types::VocabularySource;
    use crate::progress::store::SampleItem;

    fn build(note_type: &str) -> KnownWordsBuild {
        KnownWordsBuild {
            sources: vec![VocabularySource {
                note_type: note_type.to_string(),
                field: "Word".to_string(),
            }],
            mature_after_days: 21,
        }
    }

    fn words(key: &str, fingerprint: &str, content: u32, known: u32) -> SampleItem {
        SampleItem {
            key: key.to_string(),
            fingerprint: fingerprint.to_string(),
            content_tokens: content,
            known_tokens: known,
        }
    }

    /// A transcript of a thousand words, `percent` of them known.
    fn item(key: &str, fingerprint: &str, percent: u32) -> SampleItem {
        words(key, fingerprint, 1_000, percent * 10)
    }

    /// A reading at `taken_at_ms` with the word list built at `list`.
    fn reading(taken_at_ms: u64, list: u64, note_type: &str, items: Vec<SampleItem>) -> Sample {
        let day = serde_json::from_str("\"2026-09-10\"").expect("a day key is a string");
        Sample::new(taken_at_ms, day, build(note_type), list, 0, items, 1_000)
    }

    fn day(key: &str) -> DayKey {
        serde_json::from_str(&format!("\"{key}\"")).expect("a day key is a string")
    }

    fn gains(levels: &Levels) -> Vec<f64> {
        levels.reading.iter().map(|point| point.gained).collect()
    }

    #[test]
    fn the_line_climbs_only_by_what_the_same_transcripts_gained() {
        let samples = vec![
            reading(1, 100, "Kaishi", vec![item("a", "f1", 50)]),
            reading(
                2,
                200,
                "Kaishi",
                vec![item("a", "f1", 52), item("new", "n", 99)],
            ),
            reading(
                3,
                300,
                "Kaishi",
                vec![item("a", "f1", 55), item("new", "n", 99)],
            ),
        ];
        let levels = levels(&samples, None, None);
        assert_eq!(
            gains(&levels),
            [0.0, 2.0, 3.5],
            "pooled over both shared transcripts"
        );
        assert_eq!(levels.reading[1].step, Some(2.0));
        assert_eq!(
            levels.reading[1].compared, 1,
            "the new transcript is held out"
        );
        assert_eq!(
            levels.reading[1].coverage, 75.5,
            "the plain share is kept beside it"
        );
    }

    #[test]
    fn material_added_under_one_word_list_does_not_move_the_line() {
        let samples = vec![
            reading(1, 100, "Kaishi", vec![item("a", "f1", 50)]),
            reading(
                2,
                100,
                "Kaishi",
                vec![item("a", "f1", 50), item("hard", "h", 5)],
            ),
            reading(
                3,
                200,
                "Kaishi",
                vec![item("a", "f1", 51), item("hard", "h", 5)],
            ),
        ];
        let levels = levels(&samples, None, None);
        assert_eq!(levels.reading.len(), 2, "one point per word list");
        assert_eq!(gains(&levels), [0.0, 0.5]);
        assert_eq!(
            levels.reading[0].coverage, 50.0,
            "the share when the list was first read"
        );
    }

    #[test]
    fn a_settings_change_is_carried_across_flat_and_marked() {
        let samples = vec![
            reading(1, 100, "Kaishi", vec![item("a", "f1", 50)]),
            reading(2, 200, "Kaishi", vec![item("a", "f1", 53)]),
            reading(3, 300, "Lapis", vec![item("a", "f1", 90)]),
        ];
        let levels = levels(&samples, None, None);
        assert_eq!(gains(&levels), [0.0, 3.0, 3.0]);
        assert_eq!(levels.reading[2].step, None);
        assert_eq!(
            levels.reading[2].not_compared,
            Some(NotCompared::SettingsChanged)
        );
    }

    #[test]
    fn a_step_with_nothing_in_common_is_carried_across_flat() {
        let samples = vec![
            reading(1, 100, "Kaishi", vec![item("a", "f1", 50)]),
            reading(2, 200, "Kaishi", vec![item("a", "rewritten", 70)]),
        ];
        let levels = levels(&samples, None, None);
        assert_eq!(gains(&levels), [0.0, 0.0]);
        assert_eq!(
            levels.reading[1].not_compared,
            Some(NotCompared::TooLittleInCommon)
        );
    }

    #[test]
    fn a_step_on_too_little_shared_text_is_carried_across_flat() {
        let samples = vec![
            reading(
                1,
                100,
                "Kaishi",
                vec![words("a", "f1", 150, 50), item("b", "f2", 50)],
            ),
            reading(
                2,
                200,
                "Kaishi",
                vec![words("a", "f1", 150, 150), item("b", "rewritten", 90)],
            ),
        ];
        let levels = levels(&samples, None, None);
        assert_eq!(gains(&levels), [0.0, 0.0], "150 words is too few to measure");
        assert_eq!(
            levels.reading[1].not_compared,
            Some(NotCompared::TooLittleInCommon)
        );
    }

    #[test]
    fn a_reading_that_only_picked_up_wordless_transcripts_repeats_the_one_before() {
        let samples = vec![
            reading(1, 100, "Kaishi", vec![item("a", "f1", 50)]),
            reading(
                2,
                100,
                "Kaishi",
                vec![item("a", "f1", 50), words("quiet", "q", 0, 0)],
            ),
            reading(
                3,
                200,
                "Kaishi",
                vec![item("a", "f1", 50), words("quiet", "q", 0, 0), words("hush", "h", 0, 0)],
            ),
        ];
        assert_eq!(distinct_readings(&samples), 1);
        assert_eq!(
            levels(&samples, None, None).reading.len(),
            1,
            "a rebuilt list that read nothing new is no new point"
        );
    }

    #[test]
    fn the_word_list_stands_in_for_the_reading_it_has_not_had() {
        let samples = vec![reading(1, 100, "Kaishi", vec![item("a", "f1", 50)])];
        let list = WordList {
            built_at_ms: 500,
            build: build("Kaishi"),
            words: 2_528,
        };
        let levels = levels(&samples, Some(&list), None);
        let counts: Vec<(u64, u32)> = levels
            .words
            .iter()
            .map(|point| (point.at_ms, point.words))
            .collect();
        assert_eq!(counts, [(1, 1_000), (500, 2_528)]);
    }

    #[test]
    fn an_older_reading_without_a_count_takes_the_lists_own() {
        let mut older = reading(1, 500, "Kaishi", vec![item("a", "f1", 50)]);
        older.known_words = None;
        let list = WordList {
            built_at_ms: 500,
            build: build("Kaishi"),
            words: 2_528,
        };
        let levels = levels(&[older], Some(&list), None);
        assert_eq!(
            levels.words.len(),
            1,
            "the list is that reading's, not a second point"
        );
        assert_eq!(levels.words[0].words, 2_528);
    }

    fn count(at_ms: u64, words: u32, settings_changed: bool) -> WordsPoint {
        WordsPoint {
            at_ms,
            words,
            settings_changed,
        }
    }

    fn times(points: Vec<WordsPoint>) -> Vec<u64> {
        points.iter().map(|point| point.at_ms).collect()
    }

    /// Noon UTC on 10 September, `days` later.
    fn on_day(days: u64) -> u64 {
        1_789_041_600_000 + days * 86_400_000
    }

    #[test]
    fn a_run_of_counts_that_did_not_move_keeps_its_first_and_last() {
        let points = vec![
            count(on_day(0), 2_528, false),
            count(on_day(1), 2_544, false),
            count(on_day(2), 2_544, false),
            count(on_day(3), 2_544, false),
            count(on_day(4), 2_544, false),
            count(on_day(5), 2_559, false),
        ];
        assert_eq!(
            times(held(points)),
            [on_day(0), on_day(1), on_day(4), on_day(5)],
            "reached on the first day, still held on the fourth"
        );
    }

    #[test]
    fn a_run_within_one_day_keeps_only_its_first() {
        let later_that_day = on_day(1) + 3_600_000;
        let points = vec![
            count(on_day(0), 2_559, false),
            count(on_day(1), 2_568, false),
            count(later_that_day, 2_568, false),
        ];
        assert_eq!(
            times(held(points)),
            [on_day(0), on_day(1)],
            "the hour later would add a second dot on the same day and nothing else"
        );
    }

    #[test]
    fn a_count_that_did_not_move_across_a_settings_change_keeps_the_change() {
        let points = vec![
            count(on_day(0), 2_544, false),
            count(on_day(1), 2_544, true),
            count(on_day(2), 2_544, false),
            count(on_day(3), 2_544, false),
        ];
        assert_eq!(
            times(held(points)),
            [on_day(0), on_day(1), on_day(3)],
            "the second run starts at the change"
        );
    }

    #[test]
    fn refreshes_that_found_the_same_count_draw_one_stretch() {
        let samples: Vec<Sample> = (1..=5)
            .map(|at| reading(on_day(at), on_day(at), "Kaishi", vec![item("a", "f1", 50)]))
            .collect();
        assert_eq!(times(levels(&samples, None, None).words), [on_day(1), on_day(5)]);
    }

    #[test]
    fn the_word_list_joins_the_stretch_it_continues() {
        let samples = vec![
            reading(on_day(0), on_day(0), "Kaishi", vec![item("a", "f1", 50)]),
            reading(on_day(1), on_day(1), "Kaishi", vec![item("a", "f1", 50)]),
        ];
        let list = WordList {
            built_at_ms: on_day(3),
            build: build("Kaishi"),
            words: 1_000,
        };
        assert_eq!(
            times(levels(&samples, Some(&list), None).words),
            [on_day(0), on_day(3)],
            "the unread list ends the stretch its readings began"
        );
    }

    #[test]
    fn a_word_count_under_other_settings_is_marked() {
        let samples = vec![
            reading(1, 100, "Kaishi", vec![item("a", "f1", 50)]),
            reading(2, 200, "Lapis", vec![item("a", "f1", 50)]),
        ];
        let levels = levels(&samples, None, None);
        assert!(!levels.words[0].settings_changed);
        assert!(levels.words[1].settings_changed);
    }

    #[test]
    fn a_reading_that_repeats_the_one_before_is_counted_once() {
        let same = vec![item("a", "f1", 50)];
        let samples = vec![
            reading(1, 100, "Kaishi", same.clone()),
            reading(2, 100, "Kaishi", same.clone()),
            reading(3, 200, "Kaishi", same.clone()),
            reading(4, 300, "Kaishi", vec![item("a", "f1", 51)]),
            reading(5, 400, "Lapis", vec![item("a", "f1", 51)]),
        ];
        assert_eq!(
            distinct_readings(&samples),
            3,
            "a rebuilt list that read the same repeats"
        );
    }

    #[test]
    fn a_word_list_rebuilt_with_identical_results_is_one_point() {
        let same = vec![item("a", "f1", 50)];
        let samples = vec![
            reading(1, 100, "Kaishi", same.clone()),
            reading(2, 200, "Kaishi", same),
            reading(3, 300, "Kaishi", vec![item("a", "f1", 52)]),
        ];
        let levels = levels(&samples, None, None);
        assert_eq!(levels.reading.len(), 2);
        assert_eq!(gains(&levels), [0.0, 2.0]);
        assert_eq!(levels.reading[1].compared, 1);
    }

    #[test]
    fn a_reading_with_the_current_list_carries_its_count_on_its_own_day() {
        let same = vec![item("a", "f1", 50)];
        let mut earlier = reading(1, 100, "Kaishi", same.clone());
        let mut repeat = reading(2, 200, "Kaishi", same);
        earlier.known_words = None;
        repeat.known_words = None;
        let list = WordList {
            built_at_ms: 200,
            build: build("Kaishi"),
            words: 2_528,
        };
        let levels = levels(&[earlier, repeat], Some(&list), None);
        let counts: Vec<(u64, u32)> = levels
            .words
            .iter()
            .map(|point| (point.at_ms, point.words))
            .collect();
        assert_eq!(counts, [(2, 2_528)], "on the reading's day, not the list's");
    }

    #[test]
    fn the_replay_stops_where_the_first_measured_count_begins() {
        // Noon on 10 September, so the day before it ended before this reading was taken.
        let noon = 1_789_041_600_000;
        let samples = vec![reading(noon, noon, "Kaishi", vec![item("a", "f1", 50)])];
        let history = WordHistory::new(
            noon,
            day("2026-09-10"),
            build("Kaishi"),
            vec![
                (day("2026-09-08"), 2_400),
                (day("2026-09-09"), 2_450),
                (day("2026-09-10"), 2_500),
            ],
        );
        let levels = levels(&samples, None, Some(&history));
        let days: Vec<&str> = levels
            .backfill
            .iter()
            .map(|point| point.day.as_str())
            .collect();
        assert_eq!(days, ["2026-09-08", "2026-09-09"], "the measured day is its own");
        assert_eq!(levels.words.len(), 1, "and the measured count is still there");
    }

    #[test]
    fn the_replay_stops_where_the_counted_list_was_built_not_where_it_was_read() {
        let noon = 1_789_041_600_000;
        let refreshed = noon - 3 * 86_400_000;
        let samples = vec![reading(noon, refreshed, "Kaishi", vec![item("a", "f1", 50)])];
        let history = WordHistory::new(
            noon,
            day("2026-09-10"),
            build("Kaishi"),
            ["2026-09-05", "2026-09-06", "2026-09-07", "2026-09-08", "2026-09-09"]
                .into_iter()
                .map(|key| (day(key), 2_500))
                .collect(),
        );
        let levels = levels(&samples, None, Some(&history));
        let days: Vec<&str> = levels
            .backfill
            .iter()
            .map(|point| point.day.as_str())
            .collect();
        assert_eq!(days, ["2026-09-05", "2026-09-06"], "the list was refreshed on the seventh");
    }

    #[test]
    fn a_first_count_under_other_settings_meets_the_replay_as_a_change_of_measure() {
        let noon = 1_789_041_600_000;
        let samples = vec![reading(noon, noon, "Kaishi", vec![item("a", "f1", 50)])];
        let replayed = |note_type: &str, last: &str| {
            WordHistory::new(noon, day("2026-09-10"), build(note_type), vec![(day(last), 2_400)])
        };
        let first_count = |history: Option<&WordHistory>| {
            levels(&samples, None, history).words[0].settings_changed
        };
        assert!(!first_count(Some(&replayed("Kaishi", "2026-09-08"))));
        assert!(first_count(Some(&replayed("Lapis", "2026-09-08"))), "replayed under Lapis");
        assert!(!first_count(Some(&replayed("Lapis", "2026-09-10"))), "no replayed day is drawn");
        assert!(!first_count(None), "nothing replayed");
    }

    #[test]
    fn the_lines_reach_the_page_under_the_names_it_reads() {
        let samples = vec![
            reading(1, 100, "Kaishi", vec![item("a", "f1", 50)]),
            reading(2, 200, "Lapis", vec![item("a", "f1", 50)]),
        ];
        let wire = serde_json::to_value(levels(&samples, None, None)).expect("serialises");
        let keys = |value: &serde_json::Value| {
            let mut keys: Vec<String> = value
                .as_object()
                .expect("an object")
                .keys()
                .cloned()
                .collect();
            keys.sort_unstable();
            keys
        };
        assert_eq!(keys(&wire), ["backfill", "reading", "words"]);
        assert_eq!(
            keys(&wire["reading"][0]),
            [
                "atMs",
                "compared",
                "coverage",
                "gained",
                "notCompared",
                "step"
            ]
        );
        assert_eq!(
            keys(&wire["words"][0]),
            ["atMs", "settingsChanged", "words"]
        );
        assert_eq!(
            wire["reading"][1]["notCompared"],
            serde_json::json!("settingsChanged")
        );
        assert_eq!(
            serde_json::to_value(NotCompared::TooLittleInCommon).expect("serialises"),
            serde_json::json!("tooLittleInCommon")
        );
    }
}
