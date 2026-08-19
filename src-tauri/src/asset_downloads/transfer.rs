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
        // Paused downloads intentionally keep the response open until the user resumes.
        .timeout(None)
        .build()
        .map_err(|error| error.to_string())
}

/// Records new download state without telling anyone.
///
/// Separated from the emit because the byte loop needs to keep the state exact — every chunk —
/// while announcing it far less often. See `ProgressEmitter`.
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

/// How often a download in flight may announce its progress.
///
/// An emit is not cheap. `emit_app_snapshot` rebuilds the *entire* bootstrap — it deep-clones
/// the persisted state, runs four live detections including a recursive directory walk for
/// ffmpeg, serialises the lot, and pushes it across the IPC boundary, where React re-renders
/// the whole app. The measured payload floor is the size of `state.json`: **67 KB, every time**.
///
/// The byte loop was calling that once per 64 KB chunk, so the cost scaled with the download:
///
/// | asset      | size   | emits | JSON pushed |
/// |------------|--------|-------|-------------|
/// | yt-dlp     |  18 MB |   288 |      19 MB  |
/// | alass      |  25 MB |   403 |      26 MB  |
/// | ffmpeg     |  73 MB | 1,174 |      77 MB  |
/// | model      | 466 MB | 7,456 |     487 MB  |
///
/// Past roughly 400 emits the webview cannot drain the queue as fast as it fills, so events
/// back up and the interface falls behind the download it is describing. That is what made
/// ffmpeg look broken while the smaller assets seemed fine: **nothing about ffmpeg differs —
/// it was simply the largest thing anyone had paused.** Pressing Pause enqueued a "paused"
/// behind hundreds of stale "downloading"s, so the button kept its old label, and pressing it
/// again just toggled the download back on.
///
/// 200ms caps it at five emits a second whatever the size, which is well inside what the eye
/// reads as a live progress bar.
const PROGRESS_EMIT_INTERVAL: Duration = Duration::from_millis(200);

/// Rate-limits progress announcements for one transfer.
///
/// Owned by the byte loop rather than kept globally, so there is no clock to reset between
/// downloads and no state shared across them. `now` is a parameter so the policy can be tested
/// without sleeping.
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

    /// True when enough time has passed. The first call is always true, so a download says
    /// something immediately rather than appearing dead for its first interval.
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
///
/// **Only in-flight progress may be throttled.** A status *transition* — starting, paused,
/// cancelled, completed, failed — must go out immediately, because those are exactly the
/// moments the interface has to react to, and some of them are the last thing a download ever
/// says. Pause in particular emits once and then blocks, so a swallowed pause would never be
/// followed by anything to correct it.
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

/// Where managed assets live, as currently configured.
///
/// Lock, read one field, unlock. It was written out longhand twelve times across the six
/// downloaders — three of them reading it twice on the calling thread and once more inside the
/// worker, because the first read had not been moved in.
pub(super) fn asset_directory<R: Runtime>(app: &AppHandle<R>) -> Result<PathBuf, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not inspect the current app settings.".to_string())?;
    Ok(PathBuf::from(&persisted.settings.asset_directory))
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

/// The first candidate that both exists and runs, with any that does not removed.
///
/// This is what "we already have it" has to mean before a download may be skipped.
/// Existence alone was the test, and the downloads that would have repaired a broken
/// binary were the very thing it suppressed — worst for the whisper runtime, where
/// nothing removed the file afterwards either, so the failure repeated on every retry
/// with no way out through the interface.
///
/// Removal is what keeps detection honest, since it tests existence as well and would
/// otherwise report a runtime that cannot transcribe as ready.
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
    // Open and validate the archive BEFORE touching what is already installed.
    //
    // The wipe used to come first, so a truncated or corrupt download destroyed a working
    // install and replaced it with nothing. That is survivable when the directory is empty —
    // which it always was, because the only caller reaching extraction had found nothing
    // installed — but the FFmpeg reinstall deliberately runs over a copy that works, and the
    // `latest` archive it fetches is republished daily and can be read mid-republish.
    //
    // Validating first does not make extraction atomic: a failure partway through still leaves
    // a half-unpacked directory. It removes the cause that can be removed by ordering alone,
    // and it costs two moved lines.
    let archive_file = fs::File::open(archive_path).map_err(|error| error.to_string())?;
    let mut archive = ZipArchive::new(archive_file).map_err(|error| error.to_string())?;

    ensure_directory_exists(target_directory)?;
    remove_directory_contents(target_directory)?;

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

/// Pulls a single named file out of an archive, ignoring everything else.
///
/// The sibling above unpacks the whole thing, which is right for ffmpeg and wrong for alass:
/// alass's release carries its own complete copy of ffmpeg, ~70 MB the app already has a
/// better-managed version of. `select` is handed the entry names and returns the one wanted,
/// so the choice stays in the module that understands the archive.
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
    // `enclosed_name` is what rejects `../` traversal in the archive; a name that fails it is
    // not a file we are willing to write anywhere.
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

/// `kind` says which asset this is; `label` is the wording for this particular transfer,
/// which is not always the asset's own name — the model download names the chosen model and
/// the runtime names its version. They were two adjacent `&str` parameters, so a call site
/// could swap them and still compile; typing `kind` makes that swap impossible.
pub(super) fn download_file_to_path_with_progress<R: Runtime>(
    app: &AppHandle<R>,
    url: &str,
    target_path: &Path,
    kind: AssetKind,
    label: &str,
) -> Result<(), String> {
    let client = http_client()?;
    // The raw error here is a reqwest one, and it renders as "error sending request for url
    // (https://huggingface.co/.../ggml-large-v3.bin)". That was fine while it only ever reached
    // a settings card; the first-run card put it on the landing page, URL and all, where it
    // told a user with no connection nothing they could act on.
    //
    // Replaced at the point it is produced rather than filtered at the point it is shown, so
    // every other failure in this module — "ffmpeg.exe was not found", a bad zip — keeps its
    // own already-readable sentence. The original is still logged by the caller.
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

    // The transition into "downloading" is not throttled — only the per-chunk ticks below are.
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
                    // Shared with `control.rs`, which writes this same message the instant the
                    // user presses Pause. Both land; wording them separately made the card
                    // change its mind about what it was doing.
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

        // The hot path: once per 64KB. Recorded every time, announced at most five times a
        // second — see PROGRESS_EMIT_INTERVAL for what an announcement actually costs.
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

#[cfg(test)]
mod extraction_ordering_tests {
    use super::extract_zip_archive_to_directory;
    use std::fs;

    /// A bad archive must not cost the user what they already had.
    ///
    /// The wipe used to run before the archive was opened, so a truncated or corrupt download
    /// emptied the install directory and then failed — leaving nothing installed. Harmless while
    /// the only caller reaching extraction had found nothing installed, and not harmless at all
    /// once the FFmpeg reinstall began running deliberately over a copy that works.
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
