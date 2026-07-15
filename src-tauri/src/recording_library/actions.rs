use std::{
    collections::HashSet,
    fs,
    path::{Path, PathBuf},
    process::Command,
    time::Duration,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_runtime::{build_app_bootstrap, emit_app_snapshot, log_event, update_shell_snapshot},
    app_state::write_persisted_data,
    app_types::{RecentRecording, RecordingActionItem, RecordingBatchResult, SharedPersistedState},
    translation_bridge::TranslationBridge,
};

use super::{find_recent_recording, selected_recordings, update_recent_recording};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

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

fn delete_recording_files(recording: &RecentRecording) -> Result<(), String> {
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
        match fs::remove_file(path) {
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
    let removed_recording = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update the recording history.".to_string())?;
        let removed = remove_recording_from_history(&mut persisted.recent_recordings, file_path)?;
        let snapshot = persisted.clone();
        drop(persisted);
        write_persisted_data(app, &snapshot)?;
        removed
    };

    delete_recording_files(&removed_recording)?;

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

pub(crate) fn play_recording_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
) -> Result<(), String> {
    let recording = find_recent_recording(app, file_path)?;
    let path = playback_path(&recording)?;

    #[cfg(target_os = "windows")]
    {
        let mut command = Command::new("cmd");
        command.creation_flags(CREATE_NO_WINDOW);
        command.arg("/C").arg("start").arg("").arg(&path);
        command.spawn().map_err(|error| error.to_string())?;
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
