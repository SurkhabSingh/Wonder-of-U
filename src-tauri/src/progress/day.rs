use chrono::{DateTime, Duration, Local, LocalResult, TimeZone};
use serde::{Deserialize, Serialize};

/// The hour at which a new day begins, in local time.
///
/// Not midnight. A session that ends at 00:30 belongs to the evening it started in, and
/// keyed by local midnight a streak would break for someone who studied at 23:30 and again
/// at 00:30 — two sessions in one sitting, reported as a missed day.
///
/// A constant rather than a setting, because the key is written into stored rows: changing
/// it later could not re-bucket the history already keyed under the old one, so the choice
/// has to be made once.
pub(crate) const DAY_ROLLOVER_HOUR: i64 = 4;

/// The local day a moment belongs to, as `YYYY-MM-DD`.
///
/// A newtype with one constructor so that every row in the store, every chart bucket and
/// every streak comparison is keyed the same way. Two pieces of code deciding
/// independently what "today" means is how a streak breaks for a reason nobody can find.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct DayKey(String);

impl DayKey {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DayKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// The day a local moment falls in, once the rollover is applied.
///
/// Shifting the moment back and then taking its date is what makes the rule one
/// subtraction rather than a branch on the hour, so the boundary cannot be off by one in
/// only one direction.
pub(crate) fn day_key_at(moment: DateTime<Local>) -> DayKey {
    let shifted = moment - Duration::hours(DAY_ROLLOVER_HOUR);
    DayKey(shifted.format("%Y-%m-%d").to_string())
}

/// The day a millisecond timestamp falls in, or `None` when it names no local time at all.
///
/// `None` is reachable: the hour skipped by a daylight-saving jump does not exist locally,
/// and a timestamp beyond the calendar's range has no date. A caller must decide what to do
/// about that rather than be handed a plausible neighbouring day.
pub(crate) fn day_key_for_ms(ms: u64) -> Option<DayKey> {
    let millis = i64::try_from(ms).ok()?;
    match Local.timestamp_millis_opt(millis) {
        LocalResult::Single(moment) => Some(day_key_at(moment)),
        // A repeated local hour names one calendar date either way, so the ambiguity does
        // not reach the answer; the earlier reading is taken so the choice is stated.
        LocalResult::Ambiguous(earlier, _) => Some(day_key_at(earlier)),
        LocalResult::None => None,
    }
}

/// The day now.
pub(crate) fn today() -> DayKey {
    day_key_at(Local::now())
}

#[cfg(test)]
mod tests {
    use super::{day_key_at, day_key_for_ms, DayKey, DAY_ROLLOVER_HOUR};
    use chrono::{Local, LocalResult, TimeZone};

    fn local(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> chrono::DateTime<Local> {
        match Local.with_ymd_and_hms(year, month, day, hour, minute, 0) {
            LocalResult::Single(moment) => moment,
            LocalResult::Ambiguous(earlier, _) => earlier,
            LocalResult::None => panic!("{year}-{month}-{day} {hour}:{minute} is not a local time"),
        }
    }

    /// The whole point of the rollover, and the case a streak lives or dies on.
    #[test]
    fn the_minute_before_the_rollover_belongs_to_the_day_before() {
        assert_eq!(day_key_at(local(2026, 9, 10, 3, 59)).as_str(), "2026-09-09");
        assert_eq!(day_key_at(local(2026, 9, 10, 4, 0)).as_str(), "2026-09-10");
    }

    /// A late-night session and the small hours after it are one day, which is the reason
    /// the rollover exists at all.
    #[test]
    fn a_session_either_side_of_midnight_is_one_day() {
        let evening = day_key_at(local(2026, 9, 9, 23, 30));
        let after_midnight = day_key_at(local(2026, 9, 10, 0, 30));
        assert_eq!(evening, after_midnight);
        assert_eq!(evening.as_str(), "2026-09-09");
    }

    #[test]
    fn ordinary_daytime_moments_land_on_their_own_date() {
        assert_eq!(day_key_at(local(2026, 9, 10, 12, 0)).as_str(), "2026-09-10");
        assert_eq!(day_key_at(local(2026, 9, 10, 23, 59)).as_str(), "2026-09-10");
    }

    /// Whatever the machine's zone, a calendar date must produce exactly one key across
    /// its whole span — including a date on which the clocks moved.
    #[test]
    fn one_calendar_date_yields_one_key_across_its_whole_span() {
        for (year, month, day) in [(2026, 3, 29), (2026, 10, 25), (2026, 9, 10)] {
            let mut keys = std::collections::BTreeSet::new();
            for hour in 4..24 {
                if let LocalResult::Single(moment) =
                    Local.with_ymd_and_hms(year, month, day, hour, 0, 0)
                {
                    keys.insert(day_key_at(moment));
                }
            }
            assert_eq!(
                keys.len(),
                1,
                "{year}-{month}-{day} produced {} keys: {keys:?}",
                keys.len()
            );
        }
    }

    #[test]
    fn a_timestamp_resolves_to_the_same_key_as_the_moment_it_names() {
        let moment = local(2026, 9, 10, 2, 15);
        let from_ms = day_key_for_ms(u64::try_from(moment.timestamp_millis()).expect("positive"));
        assert_eq!(from_ms.as_deref_key(), Some("2026-09-09"));
    }

    #[test]
    fn a_timestamp_beyond_the_calendar_has_no_day_rather_than_a_wrong_one() {
        assert_eq!(day_key_for_ms(u64::MAX), None);
    }

    #[test]
    fn the_rollover_is_the_documented_four() {
        // Pinned because the value is baked into every stored row: the constant and the
        // history keyed under it cannot disagree later.
        assert_eq!(DAY_ROLLOVER_HOUR, 4);
    }

    trait KeyText {
        fn as_deref_key(&self) -> Option<&str>;
    }

    impl KeyText for Option<DayKey> {
        fn as_deref_key(&self) -> Option<&str> {
            self.as_ref().map(DayKey::as_str)
        }
    }
}
