use std::collections::HashMap;

use serde::Serialize;

use super::measured::Measured;
use super::store::Sample;

/// A like-for-like change between two samples.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Comparison {
    /// Percentage points, later minus earlier, over the compared items only.
    pub(crate) delta_points: f64,
    pub(crate) earlier_percent: f64,
    pub(crate) later_percent: f64,
    /// How many documents both samples measured with unchanged text.
    pub(crate) items_compared: usize,
    /// Documents the later sample has and the earlier one did not. Reported beside the
    /// change, never folded into it.
    pub(crate) items_added: usize,
    /// Documents both samples have whose text changed in between — a re-transcribe. Held
    /// out for the same reason as a new document: their words are not the same words.
    pub(crate) items_changed: usize,
    pub(crate) earlier_taken_at_ms: u64,
    pub(crate) later_taken_at_ms: u64,
}

/// The change between two samples, over the material they both measured.
///
/// This is the whole reason samples are stored rather than recomputed. With one word list
/// that gets overwritten, a share recomputed today puts the reader's growth into both the
/// numerator and the denominator, where it cancels — what is left on screen is how hard the
/// material was. Two dated samples over a FIXED set of documents can differ only because
/// the word list changed, and that difference is the learning.
///
/// Three things are held out of the comparison, all for the same reason: their words are
/// not the same words.
///
///   * documents the later sample added — new material is not progress on old material;
///   * documents whose fingerprint moved — a re-transcribe rewrites the text in place under
///     an unchanged key, so a change of speech model would otherwise read as learning;
///   * everything, when the two samples were measured under different vocabulary settings,
///     which is not a comparison at all.
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
        // Rounded like the two shares it is drawn from. Both operands are already at one
        // decimal, but their difference is not: 50.2 - 50.0 is 0.20000000000000284 in
        // binary floating point, and that is what would reach the screen.
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

/// A share, to one decimal place.
///
/// Floored just below whole while any word remains unknown, so "100%" is only ever printed
/// for a text with nothing left in it. Rounding 99.97 up to 100 tells the reader they are
/// finished with material they are not finished with.
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

/// One decimal place, which is the precision every share and change is reported at.
fn round_tenth(value: f64) -> f64 {
    (value * 10.0).round() / 10.0
}

/// The most recent sample, and the newest earlier one it can be compared against.
///
/// The pair is chosen by walking backwards for the first sample that yields a comparison,
/// rather than taking the one immediately before: a sample taken under different vocabulary
/// settings, or over a library that has since been re-transcribed, is not comparable, and
/// stopping at it would report "no change yet" while comparable history sits behind it.
pub(crate) fn latest_comparison(samples: &[Sample]) -> Option<Comparison> {
    let latest = samples.last()?;
    samples
        .iter()
        .rev()
        .skip(1)
        .find_map(|earlier| compare(earlier, latest))
}

/// Everything the Progress page is told.
///
/// Built from local files only. It never contacts Anki, so the page opens at the same speed
/// whether Anki is running or not, and a closed Anki cannot turn a measurement that was
/// taken into a number that is missing.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProgressReport {
    pub(crate) coverage_percent: Measured<f64>,
    /// When the library was worked on, back as far as the evidence goes.
    pub(crate) activity: super::library::ActivityReport,
    /// What the library holds.
    pub(crate) library: super::library::LibraryReport,
    /// The like-for-like change, when two comparable readings exist. `None` is "not yet",
    /// which the page says in words rather than drawing as zero.
    pub(crate) comparison: Option<Comparison>,
    pub(crate) readings: usize,
    /// The first day this store existed. A series that could only have been measured from
    /// here is floored at it.
    pub(crate) first_run_day: Option<super::day::DayKey>,
    /// Rows the file holds that this build could not read. Surfaced rather than swallowed:
    /// a smaller answer with no explanation is the failure this feature is built to avoid.
    pub(crate) damaged_rows: usize,
    pub(crate) newer_rows: usize,
    /// Whether the store could be read at all. False means every number above is a guess
    /// about a file nobody opened, and the page says so instead of drawing it.
    pub(crate) store_readable: bool,
}

/// Reads the stored readings and turns them into what the page shows.
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

    // Read from the recording history rather than from the progress store: every item
    // already carries when it arrived, so this needs nothing kept and reaches back as far
    // as the library does, not as far as this feature does.
    let (activity, library) = {
        let persisted_state = app.state::<crate::app_types::SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the recording history.".to_string())?;
        super::library::summarise(&persisted.recent_recordings, super::day::today())
    };

    match super::store::load(&path) {
        super::store::Loaded::Present { header, store } => Ok(ProgressReport {
            coverage_percent: coverage_from(&store.samples, &current_build),
            comparison: latest_comparison(&store.samples),
            activity,
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
            comparison: None,
            activity,
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
            comparison: None,
            activity,
            library,
            readings: 0,
            first_run_day: None,
            damaged_rows: 0,
            newer_rows: 0,
            store_readable: false,
        }),
    }
}

/// The coverage the latest sample measured, carrying whether it can still be trusted.
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
        Sample::new(taken_at_ms, day(), build(note_type), 1, 0, items)
    }

    /// The point of the whole design. Over a fixed corpus the two shares differ only
    /// because the word list grew, which is the learning.
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

    /// New material is not progress on old material. Adding an easy transcript must not be
    /// able to move the change, or the number rewards importing rather than learning.
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

    /// A re-transcribe rewrites the text in place under an unchanged key. Without the
    /// fingerprint, swapping the speech model would read as the reader having learned.
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

    /// Two readings taken under different vocabulary settings are not a trend.
    #[test]
    fn samples_from_different_vocabulary_settings_are_not_comparable() {
        let earlier = sample(1, "Kaishi", vec![item("a", "f1", 100, 50)]);
        let later = sample(2, "Lapis", vec![item("a", "f1", 100, 60)]);
        assert_eq!(compare(&earlier, &later), None);
    }

    /// Pooled, not averaged: a two-word clip must not outvote an episode.
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

    /// "100%" must mean finished. A text with one word left in it is 99.9.
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
