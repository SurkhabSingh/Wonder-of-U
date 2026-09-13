use std::time::Duration;

use serde::{Deserialize, Serialize};

pub(crate) const MAX_CHUNK_MS: u64 = 120_000;

/// Below this a rate reads as a stall; dividing by it turns milliseconds into hours.
const MIN_RATE: f64 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ImmersionSource {
    Listening,
    Watching,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct PlaybackSample {
    pub(crate) playing: bool,
    pub(crate) position_ms: u64,
    pub(crate) rate: f64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) struct Credit {
    pub(crate) credited_ms: u64,
    pub(crate) unmeasured_ms: u64,
}

pub(crate) fn credit_ms(
    previous: Option<&PlaybackSample>,
    next: &PlaybackSample,
    wall_elapsed: Duration,
) -> Credit {
    let Some(previous) = previous else {
        return Credit::default();
    };
    if !previous.playing {
        return Credit::default();
    }

    let wall_ms = u64::try_from(wall_elapsed.as_millis()).unwrap_or(u64::MAX);
    let advance_ms = next.position_ms.saturating_sub(previous.position_ms);

    if advance_ms == 0 {
        return Credit {
            credited_ms: 0,
            unmeasured_ms: if wall_ms >= MAX_CHUNK_MS { wall_ms } else { 0 },
        };
    }

    let credited = wall_ms.min(media_elapsed_ms(advance_ms, previous.rate));
    if credited > MAX_CHUNK_MS {
        return Credit {
            credited_ms: MAX_CHUNK_MS,
            unmeasured_ms: credited - MAX_CHUNK_MS,
        };
    }
    Credit {
        credited_ms: credited,
        unmeasured_ms: 0,
    }
}

fn media_elapsed_ms(advance_ms: u64, rate: f64) -> u64 {
    let rate = if rate.is_finite() {
        rate.max(MIN_RATE)
    } else {
        MIN_RATE
    };
    let millis = advance_ms as f64 / rate;
    if millis.is_finite() && millis >= 0.0 {
        millis as u64
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn playing(position_ms: u64) -> PlaybackSample {
        PlaybackSample {
            playing: true,
            position_ms,
            rate: 1.0,
        }
    }

    fn settled(position_ms: u64) -> PlaybackSample {
        PlaybackSample {
            playing: false,
            position_ms,
            rate: 1.0,
        }
    }

    #[test]
    fn a_settle_with_no_playing_sample_before_it_credits_nothing() {
        assert_eq!(
            credit_ms(None, &settled(2_500), Duration::from_millis(2_500)),
            Credit::default()
        );
        assert_eq!(
            credit_ms(
                Some(&settled(0)),
                &settled(2_500),
                Duration::from_millis(2_500)
            ),
            Credit::default()
        );
    }

    #[test]
    fn a_two_and_a_half_second_clip_credits_two_and_a_half_seconds() {
        let credit = credit_ms(Some(&playing(0)), &settled(2_500), Duration::from_millis(2_500));
        assert_eq!(credit.credited_ms, 2_500);
        assert_eq!(credit.unmeasured_ms, 0);
    }

    #[test]
    fn a_long_gap_with_no_advance_credits_nothing_and_reports_the_whole_gap() {
        let credit = credit_ms(
            Some(&playing(1_000)),
            &playing(1_000),
            Duration::from_secs(9 * 60 * 60),
        );
        assert_eq!(credit.credited_ms, 0);
        assert_eq!(credit.unmeasured_ms, 9 * 60 * 60 * 1_000);
    }

    #[test]
    fn a_short_gap_with_no_advance_reports_nothing_either_way() {
        assert_eq!(
            credit_ms(Some(&playing(1_000)), &playing(1_000), Duration::from_secs(5)),
            Credit::default()
        );
    }

    #[test]
    fn a_throttled_minute_credits_the_minute() {
        let credit = credit_ms(Some(&playing(0)), &playing(60_000), Duration::from_secs(60));
        assert_eq!(credit.credited_ms, 60_000);
        assert_eq!(credit.unmeasured_ms, 0);
    }

    #[test]
    fn double_and_half_speed_both_credit_wall_time() {
        let fast = PlaybackSample {
            playing: true,
            position_ms: 0,
            rate: 2.0,
        };
        assert_eq!(
            credit_ms(Some(&fast), &playing(120_000), Duration::from_secs(60)).credited_ms,
            60_000,
            "2x must not credit the media's 120s"
        );

        let slow = PlaybackSample {
            playing: true,
            position_ms: 0,
            rate: 0.5,
        };
        assert_eq!(
            credit_ms(Some(&slow), &playing(30_000), Duration::from_secs(60)).credited_ms,
            60_000,
            "0.5x must not credit only the media's 30s"
        );
    }

    #[test]
    fn a_five_minute_seek_inside_one_tick_credits_one_tick() {
        let credit = credit_ms(Some(&playing(0)), &playing(300_000), Duration::from_secs(1));
        assert_eq!(credit.credited_ms, 1_000);
    }

    #[test]
    fn a_gap_beyond_the_bound_credits_the_bound_and_reports_the_rest() {
        let credit = credit_ms(Some(&playing(0)), &playing(600_000), Duration::from_secs(600));
        assert_eq!(credit.credited_ms, MAX_CHUNK_MS);
        assert_eq!(credit.unmeasured_ms, 600_000 - MAX_CHUNK_MS);
    }

    #[test]
    fn a_stretch_that_barely_advanced_credits_the_advance_not_the_wall() {
        let credit = credit_ms(Some(&playing(0)), &playing(5_000), Duration::from_secs(60));
        assert_eq!(credit.credited_ms, 5_000, "only the media that actually played");
    }

    #[test]
    fn a_crawling_rate_is_floored_rather_than_covering_the_gap() {
        let crawling = PlaybackSample {
            playing: true,
            position_ms: 0,
            rate: 0.001,
        };
        let credit = credit_ms(Some(&crawling), &playing(10), Duration::from_secs(1));
        assert_eq!(credit.credited_ms, 200, "10ms at the floor of 0.05x, not a full second");
    }

    #[test]
    fn an_impossible_rate_cannot_turn_a_moment_into_hours() {
        for rate in [0.0, -1.0, f64::NAN, f64::INFINITY] {
            let stalled = PlaybackSample {
                playing: true,
                position_ms: 0,
                rate,
            };
            let credit = credit_ms(Some(&stalled), &playing(10), Duration::from_secs(1));
            assert!(credit.credited_ms <= 1_000, "rate {rate} credited {credit:?}");
        }
    }

    #[test]
    fn the_source_names_are_the_ones_the_frontend_sends() {
        let listening: ImmersionSource =
            serde_json::from_str(r#""listening""#).expect("listening");
        assert_eq!(listening, ImmersionSource::Listening);
        let watching: ImmersionSource = serde_json::from_str(r#""watching""#).expect("watching");
        assert_eq!(watching, ImmersionSource::Watching);
        assert!(serde_json::from_str::<ImmersionSource>(r#""Listening""#).is_err());
    }
}
