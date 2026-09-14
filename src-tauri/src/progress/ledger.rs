use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use super::credit::{Credit, ImmersionSource};
use super::day::DayKey;

/// What a day needs before it counts as active, unless something was mined on it.
pub(crate) const ACTIVE_MS: u64 = 60_000;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DayTotals {
    #[serde(default)]
    pub(crate) listening_ms: u64,
    #[serde(default)]
    pub(crate) watching_ms: u64,
    /// Time both sources ran at once, booked by the listening side alone.
    #[serde(default)]
    pub(crate) overlap_ms: u64,
    #[serde(default)]
    pub(crate) unmeasured_ms: u64,
    #[serde(default)]
    pub(crate) mined: bool,
}

impl DayTotals {
    /// Two sources cannot have run together for longer than the shorter of them ran at
    /// all, so a day can never come out below the surface that measured the most.
    pub(crate) fn combined_ms(&self) -> u64 {
        let shared = self.overlap_ms.min(self.listening_ms.min(self.watching_ms));
        self.listening_ms
            .saturating_add(self.watching_ms)
            .saturating_sub(shared)
    }

    pub(crate) fn is_active(&self) -> bool {
        self.combined_ms() >= ACTIVE_MS || self.mined
    }

    fn add(&mut self, other: &DayTotals) {
        self.listening_ms = self.listening_ms.saturating_add(other.listening_ms);
        self.watching_ms = self.watching_ms.saturating_add(other.watching_ms);
        self.overlap_ms = self.overlap_ms.saturating_add(other.overlap_ms);
        self.unmeasured_ms = self.unmeasured_ms.saturating_add(other.unmeasured_ms);
        self.mined |= other.mined;
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Ledger {
    days: BTreeMap<DayKey, DayTotals>,
}

impl Ledger {
    pub(crate) const fn new() -> Self {
        Self {
            days: BTreeMap::new(),
        }
    }

    /// `other_source_live` is the caller's answer to whether the other surface was playing
    /// at the same moment; only it can see both cursors.
    pub(crate) fn credit(
        &mut self,
        day: &DayKey,
        source: ImmersionSource,
        credit: Credit,
        other_source_live: bool,
    ) {
        let totals = self.days.entry(day.clone()).or_default();
        let field = match source {
            ImmersionSource::Listening => &mut totals.listening_ms,
            ImmersionSource::Watching => &mut totals.watching_ms,
        };
        *field = field.saturating_add(credit.credited_ms);
        totals.unmeasured_ms = totals.unmeasured_ms.saturating_add(credit.unmeasured_ms);
        if other_source_live {
            totals.overlap_ms = totals.overlap_ms.saturating_add(credit.credited_ms);
        }
    }

    pub(crate) fn mark_mined(&mut self, day: &DayKey) {
        self.days.entry(day.clone()).or_default().mined = true;
    }

    /// Adds `other` into this one. Every flush is a read-modify-write, so two writers of
    /// the same day accumulate instead of one overwriting the other.
    pub(crate) fn merge(&mut self, other: &Ledger) {
        for (day, totals) in &other.days {
            self.days.entry(day.clone()).or_default().add(totals);
        }
    }

    /// Drops days the store cannot speak for, and reports how many went.
    pub(crate) fn floor_at(&mut self, first_run_day: &DayKey) -> usize {
        let before = self.days.len();
        self.days.retain(|day, _| day >= first_run_day);
        before - self.days.len()
    }

    pub(crate) fn set(&mut self, day: DayKey, totals: DayTotals) {
        self.days.insert(day, totals);
    }

    pub(crate) fn get(&self, day: &DayKey) -> Option<&DayTotals> {
        self.days.get(day)
    }

    pub(crate) fn len(&self) -> usize {
        self.days.len()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.days.is_empty()
    }

    pub(crate) fn iter(&self) -> impl Iterator<Item = (&DayKey, &DayTotals)> {
        self.days.iter()
    }

    pub(crate) fn active_days(&self) -> usize {
        self.days.values().filter(|totals| totals.is_active()).count()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(day: &str) -> DayKey {
        serde_json::from_str(&format!("\"{day}\"")).expect("a day key is a string")
    }

    fn credited(ms: u64) -> Credit {
        Credit {
            credited_ms: ms,
            unmeasured_ms: 0,
        }
    }

    #[test]
    fn two_credits_on_one_day_add_rather_than_replace() {
        let mut ledger = Ledger::default();
        let today = key("2026-09-14");
        ledger.credit(&today, ImmersionSource::Listening, credited(60_000), false);
        ledger.credit(&today, ImmersionSource::Listening, credited(30_000), false);
        assert_eq!(ledger.len(), 1);
        assert_eq!(ledger.get(&today).unwrap().listening_ms, 90_000);
    }

    #[test]
    fn each_day_gets_its_own_row() {
        let mut ledger = Ledger::default();
        ledger.credit(&key("2026-09-14"), ImmersionSource::Listening, credited(60_000), false);
        ledger.credit(&key("2026-09-15"), ImmersionSource::Listening, credited(60_000), false);
        assert_eq!(ledger.len(), 2);
    }

    #[test]
    fn the_two_surfaces_are_counted_apart() {
        let mut ledger = Ledger::default();
        let today = key("2026-09-14");
        ledger.credit(&today, ImmersionSource::Listening, credited(60_000), false);
        ledger.credit(&today, ImmersionSource::Watching, credited(90_000), false);
        let totals = ledger.get(&today).unwrap();
        assert_eq!(totals.listening_ms, 60_000);
        assert_eq!(totals.watching_ms, 90_000);
        assert_eq!(totals.combined_ms(), 150_000);
    }

    /// mpv keeps playing when the app is brought forward, and two live sources would
    /// otherwise sum to more attention than the clock allows.
    #[test]
    fn time_both_sources_were_live_is_counted_once() {
        let mut ledger = Ledger::default();
        let today = key("2026-09-14");
        ledger.credit(&today, ImmersionSource::Watching, credited(60_000), false);
        ledger.credit(&today, ImmersionSource::Listening, credited(60_000), true);
        let totals = ledger.get(&today).unwrap();
        assert_eq!(totals.listening_ms + totals.watching_ms, 120_000);
        assert_eq!(totals.overlap_ms, 60_000);
        assert_eq!(totals.combined_ms(), 60_000);
    }

    /// Overlap against a surface that measured nothing is not a shared stretch, and
    /// subtracting it would throw away time the app did measure.
    #[test]
    fn overlap_never_costs_more_than_the_smaller_surface_measured() {
        let mut ledger = Ledger::default();
        let today = key("2026-09-14");
        ledger.credit(&today, ImmersionSource::Listening, credited(1_000), true);
        ledger.credit(&today, ImmersionSource::Listening, credited(1_000), true);
        let totals = ledger.get(&today).unwrap();
        assert_eq!(totals.overlap_ms, 2_000);
        assert_eq!(totals.watching_ms, 0);
        assert_eq!(
            totals.combined_ms(),
            2_000,
            "there was nothing to overlap with"
        );
    }

    /// One stretch read by two instruments is still one stretch. Subtracted once per
    /// reader, an hour spent on both at once came out as twelve seconds.
    #[test]
    fn a_stretch_booked_by_both_sides_is_still_subtracted_once() {
        let mut ledger = Ledger::default();
        let today = key("2026-09-14");
        ledger.credit(&today, ImmersionSource::Watching, credited(60_000), true);
        ledger.credit(&today, ImmersionSource::Listening, credited(60_000), true);
        let totals = ledger.get(&today).unwrap();
        assert_eq!(totals.overlap_ms, 120_000, "both sides booked it");
        assert_eq!(totals.combined_ms(), 60_000);
    }

    #[test]
    fn unmeasured_time_accumulates_without_being_credited() {
        let mut ledger = Ledger::default();
        let today = key("2026-09-14");
        ledger.credit(
            &today,
            ImmersionSource::Listening,
            Credit {
                credited_ms: 0,
                unmeasured_ms: 9 * 60 * 60 * 1_000,
            },
            false,
        );
        let totals = ledger.get(&today).unwrap();
        assert_eq!(totals.combined_ms(), 0);
        assert_eq!(totals.unmeasured_ms, 9 * 60 * 60 * 1_000);
    }

    #[test]
    fn a_day_is_active_at_a_minute_or_on_a_mining_act() {
        let mut ledger = Ledger::default();
        let quiet = key("2026-09-14");
        ledger.credit(&quiet, ImmersionSource::Listening, credited(59_999), false);
        assert!(!ledger.get(&quiet).unwrap().is_active());

        ledger.credit(&quiet, ImmersionSource::Listening, credited(1), false);
        assert!(ledger.get(&quiet).unwrap().is_active());

        let mined = key("2026-09-15");
        ledger.mark_mined(&mined);
        assert_eq!(ledger.get(&mined).unwrap().combined_ms(), 0);
        assert!(ledger.get(&mined).unwrap().is_active(), "a mined day counts");
        assert_eq!(ledger.active_days(), 2);
    }

    #[test]
    fn a_mining_act_cannot_be_undone_by_a_later_merge() {
        let mut ledger = Ledger::default();
        let today = key("2026-09-14");
        ledger.mark_mined(&today);
        let mut later = Ledger::default();
        later.credit(&today, ImmersionSource::Listening, credited(1_000), false);
        ledger.merge(&later);
        assert!(ledger.get(&today).unwrap().mined);
    }

    #[test]
    fn a_mining_act_arriving_in_a_merge_marks_the_day() {
        let mut ledger = Ledger::default();
        let today = key("2026-09-14");
        ledger.credit(&today, ImmersionSource::Listening, credited(1_000), false);
        assert!(!ledger.get(&today).unwrap().mined);

        let mut later = Ledger::default();
        later.mark_mined(&today);
        ledger.merge(&later);
        assert!(ledger.get(&today).unwrap().mined, "the act came in with the merge");
    }

    #[test]
    fn merging_adds_every_field() {
        let mut left = Ledger::default();
        let today = key("2026-09-14");
        left.credit(&today, ImmersionSource::Listening, credited(60_000), false);

        let mut right = Ledger::default();
        right.credit(&today, ImmersionSource::Watching, credited(30_000), true);
        right.credit(
            &today,
            ImmersionSource::Listening,
            Credit {
                credited_ms: 0,
                unmeasured_ms: 5_000,
            },
            false,
        );

        left.merge(&right);
        let totals = left.get(&today).unwrap();
        assert_eq!(totals.listening_ms, 60_000);
        assert_eq!(totals.watching_ms, 30_000);
        assert_eq!(totals.overlap_ms, 30_000);
        assert_eq!(totals.unmeasured_ms, 5_000);
    }

    /// The store cannot speak for days before it existed, and a row claiming one would be
    /// a number nobody measured.
    #[test]
    fn days_before_the_store_existed_are_dropped() {
        let mut ledger = Ledger::default();
        ledger.credit(&key("2026-09-12"), ImmersionSource::Listening, credited(1), false);
        ledger.credit(&key("2026-09-13"), ImmersionSource::Listening, credited(1), false);
        ledger.credit(&key("2026-09-14"), ImmersionSource::Listening, credited(1), false);

        assert_eq!(ledger.floor_at(&key("2026-09-13")), 1);
        assert_eq!(ledger.len(), 2);
        assert!(ledger.get(&key("2026-09-12")).is_none());
        assert!(ledger.get(&key("2026-09-13")).is_some(), "the first day is kept");
    }

    #[test]
    fn totals_round_trip_through_json_under_the_names_the_rows_carry() {
        let totals = DayTotals {
            listening_ms: 1,
            watching_ms: 2,
            overlap_ms: 3,
            unmeasured_ms: 4,
            mined: true,
        };
        let text = serde_json::to_string(&totals).expect("totals serialise");
        assert!(text.contains("\"listeningMs\":1"), "{text}");
        assert!(text.contains("\"unmeasuredMs\":4"), "{text}");
        assert_eq!(
            serde_json::from_str::<DayTotals>(&text).expect("totals parse"),
            totals
        );
    }

    /// A row written before a field existed still reads, as every stored row must.
    #[test]
    fn a_row_missing_the_newer_fields_reads_as_zero_rather_than_failing() {
        let totals: DayTotals =
            serde_json::from_str(r#"{"listeningMs":60000}"#).expect("a sparse row still reads");
        assert_eq!(totals.listening_ms, 60_000);
        assert_eq!(totals.watching_ms, 0);
        assert!(!totals.mined);
    }
}
