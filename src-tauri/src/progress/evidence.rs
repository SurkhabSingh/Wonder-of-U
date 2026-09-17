use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::app_types::{RecentRecording, WatchedVideo};

use super::day::{day_key_for_ms, DayKey};

/// Kept in the header and only ever moved earlier: deleting a recording shrinks the set,
/// and a horizon that moved forward would re-label a region as one nobody was counting.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct EvidenceFrom {
    #[serde(default)]
    pub(crate) library: Option<DayKey>,
}

impl EvidenceFrom {
    pub(crate) fn merge_earlier(&mut self, seen: &EvidenceFrom) -> bool {
        let Some(theirs) = seen.library.as_ref() else {
            return false;
        };
        if self.library.as_ref().is_some_and(|mine| mine <= theirs) {
            return false;
        }
        self.library = Some(theirs.clone());
        true
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct MaterialCounts {
    pub(crate) recordings: u32,
    pub(crate) videos: u32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LibraryEvidence {
    pub(crate) days: BTreeMap<DayKey, MaterialCounts>,
    pub(crate) dropped: u32,
}

impl LibraryEvidence {
    pub(crate) fn earliest(&self) -> Option<&DayKey> {
        self.days.keys().next()
    }

    pub(crate) fn horizon(&self) -> EvidenceFrom {
        EvidenceFrom {
            library: self.earliest().cloned(),
        }
    }
}

/// `last_opened_at_ms` is deliberately not read: one overwritten value, so its mark would
/// move on every reopen and the calendar would contradict what it showed yesterday.
pub(crate) fn collect(
    recordings: &[RecentRecording],
    videos: &[WatchedVideo],
    today: &DayKey,
) -> LibraryEvidence {
    let mut evidence = LibraryEvidence::default();
    let stamps = recordings
        .iter()
        .map(|recording| (recording.created_at_ms, false))
        .chain(videos.iter().map(|video| (video.added_at_ms, true)));
    for (stamp, is_video) in stamps {
        let Some(day) = usable_day(stamp, today) else {
            evidence.dropped += 1;
            continue;
        };
        let counts = evidence.days.entry(day).or_default();
        let slot = if is_video {
            &mut counts.videos
        } else {
            &mut counts.recordings
        };
        *slot = slot.saturating_add(1);
    }
    evidence
}

/// Dropped rather than clamped: an unreadable file's date falls back to now, and clamping
/// would draw a mark on a day nothing happened.
pub(crate) fn usable_day(stamp: u64, today: &DayKey) -> Option<DayKey> {
    if stamp == 0 {
        return None;
    }
    day_key_for_ms(stamp).filter(|day| day <= today)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(day: &str) -> DayKey {
        serde_json::from_str(&format!("\"{day}\"")).expect("a day key is a string")
    }

    fn from(day: &str) -> EvidenceFrom {
        EvidenceFrom {
            library: Some(key(day)),
        }
    }

    #[test]
    fn a_first_sighting_sets_the_horizon() {
        let mut header = EvidenceFrom::default();
        assert!(header.merge_earlier(&from("2026-04-06")));
        assert_eq!(header.library, Some(key("2026-04-06")));
    }

    #[test]
    fn an_earlier_sighting_moves_it_back() {
        let mut header = from("2026-06-01");
        assert!(header.merge_earlier(&from("2026-04-06")));
        assert_eq!(header.library, Some(key("2026-04-06")));
    }

    #[test]
    fn a_later_sighting_never_moves_it_forward() {
        let mut header = from("2026-04-06");
        assert!(!header.merge_earlier(&from("2026-09-10")));
        assert!(!header.merge_earlier(&EvidenceFrom::default()));
        assert_eq!(header.library, Some(key("2026-04-06")));
    }

    #[test]
    fn the_same_sighting_is_not_a_move() {
        let mut header = from("2026-04-06");
        assert!(!header.merge_earlier(&from("2026-04-06")));
    }

    #[test]
    fn a_missing_date_is_dropped_rather_than_drawn_on_today() {
        let today = key("2026-09-15");
        assert_eq!(usable_day(0, &today), None);
    }

    #[test]
    fn a_date_after_today_is_dropped() {
        let today = key("2026-09-15");
        let a_year_on = 1_820_000_000_000;
        assert_eq!(usable_day(a_year_on, &today), None);
    }

    #[test]
    fn a_real_date_becomes_the_day_it_falls_on() {
        let today = key("2026-09-15");
        let stamp = 1_789_000_000_000;
        let day = usable_day(stamp, &today).expect("a usable date");
        assert!(day <= today);
        assert_eq!(Some(day), day_key_for_ms(stamp));
    }

    #[test]
    fn videos_and_recordings_both_reach_the_horizon() {
        let today = key("2026-09-15");
        let video = |added_at_ms| WatchedVideo {
            added_at_ms,
            ..WatchedVideo::default()
        };
        let evidence = collect(&[], &[video(1_789_000_000_000), video(0)], &today);
        assert_eq!(evidence.days.len(), 1);
        assert_eq!(evidence.dropped, 1);
        assert_eq!(evidence.earliest(), evidence.days.keys().next());
    }

    #[test]
    fn each_item_is_counted_once_on_its_day_as_what_it_is() {
        let today = key("2026-09-15");
        let recording = |created_at_ms| RecentRecording {
            file_name: "a.wav".to_string(),
            file_path: format!("C:/{created_at_ms}.wav"),
            transcript_path: None,
            transcript_language: None,
            transcripts: Vec::new(),
            translation_path: None,
            anki_note_id: None,
            anki_deck_name: None,
            anki_note_type: None,
            anki_pushes: Vec::new(),
            furigana_applied: false,
            audio_deleted: false,
            duration_ms: 0,
            bytes_written: 0,
            created_at_ms,
            source: None,
            source_url: None,
            title: None,
        };
        let video = |added_at_ms| WatchedVideo {
            added_at_ms,
            ..WatchedVideo::default()
        };
        let noon = 1_789_041_600_000;
        let evidence = collect(
            &[recording(noon), recording(noon + 1), recording(0)],
            &[video(noon + 2), video(noon - 86_400_000)],
            &today,
        );
        let day = day_key_for_ms(noon).expect("a real date");
        assert_eq!(
            evidence.days.get(&day),
            Some(&MaterialCounts {
                recordings: 2,
                videos: 1,
            })
        );
        assert_eq!(evidence.days.len(), 2);
        assert_eq!(evidence.dropped, 1);
    }

    #[test]
    fn the_counts_reach_the_page_under_the_names_it_reads() {
        let wire = serde_json::to_value(MaterialCounts::default()).expect("serialises");
        let mut keys: Vec<&str> = wire
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["recordings", "videos"]);
    }

    #[test]
    fn nothing_recorded_has_no_horizon() {
        let evidence = collect(&[], &[], &key("2026-09-15"));
        assert_eq!(evidence.earliest(), None);
        assert_eq!(evidence.horizon(), EvidenceFrom::default());
        assert_eq!(evidence.dropped, 0);
    }
}
