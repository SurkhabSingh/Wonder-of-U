//! Drift-free, language-agnostic transcription: Silero VAD (sherpa-onnx) segments the
//! whole file into short, silence-bounded speech regions with absolute (drift-free) sample
//! offsets, and each region is transcribed by whisper.cpp's **ggml** model — run through a
//! `whisper-server` instance loaded once per file — so timestamps come from the VAD (they
//! cannot drift), non-speech yields no segments (no music-loop repetition), and the *text*
//! is full ggml quality (int8 ONNX quantization mangled Japanese; ggml does not).
//!
//! It emits the same `{ "transcription": [ { "offsets": { "from", "to" }, "text" } ] }`
//! JSON + newline `.txt` that `recording_library::transcription`'s `parse_whisper_segments`
//! → `clean_segments` → `store_segments_sidecar` already consume, so nothing downstream
//! changes. The contract mirrors `crate::transcription::run_whisper_transcription`.

use std::{
    env, fs,
    io::Cursor,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use sherpa_onnx::{SileroVadModelConfig, VadModelConfig, VoiceActivityDetector, Wave};

use crate::transcription::WhisperTranscriptionResult;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Whisper's native input rate. ffmpeg resamples every recording to 16 kHz mono s16le.
const TARGET_SAMPLE_RATE: i32 = 16_000;
/// Silero VAD is fed fixed 512-sample windows (its trained frame size at 16 kHz).
const VAD_WINDOW: usize = 512;
/// Cap a speech region well under Whisper's hard 30 s limit (Whisper silently truncates
/// past 30 s, and Silero only raises its cut threshold past this value, so a region can run
/// a little over). 20 s leaves margin while giving Whisper ample context per clip.
const MAX_SPEECH_SECONDS: f32 = 20.0;
/// VAD ring-buffer size; comfortably larger than a capped-plus-overrun region.
const VAD_BUFFER_SECONDS: f32 = 60.0;

/// Sentinel error returned when a cancel was requested mid-run, so the caller can render a
/// "cancelled" outcome instead of a failure.
pub const TRANSCRIPTION_CANCELLED: &str = "transcription cancelled.";

#[derive(Debug, Clone)]
pub struct SherpaTranscriptionRequest {
    /// Silero VAD ONNX model (segmentation only).
    pub vad_model_path: PathBuf,
    /// `whisper-server.exe` (ships with the whisper.cpp runtime).
    pub whisper_server_path: PathBuf,
    /// The ggml Whisper model the app already manages (e.g. `ggml-large-v3.bin`).
    pub ggml_model_path: PathBuf,
    pub audio_path: PathBuf,
    /// ffmpeg decodes the recording to a 16 kHz mono WAV. Required.
    pub ffmpeg_path: PathBuf,
    /// Whisper language hint (e.g. "ja"); empty string → auto-detect.
    pub language: String,
    /// Recognizer worker threads (the CPU-usage lever).
    pub num_threads: i32,
    /// Total audio duration in ms; used to scale progress. `0` falls back to the decoded
    /// sample count.
    pub duration_ms: u64,
}

/// The shared VAD model path under the asset directory: `{asset}/models/sherpa/silero_vad.onnx`.
pub fn sherpa_vad_model_path(asset_directory: &Path) -> PathBuf {
    asset_directory
        .join("models")
        .join("sherpa")
        .join("silero_vad.onnx")
}

/// Output JSON shape — the exact subset `parse_whisper_segments` reads. `to`/`from` are ms.
#[derive(serde::Serialize)]
struct OutOffsets {
    from: u64,
    to: u64,
}

#[derive(serde::Serialize)]
struct OutSegment {
    offsets: OutOffsets,
    text: String,
}

#[derive(serde::Serialize)]
struct OutJson {
    transcription: Vec<OutSegment>,
}

/// Deletes tracked temp files (the decoded WAV) on every return path.
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

fn hide_command_window(command: &mut Command) {
    #[cfg(target_os = "windows")]
    {
        command.creation_flags(CREATE_NO_WINDOW);
    }
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0)
}

/// A fixed ASCII base for the merged output (never derived from a possibly non-ASCII stem).
fn sherpa_output_base() -> PathBuf {
    env::temp_dir().join(format!(
        "wonder-of-u-sherpa-{}-{}",
        std::process::id(),
        unique_suffix()
    ))
}

/// Milliseconds spanned by `sample_count` at `sample_rate`.
fn samples_to_ms(sample_count: usize, sample_rate: i32) -> u64 {
    (sample_count as u64) * 1000 / (sample_rate.max(1) as u64)
}

fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// Decodes any recording to a 16 kHz mono s16le WAV at an ASCII temp path. ffmpeg handles
/// non-ASCII *input* paths on Windows; the ASCII output keeps the ONNX VAD layer happy.
fn decode_to_wav_16k(ffmpeg_path: &Path, input: &Path) -> Result<PathBuf, String> {
    let output = env::temp_dir().join(format!(
        "wonder-of-u-sherpa-input-{}-{}.wav",
        std::process::id(),
        unique_suffix()
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

/// Encodes 16 kHz mono f32 samples into an in-memory WAV (for multipart upload), avoiding a
/// temp file per segment.
fn samples_to_wav_bytes(samples: &[f32], sample_rate: i32) -> Result<Vec<u8>, String> {
    let spec = hound::WavSpec {
        channels: 1,
        sample_rate: sample_rate.max(1) as u32,
        bits_per_sample: 16,
        sample_format: hound::SampleFormat::Int,
    };
    let mut cursor = Cursor::new(Vec::<u8>::new());
    {
        let mut writer =
            hound::WavWriter::new(&mut cursor, spec).map_err(|error| error.to_string())?;
        for &sample in samples {
            let value = (sample.clamp(-1.0, 1.0) * 32767.0) as i16;
            writer
                .write_sample(value)
                .map_err(|error| error.to_string())?;
        }
        writer.finalize().map_err(|error| error.to_string())?;
    }
    Ok(cursor.into_inner())
}

/// A `whisper-server` instance loaded once with the ggml model and killed on drop. Each VAD
/// segment is POSTed to its `/inference` endpoint, so the ~1.5 GB model is loaded a single
/// time per file rather than once per segment.
struct WhisperServer {
    child: Child,
    port: u16,
}

impl WhisperServer {
    fn free_port() -> Result<u16, String> {
        let listener = TcpListener::bind("127.0.0.1:0")
            .map_err(|error| format!("Could not reserve a local port: {error}"))?;
        let port = listener
            .local_addr()
            .map_err(|error| error.to_string())?
            .port();
        drop(listener);
        Ok(port)
    }

    fn start(exe: &Path, model: &Path, threads: i32) -> Result<Self, String> {
        let port = Self::free_port()?;
        let mut command = Command::new(exe);
        hide_command_window(&mut command);
        if let Some(parent) = exe.parent() {
            if !parent.as_os_str().is_empty() {
                command.current_dir(parent);
            }
        }
        command
            .arg("--model")
            .arg(model)
            .arg("--host")
            .arg("127.0.0.1")
            .arg("--port")
            .arg(port.to_string())
            .arg("-t")
            .arg(threads.max(1).to_string())
            .stdout(Stdio::null())
            .stderr(Stdio::null());

        let child = command
            .spawn()
            .map_err(|error| format!("Could not start whisper-server: {error}"))?;
        let server = WhisperServer { child, port };

        // whisper-server begins serving only after the model finishes loading, so a
        // successful GET on the root means it is ready to transcribe.
        let probe = reqwest::blocking::Client::builder()
            .timeout(Duration::from_secs(3))
            .build()
            .map_err(|error| error.to_string())?;
        let deadline = Instant::now() + Duration::from_secs(180);
        loop {
            if Instant::now() > deadline {
                return Err("whisper-server did not become ready in time.".into());
            }
            let ready = probe
                .get(format!("http://127.0.0.1:{port}/"))
                .send()
                .map(|response| response.status().is_success())
                .unwrap_or(false);
            if ready {
                return Ok(server);
            }
            std::thread::sleep(Duration::from_millis(300));
        }
    }

    fn transcribe(
        &self,
        client: &reqwest::blocking::Client,
        wav_bytes: Vec<u8>,
        language: &str,
    ) -> Result<String, String> {
        let part = reqwest::blocking::multipart::Part::bytes(wav_bytes)
            .file_name("clip.wav")
            .mime_str("audio/wav")
            .map_err(|error| error.to_string())?;
        let mut form = reqwest::blocking::multipart::Form::new()
            .part("file", part)
            .text("response_format", "json")
            .text("temperature", "0");
        if !language.trim().is_empty() {
            form = form.text("language", language.trim().to_string());
        }

        let response = client
            .post(format!("http://127.0.0.1:{}/inference", self.port))
            .multipart(form)
            .send()
            .map_err(|error| format!("whisper-server request failed: {error}"))?;
        if !response.status().is_success() {
            return Err(format!(
                "whisper-server returned status {}",
                response.status()
            ));
        }
        let body_text = response
            .text()
            .map_err(|error| format!("whisper-server response read failed: {error}"))?;
        let body: serde_json::Value = serde_json::from_str(&body_text)
            .map_err(|error| format!("whisper-server returned invalid JSON: {error}"))?;
        Ok(body
            .get("text")
            .and_then(|value| value.as_str())
            .unwrap_or("")
            .trim()
            .to_string())
    }
}

impl Drop for WhisperServer {
    fn drop(&mut self) {
        let _ = self.child.kill();
        #[cfg(target_os = "windows")]
        {
            let mut kill = Command::new("taskkill");
            hide_command_window(&mut kill);
            let _ = kill
                .args(["/F", "/T", "/PID", &self.child.id().to_string()])
                .output();
        }
        let _ = self.child.wait();
    }
}

/// Recognizes every queued VAD speech region via whisper-server, appending a segment per
/// region with absolute ms offsets from the region's sample offset. Returns the cancel
/// sentinel if a cancel was requested.
fn drain_segments<F: Fn(u8)>(
    vad: &VoiceActivityDetector,
    server: &WhisperServer,
    client: &reqwest::blocking::Client,
    sample_rate: i32,
    language: &str,
    total_ms: u64,
    cancel: &AtomicBool,
    on_progress: &F,
    out: &mut Vec<OutSegment>,
) -> Result<(), String> {
    while let Some(segment) = vad.front() {
        if cancel.load(Ordering::Relaxed) {
            return Err(TRANSCRIPTION_CANCELLED.into());
        }

        let samples = segment.samples();
        let start_ms = samples_to_ms(segment.start().max(0) as usize, sample_rate);
        let end_ms = start_ms + samples_to_ms(samples.len(), sample_rate);

        let wav_bytes = samples_to_wav_bytes(samples, sample_rate)?;
        let text = server.transcribe(client, wav_bytes, language)?;

        if !text.is_empty() {
            out.push(OutSegment {
                offsets: OutOffsets {
                    from: start_ms,
                    to: end_ms,
                },
                text,
            });
        }

        let percent = (end_ms.min(total_ms).saturating_mul(100) / total_ms.max(1)).min(99) as u8;
        on_progress(percent);

        vad.pop();
    }
    Ok(())
}

/// Transcribes `request.audio_path` with Silero VAD + whisper.cpp ggml (via whisper-server)
/// and writes the merged `.txt` + `.json` (same shape as a whisper run). `on_progress` gets
/// 0–100 scaled over the whole file; `cancel` is checked between segments.
pub fn run_sherpa_transcription<F: Fn(u8)>(
    request: &SherpaTranscriptionRequest,
    on_progress: F,
    cancel: Arc<AtomicBool>,
) -> Result<WhisperTranscriptionResult, String> {
    for path in [
        &request.vad_model_path,
        &request.whisper_server_path,
        &request.ggml_model_path,
    ] {
        if !path.exists() {
            return Err(format!("A transcription asset is missing: {}", path.display()));
        }
    }

    let mut temps = TempCleanup::new();
    let wav_path = decode_to_wav_16k(&request.ffmpeg_path, &request.audio_path)?;
    temps.track(wav_path.clone());

    if cancel.load(Ordering::Relaxed) {
        return Err(TRANSCRIPTION_CANCELLED.into());
    }

    let mut silero = SileroVadModelConfig::default();
    silero.model = Some(path_string(&request.vad_model_path));
    silero.threshold = 0.5;
    silero.min_silence_duration = 0.25;
    silero.min_speech_duration = 0.25;
    silero.max_speech_duration = MAX_SPEECH_SECONDS;
    let vad_config = VadModelConfig {
        silero_vad: silero,
        ten_vad: Default::default(),
        sample_rate: TARGET_SAMPLE_RATE,
        num_threads: 1,
        provider: Some("cpu".to_string()),
        debug: false,
    };
    let vad = VoiceActivityDetector::create(&vad_config, VAD_BUFFER_SECONDS)
        .ok_or_else(|| "Could not load the speech detector.".to_string())?;

    let wave = Wave::read(&path_string(&wav_path))
        .ok_or_else(|| "Could not read the decoded audio for transcription.".to_string())?;
    let sample_rate = wave.sample_rate();
    let samples: Vec<f32> = wave.samples().to_vec();
    let total_ms = request
        .duration_ms
        .max(samples_to_ms(samples.len(), sample_rate));

    // Load the ggml model once for the whole file; killed on drop (any return path).
    let server = WhisperServer::start(
        &request.whisper_server_path,
        &request.ggml_model_path,
        request.num_threads,
    )?;
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(600))
        .build()
        .map_err(|error| error.to_string())?;
    let language = request.language.trim();

    let mut segments: Vec<OutSegment> = Vec::new();
    for chunk in samples.chunks(VAD_WINDOW) {
        if cancel.load(Ordering::Relaxed) {
            return Err(TRANSCRIPTION_CANCELLED.into());
        }
        vad.accept_waveform(chunk);
        drain_segments(
            &vad, &server, &client, sample_rate, language, total_ms, &cancel, &on_progress,
            &mut segments,
        )?;
    }
    vad.flush();
    drain_segments(
        &vad, &server, &client, sample_rate, language, total_ms, &cancel, &on_progress,
        &mut segments,
    )?;

    let output_base = sherpa_output_base();
    let transcript_path = PathBuf::from(format!("{}.txt", output_base.display()));
    let json_path = PathBuf::from(format!("{}.json", output_base.display()));

    let text = segments
        .iter()
        .map(|segment| segment.text.trim())
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&transcript_path, format!("{text}\n")).map_err(|error| error.to_string())?;

    let merged = OutJson {
        transcription: segments,
    };
    let serialized = serde_json::to_string(&merged).map_err(|error| error.to_string())?;
    fs::write(&json_path, serialized).map_err(|error| error.to_string())?;

    on_progress(100);

    Ok(WhisperTranscriptionResult {
        transcript_path,
        json_path,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn samples_to_ms_maps_offsets_without_drift() {
        assert_eq!(samples_to_ms(0, 16_000), 0);
        assert_eq!(samples_to_ms(16_000, 16_000), 1000);
        assert_eq!(samples_to_ms(8_000, 16_000), 500);
        assert_eq!(samples_to_ms(16_000 * 1200, 16_000), 1_200_000);
        assert_eq!(samples_to_ms(16_000, 0), 16_000_000);
    }

    #[test]
    fn sherpa_output_base_is_ascii() {
        let base = sherpa_output_base();
        let text = base.to_str().expect("output base should be unicode");
        assert!(text.is_ascii(), "sherpa output base must be ASCII: {text}");
        assert!(text.contains("wonder-of-u-sherpa-"));
    }

    #[test]
    fn wav_bytes_have_riff_header_and_expected_length() {
        let samples = vec![0.0f32; 16_000]; // 1 second at 16 kHz
        let bytes = samples_to_wav_bytes(&samples, 16_000).expect("encode wav");
        assert_eq!(&bytes[0..4], b"RIFF");
        assert_eq!(&bytes[8..12], b"WAVE");
        // 44-byte header + 16000 samples * 2 bytes.
        assert_eq!(bytes.len(), 44 + 16_000 * 2);
    }

    /// Manual end-to-end check of the real engine. Ignored by default. Run with:
    ///   WOU_VAD=".../silero_vad.onnx" WOU_SERVER=".../whisper-server.exe" \
    ///   WOU_GGML=".../ggml-large-v3.bin" WOU_AUDIO=".../#57 Ani-One....mp3" \
    ///   WOU_FFMPEG=".../ffmpeg.exe" WOU_LANG=ja \
    ///   cargo test --release end_to_end_real_audio -- --ignored --nocapture
    #[test]
    #[ignore = "manual: needs local models + audio + ffmpeg + whisper-server"]
    fn end_to_end_real_audio() {
        let request = SherpaTranscriptionRequest {
            vad_model_path: PathBuf::from(std::env::var("WOU_VAD").expect("WOU_VAD")),
            whisper_server_path: PathBuf::from(std::env::var("WOU_SERVER").expect("WOU_SERVER")),
            ggml_model_path: PathBuf::from(std::env::var("WOU_GGML").expect("WOU_GGML")),
            audio_path: PathBuf::from(std::env::var("WOU_AUDIO").expect("WOU_AUDIO")),
            ffmpeg_path: PathBuf::from(std::env::var("WOU_FFMPEG").expect("WOU_FFMPEG")),
            language: std::env::var("WOU_LANG").unwrap_or_default(),
            num_threads: 8,
            duration_ms: 0,
        };

        let result = run_sherpa_transcription(
            &request,
            |percent| {
                if percent % 20 == 0 {
                    eprintln!("progress {percent}%");
                }
            },
            Arc::new(AtomicBool::new(false)),
        )
        .expect("transcription should succeed");

        let json = fs::read_to_string(&result.json_path).expect("json written");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("valid json");
        let segments = parsed["transcription"].as_array().expect("segments array");
        let last_end = segments
            .last()
            .and_then(|segment| segment["offsets"]["to"].as_u64())
            .unwrap_or(0);
        eprintln!("RESULT segments={} last_end_ms={}", segments.len(), last_end);
        for segment in segments.iter().take(6) {
            eprintln!("  {segment}");
        }
        assert!(!segments.is_empty());
        let _ = fs::remove_file(&result.json_path);
        let _ = fs::remove_file(&result.transcript_path);
    }
}
