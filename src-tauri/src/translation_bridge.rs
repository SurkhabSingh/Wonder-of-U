use std::{
    collections::{HashMap, VecDeque},
    sync::{Arc, Condvar, Mutex},
    thread,
    time::{Duration, Instant},
};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};
use tiny_http::{Header, Method, Request, Response, Server};

use crate::app_runtime::{log_event, now_ms};

/// Loopback port the desktop app listens on for the browser-extension
/// translation worker. Mirrors the extension's default (`BRIDGE.md`).
///
/// Must stay clear of Anki: 8765 is AnkiConnect and 8766 is our own furigana
/// add-on (see `anki::furigana`). Binding either would silently fail and leave
/// the extension talking to Anki, which answers our routes with a 404.
pub(crate) const BRIDGE_PORT: u16 = 8791;
const BRIDGE_PROTOCOL: &str = "1";
const WORKER_THREADS: usize = 4;
/// How recently the extension must have polled for the bridge to count as
/// connected. The extension long-polls continuously, so a live worker refreshes
/// this well within the window.
const CONNECTION_TTL: Duration = Duration::from_secs(30);
const MAX_LONG_POLL_SECONDS: u64 = 30;

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct BridgeJob {
    id: String,
    provider: String,
    source_text: String,
    source_lang: String,
    target_lang: String,
}

enum JobOutcome {
    Done(String),
    Failed(String),
}

#[derive(Default)]
struct BridgeInner {
    pending: VecDeque<BridgeJob>,
    results: HashMap<String, JobOutcome>,
    seq: u64,
    last_seen_at: Option<Instant>,
}

/// Shared translation job broker. The `translate` command submits a job and
/// blocks on `await_result`; the HTTP worker threads hand jobs to the extension
/// and record the outcome.
pub(crate) struct TranslationBridge {
    inner: Mutex<BridgeInner>,
    signal: Condvar,
}

impl TranslationBridge {
    pub(crate) fn new() -> Self {
        Self {
            inner: Mutex::new(BridgeInner::default()),
            signal: Condvar::new(),
        }
    }

    fn touch_last_seen(&self) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.last_seen_at = Some(Instant::now());
        }
    }

    /// True when the extension has contacted the bridge recently.
    pub(crate) fn is_connected(&self) -> bool {
        self.inner
            .lock()
            .ok()
            .and_then(|inner| inner.last_seen_at)
            .is_some_and(|seen| seen.elapsed() < CONNECTION_TTL)
    }

    /// Queues a translation job and wakes any waiting long-poll. Returns the id.
    pub(crate) fn submit(
        &self,
        source_text: String,
        source_lang: String,
        target_lang: String,
        provider: String,
    ) -> String {
        let mut inner = self.inner.lock().expect("translation bridge poisoned");
        inner.seq += 1;
        let id = format!("job-{}-{}", now_ms(), inner.seq);
        inner.pending.push_back(BridgeJob {
            id: id.clone(),
            provider,
            source_text,
            source_lang,
            target_lang,
        });
        self.signal.notify_all();
        id
    }

    /// Long-poll for the next pending job, blocking up to `wait`.
    fn claim_next(&self, wait: Duration) -> Option<BridgeJob> {
        let deadline = Instant::now() + wait;
        let mut inner = self.inner.lock().expect("translation bridge poisoned");
        inner.last_seen_at = Some(Instant::now());

        loop {
            if let Some(job) = inner.pending.pop_front() {
                return Some(job);
            }

            let now = Instant::now();
            if now >= deadline {
                return None;
            }

            let (guard, _) = self
                .signal
                .wait_timeout(inner, deadline - now)
                .expect("translation bridge poisoned");
            inner = guard;
        }
    }

    fn resolve(&self, id: &str, outcome: JobOutcome) {
        let mut inner = self.inner.lock().expect("translation bridge poisoned");
        inner.results.insert(id.to_string(), outcome);
        self.signal.notify_all();
    }

    /// Blocks until the job completes/fails or `timeout` elapses.
    pub(crate) fn await_result(&self, id: &str, timeout: Duration) -> Result<String, String> {
        let deadline = Instant::now() + timeout;
        let mut inner = self.inner.lock().expect("translation bridge poisoned");

        loop {
            if let Some(outcome) = inner.results.remove(id) {
                return match outcome {
                    JobOutcome::Done(text) => Ok(text),
                    JobOutcome::Failed(error) => Err(error),
                };
            }

            let now = Instant::now();
            if now >= deadline {
                inner.pending.retain(|job| job.id != id);
                return Err(
                    "Translation timed out waiting for the browser extension.".to_string(),
                );
            }

            let (guard, _) = self
                .signal
                .wait_timeout(inner, deadline - now)
                .expect("translation bridge poisoned");
            inner = guard;
        }
    }
}

/// Starts the loopback HTTP bridge server on a background worker pool. Binding
/// failures are logged and never crash the app; translation simply stays
/// unavailable until the port is free.
pub(crate) fn start_bridge_server<R: Runtime>(app: AppHandle<R>) {
    thread::spawn(move || {
        let server = match Server::http(("127.0.0.1", BRIDGE_PORT)) {
            Ok(server) => Arc::new(server),
            Err(error) => {
                log_event(
                    &app,
                    "ERROR",
                    "translation.bridge.bind_failed",
                    serde_json::json!({ "port": BRIDGE_PORT, "error": error.to_string() }),
                );
                return;
            }
        };

        log_event(
            &app,
            "INFO",
            "translation.bridge.started",
            serde_json::json!({ "port": BRIDGE_PORT }),
        );

        let version = app.package_info().version.to_string();
        let mut workers = Vec::new();
        for _ in 0..WORKER_THREADS {
            let server = Arc::clone(&server);
            let app = app.clone();
            let version = version.clone();
            workers.push(thread::spawn(move || {
                let bridge = app.state::<TranslationBridge>();
                for request in server.incoming_requests() {
                    handle_request(bridge.inner(), &version, request);
                }
            }));
        }

        for worker in workers {
            let _ = worker.join();
        }
    });
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CompleteBody {
    #[serde(default)]
    translated_text: String,
}

#[derive(Deserialize)]
struct FailBody {
    #[serde(default)]
    error: String,
}

fn handle_request(bridge: &TranslationBridge, version: &str, mut request: Request) {
    let method = request.method().clone();
    let raw_url = request.url().to_string();
    let (path, query) = match raw_url.split_once('?') {
        Some((path, query)) => (path.to_string(), query.to_string()),
        None => (raw_url, String::new()),
    };

    if method == Method::Get && path == "/v1/health" {
        bridge.touch_last_seen();
        let body = serde_json::json!({
            "protocol": BRIDGE_PROTOCOL,
            "version": version,
            "name": "wonder-of-u-desktop",
        });
        respond_json(request, 200, &body);
        return;
    }

    if method == Method::Get && path == "/v1/translation/next" {
        match bridge.claim_next(Duration::from_secs(parse_wait_seconds(&query))) {
            Some(job) => respond_json(request, 200, &job),
            None => {
                let _ = request.respond(Response::empty(204));
            }
        }
        return;
    }

    if method == Method::Post {
        if let Some(id) = job_id_for(&path, "/complete") {
            let body = read_body(&mut request);
            let translated = serde_json::from_str::<CompleteBody>(&body)
                .map(|parsed| parsed.translated_text)
                .unwrap_or_default();
            bridge.resolve(&id, JobOutcome::Done(translated));
            respond_json(request, 200, &serde_json::json!({ "ok": true }));
            return;
        }

        if let Some(id) = job_id_for(&path, "/fail") {
            let body = read_body(&mut request);
            let error = serde_json::from_str::<FailBody>(&body)
                .map(|parsed| parsed.error)
                .unwrap_or_default();
            let error = if error.trim().is_empty() {
                "The extension reported a translation failure.".to_string()
            } else {
                error
            };
            bridge.resolve(&id, JobOutcome::Failed(error));
            respond_json(request, 200, &serde_json::json!({ "ok": true }));
            return;
        }
    }

    let _ = request.respond(Response::empty(404));
}

/// Extracts the job id from `/v1/translation/jobs/{id}{suffix}`. Job ids are
/// generated locally and contain no reserved characters, so no percent-decoding
/// is required.
fn job_id_for(path: &str, suffix: &str) -> Option<String> {
    path.strip_prefix("/v1/translation/jobs/")
        .and_then(|rest| rest.strip_suffix(suffix))
        .filter(|id| !id.is_empty())
        .map(|id| id.to_string())
}

fn parse_wait_seconds(query: &str) -> u64 {
    query
        .split('&')
        .find_map(|pair| pair.strip_prefix("wait="))
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(5)
        .clamp(1, MAX_LONG_POLL_SECONDS)
}

fn read_body(request: &mut Request) -> String {
    let mut content = String::new();
    let _ = request.as_reader().read_to_string(&mut content);
    content
}

fn respond_json<V: Serialize>(request: Request, status: u16, body: &V) {
    let json = serde_json::to_string(body).unwrap_or_else(|_| "{}".to_string());
    let header = Header::from_bytes(&b"Content-Type"[..], &b"application/json"[..])
        .expect("valid content-type header");
    let response = Response::from_string(json)
        .with_status_code(status)
        .with_header(header);
    let _ = request.respond(response);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The HTTP tests below bind an ephemeral port, so they cannot catch the
    /// bridge being configured onto a port something else already owns. Anki is
    /// the neighbour that matters: it holds 8765 (AnkiConnect) and 8766 (our
    /// furigana add-on). Binding either fails, and the extension then long-polls
    /// Anki, which 404s our routes and surfaces as "bridge not connected".
    #[test]
    fn bridge_port_stays_clear_of_anki() {
        assert_ne!(BRIDGE_PORT, 8765, "8765 is AnkiConnect");
        assert_ne!(BRIDGE_PORT, 8766, "8766 is the furigana add-on");
    }

    #[test]
    fn queue_submit_claim_and_await() {
        let bridge = TranslationBridge::new();
        assert!(!bridge.is_connected());

        let id = bridge.submit("hello".into(), "ja".into(), "en".into(), String::new());
        let job = bridge
            .claim_next(Duration::from_secs(1))
            .expect("a job should be available");
        assert_eq!(job.id, id);
        assert_eq!(job.source_text, "hello");
        assert_eq!(job.source_lang, "ja");
        assert_eq!(job.target_lang, "en");
        // claim_next records extension activity.
        assert!(bridge.is_connected());

        bridge.resolve(&id, JobOutcome::Done("world".into()));
        let result = bridge
            .await_result(&id, Duration::from_secs(1))
            .expect("a result should be available");
        assert_eq!(result, "world");
    }

    #[test]
    fn await_result_times_out_and_drops_pending() {
        let bridge = TranslationBridge::new();
        let id = bridge.submit("hello".into(), "ja".into(), "en".into(), String::new());

        let error = bridge
            .await_result(&id, Duration::from_millis(50))
            .expect_err("should time out");
        assert!(error.to_lowercase().contains("timed out"));
        // The timed-out job is removed from the pending queue.
        assert!(bridge.claim_next(Duration::from_millis(50)).is_none());
    }

    #[test]
    fn http_round_trip_matches_contract() {
        let bridge = Arc::new(TranslationBridge::new());
        let server = Arc::new(Server::http("127.0.0.1:0").expect("bind loopback"));
        let addr = server
            .server_addr()
            .to_ip()
            .expect("a bound IP address");
        let base = format!("http://{addr}");

        {
            let server = Arc::clone(&server);
            let bridge = Arc::clone(&bridge);
            thread::spawn(move || {
                for request in server.incoming_requests() {
                    handle_request(&bridge, "9.9.9", request);
                }
            });
        }

        let client = reqwest::blocking::Client::new();
        let get_json = |url: String| -> serde_json::Value {
            let body = client.get(url).send().unwrap().text().unwrap();
            serde_json::from_str(&body).unwrap()
        };

        // Health handshake.
        let health = get_json(format!("{base}/v1/health"));
        assert_eq!(health["protocol"], "1");
        assert_eq!(health["version"], "9.9.9");

        // The desktop translate side submits a job and blocks for its result.
        let waiter_bridge = Arc::clone(&bridge);
        let waiter = thread::spawn(move || {
            let id = waiter_bridge.submit(
                "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}".into(),
                "ja".into(),
                "en".into(),
                String::new(),
            );
            waiter_bridge.await_result(&id, Duration::from_secs(5))
        });

        // The extension side claims the job over HTTP (long poll tolerates the race).
        let job = get_json(format!("{base}/v1/translation/next?wait=5"));
        assert_eq!(job["sourceText"], "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}");
        assert_eq!(job["targetLang"], "en");
        let job_id = job["id"].as_str().expect("job id").to_string();

        // The extension posts the completed translation back.
        let response = client
            .post(format!("{base}/v1/translation/jobs/{job_id}/complete"))
            .header("Content-Type", "application/json")
            .body(serde_json::json!({ "translatedText": "hello" }).to_string())
            .send()
            .unwrap();
        assert!(response.status().is_success());

        let translated = waiter.join().unwrap().expect("a translation");
        assert_eq!(translated, "hello");
    }
}
