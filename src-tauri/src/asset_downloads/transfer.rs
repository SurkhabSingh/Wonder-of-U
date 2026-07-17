use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::Duration,
};

use tauri::{AppHandle, Manager, Runtime};
use zip::ZipArchive;

use crate::{
    app_runtime::emit_app_snapshot,
    app_types::{ModelDownloadControlState, ModelDownloadSnapshot, ModelDownloadState},
};

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent("Wonder of U Desktop/0.1.0")
        .connect_timeout(Duration::from_secs(15))
        // Paused downloads intentionally keep the response open until the user resumes.
        .timeout(None)
        .build()
        .map_err(|error| error.to_string())
}

pub(super) fn update_model_download_snapshot<R: Runtime, F>(
    app: &AppHandle<R>,
    update: F,
) -> Result<(), String>
where
    F: FnOnce(&mut ModelDownloadSnapshot),
{
    let download_state = app.state::<ModelDownloadState>();
    let mut snapshot = download_state
        .0
        .lock()
        .map_err(|_| "Could not update the model download state.".to_string())?;
    update(&mut snapshot);
    drop(snapshot);
    emit_app_snapshot(app);
    Ok(())
}

pub(super) fn reset_model_download_control<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let control_state = app.state::<ModelDownloadControlState>();
    let mut control = control_state
        .control
        .lock()
        .map_err(|_| "Could not reset the model download control state.".to_string())?;
    control.active = false;
    control.paused = false;
    control.cancel_requested = false;
    control_state.condvar.notify_all();
    Ok(())
}

/// Owns the single asset-download control slot that every download shares.
///
/// Between claiming the slot and handing it to the worker thread there are several
/// fallible steps, and each one used to early-return with `active` still set — which
/// wedges every asset download behind "Another download is already in progress."
/// until the app restarts. Dropping the guard releases the slot, so any `?` on the
/// way to `spawn` unwinds cleanly. The worker thread resets the slot itself on both
/// its success and failure paths, so `disarm` hands ownership over once it is running.
pub(super) struct DownloadSlotGuard<R: Runtime> {
    app: AppHandle<R>,
    armed: bool,
}

impl<R: Runtime> DownloadSlotGuard<R> {
    /// Claims the slot, or fails with `busy_message` when another download holds it.
    pub(super) fn acquire(app: &AppHandle<R>, busy_message: &str) -> Result<Self, String> {
        let control_state = app.state::<ModelDownloadControlState>();
        let mut control = control_state
            .control
            .lock()
            .map_err(|_| "Could not initialize the download control state.".to_string())?;
        if control.active {
            return Err(busy_message.to_string());
        }
        control.active = true;
        control.paused = false;
        control.cancel_requested = false;
        drop(control);

        Ok(Self {
            app: app.clone(),
            armed: true,
        })
    }

    /// Gives up responsibility for the slot: the worker thread releases it from here on.
    pub(super) fn disarm(mut self) {
        self.armed = false;
    }
}

impl<R: Runtime> Drop for DownloadSlotGuard<R> {
    fn drop(&mut self) {
        if self.armed {
            let _ = reset_model_download_control(&self.app);
        }
    }
}

pub(super) fn ensure_directory_exists(path: &Path) -> Result<(), String> {
    fs::create_dir_all(path).map_err(|error| error.to_string())
}

/// Removes the `.part` file unless the transfer got as far as its final rename.
///
/// Cancel used to be the only path that cleaned up, so any read/write error left a
/// partial file behind in the asset directory forever. Declare this guard *before*
/// the `File` handle it protects: locals drop in reverse, so the file closes first
/// and the removal is not racing its own open handle.
struct PartialDownloadGuard {
    path: PathBuf,
    armed: bool,
}

impl PartialDownloadGuard {
    fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PartialDownloadGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_file(&self.path);
        }
    }
}

/// Verifies a freshly installed managed binary, deleting it when it will not run.
///
/// Detection trusts a managed binary by existence (see `managed_binary_is_present`),
/// so a binary left on disk after a failed `--version` probe would be reported as
/// ready and then spawned by a real import. A missing VC++ runtime, antivirus
/// tampering, or a complete-but-corrupt download all land here. The removal is
/// deliberately best-effort: it must never replace the verification error the user
/// needs to see.
pub(super) fn verify_managed_binary_or_remove<V>(
    executable_path: &Path,
    verify: V,
) -> Result<(), String>
where
    V: FnOnce(&Path) -> Result<(), String>,
{
    match verify(executable_path) {
        Ok(()) => Ok(()),
        Err(error) => {
            let _ = fs::remove_file(executable_path);
            Err(error)
        }
    }
}

/// Verifies a freshly installed managed directory, deleting it when it is not usable.
///
/// The binary sibling above cannot be reused for this: it removes with
/// `fs::remove_file`, which fails on a directory and would leave a broken install
/// exactly where detection trusts it. The removal is best-effort for the same
/// reason as the binary sibling — it must never replace the verification error.
pub(super) fn verify_managed_directory_or_remove<T, V>(
    directory_path: &Path,
    verify: V,
) -> Result<T, String>
where
    V: FnOnce(&Path) -> Result<T, String>,
{
    match verify(directory_path) {
        Ok(verified) => Ok(verified),
        Err(error) => {
            let _ = fs::remove_dir_all(directory_path);
            Err(error)
        }
    }
}

/// Removes a half-installed directory unless the install got as far as verifying.
///
/// Extraction writes an archive's entries in order, so an interrupted one leaves a
/// directory that is real but incomplete — and the lindera archive happens to write
/// its small `metadata.json` long before its 32MB `dict.words`, which is precisely
/// the file detection keys on. Without this, a download that died mid-extract would
/// be trusted as ready forever and fail on every use. Covers the cancel and
/// mid-extract paths; `verify_managed_directory_or_remove` covers a complete
/// install that still will not load.
pub(super) struct PartialInstallGuard {
    path: PathBuf,
    armed: bool,
}

impl PartialInstallGuard {
    pub(super) fn new(path: PathBuf) -> Self {
        Self { path, armed: true }
    }

    pub(super) fn disarm(&mut self) {
        self.armed = false;
    }
}

impl Drop for PartialInstallGuard {
    fn drop(&mut self) {
        if self.armed {
            let _ = fs::remove_dir_all(&self.path);
        }
    }
}

fn remove_directory_contents(path: &Path) -> Result<(), String> {
    if !path.exists() {
        return Ok(());
    }

    for entry in fs::read_dir(path).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let entry_path = entry.path();
        if entry
            .file_type()
            .map_err(|error| error.to_string())?
            .is_dir()
        {
            fs::remove_dir_all(&entry_path).map_err(|error| error.to_string())?;
        } else {
            fs::remove_file(&entry_path).map_err(|error| error.to_string())?;
        }
    }

    Ok(())
}

pub(super) fn extract_zip_archive_to_directory(
    archive_path: &Path,
    target_directory: &Path,
) -> Result<(), String> {
    ensure_directory_exists(target_directory)?;
    remove_directory_contents(target_directory)?;

    let archive_file = fs::File::open(archive_path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(archive_file).map_err(|error| error.to_string())?;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let Some(relative_path) = entry.enclosed_name() else {
            continue;
        };

        let output_path = target_directory.join(relative_path);
        if entry.is_dir() {
            ensure_directory_exists(&output_path)?;
            continue;
        }

        if let Some(parent) = output_path.parent() {
            ensure_directory_exists(parent)?;
        }

        let mut output_file = fs::File::create(&output_path).map_err(|error| error.to_string())?;
        std::io::copy(&mut entry, &mut output_file).map_err(|error| error.to_string())?;
    }

    Ok(())
}

pub(super) fn download_file_to_path_with_progress<R: Runtime>(
    app: &AppHandle<R>,
    url: &str,
    target_path: &Path,
    kind: &str,
    label: &str,
) -> Result<(), String> {
    let client = http_client()?;
    let mut response = client.get(url).send().map_err(|error| error.to_string())?;
    if !response.status().is_success() {
        return Err(format!("Download failed with status {}", response.status()));
    }

    let total_bytes = response.content_length();
    let temp_path = target_path.with_extension("part");
    let mut temp_guard = PartialDownloadGuard::new(temp_path.clone());
    let mut file = fs::File::create(&temp_path).map_err(|error| error.to_string())?;
    let mut buffer = [0u8; 64 * 1024];
    let mut downloaded_bytes = 0u64;

    update_model_download_snapshot(app, |snapshot| {
        snapshot.kind = Some(kind.to_string());
        snapshot.status = "downloading".into();
        snapshot.message = format!("Downloading {label}...");
        snapshot.downloaded_bytes = 0;
        snapshot.total_bytes = total_bytes;
        snapshot.progress_percent = total_bytes.map(|_| 0.0);
        snapshot.target_path = Some(target_path.display().to_string());
    })?;

    loop {
        {
            let control_state = app.state::<ModelDownloadControlState>();
            let mut control = control_state
                .control
                .lock()
                .map_err(|_| "Could not inspect the model download state.".to_string())?;

            while control.active && control.paused && !control.cancel_requested {
                drop(control);
                update_model_download_snapshot(app, |snapshot| {
                    snapshot.kind = Some(kind.to_string());
                    snapshot.status = "paused".into();
                    snapshot.message = format!("{label} download paused.");
                })?;
                control =
                    control_state
                        .condvar
                        .wait(control_state.control.lock().map_err(|_| {
                            "Could not resume the model download state.".to_string()
                        })?)
                        .map_err(|_| "Could not resume the model download state.".to_string())?;
            }

            if control.cancel_requested {
                drop(control);
                update_model_download_snapshot(app, |snapshot| {
                    snapshot.kind = Some(kind.to_string());
                    snapshot.status = "cancelled".into();
                    snapshot.message = format!("{label} download cancelled.");
                })?;
                reset_model_download_control(app)?;
                return Err(format!("{label} download cancelled."));
            }
        }

        let read_bytes = response
            .read(&mut buffer)
            .map_err(|error| error.to_string())?;
        if read_bytes == 0 {
            break;
        }

        file.write_all(&buffer[..read_bytes])
            .map_err(|error| error.to_string())?;
        downloaded_bytes = downloaded_bytes.saturating_add(read_bytes as u64);

        update_model_download_snapshot(app, |snapshot| {
            snapshot.kind = Some(kind.to_string());
            snapshot.status = "downloading".into();
            snapshot.message = format!("Downloading {label}...");
            snapshot.downloaded_bytes = downloaded_bytes;
            snapshot.total_bytes = total_bytes;
            snapshot.progress_percent = total_bytes.map(|total| {
                if total == 0 {
                    0.0
                } else {
                    (downloaded_bytes as f64 / total as f64) * 100.0
                }
            });
            snapshot.target_path = Some(target_path.display().to_string());
        })?;
    }

    fs::rename(&temp_path, target_path).map_err(|error| error.to_string())?;
    temp_guard.disarm();
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        verify_managed_binary_or_remove, verify_managed_directory_or_remove, PartialDownloadGuard,
        PartialInstallGuard,
    };
    use std::path::Path;

    /// Stands in for an extracted dictionary: a directory with files inside it.
    fn write_extracted_directory(root: &Path) -> std::path::PathBuf {
        let directory = root.join("lindera-ipadic");
        std::fs::create_dir_all(&directory).unwrap();
        std::fs::write(directory.join("metadata.json"), b"{}").unwrap();
        std::fs::write(directory.join("dict.words"), b"words").unwrap();
        directory
    }

    #[test]
    fn a_directory_that_fails_verification_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let directory = write_extracted_directory(dir.path());

        let error = verify_managed_directory_or_remove(&directory, |_: &Path| {
            Err::<(), String>("the dictionary did not load".to_string())
        })
        .unwrap_err();

        // The original failure survives, and detection can no longer trust the
        // dictionary — a `remove_file` here would have left it in place.
        assert_eq!(error, "the dictionary did not load");
        assert!(!directory.exists());
    }

    #[test]
    fn a_directory_that_verifies_is_left_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let directory = write_extracted_directory(dir.path());

        verify_managed_directory_or_remove(&directory, |_: &Path| Ok(())).unwrap();

        assert!(directory.join("metadata.json").exists());
    }

    #[test]
    fn a_failed_directory_removal_still_reports_the_verification_error() {
        let dir = tempfile::tempdir().unwrap();
        // Nothing to remove: the removal fails and must not mask the real error.
        let missing = dir.path().join("absent");

        let error = verify_managed_directory_or_remove(&missing, |_: &Path| {
            Err::<(), String>("no metadata".to_string())
        })
        .unwrap_err();

        assert_eq!(error, "no metadata");
    }

    #[test]
    fn a_half_extracted_install_is_removed_unless_the_guard_is_disarmed() {
        let dir = tempfile::tempdir().unwrap();

        // An extraction that died after metadata.json but before the word list:
        // detection keys on metadata.json, so this must not survive.
        let stranded = write_extracted_directory(dir.path());
        drop(PartialInstallGuard::new(stranded.clone()));
        assert!(!stranded.exists());

        // A verified install disarms the guard, so nothing is touched.
        let kept = write_extracted_directory(&dir.path().join("kept"));
        let mut guard = PartialInstallGuard::new(kept.clone());
        guard.disarm();
        drop(guard);
        assert!(kept.join("metadata.json").exists());
    }

    #[test]
    fn a_binary_that_fails_verification_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("yt-dlp.exe");
        std::fs::write(&binary, b"MZ...").unwrap();

        let error = verify_managed_binary_or_remove(&binary, |_: &Path| {
            Err("the binary did not run".to_string())
        })
        .unwrap_err();

        // The original failure survives, and detection can no longer trust the binary.
        assert_eq!(error, "the binary did not run");
        assert!(!binary.exists());
    }

    #[test]
    fn a_binary_that_verifies_is_left_in_place() {
        let dir = tempfile::tempdir().unwrap();
        let binary = dir.path().join("yt-dlp.exe");
        std::fs::write(&binary, b"MZ...").unwrap();

        verify_managed_binary_or_remove(&binary, |_: &Path| Ok(())).unwrap();

        assert!(binary.exists());
    }

    #[test]
    fn a_failed_removal_still_reports_the_verification_error() {
        let dir = tempfile::tempdir().unwrap();
        // Nothing to remove: the removal fails and must not mask the real error.
        let missing = dir.path().join("absent.exe");

        let error =
            verify_managed_binary_or_remove(&missing, |_: &Path| Err("no runtime".to_string()))
                .unwrap_err();

        assert_eq!(error, "no runtime");
    }

    #[test]
    fn a_partial_download_is_removed_unless_the_guard_is_disarmed() {
        let dir = tempfile::tempdir().unwrap();

        let stranded = dir.path().join("stranded.part");
        std::fs::write(&stranded, b"partial").unwrap();
        drop(PartialDownloadGuard::new(stranded.clone()));
        assert!(!stranded.exists());

        // A renamed-into-place download disarms the guard, so nothing is touched.
        let renamed = dir.path().join("kept.part");
        std::fs::write(&renamed, b"partial").unwrap();
        let mut guard = PartialDownloadGuard::new(renamed.clone());
        guard.disarm();
        drop(guard);
        assert!(renamed.exists());
    }
}
