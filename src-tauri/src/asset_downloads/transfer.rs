use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

use tauri::{AppHandle, Manager, Runtime};
use zip::ZipArchive;

use crate::{
    app_runtime::{emit_app_snapshot, log_event},
    app_types::{
        ModelDownloadControlState, ModelDownloadSnapshot, ModelDownloadState, SharedPersistedState,
    },
};

use super::asset::{paused_message, AssetKind};

fn http_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .user_agent("Wonder of U Desktop/0.1.0")
        .connect_timeout(Duration::from_secs(15))
        .timeout(None)
        .build()
        .map_err(|error| error.to_string())
}

/// Records new download state without telling anyone.
fn write_download_snapshot<R: Runtime, F>(app: &AppHandle<R>, update: F) -> Result<(), String>
where
    F: FnOnce(&mut ModelDownloadSnapshot),
{
    let download_state = app.state::<ModelDownloadState>();
    let mut snapshot = download_state
        .0
        .lock()
        .map_err(|_| "Could not update the model download state.".to_string())?;
    update(&mut snapshot);
    Ok(())
}

pub(super) fn update_model_download_snapshot<R: Runtime, F>(
    app: &AppHandle<R>,
    update: F,
) -> Result<(), String>
where
    F: FnOnce(&mut ModelDownloadSnapshot),
{
    write_download_snapshot(app, update)?;
    emit_app_snapshot(app);
    Ok(())
}

/// Checks a downloaded file against a published digest.
pub(super) fn verify_sha256(path: &Path, expected: &str) -> Result<(), String> {
    use sha2::{Digest, Sha256};

    let mut file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut hasher = Sha256::new();
    std::io::copy(&mut file, &mut hasher).map_err(|error| error.to_string())?;
    let actual = format!("{:x}", hasher.finalize());

    if actual.eq_ignore_ascii_case(expected) {
        return Ok(());
    }
    Err(format!(
        "the download did not match its published checksum (expected {expected}, got {actual})"
    ))
}

/// How often a download in flight may announce its progress.
const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(200);

/// Rate-limits progress announcements for one transfer.
pub(super) struct ProgressEmitter {
    last_emit: Option<Instant>,
    interval: Duration,
}

impl ProgressEmitter {
    pub(super) fn new(interval: Duration) -> Self {
        Self {
            last_emit: None,
            interval,
        }
    }

    pub(super) fn should_emit(&mut self, now: Instant) -> bool {
        let due = match self.last_emit {
            None => true,
            Some(last) => now.duration_since(last) >= self.interval,
        };
        if due {
            self.last_emit = Some(now);
        }
        due
    }
}

/// A progress tick: always records the new byte count, announces it at most every interval.
fn update_download_progress<R: Runtime, F>(
    app: &AppHandle<R>,
    emitter: &mut ProgressEmitter,
    update: F,
) -> Result<(), String>
where
    F: FnOnce(&mut ModelDownloadSnapshot),
{
    write_download_snapshot(app, update)?;
    if emitter.should_emit(Instant::now()) {
        emit_app_snapshot(app);
    }
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
pub(super) struct DownloadSlotGuard<R: Runtime> {
    app: AppHandle<R>,
    armed: bool,
}

impl<R: Runtime> DownloadSlotGuard<R> {
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

/// Where managed assets live, as currently configured.
pub(super) fn asset_directory<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
    Ok(PathBuf::from(&persisted.settings.asset_directory))
}

/// Removes the `.part` file unless the transfer got as far as its final rename.
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

/// The first candidate that both exists and runs, with any that does not removed.
pub(super) fn first_runnable_binary<V>(candidates: Vec<PathBuf>, verify: V) -> Option<PathBuf>
where
    V: Fn(&Path) -> Result<(), String>,
{
    candidates.into_iter().find(|candidate| {
        candidate.exists() && verify_managed_binary_or_remove(candidate, &verify).is_ok()
    })
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
    extract_zip_archive_except(archive_path, target_directory, |_| false)
}

/// Extraction that can leave entries out.
pub(super) fn extract_zip_archive_except(
    archive_path: &Path,
    target_directory: &Path,
    skip: impl Fn(&str) -> bool,
) -> Result<(), String> {
    // Open and validate the archive BEFORE touching what is already installed.
    let archive_file = fs::File::open(archive_path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(archive_file).map_err(|error| error.to_string())?;

    ensure_directory_exists(target_directory)?;
    remove_directory_contents(target_directory)?;

    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|error| error.to_string())?;
        let Some(relative_path) = entry.enclosed_name() else {
            continue;
        };

        if relative_path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(&skip)
        {
            continue;
        }

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

/// Pulls a single named file out of an archive, ignoring everything else.
pub(super) fn extract_zip_entry_to_path(
    archive_path: &Path,
    target_path: &Path,
    select: impl Fn(&[String]) -> Option<String>,
) -> Result<(), String> {
    let archive_file = fs::File::open(archive_path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(archive_file).map_err(|error| error.to_string())?;

    let names = archive
        .file_names()
        .map(str::to_string)
        .collect::<Vec<String>>();
    let wanted = select(&names).ok_or_else(|| {
        "The download did not contain the expected program; the release layout may have changed."
            .to_string()
    })?;

    let mut entry = archive
        .by_name(&wanted)
        .map_err(|error| format!("Could not read {wanted} from the download: {error}"))?;
    if entry.enclosed_name().is_none() {
        return Err("The download contained an unsafe file path.".into());
    }

    if let Some(parent) = target_path.parent() {
        ensure_directory_exists(parent)?;
    }
    let mut output_file = fs::File::create(target_path).map_err(|error| error.to_string())?;
    std::io::copy(&mut entry, &mut output_file).map_err(|error| error.to_string())?;
    Ok(())
}

pub(super) fn download_file_to_path_with_progress<R: Runtime>(
    app: &AppHandle<R>,
    url: &str,
    target_path: &Path,
    kind: AssetKind,
    label: &str,
) -> Result<(), String> {
    let client = http_client()?;
    let mut response = client.get(url).send().map_err(|error| {
        log_event(
            app,
            "WARN",
            "download.request_failed",
            serde_json::json!({ "url": url, "message": error.to_string() }),
        );
        "Could not reach the download server. Check your internet connection and try again."
            .to_string()
    })?;
    if !response.status().is_success() {
        return Err(format!("Download failed with status {}", response.status()));
    }

    let total_bytes = response.content_length();
    let temp_path = target_path.with_extension("part");
    let mut temp_guard = PartialDownloadGuard::new(temp_path.clone());
    let mut file = fs::File::create(&temp_path).map_err(|error| error.to_string())?;
    let mut buffer = [0u8; 64 * 1024];
    let mut downloaded_bytes = 0u64;
    let mut progress_emitter = ProgressEmitter::new(PROGRESS_EMIT_INTERVAL);

    update_model_download_snapshot(app, |snapshot| {
        snapshot.kind = Some(kind);
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
                    snapshot.kind = Some(kind);
                    snapshot.status = "paused".into();
                    snapshot.message = paused_message(kind.label());
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
                    snapshot.kind = Some(kind);
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

        update_download_progress(app, &mut progress_emitter, |snapshot| {
            snapshot.kind = Some(kind);
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
        first_runnable_binary, verify_managed_binary_or_remove, PartialDownloadGuard,
        ProgressEmitter,
    };
    use std::{
        path::Path,
        time::{Duration, Instant},
    };

    /// A download must say something at once, or it looks dead for its first interval.
    #[test]
    fn the_first_tick_always_announces_itself() {
        let mut emitter = ProgressEmitter::new(Duration::from_millis(200));
        assert!(emitter.should_emit(Instant::now()));
    }

    /// The whole point: chunks arriving faster than the interval are recorded but silent.
    /// Each announcement rebuilds and ships the entire ~67 KB bootstrap.
    #[test]
    fn ticks_inside_the_interval_stay_quiet() {
        let start = Instant::now();
        let mut emitter = ProgressEmitter::new(Duration::from_millis(200));

        assert!(emitter.should_emit(start));
        assert!(!emitter.should_emit(start + Duration::from_millis(1)));
        assert!(!emitter.should_emit(start + Duration::from_millis(199)));
    }

    /// ...and it must still announce once the interval is up, or the bar would freeze.
    #[test]
    fn a_tick_past_the_interval_announces_again() {
        let start = Instant::now();
        let mut emitter = ProgressEmitter::new(Duration::from_millis(200));

        assert!(emitter.should_emit(start));
        assert!(emitter.should_emit(start + Duration::from_millis(200)));
        assert!(emitter.should_emit(start + Duration::from_millis(400)));
    }

    /// The interval is measured from the last ANNOUNCEMENT, not the last tick. Measuring from
    /// the last tick would let a steady stream of quiet chunks hold the announcement off for
    /// the whole download, which is the failure this replaced.
    #[test]
    fn a_steady_stream_of_quiet_ticks_cannot_starve_the_next_announcement() {
        let start = Instant::now();
        let mut emitter = ProgressEmitter::new(Duration::from_millis(200));

        assert!(emitter.should_emit(start));
        for millis in [50, 100, 150] {
            assert!(!emitter.should_emit(start + Duration::from_millis(millis)));
        }
        assert!(emitter.should_emit(start + Duration::from_millis(200)));
    }

    /// A 73 MB ffmpeg download is 1,174 chunks. At five announcements a second it should cost
    /// a couple of hundred, not one per chunk.
    #[test]
    fn an_ffmpeg_sized_download_announces_a_few_hundred_times_not_a_few_thousand() {
        let start = Instant::now();
        let mut emitter = ProgressEmitter::new(Duration::from_millis(200));

        // 1,174 chunks spread evenly across a 30-second download. u64 throughout: the
        // microsecond product overflows a 32-bit index well before the last chunk.
        let chunks: u64 = 1_174;
        let announced = (0..chunks)
            .filter(|index| {
                let elapsed = Duration::from_micros(index * 30_000_000 / chunks);
                emitter.should_emit(start + elapsed)
            })
            .count();

        assert!(
            (140..=160).contains(&announced),
            "expected ~150 announcements across 30s, got {announced}"
        );
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

    /// The case that had no way out: the only installed binary does not run, so the
    /// search must report nothing found and let the download proceed to replace it.
    #[test]
    fn a_candidate_that_does_not_run_is_not_treated_as_installed() {
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("whisper-cli.exe");
        std::fs::write(&broken, b"MZ...").unwrap();

        let found = first_runnable_binary(vec![broken.clone()], |_: &Path| {
            Err("a DLL beside it is missing".to_string())
        });

        assert!(
            found.is_none(),
            "a binary that cannot run is not one we have"
        );
        // Removed, so detection stops reporting a runtime that cannot transcribe.
        assert!(!broken.exists());
    }

    /// A broken candidate must not hide a working one further down the list.
    #[test]
    fn the_search_passes_over_a_broken_candidate_to_a_working_one() {
        let dir = tempfile::tempdir().unwrap();
        let broken = dir.path().join("v1/whisper-cli.exe");
        let working = dir.path().join("v2/whisper-cli.exe");
        for path in [&broken, &working] {
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, b"MZ...").unwrap();
        }

        let working_for_check = working.clone();
        let found = first_runnable_binary(vec![broken.clone(), working.clone()], move |path| {
            if path == working_for_check {
                Ok(())
            } else {
                Err("did not run".to_string())
            }
        });

        assert_eq!(found.as_deref(), Some(working.as_path()));
        assert!(!broken.exists());
        assert!(working.exists());
    }

    /// A path that was never there is not an error, and nothing is spawned for it.
    #[test]
    fn absent_candidates_are_skipped_without_being_run() {
        let dir = tempfile::tempdir().unwrap();
        let missing = dir.path().join("not-installed.exe");

        let found = first_runnable_binary(vec![missing], |_: &Path| {
            panic!("verification must not run for a path that does not exist")
        });

        assert!(found.is_none());
    }
}

/// Removes a half-installed directory unless the install got as far as verifying.
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

/// Verifies a freshly installed managed directory, deleting it when it is not usable.
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

#[cfg(test)]
mod extraction_ordering_tests {
    use super::{extract_zip_archive_except, extract_zip_archive_to_directory, verify_sha256};
    use std::fs;

    /// A digest that matches passes, and one that does not is refused.
    #[test]
    fn a_download_is_checked_against_its_published_digest() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("payload.bin");
        fs::write(&path, b"wonder of u").unwrap();

        // Both literal, so the test states the answer rather than recomputing it with the
        // same library it is checking.
        let digest = "841b102561af1b399e7d212daaa0a7e7da79953e4cfa9ee349bd5b1c314f30b0";
        let other_digest = "0d3b0b1cbe0e6a05e2c56ba9e3b3bd3f88ea0f61f5e3e0a3cbd6ea5e2c9d3aa4";

        assert!(verify_sha256(&path, digest).is_ok(), "the file's own digest must pass");
        assert!(
            verify_sha256(&path, &digest.to_uppercase()).is_ok(),
            "case must not decide whether a download is trusted"
        );
        assert!(
            verify_sha256(&path, other_digest).is_err(),
            "any other digest must be refused"
        );

        let reason = verify_sha256(&path, other_digest).unwrap_err();
        assert!(reason.contains(other_digest), "the reason names what was expected: {reason}");
    }

    #[test]
    fn a_missing_file_fails_the_digest_check_rather_than_passing_it() {
        let directory = tempfile::tempdir().unwrap();
        assert!(verify_sha256(&directory.path().join("absent.bin"), "abc").is_err());
    }

    /// One archive carries debug symbols several times the size of everything else, so
    /// extraction can leave an entry out rather than write it and delete it.
    #[test]
    fn extraction_can_skip_an_entry_and_keeps_the_rest() {
        let staging = tempfile::tempdir().unwrap();
        let install = tempfile::tempdir().unwrap();
        let archive_path = staging.path().join("bundle.zip");

        let file = fs::File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        // The nested entry is what makes this test able to fail: with every entry at the
        // root, a predicate given the whole relative path behaves identically to one given the
        // file name, and the test cannot tell the two apart.
        for name in ["mpv.exe", "mpv.pdb", "vulkan-1.dll", "nested/mpv.pdb"] {
            zip.start_file(name, zip::write::SimpleFileOptions::default())
                .unwrap();
            std::io::Write::write_all(&mut zip, name.as_bytes()).unwrap();
        }
        zip.finish().unwrap();

        extract_zip_archive_except(&archive_path, install.path(), |name| {
            name.eq_ignore_ascii_case("mpv.pdb")
        })
        .unwrap();

        assert!(install.path().join("mpv.exe").exists());
        assert!(install.path().join("vulkan-1.dll").exists(), "only the named entry is skipped");
        assert!(!install.path().join("mpv.pdb").exists(), "the skipped entry was written");
        assert!(
            !install.path().join("nested").join("mpv.pdb").exists(),
            "the predicate is given a file name, so a nested copy is skipped too"
        );
    }


    /// A bad archive must not cost the user what they already had.
    #[test]
    fn a_corrupt_archive_leaves_the_installed_files_alone() {
        let staging = tempfile::tempdir().unwrap();
        let install = tempfile::tempdir().unwrap();

        let archive_path = staging.path().join("broken.zip");
        fs::write(&archive_path, b"this is not a zip file").unwrap();

        let survivor = install.path().join("ffmpeg.exe");
        fs::write(&survivor, b"the working copy").unwrap();

        let result = extract_zip_archive_to_directory(&archive_path, install.path());

        assert!(result.is_err(), "a non-zip must not report success");
        assert!(
            survivor.exists(),
            "the installed copy was destroyed by an extraction that never ran"
        );
        assert_eq!(fs::read(&survivor).unwrap(), b"the working copy");
    }

    /// The other half of the same rule: a *good* archive still replaces what was there, so this
    /// cannot be "fixed" by never clearing the directory.
    #[test]
    fn a_good_archive_still_replaces_the_previous_install() {
        let staging = tempfile::tempdir().unwrap();
        let install = tempfile::tempdir().unwrap();

        let archive_path = staging.path().join("good.zip");
        let file = fs::File::create(&archive_path).unwrap();
        let mut zip = zip::ZipWriter::new(file);
        zip.start_file("fresh.txt", zip::write::SimpleFileOptions::default())
            .unwrap();
        std::io::Write::write_all(&mut zip, b"new").unwrap();
        zip.finish().unwrap();

        let stale = install.path().join("stale.txt");
        fs::write(&stale, b"old").unwrap();

        extract_zip_archive_to_directory(&archive_path, install.path()).unwrap();

        assert!(!stale.exists(), "the previous install was not cleared");
        assert!(install.path().join("fresh.txt").exists());
    }
}
