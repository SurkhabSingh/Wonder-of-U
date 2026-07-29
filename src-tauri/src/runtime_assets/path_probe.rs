use std::{
    sync::Mutex,
    time::{Duration, Instant},
};

/// How long a PATH probe result is trusted before the binary is spawned again.
///
/// A managed binary is trusted by existence, but a PATH-discovered one sits at an
/// unknown location and can only be confirmed by running it — and `detect_local_*`
/// runs on every app-snapshot emit (via `build_app_bootstrap`). Spawning the
/// frozen-Python `yt-dlp --version` costs ~1s, so an uncached probe puts that on
/// every emit: the same per-emit spawn cost that stalled the YouTube import queue.
///
/// This is a trust window rather than a permanent memo on purpose. A user who
/// installs (or uninstalls) a system binary mid-session is picked up on its own
/// within the window, with no app restart and nothing to invalidate by hand.
const PATH_PROBE_TRUST_WINDOW: Duration = Duration::from_secs(60);

struct CachedProbe {
    available: bool,
    probed_at: Instant,
}

/// Caches whether one PATH binary runs. Detection reads the managed candidates live
/// on every call — those are cheap metadata checks — so `asset_directory` changes
/// need no invalidation here: the managed branch answers before this cache is ever
/// consulted, and this only ever memoizes the "no managed binary" outcome.
pub(super) struct PathProbeCache {
    state: Mutex<Option<CachedProbe>>,
    trust_window: Duration,
}

impl PathProbeCache {
    pub(super) const fn new() -> Self {
        Self::with_trust_window(PATH_PROBE_TRUST_WINDOW)
    }

    const fn with_trust_window(trust_window: Duration) -> Self {
        Self {
            state: Mutex::new(None),
            trust_window,
        }
    }

    fn fresh_result(&self) -> Option<bool> {
        let cached = self.state.lock().ok()?;
        cached
            .as_ref()
            .filter(|entry| entry.probed_at.elapsed() < self.trust_window)
            .map(|entry| entry.available)
    }

    fn store(&self, available: bool) {
        if let Ok(mut cached) = self.state.lock() {
            *cached = Some(CachedProbe {
                available,
                probed_at: Instant::now(),
            });
        }
    }

    /// Runs `probe` only when there is no result inside the trust window.
    ///
    /// `probe` spawns a process, so it runs with the cache lock released — two callers
    /// racing a cold cache may both probe, and both then store the same answer. A
    /// poisoned lock degrades to probing every call rather than failing detection.
    pub(super) fn binary_is_available<P>(&self, probe: P) -> bool
    where
        P: FnOnce() -> bool,
    {
        if let Some(available) = self.fresh_result() {
            return available;
        }

        let available = probe();
        self.store(available);
        available
    }
}

#[cfg(test)]
mod tests {
    use super::PathProbeCache;
    use std::{
        sync::atomic::{AtomicUsize, Ordering},
        time::Duration,
    };

    #[test]
    fn a_fresh_result_is_reused_instead_of_spawning_the_binary_again() {
        let cache = PathProbeCache::new();
        let probes = AtomicUsize::new(0);
        let probe = || {
            probes.fetch_add(1, Ordering::SeqCst);
            true
        };

        assert!(cache.binary_is_available(probe));
        assert!(cache.binary_is_available(probe));
        assert!(cache.binary_is_available(probe));

        // Three detections — the cost of one spawn. This is the per-emit cost that
        // dragged the import queue out.
        assert_eq!(probes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_cached_miss_is_reused_too_so_a_missing_binary_is_not_re_probed_per_emit() {
        let cache = PathProbeCache::new();
        let probes = AtomicUsize::new(0);
        let probe = || {
            probes.fetch_add(1, Ordering::SeqCst);
            false
        };

        assert!(!cache.binary_is_available(probe));
        assert!(!cache.binary_is_available(probe));

        assert_eq!(probes.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn a_lapsed_result_is_probed_again_so_a_new_install_is_picked_up() {
        // A zero trust window expires immediately: stand-in for the window lapsing.
        let cache = PathProbeCache::with_trust_window(Duration::ZERO);
        let probes = AtomicUsize::new(0);

        assert!(!cache.binary_is_available(|| {
            probes.fetch_add(1, Ordering::SeqCst);
            false
        }));
        // The user installs yt-dlp; the next detection past the window sees it.
        assert!(cache.binary_is_available(|| {
            probes.fetch_add(1, Ordering::SeqCst);
            true
        }));

        assert_eq!(probes.load(Ordering::SeqCst), 2);
    }
}
