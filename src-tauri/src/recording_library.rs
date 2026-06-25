use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_state::{
        derive_transcript_language_from_path, sanitize_recording_name, write_persisted_data,
    },
    app_types::{RecentRecording, RecordingActionItem, RecordingBatchResult, SharedPersistedState},
    build_app_bootstrap, emit_app_snapshot, log_event,
    runtime_assets::{detect_local_ffmpeg, refresh_whisper_detection_state},
    transcription::{run_whisper_transcription, WhisperTranscriptionRequest},
    update_shell_snapshot,
};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;
static OUTPUT_RENAME_LOCK: Mutex<()> = Mutex::new(());

fn hide_command_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

fn find_recent_recording<R: Runtime>(
    app: &AppHandle<R>,
    file_path: &str,
) -> Result<RecentRecording, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not read the recording history.".to_string())?;
    persisted
        .recent_recordings
        .iter()
        .find(|recording| recording.file_path == file_path)
        .cloned()
        .ok_or_else(|| "The selected recording is no longer in the recent list.".to_string())
}

pub(crate) fn update_recent_recording<R: Runtime, F>(
    app: &AppHandle<R>,
    file_path: &str,
    update: F,
) -> Result<(), String>
where
    F: FnOnce(&mut RecentRecording),
{
    let persisted_snapshot = {
        let persisted_state = app.state::<SharedPersistedState>();
        let mut persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not update the recording history.".to_string())?;
        let recording = persisted
            .recent_recordings
            .iter_mut()
            .find(|recording| recording.file_path == file_path)
            .ok_or_else(|| "The selected recording is no longer in the recent list.".to_string())?;
        update(recording);
        persisted.clone()
    };

    write_persisted_data(app, &persisted_snapshot)?;
    emit_app_snapshot(app);
    Ok(())
}

pub(crate) fn selected_recordings<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
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
            .filter(|recording| recording.transcript_path.is_some())
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

fn selected_untranscribed_recordings<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
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
            .filter(|recording| recording.transcript_path.is_none())
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
    let mut audio_path = PathBuf::from(&recording.file_path);
    let mut final_transcript_path = transcript_path;

    match rename_recording_outputs_from_transcript(
        &audio_path,
        &final_transcript_path,
        recording.created_at_ms,
    ) {
        Ok((renamed_audio_path, renamed_transcript_path)) => {
            audio_path = renamed_audio_path;
            final_transcript_path = renamed_transcript_path;
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

    recording.file_name = audio_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("recording.wav")
        .to_string();
    recording.file_path = audio_path.display().to_string();
    recording.transcript_path = Some(final_transcript_path.display().to_string());
    recording.transcript_language =
        derive_transcript_language_from_path(&final_transcript_path, requested_language);
    recording.bytes_written = fs::metadata(&audio_path)
        .map(|metadata| metadata.len())
        .unwrap_or(recording.bytes_written);

    let updated_recording = recording.clone();
    update_recent_recording(app, original_file_path, |recording| {
        *recording = updated_recording.clone();
    })?;

    Ok(recording)
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
    let recordings = selected_untranscribed_recordings(app, file_paths)?;
    let total = recordings.len();
    let mut items = Vec::new();

    for (index, recording) in recordings.into_iter().enumerate() {
        if recording.transcript_path.is_some() {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "skipped".into(),
                message: "Already transcribed.".into(),
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

pub(crate) fn convert_recordings_to_mp3_inner<R: Runtime>(
    app: &AppHandle<R>,
    file_paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let recordings = selected_recordings(app, file_paths)?;
    let mut items = Vec::new();

    for recording in recordings {
        let original_file_path = recording.file_path.clone();
        let audio_path = PathBuf::from(&recording.file_path);
        let is_wav = audio_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|extension| extension.eq_ignore_ascii_case("wav"))
            .unwrap_or(false);

        if recording.audio_deleted {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "skipped".into(),
                message: "Local audio was already deleted after Anki copied it.".into(),
                note_id: recording.anki_note_id,
            });
            continue;
        }

        if recording.transcript_path.is_none() {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "skipped".into(),
                message: "Transcribe this recording before converting it to MP3.".into(),
                note_id: recording.anki_note_id,
            });
            continue;
        }

        if !is_wav {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "skipped".into(),
                message: "This recording is already MP3 or is not a WAV file.".into(),
                note_id: recording.anki_note_id,
            });
            continue;
        }

        let converted_path = compress_transcribed_audio_if_possible(app, &audio_path);
        let converted_to_mp3 = converted_path
            .extension()
            .and_then(|value| value.to_str())
            .map(|extension| extension.eq_ignore_ascii_case("mp3"))
            .unwrap_or(false);

        if !converted_to_mp3 {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "failed".into(),
                message: "MP3 conversion did not complete. The WAV file was kept.".into(),
                note_id: recording.anki_note_id,
            });
            continue;
        }

        let mut updated_recording = recording.clone();
        updated_recording.file_name = converted_path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("recording.mp3")
            .to_string();
        updated_recording.file_path = converted_path.display().to_string();
        updated_recording.bytes_written = fs::metadata(&converted_path)
            .map(|metadata| metadata.len())
            .unwrap_or(updated_recording.bytes_written);

        update_recent_recording(app, &original_file_path, |recording| {
            *recording = updated_recording.clone();
        })?;

        items.push(RecordingActionItem {
            file_path: updated_recording.file_path,
            status: "success".into(),
            message: "Recording converted to MP3.".into(),
            note_id: updated_recording.anki_note_id,
        });
    }

    let success_count = items.iter().filter(|item| item.status == "success").count();
    let skipped_count = items.iter().filter(|item| item.status == "skipped").count();
    let failed_count = items.iter().filter(|item| item.status == "failed").count();

    update_shell_snapshot(app, |shell| {
        shell.status_text = format!(
            "MP3 conversion finished: {success_count} converted, {skipped_count} skipped, {failed_count} failed."
        );
        shell.transition_count += 1;
    })?;

    Ok(RecordingBatchResult {
        status: if failed_count == 0 {
            "completed"
        } else {
            "partial"
        }
        .into(),
        message: format!(
            "MP3 conversion finished: {success_count} converted, {skipped_count} skipped, {failed_count} failed."
        ),
        items,
        bootstrap: build_app_bootstrap(app)?,
    })
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
        let index = persisted
            .recent_recordings
            .iter()
            .position(|recording| recording.file_path == file_path)
            .ok_or_else(|| "The selected recording is no longer in the recent list.".to_string())?;
        let removed = persisted.recent_recordings.remove(index);
        let snapshot = persisted.clone();
        drop(persisted);
        write_persisted_data(app, &snapshot)?;
        removed
    };

    for path in [
        Some(removed_recording.file_path.as_str()),
        removed_recording.transcript_path.as_deref(),
        removed_recording.translation_path.as_deref(),
    ]
    .into_iter()
    .flatten()
    {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(format!("Could not delete {path}: {error}")),
        }
    }

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
    let path = PathBuf::from(&recording.file_path);
    if !path.exists() {
        return Err("The audio file is missing from disk.".into());
    }

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
) -> Result<RecordingBatchResult, String> {
    let recordings = selected_recordings(app, file_paths)?;
    let mut items = Vec::new();

    for recording in recordings {
        if recording.translation_path.is_some() {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "skipped".into(),
                message: "Already translated.".into(),
                note_id: recording.anki_note_id,
            });
        } else if recording.transcript_path.is_none() {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "failed".into(),
                message: "No transcript available to translate.".into(),
                note_id: recording.anki_note_id,
            });
        } else {
            items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "failed".into(),
                message: "Translation provider is not configured yet. This will be wired through the translation/extension bridge phase.".into(),
                note_id: recording.anki_note_id,
            });
        }
    }

    let skipped_count = items.iter().filter(|item| item.status == "skipped").count();
    let failed_count = items.iter().filter(|item| item.status == "failed").count();

    Ok(RecordingBatchResult {
        status: if failed_count == 0 {
            "completed"
        } else {
            "unavailable"
        }
        .into(),
        message: format!(
            "Translation request finished: {skipped_count} skipped, {failed_count} unavailable."
        ),
        items,
        bootstrap: build_app_bootstrap(app)?,
    })
}

fn unique_path_with_suffix(directory: &Path, file_stem: &str, suffix: &str) -> PathBuf {
    let sanitized_stem = if file_stem.is_empty() {
        "recording".to_string()
    } else {
        file_stem.to_string()
    };

    let mut attempt = 0usize;
    loop {
        let candidate = if attempt == 0 {
            directory.join(format!("{sanitized_stem}{suffix}"))
        } else {
            directory.join(format!("{sanitized_stem}_{attempt}{suffix}"))
        };

        if !candidate.exists() {
            return candidate;
        }

        attempt += 1;
    }
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

pub(crate) fn compress_transcribed_audio_if_possible<R: Runtime>(
    app: &AppHandle<R>,
    audio_path: &Path,
) -> PathBuf {
    if audio_path
        .extension()
        .and_then(|value| value.to_str())
        .map(|extension| !extension.eq_ignore_ascii_case("wav"))
        .unwrap_or(true)
    {
        return audio_path.to_path_buf();
    }

    let parent = audio_path.parent().unwrap_or_else(|| Path::new("."));
    let stem = audio_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("recording");
    let mp3_path = unique_path_with_suffix(parent, stem, ".mp3");

    let settings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = match persisted_state.0.lock() {
            Ok(persisted) => persisted,
            Err(_) => return audio_path.to_path_buf(),
        };
        persisted.settings.clone()
    };
    let ffmpeg_detection = detect_local_ffmpeg(&settings);
    let executable_path = ffmpeg_detection
        .executable_path
        .clone()
        .unwrap_or_else(|| "ffmpeg".into());

    let mut command = Command::new(&executable_path);
    hide_command_window(&mut command);
    if let Some(ffmpeg_directory) = Path::new(&executable_path).parent() {
        command.current_dir(ffmpeg_directory);
    }
    command
        .arg("-y")
        .arg("-nostdin")
        .arg("-hide_banner")
        .arg("-loglevel")
        .arg("error")
        .arg("-i")
        .arg(audio_path)
        .arg("-map")
        .arg("0:a:0")
        .arg("-vn")
        .arg("-codec:a")
        .arg("libmp3lame")
        .arg("-b:a")
        .arg("128k")
        .arg(&mp3_path);

    let output = match command.output() {
        Ok(output) => output,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            log_event(
                app,
                "INFO",
                "audio.compression_skipped",
                serde_json::json!({
                    "audioPath": audio_path,
                    "ffmpegStatus": ffmpeg_detection.status,
                    "message": "FFmpeg was not found. Keeping the WAV recording."
                }),
            );
            return audio_path.to_path_buf();
        }
        Err(error) => {
            log_event(
                app,
                "WARN",
                "audio.compression_failed",
                serde_json::json!({
                    "audioPath": audio_path,
                    "executablePath": executable_path,
                    "message": error.to_string()
                }),
            );
            return audio_path.to_path_buf();
        }
    };

    let mp3_ready = output.status.success()
        && fs::metadata(&mp3_path)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false);

    if !mp3_ready {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let details = [stderr, stdout]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        let _ = fs::remove_file(&mp3_path);
        log_event(
            app,
            "WARN",
            "audio.compression_failed",
            serde_json::json!({
                "audioPath": audio_path,
                "targetPath": mp3_path,
                "executablePath": executable_path,
                "statusCode": output.status.code(),
                "message": if details.is_empty() {
                    "ffmpeg did not produce a valid MP3 file.".to_string()
                } else {
                    details
                }
            }),
        );
        return audio_path.to_path_buf();
    }

    match fs::remove_file(audio_path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => {
            log_event(
                app,
                "WARN",
                "audio.source_delete_failed",
                serde_json::json!({
                    "audioPath": audio_path,
                    "targetPath": mp3_path,
                    "message": error.to_string()
                }),
            );
        }
    }

    log_event(
        app,
        "INFO",
        "audio.compressed",
        serde_json::json!({
            "sourcePath": audio_path,
            "targetPath": mp3_path
        }),
    );
    mp3_path
}
