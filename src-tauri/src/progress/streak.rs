use serde::Serialize;

use super::day::DayKey;
use super::ledger::Ledger;

const WEEK_DAYS: usize = 7;
const RECENT_DAYS: usize = 30;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StreakReport {
    pub(crate) current: usize,
    pub(crate) longest: usize,
    pub(crate) active_days_30: usize,
    pub(crate) today_counted: bool,
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ImmersionReport {
    pub(crate) today_ms: u64,
    pub(crate) week_ms: u64,
    pub(crate) today_unmeasured_ms: u64,
    pub(crate) counted_from: DayKey,
    pub(crate) streak: StreakReport,
}

pub(crate) fn summarise(days: &Ledger, today: &DayKey, counted_from: &DayKey) -> ImmersionReport {
    let series = active_series(days, today, counted_from);
    let today_totals = days.get(today);
    ImmersionReport {
        today_ms: today_totals.map_or(0, |totals| totals.combined_ms()),
        week_ms: recent_ms(days, today, counted_from, WEEK_DAYS),
        today_unmeasured_ms: today_totals.map_or(0, |totals| totals.unmeasured_ms),
        counted_from: counted_from.clone(),
        streak: StreakReport {
            current: current_run(&series),
            longest: longest_run(&series),
            active_days_30: series.iter().rev().take(RECENT_DAYS).filter(|on| **on).count(),
            today_counted: *series.last().unwrap_or(&false),
        },
    }
}

/// Oldest first. Days before the store existed are absent rather than false: nobody was
/// counting, so they can neither break a streak nor extend one.
fn active_series(days: &Ledger, today: &DayKey, counted_from: &DayKey) -> Vec<bool> {
    let mut series = Vec::new();
    let mut cursor = Some(today.clone());
    while let Some(day) = cursor {
        if day < *counted_from {
            break;
        }
        series.push(days.get(&day).is_some_and(|totals| totals.is_active()));
        cursor = day.previous();
    }
    series.reverse();
    series
}

fn recent_ms(days: &Ledger, today: &DayKey, counted_from: &DayKey, span: usize) -> u64 {
    let mut total = 0_u64;
    let mut cursor = Some(today.clone());
    for _ in 0..span {
        let Some(day) = cursor else { break };
        if day < *counted_from {
            break;
        }
        if let Some(totals) = days.get(&day) {
            total = total.saturating_add(totals.combined_ms());
        }
        cursor = day.previous();
    }
    total
}

/// A run ending today, or ending yesterday when today is still empty — otherwise every
/// streak would read zero each morning until the first thing was played.
fn current_run(series: &[bool]) -> usize {
    let end = match series.last() {
        Some(true) => series.len(),
        _ => series.len().saturating_sub(1),
    };
    series[..end].iter().rev().take_while(|on| **on).count()
}

fn longest_run(series: &[bool]) -> usize {
    let mut longest = 0;
    let mut run = 0;
    for on in series {
        run = if *on { run + 1 } else { 0 };
        longest = longest.max(run);
    }
    longest
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::super::credit::{Credit, ImmersionSource};
    use super::super::ledger::ACTIVE_MS;

    fn key(day: &str) -> DayKey {
        serde_json::from_str(&format!("\"{day}\"")).expect("a day key is a string")
    }

    fn ledger_of(active_days: &[&str], minutes: u64) -> Ledger {
        let mut ledger = Ledger::default();
        for day in active_days {
            ledger.credit(
                &key(day),
                ImmersionSource::Watching,
                Credit {
                    credited_ms: minutes,
                    unmeasured_ms: 0,
                },
                false,
            );
        }
        ledger
    }

    #[test]
    fn the_day_before_a_key_is_the_day_before_it() {
        assert_eq!(key("2026-03-01").previous(), Some(key("2026-02-28")));
        assert_eq!(key("2026-01-01").previous(), Some(key("2025-12-31")));
    }

    #[test]
    fn an_unbroken_run_ending_today_is_the_current_streak() {
        assert_eq!(current_run(&[true, true, true]), 3);
        assert_eq!(longest_run(&[true, true, true]), 3);
    }

    #[test]
    fn a_streak_does_not_read_zero_before_the_day_has_started() {
        assert_eq!(current_run(&[true, true, false]), 2);
    }

    #[test]
    fn two_days_missed_ends_the_run() {
        assert_eq!(current_run(&[true, true, false, false]), 0);
        assert_eq!(longest_run(&[true, true, false, false]), 2);
    }

    #[test]
    fn the_longest_run_is_not_the_one_at_the_end() {
        assert_eq!(longest_run(&[true, true, true, false, true]), 3);
        assert_eq!(current_run(&[true, true, true, false, true]), 1);
    }

    #[test]
    fn nothing_measured_is_no_streak_rather_than_a_broken_one() {
        assert_eq!(current_run(&[]), 0);
        assert_eq!(longest_run(&[]), 0);
        assert_eq!(current_run(&[false]), 0);
    }

    #[test]
    fn a_run_reaching_the_horizon_is_not_broken_by_it() {
        let ledger = ledger_of(&["2026-09-13", "2026-09-14", "2026-09-15"], ACTIVE_MS);
        let report = summarise(&ledger, &key("2026-09-15"), &key("2026-09-13"));
        assert_eq!(report.streak.current, 3);
        assert_eq!(report.streak.longest, 3);
        assert!(report.streak.today_counted);
    }

    #[test]
    fn a_day_under_the_threshold_does_not_count() {
        let ledger = ledger_of(&["2026-09-15"], ACTIVE_MS - 1);
        let report = summarise(&ledger, &key("2026-09-15"), &key("2026-09-10"));
        assert_eq!(report.streak.current, 0);
        assert!(!report.streak.today_counted);
    }

    #[test]
    fn the_week_covers_seven_days_and_stops_at_the_horizon() {
        let ledger = ledger_of(
            &[
                "2026-09-09",
                "2026-09-10",
                "2026-09-11",
                "2026-09-12",
                "2026-09-13",
                "2026-09-14",
                "2026-09-15",
            ],
            60_000,
        );
        let whole = summarise(&ledger, &key("2026-09-15"), &key("2026-09-01"));
        assert_eq!(whole.week_ms, 7 * 60_000);
        assert_eq!(whole.today_ms, 60_000);

        let floored = summarise(&ledger, &key("2026-09-15"), &key("2026-09-13"));
        assert_eq!(floored.week_ms, 3 * 60_000, "only what the store can speak for");
    }

    fn consecutive_days_back_from(last: &str, count: usize) -> Vec<DayKey> {
        let mut days = vec![key(last)];
        while days.len() < count {
            days.push(days.last().expect("seeded").previous().expect("a real date"));
        }
        days
    }

    /// Read newest-first, a dead streak would read as a live one.
    #[test]
    fn the_series_runs_oldest_first() {
        let ledger = ledger_of(&["2026-09-14", "2026-09-15"], ACTIVE_MS);
        let report = summarise(&ledger, &key("2026-09-15"), &key("2026-09-12"));
        assert_eq!(report.streak.current, 2);
        assert_eq!(report.streak.longest, 2);
    }

    #[test]
    fn the_week_is_seven_days_and_not_an_eighth() {
        let days = consecutive_days_back_from("2026-09-15", 8);
        let mut ledger = Ledger::default();
        for day in &days {
            ledger.credit(
                day,
                ImmersionSource::Watching,
                Credit {
                    credited_ms: 60_000,
                    unmeasured_ms: 0,
                },
                false,
            );
        }
        let report = summarise(&ledger, &key("2026-09-15"), &key("2026-01-01"));
        assert_eq!(report.week_ms, 7 * 60_000, "the eighth day is not this week");
    }

    #[test]
    fn the_recent_count_is_thirty_days_and_not_a_thirty_first() {
        let days = consecutive_days_back_from("2026-09-15", RECENT_DAYS + 1);
        let mut ledger = Ledger::default();
        for day in &days {
            ledger.credit(
                day,
                ImmersionSource::Watching,
                Credit {
                    credited_ms: ACTIVE_MS,
                    unmeasured_ms: 0,
                },
                false,
            );
        }
        let report = summarise(&ledger, &key("2026-09-15"), &key("2026-01-01"));
        assert_eq!(report.streak.active_days_30, RECENT_DAYS);
        assert_eq!(report.streak.longest, RECENT_DAYS + 1, "the run itself is longer");
    }

    /// The names the Progress page reads. A rename here is a tile that reads unavailable
    /// forever beside a healthy status.
    #[test]
    fn the_report_reaches_the_frontend_under_the_names_it_is_read_by() {
        let report = summarise(&ledger_of(&["2026-09-15"], 60_000), &key("2026-09-15"), &key("2026-09-15"));
        let wire = serde_json::to_value(&report).expect("serialises");
        let object = wire.as_object().expect("an object");
        let mut keys: Vec<&str> = object.keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["countedFrom", "streak", "todayMs", "todayUnmeasuredMs", "weekMs"]
        );
        let mut streak: Vec<&str> = object["streak"]
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        streak.sort_unstable();
        assert_eq!(
            streak,
            ["activeDays30", "current", "longest", "todayCounted"]
        );
    }
}
