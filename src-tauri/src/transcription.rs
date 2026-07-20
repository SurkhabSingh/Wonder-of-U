use std::{
    env, fs,
    io::{BufRead, BufReader},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Audio at least this long is transcribed in overlapping ~5-minute chunks so drift
/// can never accumulate across a long file. Shorter audio takes the single-shot path.
const CHUNK_LEN_MS: u64 = 300_000;
/// Each chunk's clip extends this far past its kept window on both sides. whisper
/// hallucinates near a clip's edges (it treats the cut as an audio boundary), so the
/// overlap regions — where those artifacts land — are transcribed but then discarded,
/// leaving only each chunk's clean interior. Validated at ~15s; 20s adds margin.
const OVERLAP_MS: u64 = 20_000;

#[derive(Debug, Clone)]
pub struct WhisperTranscriptionRequest {
    pub cli_path: PathBuf,
    pub model_path: PathBuf,
    pub audio_path: PathBuf,
    pub language: String,
    /// ffmpeg, used to split long audio into ~5-minute chunks so each is transcribed
    /// drift-free and stitched by exact time offset (the permanent long-audio timestamp
    /// fix). `None` → single-shot transcription, no chunking.
    pub ffmpeg_path: Option<PathBuf>,
    /// Total audio duration in ms; drives chunk planning. `0` (unknown) → single-shot.
    pub duration_ms: u64,
}

#[derive(Debug, Clone)]
pub struct WhisperTranscriptionResult {
    pub transcript_path: PathBuf,
    /// Expected path of whisper's `--output-json` sidecar carrying per-segment
    /// offsets. It may not exist if whisper skipped writing it; callers parse it
    /// best-effort and never fail transcription over a missing json.
    pub json_path: PathBuf,
}

/// A minimal mirror of whisper's `--output-json` shape — `{ "transcription": [ {
/// "offsets": { "from", "to" }, "text" } ] }`. `recording_library/transcription.rs`
/// has its own `Deserialize`-only copy but that module depends on this one (a cycle
/// blocks importing it upward), so the chunk merger keeps its own `Serialize +
/// Deserialize` structs. No `deny_unknown_fields`: real chunk JSON carries extra
/// fields we ignore, and the merged file we write is the exact subset the reader wants.
#[derive(serde::Serialize, serde::Deserialize)]
struct ChunkOffsets {
    from: u64,
    to: u64,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ChunkSegment {
    #[serde(default)]
    text: String,
    offsets: ChunkOffsets,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct ChunkJson {
    #[serde(default)]
    transcription: Vec<ChunkSegment>,
}

/// A fixed ASCII output base for whisper's `--output-file`. We deliberately do
/// NOT derive it from the audio file stem: whisper-cli reads argv through the
/// Windows ANSI code page, so a non-ASCII stem (e.g. a Japanese recording name)
/// would be mangled into a "?"-filled path that whisper then fails to write.
fn transcript_output_base() -> PathBuf {
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);

    env::temp_dir().join(format!(
        "wonder-of-u-transcript-{}-{unique_suffix}",
        std::process::id()
    ))
}

/// Deletes every path it holds when dropped, so temp files (a staged ASCII copy of a
/// non-ASCII-named recording, or the per-chunk WAVs and outputs of a chunked run) are
/// cleaned up on every return path — success, error, or unwind — without repeating the
/// removal at each `return`.
struct TempCleanup {
    paths: Vec<PathBuf>,
}

impl TempCleanup {
    fn new() -> Self {
        Self { paths: Vec::new() }
    }

    fn track(&mut self, path: PathBuf) {
        self.paths.push(path);
    }
}

impl Drop for TempCleanup {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = fs::remove_file(path);
        }
    }
}

/// Caps a stderr/stdout dump so a whisper usage/help splurge never surfaces as a
/// giant user-facing error: first 3 lines, then hard-limited to ~400 chars.
fn cap_details(details: String) -> String {
    const MAX_CHARS: usize = 400;
    let by_lines = details.lines().take(3).collect::<Vec<_>>().join("\n");
    if by_lines.chars().count() > MAX_CHARS {
        let capped: String = by_lines.chars().take(MAX_CHARS).collect();
        format!("{capped}…")
    } else {
        by_lines
    }
}

fn hide_command_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

pub fn verify_whisper_cli(cli_path: &Path) -> Result<(), String> {
    let mut command = Command::new(cli_path);
    hide_command_window(&mut command);

    let output = command
        .arg("-h")
        .output()
        .map_err(|error| error.to_string())?;

    if output.status.success() {
        return Ok(());
    }

    Err(String::from_utf8_lossy(&output.stderr).trim().to_string())
}

pub fn verify_whisper_model(model_path: &Path) -> Result<(), String> {
    let metadata = fs::metadata(model_path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("The selected Whisper model path is not a file.".into());
    }

    if metadata.len() < 1_000_000 {
        return Err("The selected Whisper model file is unexpectedly small.".into());
    }

    Ok(())
}

/// Parse a whisper-cli progress line — `whisper_print_progress_callback: progress = N%`
/// (variable spacing) — into a clamped 0–100 percent. Non-progress lines return `None`.
fn parse_whisper_progress_line(line: &str) -> Option<u8> {
    let rest = line.split("progress =").nth(1)?;
    let digits: String = rest.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
    let value: u16 = digits.parse().ok()?;
    Some(value.min(100) as u8)
}

/// Renders milliseconds as ffmpeg's `S.mmm` seconds form for `-ss`/`-to`.
fn format_ms_timestamp(ms: u64) -> String {
    format!("{}.{:03}", ms / 1000, ms % 1000)
}

/// Extracts `[start_ms, end_ms)` of `input` to a 16 kHz mono WAV (whisper's native
/// input). Uses `-ss <start> -t <duration>` before `-i` — an accurate seek plus an
/// unambiguous duration (the validated form; `-to` before `-i` is version-dependent).
/// ffmpeg handles non-ASCII input paths on Windows (unlike whisper-cli), and the output
/// name is ASCII, so the chunk feeds whisper cleanly. On any failure the caller falls
/// back to single-shot.
fn extract_audio_chunk(
    ffmpeg_path: &Path,
    input: &Path,
    start_ms: u64,
    end_ms: u64,
    output: &Path,
) -> Result<(), String> {
    let mut command = Command::new(ffmpeg_path);
    hide_command_window(&mut command);
    if let Some(parent) = ffmpeg_path.parent() {
        if !parent.as_os_str().is_empty() {
            command.current_dir(parent);
        }
    }
    command.args([
        "-y",
        "-nostdin",
        "-hide_banner",
        "-loglevel",
        "error",
        "-ss",
        &format_ms_timestamp(start_ms),
        "-t",
        &format_ms_timestamp(end_ms.saturating_sub(start_ms)),
    ]);
    command.arg("-i").arg(input);
    command.args([
        "-map", "0:a:0", "-vn", "-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le",
    ]);
    command.arg(output);

    let result = command
        .output()
        .map_err(|error| format!("Could not run ffmpeg to extract a chunk: {error}"))?;
    if !result.status.success() {
        return Err(format!(
            "ffmpeg failed to extract a chunk: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    if !output.exists() {
        return Err("ffmpeg did not produce the chunk file.".into());
    }
    Ok(())
}

/// Reads a chunk's whisper `.json` best-effort. A missing/garbage file yields `None`
/// (that chunk simply contributes no segments) — never fatal.
fn read_chunk_segments(json_path: &Path) -> Option<Vec<ChunkSegment>> {
    let raw = fs::read_to_string(json_path).ok()?;
    let parsed: ChunkJson = serde_json::from_str(&raw).ok()?;
    Some(parsed.transcription)
}

/// Transcribes long audio: it splits into overlapping ~5-minute chunks (drift can't
/// accumulate within a short chunk), transcribes each, offsets the timestamps by the
/// exact chunk start, keeps only each chunk's interior, and merges into one `.txt` +
/// `.json` — identical in shape to a single whisper run. Short audio (or a missing
/// ffmpeg / unknown duration) transcribes in one shot. A failed chunked run falls back
/// to a single shot on the whole file.
///
/// `on_progress` is invoked with a 0–100 percent scaled across the whole file (from a
/// drain thread, so the closure must be `Send + Sync + 'static` — callers pass a cloned
/// `AppHandle` and emit an event). It is a no-op-friendly hook; transcription never
/// fails over it.
pub fn run_whisper_transcription(
    request: &WhisperTranscriptionRequest,
    on_progress: impl Fn(u8) + Send + Sync + 'static,
) -> Result<WhisperTranscriptionResult, String> {
    verify_whisper_cli(&request.cli_path)?;
    verify_whisper_model(&request.model_path)?;

    let on_progress = Arc::new(on_progress);

    // Chunk only long audio, and only when ffmpeg is available to cut it. Short files
    // and the unknown-duration case take the single-shot path (short audio doesn't drift).
    if request.ffmpeg_path.is_some() && request.duration_ms > CHUNK_LEN_MS {
        let ffmpeg_path = request.ffmpeg_path.clone().unwrap();
        match run_chunked_transcription(request, &ffmpeg_path, Arc::clone(&on_progress)) {
            Ok(result) => return Ok(result),
            Err(_chunk_error) => {
                // A chunked run failed (ffmpeg vanished mid-run, a chunk errored). Fall
                // back to a single pass on the whole file — a drift-prone transcript
                // beats none. If that also fails, its error is returned below.
            }
        }
    }

    let progress = Arc::clone(&on_progress);
    run_whisper_once(
        &request.cli_path,
        &request.model_path,
        &request.audio_path,
        &request.language,
        &transcript_output_base(),
        move |percent| (*progress)(percent),
    )
}

/// Plans the chunk windows for a duration: one entry per ~5-min window as
/// `(nominal_start, nominal_end, clip_start, clip_end)`. The nominal `[start, end)`
/// partition the audio (no gaps, no overlap → no duplicate segments); the clip extends
/// `OVERLAP_MS` past each side (clamped to `[0, duration]`) so the kept interior is
/// never at a clip edge. Pure, so the planning is unit-testable without whisper/ffmpeg.
fn plan_chunk_windows(duration_ms: u64) -> Vec<(u64, u64, u64, u64)> {
    let chunk_count = (duration_ms + CHUNK_LEN_MS - 1) / CHUNK_LEN_MS;
    (0..chunk_count)
        .map(|index| {
            let nominal_start = index * CHUNK_LEN_MS;
            let nominal_end = ((index + 1) * CHUNK_LEN_MS).min(duration_ms);
            let clip_start = nominal_start.saturating_sub(OVERLAP_MS);
            let clip_end = (nominal_end + OVERLAP_MS).min(duration_ms);
            (nominal_start, nominal_end, clip_start, clip_end)
        })
        .collect()
}

/// The chunked planner: extract → transcribe → offset → interior-keep → merge.
fn run_chunked_transcription<F: Fn(u8) + Send + Sync + 'static>(
    request: &WhisperTranscriptionRequest,
    ffmpeg_path: &Path,
    on_progress: Arc<F>,
) -> Result<WhisperTranscriptionResult, String> {
    let windows = plan_chunk_windows(request.duration_ms);
    let chunk_count = windows.len() as u64;

    let output_base = transcript_output_base();
    let transcript_path = PathBuf::from(format!("{}.txt", output_base.display()));
    let json_path = PathBuf::from(format!("{}.json", output_base.display()));

    let mut temps = TempCleanup::new();
    let mut kept: Vec<ChunkSegment> = Vec::new();

    for (index, (nominal_start, nominal_end, clip_start, clip_end)) in
        windows.into_iter().enumerate()
    {
        let index = index as u64;

        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        let chunk_wav = env::temp_dir().join(format!(
            "wonder-of-u-chunk-{}-{unix_ms}-{index}.wav",
            std::process::id()
        ));
        temps.track(chunk_wav.clone());
        extract_audio_chunk(
            ffmpeg_path,
            &request.audio_path,
            clip_start,
            clip_end,
            &chunk_wav,
        )?;

        let chunk_base = env::temp_dir().join(format!(
            "wonder-of-u-chunk-out-{}-{unix_ms}-{index}",
            std::process::id()
        ));
        temps.track(PathBuf::from(format!("{}.txt", chunk_base.display())));
        temps.track(PathBuf::from(format!("{}.json", chunk_base.display())));

        let progress = Arc::clone(&on_progress);
        let chunk_result = run_whisper_once(
            &request.cli_path,
            &request.model_path,
            &chunk_wav,
            &request.language,
            &chunk_base,
            move |percent| {
                let overall = ((index * 100 + percent as u64) / chunk_count).min(100) as u8;
                (*progress)(overall);
            },
        )?;

        if let Some(segments) = read_chunk_segments(&chunk_result.json_path) {
            for mut segment in segments {
                let absolute_from = segment.offsets.from + clip_start;
                // Keep only the chunk's INTERIOR [nominal_start, nominal_end): the overlap
                // regions (where whisper hallucinates at the clip edges) are discarded, and
                // partitioning by start means no segment is duplicated across chunks.
                if absolute_from >= nominal_start && absolute_from < nominal_end {
                    segment.offsets.from = absolute_from;
                    segment.offsets.to = segment.offsets.to + clip_start;
                    kept.push(segment);
                }
            }
        }
    }

    let merged = ChunkJson { transcription: kept };
    let serialized = serde_json::to_string(&merged).map_err(|error| error.to_string())?;
    fs::write(&json_path, serialized).map_err(|error| error.to_string())?;
    let text = merged
        .transcription
        .iter()
        .map(|segment| segment.text.trim())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&transcript_path, format!("{text}\n")).map_err(|error| error.to_string())?;

    // The last chunk's final tick already reaches 100, but emit once more so the bar
    // always completes even if a chunk produced no progress lines.
    (*on_progress)(100);

    Ok(WhisperTranscriptionResult {
        transcript_path,
        json_path,
    })
}

/// One whisper-cli pass over a single audio file to `output_base.{txt,json}`. Assumes
/// the caller already verified the cli + model. Non-ASCII audio paths are staged to an
/// ASCII temp copy (chunk WAVs are already ASCII, so it's a no-op there).
fn run_whisper_once(
    cli_path: &Path,
    model_path: &Path,
    audio_path: &Path,
    language: &str,
    output_base: &Path,
    on_progress: impl Fn(u8) + Send + 'static,
) -> Result<WhisperTranscriptionResult, String> {
    let transcript_path = PathBuf::from(format!("{}.txt", output_base.display()));
    let json_path = PathBuf::from(format!("{}.json", output_base.display()));

    // whisper-cli receives argv via the Windows ANSI code page, so a non-ASCII audio
    // path arrives as "?????.wav" and the file is "not found". When the path is not pure
    // ASCII, stage an ASCII-named temp copy and hand whisper that. ASCII paths pass
    // through untouched to avoid copying large recordings needlessly.
    let mut temp = TempCleanup::new();
    let audio_arg = if audio_path
        .to_str()
        .map(|value| value.is_ascii())
        .unwrap_or(false)
    {
        audio_path.to_path_buf()
    } else {
        let ext = audio_path
            .extension()
            .and_then(|value| value.to_str())
            .unwrap_or("wav");
        let unix_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_millis())
            .unwrap_or(0);
        let temp_path = env::temp_dir().join(format!(
            "wonder-of-u-input-{}-{unix_ms}.{ext}",
            std::process::id()
        ));
        fs::copy(audio_path, &temp_path)
            .map_err(|error| format!("Could not stage the recording for transcription: {error}"))?;
        temp.track(temp_path.clone());
        temp_path
    };

    let mut command = Command::new(cli_path);
    hide_command_window(&mut command);
    command
        .arg("--model")
        .arg(model_path)
        .arg("--file")
        .arg(&audio_arg)
        .arg("--output-txt")
        .arg("--output-json")
        .arg("--output-file")
        .arg(output_base)
        .arg("--no-prints")
        // Emits `progress = N%` to stderr; it still prints under `--no-prints`, so we
        // stream it for the UI progress bar. The transcript is read from the output
        // files, so this stderr noise never touches the result.
        .arg("--print-progress");

    // Use most cores but leave a couple free so the machine stays responsive during a
    // long transcription. whisper-cli defaults to 4 threads, which idles bigger CPUs.
    let threads = std::thread::available_parallelism()
        .map(|cores| cores.get().saturating_sub(2).max(1))
        .unwrap_or(4);
    command.arg("-t").arg(threads.to_string());

    // Stop whisper's runaway repetition on non-vocal audio (a single line looping to
    // the end of a song's instrumental outro). `-mc 0` drops the cross-window text
    // context that feeds the loop; `--suppress-nst` suppresses non-speech tokens.
    command.arg("-mc").arg("0").arg("--suppress-nst");

    if !language.trim().is_empty() {
        command.arg("--language").arg(language.trim());
    }

    // Stream instead of `.output()`: whisper writes the transcript to files, so the only
    // reason to read its pipes live is the progress bar. Each pipe is drained on its own
    // thread — stderr also parses `progress = N%` → `on_progress` — into a bounded buffer
    // the error branches read after the child exits. Mirrors the yt-dlp downloader.
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| error.to_string())?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| "whisper-cli produced no stdout stream.".to_string())?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| "whisper-cli produced no stderr stream.".to_string())?;

    let stderr_buffer: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let stderr_sink = Arc::clone(&stderr_buffer);
    let stderr_thread = thread::spawn(move || {
        for line in BufReader::new(stderr).lines().map_while(Result::ok) {
            if let Some(percent) = parse_whisper_progress_line(&line) {
                on_progress(percent);
            }
            if let Ok(mut sink) = stderr_sink.lock() {
                if sink.len() < 8192 {
                    sink.push_str(&line);
                    sink.push('\n');
                }
            }
        }
    });

    let stdout_buffer: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let stdout_sink = Arc::clone(&stdout_buffer);
    let stdout_thread = thread::spawn(move || {
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Ok(mut sink) = stdout_sink.lock() {
                if sink.len() < 8192 {
                    sink.push_str(&line);
                    sink.push('\n');
                }
            }
        }
    });

    let status = child.wait().map_err(|error| error.to_string())?;
    let _ = stderr_thread.join();
    let _ = stdout_thread.join();

    let stderr_text = stderr_buffer
        .lock()
        .map(|guard| guard.trim().to_string())
        .unwrap_or_default();
    let stdout_text = stdout_buffer
        .lock()
        .map(|guard| guard.trim().to_string())
        .unwrap_or_default();

    if !status.success() {
        let details = cap_details(if !stderr_text.is_empty() {
            stderr_text
        } else {
            stdout_text
        });
        return Err(if details.is_empty() {
            "whisper-cli failed to transcribe the recording.".into()
        } else {
            details
        });
    }

    if !transcript_path.exists() {
        let details = cap_details(
            [stderr_text, stdout_text]
                .into_iter()
                .filter(|value| !value.is_empty())
                .collect::<Vec<_>>()
                .join("\n"),
        );
        return Err(if details.is_empty() {
            format!(
                "whisper-cli finished without writing the transcript file at {}.",
                transcript_path.display()
            )
        } else {
            format!(
                "whisper-cli finished without writing the transcript file at {}. {}",
                transcript_path.display(),
                details
            )
        });
    }

    Ok(WhisperTranscriptionResult {
        transcript_path,
        json_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn transcript_output_base_is_ascii_without_stem() {
        let base = transcript_output_base();
        let text = base
            .to_str()
            .expect("temp output base should be valid unicode");
        assert!(text.is_ascii(), "temp output base must be pure ASCII: {text}");
        assert!(
            text.contains("wonder-of-u-transcript-"),
            "temp output base should use the fixed ASCII prefix, not a file stem: {text}"
        );
    }

    #[test]
    fn parse_whisper_progress_line_reads_and_clamps_percent() {
        assert_eq!(
            parse_whisper_progress_line("whisper_print_progress_callback: progress =  96%"),
            Some(96)
        );
        assert_eq!(
            parse_whisper_progress_line("whisper_print_progress_callback: progress = 100%"),
            Some(100)
        );
        assert_eq!(parse_whisper_progress_line("progress =   7%"), Some(7));
        // A nonsensical over-100 value is clamped rather than overflowing a u8.
        assert_eq!(parse_whisper_progress_line("progress = 250%"), Some(100));
        // Non-progress lines are ignored.
        assert_eq!(parse_whisper_progress_line("whisper_full_with_state: decode"), None);
        assert_eq!(parse_whisper_progress_line(""), None);
    }

    #[test]
    fn format_ms_timestamp_renders_seconds_and_millis() {
        assert_eq!(format_ms_timestamp(0), "0.000");
        assert_eq!(format_ms_timestamp(900_000), "900.000");
        assert_eq!(format_ms_timestamp(905_250), "905.250");
        assert_eq!(format_ms_timestamp(1_419_970), "1419.970");
    }

    #[test]
    fn plan_chunk_windows_partitions_with_clamped_overlap() {
        let windows = plan_chunk_windows(1_419_970); // 23m40s
        assert_eq!(windows.len(), 5);
        // First window: overlap before 0 is clamped away.
        assert_eq!(windows[0], (0, 300_000, 0, 320_000));
        // Interior window: 20s overlap on both sides.
        assert_eq!(windows[2], (600_000, 900_000, 580_000, 920_000));
        // Last window: nominal end and clip end both clamp to the real duration.
        assert_eq!(windows[4], (1_200_000, 1_419_970, 1_180_000, 1_419_970));
        // Nominal windows partition the timeline with no gaps or overlap.
        for pair in windows.windows(2) {
            assert_eq!(pair[0].1, pair[1].0, "each nominal end must meet the next start");
        }
    }

    #[test]
    fn plan_chunk_windows_count_is_ceil_of_duration() {
        assert_eq!(plan_chunk_windows(300_000).len(), 1);
        assert_eq!(plan_chunk_windows(300_001).len(), 2);
        assert_eq!(plan_chunk_windows(600_000).len(), 2);
        assert_eq!(plan_chunk_windows(600_001).len(), 3);
    }

    #[test]
    fn interior_keep_drops_overlap_regions_after_absolute_offset() {
        // Window 2's clip runs 580s..920s. A segment is kept only if its ABSOLUTE start
        // (chunk-relative + clip_start) lands in the nominal interior [600s, 900s).
        let clip_start = 580_000u64;
        let (nominal_start, nominal_end) = (600_000u64, 900_000u64);
        let kept = |relative_from: u64| {
            let absolute = relative_from + clip_start;
            absolute >= nominal_start && absolute < nominal_end
        };
        assert!(kept(25_000), "605s is interior → kept");
        assert!(!kept(5_000), "585s is leading overlap → dropped");
        assert!(!kept(345_000), "925s is trailing overlap → dropped");
        assert_eq!(25_000u64 + clip_start, 605_000, "offset is exact addition");
    }

    #[test]
    fn cap_details_limits_a_giant_dump() {
        let dump = "x".repeat(5000);
        let capped = cap_details(dump);
        assert!(
            capped.chars().count() <= 401,
            "capped details should stay bounded, got {} chars",
            capped.chars().count()
        );
    }
}
