use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_runtime::{build_app_bootstrap, log_event, update_shell_snapshot},
    app_types::{RecordingActionItem, RecordingBatchResult, SharedPersistedState},
    runtime_assets::detect_local_ffmpeg,
};

use super::{selected_recordings, unique_path_with_suffix, update_recent_recording};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

fn hide_command_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
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

fn compress_transcribed_audio_if_possible<R: Runtime>(
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
