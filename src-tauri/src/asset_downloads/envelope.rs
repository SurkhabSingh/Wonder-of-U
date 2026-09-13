use std::path::PathBuf;

use tauri::{AppHandle, Runtime};

use crate::app_runtime::{log_event, update_shell_snapshot};

use super::asset::AssetKind;
use super::transfer::{
    download_file_to_path_with_progress, reset_model_download_control,
    update_model_download_snapshot, DownloadSlotGuard,
};

/// Why a download stopped.
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
    fn from_transfer_error(error: String) -> Self {
        if error.ends_with("download cancelled.") {
            DownloadFailure::Cancelled
        } else {
            DownloadFailure::Failed(error)
        }
    }
}

/// What a finished install has to say for itself.
pub(super) struct Installed {
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
    pub(super) fn fetch(
        &self,
        url: &str,
        target: &std::path::Path,
        label: &str,
    ) -> Result<(), DownloadFailure> {
        download_file_to_path_with_progress(&self.app, url, target, self.kind, label)
            .map_err(DownloadFailure::from_transfer_error)
    }

    pub(super) fn app(&self) -> &AppHandle<R> {
        &self.app
    }
}

/// Phase G: everything specific to one asset, and nothing else.
type InstallStep<R> =
    Box<dyn FnOnce(&DownloadContext<R>) -> Result<Installed, DownloadFailure> + Send + 'static>;

/// One asset download, described completely enough that the envelope can run it without
/// knowing which asset it is.
pub(super) struct AssetDownloadPlan<R: Runtime> {
    pub(super) kind: AssetKind,
    pub(super) slot_busy_message: String,
    pub(super) shell_start_text: String,
    pub(super) starting_message: String,
    pub(super) starting_target_path: PathBuf,
    pub(super) cancelled_message: String,
    pub(super) cancelled_shell_text: String,
    pub(super) failed_message_prefix: String,
    pub(super) failed_shell_prefix: String,
    pub(super) success_log_event: &'static str,
    pub(super) failure_log_event: &'static str,
    pub(super) install: InstallStep<R>,
}

/// Runs one download to completion, on the calling thread.
pub(super) fn run_asset_download<R: Runtime>(
    app: &AppHandle<R>,
    plan: AssetDownloadPlan<R>,
) -> Result<(), DownloadFailure> {
    let download_slot = DownloadSlotGuard::acquire(app, &plan.slot_busy_message)?;

    update_shell_snapshot(app, |shell| {
        shell.status_text = plan.shell_start_text;
    })?;

    let AssetDownloadPlan {
        kind,
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

    let outcome = install(&context);
    download_slot.disarm();

    match outcome {
        Ok(installed) => {
            let _ = update_model_download_snapshot(app, |snapshot| {
                snapshot.kind = Some(kind);
                snapshot.status = "completed".into();
                snapshot.message = installed.completed_message;
                snapshot.downloaded_bytes =
                    snapshot.total_bytes.unwrap_or(snapshot.downloaded_bytes);
                snapshot.progress_percent = Some(100.0);
                snapshot.target_path = Some(installed.target_path.display().to_string());
            });
            let _ = reset_model_download_control(app);
            let _ = update_shell_snapshot(app, |shell| {
                shell.status_text = installed.shell_success_text;
            });
            log_event(app, "INFO", success_log_event, installed.log_details);
            Ok(())
        }
        Err(failure) => {
            let cancelled = matches!(failure, DownloadFailure::Cancelled);
            let error = match &failure {
                DownloadFailure::Cancelled => cancelled_message.clone(),
                DownloadFailure::Failed(message) => message.clone(),
            };
            let _ = update_model_download_snapshot(app, |snapshot| {
                snapshot.kind = Some(kind);
                if cancelled {
                    snapshot.status = "cancelled".into();
                    snapshot.message = cancelled_message;
                } else {
                    snapshot.status = "failed".into();
                    snapshot.message = format!("{failed_message_prefix}: {error}");
                }
            });
            let _ = reset_model_download_control(app);
            let _ = update_shell_snapshot(app, |shell| {
                shell.status_text = if cancelled {
                    cancelled_shell_text
                } else {
                    format!("{failed_shell_prefix}: {error}")
                };
            });
            // A cancel is the outcome the user asked for, so it is not an error. Logging it as
            // one made every count of real failures wrong, and the snapshot and the shell text
            // above already draw this distinction.
            log_event(
                app,
                if cancelled { "INFO" } else { "ERROR" },
                failure_log_event,
                serde_json::json!({ "message": error, "cancelled": cancelled }),
            );
            Err(failure)
        }
    }
}
