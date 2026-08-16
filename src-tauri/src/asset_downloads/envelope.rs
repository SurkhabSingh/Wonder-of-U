//! The parts of a download that are the same for every asset.
//!
//! Each downloader used to carry roughly a hundred lines of orchestration around eight to
//! forty lines of actual work — for `ytdlp.rs`, two thirds of the file. Refuse if the app is
//! busy, claim the single download slot, read the asset directory, set the shell to
//! "downloading", write a "starting" snapshot, spawn a named thread, **do the work**, write
//! "completed", release the slot, update the shell, log it, and a twenty-nine line error tail
//! that was structurally identical in all six. Only the bolded step differed.
//!
//! That lives here now, once. The rule for reading it: **the plan holds the sentences the
//! envelope prints; the closure holds the work.**
//!
//! Phase G is a boxed closure rather than a trait or an `enum` match, and the reason is the
//! assets themselves. The model runs *two* downloads under one slot; the runtime takes a
//! version parameter and builds its own URL; ffmpeg and the runtime skip entirely when a
//! runnable binary is already installed; the dictionary verifies a directory where the others
//! verify a binary, and the model verifies without removing on failure. Those differ in the
//! *shape* of the work, not just its parameters, so a `download_url` field would have needed a
//! `Vec` and then an escape hatch beside it. A trait would cost object-safety and six `impl`
//! blocks that are one method each; an `enum` match would move the copy-paste into this file
//! and couple it to all six assets.

use std::path::PathBuf;

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_runtime::{log_event, update_shell_snapshot},
    app_types::SharedShellState,
};

use super::asset::AssetKind;
use super::transfer::{
    download_file_to_path_with_progress, reset_model_download_control,
    update_model_download_snapshot, DownloadSlotGuard,
};

/// Why a download stopped.
///
/// The error tail has to tell "the user pressed Cancel" apart from "it broke", and it did that
/// by testing whether the message ended in `"download cancelled."` — a sentence the network
/// could plausibly hand back, and one that extraction never produced at all, so a cancel during
/// unpacking was reported as a failure. Six modules each re-derived that from one producer.
pub(super) enum DownloadFailure {
    Cancelled,
    Failed(String),
}

impl From<String> for DownloadFailure {
    fn from(message: String) -> Self {
        DownloadFailure::Failed(message)
    }
}

impl DownloadFailure {
    /// Reads the transfer's own cancellation signal.
    ///
    /// **This is the last place that knows about the magic string**, and it is here rather
    /// than in each downloader so that converting a module deletes one more copy of it. When
    /// the sixth module is converted, `download_file_to_path_with_progress` can return this
    /// type directly and this function goes away with it.
    fn from_transfer_error(error: String) -> Self {
        if error.ends_with("download cancelled.") {
            DownloadFailure::Cancelled
        } else {
            DownloadFailure::Failed(error)
        }
    }
}

/// What a finished install has to say for itself.
///
/// The envelope writes the snapshots, but only the asset knows what to put in them: the
/// dictionary discovers its own path during extraction, and the whisper runtime's wording
/// depends on whether detection came back ready afterwards. Returning the sentences — rather
/// than having the envelope guess them from the kind — is what lets the wording stay exactly
/// as it is today while the surrounding hundred lines are deleted.
pub(super) struct Installed {
    /// What the finished card points at, which is **not always what it started pointing at**.
    /// The dictionary begins by naming the archive it is fetching and ends by naming the
    /// dictionary root it found inside — a path that does not exist until extraction has run.
    /// yt-dlp and alass finish where they started and simply hand back the same path.
    pub(super) target_path: PathBuf,
    pub(super) completed_message: String,
    pub(super) shell_success_text: String,
    pub(super) log_details: serde_json::Value,
}

/// The handles phase G needs, and nothing else.
pub(super) struct DownloadContext<R: Runtime> {
    app: AppHandle<R>,
    kind: AssetKind,
}

impl<R: Runtime> DownloadContext<R> {
    /// One file, with progress, pause and cancel.
    ///
    /// `kind` and the byte bookkeeping come from the context, so a caller says only what and
    /// where. `label` stays a parameter because it is not always the asset's own name — the
    /// model names the model it chose, the runtime names its version.
    pub(super) fn fetch(
        &self,
        url: &str,
        target: &std::path::Path,
        label: &str,
    ) -> Result<(), DownloadFailure> {
        download_file_to_path_with_progress(&self.app, url, target, self.kind, label)
            .map_err(DownloadFailure::from_transfer_error)
    }

    /// For the two installs that do more than write files: the runtime activates the version
    /// it just fetched and refreshes whisper detection, and the model clears a stale override.
    /// Both write persisted settings, which needs the handle.
    pub(super) fn app(&self) -> &AppHandle<R> {
        &self.app
    }
}

/// Phase G: everything specific to one asset, and nothing else.
///
/// `FnOnce` because it consumes the paths it captured; `Send + 'static` because it is handed
/// to a worker thread. Named rather than written inline so the plan below reads as a list of
/// what a download *is* instead of one line of type signature.
type InstallStep<R> =
    Box<dyn FnOnce(&DownloadContext<R>) -> Result<Installed, DownloadFailure> + Send + 'static>;

/// One asset download, described completely enough that the envelope can run it without
/// knowing which asset it is.
///
/// The string fields are not an abstraction leak; they are the ten literals each module
/// already contained, gathered into one readable block. Carrying them verbatim is what makes a
/// conversion provable — the diff shows every sentence preserved — and it is why there are
/// *two* cancelled and *two* failed strings: the snapshot says "Model download failed" where
/// the shell says "Whisper model download failed", and collapsing them would silently reword
/// the interface.
pub(super) struct AssetDownloadPlan<R: Runtime> {
    pub(super) kind: AssetKind,
    /// Names the OS thread, so a stack trace says which download it belongs to.
    pub(super) thread_name: &'static str,
    /// Refused when the app is mid-task: "Finish the current task before downloading X."
    pub(super) shell_busy_message: String,
    /// Refused when another download holds the single slot.
    pub(super) slot_busy_message: String,
    /// The shell's status line while this runs.
    pub(super) shell_start_text: String,
    /// The card's message before the first byte arrives.
    pub(super) starting_message: String,
    /// What the card points at. Three assets show the binary they are installing, three show
    /// the archive they are fetching; the envelope must not decide which.
    pub(super) starting_target_path: PathBuf,
    pub(super) cancelled_message: String,
    pub(super) cancelled_shell_text: String,
    /// Joined with the error as `"{prefix}: {error}"`.
    pub(super) failed_message_prefix: String,
    pub(super) failed_shell_prefix: String,
    pub(super) success_log_event: &'static str,
    pub(super) failure_log_event: &'static str,
    /// Phase G — everything that is actually specific to this asset.
    pub(super) install: InstallStep<R>,
}

/// Runs a download to completion on its own thread, reporting every step.
///
/// Returns as soon as the worker is spawned — the caller's command returns immediately and the
/// interface follows the emitted snapshots from there. That is why the two control commands
/// return nothing: a bootstrap built here would be a second, unordered writer of the same
/// state.
pub(super) fn run_asset_download<R: Runtime>(
    app: &AppHandle<R>,
    plan: AssetDownloadPlan<R>,
) -> Result<(), String> {
    {
        let shell_state = app.state::<SharedShellState>();
        let shell = shell_state
            .0
            .lock()
            .map_err(|_| "Could not inspect the shell state.".to_string())?;
        if shell.phase != "idle" && shell.phase != "error" {
            return Err(plan.shell_busy_message);
        }
    }

    let download_slot = DownloadSlotGuard::acquire(app, &plan.slot_busy_message)?;

    update_shell_snapshot(app, |shell| {
        shell.phase = "downloading-model".into();
        shell.status_text = plan.shell_start_text;
        shell.started_at_ms = None;
        shell.current_recording_name = None;
    })?;

    let AssetDownloadPlan {
        kind,
        thread_name,
        starting_message,
        starting_target_path,
        cancelled_message,
        cancelled_shell_text,
        failed_message_prefix,
        failed_shell_prefix,
        success_log_event,
        failure_log_event,
        install,
        ..
    } = plan;

    update_model_download_snapshot(app, |snapshot| {
        snapshot.kind = Some(kind);
        snapshot.status = "starting".into();
        snapshot.message = starting_message;
        snapshot.downloaded_bytes = 0;
        snapshot.total_bytes = None;
        snapshot.progress_percent = None;
        snapshot.target_path = Some(starting_target_path.display().to_string());
    })?;

    let context = DownloadContext {
        app: app.clone(),
        kind,
    };

    std::thread::Builder::new()
        .name(thread_name.into())
        .spawn(move || {
            let app_handle = context.app.clone();
            match install(&context) {
                Ok(installed) => {
                    // Best-effort from here: the install itself succeeded, and failing to
                    // describe it must not be reported as a failed download.
                    let _ = update_model_download_snapshot(&app_handle, |snapshot| {
                        snapshot.kind = Some(kind);
                        snapshot.status = "completed".into();
                        snapshot.message = installed.completed_message;
                        snapshot.downloaded_bytes =
                            snapshot.total_bytes.unwrap_or(snapshot.downloaded_bytes);
                        snapshot.progress_percent = Some(100.0);
                        snapshot.target_path = Some(installed.target_path.display().to_string());
                    });
                    let _ = reset_model_download_control(&app_handle);
                    let _ = update_shell_snapshot(&app_handle, |shell| {
                        shell.phase = "idle".into();
                        shell.status_text = installed.shell_success_text;
                        shell.started_at_ms = None;
                    });
                    log_event(&app_handle, "INFO", success_log_event, installed.log_details);
                }
                Err(failure) => {
                    let cancelled = matches!(failure, DownloadFailure::Cancelled);
                    let error = match failure {
                        DownloadFailure::Cancelled => cancelled_message.clone(),
                        DownloadFailure::Failed(message) => message,
                    };
                    let _ = update_model_download_snapshot(&app_handle, |snapshot| {
                        snapshot.kind = Some(kind);
                        if cancelled {
                            snapshot.status = "cancelled".into();
                            snapshot.message = cancelled_message;
                        } else {
                            snapshot.status = "failed".into();
                            snapshot.message = format!("{failed_message_prefix}: {error}");
                        }
                    });
                    let _ = reset_model_download_control(&app_handle);
                    let _ = update_shell_snapshot(&app_handle, |shell| {
                        shell.phase = "idle".into();
                        shell.status_text = if cancelled {
                            cancelled_shell_text
                        } else {
                            format!("{failed_shell_prefix}: {error}")
                        };
                        shell.started_at_ms = None;
                    });
                    log_event(
                        &app_handle,
                        "ERROR",
                        failure_log_event,
                        serde_json::json!({ "message": error }),
                    );
                }
            }
        })
        .map_err(|error| error.to_string())?;

    download_slot.disarm();
    Ok(())
}
