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

/// Cap a VAD speech region so whisper always gets a clip inside its 30 s window; longer
/// continuous speech is auto-split by the VAD. Passed to `--vad-max-speech-duration-s`.
const VAD_MAX_SPEECH_SECONDS: &str = "20";

#[derive(Debug, Clone)]
pub struct WhisperTranscriptionRequest {
    pub cli_path: PathBuf,
    pub model_path: PathBuf,
    /// whisper.cpp's built-in Silero VAD ggml model. VAD segments the audio into speech
    /// regions — drift-free absolute timestamps, and non-speech (music/silence) yields no
    /// text — superseding the old overlapping-chunk approach.
    pub vad_model_path: PathBuf,
    pub audio_path: PathBuf,
    pub language: String,
    /// ffmpeg, used to decode the recording to the 16 kHz mono WAV whisper + VAD require.
    pub ffmpeg_path: PathBuf,
    /// Total audio duration in ms; passed through for the downstream segment tail-clamp.
    pub duration_ms: u64,
}

#[derive(Debug, Clone)]
pub struct WhisperTranscriptionResult {
    pub transcript_path: PathBuf,
    /// Expected path of whisper's `--output-json` sidecar carrying per-segment offsets. It
    /// may not exist if whisper skipped writing it; callers parse it best-effort and never
    /// fail transcription over a missing json.
    pub json_path: PathBuf,
}

/// A fixed ASCII output base for whisper's `--output-file`. We deliberately do NOT derive it
/// from the audio file stem: whisper-cli reads argv through the Windows ANSI code page, so a
/// non-ASCII stem (e.g. a Japanese recording name) would be mangled into a "?"-filled path
/// that whisper then fails to write.
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
/// non-ASCII-named recording, or the decoded 16 kHz WAV) are cleaned up on every return path
/// — success, error, or unwind — without repeating the removal at each `return`.
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

/// Caps a stderr/stdout dump so a whisper usage/help splurge never surfaces as a giant
/// user-facing error: first 3 lines, then hard-limited to ~400 chars.
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

pub fn verify_whisper_vad_model(vad_model_path: &Path) -> Result<(), String> {
    let metadata = fs::metadata(vad_model_path).map_err(|error| error.to_string())?;
    if !metadata.is_file() {
        return Err("The selected VAD model path is not a file.".into());
    }
    if metadata.len() < 100_000 {
        return Err("The selected VAD model file is unexpectedly small.".into());
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

/// Decodes any recording to a 16 kHz mono s16le WAV at an ASCII temp path — the format
/// whisper.cpp and its Silero VAD want. ffmpeg handles non-ASCII *input* paths on Windows,
/// and the ASCII output name feeds whisper-cli cleanly.
fn decode_to_wav_16k(ffmpeg_path: &Path, input: &Path) -> Result<PathBuf, String> {
    let unix_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);
    let output = env::temp_dir().join(format!(
        "wonder-of-u-input-{}-{unix_ms}.wav",
        std::process::id()
    ));

    let mut command = Command::new(ffmpeg_path);
    hide_command_window(&mut command);
    if let Some(parent) = ffmpeg_path.parent() {
        if !parent.as_os_str().is_empty() {
            command.current_dir(parent);
        }
    }
    command.args(["-y", "-nostdin", "-hide_banner", "-loglevel", "error"]);
    command.arg("-i").arg(input);
    command.args([
        "-map", "0:a:0", "-vn", "-ar", "16000", "-ac", "1", "-c:a", "pcm_s16le",
    ]);
    command.arg(&output);

    let result = command
        .output()
        .map_err(|error| format!("Could not run ffmpeg to decode the recording: {error}"))?;
    if !result.status.success() {
        return Err(format!(
            "ffmpeg failed to decode the recording: {}",
            String::from_utf8_lossy(&result.stderr).trim()
        ));
    }
    if !output.exists() {
        return Err("ffmpeg did not produce the decoded audio.".into());
    }
    Ok(output)
}

/// Transcribes a recording with whisper.cpp using its built-in Silero VAD: the audio is
/// decoded to 16 kHz mono, then a single whisper-cli pass detects speech regions, transcribes
/// only those, and maps each region's timestamps back onto the absolute timeline. Drift-free
/// on arbitrarily long audio, non-speech excluded, no manual chunking. Output is the same
/// `{txt, json}` a plain whisper run produces.
///
/// `on_progress` is invoked with a 0–100 percent (from a drain thread, so the closure must be
/// `Send + 'static`; callers pass a cloned `AppHandle` and emit an event). It is a
/// no-op-friendly hook; transcription never fails over it.
pub fn run_whisper_transcription(
    request: &WhisperTranscriptionRequest,
    on_progress: impl Fn(u8) + Send + 'static,
) -> Result<WhisperTranscriptionResult, String> {
    verify_whisper_cli(&request.cli_path)?;
    verify_whisper_model(&request.model_path)?;
    verify_whisper_vad_model(&request.vad_model_path)?;

    // Decode to the 16 kHz mono WAV whisper + Silero VAD want. The decoded WAV is a tracked
    // temp file cleaned up on every return path.
    let mut temps = TempCleanup::new();
    let wav_path = decode_to_wav_16k(&request.ffmpeg_path, &request.audio_path)?;
    temps.track(wav_path.clone());

    run_whisper_once(
        &request.cli_path,
        &request.model_path,
        &request.vad_model_path,
        &wav_path,
        &request.language,
        &transcript_output_base(),
        on_progress,
    )
}

/// One whisper-cli `--vad` pass over a single (already 16 kHz mono) WAV to
/// `output_base.{txt,json}`. Assumes the caller already verified cli + model + vad model.
fn run_whisper_once(
    cli_path: &Path,
    model_path: &Path,
    vad_model_path: &Path,
    audio_path: &Path,
    language: &str,
    output_base: &Path,
    on_progress: impl Fn(u8) + Send + 'static,
) -> Result<WhisperTranscriptionResult, String> {
    let transcript_path = PathBuf::from(format!("{}.txt", output_base.display()));
    let json_path = PathBuf::from(format!("{}.json", output_base.display()));

    // whisper-cli receives argv via the Windows ANSI code page, so a non-ASCII audio path
    // arrives as "?????.wav" and the file is "not found". When the path is not pure ASCII,
    // stage an ASCII-named temp copy and hand whisper that. Our decoded WAV is already ASCII,
    // so this is normally a no-op.
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
        // Emits `progress = N%` to stderr; it still prints under `--no-prints`, so we stream
        // it for the UI progress bar. The transcript is read from the output files, so this
        // stderr noise never touches the result.
        .arg("--print-progress");

    // Use most cores but leave a couple free so the machine stays responsive during a long
    // transcription. whisper-cli defaults to 4 threads, which idles bigger CPUs.
    let threads = std::thread::available_parallelism()
        .map(|cores| cores.get().saturating_sub(2).max(1))
        .unwrap_or(4);
    command.arg("-t").arg(threads.to_string());

    // Stop whisper's runaway repetition on non-vocal audio. `-mc 0` drops the cross-window
    // text context that feeds the loop; `--suppress-nst` suppresses non-speech tokens.
    command.arg("-mc").arg("0").arg("--suppress-nst");

    // whisper.cpp's built-in Silero VAD: only detected speech regions are transcribed and
    // their timestamps mapped back to the absolute timeline — drift-free on long audio, and
    // non-speech yields no segments. This supersedes the old overlapping-chunk machinery.
    command
        .arg("--vad")
        .arg("--vad-model")
        .arg(vad_model_path)
        .arg("--vad-max-speech-duration-s")
        .arg(VAD_MAX_SPEECH_SECONDS);

    if !language.trim().is_empty() {
        command.arg("--language").arg(language.trim());
    }

    // Stream instead of `.output()`: whisper writes the transcript to files, so the only
    // reason to read its pipes live is the progress bar. Each pipe is drained on its own
    // thread — stderr also parses `progress = N%` → `on_progress` — into a bounded buffer the
    // error branches read after the child exits. Mirrors the yt-dlp downloader.
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
    fn cap_details_limits_a_giant_dump() {
        let dump = "x".repeat(5000);
        let capped = cap_details(dump);
        assert!(
            capped.chars().count() <= 401,
            "capped details should stay bounded, got {} chars",
            capped.chars().count()
        );
    }

    /// Manual end-to-end check of the real engine (ffmpeg decode -> whisper-cli --vad ->
    /// segment JSON). Ignored by default. Run with:
    ///   WOU_CLI=".../whisper-cli.exe" WOU_MODEL=".../ggml-large-v3.bin" \
    ///   WOU_VAD=".../ggml-silero-v6.2.0.bin" WOU_FFMPEG=".../ffmpeg.exe" \
    ///   WOU_AUDIO=".../clip.mp3" WOU_LANG=ja \
    ///   cargo test --release end_to_end_vad -- --ignored --nocapture
    #[test]
    #[ignore = "manual: needs local cli + models + ffmpeg + audio"]
    fn end_to_end_vad() {
        let request = WhisperTranscriptionRequest {
            cli_path: PathBuf::from(std::env::var("WOU_CLI").expect("WOU_CLI")),
            model_path: PathBuf::from(std::env::var("WOU_MODEL").expect("WOU_MODEL")),
            vad_model_path: PathBuf::from(std::env::var("WOU_VAD").expect("WOU_VAD")),
            audio_path: PathBuf::from(std::env::var("WOU_AUDIO").expect("WOU_AUDIO")),
            ffmpeg_path: PathBuf::from(std::env::var("WOU_FFMPEG").expect("WOU_FFMPEG")),
            language: std::env::var("WOU_LANG").unwrap_or_default(),
            duration_ms: 0,
        };
        let result = run_whisper_transcription(&request, |percent| {
            if percent % 25 == 0 {
                eprintln!("progress {percent}%");
            }
        })
        .expect("transcription should succeed");
        let json = fs::read_to_string(&result.json_path).expect("json written");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        let segments = parsed["transcription"].as_array().expect("segments array");
        eprintln!("RESULT segments={}", segments.len());
        for seg in segments.iter().take(5) {
            eprintln!("  [{}->{}] {}", seg["offsets"]["from"], seg["offsets"]["to"], seg["text"]);
        }
        assert!(!segments.is_empty());
        let _ = fs::remove_file(&result.json_path);
        let _ = fs::remove_file(&result.transcript_path);
    }
}
