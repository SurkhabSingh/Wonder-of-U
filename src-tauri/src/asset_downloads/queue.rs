use std::collections::VecDeque;

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_runtime::update_shell_snapshot,
    app_types::{ModelDownloadQueueState, SharedShellState},
};

use super::asset::AssetKind;
use super::envelope::{run_asset_download, AssetDownloadPlan};
use super::transfer::update_model_download_snapshot;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum QueuedDownload {
    WhisperModel,
    WhisperVadModel,
    WhisperRuntime { version: String },
    Ffmpeg { reinstall: bool },
    Ytdlp,
    Alass,
    Dictionary,
    Mpv { reinstall: bool },
}

impl QueuedDownload {
    pub(crate) fn kind(&self) -> AssetKind {
        match self {
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
#[derive(Default)]
pub(crate) struct DownloadQueue {
    pending: VecDeque<QueuedDownload>,
    active: Option<QueuedDownload>,
    running: bool,
}

impl DownloadQueue {
    fn already_wanted(&self, request: &QueuedDownload) -> bool {
        self.active.as_ref() == Some(request) || self.pending.contains(request)
    }
}

/// Refuses while the app is busy with something else.
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
        let should_start = !queue.running;
        if should_start {
            queue.running = true;
        }
        should_start
    };

    if !should_start {
        publish_queue_depth(app);
        return Ok(());
    }

    if let Err(busy) = refuse_when_app_is_busy(app, request.busy_message()) {
        abandon_queue(app);
        return Err(busy);
    }

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
    Drained,
}

/// Takes the next request, or retires.
fn take_next<R: Runtime>(app: &AppHandle<R>) -> NextRequest {
    let state = app.state::<ModelDownloadQueueState>();
    let Ok(mut queue) = state.0.lock() else {
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
    let Ok(mut queue) = state.0.lock() else {
        return;
    };
    queue.pending.clear();
    queue.active = None;
    queue.running = false;
}

/// Tells the card how many requests are still waiting, so it can say "2 more queued".
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
