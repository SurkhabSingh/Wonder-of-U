use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use crate::app_config::PROGRESS_EVENT;

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WriteFailure {
    pub(crate) since_ms: u64,
    pub(crate) reason: String,
}

static FAILURE: Mutex<Option<WriteFailure>> = Mutex::new(None);

pub(crate) fn current() -> Option<WriteFailure> {
    FAILURE.lock().ok().and_then(|failure| failure.clone())
}

/// Every write to the store reports here. A write that landed refreshes an open page, the
/// first failure puts the notice up, and the failures after it stay quiet.
pub(crate) fn record<R: Runtime>(app: &AppHandle<R>, outcome: &Result<(), String>) {
    let now_ms = crate::app_runtime::now_ms();
    let announce = FAILURE
        .lock()
        .map(|mut failure| advance(&mut failure, outcome, now_ms))
        .unwrap_or(false);
    if announce {
        let _ = app.emit(PROGRESS_EVENT, ());
    }
}

/// Keeps the first failure: the notice says since when nothing has been saved, and a later
/// failure does not move that moment forward.
fn advance(failure: &mut Option<WriteFailure>, outcome: &Result<(), String>, now_ms: u64) -> bool {
    match outcome {
        Ok(()) => {
            *failure = None;
            true
        }
        Err(_) if failure.is_some() => false,
        Err(reason) => {
            *failure = Some(WriteFailure {
                since_ms: now_ms,
                reason: reason.clone(),
            });
            true
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn failed(reason: &str) -> Result<(), String> {
        Err(reason.to_string())
    }

    #[test]
    fn a_write_that_landed_is_always_announced() {
        let mut failure = None;
        assert!(advance(&mut failure, &Ok(()), 1));
        assert!(advance(&mut failure, &Ok(()), 2), "every landed write refreshes the page");
        assert_eq!(failure, None);
    }

    #[test]
    fn the_first_failure_is_announced_and_remembered() {
        let mut failure = None;
        assert!(advance(&mut failure, &failed("Access is denied."), 10));
        assert_eq!(
            failure,
            Some(WriteFailure {
                since_ms: 10,
                reason: "Access is denied.".to_string(),
            })
        );
    }

    #[test]
    fn a_later_failure_neither_announces_nor_moves_the_start() {
        let mut failure = None;
        advance(&mut failure, &failed("Access is denied."), 10);
        assert!(!advance(&mut failure, &failed("The disk is full."), 40));
        assert_eq!(failure.as_ref().map(|failure| failure.since_ms), Some(10));
        assert_eq!(
            failure.map(|failure| failure.reason),
            Some("Access is denied.".to_string())
        );
    }

    #[test]
    fn a_landed_write_takes_the_notice_down() {
        let mut failure = None;
        advance(&mut failure, &failed("Access is denied."), 10);
        assert!(advance(&mut failure, &Ok(()), 70));
        assert_eq!(failure, None);
    }

    #[test]
    fn the_notice_reaches_the_page_under_the_names_it_is_read_by() {
        let wire = serde_json::to_value(WriteFailure {
            since_ms: 1,
            reason: "x".to_string(),
        })
        .expect("serialises");
        let mut keys: Vec<&str> = wire
            .as_object()
            .expect("an object")
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(keys, ["reason", "sinceMs"]);
    }
}
