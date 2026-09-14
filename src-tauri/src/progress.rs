pub(crate) mod credit;
pub(crate) mod day;
// The streak rule and the mining mark have no caller until the report and the mine
// shims read them.
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

/// Clears the delta only on a write that landed, so a failure is retried rather than lost.
pub(crate) fn flush_days<R: Runtime>(app: &AppHandle<R>, pending: &mut ledger::Ledger) {
    let path = app
        .state::<crate::app_types::AppPathsState>()
        .progress_file
        .clone();
    match store::merge_days(&path, pending) {
        Ok(()) => *pending = ledger::Ledger::default(),
        Err(reason) => crate::app_runtime::log_event(
            app,
            "WARN",
            "progress.day_flush_failed",
            serde_json::json!({ "message": reason }),
        ),
    }
}
