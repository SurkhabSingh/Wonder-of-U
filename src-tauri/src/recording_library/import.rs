use std::{
    fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use tauri::{AppHandle, Emitter, Listener, Manager, Runtime};

use crate::{
    app_runtime::{build_app_bootstrap, ensure_directory_exists, log_event, now_ms, update_shell_snapshot},
    app_state::sanitize_recording_name,
    app_types::{
        AppSettings, RecentRecording, RecordingActionItem, RecordingBatchResult,
        SharedPersistedState,
    },
    runtime_assets::{detect_local_ffmpeg, detect_local_ytdlp},
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

const YTDLP_REQUIRED_MESSAGE: &str =
    "yt-dlp is required to import from YouTube; install it in Setup.";
const YOUTUBE_FFMPEG_REQUIRED_MESSAGE: &str =
    "FFmpeg is required to import from YouTube; install it in Setup.";
const LIVESTREAM_REJECTED_MESSAGE: &str =
    "This is a live or upcoming stream, so it can't be imported.";

/// True when yt-dlp's stderr indicates a live/upcoming/premiere video was rejected.
/// Matched case-insensitively because the wording differs across yt-dlp versions
/// (the `--match-filter` rejection prints `has not passed filter` on current
/// releases, not the older `does not pass filter`) and across the live/upcoming
/// cases (`premieres in`, `live event will begin`).
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

/// The metadata a single up-front probe collects: the video's `live_status` (so a
/// live/upcoming/premiere can be refused without downloading) plus its `title` and
/// `id`, which pin a guaranteed-unique output path before the fetch begins.
struct VideoMetadata {
    live_status: String,
    title: String,
    id: String,
}

/// Quick metadata-only probe of a URL before any download. One `--print` returns
/// `live_status<TAB>title<TAB>id` on a clean run; on failure it returns `Err`
/// carrying yt-dlp's stderr (network/unavailable). The caller treats a failing
/// probe as advisory — a flaky probe must never block a normal video — but uses the
/// title/id it does return to name a collision-proof output file.
fn probe_video_metadata(ytdlp_executable: &str, url: &str) -> Result<VideoMetadata, String> {
    let mut command = Command::new(ytdlp_executable);
    hide_command_window(&mut command);
    // Same frozen-Python buffering caveat as the fetch; harmless here.
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
    let line = stdout.lines().next().unwrap_or("");
    let mut fields = line.splitn(3, '\t');
    Ok(VideoMetadata {
        live_status: fields.next().unwrap_or("").trim().to_string(),
        title: fields.next().unwrap_or("").trim().to_string(),
        id: fields.next().unwrap_or("").trim().to_string(),
    })
}

/// True when a probed `live_status` marks a stream we must never download.
fn live_status_is_stream(live_status: &str) -> bool {
    matches!(live_status, "is_live" | "is_upcoming" | "post_live")
}

/// What a completed yt-dlp fetch produced, or the signal that the user cancelled.
/// The path is the caller's precomputed `expected_output` in the normal case, or a
/// same-stem fallback the resolver found if the literal name was munged.
enum FetchOutcome {
    Completed { path: PathBuf },
    Cancelled,
}

/// Resolves the audio file a successful fetch produced. Normally it is exactly
/// `expected_output`; if that precise name is absent, fall back to any same-stem
/// audio file in the output directory so a produced download is never reported as
/// missing. Returns `None` when nothing was produced (a filter/livestream skip on a
/// clean exit).
fn resolve_downloaded_audio(output_directory: &Path, expected_output: &Path) -> Option<PathBuf> {
    if expected_output.is_file() {
        return Some(expected_output.to_path_buf());
    }
    let stem = expected_output.file_stem().and_then(|value| value.to_str())?;
    for entry in fs::read_dir(output_directory).ok()?.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let same_stem = path
            .file_stem()
            .and_then(|value| value.to_str())
            .map(|value| value == stem)
            .unwrap_or(false);
        let is_audio = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| {
                matches!(
                    value.to_ascii_lowercase().as_str(),
                    "mp3" | "m4a" | "opus" | "webm" | "ogg" | "oga" | "aac" | "wav" | "flac"
                )
            })
            .unwrap_or(false);
        if same_stem && is_audio {
            return Some(path);
        }
    }
    None
}

/// Validates a user-supplied import URL with a light parse: it must carry an
/// `http`/`https` scheme and a non-empty host. Returns the trimmed URL (original
/// case preserved) so it can be handed to yt-dlp verbatim as a single argv
/// element — the URL never reaches a shell.
fn validate_import_url(raw: &str) -> Result<String, String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Err("Enter a video URL to import from YouTube.".into());
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

/// Parses a progress line into a 0..=100 percent. The `download:` in a
/// `--progress-template "download:…"` is yt-dlp's *type selector* — it is consumed,
/// never emitted — so the template prefixes an explicit `YTDLP_PCT` marker that DOES
/// reach stdout. The emitted line is e.g. `YTDLP_PCT   42.3%`; we strip the marker
/// and the trailing `%`. Anything unparseable (`N/A`, ANSI colouring) is treated as
/// "no update" and yields `None`.
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

/// Builds the yt-dlp argument list. Kept pure so the profile can be asserted
/// without spawning yt-dlp. `output_template` is a caller-precomputed, guaranteed
/// unique path (`…/<stem>.%(ext)s`), so the final file lands at a location we
/// already know — there is no `--print` to parse back off stdout.
///
/// The `--progress-template` emits an explicit `YTDLP_PCT` marker and, crucially,
/// ends in a literal `\n` so each progress render is its own line — that is what
/// lets a plain `BufReader::lines()` yield one update per line without passing
/// `--newline` (the same mechanism vibe's proven downloader relies on).
///
/// Hardening: `--ignore-config`/`--no-config-locations` lead so a stray
/// `yt-dlp.conf` (beside the binary or in the user config path) can never merge
/// in options like `--exec`; `--match-filter "!is_live"` rejects never-ending
/// livestreams; `--max-filesize` bounds a hostile/huge download; and a literal
/// `--` seals the URL as a positional argument even if validation were bypassed.
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
    ];
    if let Some(location) = ffmpeg_location {
        args.push("--ffmpeg-location".into());
        args.push(location.to_string());
    }
    args.extend([
        // The trailing `\n` is essential: it forces one newline-terminated line per
        // progress render so the stdout loop reads updates as they stream. NB: no
        // `--restrict-filenames`. `output_template` is an already-sanitized,
        // caller-precomputed literal path; that flag would rewrite the literal
        // (spaces→underscores, brackets/non-ASCII stripped) so the produced file's
        // name would no longer equal `expected_output` — the file would then be
        // "missing" and the import fail. yt-dlp writes the literal path verbatim.
        "--progress-template".into(),
        "YTDLP_PCT %(progress._percent_str)s\n".into(),
        "-o".into(),
        output_template.to_string(),
        "--".into(),
        url.to_string(),
    ]);
    args
}

/// True when `candidate` resolves to a location inside `root`. Both sides are
/// canonicalized (resolving `..`, symlinks, and Windows verbatim prefixes) so a
/// crafted output template can never smuggle the produced file outside the
/// recordings folder. A candidate that cannot be canonicalized is treated as
/// outside — it is refused rather than registered.
fn path_is_within(root: &Path, candidate: &Path) -> bool {
    match (root.canonicalize(), candidate.canonicalize()) {
        (Ok(root), Ok(candidate)) => candidate.starts_with(&root),
        _ => false,
    }
}

/// Runs yt-dlp to completion, emitting a `youtube-progress` percent per line, and
/// BLOCKS until the download finishes. `output_template` (`…/<stem>.%(ext)s`) and
/// `expected_output` (`…/<stem>.mp3`) are precomputed and collision-proof, so the
/// fetch never has to parse the produced path back off stdout: on a clean exit it
/// just resolves `expected_output`.
///
/// This mirrors vibe's proven downloader: register a one-shot `youtube-cancel`
/// listener that flips an `AtomicBool`; drain stdout in a single blocking loop on
/// this (spawn_blocking) thread, emitting progress and checking the cancel flag
/// once per line; drain stderr on a separate thread into a bounded buffer so a
/// chatty stderr can't fill the pipe and wedge yt-dlp; then `wait()` and join the
/// stderr thread. Because the loop owns the stdout pipe and reads it to EOF before
/// `wait()`, the command always returns cleanly — the frontend `await` resolves.
fn fetch_youtube_audio<R: Runtime>(
    app: &AppHandle<R>,
    ytdlp_executable: &str,
    ffmpeg_location: Option<&str>,
    output_directory: &Path,
    output_template: &str,
    expected_output: &Path,
    url: &str,
) -> Result<FetchOutcome, String> {
    let args = ytdlp_fetch_args(output_template, ffmpeg_location, url);

    let mut command = Command::new(ytdlp_executable);
    hide_command_window(&mut command);
    // yt-dlp is a PyInstaller-frozen Python binary; `PYTHONUNBUFFERED=1` keeps its
    // stdout unbuffered so progress lines flush as they happen rather than in a burst.
    command.env("PYTHONUNBUFFERED", "1");
    command
        .args(&args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    // Register the cancel listener BEFORE spawning so a Cancel arriving the instant
    // the download starts is never missed. The stdout loop reads this flag per line.
    let cancel = Arc::new(AtomicBool::new(false));
    let cancel_for_listener = Arc::clone(&cancel);
    app.once("youtube-cancel", move |_| {
        cancel_for_listener.store(true, Ordering::Relaxed);
    });

    let mut child = command
        .spawn()
        .map_err(|error| format!("Could not start yt-dlp: {error}"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "yt-dlp produced no stdout stream.".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "yt-dlp produced no stderr stream.".to_string())?;

    // stderr carries only error/warning text for this invocation; drain it on its
    // own thread into a bounded buffer so a chatty stderr can never fill the pipe
    // and block yt-dlp. Joined after the child is reaped.
    let stderr_buffer: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let stderr_sink = Arc::clone(&stderr_buffer);
    let stderr_thread = thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Ok(mut sink) = stderr_sink.lock() {
                if sink.len() < 8192 {
                    sink.push_str(&line);
                    sink.push('\n');
                }
            }
        }
    });

    // Drain stdout in a single blocking loop on THIS thread. Each `YTDLP_PCT …%`
    // line emits a plain percent number on `youtube-progress`. The cancel flag is
    // checked once per line; on Cancel we kill the whole process tree (yt-dlp plus
    // its ffmpeg grandchild) and stop reading.
    for line in BufReader::new(stdout).lines() {
        if cancel.load(Ordering::Relaxed) {
            let child_pid = child.id();
            let _ = child.kill();
            kill_process_tree(child_pid);
            break;
        }
        let Ok(line) = line else { break };
        let line = line.replace('\r', "");
        let line = line.trim();
        if let Some(percent) = parse_ytdlp_progress_line(line) {
            let _ = app.emit("youtube-progress", percent);
        }
    }

    let exit_status = child
        .wait()
        .map_err(|error| format!("yt-dlp did not exit cleanly: {error}"))?;
    let _ = stderr_thread.join();

    let stderr_text = stderr_buffer
        .lock()
        .ok()
        .map(|guard| guard.trim().to_string())
        .unwrap_or_default();

    // A Cancel reached the child (killed in the loop above and reaped by `wait`):
    // remove the precomputed partial output, sweep leftover `.part`/temp fragments,
    // and report the cancellation.
    if cancel.load(Ordering::Relaxed) {
        let _ = fs::remove_file(expected_output);
        sweep_partial_fragments(output_directory);
        return Ok(FetchOutcome::Cancelled);
    }

    if !exit_status.success() {
        // Upcoming/premiere videos fail on this non-zero-exit path (the filter only
        // fires for already-live streams), so translate the raw stderr into the
        // friendly livestream message before falling back to the generic error.
        if stderr_indicates_livestream(&stderr_text) {
            return Err(LIVESTREAM_REJECTED_MESSAGE.to_string());
        }
        let capped: String = stderr_text.chars().take(600).collect();
        return Err(if capped.is_empty() {
            "yt-dlp could not download this video.".to_string()
        } else {
            format!("yt-dlp could not download this video: {capped}")
        });
    }

    // Resolve the produced audio file (normally exactly `expected_output`). No file
    // at all on a clean exit is what a `--match-filter` rejection (livestream) looks
    // like — yt-dlp skips the video and exits 0.
    match resolve_downloaded_audio(output_directory, expected_output) {
        Some(path) => {
            // The produced file must live inside the recordings folder; a path that
            // escaped it is refused rather than registered into the library.
            if !path_is_within(output_directory, &path) {
                return Err("yt-dlp wrote outside the recordings folder.".into());
            }
            Ok(FetchOutcome::Completed { path })
        }
        None => {
            if stderr_indicates_livestream(&stderr_text) {
                Err(LIVESTREAM_REJECTED_MESSAGE.into())
            } else {
                Err("yt-dlp finished but did not produce an audio file.".into())
            }
        }
    }
}

/// Best-effort kill of the whole process tree rooted at `pid`. yt-dlp spawns
/// ffmpeg as a child for `--extract-audio`; a bare `child.kill()` leaves that
/// grandchild running on Windows, so we `taskkill /T` the tree. Errors are
/// swallowed: the process may already be gone, which is the desired end state.
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

/// Best-effort sweep of leftover download fragments in the recordings folder
/// after a cancelled import. yt-dlp writes `.part` (and fragment/temp) files it
/// only renames on success, so a mid-download kill can strand them.
fn sweep_partial_fragments(directory: &Path) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let is_fragment = path
            .file_name()
            .and_then(|name| name.to_str())
            .map(|name| {
                name.ends_with(".part")
                    || name.ends_with(".ytdl")
                    || name.ends_with(".temp")
                    || name.contains(".part-")
            })
            .unwrap_or(false);
        if is_fragment {
            let _ = fs::remove_file(&path);
        }
    }
}

/// Probes and registers a freshly fetched YouTube file into the library, exactly
/// like a local import but tagged with its origin URL.
fn register_youtube_recording<R: Runtime>(
    app: &AppHandle<R>,
    settings: &AppSettings,
    final_path: &Path,
    title: Option<String>,
    source_url: &str,
) -> Result<RecentRecording, String> {
    let file_name = final_path
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("youtube")
        .to_string();

    let bytes_written = fs::metadata(final_path)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    if bytes_written == 0 {
        let _ = fs::remove_file(final_path);
        return Err("yt-dlp produced an empty audio file.".into());
    }

    let ffmpeg_executable = detect_local_ffmpeg(settings).executable_path;
    let duration_ms = probe_duration_ms(ffmpeg_executable.as_deref(), final_path);

    let recording = RecentRecording {
        file_name: file_name.clone(),
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
        title: title.or(Some(file_name)),
    };

    insert_recent_recording(app, recording.clone())?;
    Ok(recording)
}

/// Turns a fetch failure into the failed single-item batch the frontend renders.
/// Cancellation is not a hard error, so this returns `Ok` with a failed item rather
/// than `Err`.
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

    // --extract-audio needs ffmpeg; pass its directory to yt-dlp so a managed
    // install (not on PATH) is still found.
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

    // One up-front probe does double duty. First, refuse a live/upcoming/premiere
    // stream before downloading anything (`--match-filter "!is_live"` stays as a
    // backstop, but it only catches already-live streams and its clean-exit rejection
    // is easy to miss). Second, it returns the title and id used to pin a
    // collision-proof output path. A probe that errors (flaky network, region block)
    // is advisory: we fall through to the normal fetch rather than block a normal
    // video — unless its stderr clearly names a livestream, which we still refuse.
    let metadata = match probe_video_metadata(&ytdlp_executable, &normalized_url) {
        Ok(metadata) => {
            if live_status_is_stream(&metadata.live_status) {
                return Err(LIVESTREAM_REJECTED_MESSAGE.to_string());
            }
            Some(metadata)
        }
        Err(stderr) => {
            if stderr_indicates_livestream(&stderr) {
                return Err(LIVESTREAM_REJECTED_MESSAGE.to_string());
            }
            None
        }
    };

    // Precompute a guaranteed-unique output path from the probed title+id, exactly
    // like a local import uses `unique_path_with_suffix`. This is what makes two
    // imports of the same video (or two same-titled videos) land on DISTINCT paths
    // instead of colliding into one library entry. yt-dlp gets a FIXED `-o` — a
    // literal stem, not a `%(title)s [%(id)s]` template — so the final file lands
    // exactly where we expect and no path has to be parsed back off stdout.
    let (video_title, output_stem) = match &metadata {
        Some(metadata) => {
            let sanitized_title = sanitize_recording_name(&metadata.title);
            let id = metadata.id.trim();
            let stem = match (sanitized_title.is_empty(), id.is_empty()) {
                (false, false) => format!("{sanitized_title} [{id}]"),
                (false, true) => sanitized_title.clone(),
                (true, false) => id.to_string(),
                (true, true) => "youtube".to_string(),
            };
            let title = (!metadata.title.trim().is_empty())
                .then(|| metadata.title.trim().to_string());
            (title, stem)
        }
        // The probe failed but wasn't a livestream: proceed with a generic stem
        // (uniqueness is still guaranteed by the suffixing below) and let the title
        // fall back to the final file name.
        None => (None, "youtube".to_string()),
    };

    let expected_output = unique_path_with_suffix(&output_directory, &output_stem, ".mp3");
    // Reuse the exact (possibly `_N`-suffixed) stem the uniqueness check chose, and
    // hand yt-dlp `<stem>.%(ext)s` so it downloads/extracts to `<stem>.mp3`.
    let unique_file_name = expected_output
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("youtube.mp3");
    let unique_stem = unique_file_name
        .strip_suffix(".mp3")
        .unwrap_or(unique_file_name);
    let output_template = output_directory
        .join(format!("{unique_stem}.%(ext)s"))
        .display()
        .to_string();

    // Single-flight is now the frontend's sequential import loop; cancellation is
    // the `youtube-cancel` event + `AtomicBool` inside `fetch_youtube_audio`. There
    // is no shared control slot or model-download snapshot on this path anymore —
    // that machinery is what kept the command from returning cleanly. Progress is
    // streamed as `youtube-progress` events; the shell just shows a status line.
    update_shell_snapshot(app, |shell| {
        shell.status_text = "Importing audio from YouTube…".into();
        shell.transition_count += 1;
    })?;

    match fetch_youtube_audio(
        app,
        &ytdlp_executable,
        ffmpeg_location.as_deref(),
        &output_directory,
        &output_template,
        &expected_output,
        &normalized_url,
    ) {
        Ok(FetchOutcome::Completed { path }) => {
            match register_youtube_recording(
                app,
                &settings,
                &path,
                video_title,
                &normalized_url,
            ) {
                Ok(recording) => {
                    let message = format!("Imported {} from YouTube.", recording.file_name);
                    let status_text = message.clone();
                    update_shell_snapshot(app, |shell| {
                        shell.status_text = status_text;
                        shell.transition_count += 1;
                    })?;
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

                    Ok(RecordingBatchResult {
                        status: "completed".into(),
                        message: message.clone(),
                        items: vec![RecordingActionItem {
                            file_path: recording.file_path,
                            status: "success".into(),
                            message,
                            note_id: None,
                        }],
                        bootstrap: build_app_bootstrap(app)?,
                    })
                }
                Err(message) => finish_youtube_failure(app, &normalized_url, message),
            }
        }
        Ok(FetchOutcome::Cancelled) => {
            update_shell_snapshot(app, |shell| {
                shell.status_text = "YouTube import cancelled.".into();
                shell.transition_count += 1;
            })?;

            Ok(RecordingBatchResult {
                status: "cancelled".into(),
                message: "YouTube import cancelled.".into(),
                items: vec![RecordingActionItem {
                    file_path: normalized_url.clone(),
                    status: "failed".into(),
                    message: "YouTube import cancelled.".into(),
                    note_id: None,
                }],
                bootstrap: build_app_bootstrap(app)?,
            })
        }
        Err(message) => finish_youtube_failure(app, &normalized_url, message),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        classify_extension, convert_ffmpeg_args, ffprobe_path_for, is_same_file,
        parse_ffprobe_duration_ms, parse_ytdlp_progress_line, stderr_indicates_livestream,
        validate_import_url, ytdlp_fetch_args, ImportPlan,
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
        // Progress is emitted on stdout with an explicit `YTDLP_PCT` marker, and the
        // template MUST end in a literal newline so each render is its own line the
        // blocking stdout loop can read (the mechanism that keeps updates flowing
        // without `--newline`).
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
        // Config files must never merge in (e.g. a malicious --exec).
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

    #[test]
    fn resolve_downloaded_audio_prefers_exact_then_same_stem_fallback() {
        let dir = tempfile::tempdir().unwrap();
        // Exact match wins.
        let exact = dir.path().join("clip [id].mp3");
        std::fs::write(&exact, b"a").unwrap();
        assert_eq!(
            super::resolve_downloaded_audio(dir.path(), &exact),
            Some(exact.clone())
        );

        // Same stem, different (audio) extension is the fallback when the exact
        // .mp3 is absent.
        let expected = dir.path().join("song [id2].mp3");
        let produced = dir.path().join("song [id2].opus");
        std::fs::write(&produced, b"a").unwrap();
        assert_eq!(
            super::resolve_downloaded_audio(dir.path(), &expected),
            Some(produced)
        );

        // Nothing produced -> None (a filter/livestream skip).
        let missing = dir.path().join("nope [id3].mp3");
        assert_eq!(super::resolve_downloaded_audio(dir.path(), &missing), None);
    }

}
