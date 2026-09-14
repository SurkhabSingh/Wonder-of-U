use std::sync::Mutex;
use std::time::Instant;

use tauri::{AppHandle, Runtime};

use super::credit::{credit_ms, Credit, ImmersionSource, PlaybackSample};
use super::day::today;
use super::ledger::Ledger;
use super::liveness;

const FLUSH_AT_MS: u64 = 30_000;

struct Listening {
    cursor: Option<(Instant, PlaybackSample)>,
    pending: Ledger,
    unflushed_ms: u64,
}

static LISTENING: Mutex<Listening> = Mutex::new(Listening {
    cursor: None,
    pending: Ledger::new(),
    unflushed_ms: 0,
});

/// `arrived` is stamped by the shim before the hop to the blocking pool, where two samples
/// microseconds apart can swap places.
pub(crate) fn record<R: Runtime>(
    app: &AppHandle<R>,
    arrived: Instant,
    playing: bool,
    position_ms: u64,
    rate: f64,
) {
    let Ok(mut state) = LISTENING.lock() else {
        return;
    };
    let sample = PlaybackSample {
        playing,
        position_ms,
        rate,
    };
    let Some(credit) = advance(&mut state.cursor, arrived, sample) else {
        return;
    };
    if credit != Credit::default() {
        let overlapping = liveness::watching_is_live();
        let day = today();
        state
            .pending
            .credit(&day, ImmersionSource::Listening, credit, overlapping);
        state.unflushed_ms = state.unflushed_ms.saturating_add(credit.credited_ms);
    }

    if should_flush(state.unflushed_ms, playing, state.pending.is_empty()) {
        state.unflushed_ms = 0;
        let Listening { pending, .. } = &mut *state;
        super::flush_days(app, pending);
    }
}

/// `None` when the sample is not newer than the cursor, which would credit backwards.
fn advance(
    cursor: &mut Option<(Instant, PlaybackSample)>,
    arrived: Instant,
    sample: PlaybackSample,
) -> Option<Credit> {
    let credit = match cursor {
        Some((at, _)) if arrived <= *at => return None,
        Some((at, previous)) => credit_ms(Some(previous), &sample, arrived - *at),
        None => Credit::default(),
    };
    *cursor = Some((arrived, sample));
    Some(credit)
}

fn should_flush(unflushed_ms: u64, playing: bool, pending_empty: bool) -> bool {
    !pending_empty && (unflushed_ms >= FLUSH_AT_MS || !playing)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn at(base: Instant, ms: u64) -> Instant {
        base + Duration::from_millis(ms)
    }

    fn playing(position_ms: u64) -> PlaybackSample {
        PlaybackSample {
            playing: true,
            position_ms,
            rate: 1.0,
        }
    }

    fn stopped(position_ms: u64) -> PlaybackSample {
        PlaybackSample {
            playing: false,
            position_ms,
            rate: 1.0,
        }
    }

    #[test]
    fn a_settle_with_no_playing_sample_before_it_credits_nothing() {
        let base = Instant::now();
        let mut cursor = None;
        assert_eq!(
            advance(&mut cursor, base, stopped(0)),
            Some(Credit::default())
        );
        let credit = advance(&mut cursor, at(base, 8_000), stopped(0)).expect("accepted");
        assert_eq!(credit, Credit::default());
    }

    #[test]
    fn a_clip_of_two_and_a_half_seconds_credits_two_and_a_half_seconds() {
        let base = Instant::now();
        let mut cursor = None;
        advance(&mut cursor, base, playing(0)).expect("opened");
        let credit = advance(&mut cursor, at(base, 2_500), stopped(2_500)).expect("settled");
        assert_eq!(credit.credited_ms, 2_500);
        assert_eq!(credit.unmeasured_ms, 0);
    }

    #[test]
    fn a_sixty_second_gap_that_advanced_sixty_seconds_credits_sixty_seconds() {
        let base = Instant::now();
        let mut cursor = None;
        advance(&mut cursor, base, playing(0)).expect("opened");
        let credit = advance(&mut cursor, at(base, 60_000), playing(60_000)).expect("heartbeat");
        assert_eq!(credit.credited_ms, 60_000);
    }

    #[test]
    fn a_sample_older_than_the_cursor_is_dropped() {
        let base = Instant::now();
        let mut cursor = None;
        advance(&mut cursor, at(base, 1_000), playing(1_000)).expect("opened");
        assert_eq!(advance(&mut cursor, at(base, 500), playing(500)), None);
        assert_eq!(advance(&mut cursor, at(base, 1_000), playing(1_000)), None);

        let (kept, sample) = cursor.expect("the cursor is still the newer sample");
        assert_eq!(kept, at(base, 1_000));
        assert_eq!(sample.position_ms, 1_000);
    }

    #[test]
    fn a_settle_is_written_without_waiting_for_the_flush_mark() {
        assert!(should_flush(200, false, false), "a settle writes at once");
        assert!(!should_flush(200, true, false), "a heartbeat waits");
        assert!(should_flush(FLUSH_AT_MS, true, false), "until the mark");
        assert!(!should_flush(200, false, true), "nothing to write");
    }
}
