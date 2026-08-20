//! One download at a time, but you can ask for several.
//!
//! Clicking Download on a second asset while one is running used to fail with "Another download
//! is already in progress." It waits its turn now. First run needs the same thing — fetch the
//! runtime, then the model, then ffmpeg, without anyone standing over it — so it is one
//! mechanism rather than two.
//!
//! **Sequential, not parallel, and that is a deliberate limit.** Two transfers at once would
//! share one connection, so total time barely moves unless the server rather than the link is
//! the bottleneck — and six things would break: releasing the download slot is global, so the
//! first to finish would clear `paused` for the rest; `shell.phase` is a single value; Pause and
//! Cancel take no argument and so could not say which; one snapshot cannot describe two
//! transfers; the emit cost multiplies; and the model and runtime each read-modify-write the
//! whole persisted settings after installing, so running them together can lose one's change.
//! A queue keeps exactly one download active, and every one of those stays true.

use std::collections::VecDeque;

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_runtime::update_shell_snapshot,
    app_types::{ModelDownloadQueueState, SharedShellState},
};

use super::asset::AssetKind;
use super::envelope::{run_asset_download, AssetDownloadPlan};
use super::transfer::update_model_download_snapshot;

/// One thing the user has asked for.
///
/// Not a bare [`AssetKind`], and the reason is two assets that `AssetKind` cannot tell apart.
/// The whisper runtime installs versions side by side, so "download the runtime" is an
/// incomplete request — it has to say which. And the speech detector deliberately reports as
/// `AssetKind::Model` so it shares the model's progress card, which would make `Model`
/// ambiguous between two genuinely different downloads.
///
/// So this lists what can be *asked for*, while `AssetKind` stays what gets *reported*. Seven
/// variants, one per real request, and `plan_for` matches all of them — a seventh asset will
/// not compile until it is wired in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum QueuedDownload {
    WhisperModel,
    WhisperVadModel,
    WhisperRuntime { version: String },
    /// `reinstall` fetches a fresh copy over one that is already installed and working.
    ///
    /// It exists because the ordinary download deliberately SKIPS when a runnable ffmpeg is
    /// already present, which is right when the request means "I have none" and wrong when it
    /// means "replace the one I have". Carrying the intent on the request keeps the skip honest
    /// rather than adding a second way in — and the two are different requests to the queue, so
    /// a reinstall is never mistaken for a duplicate of a pending first install.
    ///
    /// Safe ordering is a property of the plan, not of this flag: extraction clears the install
    /// directory, so the working copy is only replaced once the new archive is on disk.
    Ffmpeg { reinstall: bool },
    Ytdlp,
    Alass,
    Dictionary,
    /// `reinstall` carries the same meaning as it does for ffmpeg: fetch a fresh copy over one
    /// that already works. Without it the Settings button that offers exactly that skips the
    /// fetch and reports a download that never happened.
    Mpv { reinstall: bool },
}

impl QueuedDownload {
    /// Which asset the progress card should attribute this to.
    pub(crate) fn kind(&self) -> AssetKind {
        match self {
            // The detector is part of provisioning the model, and belongs in its card.
            QueuedDownload::WhisperModel | QueuedDownload::WhisperVadModel => AssetKind::Model,
            QueuedDownload::WhisperRuntime { .. } => AssetKind::Runtime,
            QueuedDownload::Ffmpeg { .. } => AssetKind::Ffmpeg,
            QueuedDownload::Ytdlp => AssetKind::Ytdlp,
            QueuedDownload::Alass => AssetKind::Alass,
            QueuedDownload::Dictionary => AssetKind::Dictionary,
            QueuedDownload::Mpv { .. } => AssetKind::Mpv,
        }
    }

    /// Refusal shown when the app is mid-task — recording, or transcribing.
    ///
    /// It lives with the request rather than being passed in by the command, because it names
    /// the asset and everything that names an asset belongs in one place. It is emphatically
    /// NOT "another download is already in progress": that is what the queue exists to stop
    /// being an error, and saying it here would describe the wrong condition entirely.
    fn busy_message(&self) -> &'static str {
        match self {
            QueuedDownload::WhisperModel => {
                "Finish the current task before downloading the Whisper model."
            }
            QueuedDownload::WhisperVadModel => {
                "Finish the current task before downloading the speech detector."
            }
            QueuedDownload::WhisperRuntime { .. } => {
                "Finish the current task before downloading the Whisper runtime."
            }
            QueuedDownload::Ffmpeg { .. } => "Finish the current task before downloading FFmpeg.",
            QueuedDownload::Ytdlp => "Finish the current task before downloading yt-dlp.",
            QueuedDownload::Alass => "Finish the current task before downloading alass.",
            QueuedDownload::Dictionary => {
                "Finish the current task before downloading the Japanese dictionary."
            }
            QueuedDownload::Mpv { .. } => "Finish the current task before downloading mpv.",
        }
    }
}

/// What is waiting, what is running, and whether anyone is working through it.
///
/// `running` sits beside `pending` under one lock on purpose. "Is a worker alive" and "is there
/// anything left" have to be decided together — otherwise a worker that finds the queue empty
/// can stop being the worker at the same moment an enqueue decides not to start one, and the
/// request sits there forever.
#[derive(Default)]
pub(crate) struct DownloadQueue {
    pending: VecDeque<QueuedDownload>,
    /// What the active download is, so a repeat press is recognised as a duplicate rather than
    /// queued behind itself.
    active: Option<QueuedDownload>,
    /// Whether a worker thread is alive. True *between* items too, when `active` is `None`,
    /// which is why both fields exist.
    running: bool,
}

impl DownloadQueue {
    fn already_wanted(&self, request: &QueuedDownload) -> bool {
        self.active.as_ref() == Some(request) || self.pending.contains(request)
    }
}

/// Refuses while the app is busy with something else.
///
/// Checked once per *queue*, not once per download: the phase is `"downloading-model"` for as
/// long as the queue runs, so a per-download check would reject every item after the first.
fn refuse_when_app_is_busy<R: Runtime>(app: &AppHandle<R>, busy_message: &str) -> Result<(), String> {
    let shell_state = app.state::<SharedShellState>();
    let shell = shell_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the shell state.".to_string())?;
    if shell.phase != "idle" && shell.phase != "error" {
        return Err(busy_message.to_string());
    }
    Ok(())
}

/// Adds a request to the queue, starting a worker if nothing is working yet.
///
/// Pressing the same button twice is harmless: a request already active or already waiting is
/// ignored rather than queued behind itself.
pub(crate) fn enqueue_download<R: Runtime>(
    app: &AppHandle<R>,
    request: QueuedDownload,
) -> Result<(), String> {
    let should_start = {
        let state = app.state::<ModelDownloadQueueState>();
        let mut queue = state
            .0
            .lock()
            .map_err(|_| "Could not inspect the download queue.".to_string())?;
        if queue.already_wanted(&request) {
            return Ok(());
        }
        queue.pending.push_back(request.clone());
        // Claiming the right to run happens in the same lock as observing that nobody has it.
        let should_start = !queue.running;
        if should_start {
            queue.running = true;
        }
        should_start
    };

    if !should_start {
        // A worker is already going; it will pick this up when it finishes the current item.
        publish_queue_depth(app);
        return Ok(());
    }

    // Only the request that starts the queue is subject to the busy check, and it is checked
    // AFTER claiming the run so two simultaneous presses cannot both decide to start.
    if let Err(busy) = refuse_when_app_is_busy(app, request.busy_message()) {
        // Claimed the right to run and cannot use it, so hand it straight back — otherwise the
        // queue is left marked running with no worker, and every later request waits forever.
        abandon_queue(app);
        return Err(busy);
    }

    // The worker owns the phase for the whole queue — set once here, cleared once when it
    // drains. Doing it per download would flick to "idle" between items, and a recording
    // started in that gap would make the next item fail the check above.
    update_shell_snapshot(app, |shell| {
        shell.phase = "downloading-model".into();
        shell.started_at_ms = None;
        shell.current_recording_name = None;
    })?;

    let worker_app = app.clone();
    std::thread::Builder::new()
        .name("asset-download-queue".into())
        .spawn(move || drive_queue(&worker_app))
        .map_err(|error| {
            // The thread never started, so nothing will release any of this.
            abandon_queue(app);
            let _ = update_shell_snapshot(app, |shell| {
                shell.phase = "idle".into();
                shell.started_at_ms = None;
            });
            error.to_string()
        })?;

    Ok(())
}

/// What the worker should do next.
enum NextRequest {
    Run(QueuedDownload),
    /// Nothing left, and this worker has stopped being the worker.
    Drained,
}

/// Takes the next request, or retires.
///
/// Both outcomes happen under one lock, which is what makes the retirement safe: an enqueue
/// arriving afterwards sees `running == false` and starts a fresh worker, and one arriving
/// before is still in `pending` and comes back as `Run`.
fn take_next<R: Runtime>(app: &AppHandle<R>) -> NextRequest {
    let state = app.state::<ModelDownloadQueueState>();
    let Ok(mut queue) = state.0.lock() else {
        // A poisoned lock means the queue can no longer be trusted; stopping is the only
        // honest option, and the shell phase is released by the caller either way.
        return NextRequest::Drained;
    };
    match queue.pending.pop_front() {
        Some(request) => {
            queue.active = Some(request.clone());
            NextRequest::Run(request)
        }
        None => {
            queue.active = None;
            queue.running = false;
            NextRequest::Drained
        }
    }
}

/// Empties the queue and retires the worker. Used when a download fails or is cancelled.
fn abandon_queue<R: Runtime>(app: &AppHandle<R>) {
    let state = app.state::<ModelDownloadQueueState>();
    // `let … else` rather than `if let`: an `if let` scrutinee temporary outlives the `state`
    // binding it borrows from, which the compiler refuses.
    let Ok(mut queue) = state.0.lock() else {
        return;
    };
    queue.pending.clear();
    queue.active = None;
    queue.running = false;
}

/// Tells the card how many requests are still waiting, so it can say "2 more queued".
///
/// Read and released before writing, rather than writing under the queue lock: nothing else
/// takes these two locks in the other order today, and keeping it that way is cheaper than
/// remembering not to.
fn publish_queue_depth<R: Runtime>(app: &AppHandle<R>) {
    let depth = {
        let state = app.state::<ModelDownloadQueueState>();
        let Ok(queue) = state.0.lock() else {
            return;
        };
        queue.pending.len()
    };
    let _ = update_model_download_snapshot(app, |snapshot| {
        snapshot.queued_remaining = depth;
    });
}

/// Puts the item about to run into the snapshot, before anything that can fail.
///
/// Two things went wrong without this. The card is shown for as long as the shell phase is held,
/// but everything inside it reads the snapshot STATUS — so between one item completing and the
/// next reporting, the card announced "Download in progress" over a finished bar with no Cancel
/// button. And `report_queue_error` writes "failed" directly when `plan_for` fails, with no
/// non-terminal status in between, so a second press on the same broken condition was a
/// "failed" to "failed" transition that the edge-triggered toast could not see: the button did
/// nothing, twice, silently.
///
/// Claiming the snapshot here gives every item a real transition of its own, which fixes both
/// without either surface needing a guard of its own.
fn claim_snapshot_for<R: Runtime>(app: &AppHandle<R>, request: &QueuedDownload) {
    let depth = {
        let state = app.state::<ModelDownloadQueueState>();
        let Ok(queue) = state.0.lock() else {
            return;
        };
        queue.pending.len()
    };
    let kind = request.kind();
    let _ = update_model_download_snapshot(app, |snapshot| {
        snapshot.kind = Some(kind);
        snapshot.status = "starting".into();
        snapshot.message = format!("Preparing the {} download...", kind.label());
        snapshot.downloaded_bytes = 0;
        snapshot.total_bytes = None;
        snapshot.progress_percent = None;
        snapshot.target_path = None;
        snapshot.queued_remaining = depth;
    });
}

/// Works through the queue, one download at a time, until it is empty or something stops it.
fn drive_queue<R: Runtime>(app: &AppHandle<R>) {
    while let NextRequest::Run(request) = take_next(app) {
        claim_snapshot_for(app, &request);

        let plan = match plan_for(app, &request) {
            Ok(plan) => plan,
            Err(error) => {
                report_queue_error(app, request.kind(), &error);
                abandon_queue(app);
                break;
            }
        };

        // A failure stops the queue. Carrying on reads better in the abstract, but the next
        // item's "starting" snapshot would immediately paint over the failure and the user
        // would never learn what went wrong. Whatever did not run is simply still not
        // downloaded, so retrying is one press.
        if run_asset_download(app, plan).is_err() {
            abandon_queue(app);
            break;
        }
    }

    // The phase is this worker's to release, however it got here.
    let _ = update_shell_snapshot(app, |shell| {
        shell.phase = "idle".into();
        shell.started_at_ms = None;
    });
    let _ = update_model_download_snapshot(app, |snapshot| {
        snapshot.queued_remaining = 0;
    });
}

/// A request that could not even be described — a missing asset directory, an unreadable
/// settings file. The download itself reports its own failures; this covers the step before.
fn report_queue_error<R: Runtime>(app: &AppHandle<R>, kind: AssetKind, error: &str) {
    let _ = update_model_download_snapshot(app, |snapshot| {
        snapshot.kind = Some(kind);
        snapshot.status = "failed".into();
        snapshot.message = format!("{} download failed: {error}", kind.label());
    });
}

/// Turns a request into the plan that performs it.
///
/// The one place that names every request, so `QueuedDownload`'s exhaustiveness is what forces
/// a new asset to be wired in here before it can compile.
fn plan_for<R: Runtime>(
    app: &AppHandle<R>,
    request: &QueuedDownload,
) -> Result<AssetDownloadPlan<R>, String> {
    match request {
        QueuedDownload::WhisperModel => super::model::whisper_model_plan(app),
        QueuedDownload::WhisperVadModel => super::model::whisper_vad_model_plan(app),
        QueuedDownload::WhisperRuntime { version } => {
            super::runtime::whisper_runtime_plan(app, version)
        }
        QueuedDownload::Ffmpeg { reinstall } => super::ffmpeg::ffmpeg_plan(app, *reinstall),
        QueuedDownload::Ytdlp => super::ytdlp::ytdlp_plan(app),
        QueuedDownload::Alass => super::alass::alass_plan(app),
        QueuedDownload::Dictionary => super::dictionary::dictionary_plan(app),
        QueuedDownload::Mpv { reinstall } => super::mpv::mpv_plan(app, *reinstall),
    }
}

#[cfg(test)]
mod tests {
    use super::{AssetKind, DownloadQueue, QueuedDownload};

    /// The queue's own bookkeeping, without a Tauri app.
    ///
    /// `take_next` and `enqueue_download` need an `AppHandle` for the snapshot and the shell, so
    /// what is tested here is the decision each of them makes: whether a request is already
    /// wanted, and what a pop leaves behind. Those are where a queue goes wrong.
    fn queue_with(pending: &[QueuedDownload]) -> DownloadQueue {
        let mut queue = DownloadQueue::default();
        for request in pending {
            queue.pending.push_back(request.clone());
        }
        queue
    }

    /// Pressing Download twice must not queue the same thing twice.
    #[test]
    fn a_request_already_waiting_is_not_wanted_again() {
        let queue = queue_with(&[QueuedDownload::Ffmpeg { reinstall: false }, QueuedDownload::Ytdlp]);

        assert!(queue.already_wanted(&QueuedDownload::Ffmpeg { reinstall: false }));
        assert!(queue.already_wanted(&QueuedDownload::Ytdlp));
        assert!(!queue.already_wanted(&QueuedDownload::Alass));
    }

    /// The one being downloaded counts too, or a second press would queue it behind itself and
    /// download it twice in a row.
    #[test]
    fn the_active_request_is_not_wanted_again() {
        let queue = DownloadQueue {
            active: Some(QueuedDownload::Dictionary),
            ..Default::default()
        };

        assert!(queue.already_wanted(&QueuedDownload::Dictionary));
        assert!(!queue.already_wanted(&QueuedDownload::Ffmpeg { reinstall: false }));
    }

    /// Two runtime versions are two different requests — they install side by side — while two
    /// presses for the same version are one. This is why the queue holds requests rather than
    /// bare `AssetKind`s.
    #[test]
    fn runtime_requests_are_told_apart_by_version() {
        let queue = queue_with(&[QueuedDownload::WhisperRuntime {
            version: "v1.8.4".into(),
        }]);

        assert!(queue.already_wanted(&QueuedDownload::WhisperRuntime {
            version: "v1.8.4".into()
        }));
        assert!(!queue.already_wanted(&QueuedDownload::WhisperRuntime {
            version: "v1.9.2".into()
        }));
    }

    /// The model and the speech detector share a progress card by reporting the same kind, so
    /// `AssetKind` cannot tell them apart — but the queue must, or asking for one would look
    /// like asking for the other.
    #[test]
    fn the_model_and_the_speech_detector_are_separate_requests() {
        let queue = queue_with(&[QueuedDownload::WhisperModel]);

        assert!(!queue.already_wanted(&QueuedDownload::WhisperVadModel));
        assert_eq!(
            QueuedDownload::WhisperModel.kind(),
            QueuedDownload::WhisperVadModel.kind(),
            "they deliberately report as the same asset"
        );
        assert_eq!(QueuedDownload::WhisperVadModel.kind(), AssetKind::Model);
    }

    /// Every request reports as exactly one asset, and between them they cover all six cards —
    /// otherwise a download would run with no progress to show, which is the class of bug
    /// `AssetKind` was introduced to end.
    #[test]
    fn every_asset_card_is_reachable_from_some_request() {
        let requests = [
            QueuedDownload::WhisperModel,
            QueuedDownload::WhisperVadModel,
            QueuedDownload::WhisperRuntime {
                version: "v1.8.4".into(),
            },
            QueuedDownload::Ffmpeg { reinstall: false },
            QueuedDownload::Ytdlp,
            QueuedDownload::Alass,
            QueuedDownload::Dictionary,
            QueuedDownload::Mpv { reinstall: false },
        ];
        let mut kinds: Vec<AssetKind> = requests.iter().map(QueuedDownload::kind).collect();
        kinds.sort_by_key(|kind| kind.label());
        kinds.dedup();

        assert_eq!(kinds.len(), 7, "got {kinds:?}");
    }

    /// A reinstall is a different request from a download, so asking for one while the other is
    /// queued does not collapse into a single press. They differ in what the plan is allowed to
    /// skip, so treating them as the same request would silently turn a reinstall into a no-op.
    #[test]
    fn a_reinstall_is_not_a_duplicate_of_a_plain_download() {
        let queue = queue_with(&[QueuedDownload::Ffmpeg { reinstall: false }]);

        assert!(queue.already_wanted(&QueuedDownload::Ffmpeg { reinstall: false }));
        assert!(!queue.already_wanted(&QueuedDownload::Ffmpeg { reinstall: true }));
        assert_eq!(
            QueuedDownload::Ffmpeg { reinstall: true }.kind(),
            QueuedDownload::Ffmpeg { reinstall: false }.kind(),
            "both belong to the FFmpeg progress card"
        );
    }

    /// A busy message must name its own asset. They were briefly all "Another download is
    /// already in progress." — which describes the condition the queue exists to remove, not
    /// the one this check is about.
    #[test]
    fn each_busy_message_names_its_own_asset_and_not_another_download() {
        for request in [
            QueuedDownload::WhisperModel,
            QueuedDownload::WhisperVadModel,
            QueuedDownload::WhisperRuntime {
                version: "v1.8.4".into(),
            },
            QueuedDownload::Ffmpeg { reinstall: false },
            QueuedDownload::Ytdlp,
            QueuedDownload::Alass,
            QueuedDownload::Dictionary,
            QueuedDownload::Mpv { reinstall: false },
        ] {
            let message = request.busy_message();
            assert!(
                message.starts_with("Finish the current task before downloading "),
                "{request:?}: {message}"
            );
            assert!(
                !message.contains("already in progress"),
                "{request:?} describes another download rather than a busy app: {message}"
            );
        }
    }
}
