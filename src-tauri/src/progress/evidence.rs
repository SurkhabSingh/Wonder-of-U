use std::collections::BTreeSet;

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct LibraryEvidence {
    pub(crate) days: BTreeSet<DayKey>,
    pub(crate) dropped: u32,
}

impl LibraryEvidence {
    pub(crate) fn earliest(&self) -> Option<&DayKey> {
        self.days.iter().next()
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
        .map(|recording| recording.created_at_ms)
        .chain(videos.iter().map(|video| video.added_at_ms));
    for stamp in stamps {
        match usable_day(stamp, today) {
            Some(day) => {
                evidence.days.insert(day);
            }
            None => evidence.dropped += 1,
        }
    }
    evidence
}

/// Dropped rather than clamped: an unreadable file's date falls back to now, and clamping
/// would draw a mark on a day nothing happened.
fn usable_day(stamp: u64, today: &DayKey) -> Option<DayKey> {
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
        assert_eq!(evidence.earliest(), evidence.days.iter().next());
    }

    #[test]
    fn nothing_recorded_has_no_horizon() {
        let evidence = collect(&[], &[], &key("2026-09-15"));
        assert_eq!(evidence.earliest(), None);
        assert_eq!(evidence.horizon(), EvidenceFrom::default());
        assert_eq!(evidence.dropped, 0);
    }
}
