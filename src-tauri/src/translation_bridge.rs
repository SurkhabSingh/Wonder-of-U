use std::{
    collections::{HashMap, HashSet, VecDeque},
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
/// connected. The client long-polls with `wait=25`, so it refreshes this at least
/// that often; the window has to be comfortably wider than one poll, or a single
/// slow round trip reads as a disconnection.
const CONNECTION_TTL: Duration = Duration::from_secs(60);
const MAX_LONG_POLL_SECONDS: u64 = 30;
/// How long a claimed job may go unanswered before it is handed back out.
///
/// The client gives the browser 120s to translate and then reports the failure
/// itself, so under normal operation the lease never fires — it exists for the
/// case where the client process dies mid-job, which previously lost the job
/// silently and made the caller wait out its entire timeout for nothing.
const LEASE_TIMEOUT: Duration = Duration::from_secs(135);
/// One retry. A job that two separate claims could not translate is not going to
/// start working on a third.
const MAX_ATTEMPTS: u32 = 2;
/// Back-pressure. A caller submits one job and blocks on it, so the queue only
/// grows past a handful if something is badly wrong.
const MAX_QUEUE_DEPTH: usize = 64;

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

/// A job handed to the client and not yet answered for.
struct ClaimedJob {
    job: BridgeJob,
    claimed_at: Instant,
    attempts: u32,
}

#[derive(Default)]
struct BridgeInner {
    pending: VecDeque<BridgeJob>,
    /// Jobs the client has taken but not yet resolved. Without this a claimed job
    /// existed nowhere: `claim_next` popped it off `pending` and tracked it
    /// nowhere, so a client that died mid-job dropped it on the floor.
    claimed: HashMap<String, ClaimedJob>,
    results: HashMap<String, JobOutcome>,
    /// Ids a caller is currently blocked on. `resolve` refuses to record a result
    /// for anything not in here, which is what keeps `results` bounded: it used to
    /// accept any id at all, so late, duplicate, and unsolicited posts accumulated
    /// for the lifetime of the process.
    waiting: HashSet<String>,
    attempts: HashMap<String, u32>,
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
    ) -> Result<String, String> {
        let mut inner = self.inner.lock().expect("translation bridge poisoned");

        if inner.pending.len() >= MAX_QUEUE_DEPTH {
            return Err("The translation queue is full. Try again shortly.".to_string());
        }

        inner.seq += 1;
        let id = format!("job-{}-{}", now_ms(), inner.seq);
        inner.pending.push_back(BridgeJob {
            id: id.clone(),
            provider,
            source_text,
            source_lang,
            target_lang,
        });
        inner.waiting.insert(id.clone());
        inner.attempts.insert(id.clone(), 0);
        self.signal.notify_all();
        Ok(id)
    }

    /// Long-poll for the next pending job, blocking up to `wait`.
    fn claim_next(&self, wait: Duration) -> Option<BridgeJob> {
        let deadline = Instant::now() + wait;
        let mut inner = self.inner.lock().expect("translation bridge poisoned");
        inner.last_seen_at = Some(Instant::now());

        loop {
            Self::requeue_expired_leases(&mut inner);

            if let Some(job) = inner.pending.pop_front() {
                let attempts = inner.attempts.entry(job.id.clone()).or_insert(0);
                *attempts += 1;
                let attempts = *attempts;

                inner.claimed.insert(
                    job.id.clone(),
                    ClaimedJob {
                        job: job.clone(),
                        claimed_at: Instant::now(),
                        attempts,
                    },
                );

                return Some(job);
            }

            let now = Instant::now();
            if now >= deadline {
                return None;
            }

            // Wake at least once a second so an expired lease is noticed even when
            // no new job arrives to signal us.
            let slice = (deadline - now).min(Duration::from_secs(1));
            let (guard, _) = self
                .signal
                .wait_timeout(inner, slice)
                .expect("translation bridge poisoned");
            inner = guard;
        }
    }

    /// Hands a job back out when the client took it and never answered — it died,
    /// or the browser was closed mid-translation. One retry, then it is failed
    /// rather than looped forever.
    fn requeue_expired_leases(inner: &mut BridgeInner) {
        let expired: Vec<String> = inner
            .claimed
            .iter()
            .filter(|(_, claimed)| claimed.claimed_at.elapsed() >= LEASE_TIMEOUT)
            .map(|(id, _)| id.clone())
            .collect();

        for id in expired {
            let Some(claimed) = inner.claimed.remove(&id) else {
                continue;
            };

            if !inner.waiting.contains(&id) {
                // Nobody is listening any more; drop it.
                inner.attempts.remove(&id);
                continue;
            }

            if claimed.attempts >= MAX_ATTEMPTS {
                inner.results.insert(
                    id.clone(),
                    JobOutcome::Failed(
                        "The browser extension stopped responding while translating.".to_string(),
                    ),
                );
                inner.attempts.remove(&id);
                continue;
            }

            inner.pending.push_back(claimed.job);
        }
    }

    /// Records a completion or failure. Unknown ids are ignored: a result nobody
    /// is waiting for is either a duplicate post, a late one, or not ours.
    fn resolve(&self, id: &str, outcome: JobOutcome) -> bool {
        let mut inner = self.inner.lock().expect("translation bridge poisoned");
        inner.claimed.remove(id);

        if !inner.waiting.contains(id) {
            return false;
        }

        // First writer wins, so a retried POST cannot overwrite a stored result.
        if inner.results.contains_key(id) {
            return true;
        }

        inner.results.insert(id.to_string(), outcome);
        self.signal.notify_all();
        true
    }

    /// Blocks until the job completes/fails or `timeout` elapses.
    pub(crate) fn await_result(&self, id: &str, timeout: Duration) -> Result<String, String> {
        let deadline = Instant::now() + timeout;
        let mut inner = self.inner.lock().expect("translation bridge poisoned");

        loop {
            if let Some(outcome) = inner.results.remove(id) {
                // Clear the job from everywhere, not just from `results`. A result
                // can land before the job was ever claimed, and a `pending` entry
                // left behind would later be handed out as a phantom job.
                inner.pending.retain(|job| job.id != id);
                inner.claimed.remove(id);
                inner.waiting.remove(id);
                inner.attempts.remove(id);
                return match outcome {
                    JobOutcome::Done(text) => Ok(text),
                    JobOutcome::Failed(error) => Err(error),
                };
            }

            let now = Instant::now();
            if now >= deadline {
                // Stop tracking the job everywhere, so nothing about it lingers.
                inner.pending.retain(|job| job.id != id);
                inner.claimed.remove(id);
                inner.waiting.remove(id);
                inner.attempts.remove(id);
                inner.results.remove(id);
                return Err(
                    "Translation timed out waiting for the browser extension.".to_string(),
                );
            }

            let slice = (deadline - now).min(Duration::from_secs(1));
            let (guard, _) = self
                .signal
                .wait_timeout(inner, slice)
                .expect("translation bridge poisoned");
            inner = guard;

            Self::requeue_expired_leases(&mut inner);
        }
    }

    #[cfg(test)]
    fn queue_depth(&self) -> (usize, usize, usize) {
        let inner = self.inner.lock().expect("translation bridge poisoned");
        (inner.pending.len(), inner.claimed.len(), inner.results.len())
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

/// Only our own local client may talk to the bridge.
///
/// The client is the native messaging host, a plain Node process: it sends no
/// `Origin`. A browser page can reach loopback, but it cannot suppress `Origin` —
/// so the presence of one means the caller is a web page, and a web page has no
/// business claiming translation jobs (it would be reading transcript text) or
/// injecting results. The `Host` check is the usual DNS-rebinding guard: a name
/// that resolves to 127.0.0.1 still arrives with its own Host header.
fn is_authorized(request: &Request) -> bool {
    let mut host_ok = false;

    for header in request.headers() {
        let field = header.field.as_str().as_str().to_ascii_lowercase();
        let value = header.value.as_str();

        if field == "origin" && !value.trim().is_empty() {
            return false;
        }

        if field == "host" {
            let hostname = value.split(':').next().unwrap_or_default();
            host_ok = matches!(hostname, "127.0.0.1" | "localhost" | "[::1]");
        }
    }

    host_ok
}

fn handle_request(bridge: &TranslationBridge, version: &str, mut request: Request) {
    if !is_authorized(&request) {
        let _ = request.respond(Response::empty(403));
        return;
    }

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

    // `complete` and `fail` answer 200 even for an id the bridge no longer knows,
    // as the contract requires (BRIDGE.md): a retried post must be a harmless
    // no-op rather than something that corrupts state. `accepted` tells the client
    // whether the result was actually recorded.
    if method == Method::Post {
        if let Some(id) = job_id_for(&path, "/complete") {
            let body = read_body(&mut request);
            let translated = serde_json::from_str::<CompleteBody>(&body)
                .map(|parsed| parsed.translated_text)
                .unwrap_or_default();
            let accepted = bridge.resolve(&id, JobOutcome::Done(translated));
            respond_json(
                request,
                200,
                &serde_json::json!({ "ok": true, "accepted": accepted }),
            );
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
            let accepted = bridge.resolve(&id, JobOutcome::Failed(error));
            respond_json(
                request,
                200,
                &serde_json::json!({ "ok": true, "accepted": accepted }),
            );
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

    fn submit_test_job(bridge: &TranslationBridge) -> String {
        bridge
            .submit("hello".into(), "ja".into(), "en".into(), String::new())
            .expect("the queue should accept a job")
    }

    #[test]
    fn queue_submit_claim_and_await() {
        let bridge = TranslationBridge::new();
        assert!(!bridge.is_connected());

        let id = submit_test_job(&bridge);
        let job = bridge
            .claim_next(Duration::from_secs(1))
            .expect("a job should be available");
        assert_eq!(job.id, id);
        assert_eq!(job.source_text, "hello");
        assert_eq!(job.source_lang, "ja");
        assert_eq!(job.target_lang, "en");
        // claim_next records extension activity.
        assert!(bridge.is_connected());

        assert!(bridge.resolve(&id, JobOutcome::Done("world".into())));
        let result = bridge
            .await_result(&id, Duration::from_secs(1))
            .expect("a result should be available");
        assert_eq!(result, "world");

        // Nothing is left behind once the caller has its answer.
        assert_eq!(bridge.queue_depth(), (0, 0, 0));
    }

    #[test]
    fn failed_jobs_report_their_reason() {
        let bridge = TranslationBridge::new();
        let id = submit_test_job(&bridge);

        assert!(bridge.resolve(&id, JobOutcome::Failed("DeepL was unreadable.".into())));
        let error = bridge
            .await_result(&id, Duration::from_secs(1))
            .expect_err("a failed job should surface as an error");
        assert_eq!(error, "DeepL was unreadable.");
        assert_eq!(bridge.queue_depth(), (0, 0, 0));
    }

    #[test]
    fn await_result_times_out_and_drops_pending() {
        let bridge = TranslationBridge::new();
        let id = submit_test_job(&bridge);

        let error = bridge
            .await_result(&id, Duration::from_millis(50))
            .expect_err("should time out");
        assert!(error.to_lowercase().contains("timed out"));
        // The timed-out job is removed from the pending queue.
        assert!(bridge.claim_next(Duration::from_millis(50)).is_none());
        assert_eq!(bridge.queue_depth(), (0, 0, 0));
    }

    /// The bug this guards: `resolve` used to insert any id at all, and only
    /// `await_result` ever removed one. So a late, duplicate, or simply bogus POST
    /// left an entry in `results` that nothing would ever collect, and the map grew
    /// for the lifetime of the process.
    #[test]
    fn results_for_unknown_jobs_are_rejected_not_stored() {
        let bridge = TranslationBridge::new();

        assert!(!bridge.resolve("job-nobody-asked-for", JobOutcome::Done("junk".into())));
        assert_eq!(bridge.queue_depth(), (0, 0, 0));

        // A duplicate post after the caller already collected its result is a no-op
        // rather than a fresh, permanent entry.
        let id = submit_test_job(&bridge);
        assert!(bridge.resolve(&id, JobOutcome::Done("world".into())));
        assert_eq!(
            bridge
                .await_result(&id, Duration::from_secs(1))
                .expect("a result"),
            "world"
        );
        assert!(!bridge.resolve(&id, JobOutcome::Done("world again".into())));
        assert_eq!(bridge.queue_depth(), (0, 0, 0));
    }

    /// A retried POST must not overwrite the result already recorded for a job.
    #[test]
    fn first_result_wins_over_a_retried_post() {
        let bridge = TranslationBridge::new();
        let id = submit_test_job(&bridge);

        assert!(bridge.resolve(&id, JobOutcome::Done("first".into())));
        assert!(bridge.resolve(&id, JobOutcome::Failed("a retry that lost the race".into())));

        assert_eq!(
            bridge
                .await_result(&id, Duration::from_secs(1))
                .expect("the first result"),
            "first"
        );
    }

    /// The job-loss bug: `claim_next` popped a job off `pending` and tracked it
    /// nowhere, so a client that died mid-job took the job with it and the caller
    /// waited out its whole timeout for a result that could never arrive.
    #[test]
    fn an_abandoned_job_is_handed_back_out() {
        let bridge = TranslationBridge::new();
        let id = submit_test_job(&bridge);

        let claimed = bridge
            .claim_next(Duration::from_millis(50))
            .expect("a job to claim");
        assert_eq!(claimed.id, id);
        assert_eq!(bridge.queue_depth(), (0, 1, 0), "the job is now leased");

        // Nothing is pending while the lease is live, so it is not handed out twice.
        assert!(bridge.claim_next(Duration::from_millis(50)).is_none());

        expire_all_leases(&bridge);

        let requeued = bridge
            .claim_next(Duration::from_millis(50))
            .expect("the abandoned job should be handed back out");
        assert_eq!(requeued.id, id, "the same job, not a new one");
    }

    /// ...but it is not handed out forever. After MAX_ATTEMPTS the caller is told
    /// what happened instead of waiting for a client that clearly is not coming back.
    #[test]
    fn an_abandoned_job_fails_after_its_retry() {
        let bridge = TranslationBridge::new();
        let id = submit_test_job(&bridge);

        for _ in 0..MAX_ATTEMPTS {
            bridge
                .claim_next(Duration::from_millis(50))
                .expect("a job to claim");
            expire_all_leases(&bridge);
        }

        let error = bridge
            .await_result(&id, Duration::from_millis(200))
            .expect_err("the job should be failed, not retried forever");
        assert!(
            error.contains("stopped responding"),
            "unexpected error: {error}"
        );
        assert_eq!(bridge.queue_depth(), (0, 0, 0));
    }

    /// Ages every live lease past LEASE_TIMEOUT without sleeping for it.
    fn expire_all_leases(bridge: &TranslationBridge) {
        let mut inner = bridge.inner.lock().expect("translation bridge poisoned");
        let aged = Instant::now() - LEASE_TIMEOUT - Duration::from_secs(1);
        for claimed in inner.claimed.values_mut() {
            claimed.claimed_at = aged;
        }
        TranslationBridge::requeue_expired_leases(&mut inner);
    }

    #[test]
    fn the_queue_is_bounded() {
        let bridge = TranslationBridge::new();

        for _ in 0..MAX_QUEUE_DEPTH {
            submit_test_job(&bridge);
        }

        let error = bridge
            .submit("one too many".into(), "ja".into(), "en".into(), String::new())
            .expect_err("the queue must refuse to grow without limit");
        assert!(error.to_lowercase().contains("full"), "unexpected: {error}");
    }

    #[test]
    fn connection_goes_stale_without_polling() {
        let bridge = TranslationBridge::new();
        assert!(!bridge.is_connected(), "never seen is not connected");

        bridge.touch_last_seen();
        assert!(bridge.is_connected());

        {
            let mut inner = bridge.inner.lock().expect("translation bridge poisoned");
            inner.last_seen_at = Some(Instant::now() - CONNECTION_TTL - Duration::from_secs(1));
        }

        assert!(
            !bridge.is_connected(),
            "a client that stopped polling must read as disconnected"
        );
    }

    #[test]
    fn long_poll_wait_is_parsed_and_clamped() {
        assert_eq!(parse_wait_seconds("wait=25"), 25);
        assert_eq!(parse_wait_seconds(""), 5, "the default");
        assert_eq!(parse_wait_seconds("wait=nonsense"), 5);
        assert_eq!(parse_wait_seconds("wait=0"), 1, "clamped up");
        assert_eq!(
            parse_wait_seconds("wait=9999"),
            MAX_LONG_POLL_SECONDS,
            "clamped down, so a client cannot pin a worker thread indefinitely"
        );
        assert_eq!(parse_wait_seconds("other=1&wait=7"), 7);
    }

    #[test]
    fn job_ids_are_parsed_from_their_routes() {
        assert_eq!(
            job_id_for("/v1/translation/jobs/job-7/complete", "/complete").as_deref(),
            Some("job-7")
        );
        assert_eq!(
            job_id_for("/v1/translation/jobs/job-7/fail", "/fail").as_deref(),
            Some("job-7")
        );
        assert_eq!(job_id_for("/v1/translation/jobs//complete", "/complete"), None);
        assert_eq!(job_id_for("/v1/translation/jobs/job-7", "/complete"), None);
        assert_eq!(job_id_for("/somewhere/else/complete", "/complete"), None);
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

        // An empty queue answers 204 so the client simply polls again, rather than
        // reading a bodyless 200 as a malformed job.
        let empty = client
            .get(format!("{base}/v1/translation/next?wait=1"))
            .send()
            .unwrap();
        assert_eq!(empty.status().as_u16(), 204);

        // The desktop translate side submits a job and blocks for its result.
        let waiter_bridge = Arc::clone(&bridge);
        let waiter = thread::spawn(move || {
            let id = waiter_bridge
                .submit(
                    "\u{3053}\u{3093}\u{306b}\u{3061}\u{306f}".into(),
                    "ja".into(),
                    "en".into(),
                    String::new(),
                )
                .expect("the queue should accept a job");
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

    #[test]
    fn http_reports_a_failed_job_to_the_caller() {
        let bridge = Arc::new(TranslationBridge::new());
        let base = spawn_bridge_server(Arc::clone(&bridge));
        let client = reqwest::blocking::Client::new();

        let waiter_bridge = Arc::clone(&bridge);
        let waiter = thread::spawn(move || {
            let id = submit_test_job(&waiter_bridge);
            waiter_bridge.await_result(&id, Duration::from_secs(5))
        });

        let body = client
            .get(format!("{base}/v1/translation/next?wait=5"))
            .send()
            .unwrap()
            .text()
            .unwrap();
        let job: serde_json::Value = serde_json::from_str(&body).unwrap();
        let job_id = job["id"].as_str().expect("job id").to_string();

        let response = client
            .post(format!("{base}/v1/translation/jobs/{job_id}/fail"))
            .header("Content-Type", "application/json")
            .body(serde_json::json!({ "error": "" }).to_string())
            .send()
            .unwrap();
        assert!(response.status().is_success());

        let error = waiter.join().unwrap().expect_err("a failure");
        assert_eq!(error, "The extension reported a translation failure.");
    }

    /// A web page can reach 127.0.0.1, but it cannot suppress its `Origin` header.
    /// Our only client is a native process, which sends none — so an `Origin` means
    /// a page is calling, and a page must not be able to claim jobs (that would read
    /// the user's transcripts) or inject results.
    #[test]
    fn requests_from_a_web_page_are_refused() {
        let bridge = Arc::new(TranslationBridge::new());
        let base = spawn_bridge_server(Arc::clone(&bridge));
        let client = reqwest::blocking::Client::new();

        let response = client
            .get(format!("{base}/v1/health"))
            .header("Origin", "https://evil.example")
            .send()
            .unwrap();
        assert_eq!(response.status().as_u16(), 403);

        // And the native client, which sends no Origin, still gets through.
        let allowed = client.get(format!("{base}/v1/health")).send().unwrap();
        assert!(allowed.status().is_success());
    }

    #[test]
    fn unknown_routes_are_not_found() {
        let bridge = Arc::new(TranslationBridge::new());
        let base = spawn_bridge_server(bridge);
        let client = reqwest::blocking::Client::new();

        let response = client.get(format!("{base}/v1/nope")).send().unwrap();
        assert_eq!(response.status().as_u16(), 404);
    }

    fn spawn_bridge_server(bridge: Arc<TranslationBridge>) -> String {
        let server = Arc::new(Server::http("127.0.0.1:0").expect("bind loopback"));
        let addr = server.server_addr().to_ip().expect("a bound IP address");

        thread::spawn(move || {
            for request in server.incoming_requests() {
                handle_request(&bridge, "9.9.9", request);
            }
        });

        format!("http://{addr}")
    }
}
