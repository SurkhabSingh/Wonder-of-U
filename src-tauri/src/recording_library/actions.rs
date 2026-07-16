use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

#[cfg(not(target_os = "windows"))]
use std::process::Command;
use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_runtime::{build_app_bootstrap, emit_app_snapshot, log_event, update_shell_snapshot},
    app_state::write_persisted_data,
    app_types::{RecentRecording, RecordingActionItem, RecordingBatchResult, SharedPersistedState},
    translation_bridge::TranslationBridge,
};

use super::{find_recent_recording, selected_recordings, update_recent_recording};

/// Target language for the current translation pass. The bridge sends an empty
/// provider so the extension uses its own selected provider (Google/DeepL).
/// TODO: promote this to a user-configurable setting.
const TRANSLATION_TARGET_LANGUAGE: &str = "en";
/// Long enough to cover a chunked transcript (the extension translates a long
/// transcript in several passes) plus one lease requeue if the extension dies
/// mid-job. See `translation_bridge::LEASE_TIMEOUT`.
const TRANSLATION_TIMEOUT: Duration = Duration::from_secs(180);

fn remove_recording_from_history(
    recordings: &mut Vec<RecentRecording>,
    file_path: &str,
) -> Result<RecentRecording, String> {
    let index = recordings
        .iter()
        .position(|recording| recording.file_path == file_path)
        .ok_or_else(|| "The selected recording is no longer in the recent list.".to_string())?;
    Ok(recordings.remove(index))
}

/// True when `candidate` resolves to a location inside `root`. Both sides are
/// canonicalized (resolving `..`, symlinks, and Windows verbatim prefixes) so no
/// stored path can name a file outside the recordings folder by spelling. A side
/// that cannot be canonicalized is treated as outside — refused, not deleted.
///
/// Deliberately a copy of `import::path_is_within` rather than a shared helper:
/// that one guards where yt-dlp may WRITE, this one guards what we may UNLINK, and
/// the two must be able to tighten independently.
fn path_is_within(root: &Path, candidate: &Path) -> bool {
    match (root.canonicalize(), candidate.canonicalize()) {
        (Ok(root), Ok(candidate)) => candidate.starts_with(&root),
        _ => false,
    }
}

/// Unlinks a removed entry's files, refusing anything outside the recordings folder.
///
/// `reconcile_recording_history` adopts every audio file it finds in the recordings
/// folder, and that folder is a user-picked one: point it at a music library and
/// those tracks become entries. Nothing here can tell an adopted file from one we
/// created, and there is no recycle bin behind `remove_file` — so the containment
/// check is what bounds the blast radius of a stale entry, a hand-edited state file,
/// or an `output_directory` that has since moved, to the folder the user pointed us
/// at.
///
/// A path that is already gone is not an error (the delete has nothing to do) and
/// is not confined either — there is nothing left to protect.
fn delete_recording_files(
    recording: &RecentRecording,
    recordings_directory: &Path,
) -> Result<(), String> {
    let mut paths = HashSet::new();
    paths.extend(
        [
            Some(recording.file_path.as_str()),
            recording.transcript_path.as_deref(),
            recording.translation_path.as_deref(),
        ]
        .into_iter()
        .flatten(),
    );
    paths.extend(
        recording
            .transcripts
            .iter()
            .map(|transcript| transcript.file_path.as_str()),
    );

    for path in paths {
        let candidate = Path::new(path);
        if !candidate.exists() {
            continue;
        }

        if !path_is_within(recordings_directory, candidate) {
            return Err(format!(
                "Could not delete {path}: it is outside the recordings folder. It was removed from the library and left on disk."
            ));
        }

        match fs::remove_file(candidate) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Could not delete {path}: {error}")),
        }
    }

    Ok(())
}

pub(crate) fn playback_path(recording: &RecentRecording) -> Result<PathBuf, String> {
    let path = PathBuf::from(&recording.file_path);
    if path.exists() {
        Ok(path)
    } else {
        Err("The audio file is missing from disk.".into())
    }
}

fn failed_translation_item(recording: &RecentRecording, message: impl Into<String>) -> RecordingActionItem {
    RecordingActionItem {
        file_path: recording.file_path.clone(),
        status: "failed".into(),
        message: message.into(),
        note_id: recording.anki_note_id,
    }
}

fn translation_output_path(audio_path: &str, language: &str) -> PathBuf {
    let path = Path::new(audio_path);
    let stem = path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("recording");
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    directory.join(format!("{stem}.translation.{language}.txt"))
}

/// Translates a freshly created transcript, for the "translate after transcription"
/// setting. Returns a short note for the caller to append to its own message, or
/// `None` when there was nothing to say.
///
/// Translation is an optional extra here, never a reason for transcription to
/// fail: if the extension is not connected we skip immediately rather than block
/// the caller for the full translation timeout waiting for a worker that is not
/// there.
pub(crate) fn auto_translate_after_transcription<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
) -> Option<String> {
    let bridge = app.state::<TranslationBridge>();
    let bridge = bridge.inner();

    if !bridge.is_connected() {
        return Some(
            "Translation was skipped because the browser extension is not connected.".to_string(),
        );
    }

    // The audio is renamed when its first transcript lands, so the caller's path
    // is the only one that still resolves.
    let recording = find_recent_recording(app, file_path).ok()?;

    if recording.translation_path.is_some() {
        return None;
    }

    // The translate-after-transcription path never forces a re-translate; `force`
    // is only for the manual re-translate command.
    let provider = configured_translation_provider(app);
    let item = translate_single_recording(app, bridge, recording, false, &provider);

    match item.status.as_str() {
        "success" => Some("Translated.".to_string()),
        "skipped" => None,
        _ => Some(format!("Translation failed: {}", item.message)),
    }
}

/// The translation provider the user picked in Settings, sent with every job so
/// the extension routes on it. Falls back to the default if the settings lock is
/// somehow poisoned rather than failing the translation.
fn configured_translation_provider<R: Runtime>(app: &AppHandle<R>) -> String {
    app.state::<SharedPersistedState>()
        .0
        .lock()
        .map(|persisted| persisted.settings.translation.provider.clone())
        .unwrap_or_else(|_| crate::app_types::default_translation_provider())
}

fn translate_single_recording<R: Runtime>(
    app: &AppHandle<R>,
    bridge: &TranslationBridge,
    recording: RecentRecording,
    force: bool,
    provider: &str,
) -> RecordingActionItem {
    // `force` re-translates even recordings that already have a translation,
    // deterministically overwriting {stem}.translation.{lang}.txt.
    if !force && recording.translation_path.is_some() {
        return RecordingActionItem {
            file_path: recording.file_path,
            status: "skipped".into(),
            message: "Already translated.".into(),
            note_id: recording.anki_note_id,
        };
    }

    // Fail fast when no worker is connected: otherwise submit() queues a job that
    // no browser ever claims and await_result() blocks for the full 180s timeout,
    // leaving the UI stuck on the busy overlay. The auto-translate path already
    // guards this way; the manual/re-translate path must too.
    if !bridge.is_connected() {
        return failed_translation_item(
            &recording,
            "The browser extension is not connected. Open it in App Support mode, then try again.",
        );
    }

    let transcript_path = match recording.transcript_path.clone() {
        Some(path) => path,
        None => return failed_translation_item(&recording, "No transcript available to translate."),
    };

    let source_text = match fs::read_to_string(&transcript_path) {
        Ok(text) => text,
        Err(error) => {
            return failed_translation_item(&recording, format!("Could not read the transcript: {error}"))
        }
    };

    if source_text.trim().is_empty() {
        return failed_translation_item(&recording, "The transcript is empty.");
    }

    let source_lang = recording
        .transcript_language
        .clone()
        .unwrap_or_else(|| "auto".to_string());
    let job_id = match bridge.submit(
        source_text,
        source_lang,
        TRANSLATION_TARGET_LANGUAGE.to_string(),
        provider.to_string(),
    ) {
        Ok(id) => id,
        Err(error) => return failed_translation_item(&recording, error),
    };

    let translated = match bridge.await_result(&job_id, TRANSLATION_TIMEOUT) {
        Ok(text) => text,
        Err(error) => return failed_translation_item(&recording, error),
    };

    if translated.trim().is_empty() {
        return failed_translation_item(&recording, "The extension returned an empty translation.");
    }

    let output_path = translation_output_path(&recording.file_path, TRANSLATION_TARGET_LANGUAGE);
    if let Err(error) = fs::write(&output_path, translated.trim().as_bytes()) {
        return failed_translation_item(&recording, format!("Could not save the translation: {error}"));
    }

    let stored_path = output_path.display().to_string();
    let file_path = recording.file_path.clone();
    let note_id = recording.anki_note_id;

    if let Err(error) = update_recent_recording(app, &file_path, {
        let stored_path = stored_path.clone();
        move |recording| recording.translation_path = Some(stored_path)
    }) {
        return RecordingActionItem {
            file_path,
            status: "failed".into(),
            message: format!("Translation saved, but the history could not be updated: {error}"),
            note_id,
        };
    }

    log_event(
        app,
        "INFO",
        "recording.translated",
        serde_json::json!({ "filePath": file_path, "language": TRANSLATION_TARGET_LANGUAGE }),
    );

    RecordingActionItem {
        file_path,
        status: "success".into(),
        message: "Translated.".into(),
        note_id,
    }
}

pub(crate) fn delete_recording_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
) -> Result<(), String> {
    // The recordings folder is read under the same lock that removes the entry, so
    // the containment check below cannot be racing a settings change.
    let (removed_recording, recordings_directory) = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update the recording history.".to_string())?;
        let removed = remove_recording_from_history(&mut persisted.recent_recordings, file_path)?;
        let recordings_directory = PathBuf::from(&persisted.settings.output_directory);
        let snapshot = persisted.clone();
        drop(persisted);
        write_persisted_data(app, &snapshot)?;
        (removed, recordings_directory)
    };

    delete_recording_files(&removed_recording, &recordings_directory)?;

    log_event(
        app,
        "INFO",
        "recording.deleted",
        serde_json::json!({ "filePath": file_path }),
    );
    emit_app_snapshot(app);
    Ok(())
}

pub(crate) fn delete_recordings_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let mut items = Vec::new();

    for file_path in file_paths {
        match delete_recording_inner(app, &file_path) {
            Ok(()) => items.push(RecordingActionItem {
                file_path,
                status: "success".into(),
                message: "Deleted recording files.".into(),
                note_id: None,
            }),
            Err(error) => items.push(RecordingActionItem {
                file_path,
                status: "failed".into(),
                message: error,
                note_id: None,
            }),
        }
    }

    let success_count = items.iter().filter(|item| item.status == "success").count();
    let failed_count = items.iter().filter(|item| item.status == "failed").count();
    let message = format!("Delete finished: {success_count} deleted, {failed_count} failed.");

    update_shell_snapshot(app, |shell| {
        shell.status_text = message.clone();
        shell.transition_count += 1;
    })?;

    Ok(RecordingBatchResult {
        status: if failed_count == 0 {
            "completed"
        } else {
            "partial"
        }
        .into(),
        message,
        items,
        bootstrap: build_app_bootstrap(app)?,
    })
}

/// Opens a path in the user's default application, passing it to Win32 as DATA.
///
/// This used to be `cmd /C start "" <path>`, which was the only place in the app
/// where an argv element was handed back to a shell to re-parse. The name in that
/// path is attacker-chosen — the app records system audio, so a video the user
/// plays writes the transcript that `derive_transcript_stem` turns into the
/// filename — and Rust only quotes a Windows argv element that contains a space or
/// tab, so a stem like `a&calc&` reached `cmd` live and `calc` ran. Only the
/// default recordings folder having a space in its name stopped that today.
///
/// `ShellExecuteW` takes the path as one counted string and never parses it, so the
/// whole class is gone rather than one more character being added to a denylist.
/// It resolves the default verb exactly as `start` did, returns as soon as the
/// player is launched, and involves no console to hide.
///
/// A null `lpOperation` is what `start` uses: the file type's own default verb,
/// rather than an "open" that a type may not register.
///
/// COM: `ShellExecuteW` can delegate to a Shell extension that expects an
/// initialized apartment, and a Tauri command is not guaranteed to run on the
/// already-initialized main thread. `RPC_E_CHANGED_MODE` means this thread is
/// already in a different apartment — fine to proceed on, and it must NOT be
/// balanced with a `CoUninitialize`, which is why the flag is tracked.
#[cfg(target_os = "windows")]
fn open_with_default_application(path: &Path) -> Result<(), String> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Foundation::RPC_E_CHANGED_MODE,
        System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED},
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    };

    let wide_path = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<u16>>();

    // SAFETY: `wide_path` is null-terminated and outlives the call; every other
    // pointer is null, which each parameter documents as valid.
    let result = unsafe {
        let com = CoInitializeEx(std::ptr::null(), COINIT_APARTMENTTHREADED as u32);
        let owns_com = com != RPC_E_CHANGED_MODE;

        let instance = ShellExecuteW(
            std::ptr::null_mut(),
            std::ptr::null(),
            wide_path.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL as i32,
        );

        if owns_com {
            CoUninitialize();
        }

        instance as isize
    };

    // ShellExecuteW's legacy contract: a value greater than 32 is success, and
    // anything at or below it is an error code rather than an instance handle.
    if result > 32 {
        return Ok(());
    }

    // SE_ERR_NOASSOC (31) is the one a user can actually act on; the rest are
    // indistinguishable to them, so the code goes in the message for the log.
    Err(match result {
        31 => "No app is set up to play this file. Choose a default player for it in Windows.".to_string(),
        2 | 3 => "The audio file is missing from disk.".to_string(),
        code => format!("Windows could not open the recording (error {code})."),
    })
}

pub(crate) fn play_recording_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
) -> Result<(), String> {
    let recording = find_recent_recording(app, file_path)?;
    let path = playback_path(&recording)?;

    #[cfg(target_os = "windows")]
    {
        open_with_default_application(&path)?;
    }

    #[cfg(target_os = "macos")]
    {
        Command::new("open")
            .arg(&path)
            .spawn()
            .map_err(|error| error.to_string())?;
    }

    #[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
    {
        Command::new("xdg-open")
            .arg(&path)
            .spawn()
            .map_err(|error| error.to_string())?;
    }

    Ok(())
}

pub(crate) fn translate_recordings_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
    force: bool,
) -> Result<RecordingBatchResult, String> {
    let recordings = selected_recordings(app, file_paths)?;
    let bridge = app.state::<TranslationBridge>();
    let bridge = bridge.inner();

    if !bridge.is_connected() {
        let items = recordings
            .into_iter()
            .map(|recording| {
                failed_translation_item(
                    &recording,
                    "The browser extension is not connected. Open it and select \"App Support\" mode, then try again.",
                )
            })
            .collect::<Vec<_>>();
        let failed_count = items.len();

        return Ok(RecordingBatchResult {
            status: if failed_count == 0 {
                "completed"
            } else {
                "unavailable"
            }
            .into(),
            message: "Translation is unavailable because the browser extension is not connected in App Support mode.".to_string(),
            items,
            bootstrap: build_app_bootstrap(app)?,
        });
    }

    let provider = configured_translation_provider(app);
    let mut items = Vec::new();
    for recording in recordings {
        // Each job blocks for up to TRANSLATION_TIMEOUT, so a batch that loses the
        // extension part-way through would otherwise sit there timing out once per
        // remaining recording. Stop asking as soon as nobody is listening.
        if !bridge.is_connected() {
            items.push(failed_translation_item(
                &recording,
                "The browser extension disconnected before this recording was translated.",
            ));
            continue;
        }

        items.push(translate_single_recording(
            app, bridge, recording, force, &provider,
        ));
    }

    let success_count = items.iter().filter(|item| item.status == "success").count();
    let skipped_count = items.iter().filter(|item| item.status == "skipped").count();
    let failed_count = items.iter().filter(|item| item.status == "failed").count();

    let status = if failed_count == 0 {
        "completed"
    } else if success_count > 0 || skipped_count > 0 {
        "partial"
    } else {
        "unavailable"
    };

    Ok(RecordingBatchResult {
        status: status.into(),
        message: format!(
            "Translation finished: {success_count} translated, {skipped_count} skipped, {failed_count} failed."
        ),
        items,
        bootstrap: build_app_bootstrap(app)?,
    })
}

#[cfg(test)]
mod tests {
    use super::{delete_recording_files, path_is_within};
    use crate::app_types::{RecentRecording, RecordingTranscript};
    use std::{fs, path::Path};

    fn recording_at(audio_path: &Path) -> RecentRecording {
        RecentRecording {
            file_name: audio_path
                .file_name()
                .unwrap()
                .to_string_lossy()
                .to_string(),
            file_path: audio_path.display().to_string(),
            transcript_path: None,
            transcript_language: None,
            transcripts: Vec::new(),
            translation_path: None,
            anki_note_id: None,
            anki_deck_name: None,
            anki_note_type: None,
            anki_pushes: Vec::new(),
            furigana_applied: false,
            audio_deleted: false,
            duration_ms: 0,
            bytes_written: 0,
            created_at_ms: 0,
            source: None,
            source_url: None,
            title: None,
        }
    }

    #[test]
    fn a_recordings_own_files_are_all_deleted() {
        let dir = tempfile::tempdir().unwrap();
        let audio = dir.path().join("lesson.wav");
        let transcript = dir.path().join("lesson.transcript.txt");
        let translation = dir.path().join("lesson.translation.en.txt");
        fs::write(&audio, b"audio").unwrap();
        fs::write(&transcript, "transcript").unwrap();
        fs::write(&translation, "translation").unwrap();

        let mut recording = recording_at(&audio);
        recording.transcript_path = Some(transcript.display().to_string());
        recording.translation_path = Some(translation.display().to_string());
        recording.transcripts.push(RecordingTranscript {
            language: "ja".into(),
            file_path: transcript.display().to_string(),
            detected_language: Some("ja".into()),
            segments_path: None,
        });

        delete_recording_files(&recording, dir.path()).unwrap();

        assert!(!audio.exists());
        assert!(!transcript.exists());
        assert!(!translation.exists());
    }

    #[test]
    fn a_file_outside_the_recordings_folder_is_never_unlinked() {
        let recordings_dir = tempfile::tempdir().unwrap();
        let music_dir = tempfile::tempdir().unwrap();
        // What adoption plus a since-changed output_directory produces: an entry
        // whose audio is somebody's music library.
        let track = music_dir.path().join("track.mp3");
        fs::write(&track, b"music").unwrap();

        let error = delete_recording_files(&recording_at(&track), recordings_dir.path())
            .expect_err("a file outside the recordings folder must not be deleted");

        assert!(error.contains("outside the recordings folder"));
        assert!(track.exists());
    }

    #[test]
    fn a_traversal_path_does_not_escape_the_recordings_folder() {
        let root = tempfile::tempdir().unwrap();
        let recordings_dir = root.path().join("recordings");
        fs::create_dir(&recordings_dir).unwrap();
        let outside = root.path().join("private.wav");
        fs::write(&outside, b"audio").unwrap();

        // Spelled as if it were inside the recordings folder; canonicalization is
        // what refuses it.
        let traversal = recordings_dir.join("..").join("private.wav");
        let error = delete_recording_files(&recording_at(&traversal), &recordings_dir)
            .expect_err("a traversal path must not escape the recordings folder");

        assert!(error.contains("outside the recordings folder"));
        assert!(outside.exists());
    }

    #[test]
    fn an_already_missing_file_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        // Audio deleted after an Anki push, transcript still there.
        let audio = dir.path().join("gone.wav");
        let transcript = dir.path().join("gone.transcript.txt");
        fs::write(&transcript, "transcript").unwrap();

        let mut recording = recording_at(&audio);
        recording.transcript_path = Some(transcript.display().to_string());

        delete_recording_files(&recording, dir.path()).unwrap();

        assert!(!transcript.exists());
    }

    #[test]
    fn containment_refuses_paths_it_cannot_resolve() {
        let dir = tempfile::tempdir().unwrap();
        // A root that does not exist cannot be canonicalized, so nothing may be
        // judged inside it.
        assert!(!path_is_within(
            &dir.path().join("absent"),
            &dir.path().join("absent").join("file.wav")
        ));
    }
}
