use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_runtime::{build_app_bootstrap, log_event, update_shell_snapshot},
    app_state::{derive_transcript_language_from_path, sanitize_recording_name},
    app_types::{
        transcript_language_key, RecentRecording, RecordingActionItem, RecordingBatchResult,
        RecordingTranscript, SharedPersistedState,
    },
    runtime_assets::refresh_whisper_detection_state,
    transcription::{run_whisper_transcription, WhisperTranscriptionRequest},
};

use super::update_recent_recording;

static OUTPUT_RENAME_LOCK: Mutex<()> = Mutex::new(());

fn selected_untranscribed_recordings<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
    language: &str,
) -> Result<Vec<RecentRecording>, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not read the recording history.".to_string())?;
    let recordings = if file_paths.is_empty() {
        persisted
            .recent_recordings
            .iter()
            .filter(|recording| !recording.has_transcript_for_language(language))
            .cloned()
            .collect()
    } else {
        file_paths
            .iter()
            .filter_map(|file_path| {
                persisted
                    .recent_recordings
                    .iter()
                    .find(|recording| recording.file_path == *file_path)
                    .cloned()
            })
            .collect()
    };

    Ok(recordings)
}

fn apply_transcription_result_to_recording<R: Runtime>(
    app: &AppHandle<R>,
    original_file_path: &str,
    mut recording: RecentRecording,
    transcript_path: PathBuf,
    requested_language: &str,
) -> Result<RecentRecording, String> {
    let language = transcript_language_key(requested_language);
    let audio_path = PathBuf::from(&recording.file_path);
    // A recording that already carries a transcript is being transcribed into an
    // additional language. Keep the audio file and its name untouched so the
    // recording's identity (and any current selection or pending Anki push that
    // references `file_path`) stays valid, and store this language's transcript
    // beside the audio under a language-tagged name.
    let already_transcribed =
        !recording.transcripts.is_empty() || recording.transcript_path.is_some();

    let final_transcript_path = if already_transcribed {
        match store_additional_language_transcript(&audio_path, &transcript_path, &language) {
            Ok(stored_transcript_path) => stored_transcript_path,
            Err(error) => {
                log_event(
                    app,
                    "ERROR",
                    "recording.store_additional_transcript_failed",
                    serde_json::json!({
                        "audioPath": recording.file_path,
                        "message": error
                    }),
                );
                transcript_path
            }
        }
    } else {
        // First transcript for this recording: derive friendly file names from the
        // transcript text and rename both the audio and transcript to match.
        let mut renamed_transcript_path = transcript_path;
        match rename_recording_outputs_from_transcript(
            &audio_path,
            &renamed_transcript_path,
            recording.created_at_ms,
        ) {
            Ok((renamed_audio_path, renamed_transcript)) => {
                recording.file_name = renamed_audio_path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("recording.wav")
                    .to_string();
                recording.file_path = renamed_audio_path.display().to_string();
                recording.bytes_written = fs::metadata(&renamed_audio_path)
                    .map(|metadata| metadata.len())
                    .unwrap_or(recording.bytes_written);
                renamed_transcript_path = renamed_transcript;
            }
            Err(error) => {
                log_event(
                    app,
                    "ERROR",
                    "recording.rename_from_transcript_failed",
                    serde_json::json!({
                        "audioPath": recording.file_path,
                        "message": error
                    }),
                );
            }
        }
        renamed_transcript_path
    };

    recording.transcript_path = Some(final_transcript_path.display().to_string());
    recording.transcript_language =
        derive_transcript_language_from_path(&final_transcript_path, requested_language);
    recording
        .transcripts
        .retain(|transcript| transcript.language != language);
    recording.transcripts.push(RecordingTranscript {
        language,
        file_path: final_transcript_path.display().to_string(),
        detected_language: recording.transcript_language.clone(),
    });

    let updated_recording = recording.clone();
    update_recent_recording(app, original_file_path, |recording| {
        *recording = updated_recording.clone();
    })?;

    Ok(recording)
}

/// Move a freshly generated transcript for an additional language next to the
/// audio file without renaming the audio. Returns the stored transcript path.
fn store_additional_language_transcript(
    audio_path: &Path,
    transcript_path: &Path,
    language: &str,
) -> Result<PathBuf, String> {
    let _rename_guard = OUTPUT_RENAME_LOCK
        .lock()
        .map_err(|_| "Could not reserve a transcript output name.".to_string())?;
    let parent = audio_path
        .parent()
        .ok_or_else(|| "The saved recording path did not have a parent folder.".to_string())?;
    let stem = audio_path
        .file_stem()
        .and_then(|value| value.to_str())
        .ok_or_else(|| "The saved recording path did not have a file name.".to_string())?;
    let language_tag = sanitize_language_tag(language);
    // Deterministic per-language name so re-transcribing the same language
    // overwrites its previous transcript instead of leaving orphans behind.
    let target = parent.join(format!("{stem}.{language_tag}.transcript.txt"));
    move_file(transcript_path, &target)?;
    Ok(target)
}

fn sanitize_language_tag(language: &str) -> String {
    let sanitized: String = language
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' {
                character
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "lang".into()
    } else {
        sanitized
    }
}

/// Move a file, tolerating a pre-existing destination (Windows `rename` fails on
/// an existing target) and cross-device moves (temp dir on a different volume
/// than the output directory) by falling back to copy + delete.
fn move_file(source: &Path, target: &Path) -> Result<(), String> {
    if target.exists() {
        fs::remove_file(target).map_err(|error| error.to_string())?;
    }
    match fs::rename(source, target) {
        Ok(()) => Ok(()),
        Err(_) => {
            fs::copy(source, target).map_err(|error| error.to_string())?;
            let _ = fs::remove_file(source);
            Ok(())
        }
    }
}

pub(crate) fn transcribe_recordings_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let settings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not inspect transcription settings.".to_string())?;
        persisted.settings.clone()
    };
    let whisper_detection = refresh_whisper_detection_state(app)?;
    if whisper_detection.status != "ready" {
        return Ok(RecordingBatchResult {
            status: "unavailable".into(),
            message: format!("Whisper is not ready yet: {}", whisper_detection.message),
            items: Vec::new(),
            bootstrap: build_app_bootstrap(app)?,
        });
    }

    let cli_path = PathBuf::from(
        whisper_detection
            .executable_path
            .clone()
            .unwrap_or_default(),
    );
    let model_path = PathBuf::from(whisper_detection.model_path.clone().unwrap_or_default());
    let language = transcript_language_key(&settings.whisper.language);
    let recordings = selected_untranscribed_recordings(app, file_paths, &language)?;
    let total = recordings.len();
    let mut items = Vec::new();

    for (index, recording) in recordings.into_iter().enumerate() {
        if recording.has_transcript_for_language(&language) {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "skipped".into(),
                message: format!("Already transcribed for {language}."),
                note_id: recording.anki_note_id,
            });
            continue;
        }

        let original_file_path = recording.file_path.clone();
        update_shell_snapshot(app, |shell| {
            shell.phase = "transcribing".into();
            shell.status_text = format!(
                "Transcribing {} of {}: {}",
                index + 1,
                total,
                recording.file_name
            );
            shell.started_at_ms = None;
            shell.current_recording_name = None;
            shell.last_output_path = Some(recording.file_path.clone());
        })?;

        let result = run_whisper_transcription(&WhisperTranscriptionRequest {
            cli_path: cli_path.clone(),
            model_path: model_path.clone(),
            audio_path: PathBuf::from(&recording.file_path),
            language: settings.whisper.language.clone(),
        })
        .and_then(|result| {
            apply_transcription_result_to_recording(
                app,
                &original_file_path,
                recording.clone(),
                result.transcript_path,
                &settings.whisper.language,
            )
        });

        match result {
            Ok(updated_recording) => {
                log_event(
                    app,
                    "INFO",
                    "transcription.saved",
                    serde_json::json!({
                        "audioPath": updated_recording.file_path,
                        "transcriptPath": updated_recording.transcript_path
                    }),
                );
                items.push(RecordingActionItem {
                    file_path: updated_recording.file_path,
                    status: "success".into(),
                    message: "Transcript created. WAV audio was kept for transcription accuracy."
                        .into(),
                    note_id: updated_recording.anki_note_id,
                });
            }
            Err(error) => {
                log_event(
                    app,
                    "ERROR",
                    "transcription.failed",
                    serde_json::json!({
                        "audioPath": original_file_path,
                        "message": error
                    }),
                );
                items.push(RecordingActionItem {
                    file_path: original_file_path,
                    status: "failed".into(),
                    message: error,
                    note_id: None,
                });
            }
        }
    }

    let success_count = items.iter().filter(|item| item.status == "success").count();
    let skipped_count = items.iter().filter(|item| item.status == "skipped").count();
    let failed_count = items.iter().filter(|item| item.status == "failed").count();
    let message = format!(
        "Transcription finished: {success_count} created, {skipped_count} skipped, {failed_count} failed."
    );

    update_shell_snapshot(app, |shell| {
        shell.phase = "idle".into();
        shell.status_text = message.clone();
        shell.started_at_ms = None;
        shell.current_recording_name = None;
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

fn derive_transcript_stem(transcript_path: &Path) -> Result<String, String> {
    let transcript = fs::read_to_string(transcript_path).map_err(|error| error.to_string())?;
    let collapsed = transcript.split_whitespace().collect::<Vec<_>>().join(" ");
    let shortened = collapsed.chars().take(10).collect::<String>();
    let sanitized = sanitize_recording_name(&shortened);
    if sanitized.is_empty() {
        return Err("The generated transcript title was empty.".into());
    }

    Ok(sanitized)
}

pub(crate) fn rename_recording_outputs_from_transcript(
    audio_path: &Path,
    transcript_path: &Path,
    recording_id: u64,
) -> Result<(PathBuf, PathBuf), String> {
    let _rename_guard = OUTPUT_RENAME_LOCK
        .lock()
        .map_err(|_| "Could not reserve unique recording output names.".to_string())?;
    let parent = audio_path
        .parent()
        .ok_or_else(|| "The saved recording path did not have a parent folder.".to_string())?;
    let new_stem = derive_transcript_stem(transcript_path)?;
    let timestamped_stem = format!("{new_stem}_{recording_id}");
    let (new_audio_path, new_transcript_path) =
        unique_recording_output_paths(parent, &timestamped_stem);

    fs::rename(audio_path, &new_audio_path).map_err(|error| error.to_string())?;
    if let Err(error) = fs::rename(transcript_path, &new_transcript_path) {
        let rollback_result = fs::rename(&new_audio_path, audio_path);
        return Err(match rollback_result {
            Ok(()) => error.to_string(),
            Err(rollback_error) => {
                format!("{error}. The audio rename also could not be rolled back: {rollback_error}")
            }
        });
    }

    Ok((new_audio_path, new_transcript_path))
}

fn unique_recording_output_paths(directory: &Path, file_stem: &str) -> (PathBuf, PathBuf) {
    let mut attempt = 0usize;
    loop {
        let candidate_stem = if attempt == 0 {
            file_stem.to_string()
        } else {
            format!("{file_stem}_{attempt}")
        };
        let audio_path = directory.join(format!("{candidate_stem}.wav"));
        let transcript_path = directory.join(format!("{candidate_stem}.transcript.txt"));

        if !audio_path.exists() && !transcript_path.exists() {
            return (audio_path, transcript_path);
        }

        attempt += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn additional_language_transcript_keeps_audio_and_tags_language() {
        let dir = tempfile::tempdir().unwrap();
        let audio_path = dir.path().join("hola_100.wav");
        fs::write(&audio_path, b"audio").unwrap();

        let temp_transcript = dir.path().join("whisper-temp.txt");
        fs::write(&temp_transcript, "bonjour le monde").unwrap();

        let stored =
            store_additional_language_transcript(&audio_path, &temp_transcript, "fr").unwrap();

        // The audio file is untouched, and the transcript lands beside it tagged
        // with the language.
        assert!(audio_path.exists(), "audio must not be renamed or removed");
        assert_eq!(stored, dir.path().join("hola_100.fr.transcript.txt"));
        assert_eq!(fs::read_to_string(&stored).unwrap(), "bonjour le monde");
        assert!(!temp_transcript.exists(), "temp source should be moved");
    }

    #[test]
    fn retranscribing_same_language_overwrites_without_orphans() {
        let dir = tempfile::tempdir().unwrap();
        let audio_path = dir.path().join("hola_100.wav");
        fs::write(&audio_path, b"audio").unwrap();

        let first = dir.path().join("first-temp.txt");
        fs::write(&first, "old text").unwrap();
        let first_stored =
            store_additional_language_transcript(&audio_path, &first, "es").unwrap();

        let second = dir.path().join("second-temp.txt");
        fs::write(&second, "new text").unwrap();
        let second_stored =
            store_additional_language_transcript(&audio_path, &second, "es").unwrap();

        assert_eq!(first_stored, second_stored);
        assert_eq!(fs::read_to_string(&second_stored).unwrap(), "new text");
        // Only one transcript file exists for the language.
        let transcripts = fs::read_dir(dir.path())
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .map(|name| name.ends_with(".transcript.txt"))
                    .unwrap_or(false)
            })
            .count();
        assert_eq!(transcripts, 1);
    }

    #[test]
    fn language_tags_are_filename_safe() {
        assert_eq!(sanitize_language_tag("es"), "es");
        assert_eq!(sanitize_language_tag("zh-hans"), "zh-hans");
        assert_eq!(sanitize_language_tag("a/b\\c"), "a_b_c");
        assert_eq!(sanitize_language_tag(""), "lang");
    }
}
