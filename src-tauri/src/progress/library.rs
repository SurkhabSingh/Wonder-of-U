use serde::Serialize;

use crate::app_types::RecentRecording;

use super::day::{day_key_for_ms, DayKey};

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivityDay {
    pub(crate) day: DayKey,
    pub(crate) items: u32,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivityReport {
    pub(crate) days: Vec<ActivityDay>,
    pub(crate) since_day: Option<DayKey>,
    pub(crate) active_days: usize,
    pub(crate) today: DayKey,
    pub(crate) items_without_a_day: usize,
    pub(crate) streak: Streak,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Streak {
    pub(crate) current: usize,
    pub(crate) best: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LibraryReport {
    pub(crate) items: usize,
    pub(crate) total_ms: u64,
    pub(crate) items_without_length: usize,
    pub(crate) recorded: usize,
    pub(crate) imported_from_a_link: usize,
    pub(crate) imported_from_a_file: usize,
    pub(crate) unknown_origin: usize,
    pub(crate) transcribed: usize,
    pub(crate) japanese: usize,
    pub(crate) translated: usize,
}

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

    let streak = streak_from(&days, &today);

    (
        ActivityReport {
            since_day: days.first().map(|entry| entry.day.clone()),
            active_days: days.len(),
            today,
            streak,
            days,
            items_without_a_day,
        },
        library,
    )
}

/// A day still in progress does not break a run: counting from today alone reports every
/// streak as broken every morning. Labels are pre-shifted, so stepping back needs no zone.
fn streak_from(days: &[ActivityDay], today: &DayKey) -> Streak {
    use std::collections::HashSet;

    let present: HashSet<&str> = days.iter().map(|entry| entry.day.as_str()).collect();

    let mut current = 0;
    let mut cursor = if present.contains(today.as_str()) {
        Some(today.as_str().to_string())
    } else {
        let yesterday = step_back(today.as_str());
        yesterday.filter(|day| present.contains(day.as_str()))
    };
    while let Some(day) = cursor {
        if !present.contains(day.as_str()) {
            break;
        }
        current += 1;
        cursor = step_back(&day);
    }

    let mut best = 0;
    let mut run = 0;
    let mut previous: Option<String> = None;
    for entry in days {
        let day = entry.day.as_str();
        run = match previous.as_deref().and_then(step_back_of(day)) {
            Some(_) => run + 1,
            None => 1,
        };
        best = best.max(run);
        previous = Some(day.to_string());
    }

    Streak { current, best }
}

fn step_back_of(day: &str) -> impl Fn(&str) -> Option<()> + '_ {
    move |previous| {
        step_back(day).filter(|before| before == previous).map(|_| ())
    }
}

/// Arithmetic on the date, not an instant: the rollover was applied when the label was
/// made, and re-reading it as a moment re-applies a timezone it no longer has.
fn step_back(day: &str) -> Option<String> {
    let mut parts = day.split('-');
    let year: i32 = parts.next()?.parse().ok()?;
    let month: u32 = parts.next()?.parse().ok()?;
    let date: u32 = parts.next()?.parse().ok()?;
    let civil = chrono::NaiveDate::from_ymd_opt(year, month, date)?;
    Some(
        civil
            .pred_opt()?
            .format("%Y-%m-%d")
            .to_string(),
    )
}

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

    fn days_on(labels: &[&str]) -> Vec<ActivityDay> {
        labels
            .iter()
            .map(|label| ActivityDay {
                day: key(label),
                items: 1,
            })
            .collect()
    }

    #[test]
    fn a_day_not_yet_worked_does_not_break_a_run() {
        let days = days_on(&["2026-09-07", "2026-09-08", "2026-09-09"]);
        assert_eq!(
            streak_from(&days, &key("2026-09-10")).current,
            3,
            "yesterday was worked, so the run is alive"
        );
        assert_eq!(
            streak_from(&days, &key("2026-09-11")).current,
            0,
            "a whole day was missed, so it is not"
        );
    }

    #[test]
    fn a_run_counts_only_consecutive_days() {
        let days = days_on(&["2026-09-01", "2026-09-03", "2026-09-04", "2026-09-05"]);
        let streak = streak_from(&days, &key("2026-09-05"));
        assert_eq!(streak.current, 3, "the gap on the 2nd ends the earlier run");
        assert_eq!(streak.best, 3);
    }

    #[test]
    fn the_best_run_need_not_be_the_current_one() {
        let days = days_on(&[
            "2026-08-01", "2026-08-02", "2026-08-03", "2026-08-04", "2026-09-10",
        ]);
        let streak = streak_from(&days, &key("2026-09-10"));
        assert_eq!(streak.current, 1);
        assert_eq!(streak.best, 4);
    }

    #[test]
    fn a_month_boundary_does_not_end_a_run() {
        let days = days_on(&["2026-08-30", "2026-08-31", "2026-09-01"]);
        assert_eq!(streak_from(&days, &key("2026-09-01")).current, 3);
    }

    #[test]
    fn no_days_is_no_streak_rather_than_a_broken_one() {
        let streak = streak_from(&[], &key("2026-09-10"));
        assert_eq!(streak.current, 0);
        assert_eq!(streak.best, 0);
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
