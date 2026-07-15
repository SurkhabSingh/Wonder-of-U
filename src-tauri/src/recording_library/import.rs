use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_runtime::{build_app_bootstrap, ensure_directory_exists, log_event, now_ms, update_shell_snapshot},
    app_state::sanitize_recording_name,
    app_types::{
        AppSettings, RecentRecording, RecordingActionItem, RecordingBatchResult,
        SharedPersistedState,
    },
    runtime_assets::detect_local_ffmpeg,
};

use super::{insert_recent_recording, unique_path_with_suffix};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Formats whisper.cpp reads directly. The file is copied into the recordings
/// folder byte-for-byte: re-encoding here would only lose quality and time.
const PASSTHROUGH_EXTENSIONS: [&str; 4] = ["wav", "mp3", "flac", "ogg"];

/// Container/codec combinations whisper.cpp cannot open. These are transcoded to
/// MP3 (the same libmp3lame/128k profile the WAV compressor uses), so ffmpeg is
/// mandatory for them and only for them.
const CONVERT_EXTENSIONS: [&str; 10] = [
    "m4a", "opus", "mp4", "webm", "aac", "mkv", "mov", "m4v", "wma", "aiff",
];

const FFMPEG_REQUIRED_MESSAGE: &str =
    "FFmpeg is required to import this format; install it in Setup.";

/// What an imported file needs before it can land in the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportPlan {
    /// whisper.cpp reads it as-is: copy it verbatim.
    Passthrough,
    /// whisper.cpp cannot read it: transcode the first audio stream to MP3.
    ConvertToMp3,
    /// Not an audio/video container we can do anything useful with.
    Unsupported,
}

fn hide_command_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

/// Classifies a bare extension (no dot, any case) into an import plan.
fn classify_extension(extension: &str) -> ImportPlan {
    let key = extension
        .trim()
        .trim_start_matches('.')
        .to_ascii_lowercase();

    if PASSTHROUGH_EXTENSIONS.contains(&key.as_str()) {
        ImportPlan::Passthrough
    } else if CONVERT_EXTENSIONS.contains(&key.as_str()) {
        ImportPlan::ConvertToMp3
    } else {
        ImportPlan::Unsupported
    }
}

fn classify_path(path: &Path) -> ImportPlan {
    path.extension()
        .and_then(|value| value.to_str())
        .map(classify_extension)
        .unwrap_or(ImportPlan::Unsupported)
}

fn supported_extensions_sentence() -> String {
    let all = PASSTHROUGH_EXTENSIONS
        .iter()
        .chain(CONVERT_EXTENSIONS.iter())
        .copied()
        .collect::<Vec<_>>()
        .join(", ");
    format!("Import one of: {all}.")
}

/// Builds the ffmpeg argument list that transcodes any container's first audio
/// stream into MP3. Kept pure so the profile can be asserted without spawning
/// ffmpeg. `-vn` plus `-map 0:a:0` is what keeps an mp4/mkv from dragging its
/// video stream (or a cover-art "video" stream) into the output.
fn convert_ffmpeg_args(input: &str, output: &str) -> Vec<String> {
    vec![
        "-y".into(),
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-i".into(),
        input.to_string(),
        "-map".into(),
        "0:a:0".into(),
        "-vn".into(),
        "-codec:a".into(),
        "libmp3lame".into(),
        "-b:a".into(),
        "128k".into(),
        output.to_string(),
    ]
}

/// ffprobe ships beside ffmpeg in every distribution we detect (managed unpack or
/// PATH), so we look for it as a sibling of the resolved ffmpeg binary, keeping
/// the executable suffix (`ffmpeg.exe` -> `ffprobe.exe`).
fn ffprobe_path_for(ffmpeg_executable: &str) -> PathBuf {
    let ffmpeg_path = Path::new(ffmpeg_executable);
    let file_name = match ffmpeg_path.extension().and_then(|value| value.to_str()) {
        Some(extension) => format!("ffprobe.{extension}"),
        None => "ffprobe".to_string(),
    };

    match ffmpeg_path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => parent.join(file_name),
        _ => PathBuf::from(file_name),
    }
}

/// Parses ffprobe's `format=duration` output (fractional seconds) into whole
/// milliseconds. `N/A`, empty output, or a negative value yield `None`.
fn parse_ffprobe_duration_ms(stdout: &str) -> Option<u64> {
    let seconds = stdout.trim().lines().next()?.trim().parse::<f64>().ok()?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    Some((seconds * 1000.0).round() as u64)
}

/// Best-effort duration probe. A missing or failing ffprobe is never an import
/// failure — the recording simply lands with `duration_ms = 0`, exactly as an
/// unprobeable file recovered from disk does.
fn probe_duration_ms(ffmpeg_executable: Option<&str>, audio_path: &Path) -> u64 {
    if let Some(ffmpeg_executable) = ffmpeg_executable {
        let ffprobe = ffprobe_path_for(ffmpeg_executable);
        let mut command = Command::new(&ffprobe);
        hide_command_window(&mut command);
        command
            .arg("-v")
            .arg("error")
            .arg("-show_entries")
            .arg("format=duration")
            .arg("-of")
            .arg("default=noprint_wrappers=1:nokey=1")
            .arg(audio_path);

        if let Ok(output) = command.output() {
            if output.status.success() {
                if let Some(duration_ms) =
                    parse_ffprobe_duration_ms(&String::from_utf8_lossy(&output.stdout))
                {
                    return duration_ms;
                }
            }
        }
    }

    wav_duration_ms(audio_path).unwrap_or(0)
}

/// WAV fallback so a passthrough import still shows a duration on a machine with
/// no ffmpeg at all.
fn wav_duration_ms(path: &Path) -> Option<u64> {
    if !path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| extension.eq_ignore_ascii_case("wav"))
        .unwrap_or(false)
    {
        return None;
    }

    let reader = hound::WavReader::open(path).ok()?;
    let sample_rate = reader.spec().sample_rate;
    (sample_rate > 0).then(|| reader.duration() as u64 * 1000 / sample_rate as u64)
}

/// True when both paths resolve to the same file on disk. Guards the copy: a
/// source already inside the recordings folder must never be copied onto itself,
/// which would truncate it to zero bytes.
fn is_same_file(left: &Path, right: &Path) -> bool {
    match (left.canonicalize(), right.canonicalize()) {
        (Ok(left), Ok(right)) => left == right,
        _ => left.to_string_lossy().to_lowercase() == right.to_string_lossy().to_lowercase(),
    }
}

/// Transcodes `source` into `target`, treating a missing binary as the actionable
/// "install ffmpeg" error rather than a generic io failure.
fn transcode_to_mp3<R: Runtime>(
    app: &AppHandle<R>,
    settings: &AppSettings,
    source: &Path,
    target: &Path,
) -> Result<(), String> {
    let detection = detect_local_ffmpeg(settings);
    let executable_path = detection
        .executable_path
        .clone()
        .ok_or_else(|| FFMPEG_REQUIRED_MESSAGE.to_string())?;

    let mut command = Command::new(&executable_path);
    hide_command_window(&mut command);
    if let Some(ffmpeg_directory) = Path::new(&executable_path).parent() {
        command.current_dir(ffmpeg_directory);
    }
    command.args(convert_ffmpeg_args(
        &source.display().to_string(),
        &target.display().to_string(),
    ));

    let output = command.output().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            FFMPEG_REQUIRED_MESSAGE.to_string()
        } else {
            format!("FFmpeg could not convert this file: {error}")
        }
    })?;

    let mp3_ready = output.status.success()
        && fs::metadata(target)
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false);

    if !mp3_ready {
        let _ = fs::remove_file(target);
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        log_event(
            app,
            "WARN",
            "import.convert_failed",
            serde_json::json!({
                "sourcePath": source,
                "targetPath": target,
                "executablePath": executable_path,
                "statusCode": output.status.code(),
                "message": stderr
            }),
        );
        return Err(if stderr.is_empty() {
            "FFmpeg did not produce a playable MP3 for this file.".to_string()
        } else {
            format!("FFmpeg could not convert this file: {stderr}")
        });
    }

    Ok(())
}

/// Acquires one file into the recordings folder and returns the registered
/// recording. Every error here is scoped to this file: the caller turns it into a
/// failed item and moves on to the next path in the batch.
fn import_single_file<R: Runtime>(
    app: &AppHandle<R>,
    settings: &AppSettings,
    raw_path: &str,
) -> Result<RecentRecording, String> {
    let source = PathBuf::from(raw_path.trim());
    if source.as_os_str().is_empty() {
        return Err("That import path is empty.".into());
    }

    let metadata = fs::metadata(&source)
        .map_err(|_| "That file could not be read. It may have been moved or deleted.".to_string())?;
    if !metadata.is_file() {
        return Err("Only files can be imported, not folders.".into());
    }

    let original_file_name = source
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "That file name could not be read.".to_string())?
        .to_string();

    let plan = classify_path(&source);
    if plan == ImportPlan::Unsupported {
        return Err(format!(
            "{original_file_name} is not a supported audio or video file. {}",
            supported_extensions_sentence()
        ));
    }

    let output_directory = PathBuf::from(&settings.output_directory);
    ensure_directory_exists(&output_directory)
        .map_err(|error| format!("Could not open the recordings folder: {error}"))?;

    let sanitized_stem = sanitize_recording_name(
        source
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or_default(),
    );
    let file_stem = if sanitized_stem.is_empty() {
        "imported".to_string()
    } else {
        sanitized_stem
    };

    let suffix = match plan {
        ImportPlan::ConvertToMp3 => ".mp3".to_string(),
        // Keep the original container: whisper reads it, and re-encoding would
        // only cost quality.
        ImportPlan::Passthrough => format!(
            ".{}",
            source
                .extension()
                .and_then(|value| value.to_str())
                .unwrap_or("wav")
                .to_ascii_lowercase()
        ),
        ImportPlan::Unsupported => unreachable!("unsupported files are rejected above"),
    };
    // `unique_path_with_suffix` only ever returns a path that does not exist, so
    // the target can never be the (existing) source. The explicit guard below
    // keeps that invariant honest even if the helper changes.
    let target = unique_path_with_suffix(&output_directory, &file_stem, &suffix);
    if is_same_file(&source, &target) {
        return Err(format!(
            "{original_file_name} is already in the recordings folder."
        ));
    }

    match plan {
        ImportPlan::Passthrough => {
            fs::copy(&source, &target)
                .map_err(|error| format!("Could not copy this file into the recordings folder: {error}"))?;
        }
        ImportPlan::ConvertToMp3 => transcode_to_mp3(app, settings, &source, &target)?,
        ImportPlan::Unsupported => unreachable!("unsupported files are rejected above"),
    }

    let bytes_written = fs::metadata(&target)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if bytes_written == 0 {
        let _ = fs::remove_file(&target);
        return Err(format!("{original_file_name} produced an empty audio file."));
    }

    let ffmpeg_executable = detect_local_ffmpeg(settings).executable_path;
    let duration_ms = probe_duration_ms(ffmpeg_executable.as_deref(), &target);

    let recording = RecentRecording {
        file_name: target
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("imported")
            .to_string(),
        file_path: target.display().to_string(),
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
        duration_ms,
        bytes_written,
        created_at_ms: now_ms(),
        source: Some("import".into()),
        source_url: None,
        title: Some(original_file_name),
    };

    insert_recent_recording(app, recording.clone())?;

    log_event(
        app,
        "INFO",
        "import.completed",
        serde_json::json!({
            "sourcePath": source,
            "targetPath": recording.file_path,
            "converted": plan == ImportPlan::ConvertToMp3,
            "durationMs": recording.duration_ms,
            "bytesWritten": recording.bytes_written
        }),
    );

    Ok(recording)
}

/// Imports every path into the recordings folder as a transcript-less recording.
/// Import deliberately never transcribes: the file lands in the library as "Needs
/// transcript" and the user decides when to spend the compute.
pub(crate) fn import_media_inner<R: Runtime>(
    app: &AppHandle<R>,
    paths: Vec<String>,
) -> Result<RecordingBatchResult, String> {
    let settings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the app settings.".to_string())?;
        persisted.settings.clone()
    };

    let mut items = Vec::new();
    for raw_path in &paths {
        match import_single_file(app, &settings, raw_path) {
            Ok(recording) => items.push(RecordingActionItem {
                file_path: recording.file_path,
                status: "success".into(),
                message: format!("Imported {}.", recording.file_name),
                note_id: None,
            }),
            Err(message) => {
                log_event(
                    app,
                    "WARN",
                    "import.failed",
                    serde_json::json!({ "sourcePath": raw_path, "message": message }),
                );
                items.push(RecordingActionItem {
                    file_path: raw_path.clone(),
                    status: "failed".into(),
                    message,
                    note_id: None,
                });
            }
        }
    }

    let success_count = items.iter().filter(|item| item.status == "success").count();
    let failed_count = items.iter().filter(|item| item.status == "failed").count();
    let message = if items.is_empty() {
        "No files were selected to import.".to_string()
    } else {
        format!("Import finished: {success_count} imported, {failed_count} failed.")
    };

    let status_text = message.clone();
    update_shell_snapshot(app, |shell| {
        shell.status_text = status_text;
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

#[cfg(test)]
mod tests {
    use super::{
        classify_extension, convert_ffmpeg_args, ffprobe_path_for, is_same_file,
        parse_ffprobe_duration_ms, ImportPlan,
    };
    use std::path::{Path, PathBuf};

    #[test]
    fn whisper_readable_formats_are_copied_verbatim() {
        for extension in ["wav", "mp3", "flac", "ogg"] {
            assert_eq!(classify_extension(extension), ImportPlan::Passthrough);
        }
    }

    #[test]
    fn formats_whisper_cannot_read_are_converted() {
        for extension in [
            "m4a", "opus", "mp4", "webm", "aac", "mkv", "mov", "m4v", "wma", "aiff",
        ] {
            assert_eq!(classify_extension(extension), ImportPlan::ConvertToMp3);
        }
    }

    #[test]
    fn classification_ignores_case_and_a_leading_dot() {
        assert_eq!(classify_extension("WAV"), ImportPlan::Passthrough);
        assert_eq!(classify_extension(".Mp3"), ImportPlan::Passthrough);
        assert_eq!(classify_extension("M4A"), ImportPlan::ConvertToMp3);
    }

    #[test]
    fn unknown_extensions_are_rejected() {
        for extension in ["txt", "pdf", "", "wavy", "mp"] {
            assert_eq!(classify_extension(extension), ImportPlan::Unsupported);
        }
    }

    #[test]
    fn convert_args_take_only_the_first_audio_stream_at_the_shared_mp3_profile() {
        let args = convert_ffmpeg_args("C:\\in.mkv", "C:\\out.mp3");

        let input = args.iter().position(|arg| arg == "-i").expect("-i present");
        assert_eq!(args[input + 1], "C:\\in.mkv");
        assert_eq!(args.last().map(String::as_str), Some("C:\\out.mp3"));

        let map = args.iter().position(|arg| arg == "-map").expect("-map");
        assert_eq!(args[map + 1], "0:a:0");
        // Video must be dropped, or an mp4/mkv import carries its picture stream.
        assert!(args.iter().any(|arg| arg == "-vn"));
        assert!(args.iter().any(|arg| arg == "libmp3lame"));
        assert!(args.iter().any(|arg| arg == "128k"));
        // Never block on stdin inside a spawned command.
        assert!(args.iter().any(|arg| arg == "-nostdin"));
    }

    #[test]
    fn ffprobe_is_resolved_beside_ffmpeg_keeping_the_executable_suffix() {
        assert_eq!(
            ffprobe_path_for("C:\\assets\\ffmpeg-runtime\\latest\\bin\\ffmpeg.exe"),
            PathBuf::from("C:\\assets\\ffmpeg-runtime\\latest\\bin\\ffprobe.exe")
        );
        assert_eq!(
            ffprobe_path_for("/usr/local/bin/ffmpeg"),
            PathBuf::from("/usr/local/bin/ffprobe")
        );
        // A bare PATH lookup stays a bare PATH lookup.
        assert_eq!(ffprobe_path_for("ffmpeg"), PathBuf::from("ffprobe"));
    }

    #[test]
    fn ffprobe_duration_parses_seconds_into_milliseconds() {
        assert_eq!(parse_ffprobe_duration_ms("12.345\n"), Some(12345));
        assert_eq!(parse_ffprobe_duration_ms("0.5"), Some(500));
        assert_eq!(parse_ffprobe_duration_ms("N/A"), None);
        assert_eq!(parse_ffprobe_duration_ms(""), None);
        assert_eq!(parse_ffprobe_duration_ms("-1"), None);
    }

    #[test]
    fn a_file_is_recognized_as_itself_so_a_copy_can_never_truncate_it() {
        let temp_dir = tempfile::tempdir().unwrap();
        let file = temp_dir.path().join("clip.wav");
        std::fs::write(&file, b"audio").unwrap();

        let indirect = temp_dir.path().join(".").join("clip.wav");
        assert!(is_same_file(&file, &indirect));
        assert!(!is_same_file(
            &file,
            &temp_dir.path().join("clip_1.wav")
        ));
        assert!(!is_same_file(Path::new("a.wav"), Path::new("b.wav")));
    }
}
