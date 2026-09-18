use serde::Serialize;

use crate::app_types::KnownWordsBuild;

use super::report::{compare, percent, round_tenth};
use super::store::Sample;

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
    NothingInCommon,
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

#[derive(Debug, Clone, Default, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Levels {
    pub(crate) reading: Vec<ReadingPoint>,
    pub(crate) words: Vec<WordsPoint>,
}

pub(crate) struct Version<'a> {
    pub(crate) first: &'a Sample,
    pub(crate) last: &'a Sample,
}

/// A reading that measured exactly what the one before it did, whatever rebuilt the word list
/// in between, so it adds a repeat and nothing else.
fn repeats(earlier: &Sample, later: &Sample) -> bool {
    earlier.build.matches(&later.build) && earlier.items == later.items
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
pub(crate) fn levels(samples: &[Sample], list: Option<&WordList>) -> Levels {
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
            (Some(_), None) => Some(NotCompared::NothingInCommon),
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

    // A count belongs to a word list, so this groups by list alone and merges no repeats.
    let mut words = Vec::new();
    let mut previous: Option<&KnownWordsBuild> = None;
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
        }
    }

    Levels { reading, words }
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

    fn item(key: &str, fingerprint: &str, known: u32) -> SampleItem {
        SampleItem {
            key: key.to_string(),
            fingerprint: fingerprint.to_string(),
            content_tokens: 100,
            known_tokens: known,
        }
    }

    /// A reading at `taken_at_ms` with the word list built at `list`.
    fn reading(taken_at_ms: u64, list: u64, note_type: &str, items: Vec<SampleItem>) -> Sample {
        let day = serde_json::from_str("\"2026-09-10\"").expect("a day key is a string");
        Sample::new(taken_at_ms, day, build(note_type), list, 0, items, 1_000)
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
        let levels = levels(&samples, None);
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
        let levels = levels(&samples, None);
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
        let levels = levels(&samples, None);
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
        let levels = levels(&samples, None);
        assert_eq!(gains(&levels), [0.0, 0.0]);
        assert_eq!(
            levels.reading[1].not_compared,
            Some(NotCompared::NothingInCommon)
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
        let levels = levels(&samples, Some(&list));
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
        let levels = levels(&[older], Some(&list));
        assert_eq!(
            levels.words.len(),
            1,
            "the list is that reading's, not a second point"
        );
        assert_eq!(levels.words[0].words, 2_528);
    }

    #[test]
    fn a_word_count_under_other_settings_is_marked() {
        let samples = vec![
            reading(1, 100, "Kaishi", vec![item("a", "f1", 50)]),
            reading(2, 200, "Lapis", vec![item("a", "f1", 50)]),
        ];
        let levels = levels(&samples, None);
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
        let levels = levels(&samples, None);
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
        let levels = levels(&[earlier, repeat], Some(&list));
        let counts: Vec<(u64, u32)> = levels
            .words
            .iter()
            .map(|point| (point.at_ms, point.words))
            .collect();
        assert_eq!(counts, [(2, 2_528)], "on the reading's day, not the list's");
    }

    #[test]
    fn the_lines_reach_the_page_under_the_names_it_reads() {
        let samples = vec![
            reading(1, 100, "Kaishi", vec![item("a", "f1", 50)]),
            reading(2, 200, "Lapis", vec![item("a", "f1", 50)]),
        ];
        let wire = serde_json::to_value(levels(&samples, None)).expect("serialises");
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
        assert_eq!(keys(&wire), ["reading", "words"]);
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
    }
}
