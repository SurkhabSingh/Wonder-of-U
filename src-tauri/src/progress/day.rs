use chrono::{DateTime, Duration, Local, LocalResult, TimeZone};
use serde::{Deserialize, Serialize};

/// When a new day begins locally. Not midnight: 23:30 and 00:30 are one sitting, and a
/// midnight key reports them as a missed day. Constant, because stored rows are keyed by it.
pub(crate) const DAY_ROLLOVER_HOUR: i64 = 4;

/// The local day a moment belongs to, as `YYYY-MM-DD`. One constructor, so rows, buckets
/// and streaks cannot disagree about what "today" means.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub(crate) struct DayKey(String);

impl DayKey {
    pub(crate) fn as_str(&self) -> &str {
        &self.0
    }

    /// The day before this one, derived from a key that already exists rather than from a
    /// clock, so the rollover cannot be applied a second time.
    pub(crate) fn previous(&self) -> Option<DayKey> {
        let date = chrono::NaiveDate::parse_from_str(&self.0, "%Y-%m-%d").ok()?;
        let earlier = date.checked_sub_days(chrono::Days::new(1))?;
        Some(DayKey(earlier.format("%Y-%m-%d").to_string()))
    }
}

impl std::fmt::Display for DayKey {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(self.as_str())
    }
}

pub(crate) fn day_key_at(moment: DateTime<Local>) -> DayKey {
    let shifted = moment - Duration::hours(DAY_ROLLOVER_HOUR);
    DayKey(shifted.format("%Y-%m-%d").to_string())
}

pub(crate) fn day_key_for_ms(ms: u64) -> Option<DayKey> {
    let millis = i64::try_from(ms).ok()?;
    match Local.timestamp_millis_opt(millis) {
        LocalResult::Single(moment) => Some(day_key_at(moment)),
        LocalResult::Ambiguous(earlier, _) => Some(day_key_at(earlier)),
        LocalResult::None => None,
    }
}

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

    #[test]
    fn the_minute_before_the_rollover_belongs_to_the_day_before() {
        assert_eq!(day_key_at(local(2026, 9, 10, 3, 59)).as_str(), "2026-09-09");
        assert_eq!(day_key_at(local(2026, 9, 10, 4, 0)).as_str(), "2026-09-10");
    }

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
        let millis = u64::try_from(moment.timestamp_millis()).expect("a positive timestamp");
        assert_eq!(
            day_key_for_ms(millis).as_ref().map(DayKey::as_str),
            Some("2026-09-09"),
            "02:15 belongs to the evening before"
        );
    }

    #[test]
    fn a_timestamp_beyond_the_calendar_has_no_day_rather_than_a_wrong_one() {
        assert_eq!(day_key_for_ms(u64::MAX), None);
    }

    #[test]
    fn the_rollover_is_the_documented_four() {
        assert_eq!(DAY_ROLLOVER_HOUR, 4);
    }
}
