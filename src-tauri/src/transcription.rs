use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use crate::{child_io::drain_lines, media_errors::stderr_indicates_no_audio};

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

const VAD_MAX_SPEECH_SECONDS: &str = "20";

pub const TRANSCRIPTION_CANCELLED: &str = "transcription cancelled.";

/// The one whisper-cli slot, and the single fact that decides whether a run may start.
static WHISPER_SLOT_BUSY: AtomicBool = AtomicBool::new(false);

pub struct WhisperSlotGuard;

impl WhisperSlotGuard {
    pub fn acquire(busy_message: &str) -> Result<Self, String> {
        WHISPER_SLOT_BUSY
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .map(|_| Self)
            .map_err(|_| busy_message.to_string())
    }
}

impl Drop for WhisperSlotGuard {
    fn drop(&mut self) {
        WHISPER_SLOT_BUSY.store(false, Ordering::SeqCst);
    }
}

const CANCEL_POLL_INTERVAL: Duration = Duration::from_millis(100);

#[derive(Debug, Clone)]
pub struct WhisperTranscriptionRequest {
    pub cli_path: PathBuf,
    pub model_path: PathBuf,
    pub vad_model_path: PathBuf,
    pub audio_path: PathBuf,
    pub language: String,
    pub ffmpeg_path: PathBuf,
    pub thread_count: usize,
    pub music_mode: bool,
    pub fast_decode: bool,
}

#[derive(Debug, Clone)]
pub struct WhisperTranscriptionResult {
    pub transcript_path: PathBuf,
    pub json_path: PathBuf,
    pub speech_regions: Vec<SpeechRegion>,
    pub speech_envelope: Option<SpeechEnvelope>,
}

/// One bit per 10 ms of audio: was anyone speaking.
#[derive(Debug, Clone)]
pub struct SpeechEnvelope {
    voiced: Vec<bool>,
}

const FRAME_MS: usize = 10;

/// Where to put the speech/silence line between the noise floor and the speech level.
const SPEECH_FRACTION: f32 = 0.45;

impl SpeechEnvelope {
    pub fn from_wav_16k_mono(path: &Path) -> Option<Self> {
        let bytes = fs::read(path).ok()?;
        let mut offset = 12usize;
        let data = loop {
            if offset + 8 > bytes.len() {
                return None;
            }
            let id = &bytes[offset..offset + 4];
            let size = u32::from_le_bytes(bytes[offset + 4..offset + 8].try_into().ok()?) as usize;
            let body = offset + 8;
            if id == b"data" {
                break bytes.get(body..(body + size).min(bytes.len()))?;
            }
            offset = body + size + (size & 1);
        };

        let samples_per_frame = 16 * FRAME_MS; // 16 kHz -> 16 samples per ms
        let frame_bytes = samples_per_frame * 2;
        if data.len() < frame_bytes {
            return None;
        }

        let mut decibels: Vec<f32> = Vec::with_capacity(data.len() / frame_bytes);
        for frame in data.chunks_exact(frame_bytes) {
            let sum: f64 = frame
                .chunks_exact(2)
                .map(|pair| {
                    let sample = i16::from_le_bytes([pair[0], pair[1]]) as f64 / 32768.0;
                    sample * sample
                })
                .sum();
            let rms = (sum / samples_per_frame as f64).sqrt().max(1e-6);
            decibels.push(20.0 * rms.log10() as f32);
        }

        let noise = percentile(&decibels, 0.10);
        let speech = percentile(&decibels, 0.85);
        if speech - noise < 6.0 {
            return None;
        }
        let threshold = noise + (speech - noise) * SPEECH_FRACTION;

        Some(Self {
            voiced: decibels.into_iter().map(|value| value > threshold).collect(),
        })
    }

    #[cfg(test)]
    pub fn from_frames(voiced: Vec<bool>) -> Self {
        Self { voiced }
    }

    pub fn trim(&self, start_ms: u64, end_ms: u64, pad_ms: u64) -> (u64, u64) {
        if end_ms <= start_ms {
            return (start_ms, end_ms);
        }
        let first = (start_ms as usize / FRAME_MS).min(self.voiced.len());
        let last = (end_ms as usize / FRAME_MS).min(self.voiced.len());
        let window = self.voiced.get(first..last).unwrap_or(&[]);

        let Some(first_voiced) = window.iter().position(|voiced| *voiced) else {
            return (start_ms, end_ms);
        };
        let last_voiced = window
            .iter()
            .rposition(|voiced| *voiced)
            .unwrap_or(first_voiced);

        let speech_start = start_ms + (first_voiced * FRAME_MS) as u64;
        let speech_end = start_ms + ((last_voiced + 1) * FRAME_MS) as u64;
        (
            start_ms.max(speech_start.saturating_sub(pad_ms)),
            end_ms.min(speech_end.saturating_add(pad_ms)),
        )
    }
}

/// Nearest-rank percentile over a copy, so the caller's order is left alone.
fn percentile(values: &[f32], fraction: f32) -> f32 {
    if values.is_empty() {
        return 0.0;
    }
    let mut sorted = values.to_vec();
    sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let index = ((sorted.len() - 1) as f32 * fraction).round() as usize;
    sorted[index.min(sorted.len() - 1)]
}

/// A fixed ASCII output base for whisper's `--output-file`. 
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

/// True for whisper-cli's routine chatter — the load banner, VAD/timing tables, and the
/// progress callback. Dropping it is what keeps a real `error:` line visible.
fn is_whisper_noise_line(line: &str) -> bool {
    const PREFIXES: [&str; 10] = [
        "load_backend:",
        "whisper_init_",
        "whisper_model_load:",
        "whisper_backend_init",
        "whisper_vad",
        "whisper_print_timings:",
        "whisper_print_progress_callback:",
        "system_info:",
        "output_txt:",
        "output_json:",
    ];
    let trimmed = line.trim_start();
    trimmed.is_empty() || PREFIXES.iter().any(|prefix| trimmed.starts_with(prefix))
}

/// Appends a line to a bounded diagnostic buffer, skipping routine chatter.
fn push_diagnostic_line(sink: &Arc<Mutex<String>>, line: &str) {
    if is_whisper_noise_line(line) {
        return;
    }
    if let Ok(mut sink) = sink.lock() {
        if sink.len() < 8192 {
            sink.push_str(line);
            sink.push('\n');
        }
    }
}

/// Caps a stderr/stdout dump so a whisper usage/help splurge never surfaces as a giant
/// user-facing error: the most explanatory 3 lines, then hard-limited to ~400 chars.
fn cap_details(details: String) -> String {
    const MAX_CHARS: usize = 400;
    fn is_error(line: &str) -> bool {
        let lowered = line.to_ascii_lowercase();
        lowered.starts_with("error") || lowered.contains("error:") || lowered.contains("failed")
    }
    let lines = details.lines().collect::<Vec<_>>();
    let mut ordered = lines
        .iter()
        .copied()
        .filter(|line| is_error(line))
        .collect::<Vec<_>>();
    ordered.extend(lines.iter().copied().filter(|line| !is_error(line)));
    ordered.truncate(3);
    let by_lines = ordered.join("\n");
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

/// Maps the user's CPU-usage preference to a whisper-cli `-t` worker-thread count so a long
/// transcription never has to max out the machine.
pub(crate) fn transcription_thread_count(cpu_usage: &str) -> usize {
    let cores = std::thread::available_parallelism()
        .map(|c| c.get())
        .unwrap_or(4);
    match cpu_usage {
        "high" => cores.saturating_sub(2).max(1),
        "low" => (cores / 4).max(1),
        _ => (cores / 2).max(1),
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

/// Parse a whisper-cli segment line — `[00:00:06.830 --> 00:00:13.490]  text` — into
/// absolute millisecond bounds and its text. Non-segment lines return `None`.
fn parse_whisper_segment_line(line: &str) -> Option<(u64, u64, String)> {
    let inner = line.trim().strip_prefix('[')?;
    let (span, text) = inner.split_once(']')?;
    let (start, end) = span.split_once("-->")?;
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some((
        parse_whisper_timestamp(start.trim())?,
        parse_whisper_timestamp(end.trim())?,
        text.to_string(),
    ))
}

/// `HH:MM:SS.mmm` → milliseconds.
fn parse_whisper_timestamp(value: &str) -> Option<u64> {
    let (clock, millis) = value.split_once('.')?;
    let mut parts = clock.split(':');
    let hours = parts.next()?.parse::<u64>().ok()?;
    let minutes = parts.next()?.parse::<u64>().ok()?;
    let seconds = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() || minutes > 59 || seconds > 59 {
        return None;
    }
    let millis = millis.parse::<u64>().ok()?;
    hours
        .checked_mul(60)?
        .checked_add(minutes)?
        .checked_mul(60)?
        .checked_add(seconds)?
        .checked_mul(1000)?
        .checked_add(millis)
}

/// One region Silero VAD judged to be speech, in absolute milliseconds against the source
/// audio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SpeechRegion {
    pub start_ms: u64,
    pub end_ms: u64,
}


#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpeechRegionSource {
    Mapped,
    Probe,
}

#[derive(Debug, Default)]
struct VadRegionLog {
    mapped: Vec<SpeechRegion>,
    probe: Vec<SpeechRegion>,
}

impl VadRegionLog {
    fn push(&mut self, source: SpeechRegionSource, region: SpeechRegion) {
        match source {
            SpeechRegionSource::Mapped => self.mapped.push(region),
            SpeechRegionSource::Probe => self.probe.push(region),
        }
    }

    fn into_regions(self) -> Vec<SpeechRegion> {
        if self.mapped.is_empty() {
            self.probe
        } else {
            self.mapped
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Rounding {
    Down,
    Up,
}

/// Seconds as whisper prints them (`1.92`) → milliseconds.
fn parse_vad_seconds(value: &str, round: Rounding) -> Option<u64> {
    let value = value.trim();
    let (whole, fraction) = value.split_once('.').unwrap_or((value, ""));
    if whole.is_empty()
        || !whole.bytes().all(|byte| byte.is_ascii_digit())
        || !fraction.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }

    let millis_digits = fraction.get(..3).unwrap_or(fraction);
    let millis = match millis_digits.len() {
        0 => 0,
        _ => millis_digits.parse::<u64>().ok()? * 10u64.pow(3 - millis_digits.len() as u32),
    };
    let total = whole
        .parse::<u64>()
        .ok()?
        .checked_mul(1000)?
        .checked_add(millis)?;

    let discarded_precision = fraction.len() > 3 && fraction[3..].bytes().any(|byte| byte != b'0');
    if round == Rounding::Up && discarded_precision {
        total.checked_add(1)
    } else {
        Some(total)
    }
}

fn value_after<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let rest = line.split_once(key)?.1.trim_start();
    let end = rest
        .find(|character: char| !(character.is_ascii_digit() || character == '.'))
        .unwrap_or(rest.len());
    (end > 0).then(|| &rest[..end])
}

/// Parse one of whisper's two VAD region lines into an absolute speech region.
fn parse_vad_region_line(line: &str) -> Option<(SpeechRegionSource, SpeechRegion)> {
    let (source, start, end) = if line.contains("vad_segment_info:") {
        (
            SpeechRegionSource::Mapped,
            value_after(line, "orig_start:")?,
            value_after(line, "orig_end:")?,
        )
    } else if line.contains("VAD segment ") {
        (
            SpeechRegionSource::Probe,
            value_after(line, "start = ")?,
            value_after(line, "end = ")?,
        )
    } else {
        return None;
    };

    let region = SpeechRegion {
        start_ms: parse_vad_seconds(start, Rounding::Down)?,
        end_ms: parse_vad_seconds(end, Rounding::Up)?,
    };
    // A backwards region would corrupt the forward walk the clamp does over this list.
    (region.end_ms >= region.start_ms).then_some((source, region))
}

/// Parse a whisper-cli progress line — `whisper_print_progress_callback: progress = N%`
/// (variable spacing) — into a clamped 0–100 percent. Non-progress lines return `None`.
fn parse_whisper_progress_line(line: &str) -> Option<u8> {
    let rest = line.split("progress =").nth(1)?;
    let digits: String = rest.trim().chars().take_while(|c| c.is_ascii_digit()).collect();
    let value: u16 = digits.parse().ok()?;
    Some(value.min(100) as u8)
}

/// Shown when the media handed to transcription carries no audio track.
const NO_AUDIO_MESSAGE: &str = "This video has no sound, so there is nothing to transcribe.";

/// What a failed decode reports. Kept pure so both branches can be asserted without
/// spawning ffmpeg.
fn decode_failure_message(stderr: &str) -> String {
    if stderr_indicates_no_audio(stderr) {
        NO_AUDIO_MESSAGE.to_string()
    } else {
        format!("ffmpeg failed to decode the recording: {}", stderr.trim())
    }
}

/// Decodes any recording to a 16 kHz mono s16le WAV at an ASCII temp path — the format
/// whisper.cpp and its Silero VAD want.
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
        let _ = fs::remove_file(&output);
        return Err(decode_failure_message(&String::from_utf8_lossy(
            &result.stderr,
        )));
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
/// `on_progress` is invoked with a 0–100 percent, and `on_segment` with each sentence as
/// whisper decodes it (`start_ms`, `end_ms`, text). Both run on a drain thread, so the
/// closures must be `Send + 'static`; callers pass a cloned `AppHandle` and emit an event.
/// Both are no-op-friendly hooks; transcription never fails over either, and the saved
/// transcript is read from the output files regardless of what they did.
pub fn run_whisper_transcription(
    request: &WhisperTranscriptionRequest,
    cancel: Arc<AtomicBool>,
    on_progress: impl Fn(u8) + Send + 'static,
    on_segment: impl Fn(u64, u64, String) + Send + 'static,
) -> Result<WhisperTranscriptionResult, String> {
    verify_whisper_cli(&request.cli_path)?;
    verify_whisper_model(&request.model_path)?;
    if !request.music_mode {
        verify_whisper_vad_model(&request.vad_model_path)?;
    }

    if cancel.load(Ordering::Relaxed) {
        return Err(TRANSCRIPTION_CANCELLED.into());
    }

    let mut temps = TempCleanup::new();
    let wav_path = decode_to_wav_16k(&request.ffmpeg_path, &request.audio_path)?;
    temps.track(wav_path.clone());

    let speech_envelope = SpeechEnvelope::from_wav_16k_mono(&wav_path);

    let mut result = run_whisper_once(
        &request.cli_path,
        &request.model_path,
        &request.vad_model_path,
        &wav_path,
        &request.language,
        &transcript_output_base(),
        request.thread_count,
        request.music_mode,
        request.fast_decode,
        cancel,
        on_progress,
        on_segment,
    )?;
    result.speech_envelope = speech_envelope;
    Ok(result)
}

/// One whisper-cli `--vad` pass over a single (already 16 kHz mono) WAV to
/// `output_base.{txt,json}`. Assumes the caller already verified cli + model + vad model.
#[allow(clippy::too_many_arguments)]
fn run_whisper_once(
    cli_path: &Path,
    model_path: &Path,
    vad_model_path: &Path,
    audio_path: &Path,
    language: &str,
    output_base: &Path,
    thread_count: usize,
    music_mode: bool,
    fast_decode: bool,
    cancel: Arc<AtomicBool>,
    on_progress: impl Fn(u8) + Send + 'static,
    on_segment: impl Fn(u64, u64, String) + Send + 'static,
) -> Result<WhisperTranscriptionResult, String> {
    let transcript_path = PathBuf::from(format!("{}.txt", output_base.display()));
    let json_path = PathBuf::from(format!("{}.json", output_base.display()));

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
        .arg("--print-progress");

    command.arg("-t").arg(thread_count.to_string());

    command.arg("-mc").arg("0").arg("--suppress-nst");

    if fast_decode {
        command.arg("-bs").arg("1");
    }

    if !music_mode {
        command
            .arg("--vad")
            .arg("--vad-model")
            .arg(vad_model_path)
            .arg("--vad-max-speech-duration-s")
            .arg(VAD_MAX_SPEECH_SECONDS);
    }

    if !language.trim().is_empty() {
        command.arg("--language").arg(language.trim());
    }

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
    let region_log: Arc<Mutex<VadRegionLog>> = Arc::new(Mutex::new(VadRegionLog::default()));
    let region_sink = Arc::clone(&region_log);
    let stderr_thread = thread::spawn(move || {
        for line in drain_lines(stderr) {
            if let Some(percent) = parse_whisper_progress_line(&line) {
                on_progress(percent);
            }
            if let Some((source, region)) = parse_vad_region_line(&line) {
                if let Ok(mut log) = region_sink.lock() {
                    log.push(source, region);
                }
            }
            push_diagnostic_line(&stderr_sink, &line);
        }
    });

    // The stdout drain owns the `done` sender and never sends on it: dropping the sender when
    // the drain hits EOF IS the signal that whisper-cli closed its pipes, so the wait below
    // returns `Disconnected` the moment it exits. Mirrors the yt-dlp downloader.
    let (done_sender, done_receiver) = mpsc::channel::<()>();
    let stdout_buffer: Arc<Mutex<String>> = Arc::new(Mutex::new(String::new()));
    let stdout_sink = Arc::clone(&stdout_buffer);
    let segment_cancel = Arc::clone(&cancel);
    let stdout_thread = thread::spawn(move || {
        let _done_sender = done_sender;
        for line in drain_lines(stdout) {
            if let Some((start_ms, end_ms, text)) = parse_whisper_segment_line(&line) {
                if !segment_cancel.load(Ordering::Relaxed) {
                    on_segment(start_ms, end_ms, text);
                }
            }
            push_diagnostic_line(&stdout_sink, &line);
        }
    });

    // Wake every tick to re-read the cancel flag regardless of whether the pipes said
    // anything, so a Cancel lands even during whisper's long silent decode/VAD phases. The
    // stderr drain still parses progress on its own thread throughout.
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

    // Reap on EVERY path — a killed child still has to be waited on — and only then join the
    // drains, which end as soon as the reaped process's pipes close.
    let status = child.wait().map_err(|error| error.to_string())?;
    let _ = stderr_thread.join();
    let _ = stdout_thread.join();

    if cancel.load(Ordering::Relaxed) {
        return Err(TRANSCRIPTION_CANCELLED.into());
    }

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

    // Both drain threads are joined by now, so this is the only remaining holder.
    let speech_regions = region_log
        .lock()
        .map(|mut log| std::mem::take(&mut *log).into_regions())
        .unwrap_or_default();

    Ok(WhisperTranscriptionResult {
        transcript_path,
        json_path,
        speech_regions,
        // Filled in by the caller, which is the layer that still has the decoded WAV.
        speech_envelope: None,
    })
}

/// Best-effort kill of the whole process tree rooted at `pid`.
#[cfg(target_os = "windows")]
fn kill_process_tree(pid: u32) {
    let mut command = Command::new("taskkill");
    hide_command_window(&mut command);
    let _ = command.args(["/F", "/T", "/PID", &pid.to_string()]).output();
}

#[cfg(not(target_os = "windows"))]
fn kill_process_tree(_pid: u32) {}

#[cfg(test)]
mod tests {
    /// The slot is process-wide, and cargo runs tests in parallel.
    static SLOT_TEST_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

    use super::*;

    #[test]
    fn a_silent_video_is_named_rather_than_reported_as_a_decode_fault() {
        // Reached from the Video Library's "Generate subtitles", which hands the video
        // itself to the decoder — nothing between it and here checks for audio.
        assert_eq!(
            decode_failure_message(
                "Stream map '' matches no streams.\nTo ignore this, add a trailing '?' to the map."
            ),
            "This video has no sound, so there is nothing to transcribe."
        );

        // A real decode fault still says what ffmpeg said.
        assert_eq!(
            decode_failure_message("  Error opening input file /nope.mkv.  "),
            "ffmpeg failed to decode the recording: Error opening input file /nope.mkv."
        );
    }

    /// Writes a 16 kHz mono WAV whose middle second is loud and whose edges are near-silent,
    /// optionally behind an extra chunk before `data` — ffmpeg writes a LIST/INFO chunk often
    /// enough that assuming the canonical 44-byte header would eventually read noise as audio.
    fn write_test_wav(path: &Path, extra_chunk: bool) {
        let sample_rate = 16_000u32;
        let mut samples: Vec<i16> = Vec::new();
        for index in 0..sample_rate * 3 {
            let loud = index >= sample_rate && index < sample_rate * 2;
            let amplitude = if loud { 8_000.0 } else { 8.0 };
            let phase = index as f32 / sample_rate as f32 * 440.0 * std::f32::consts::TAU;
            samples.push((phase.sin() * amplitude) as i16);
        }
        let audio: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();

        let mut body = Vec::new();
        body.extend_from_slice(b"WAVE");
        body.extend_from_slice(b"fmt ");
        body.extend_from_slice(&16u32.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes()); // PCM
        body.extend_from_slice(&1u16.to_le_bytes()); // mono
        body.extend_from_slice(&sample_rate.to_le_bytes());
        body.extend_from_slice(&(sample_rate * 2).to_le_bytes());
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(&16u16.to_le_bytes());
        if extra_chunk {
            body.extend_from_slice(b"LIST");
            body.extend_from_slice(&5u32.to_le_bytes());
            body.extend_from_slice(b"INFOx");
            body.push(0); // odd size carries a pad byte
        }
        body.extend_from_slice(b"data");
        body.extend_from_slice(&(audio.len() as u32).to_le_bytes());
        body.extend_from_slice(&audio);

        let mut file = Vec::new();
        file.extend_from_slice(b"RIFF");
        file.extend_from_slice(&(body.len() as u32).to_le_bytes());
        file.extend_from_slice(&body);
        fs::write(path, file).expect("write test wav");
    }

    /// The envelope must find the loud second and call the quiet edges silence, so a cue
    /// spanning the whole file trims to the speech inside it.
    #[test]
    fn the_envelope_finds_the_loud_stretch_in_a_wav() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("tone.wav");
        write_test_wav(&path, false);

        let envelope = SpeechEnvelope::from_wav_16k_mono(&path).expect("an envelope");
        let (start_ms, end_ms) = envelope.trim(0, 3_000, 100);

        assert!(
            (900..=1_000).contains(&start_ms),
            "speech starts at 1000ms, less the 100ms pad, got {start_ms}"
        );
        assert!(
            (2_000..=2_100).contains(&end_ms),
            "speech ends at 2000ms, plus the 100ms pad, got {end_ms}"
        );
    }

    /// The `data` chunk is found by walking the chunk list, not by assuming its offset.
    #[test]
    fn a_wav_with_an_extra_chunk_before_data_still_parses() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("tagged.wav");
        write_test_wav(&path, true);

        let envelope = SpeechEnvelope::from_wav_16k_mono(&path).expect("an envelope");
        let (start_ms, _) = envelope.trim(0, 3_000, 100);

        assert!(
            (900..=1_000).contains(&start_ms),
            "a LIST chunk before the audio must not shift the envelope, got {start_ms}"
        );
    }

    /// Audio with no dynamic range gives nothing to measure against, and guessing at a
    /// threshold there would trim real speech. It must decline instead.
    #[test]
    fn flat_audio_yields_no_envelope() {
        let dir = tempfile::tempdir().expect("temp dir");
        let path = dir.path().join("flat.wav");
        let audio: Vec<u8> = (0..16_000u32 * 2)
            .flat_map(|_| 1_000i16.to_le_bytes())
            .collect();
        let mut file = Vec::new();
        file.extend_from_slice(b"RIFF");
        file.extend_from_slice(&(36 + audio.len() as u32).to_le_bytes());
        file.extend_from_slice(b"WAVEfmt ");
        file.extend_from_slice(&16u32.to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&1u16.to_le_bytes());
        file.extend_from_slice(&16_000u32.to_le_bytes());
        file.extend_from_slice(&32_000u32.to_le_bytes());
        file.extend_from_slice(&2u16.to_le_bytes());
        file.extend_from_slice(&16u16.to_le_bytes());
        file.extend_from_slice(b"data");
        file.extend_from_slice(&(audio.len() as u32).to_le_bytes());
        file.extend_from_slice(&audio);
        fs::write(&path, file).expect("write flat wav");

        assert!(SpeechEnvelope::from_wav_16k_mono(&path).is_none());
    }

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
        assert_eq!(parse_whisper_progress_line("progress = 250%"), Some(100));
        assert_eq!(parse_whisper_progress_line("whisper_full_with_state: decode"), None);
        assert_eq!(parse_whisper_progress_line(""), None);
    }

    /// Both lines are copied verbatim from the pinned v1.8.4 binary's stderr, so a runtime
    /// upgrade that reformats them fails here rather than silently disabling the clamp.
    #[test]
    fn the_mapped_vad_line_gives_absolute_region_bounds() {
        let line = "whisper_vad: vad_segment_info: orig_start: 2.53, orig_end: 3.71,                     vad_start: 0.52, vad_end: 1.70";

        let (source, region) = parse_vad_region_line(line).expect("mapped line parses");

        assert_eq!(source, SpeechRegionSource::Mapped);
        assert_eq!(region.start_ms, 2530);
        assert_eq!(region.end_ms, 3710);
    }

    #[test]
    fn the_probe_vad_line_is_read_as_a_fallback() {
        let line = "whisper_vad_segments_from_probs: VAD segment 0: start = 1.92, end = 2.24                     (duration: 0.32)";

        let (source, region) = parse_vad_region_line(line).expect("probe line parses");

        assert_eq!(source, SpeechRegionSource::Probe);
        assert_eq!(region.start_ms, 1920);
        assert_eq!(region.end_ms, 2240);
    }

    #[test]
    fn ordinary_whisper_output_is_not_a_region() {
        for line in [
            "[00:00:01.920 --> 00:00:04.830]  生まれ変わる今",
            "whisper_print_progress_callback: progress =  40%",
            "whisper_vad: vad_segment_info: orig_start: , orig_end: 3.71",
            "",
        ] {
            assert!(parse_vad_region_line(line).is_none(), "{line:?}");
        }
    }

    #[test]
    fn seconds_convert_without_losing_a_millisecond() {
        assert_eq!(parse_vad_seconds("1.92", Rounding::Down), Some(1920));
        assert_eq!(parse_vad_seconds("1.92", Rounding::Up), Some(1920));
        assert_eq!(parse_vad_seconds("0.00", Rounding::Down), Some(0));
        assert_eq!(parse_vad_seconds("7", Rounding::Down), Some(7000));
        assert_eq!(parse_vad_seconds("1.5", Rounding::Down), Some(1500));
        assert_eq!(parse_vad_seconds("1.9204", Rounding::Down), Some(1920));
        assert_eq!(parse_vad_seconds("1.9204", Rounding::Up), Some(1921));
        assert_eq!(parse_vad_seconds("1.9200", Rounding::Up), Some(1920));
        assert_eq!(parse_vad_seconds("abc", Rounding::Down), None);
    }

    #[test]
    fn the_mapped_list_wins_when_both_were_printed() {
        let mut log = VadRegionLog::default();
        let mapped = SpeechRegion {
            start_ms: 2530,
            end_ms: 3710,
        };
        let probe = SpeechRegion {
            start_ms: 1920,
            end_ms: 2240,
        };
        log.push(SpeechRegionSource::Probe, probe);
        log.push(SpeechRegionSource::Mapped, mapped);

        assert_eq!(log.into_regions(), vec![mapped]);
    }

    #[test]
    fn the_probe_list_carries_a_build_that_stopped_printing_the_mapping() {
        let mut log = VadRegionLog::default();
        let probe = SpeechRegion {
            start_ms: 1920,
            end_ms: 2240,
        };
        log.push(SpeechRegionSource::Probe, probe);

        assert_eq!(log.into_regions(), vec![probe]);
        assert!(VadRegionLog::default().into_regions().is_empty());
    }

    /// The whole point: a second claim is refused while the first is alive.
    ///
    /// Serialised against the other slot test by a mutex, because the slot is process-wide
    /// state and cargo runs tests in parallel — two of them racing it would make both flaky
    /// for a reason that has nothing to do with the code under test.
    #[test]
    fn the_whisper_slot_admits_one_run_at_a_time() {
        let _serialise = SLOT_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());

        let first = WhisperSlotGuard::acquire("busy").expect("the slot starts free");

        let second = WhisperSlotGuard::acquire("a transcription is already running");
        assert_eq!(
            second.err().as_deref(),
            Some("a transcription is already running"),
            "the second claim is refused, and told why"
        );

        drop(first);
        WhisperSlotGuard::acquire("busy").expect("the slot is free again");
    }

    #[test]
    fn the_whisper_slot_is_released_by_a_panicking_run() {
        let _serialise = SLOT_TEST_LOCK.lock().unwrap_or_else(|error| error.into_inner());

        let panicked = std::panic::catch_unwind(|| {
            let _slot = WhisperSlotGuard::acquire("busy").expect("free");
            panic!("whisper blew up mid-pass");
        });

        assert!(panicked.is_err());
        WhisperSlotGuard::acquire("busy").expect("an unwind still released the slot");
    }

    #[test]
    fn parse_whisper_segment_line_reads_bounds_and_text() {
        assert_eq!(
            parse_whisper_segment_line(
                "[00:00:06.830 --> 00:00:13.490]  僕はドイツ人なんですけど、日本人の友達作りたいです。"
            ),
            Some((
                6830,
                13490,
                "僕はドイツ人なんですけど、日本人の友達作りたいです。".to_string()
            ))
        );
        assert_eq!(
            parse_whisper_segment_line("[01:02:03.004 --> 01:02:04.000]   the  cat sat  "),
            Some((3723004, 3724000, "the  cat sat".to_string()))
        );
    }

    #[test]
    fn parse_whisper_segment_line_ignores_everything_else() {
        assert_eq!(parse_whisper_segment_line("whisper_init_from_file: loading"), None);
        assert_eq!(parse_whisper_segment_line(""), None);
        assert_eq!(parse_whisper_segment_line("[00:00:00.000 --> 00:00:01.000]   "), None);
        assert_eq!(parse_whisper_segment_line("[00:00:00.000 00:00:01.000] hi"), None);
        assert_eq!(parse_whisper_segment_line("[bad --> worse] hi"), None);
        assert_eq!(parse_whisper_segment_line("[00:00:99.000 --> 00:00:01.000] hi"), None);
        assert_eq!(
            parse_whisper_segment_line("[1000000000000000:00:00.000 --> 00:00:01.000] hi"),
            None
        );
    }

    #[test]
    fn cap_details_surfaces_the_error_line_over_the_banner() {
        let dump = [
            "whisper_model_load: loading model",
            "some other chatter",
            "more chatter",
            "error: failed to read audio file 'x.wav'",
        ]
        .join("\n");
        let capped = cap_details(dump);
        assert!(
            capped.starts_with("error: failed to read audio file"),
            "the failure reason should lead, got {capped:?}"
        );
    }

    #[test]
    fn whisper_noise_lines_are_kept_out_of_the_diagnostics() {
        assert!(is_whisper_noise_line("whisper_print_timings:    total time = 3759.40 ms"));
        assert!(is_whisper_noise_line("load_backend: loaded CPU backend from x.dll"));
        assert!(is_whisper_noise_line("whisper_print_progress_callback: progress =  96%"));
        assert!(is_whisper_noise_line("   "));
        // A real failure is never noise.
        assert!(!is_whisper_noise_line("error: failed to read audio file 'x.wav'"));
        assert!(!is_whisper_noise_line("whisper-cli: unrecognized argument"));
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

    #[test]
    fn transcription_thread_count_is_bounded_and_ordered() {
        let low = transcription_thread_count("low");
        let balanced = transcription_thread_count("balanced");
        let high = transcription_thread_count("high");
        assert!(low >= 1, "low must be at least 1, got {low}");
        assert!(balanced >= 1, "balanced must be at least 1, got {balanced}");
        assert!(high >= 1, "high must be at least 1, got {high}");
        assert!(low <= balanced, "low ({low}) must not exceed balanced ({balanced})");
        assert!(balanced <= high, "balanced ({balanced}) must not exceed high ({high})");
        assert_eq!(transcription_thread_count("nonsense"), balanced);
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
            thread_count: transcription_thread_count("balanced"),
            music_mode: std::env::var("WOU_MUSIC").is_ok(),
            fast_decode: std::env::var("WOU_FAST").is_ok(),
        };
        let result = run_whisper_transcription(
            &request,
            Arc::new(AtomicBool::new(false)),
            |percent| {
                if percent % 25 == 0 {
                    eprintln!("progress {percent}%");
                }
            },
            // Streamed sentences: these should match the sidecar the assertions below read.
            |start_ms, end_ms, text| eprintln!("LIVE [{start_ms}-{end_ms}] {text}"),
        )
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
