use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use tauri::{AppHandle, Manager, Runtime};

use crate::watch::{playback_probe, Playback};

use super::credit::{credit_ms, Credit, ImmersionSource, PlaybackSample};
use super::day::today;
use super::ledger::Ledger;
use super::store::merge_days;

const TICK: Duration = Duration::from_secs(1);

/// How much credit may sit unwritten. A crash costs at most this.
const FLUSH_AT_MS: u64 = 30_000;

/// Bumped per session, so a thread from an older one ends when a newer one starts.
static GENERATION: AtomicU64 = AtomicU64::new(0);

pub(crate) fn spawn_watch_sampler<R: Runtime>(app: &AppHandle<R>) {
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        let superseded = move || GENERATION.load(Ordering::SeqCst) != generation;
        run(superseded, TICK, playback_probe, |pending| {
            flush(&app, pending);
        });
    });
}

/// One session's worth of ticks.
///
/// `superseded` is read ABOVE the probe so a session held forever still ends this thread;
/// read after it, a tick waiting on the player could never notice it was superseded.
fn run(
    superseded: impl Fn() -> bool,
    tick: Duration,
    probe: impl Fn() -> Playback,
    mut flush: impl FnMut(&mut Ledger),
) {
    let mut cursor: Option<(Instant, PlaybackSample)> = None;
    let mut pending = Ledger::default();
    let mut unflushed_ms = 0_u64;

    loop {
        if superseded() {
            break;
        }
        let arrived = Instant::now();
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(&probe)) {
            Ok(Playback::Live {
                playing,
                position_ms,
                speed,
            }) => {
                let next = PlaybackSample {
                    playing,
                    position_ms,
                    rate: speed,
                };
                if let Some((at, previous)) = cursor {
                    record(&mut pending, &mut unflushed_ms, credit_ms(Some(&previous), &next, arrived - at));
                }
                cursor = Some((arrived, next));
                if unflushed_ms >= FLUSH_AT_MS {
                    flush(&mut pending);
                    unflushed_ms = 0;
                }
            }
            // Held by another caller. The cursor keeps its stamp, so the wait becomes a
            // longer interval rather than a lost one.
            Ok(Playback::Busy) => {}
            Ok(Playback::Gone) | Err(_) => break,
        }
        std::thread::sleep(tick);
    }

    if let Some((at, previous)) = cursor.take() {
        let stopped = PlaybackSample {
            playing: false,
            position_ms: previous.position_ms,
            rate: previous.rate,
        };
        record(&mut pending, &mut unflushed_ms, credit_ms(Some(&previous), &stopped, Instant::now() - at));
    }
    if !pending.is_empty() {
        flush(&mut pending);
    }
}

fn record(pending: &mut Ledger, unflushed_ms: &mut u64, credit: Credit) {
    if credit == Credit::default() {
        return;
    }
    // Always false for now: nothing else reports playback, so no stretch can overlap.
    pending.credit(&today(), ImmersionSource::Watching, credit, false);
    *unflushed_ms = unflushed_ms.saturating_add(credit.credited_ms);
}

/// Clears the delta only on a write that landed, so a failure is retried rather than lost.
fn flush<R: Runtime>(app: &AppHandle<R>, pending: &mut Ledger) {
    let path = app
        .state::<crate::app_types::AppPathsState>()
        .progress_file
        .clone();
    match merge_days(&path, pending) {
        Ok(()) => *pending = Ledger::default(),
        Err(reason) => crate::app_runtime::log_event(
            app,
            "WARN",
            "progress.day_flush_failed",
            serde_json::json!({ "message": reason }),
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::sync::{Arc, Mutex};

    fn running() -> impl Fn() -> bool {
        || false
    }

    /// Counts probes and hands back a scripted sequence, ending on `Gone`.
    fn scripted(script: Vec<Playback>) -> (impl Fn() -> Playback, Arc<AtomicUsize>) {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let script = Arc::new(Mutex::new(script));
        let probe = move || {
            let index = seen.fetch_add(1, Ordering::SeqCst);
            script
                .lock()
                .expect("script")
                .get(index)
                .copied()
                .unwrap_or(Playback::Gone)
        };
        (probe, calls)
    }

    fn live(position_ms: u64) -> Playback {
        Playback::Live {
            playing: true,
            position_ms,
            speed: 1.0,
        }
    }

    #[test]
    fn a_gone_session_ends_the_thread() {
        let (probe, calls) = scripted(vec![Playback::Gone]);
        run(running(), Duration::ZERO, probe, |_| {});
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// The player is still there; someone else is merely reading it.
    #[test]
    fn a_busy_tick_does_not_end_the_thread() {
        let (probe, calls) = scripted(vec![
            Playback::Busy,
            Playback::Busy,
            Playback::Busy,
            Playback::Gone,
        ]);
        run(running(), Duration::ZERO, probe, |_| {});
        assert_eq!(calls.load(Ordering::SeqCst), 4, "three waits survived");
    }

    /// A newer session must end an older thread even while the player never answers.
    #[test]
    fn a_superseded_generation_ends_the_thread_without_probing() {
        let (probe, calls) = scripted(vec![Playback::Busy; 64]);
        run(|| true, Duration::ZERO, probe, |_| {});
        assert_eq!(
            calls.load(Ordering::SeqCst),
            0,
            "the check is read before the player is"
        );
    }

    /// The real spawn still supersedes: a newer session bumps the counter the older
    /// thread is comparing against.
    #[test]
    fn spawning_again_supersedes_the_generation_before_it() {
        let first = GENERATION.fetch_add(1, Ordering::SeqCst) + 1;
        let superseded = move || GENERATION.load(Ordering::SeqCst) != first;
        assert!(superseded() || GENERATION.fetch_add(1, Ordering::SeqCst) + 1 > first);
        assert!(superseded(), "a later generation ends the earlier thread");
    }

    #[test]
    fn a_panicking_probe_ends_the_thread_rather_than_the_process() {
        let calls = Arc::new(AtomicUsize::new(0));
        let seen = Arc::clone(&calls);
        let probe = move || -> Playback {
            seen.fetch_add(1, Ordering::SeqCst);
            panic!("the player went away mid-read");
        };
        run(running(), Duration::ZERO, probe, |_| {});
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }

    /// Whatever a session earned has to reach the store when it ends, not only when it
    /// crosses the flush mark.
    #[test]
    fn ending_writes_what_the_session_earned() {
        let (probe, _) = scripted(vec![live(0), live(60_000), Playback::Gone]);
        let written = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&written);
        // A real tick, so wall time actually passes: the rule credits the smaller of
        // wall and media, and a zero tick earns nothing however far the media moved.
        run(running(), Duration::from_millis(30), probe, move |pending| {
            sink.lock()
                .expect("sink")
                .push(pending.get(&today()).copied());
            *pending = Ledger::default();
        });
        let flushes = written.lock().expect("sink");
        assert_eq!(flushes.len(), 1, "one write, at the end");
        let totals = flushes[0].expect("the day is there");
        assert!(totals.watching_ms > 0, "the stretch was credited");
    }

    /// A wait is a longer interval, not a lost one: the cursor has to survive it or the
    /// stretch either side of the wait is thrown away.
    #[test]
    fn a_busy_tick_does_not_throw_away_the_stretch_around_it() {
        let (probe, _) = scripted(vec![live(0), Playback::Busy, live(60_000), Playback::Gone]);
        let written = Arc::new(Mutex::new(Vec::new()));
        let sink = Arc::clone(&written);
        run(running(), Duration::from_millis(20), probe, move |pending| {
            sink.lock()
                .expect("sink")
                .push(pending.get(&today()).copied());
            *pending = Ledger::default();
        });
        let flushes = written.lock().expect("sink");
        let totals = flushes
            .first()
            .and_then(|entry| *entry)
            .expect("the day was written");
        assert!(
            totals.watching_ms >= 20,
            "the wait ended a stretch instead of joining it: {totals:?}"
        );
    }

    #[test]
    fn a_session_that_never_played_writes_nothing() {
        let (probe, _) = scripted(vec![Playback::Gone]);
        let flushes = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&flushes);
        run(running(), Duration::ZERO, probe, move |_| {
            counted.fetch_add(1, Ordering::SeqCst);
        });
        assert_eq!(flushes.load(Ordering::SeqCst), 0);
    }
}
