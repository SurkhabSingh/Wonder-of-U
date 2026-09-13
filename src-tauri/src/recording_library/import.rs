use std::{
    collections::VecDeque,
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::Duration,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Emitter, EventId, Listener, Manager, Runtime};

use crate::{
    app_runtime::{build_app_bootstrap, ensure_directory_exists, log_event, now_ms, update_shell_snapshot},
    app_state::sanitize_recording_name,
    child_io::drain_lines,
    app_types::{
        AppSettings, RecentRecording, RecordingActionItem, RecordingBatchResult,
        SharedPersistedState,
    },
    media_errors::stderr_indicates_no_audio,
    runtime_assets::{detect_local_ffmpeg, detect_local_ytdlp},
};

use super::{insert_recent_recording, unique_path_with_suffix};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const PASSTHROUGH_EXTENSIONS: [&str; 4] = ["wav", "mp3", "flac", "ogg"];

const CONVERT_EXTENSIONS: [&str; 10] = [
    "m4a", "opus", "mp4", "webm", "aac", "mkv", "mov", "m4v", "wma", "aiff",
];

const FFMPEG_REQUIRED_MESSAGE: &str =
    "FFmpeg is required to import this format; install it in Setup.";

/// What an imported file needs before it can land in the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImportPlan {
    Passthrough,
    ConvertToMp3,
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
/// stream into MP3.
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

fn parse_ffprobe_duration_ms(stdout: &str) -> Option<u64> {
    let seconds = stdout.trim().lines().next()?.trim().parse::<f64>().ok()?;
    if !seconds.is_finite() || seconds <= 0.0 {
        return None;
    }
    Some((seconds * 1000.0).round() as u64)
}

pub(crate) fn probe_duration_ms(ffmpeg_executable: Option<&str>, audio_path: &Path) -> u64 {
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
        return Err(if stderr_indicates_no_audio(&stderr) {
            NO_AUDIO_REJECTED_MESSAGE.to_string()
        } else if stderr.is_empty() {
            "FFmpeg did not produce a playable MP3 for this file.".to_string()
        } else {
            format!("FFmpeg could not convert this file: {stderr}")
        });
    }

    Ok(())
}

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

const YTDLP_REQUIRED_MESSAGE: &str =
    "yt-dlp is required to import from a link; install it in Setup.";
const YOUTUBE_FFMPEG_REQUIRED_MESSAGE: &str =
    "FFmpeg is required to import from a link; install it in Setup.";
const LIVESTREAM_REJECTED_MESSAGE: &str =
    "This is a live or upcoming stream, so it can't be imported.";
const OVERSIZE_REJECTED_MESSAGE: &str =
    "This video is too large to import; the limit is 2 GB.";
const NO_AUDIO_REJECTED_MESSAGE: &str =
    "This video has no sound, so there is nothing to import.";

fn stderr_indicates_livestream(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    [
        "has not passed filter",
        "does not pass filter",
        "is not live",
        "premieres in",
        "live event will begin",
    ]
    .iter()
    .any(|needle| lower.contains(needle))
}

fn stderr_indicates_oversize(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    ["larger than max-filesize", "larger than --max-filesize"]
        .iter()
        .any(|needle| lower.contains(needle))
}

/// The lines of yt-dlp's stderr that say why the run FAILED.
fn fatal_stderr_lines(stderr: &str) -> impl Iterator<Item = &str> {
    stderr
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("ERROR:"))
}

fn stderr_indicates_js_runtime(stderr: &str) -> bool {
    fatal_stderr_lines(stderr).any(|line| {
        let lower = line.to_ascii_lowercase();
        [
            "javascript runtime",
            "javascript interpreter",
            "js runtime",
            "failed to extract nsig",
            "nsig extraction failed",
            "unable to run the javascript",
        ]
        .iter()
        .any(|needle| lower.contains(needle))
    })
}

const JS_RUNTIME_REJECTED_MESSAGE: &str =
    "YouTube couldn't be read this time — it asked for a JavaScript runtime to unlock this video. This is usually temporary, so try the import again. Installing Deno or Node.js makes it reliable.";

struct VideoMetadata {
    live_status: String,
    title: String,
    id: String,
}

fn probe_video_metadata(ytdlp_executable: &str, url: &str) -> Result<VideoMetadata, String> {
    let mut command = Command::new(ytdlp_executable);
    hide_command_window(&mut command);
    command.env("PYTHONUNBUFFERED", "1");
    command.args([
        "--ignore-config",
        "--no-config-locations",
        "--no-playlist",
        "--print",
        "%(live_status)s\t%(title)s\t%(id)s",
        "--",
        url,
    ]);
    let output = command
        .output()
        .map_err(|error| format!("Could not start yt-dlp: {error}"))?;
    if !output.status.success() {
        return Err(String::from_utf8_lossy(&output.stderr).trim().to_string());
    }
    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(parse_probe_metadata_line(stdout.lines().next().unwrap_or("")))
}

fn parse_probe_metadata_line(line: &str) -> VideoMetadata {
    let (live_status, rest) = line.split_once('\t').unwrap_or((line, ""));
    let (title, id) = rest.rsplit_once('\t').unwrap_or((rest, ""));
    VideoMetadata {
        live_status: live_status.trim().to_string(),
        title: title.trim().to_string(),
        id: id.trim().to_string(),
    }
}

fn youtube_output_stem(title: &str, id: &str) -> String {
    let sanitized_title = sanitize_recording_name(title);
    let sanitized_id = sanitize_recording_name(id);
    match (sanitized_title.is_empty(), sanitized_id.is_empty()) {
        (false, false) => format!("{sanitized_title} [{sanitized_id}]"),
        (false, true) => sanitized_title,
        (true, false) => sanitized_id,
        (true, true) => "youtube".to_string(),
    }
}

/// How many entries a run announced (`[download] Downloading item 2 of 3` -> 3).
fn parse_ytdlp_item_total(line: &str) -> Option<usize> {
    let rest = line.strip_prefix("[download] Downloading item ")?;
    let (_, total) = rest.split_once(" of ")?;
    total.trim().parse().ok()
}

/// Builds yt-dlp's `-o` value for a caller-precomputed literal path.
fn ytdlp_output_template(output_directory: &Path, unique_stem: &str) -> String {
    let literal_path = output_directory.join(unique_stem).display().to_string();
    format!(
        "{}%(playlist_index& {{}}|)s.%(ext)s",
        literal_path.replace('%', "%%")
    )
}

fn live_status_is_stream(live_status: &str) -> bool {
    matches!(live_status, "is_live" | "is_upcoming" | "post_live")
}

/// Owns the import's `youtube-cancel` listener for the whole command.
struct CancelListener<R: Runtime> {
    app: AppHandle<R>,
    event_id: EventId,
    flag: Arc<AtomicBool>,
}

impl<R: Runtime> CancelListener<R> {
    fn register(app: &AppHandle<R>) -> Self {
        let flag = Arc::new(AtomicBool::new(false));
        let flag_for_listener = Arc::clone(&flag);
        let event_id = app.once("youtube-cancel", move |_| {
            flag_for_listener.store(true, Ordering::Relaxed);
        });

        Self {
            app: app.clone(),
            event_id,
            flag,
        }
    }

    fn is_cancelled(&self) -> bool {
        self.flag.load(Ordering::Relaxed)
    }

    fn flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.flag)
    }
}

impl<R: Runtime> Drop for CancelListener<R> {
    fn drop(&mut self) {
        self.app.unlisten(self.event_id);
    }
}

/// Why a fetch failed, decided from yt-dlp's own output while that output is still
/// intact.
enum FetchFailure {
    JsRuntime,
    Livestream,
    Oversize,
    NoAudio,
    Other(String),
}

impl FetchFailure {
    fn message(self) -> String {
        match self {
            FetchFailure::JsRuntime => JS_RUNTIME_REJECTED_MESSAGE.to_string(),
            FetchFailure::Livestream => LIVESTREAM_REJECTED_MESSAGE.to_string(),
            FetchFailure::Oversize => OVERSIZE_REJECTED_MESSAGE.to_string(),
            FetchFailure::NoAudio => NO_AUDIO_REJECTED_MESSAGE.to_string(),
            FetchFailure::Other(message) => message,
        }
    }
}

/// The part of a failed run's output worth showing when nothing else explains it.
fn failure_detail(stderr: &str) -> String {
    let fatal = fatal_stderr_lines(stderr).collect::<Vec<_>>().join("\n");
    let source = if fatal.is_empty() { stderr } else { fatal.as_str() };
    let characters: Vec<char> = source.chars().collect();
    if characters.len() <= 600 {
        return source.to_string();
    }
    characters[characters.len() - 600..].iter().collect()
}

fn without_reflected_inputs(stderr: &str, url: &str, stem: &str) -> String {
    let stripped = stderr.replace(url, " ");
    if stem.is_empty() {
        return stripped;
    }
    stripped.replace(stem, " ")
}

enum FetchOutcome {
    Completed {
        paths: Vec<PathBuf>,
        missing: Vec<String>,
    },
    Cancelled,
}

fn entry_failure_message(classifiable: &str, stderr: &str) -> String {
    if stderr_indicates_no_audio(classifiable) {
        NO_AUDIO_REJECTED_MESSAGE.to_string()
    } else if stderr_indicates_livestream(classifiable) {
        LIVESTREAM_REJECTED_MESSAGE.to_string()
    } else if stderr_indicates_oversize(classifiable) {
        OVERSIZE_REJECTED_MESSAGE.to_string()
    } else {
        let detail = failure_detail(stderr);
        if detail.is_empty() {
            "A video in this link could not be fetched.".to_string()
        } else {
            format!("A video in this link could not be fetched: {detail}")
        }
    }
}

/// True when `name` is a file the import owning `stem` produced, or part-produced.
fn is_own_import_artifact(name: &str, stem: &str) -> bool {
    let Some(remainder) = name.strip_prefix(stem) else {
        return false;
    };
    strip_playlist_index(remainder).starts_with('.')
}

/// Strips a leading ` <digits>` playlist index, returning the remainder untouched
/// when there is none — which is what a single-video import always looks like.
fn strip_playlist_index(remainder: &str) -> &str {
    let Some(rest) = remainder.strip_prefix(' ') else {
        return remainder;
    };
    let digits = rest.len() - rest.trim_start_matches(|c: char| c.is_ascii_digit()).len();
    if digits == 0 {
        return remainder;
    }
    &rest[digits..]
}

fn artifact_playlist_index(name: &str, stem: &str) -> u32 {
    name.strip_prefix(stem)
        .and_then(|remainder| remainder.strip_prefix(' '))
        .map(|rest| {
            rest.chars()
                .take_while(|value| value.is_ascii_digit())
                .collect::<String>()
        })
        .and_then(|digits| digits.parse().ok())
        .unwrap_or(0)
}

/// Picks a stem no file already in the recordings folder could belong to.
fn unique_import_stem(directory: &Path, base_stem: &str) -> Result<String, String> {
    let base = if base_stem.is_empty() {
        "youtube"
    } else {
        base_stem
    };
    let existing: Vec<String> = fs::read_dir(directory)
        .map_err(|error| format!("Could not read the recordings folder: {error}"))?
        .flatten()
        .filter(|entry| entry.path().is_file())
        .filter_map(|entry| entry.file_name().to_str().map(str::to_lowercase))
        .collect();

    let mut attempt = 0usize;
    loop {
        let candidate = if attempt == 0 {
            base.to_string()
        } else {
            format!("{base}_{attempt}")
        };
        let folded = candidate.to_lowercase();
        if !existing
            .iter()
            .any(|name| is_own_import_artifact(name, &folded))
        {
            return Ok(candidate);
        }
        attempt += 1;
    }
}

/// True when this is the file a fetch set out to produce.
fn is_produced_audio(path: &Path) -> bool {
    path.extension()
        .and_then(|value| value.to_str())
        .map(|value| value.eq_ignore_ascii_case("mp3"))
        .unwrap_or(false)
}

/// Resolves the audio files a fetch produced, in playlist order.
fn resolve_downloaded_audio_files(output_directory: &Path, expected_output: &Path) -> Vec<PathBuf> {
    let Some(stem) = expected_output.file_stem().and_then(|value| value.to_str()) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(output_directory) else {
        return Vec::new();
    };

    let mut produced: Vec<(u32, PathBuf)> = entries
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_file() && is_produced_audio(path))
        .filter_map(|path| {
            let index = path
                .file_name()
                .and_then(|value| value.to_str())
                .filter(|name| is_own_import_artifact(name, stem))
                .map(|name| artifact_playlist_index(name, stem))?;
            Some((index, path))
        })
        .collect();

    produced.sort_by(|left, right| left.0.cmp(&right.0).then_with(|| left.1.cmp(&right.1)));
    produced.into_iter().map(|(_, path)| path).collect()
}

fn validate_import_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Enter a link to import.".into());
    }

    let lower = trimmed.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"))
        .ok_or_else(|| "The import URL must start with http:// or https://.".to_string())?;

    let host_end = rest
        .find(|character| character == '/' || character == '?' || character == '#')
        .unwrap_or(rest.len());
    if rest[..host_end].is_empty() {
        return Err("That import URL is missing a host.".into());
    }

    Ok(trimmed.to_string())
}

fn parse_ytdlp_progress_line(line: &str) -> Option<f64> {
    let rest = line.trim().strip_prefix("YTDLP_PCT")?;
    let value = rest
        .trim()
        .trim_end_matches('%')
        .trim()
        .parse::<f64>()
        .ok()?;
    (value.is_finite() && (0.0..=100.0).contains(&value)).then_some(value)
}

fn ytdlp_fetch_args(
    output_template: &str,
    ffmpeg_location: Option<&str>,
    url: &str,
) -> Vec<String> {
    let mut args: Vec<String> = vec![
        "--ignore-config".into(),
        "--no-config-locations".into(),
        "--no-playlist".into(),
        "--match-filter".into(),
        "!is_live".into(),
        "--max-filesize".into(),
        "2G".into(),
        "-x".into(),
        "--audio-format".into(),
        "mp3".into(),
        "--audio-quality".into(),
        "128K".into(),
    ];
    if let Some(location) = ffmpeg_location {
        args.push("--ffmpeg-location".into());
        args.push(location.to_string());
    }
    args.extend([
        "--progress-template".into(),
        "YTDLP_PCT %(progress._percent_str)s\n".into(),
        "-o".into(),
        output_template.to_string(),
        "--".into(),
        url.to_string(),
    ]);
    args
}

fn path_is_within(root: &Path, candidate: &Path) -> bool {
    match (root.canonicalize(), candidate.canonicalize()) {
        (Ok(root), Ok(candidate)) => candidate.starts_with(&root),
        _ => false,
    }
}

fn fetch_youtube_audio<R: Runtime>(
    app: &AppHandle<R>,
    cancel: Arc<AtomicBool>,
    ytdlp_executable: &str,
    ffmpeg_location: Option<&str>,
    output_directory: &Path,
    output_template: &str,
    expected_output: &Path,
    url: &str,
) -> Result<FetchOutcome, FetchFailure> {
    let args = ytdlp_fetch_args(output_template, ffmpeg_location, url);

    let mut command = Command::new(ytdlp_executable);
    hide_command_window(&mut command);
    command.env("PYTHONUNBUFFERED", "1");
    command
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut child = command
        .spawn()
        .map_err(|error| FetchFailure::Other(format!("Could not start yt-dlp: {error}")))?;
    let stdout = child.stdout.take().ok_or_else(|| {
        FetchFailure::Other("yt-dlp produced no stdout stream.".to_string())
    })?;
    let stderr = child.stderr.take().ok_or_else(|| {
        FetchFailure::Other("yt-dlp produced no stderr stream.".to_string())
    })?;

    let stderr_buffer: Arc<Mutex<StderrTail>> = Arc::new(Mutex::new(StderrTail::default()));
    let stderr_sink = Arc::clone(&stderr_buffer);
    let stderr_thread = thread::spawn(move || {
        for line in drain_lines(stderr) {
            if let Ok(mut sink) = stderr_sink.lock() {
                sink.push(line);
            }
        }
    });

    let (done_sender, done_receiver) = mpsc::channel::<()>();
    let app_for_stdout = app.clone();
    let announced_entries = Arc::new(AtomicUsize::new(0));
    let announced_sink = Arc::clone(&announced_entries);
    let stdout_thread = thread::spawn(move || {
        let _done_sender = done_sender;
        for line in drain_lines(stdout) {
            let line = line.replace('\r', "");
            let line = line.trim();
            if let Some(percent) = parse_ytdlp_progress_line(line) {
                let _ = app_for_stdout.emit("youtube-progress", percent);
            }
            if let Some(total) = parse_ytdlp_item_total(line) {
                announced_sink.store(total, Ordering::Relaxed);
            }
        }
    });

    loop {
        match done_receiver.recv_timeout(CANCEL_POLL_INTERVAL) {
            Ok(()) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if cancel.load(Ordering::Relaxed) {
                    kill_process_tree(child.id());
                    let _ = child.kill();
                    break;
                }
            }
        }
    }

    let exit_status = child
        .wait()
        .map_err(|error| FetchFailure::Other(format!("yt-dlp did not exit cleanly: {error}")))?;
    let _ = stdout_thread.join();
    let _ = stderr_thread.join();

    let stderr_text = stderr_buffer
        .lock()
        .ok()
        .map(|guard| guard.text().trim().to_string())
        .unwrap_or_default();

    let unique_stem = expected_output
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or_default();

    if cancel.load(Ordering::Relaxed) {
        sweep_import_artifacts(output_directory, unique_stem, &[]);
        return Ok(FetchOutcome::Cancelled);
    }

    let classifiable = without_reflected_inputs(&stderr_text, url, unique_stem);

    let produced: Vec<PathBuf> = resolve_downloaded_audio_files(output_directory, expected_output)
        .into_iter()
        .filter(|path| path_is_within(output_directory, path))
        .collect();

    if !produced.is_empty() {
        sweep_import_artifacts(output_directory, unique_stem, &produced);

        let shortfall = announced_entries
            .load(Ordering::Relaxed)
            .saturating_sub(produced.len());
        let missing = (0..shortfall)
            .map(|_| entry_failure_message(&classifiable, &stderr_text))
            .collect();
        return Ok(FetchOutcome::Completed {
            paths: produced,
            missing,
        });
    }

    sweep_import_artifacts(output_directory, unique_stem, &[]);

    if !exit_status.success() {
        log_event(
            app,
            "WARN",
            "youtube.ytdlp_failed",
            serde_json::json!({
                "sourceUrl": url,
                "exitCode": exit_status.code(),
                "stderr": stderr_text.clone()
            }),
        );

        if stderr_indicates_livestream(&classifiable) {
            return Err(FetchFailure::Livestream);
        }
        if stderr_indicates_no_audio(&classifiable) {
            return Err(FetchFailure::NoAudio);
        }
        if stderr_indicates_js_runtime(&classifiable) {
            return Err(FetchFailure::JsRuntime);
        }
        let detail = failure_detail(&stderr_text);
        return Err(FetchFailure::Other(if detail.is_empty() {
            "yt-dlp could not download this video.".to_string()
        } else {
            format!("yt-dlp could not download this video: {detail}")
        }));
    }

    if stderr_indicates_livestream(&classifiable) {
        Err(FetchFailure::Livestream)
    } else if stderr_indicates_oversize(&classifiable) {
        Err(FetchFailure::Oversize)
    } else if stderr_indicates_no_audio(&classifiable) {
        Err(FetchFailure::NoAudio)
    } else {
        Err(FetchFailure::Other(
            "yt-dlp finished but did not produce an audio file.".into(),
        ))
    }
}

/// yt-dlp's stderr, bounded so a chatty extractor can never fill the pipe and block
/// the child, and bounded at the FRONT.
#[derive(Default)]
struct StderrTail {
    lines: VecDeque<String>,
    bytes: usize,
}

impl StderrTail {
    const MAX_BYTES: usize = 8192;

    fn push(&mut self, line: String) {
        self.bytes += line.len() + 1;
        self.lines.push_back(line);
        while self.bytes > Self::MAX_BYTES && self.lines.len() > 1 {
            if let Some(dropped) = self.lines.pop_front() {
                self.bytes -= dropped.len() + 1;
            }
        }
    }

    fn text(&self) -> String {
        self.lines
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// How often the fetch wakes to re-read the cancel flag while the stdout pipe is
/// silent. This bounds how long Cancel can appear to do nothing.
const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[cfg(target_os = "windows")]
fn kill_process_tree(pid: u32) {
    let mut command = Command::new("taskkill");
    hide_command_window(&mut command);
    let _ = command
        .args(["/F", "/T", "/PID", &pid.to_string()])
        .output();
}

#[cfg(not(target_os = "windows"))]
fn kill_process_tree(_pid: u32) {}

fn sweep_import_artifacts(directory: &Path, stem: &str, keep: &[PathBuf]) {
    if stem.is_empty() {
        return;
    }
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || keep.contains(&path) {
            continue;
        }
        let is_own = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| is_own_import_artifact(name, stem))
            .unwrap_or(false);
        if is_own {
            let _ = fs::remove_file(&path);
        }
    }
}

fn register_youtube_recording<R: Runtime>(
    app: &AppHandle<R>,
    ffmpeg_executable: Option<&str>,
    final_path: &Path,
    title: Option<String>,
    source_url: &str,
) -> Result<RecentRecording, String> {
    let file_name = final_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("youtube")
        .to_string();
    let file_stem = final_path
        .file_stem()
        .and_then(|stem| stem.to_str())
        .unwrap_or("youtube")
        .to_string();

    let bytes_written = fs::metadata(final_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if bytes_written == 0 {
        let _ = fs::remove_file(final_path);
        return Err("yt-dlp produced an empty audio file.".into());
    }

    let duration_ms = probe_duration_ms(ffmpeg_executable, final_path);

    let recording = RecentRecording {
        file_name,
        file_path: final_path.display().to_string(),
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
        source: Some("youtube".into()),
        source_url: Some(source_url.to_string()),
        title: title.or(Some(file_stem)),
    };

    insert_recent_recording(app, recording.clone())?;
    Ok(recording)
}

fn finish_youtube_failure<R: Runtime>(
    app: &AppHandle<R>,
    source_url: &str,
    message: String,
) -> Result<RecordingBatchResult, String> {
    let status_text = message.clone();
    update_shell_snapshot(app, |shell| {
        shell.status_text = status_text;
        shell.transition_count += 1;
    })?;
    log_event(
        app,
        "WARN",
        "youtube.import_failed",
        serde_json::json!({ "sourceUrl": source_url, "message": message }),
    );

    Ok(RecordingBatchResult {
        status: "failed".into(),
        message: message.clone(),
        items: vec![RecordingActionItem {
            file_path: source_url.to_string(),
            status: "failed".into(),
            message,
            note_id: None,
        }],
        bootstrap: build_app_bootstrap(app)?,
    })
}

fn finish_youtube_cancelled<R: Runtime>(
    app: &AppHandle<R>,
    source_url: &str,
) -> Result<RecordingBatchResult, String> {
    update_shell_snapshot(app, |shell| {
        shell.status_text = "Import cancelled.".into();
        shell.transition_count += 1;
    })?;

    Ok(RecordingBatchResult {
        status: "cancelled".into(),
        message: "Import cancelled.".into(),
        items: vec![RecordingActionItem {
            file_path: source_url.to_string(),
            status: "failed".into(),
            message: "Import cancelled.".into(),
            note_id: None,
        }],
        bootstrap: build_app_bootstrap(app)?,
    })
}

/// Imports a single video's audio from a URL into the library via yt-dlp. Like
/// local import, it never transcribes: the MP3 lands as "Needs transcript" and
/// the user decides when to spend the compute.
pub(crate) fn import_youtube_inner<R: Runtime>(
    app: &AppHandle<R>,
    url: String,
) -> Result<RecordingBatchResult, String> {
    let normalized_url = validate_import_url(&url)?;

    let settings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the app settings.".to_string())?;
        persisted.settings.clone()
    };

    let ytdlp_detection = detect_local_ytdlp(&settings);
    let ytdlp_executable = ytdlp_detection
        .executable_path
        .filter(|_| ytdlp_detection.status == "ready")
        .ok_or_else(|| YTDLP_REQUIRED_MESSAGE.to_string())?;

    let ffmpeg_executable = detect_local_ffmpeg(&settings)
        .executable_path
        .ok_or_else(|| YOUTUBE_FFMPEG_REQUIRED_MESSAGE.to_string())?;
    let ffmpeg_location = Path::new(&ffmpeg_executable)
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(|parent| parent.display().to_string());

    let output_directory = PathBuf::from(&settings.output_directory);
    ensure_directory_exists(&output_directory)
        .map_err(|error| format!("Could not open the recordings folder: {error}"))?;

    let cancel_listener = CancelListener::register(app);

    let probe = probe_video_metadata(&ytdlp_executable, &normalized_url);

    if cancel_listener.is_cancelled() {
        return finish_youtube_cancelled(app, &normalized_url);
    }

    let metadata = match probe {
        Ok(metadata) => {
            if live_status_is_stream(&metadata.live_status) {
                return Err(LIVESTREAM_REJECTED_MESSAGE.to_string());
            }
            Some(metadata)
        }
        Err(stderr) => {
            if stderr_indicates_livestream(&without_reflected_inputs(&stderr, &normalized_url, "")) {
                return Err(LIVESTREAM_REJECTED_MESSAGE.to_string());
            }
            None
        }
    };

    let (video_title, output_stem) = match &metadata {
        Some(metadata) => {
            let title = (!metadata.title.trim().is_empty())
                .then(|| metadata.title.trim().to_string());
            (title, youtube_output_stem(&metadata.title, &metadata.id))
        }
        None => (None, "youtube".to_string()),
    };

    let unique_stem = unique_import_stem(&output_directory, &output_stem)?;
    let expected_output = output_directory.join(format!("{unique_stem}.mp3"));
    let output_template = ytdlp_output_template(&output_directory, &unique_stem);

    update_shell_snapshot(app, |shell| {
        shell.status_text = "Importing audio from the link…".into();
        shell.transition_count += 1;
    })?;

    const MAX_JS_RUNTIME_ATTEMPTS: usize = 3;
    let mut attempt = 0;
    let fetch_result = loop {
        attempt += 1;
        let outcome = fetch_youtube_audio(
            app,
            cancel_listener.flag(),
            &ytdlp_executable,
            ffmpeg_location.as_deref(),
            &output_directory,
            &output_template,
            &expected_output,
            &normalized_url,
        );
        let should_retry = attempt < MAX_JS_RUNTIME_ATTEMPTS
            && !cancel_listener.flag().load(Ordering::Relaxed)
            && matches!(&outcome, Err(FetchFailure::JsRuntime));
        if !should_retry {
            break outcome;
        }
        log_event(
            app,
            "WARN",
            "youtube.js_runtime_retry",
            serde_json::json!({ "attempt": attempt, "sourceUrl": normalized_url }),
        );
        update_shell_snapshot(app, |shell| {
            shell.status_text = "YouTube needs a moment; retrying the import…".into();
            shell.transition_count += 1;
        })?;
    };

    match fetch_result {
        Ok(FetchOutcome::Completed { paths, missing }) => {
            let mut items = Vec::new();
            let mut first_failure: Option<String> = None;

            for path in &paths {
                match register_youtube_recording(
                    app,
                    Some(ffmpeg_executable.as_str()),
                    path,
                    video_title.clone(),
                    &normalized_url,
                ) {
                    Ok(recording) => {
                        log_event(
                            app,
                            "INFO",
                            "youtube.imported",
                            serde_json::json!({
                                "sourceUrl": normalized_url,
                                "targetPath": recording.file_path.clone(),
                                "durationMs": recording.duration_ms,
                                "bytesWritten": recording.bytes_written
                            }),
                        );
                        items.push(RecordingActionItem {
                            file_path: recording.file_path,
                            status: "success".into(),
                            message: format!("Imported {}.", recording.file_name),
                            note_id: None,
                        });
                    }
                    Err(message) => {
                        log_event(
                            app,
                            "WARN",
                            "youtube.register_failed",
                            serde_json::json!({
                                "sourceUrl": normalized_url,
                                "targetPath": path.display().to_string(),
                                "message": message.clone()
                            }),
                        );
                        if first_failure.is_none() {
                            first_failure = Some(message.clone());
                        }
                        items.push(RecordingActionItem {
                            file_path: path.display().to_string(),
                            status: "failed".into(),
                            message,
                            note_id: None,
                        });
                    }
                }
            }

            for message in missing {
                log_event(
                    app,
                    "WARN",
                    "youtube.entry_missing",
                    serde_json::json!({ "sourceUrl": normalized_url, "message": message.clone() }),
                );
                items.push(RecordingActionItem {
                    file_path: normalized_url.clone(),
                    status: "failed".into(),
                    message,
                    note_id: None,
                });
            }

            let succeeded = items.iter().filter(|item| item.status == "success").count();
            let failed = items.len() - succeeded;

            if succeeded == 0 {
                let message = first_failure
                    .unwrap_or_else(|| "yt-dlp finished but did not produce an audio file.".into());
                return finish_youtube_failure(app, &normalized_url, message);
            }

            let message = match (succeeded, failed) {
                (1, 0) => items
                    .iter()
                    .find(|item| item.status == "success")
                    .map(|item| item.message.clone())
                    .unwrap_or_else(|| "Imported 1 recording from this link.".into()),
                (_, 0) => format!("Imported {succeeded} recordings from this link."),
                _ => format!("Import finished: {succeeded} imported, {failed} failed."),
            };

            let status_text = message.clone();
            update_shell_snapshot(app, |shell| {
                shell.status_text = status_text;
                shell.transition_count += 1;
            })?;

            Ok(RecordingBatchResult {
                status: if failed == 0 { "completed" } else { "partial" }.into(),
                message,
                items,
                bootstrap: build_app_bootstrap(app)?,
            })
        }
        Ok(FetchOutcome::Cancelled) => finish_youtube_cancelled(app, &normalized_url),
        Err(failure) => finish_youtube_failure(app, &normalized_url, failure.message()),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        stderr_indicates_no_audio,
        classify_extension, convert_ffmpeg_args, ffprobe_path_for, is_same_file,
        parse_ffprobe_duration_ms, parse_probe_metadata_line, parse_ytdlp_progress_line,
        is_own_import_artifact, stderr_indicates_js_runtime, stderr_indicates_livestream,
        stderr_indicates_oversize, validate_import_url,
        ytdlp_fetch_args, ytdlp_output_template, youtube_output_stem, ImportPlan, StderrTail,
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

    #[test]
    fn valid_http_and_https_urls_are_accepted_with_case_preserved() {
        assert_eq!(
            validate_import_url("  https://www.youtube.com/watch?v=abc123  ").unwrap(),
            "https://www.youtube.com/watch?v=abc123"
        );
        assert_eq!(
            validate_import_url("HTTPS://YouTu.be/AbC").unwrap(),
            "HTTPS://YouTu.be/AbC"
        );
        assert_eq!(
            validate_import_url("http://example.com").unwrap(),
            "http://example.com"
        );
    }

    #[test]
    fn urls_without_scheme_or_host_are_rejected() {
        assert!(validate_import_url("").is_err());
        assert!(validate_import_url("   ").is_err());
        assert!(validate_import_url("www.youtube.com/watch?v=abc").is_err());
        assert!(validate_import_url("ftp://example.com/file").is_err());
        // Scheme present but no host.
        assert!(validate_import_url("https://").is_err());
        assert!(validate_import_url("http:///path").is_err());
    }

    #[test]
    fn progress_lines_parse_a_bounded_percent() {
        // The real emitted format: the `YTDLP_PCT` marker then `_percent_str`'s
        // padded value (the `download:` type selector is consumed by yt-dlp).
        assert_eq!(parse_ytdlp_progress_line("YTDLP_PCT   42.3%"), Some(42.3));
        assert_eq!(parse_ytdlp_progress_line("YTDLP_PCT 0.0%"), Some(0.0));
        assert_eq!(parse_ytdlp_progress_line("YTDLP_PCT 100.0%"), Some(100.0));
        // Trailing whitespace / carriage returns are tolerated.
        assert_eq!(parse_ytdlp_progress_line("YTDLP_PCT 7.5% \r"), Some(7.5));
    }

    #[test]
    fn non_progress_or_unparseable_lines_yield_none() {
        // A bare percent with no marker (what the old buggy `download:` template
        // actually emitted) must NOT parse — that mismatch was the live-progress bug.
        assert_eq!(parse_ytdlp_progress_line("  42.3%"), None);
        assert_eq!(parse_ytdlp_progress_line("download:  42.3%"), None);
        assert_eq!(parse_ytdlp_progress_line("YTDLP_PCT N/A"), None);
        assert_eq!(parse_ytdlp_progress_line("[ExtractAudio] Destination: x"), None);
        assert_eq!(parse_ytdlp_progress_line(""), None);
        // Out-of-range values are ignored rather than clamped.
        assert_eq!(parse_ytdlp_progress_line("YTDLP_PCT 250.0%"), None);
        assert_eq!(parse_ytdlp_progress_line("YTDLP_PCT -5.0%"), None);
    }

    #[test]
    fn livestream_stderr_is_detected_across_wordings_and_case() {
        // Current yt-dlp phrasing for a `--match-filter "!is_live"` rejection.
        assert!(stderr_indicates_livestream(
            "ERROR: [youtube] abc: Video has not passed filter (!is_live), skipping .."
        ));
        // Older phrasing kept for back-compat.
        assert!(stderr_indicates_livestream(
            "ERROR: abc: Video does not pass filter (!is_live)"
        ));
        // Upcoming / premiere wordings (these fail on the non-zero-exit path).
        assert!(stderr_indicates_livestream(
            "ERROR: This live event will begin in 3 hours"
        ));
        assert!(stderr_indicates_livestream("ERROR: Premieres in 2 days"));
        assert!(stderr_indicates_livestream(
            "ERROR: The channel is not live"
        ));
        // Matching is case-insensitive.
        assert!(stderr_indicates_livestream("HAS NOT PASSED FILTER"));
        // A plain unavailable/geo error is not a livestream.
        assert!(!stderr_indicates_livestream(
            "ERROR: Video unavailable. This video is private"
        ));
        assert!(!stderr_indicates_livestream(""));
    }

    #[test]
    fn fetch_args_carry_the_audio_profile_and_optional_ffmpeg_location() {
        let with_ffmpeg = ytdlp_fetch_args(
            "C:\\out\\%(title)s [%(id)s].%(ext)s",
            Some("C:\\assets\\ffmpeg-runtime\\latest\\bin"),
            "https://youtu.be/abc",
        );
        // Audio extraction to mp3 (the `-x` short form of `--extract-audio`).
        assert!(with_ffmpeg.iter().any(|arg| arg == "-x"));
        let format = with_ffmpeg
            .iter()
            .position(|arg| arg == "--audio-format")
            .expect("--audio-format present");
        assert_eq!(with_ffmpeg[format + 1], "mp3");
        assert!(with_ffmpeg.iter().any(|arg| arg == "--no-playlist"));
        let progress = with_ffmpeg
            .iter()
            .position(|arg| arg == "--progress-template")
            .expect("--progress-template present");
        assert_eq!(with_ffmpeg[progress + 1], "YTDLP_PCT %(progress._percent_str)s\n");
        assert!(
            with_ffmpeg[progress + 1].ends_with('\n'),
            "the progress template must be newline-terminated"
        );
        // The literal `-o` path must survive verbatim, so `--restrict-filenames`
        // (which would rewrite it) must never be present.
        assert!(!with_ffmpeg.iter().any(|arg| arg == "--restrict-filenames"));
        // The path is precomputed and fixed, so no `--print` is parsed back.
        assert!(!with_ffmpeg.iter().any(|arg| arg == "--print"));
        assert!(with_ffmpeg.iter().any(|arg| arg == "--ignore-config"));
        assert!(with_ffmpeg.iter().any(|arg| arg == "--no-config-locations"));
        // Livestreams and oversized downloads are rejected.
        let filter = with_ffmpeg
            .iter()
            .position(|arg| arg == "--match-filter")
            .expect("--match-filter present");
        assert_eq!(with_ffmpeg[filter + 1], "!is_live");
        let max_size = with_ffmpeg
            .iter()
            .position(|arg| arg == "--max-filesize")
            .expect("--max-filesize present");
        assert_eq!(with_ffmpeg[max_size + 1], "2G");
        // The URL is always the final argv element (never a shell string), and a
        // literal `--` seals it as a positional argument.
        assert_eq!(with_ffmpeg.last().map(String::as_str), Some("https://youtu.be/abc"));
        assert_eq!(
            with_ffmpeg[with_ffmpeg.len() - 2].as_str(),
            "--",
            "the end-of-options token must immediately precede the URL"
        );
        // ffmpeg location is threaded through when supplied.
        let location = with_ffmpeg
            .iter()
            .position(|arg| arg == "--ffmpeg-location")
            .expect("--ffmpeg-location present");
        assert_eq!(with_ffmpeg[location + 1], "C:\\assets\\ffmpeg-runtime\\latest\\bin");

        // With no ffmpeg location, the flag is omitted entirely.
        let without_ffmpeg =
            ytdlp_fetch_args("out.%(ext)s", None, "https://youtu.be/abc");
        assert!(!without_ffmpeg.iter().any(|arg| arg == "--ffmpeg-location"));
    }

    /// A constant bitrate, because seeking depends on it.
    #[test]
    fn youtube_audio_is_fetched_at_a_constant_bitrate() {
        let args = ytdlp_fetch_args("out.%(ext)s", None, "https://youtu.be/abc");

        let quality = args
            .iter()
            .position(|arg| arg == "--audio-quality")
            .expect("an explicit audio quality, not yt-dlp's VBR default");
        // A bare number would be a VBR quality level; a bitrate is what asks for CBR.
        assert_eq!(args[quality + 1], "128K");
    }

    #[test]
    fn the_resolver_finds_the_exact_file_a_same_stem_one_or_none() {
        let dir = tempfile::tempdir().unwrap();
        // Exact match.
        let exact = dir.path().join("clip [id].mp3");
        std::fs::write(&exact, b"a").unwrap();
        assert_eq!(
            super::resolve_downloaded_audio_files(dir.path(), &exact),
            vec![exact.clone()]
        );

        let expected = dir.path().join("song [id2].mp3");
        let leftover = dir.path().join("song [id2].opus");
        std::fs::write(&leftover, b"a").unwrap();
        assert!(super::resolve_downloaded_audio_files(dir.path(), &expected).is_empty());

        // Nothing produced -> empty (a filter/livestream skip).
        let missing = dir.path().join("nope [id3].mp3");
        assert!(super::resolve_downloaded_audio_files(dir.path(), &missing).is_empty());
    }

    #[test]
    fn the_resolver_returns_every_entry_of_a_multi_video_link_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let expected = dir.path().join("tweet [id].mp3");

        // Written out of order, and with a two-digit index, so neither filesystem
        // order nor a lexicographic sort would produce the right answer by accident.
        let second = dir.path().join("tweet [id] 2.mp3");
        let tenth = dir.path().join("tweet [id] 10.mp3");
        let first = dir.path().join("tweet [id] 1.mp3");
        // The video a failed extract left whole is not audio, so it is never returned
        // as a recording however it is named.
        let leftover_video = dir.path().join("tweet [id] 3.mp4");
        let bystander = dir.path().join("tweet [id] extra.mp3");
        for path in [&second, &tenth, &first, &leftover_video, &bystander] {
            std::fs::write(path, b"a").unwrap();
        }

        assert_eq!(
            super::resolve_downloaded_audio_files(dir.path(), &expected),
            vec![first, second, tenth]
        );
    }

    #[test]
    fn a_literal_percent_is_doubled_in_the_template_but_not_in_the_expected_path() {
        // Both halves are literal text yt-dlp would otherwise read as a format spec:
        // the `100% Real` title AND a recordings folder that contains a percent.
        let output_directory = PathBuf::from("C:\\100%\\recordings");
        let template = ytdlp_output_template(&output_directory, "100% Real [abc123]");

        let literal = template
            .strip_suffix("%(playlist_index& {}|)s.%(ext)s")
            .expect("the template ends in the index and extension specs");
        assert!(!literal.contains("%(ext)s"));
        // Every literal percent is doubled, and un-doubling returns the exact path.
        assert!(literal.contains("100%% Real [abc123]"), "stem: {template}");
        assert_eq!(
            literal.replace("%%", "%"),
            output_directory
                .join("100% Real [abc123]")
                .display()
                .to_string()
        );

        // yt-dlp writes the UNESCAPED name to disk, so the precomputed path the
        // resolver looks for keeps its single literal percent. If these two ever
        // disagree, a produced download is reported as missing.
        let expected_output = output_directory.join("100% Real [abc123].mp3");
        assert_eq!(
            expected_output.file_name().and_then(|name| name.to_str()),
            Some("100% Real [abc123].mp3")
        );

        // A percent-free path is left exactly as it was.
        let plain = ytdlp_output_template(Path::new("C:\\recordings"), "clip [abc]");
        assert!(!plain.contains("%%"));
        assert!(plain.ends_with("clip [abc]%(playlist_index& {}|)s.%(ext)s"));
    }

    #[test]
    fn oversize_stderr_is_detected_across_wordings_and_case() {
        // yt-dlp does not treat the `--max-filesize` cap as an error: it prints this,
        // downloads nothing, and exits 0.
        assert!(stderr_indicates_oversize(
            "[download] File is larger than max-filesize (3221225472 > 2147483648). Aborting."
        ));
        assert!(stderr_indicates_oversize(
            "File is larger than --max-filesize (3221225472 > 2147483648)"
        ));
        // Matching is case-insensitive.
        assert!(stderr_indicates_oversize("FILE IS LARGER THAN MAX-FILESIZE"));
        // The two clean-exit skips must never be mistaken for each other.
        assert!(!stderr_indicates_oversize(
            "ERROR: [youtube] abc: Video has not passed filter (!is_live), skipping .."
        ));
        assert!(!stderr_indicates_livestream(
            "[download] File is larger than max-filesize (3221225472 > 2147483648). Aborting."
        ));
        assert!(!stderr_indicates_oversize(""));
    }

    #[test]
    fn js_runtime_stderr_is_detected_across_wordings_and_case() {
        // The nsig JS-runtime failure whose wording varies across yt-dlp versions.
        assert!(stderr_indicates_js_runtime(
            "ERROR: [youtube] abc: Failed to extract nsig function code; please install a JavaScript runtime"
        ));
        assert!(stderr_indicates_js_runtime(
            "ERROR: nsig extraction failed: Some formats may be missing"
        ));
        assert!(stderr_indicates_js_runtime(
            "ERROR: This extractor requires a JavaScript interpreter"
        ));
        // Matching is case-insensitive.
        assert!(stderr_indicates_js_runtime(
            "ERROR: NO SUPPORTED JS RUNTIME FOUND"
        ));
        // The other named skips must never be mistaken for it, nor it for them.
        assert!(!stderr_indicates_js_runtime(
            "ERROR: [youtube] abc: Video has not passed filter (!is_live), skipping .."
        ));
        assert!(!stderr_indicates_livestream("ERROR: please install a js runtime"));
        assert!(!stderr_indicates_js_runtime(""));
    }

    #[test]
    fn the_js_runtime_warning_every_youtube_run_prints_is_not_a_js_runtime_failure() {
        // Verbatim from yt-dlp 2026.08.19 with no runtime installed. It precedes
        // EVERY YouTube extraction, including the ones that go on to succeed, so
        // reading it as the cause made every YouTube failure — whatever it was —
        // retry three times and then tell the user to install a JavaScript runtime.
        let unavailable = "WARNING: [youtube] No supported JavaScript runtime could be found. \
             Only deno is enabled by default; to use another runtime add  --js-runtimes \
             RUNTIME[:PATH]  to your command/config. YouTube extraction without a JS runtime \
             has been deprecated, and some formats may be missing.\n\
             ERROR: [youtube] aaaaaaaaaaa: This video is unavailable";

        assert!(
            !stderr_indicates_js_runtime(unavailable),
            "the fatal line names the real cause; the warning above it is noise"
        );

        // The same warning must not suppress the real thing when it does happen.
        let genuine = format!("{unavailable}\nERROR: [youtube] abc: Failed to extract nsig function");
        assert!(stderr_indicates_js_runtime(&genuine));
    }

    #[test]
    fn a_multi_entry_link_announces_how_many_videos_it_holds() {
        // Verbatim from yt-dlp on a two-clip tweet, as the drain sees it once the
        // carriage returns are stripped and the line trimmed. This line is the ONLY
        // signal that a link held more videos than the run produced files for.
        assert_eq!(
            super::parse_ytdlp_item_total("[download] Downloading item 1 of 2"),
            Some(2)
        );
        assert_eq!(
            super::parse_ytdlp_item_total("[download] Downloading item 10 of 12"),
            Some(12)
        );

        // A single-video link never prints it, and its absence is what "one" means —
        // so nothing else may be mistaken for it.
        assert_eq!(super::parse_ytdlp_item_total(""), None);
        assert_eq!(
            super::parse_ytdlp_item_total("[download] Destination: clip of 2.mp3"),
            None
        );
        assert_eq!(
            super::parse_ytdlp_item_total("[download] Downloading item 1 of many"),
            None
        );
        assert_eq!(super::parse_ytdlp_item_total("YTDLP_PCT  42.0%"), None);
        // The prefix is what earns its keep here. This is a real line from the same
        // run, and its tail parses as a number just as readily — matching on " of N"
        // alone would read the count off whichever line happened to come last.
        assert_eq!(
            super::parse_ytdlp_item_total(
                "[twitter] Playlist Ben Davis - The computer use…: Downloading 2 items of 2"
            ),
            None
        );
    }

    #[test]
    fn every_named_failure_reason_carries_its_own_sentence() {
        // The reason is decided once, from intact output, and rendered here. It used
        // to be rendered first and re-decided by reading the rendered sentence back,
        // which stopped working as soon as that sentence gained a prefix.
        use super::FetchFailure;
        assert_eq!(
            FetchFailure::JsRuntime.message(),
            super::JS_RUNTIME_REJECTED_MESSAGE
        );
        assert_eq!(
            FetchFailure::Livestream.message(),
            super::LIVESTREAM_REJECTED_MESSAGE
        );
        assert_eq!(
            FetchFailure::Oversize.message(),
            super::OVERSIZE_REJECTED_MESSAGE
        );
        assert_eq!(
            FetchFailure::NoAudio.message(),
            super::NO_AUDIO_REJECTED_MESSAGE
        );
        assert_eq!(
            FetchFailure::Other("raw tool text".into()).message(),
            "raw tool text"
        );
    }

    #[test]
    fn the_failure_detail_keeps_the_end_and_prefers_the_fatal_lines() {
        // yt-dlp opens with warnings and closes with the line that explains the run,
        // so a cap taken from the front shows the least useful part of a long failure.
        let chatty = format!(
            "{}\nERROR: [youtube] abc: This video is unavailable",
            "WARNING: [youtube] some advisory line that goes on\n".repeat(40)
        );
        let detail = super::failure_detail(&chatty);
        assert_eq!(detail, "ERROR: [youtube] abc: This video is unavailable");

        // With no fatal line marked, the END of the output is kept, not the start.
        let unmarked = format!("{}TAIL", "x".repeat(1000));
        let detail = super::failure_detail(&unmarked);
        assert!(detail.ends_with("TAIL"), "kept: {detail}");
        assert_eq!(detail.chars().count(), 600);

        // Short output survives whole.
        assert_eq!(super::failure_detail("brief"), "brief");
    }

    #[test]
    fn a_video_title_is_never_classified_as_the_reason_the_run_failed() {
        // yt-dlp quotes the output path back, so the TITLE arrives inside its error
        // text. Without stripping our own inputs first, this video is refused as a
        // livestream and the real reason is discarded.
        let stem = "Cruella marks return of film premieres in Hollywood [ab12]";
        let stderr = format!(
            "ERROR: Postprocessing: Error opening output file C:\\rec\\{stem}.mp3"
        );
        assert!(
            stderr_indicates_livestream(&stderr),
            "the raw text does contain the needle — that is the trap"
        );
        assert!(!stderr_indicates_livestream(&super::without_reflected_inputs(
            &stderr, "https://x.com/a/1", stem
        )));

        // The URL is stripped too, but defensively rather than against a proven trap:
        // every needle contains a space and a valid URL percent-encodes spaces, so a
        // reflected URL cannot match one. Stripping it costs nothing and removes the
        // question.
        let url = "https://example.com/watch/this-is-not-live-anymore";
        assert!(
            !stderr_indicates_livestream(&format!("ERROR: Unable to download {url}")),
            "hyphens are not spaces, so this never matched in the first place"
        );

        // And yt-dlp's own wording still classifies.
        assert!(stderr_indicates_livestream(&super::without_reflected_inputs(
            "ERROR: [youtube] TQRjW0WSdy0: This live event will begin in 46 hours.",
            "https://x.com/a/1",
            "clip [ab12]",
        )));
    }

    #[test]
    fn only_the_mp3_a_fetch_asked_for_counts_as_produced() {
        // `-x` downloads the source and converts second, so these are what a FAILED
        // extract leaves behind — not the recording the run was supposed to make.
        for leftover in ["clip.webm", "clip.m4a", "clip.opus", "clip.mp4"] {
            assert!(
                !super::is_produced_audio(Path::new(leftover)),
                "{leftover} is a leftover download, not a produced recording"
            );
        }
        assert!(super::is_produced_audio(Path::new("clip.mp3")));
        assert!(super::is_produced_audio(Path::new("clip.MP3")));
    }

    #[test]
    fn a_stem_is_refused_when_a_case_variant_already_answers_to_it() {
        let dir = tempfile::tempdir().unwrap();
        // Windows treats these as the same file, so yt-dlp would find the output
        // "already downloaded" and write nothing at all.
        std::fs::write(dir.path().join("NARUTO OP [ab12].mp3"), b"x").unwrap();
        assert_eq!(
            super::unique_import_stem(dir.path(), "Naruto OP [ab12]").unwrap(),
            "Naruto OP [ab12]_1"
        );
    }

    #[test]
    fn a_folder_that_cannot_be_read_refuses_to_claim_a_stem() {
        // "I could not look" must never become "nothing is there": the sweep would
        // then delete an earlier import's recordings under the same stem.
        let missing = std::path::Path::new("C:\\definitely\\not\\a\\folder\\here");
        assert!(super::unique_import_stem(missing, "clip [ab12]").is_err());
    }

    #[test]
    fn a_silent_link_is_answered_with_the_import_sentence() {
        // The two tool wordings are pinned in `media_errors`; what this path owns is
        // which sentence they are answered with.
        assert!(stderr_indicates_no_audio(
            "ERROR: Postprocessing: WARNING: unable to obtain file audio codec with ffprobe"
        ));
        assert_eq!(
            super::NO_AUDIO_REJECTED_MESSAGE,
            "This video has no sound, so there is nothing to import."
        );
    }

    #[test]
    fn the_stderr_buffer_drops_the_oldest_lines_not_the_decisive_last_one() {
        let mut tail = StderrTail::default();
        // Far more warning text than the cap holds, exactly as a chatty extractor
        // produces, and then the one line that says what actually failed.
        for index in 0..400 {
            tail.push(format!(
                "WARNING: [youtube] filler line {index} padded out to a realistic width \
                 so the buffer has to start dropping"
            ));
        }
        tail.push("ERROR: [youtube] abc: Failed to extract nsig function".to_string());

        let text = tail.text();
        assert!(
            text.contains("Failed to extract nsig"),
            "the last line is the one worth keeping"
        );
        assert!(
            !text.contains("filler line 0 "),
            "the oldest lines are the ones dropped"
        );
        assert!(text.len() <= StderrTail::MAX_BYTES + 200);
        // And the classification the retry depends on still lands.
        assert!(stderr_indicates_js_runtime(&text));
    }

    #[test]
    fn a_single_line_longer_than_the_cap_is_still_kept() {
        let mut tail = StderrTail::default();
        tail.push(format!("ERROR: {}", "x".repeat(StderrTail::MAX_BYTES * 2)));
        assert!(
            tail.text().starts_with("ERROR: xxx"),
            "an over-long line is the only account of the failure there is"
        );
    }

    #[test]
    fn the_video_id_is_sanitized_into_the_stem_like_the_title() {
        assert_eq!(youtube_output_stem("Clip", "abc123"), "Clip [abc123]");
        // yt-dlp serves far more than YouTube: an extractor id carrying path
        // characters must not reach the filesystem raw.
        assert_eq!(
            youtube_output_stem("Clip", "series:42/part"),
            "Clip [series 42 part]"
        );
        assert_eq!(youtube_output_stem("A/B: C", "abc123"), "A B C [abc123]");
        // Either half missing degrades to the other; neither leaves an empty stem.
        assert_eq!(youtube_output_stem("Clip", ""), "Clip");
        assert_eq!(youtube_output_stem("", "abc123"), "abc123");
        assert_eq!(youtube_output_stem("", ""), "youtube");
        // An id that sanitizes away entirely must not leave dangling brackets.
        assert_eq!(youtube_output_stem("Clip", "//"), "Clip");
    }

    #[test]
    fn the_probe_line_parses_from_both_ends_so_a_tab_in_the_title_cannot_desync_it() {
        let metadata = parse_probe_metadata_line("not_live\tMy Clip\tabc123");
        assert_eq!(metadata.live_status, "not_live");
        assert_eq!(metadata.title, "My Clip");
        assert_eq!(metadata.id, "abc123");

        // A tab inside the title stays in the title; a left-to-right split would have
        // folded " B [x]\tabc123" apart and named the file after the title's tail.
        let tabbed = parse_probe_metadata_line("is_live\tA\tB [x]\tabc123");
        assert_eq!(tabbed.live_status, "is_live");
        assert_eq!(tabbed.title, "A\tB [x]");
        assert_eq!(tabbed.id, "abc123");
        // The interior tab is a control character, so the stem never sees it.
        assert_eq!(youtube_output_stem(&tabbed.title, &tabbed.id), "A B [x] [abc123]");

        // A malformed/empty probe line yields empty fields rather than panicking.
        let empty = parse_probe_metadata_line("");
        assert_eq!(empty.live_status, "");
        assert_eq!(empty.title, "");
        assert_eq!(empty.id, "");
    }

    #[test]
    fn the_fragment_sweep_only_removes_this_imports_own_leftovers() {
        let dir = tempfile::tempdir().unwrap();
        let own_part = dir.path().join("clip [id].mp3.part");
        let own_ytdl = dir.path().join("clip [id].webm.ytdl");
        let own_temp = dir.path().join("clip [id].mp3.temp");
        let own_fragment = dir.path().join("clip [id].f251.webm.part-Frag1");
        let own_finished = dir.path().join("clip [id].mp3");
        // The recordings folder may be a shared one (Downloads); this is somebody
        // else's in-flight download.
        let other_part = dir.path().join("movie.mp4.part");
        for path in [
            &own_part,
            &own_ytdl,
            &own_temp,
            &own_fragment,
            &own_finished,
            &other_part,
        ] {
            std::fs::write(path, b"x").unwrap();
        }

        super::sweep_import_artifacts(dir.path(), "clip [id]", &[]);

        assert!(!own_part.exists());
        assert!(!own_ytdl.exists());
        assert!(!own_temp.exists());
        assert!(!own_fragment.exists());
        assert!(other_part.exists(), "an unrelated download must survive");
        assert!(
            !own_finished.exists(),
            "a finished file this import produced is swept too: leaving it behind is \
             what made the next attempt at the same link skip the download and fail \
             on the stale copy"
        );

        // An empty stem would prefix-match every file, so the sweep refuses to run.
        super::sweep_import_artifacts(dir.path(), "", &[]);
        assert!(other_part.exists());
    }

    #[test]
    fn the_sweep_keeps_what_a_partly_successful_run_produced() {
        let dir = tempfile::tempdir().unwrap();
        // A two-video link where the first clip was silent: the extract failed and
        // left the video whole, while the second clip produced real audio.
        let kept = dir.path().join("tweet [id] 2.mp3");
        let failed_video = dir.path().join("tweet [id] 1.mp4");
        let fragment = dir.path().join("tweet [id] 1.mp4.part");
        // Somebody else's file, in a folder that may well be shared.
        let bystander = dir.path().join("tweet [id] mine.mp3");
        for path in [&kept, &failed_video, &fragment, &bystander] {
            std::fs::write(path, b"x").unwrap();
        }

        super::sweep_import_artifacts(dir.path(), "tweet [id]", std::slice::from_ref(&kept));

        assert!(kept.exists(), "the recording that landed must survive");
        assert!(!failed_video.exists());
        assert!(!fragment.exists());
        assert!(bystander.exists(), "not this import's file");
    }

    #[test]
    fn a_stem_is_only_taken_when_nothing_on_disk_already_answers_to_it() {
        let dir = tempfile::tempdir().unwrap();

        // Nothing there: the base stem is free.
        assert_eq!(super::unique_import_stem(dir.path(), "clip [id]").unwrap(), "clip [id]");

        // An earlier single-video import of the same link.
        std::fs::write(dir.path().join("clip [id].mp3"), b"x").unwrap();
        assert_eq!(
            super::unique_import_stem(dir.path(), "clip [id]").unwrap(),
            "clip [id]_1"
        );

        // An earlier MULTI-video import leaves `<stem>.mp3` free while owning the
        // numbered files. Reusing the stem here would hand the failure sweep two
        // recordings that belong to that earlier import.
        let dir2 = tempfile::tempdir().unwrap();
        std::fs::write(dir2.path().join("tweet [id] 1.mp3"), b"x").unwrap();
        std::fs::write(dir2.path().join("tweet [id] 2.mp3"), b"x").unwrap();
        assert!(!dir2.path().join("tweet [id].mp3").exists());
        assert_eq!(
            super::unique_import_stem(dir2.path(), "tweet [id]").unwrap(),
            "tweet [id]_1"
        );

        // A file that merely starts the same way is nobody's business but its own.
        let dir3 = tempfile::tempdir().unwrap();
        std::fs::write(dir3.path().join("clip [id] notes.txt"), b"x").unwrap();
        assert_eq!(
            super::unique_import_stem(dir3.path(), "clip [id]").unwrap(),
            "clip [id]"
        );
    }

    #[test]
    fn only_the_stem_plus_an_index_and_an_extension_is_this_imports_own_file() {
        // What yt-dlp produces from our template.
        assert!(is_own_import_artifact("clip [id].mp3", "clip [id]"));
        assert!(is_own_import_artifact("clip [id].mp4.part", "clip [id]"));
        assert!(is_own_import_artifact("clip [id] 2.mp3", "clip [id]"));
        assert!(is_own_import_artifact("clip [id] 17.f251.webm", "clip [id]"));

        // What it never produces, and must therefore never delete: the recordings
        // folder may be a shared one, and these are somebody else's.
        assert!(!is_own_import_artifact("clip [id] extra.mp3", "clip [id]"));
        assert!(!is_own_import_artifact("clip [id]2.mp3", "clip [id]"));
        assert!(!is_own_import_artifact("clip [id] .mp3", "clip [id]"));
        assert!(!is_own_import_artifact("clip [id2].mp3", "clip [id]"));
        assert!(!is_own_import_artifact("movie.mp4.part", "clip [id]"));
    }
}
