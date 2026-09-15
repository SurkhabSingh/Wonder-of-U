pub(crate) mod credit;
pub(crate) mod day;
// The streak rule has no caller until the report reads it.
#[allow(dead_code)]
pub(crate) mod ledger;
pub(crate) mod library;
pub(crate) mod listen_sampler;
pub(crate) mod liveness;
pub(crate) mod measured;
pub(crate) mod report;
pub(crate) mod store;
pub(crate) mod watch_sampler;

use tauri::{AppHandle, Manager, Runtime};

pub(crate) fn flush_days<R: Runtime>(app: &AppHandle<R>, pending: &mut ledger::Ledger) -> bool {
    let path = app
        .state::<crate::app_types::AppPathsState>()
        .progress_file
        .clone();
    match store::merge_days(&path, pending) {
        Ok(()) => {
            *pending = ledger::Ledger::default();
            true
        }
        Err(reason) => {
            crate::app_runtime::log_event(
                app,
                "WARN",
                "progress.day_flush_failed",
                serde_json::json!({ "message": reason }),
            );
            false
        }
    }
}

/// Marks the day a card reached Anki, from the two places a note id comes back rather than
/// from each shim, so a mining path added later cannot forget to.
pub(crate) fn record_mined_act<R: Runtime>(app: &AppHandle<R>) {
    let today = day::today();
    // Remembered only after a write that landed: fifty cards rewrite the store once, and a
    // failed write is retried by the next card rather than being counted as done.
    if MARKED_ON
        .lock()
        .is_ok_and(|marked| marked.as_ref() == Some(&today))
    {
        return;
    }
    let mut delta = ledger::Ledger::new();
    delta.mark_mined(&today);
    if flush_days(app, &mut delta) {
        if let Ok(mut marked) = MARKED_ON.lock() {
            *marked = Some(today);
        }
    }
}

static MARKED_ON: std::sync::Mutex<Option<day::DayKey>> = std::sync::Mutex::new(None);
