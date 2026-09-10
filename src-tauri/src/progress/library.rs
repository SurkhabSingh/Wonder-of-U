use serde::Serialize;

use crate::app_types::RecentRecording;

use super::day::{day_key_for_ms, DayKey};

/// One day the library grew.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivityDay {
    pub(crate) day: DayKey,
    pub(crate) items: u32,
}

/// When the library was worked on.
///
/// Sparse rather than dense — only days that had something. Most days have nothing, and a
/// row per empty day would be mostly zeroes for the frontend to filter back out.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivityReport {
    pub(crate) days: Vec<ActivityDay>,
    /// The oldest day anything is known for.
    ///
    /// NOT floored at the day this feature first ran. Every recording carries the moment it
    /// arrived, and those moments are real dated evidence from before any of this existed —
    /// refusing to draw them would tell a user with months of history that they had done
    /// nothing until the day the feature shipped.
    pub(crate) since_day: Option<DayKey>,
    pub(crate) active_days: usize,
    /// Today, as this app keys days.
    ///
    /// Sent rather than worked out again on the other side. The day boundary is 04:00 and
    /// that rule lives in one place; a frontend that derived "today" from local midnight
    /// would put the last column of the calendar on a different day from the rows in it,
    /// for the four hours a night when the two disagree.
    pub(crate) today: DayKey,
    /// Items whose arrival time is unknown, so they are on no day at all. Counted, because
    /// a calendar quietly missing rows is a calendar that understates the work.
    pub(crate) items_without_a_day: usize,
}

/// What the library holds, in total.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LibraryReport {
    pub(crate) items: usize,
    pub(crate) total_ms: u64,
    /// Items whose length was never recorded. Their time is not in `total_ms`, so a total
    /// that did not say so would be quietly short.
    pub(crate) items_without_length: usize,
    pub(crate) recorded: usize,
    pub(crate) imported_from_a_link: usize,
    pub(crate) imported_from_a_file: usize,
    /// Items that predate the app noting how they arrived.
    ///
    /// Its own count rather than folded into `recorded`, which is what a reader would
    /// assume of an item with no origin. "We do not know" and "microphone" are different
    /// answers, and on this library the first is the larger of the two.
    pub(crate) unknown_origin: usize,
    pub(crate) transcribed: usize,
    pub(crate) japanese: usize,
    pub(crate) translated: usize,
}

/// Everything the two reports need, from one pass over the history.
pub(crate) fn summarise(
    recordings: &[RecentRecording],
    today: DayKey,
) -> (ActivityReport, LibraryReport) {
    let mut by_day: std::collections::BTreeMap<DayKey, u32> = std::collections::BTreeMap::new();
    let mut items_without_a_day = 0_usize;

    let mut library = LibraryReport {
        items: recordings.len(),
        total_ms: 0,
        items_without_length: 0,
        recorded: 0,
        imported_from_a_link: 0,
        imported_from_a_file: 0,
        unknown_origin: 0,
        transcribed: 0,
        japanese: 0,
        translated: 0,
    };

    for recording in recordings {
        match day_key_for_ms(recording.created_at_ms) {
            Some(day) => *by_day.entry(day).or_insert(0) += 1,
            None => items_without_a_day += 1,
        }

        if recording.duration_ms == 0 {
            library.items_without_length += 1;
        } else {
            library.total_ms += recording.duration_ms;
        }

        // Matched against the values the app actually writes: "recording" at the end of a
        // capture, "import" for a local file, "youtube" for a link. Anything else, `None`
        // included, is an origin nobody recorded — never guessed at.
        match recording.source.as_deref() {
            Some("recording") => library.recorded += 1,
            Some("import") => library.imported_from_a_file += 1,
            Some("youtube") => library.imported_from_a_link += 1,
            _ => library.unknown_origin += 1,
        }

        if !recording.transcripts.is_empty() {
            library.transcribed += 1;
        }
        if recording.transcripts.iter().any(is_japanese) {
            library.japanese += 1;
        }
        if recording.translation_path.is_some() {
            library.translated += 1;
        }
    }

    let days: Vec<ActivityDay> = by_day
        .into_iter()
        .map(|(day, items)| ActivityDay { day, items })
        .collect();

    (
        ActivityReport {
            since_day: days.first().map(|entry| entry.day.clone()),
            active_days: days.len(),
            today,
            days,
            items_without_a_day,
        },
        library,
    )
}

/// The same rule the comprehension sampler uses: `auto` counts only once the language has
/// actually been detected, because a request is not an answer.
fn is_japanese(transcript: &crate::app_types::RecordingTranscript) -> bool {
    let language = transcript.language.trim().to_ascii_lowercase();
    let detected = transcript
        .detected_language
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    language == "ja" || (language == "auto" && detected == "ja")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_types::RecordingTranscript;

    fn recording(
        created_at_ms: u64,
        duration_ms: u64,
        source: Option<&str>,
        transcripts: Vec<RecordingTranscript>,
        translated: bool,
    ) -> RecentRecording {
        RecentRecording {
            file_name: "a.wav".to_string(),
            file_path: format!("C:/{created_at_ms}.wav"),
            transcript_path: None,
            transcript_language: None,
            transcripts,
            translation_path: translated.then(|| "C:/a.txt".to_string()),
            anki_note_id: None,
            anki_deck_name: None,
            anki_note_type: None,
            anki_pushes: Vec::new(),
            furigana_applied: false,
            audio_deleted: false,
            duration_ms,
            bytes_written: 0,
            created_at_ms,
            source: source.map(str::to_string),
            source_url: None,
            title: None,
        }
    }

    fn key(day: &str) -> DayKey {
        serde_json::from_str(&format!("\"{day}\"")).expect("a day key is a string")
    }

    fn ja() -> RecordingTranscript {
        RecordingTranscript {
            language: "ja".to_string(),
            file_path: "C:/a.ja.txt".to_string(),
            detected_language: Some("ja".to_string()),
            segments_path: None,
        }
    }

    /// The finding this test exists for. On a real library most items predate the origin
    /// field, and calling those "recorded" states something nobody recorded. It is the
    /// same mistake as reading an empty answer as a zero, one field along.
    #[test]
    fn an_item_with_no_origin_is_counted_as_unknown_and_not_as_recorded() {
        let recordings = vec![
            recording(1_700_000_000_000, 1000, Some("recording"), vec![], false),
            recording(1_700_000_000_000, 1000, Some("youtube"), vec![], false),
            recording(1_700_000_000_000, 1000, Some("import"), vec![], false),
            recording(1_700_000_000_000, 1000, None, vec![], false),
            recording(1_700_000_000_000, 1000, Some("something-new"), vec![], false),
        ];
        let (_, library) = summarise(&recordings, key("2026-09-10"));
        assert_eq!(library.recorded, 1);
        assert_eq!(library.imported_from_a_link, 1);
        assert_eq!(library.imported_from_a_file, 1);
        assert_eq!(
            library.unknown_origin, 2,
            "an absent origin and one this build does not know are both unknown"
        );
        assert_eq!(
            library.recorded + library.imported_from_a_link + library.imported_from_a_file
                + library.unknown_origin,
            library.items,
            "every item lands in exactly one bucket"
        );
    }

    /// A length nobody recorded contributes nothing to the total, so the total has to be
    /// able to say how many it could not include.
    #[test]
    fn a_total_says_how_many_items_it_could_not_measure() {
        let recordings = vec![
            recording(1_700_000_000_000, 60_000, Some("recording"), vec![], false),
            recording(1_700_000_000_000, 0, Some("recording"), vec![], false),
        ];
        let (_, library) = summarise(&recordings, key("2026-09-10"));
        assert_eq!(library.total_ms, 60_000);
        assert_eq!(library.items_without_length, 1);
    }

    #[test]
    fn activity_is_grouped_by_day_and_counted() {
        // Two on one day, one on another. Exact instants do not matter here — only that
        // items sharing a day are summed and the day list is ordered.
        let day_one = 1_700_000_000_000;
        let day_two = day_one + 3 * 24 * 60 * 60 * 1000;
        let recordings = vec![
            recording(day_one, 1000, None, vec![], false),
            recording(day_one, 1000, None, vec![], false),
            recording(day_two, 1000, None, vec![], false),
        ];
        let (activity, _) = summarise(&recordings, key("2026-09-10"));
        assert_eq!(activity.active_days, 2);
        assert_eq!(activity.days.len(), 2);
        assert_eq!(activity.days[0].items, 2);
        assert_eq!(activity.days[1].items, 1);
        assert!(activity.days[0].day < activity.days[1].day, "oldest first");
        assert_eq!(activity.since_day.as_ref(), Some(&activity.days[0].day));
    }

    /// The horizon decision, pinned. Evidence from before the feature existed is real and
    /// is drawn; flooring it at first run would report a user's own history as inactivity.
    #[test]
    fn activity_reaches_back_as_far_as_the_evidence_does() {
        let long_ago = 1_600_000_000_000;
        let (activity, _) = summarise(&[recording(long_ago, 1000, None, vec![], false)], key("2026-09-10"));
        assert_eq!(activity.active_days, 1);
        assert!(activity.since_day.is_some(), "the oldest day is reported as-is");
    }

    #[test]
    fn transcribed_japanese_and_translated_are_counted_per_item() {
        let recordings = vec![
            recording(1_700_000_000_000, 1000, None, vec![ja()], true),
            recording(1_700_000_000_000, 1000, None, vec![ja(), ja()], false),
            recording(1_700_000_000_000, 1000, None, vec![], false),
        ];
        let (_, library) = summarise(&recordings, key("2026-09-10"));
        assert_eq!(library.transcribed, 2);
        assert_eq!(library.japanese, 2, "an item is Japanese once, not once per transcript");
        assert_eq!(library.translated, 1);
    }

    #[test]
    fn an_empty_library_reports_nothing_rather_than_guessing() {
        let (activity, library) = summarise(&[], key("2026-09-10"));
        assert_eq!(activity.active_days, 0);
        assert_eq!(activity.since_day, None);
        assert_eq!(library.items, 0);
        assert_eq!(library.total_ms, 0);
    }
}
