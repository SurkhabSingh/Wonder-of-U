use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;
use std::time::Instant;

use super::credit::MAX_CHUNK_MS;

/// One store publishes both facts, so a reader can never pair "playing" with a different
/// sample's stamp: the top bit is playing, the rest milliseconds since `base`.
const PLAYING: u64 = 1 << 63;

static WATCHING: AtomicU64 = AtomicU64::new(0);

fn base() -> Instant {
    static BASE: OnceLock<Instant> = OnceLock::new();
    *BASE.get_or_init(Instant::now)
}

fn now_ms() -> u64 {
    u64::try_from(base().elapsed().as_millis()).unwrap_or(u64::MAX) & !PLAYING
}

pub(crate) fn mark_watching(playing: bool) {
    WATCHING.store(now_ms() | if playing { PLAYING } else { 0 }, Ordering::SeqCst);
}

/// Whether mpv was playing recently enough to be sharing the stretch the audio side is
/// about to credit. Only that side books the overlap: booked from both, one stretch would
/// be subtracted twice. A player nobody has marked is not live.
pub(crate) fn watching_is_live() -> bool {
    is_live(WATCHING.load(Ordering::SeqCst), now_ms())
}

fn is_live(packed: u64, now_ms: u64) -> bool {
    if packed & PLAYING == 0 {
        return false;
    }
    now_ms.saturating_sub(packed & !PLAYING) <= MAX_CHUNK_MS
}

/// The two surfaces are process-wide. Any test that marks one holds this, or a parallel
/// test reads a flag it did not set.
#[cfg(test)]
pub(crate) fn test_gate() -> std::sync::MutexGuard<'static, ()> {
    static GATE: std::sync::Mutex<()> = std::sync::Mutex::new(());
    GATE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_surface_nobody_marked_is_not_live() {
        assert!(!is_live(0, 10_000));
    }

    #[test]
    fn a_surface_marked_not_playing_is_not_live() {
        assert!(!is_live(5_000, 5_000), "a fresh stamp is not enough");
    }

    #[test]
    fn a_surface_playing_now_is_live() {
        assert!(is_live(PLAYING | 5_000, 5_100));
    }

    /// A player the app stopped hearing from cannot go on claiming the user's attention.
    #[test]
    fn a_mark_older_than_the_cap_is_not_live() {
        assert!(is_live(PLAYING | 1_000, 1_000 + MAX_CHUNK_MS));
        assert!(!is_live(PLAYING | 1_000, 1_000 + MAX_CHUNK_MS + 1));
    }

    #[test]
    fn the_player_is_live_only_while_it_says_it_is_playing() {
        let _gate = test_gate();
        mark_watching(true);
        assert!(watching_is_live());

        mark_watching(false);
        assert!(!watching_is_live());
    }
}
