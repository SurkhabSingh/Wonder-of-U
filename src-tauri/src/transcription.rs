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

#[derive(Debug, Clone)]
pub struct WhisperTranscriptionRequest {
    pub cli_path: PathBuf,
    pub model_path: PathBuf,
    pub audio_path: PathBuf,
    pub language: String,
    /// The managed Silero VAD model, when installed. Present → transcribe with
    /// `--vad`, which re-anchors segment timestamps to the original timeline (fixing
    /// the long-audio drift) and skips silence (faster). Absent → plain transcription.
    pub vad_model_path: Option<PathBuf>,
}

#[derive(Debug, Clone)]
pub struct WhisperTranscriptionResult {
    pub transcript_path: PathBuf,
    /// Expected path of whisper's `--output-json` sidecar carrying per-segment
    /// offsets. It may not exist if whisper skipped writing it; callers parse it
    /// best-effort and never fail transcription over a missing json.
    pub json_path: PathBuf,
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

/// Deletes the file it holds when dropped, so a staged ASCII copy of a
/// non-ASCII-named recording is cleaned up on every return path — success or
/// error — without repeating the removal at each `return`.
struct TempInputGuard {
    path: Option<PathBuf>,
}

impl Drop for TempInputGuard {
    fn drop(&mut self) {
        if let Some(path) = &self.path {
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

/// `on_progress` is invoked with a 0–100 percent as whisper reports it (from a drain
/// thread, so the closure must be `Send + 'static` — callers pass a cloned `AppHandle`
/// and emit an event). It is a no-op-friendly hook; transcription never fails over it.
pub fn run_whisper_transcription(
    request: &WhisperTranscriptionRequest,
    on_progress: impl Fn(u8) + Send + 'static,
) -> Result<WhisperTranscriptionResult, String> {
    verify_whisper_cli(&request.cli_path)?;
    verify_whisper_model(&request.model_path)?;

    let output_base = transcript_output_base();
    let transcript_path = PathBuf::from(format!("{}.txt", output_base.display()));
    let json_path = PathBuf::from(format!("{}.json", output_base.display()));

    // whisper-cli receives argv via the Windows ANSI code page, so a non-ASCII
    // audio path arrives as "?????.wav" and the file is "not found". When the
    // path is not pure ASCII, stage an ASCII-named temp copy and hand whisper
    // that instead. ASCII paths are passed through untouched to avoid copying
    // large recordings needlessly. The guard removes any staged copy on drop.
    let mut temp_input = TempInputGuard { path: None };
    let audio_arg = if request
        .audio_path
        .to_str()
        .map(|value| value.is_ascii())
        .unwrap_or(false)
    {
        request.audio_path.clone()
    } else {
        let ext = request
            .audio_path
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
        fs::copy(&request.audio_path, &temp_path)
            .map_err(|error| format!("Could not stage the recording for transcription: {error}"))?;
        temp_input.path = Some(temp_path.clone());
        temp_path
    };

    let mut command = Command::new(&request.cli_path);
    hide_command_window(&mut command);
    command
        .arg("--model")
        .arg(&request.model_path)
        .arg("--file")
        .arg(&audio_arg)
        .arg("--output-txt")
        .arg("--output-json")
        .arg("--output-file")
        .arg(&output_base)
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
    // Proven on long music tracks, and unlike VAD it never discards actual speech.
    command.arg("-mc").arg("0").arg("--suppress-nst");

    // VAD re-anchors segment timestamps to the original audio timeline (removing the
    // long-audio drift) and skips silence — only when the managed VAD model exists.
    // Short max-speech + padding keep segment boundaries tight without clipping onsets.
    if let Some(vad_model) = request.vad_model_path.as_ref() {
        command
            .arg("--vad")
            .arg("--vad-model")
            .arg(vad_model)
            .arg("--vad-min-silence-duration-ms")
            .arg("100")
            .arg("--vad-speech-pad-ms")
            .arg("60")
            .arg("--vad-max-speech-duration-s")
            .arg("15");
    }

    if !request.language.trim().is_empty() {
        command.arg("--language").arg(request.language.trim());
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
