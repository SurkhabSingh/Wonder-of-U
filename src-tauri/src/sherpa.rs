//! Frame-synchronous transcription engine: Silero VAD segments the whole file into
//! short, silence-bounded speech regions with absolute (drift-free) sample offsets, and
//! a multilingual Whisper ONNX recognizer transcribes each region. Every subtitle's time
//! comes from the VAD's running sample count — never from an accumulating decoder clock —
//! so timestamps cannot drift on long audio, and non-speech (music/silence) yields no
//! segments (so the whisper-on-instrumental repetition loop can't happen). Language-
//! agnostic: an empty `language` lets Whisper auto-detect per segment.
//!
//! It emits the exact same `{ "transcription": [ { "offsets": { "from", "to" }, "text" } ] }`
//! JSON + newline `.txt` that `recording_library::transcription`'s `parse_whisper_segments`
//! → `clean_segments` → `store_segments_sidecar` already consume, so nothing downstream
//! changes. The contract mirrors `crate::transcription::run_whisper_transcription`.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

use sherpa_onnx::{
    OfflineRecognizer, OfflineRecognizerConfig, OfflineWhisperModelConfig, SileroVadModelConfig,
    VadModelConfig, VoiceActivityDetector, Wave,
};

use crate::transcription::WhisperTranscriptionResult;

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Whisper's native input rate. ffmpeg resamples every recording to 16 kHz mono s16le
/// before the VAD/recognizer see it.
const TARGET_SAMPLE_RATE: i32 = 16_000;
/// Silero VAD is fed fixed 512-sample windows (its trained frame size at 16 kHz).
const VAD_WINDOW: usize = 512;
/// Cap a speech region well under Whisper's hard 30 s limit. Whisper *silently truncates*
/// audio past 30 s (dropping the tail of a long continuous-speech region), and Silero's
/// `max_speech_duration` only *raises the cut threshold* past this value rather than cutting
/// exactly at it — so a segment can still run a few seconds over. 20 s leaves ample margin
/// so no region ever reaches 30 s, while still giving Whisper plenty of context per clip.
const MAX_SPEECH_SECONDS: f32 = 20.0;
/// VAD ring-buffer size. Comfortably larger than `MAX_SPEECH_SECONDS` (+ the overrun before
/// a cut lands) so it never has to grow/copy mid-run.
const VAD_BUFFER_SECONDS: f32 = 60.0;

/// Sentinel error returned when a cancel was requested mid-run, so the caller can render a
/// "cancelled" outcome instead of a failure (mirrors the yt-dlp "download cancelled." tail).
pub const TRANSCRIPTION_CANCELLED: &str = "transcription cancelled.";

#[derive(Debug, Clone)]
pub struct SherpaTranscriptionRequest {
    pub vad_model_path: PathBuf,
    pub encoder_path: PathBuf,
    pub decoder_path: PathBuf,
    pub tokens_path: PathBuf,
    pub audio_path: PathBuf,
    /// ffmpeg decodes the (possibly mp3 / non-16 kHz) recording to a 16 kHz mono WAV the
    /// ONNX models require. Required — the engine cannot run without it.
    pub ffmpeg_path: PathBuf,
    /// Whisper language hint (e.g. "ja"); empty string → auto-detect per segment.
    pub language: String,
    /// Recognizer worker threads (the CPU-usage lever, mirrors whisper's `-t`).
    pub num_threads: i32,
    /// Total audio duration in ms; used to scale progress. `0` falls back to the decoded
    /// sample count.
    pub duration_ms: u64,
}

/// Resolved on-disk paths of a sherpa model set. Layout under the asset directory:
///   `{asset}/models/sherpa/silero_vad.onnx`            (shared VAD)
///   `{asset}/models/sherpa/{choice}/{choice}-encoder.int8.onnx`
///   `{asset}/models/sherpa/{choice}/{choice}-decoder.int8.onnx`
///   `{asset}/models/sherpa/{choice}/{choice}-tokens.txt`
#[derive(Debug, Clone)]
pub struct SherpaModelPaths {
    pub vad_model_path: PathBuf,
    pub encoder_path: PathBuf,
    pub decoder_path: PathBuf,
    pub tokens_path: PathBuf,
}

impl SherpaModelPaths {
    /// True only when every file the engine needs is present on disk.
    pub fn all_present(&self) -> bool {
        self.vad_model_path.exists()
            && self.encoder_path.exists()
            && self.decoder_path.exists()
            && self.tokens_path.exists()
    }
}

/// The root directory holding the shared VAD and the per-choice model folders.
pub fn sherpa_models_directory(asset_directory: &Path) -> PathBuf {
    asset_directory.join("models").join("sherpa")
}

/// Builds the expected file paths for a given model choice (e.g. `"large-v3"`). Existence
/// is a separate check (`all_present`).
pub fn resolve_sherpa_model_paths(asset_directory: &Path, model_choice: &str) -> SherpaModelPaths {
    let root = sherpa_models_directory(asset_directory);
    let model_dir = root.join(model_choice);
    SherpaModelPaths {
        vad_model_path: root.join("silero_vad.onnx"),
        encoder_path: model_dir.join(format!("{model_choice}-encoder.int8.onnx")),
        decoder_path: model_dir.join(format!("{model_choice}-decoder.int8.onnx")),
        tokens_path: model_dir.join(format!("{model_choice}-tokens.txt")),
    }
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

/// A fixed ASCII base for the merged output, like `transcription.rs`'s
/// `transcript_output_base`. Deliberately not derived from the (possibly non-ASCII)
/// recording stem so downstream file writes stay clean.
fn sherpa_output_base() -> PathBuf {
    env::temp_dir().join(format!(
        "wonder-of-u-sherpa-{}-{}",
        std::process::id(),
        unique_suffix()
    ))
}

/// Convert milliseconds spanned by `sample_count` at `sample_rate`.
fn samples_to_ms(sample_count: usize, sample_rate: i32) -> u64 {
    (sample_count as u64) * 1000 / (sample_rate.max(1) as u64)
}

/// Decodes any recording to a 16 kHz mono s16le WAV at an ASCII temp path. ffmpeg handles
/// non-ASCII *input* paths on Windows; the ASCII output keeps the ONNX layer (which mishandles
/// non-ASCII paths) happy.
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
        "-map",
        "0:a:0",
        "-vn",
        "-ar",
        "16000",
        "-ac",
        "1",
        "-c:a",
        "pcm_s16le",
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

fn path_string(path: &Path) -> String {
    path.to_string_lossy().to_string()
}

/// Recognizes every VAD speech region currently queued, appending a segment per region with
/// absolute ms offsets derived from the region's sample offset. Returns the cancel sentinel
/// error if a cancel was requested.
fn drain_segments<F: Fn(u8)>(
    vad: &VoiceActivityDetector,
    recognizer: &OfflineRecognizer,
    sample_rate: i32,
    total_ms: u64,
    cancel: &AtomicBool,
    on_progress: &F,
    out: &mut Vec<OutSegment>,
) -> Result<(), String> {
    while let Some(segment) = vad.front() {
        if cancel.load(Ordering::Relaxed) {
            return Err(TRANSCRIPTION_CANCELLED.into());
        }

        let start_ms = samples_to_ms(segment.start().max(0) as usize, sample_rate);
        let end_ms = start_ms + samples_to_ms(segment.samples().len(), sample_rate);

        let stream = recognizer.create_stream();
        stream.accept_waveform(sample_rate, segment.samples());
        recognizer.decode(&stream);
        let text = stream
            .get_result()
            .map(|result| result.text.trim().to_string())
            .unwrap_or_default();

        if !text.is_empty() {
            out.push(OutSegment {
                offsets: OutOffsets {
                    from: start_ms,
                    to: end_ms,
                },
                text,
            });
        }

        // Progress tracks the latest recognized position over the whole file; hold at 99
        // until the merge writes files so the bar only completes when the result is ready.
        let percent = (end_ms.min(total_ms).saturating_mul(100) / total_ms.max(1)).min(99) as u8;
        on_progress(percent);

        vad.pop();
    }
    Ok(())
}

/// Transcribes `request.audio_path` with Silero VAD + a Whisper ONNX model and writes the
/// merged `.txt` + `.json` (same shape as a whisper run). `on_progress` gets 0–100 scaled
/// over the whole file; `cancel` is checked between windows and segments (in-process, so a
/// cancel is a clean early return — no child process to kill).
pub fn run_sherpa_transcription<F: Fn(u8)>(
    request: &SherpaTranscriptionRequest,
    on_progress: F,
    cancel: Arc<AtomicBool>,
) -> Result<WhisperTranscriptionResult, String> {
    for model in [
        &request.vad_model_path,
        &request.encoder_path,
        &request.decoder_path,
        &request.tokens_path,
    ] {
        if !model.exists() {
            return Err(format!(
                "A transcription model file is missing: {}",
                model.display()
            ));
        }
    }

    let mut temps = TempCleanup::new();
    let wav_path = decode_to_wav_16k(&request.ffmpeg_path, &request.audio_path)?;
    temps.track(wav_path.clone());

    if cancel.load(Ordering::Relaxed) {
        return Err(TRANSCRIPTION_CANCELLED.into());
    }

    let mut config = OfflineRecognizerConfig::default();
    config.model_config.whisper = OfflineWhisperModelConfig {
        encoder: Some(path_string(&request.encoder_path)),
        decoder: Some(path_string(&request.decoder_path)),
        language: Some(request.language.trim().to_string()),
        task: Some("transcribe".to_string()),
        tail_paddings: 0,
        enable_token_timestamps: false,
        enable_segment_timestamps: false,
    };
    config.model_config.tokens = Some(path_string(&request.tokens_path));
    config.model_config.provider = Some("cpu".to_string());
    config.model_config.num_threads = request.num_threads.max(1);
    config.model_config.debug = false;

    let recognizer = OfflineRecognizer::create(&config)
        .ok_or_else(|| "Could not load the transcription model.".to_string())?;

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

    let mut segments: Vec<OutSegment> = Vec::new();
    for chunk in samples.chunks(VAD_WINDOW) {
        if cancel.load(Ordering::Relaxed) {
            return Err(TRANSCRIPTION_CANCELLED.into());
        }
        vad.accept_waveform(chunk);
        drain_segments(
            &vad,
            &recognizer,
            sample_rate,
            total_ms,
            &cancel,
            &on_progress,
            &mut segments,
        )?;
    }
    vad.flush();
    drain_segments(
        &vad,
        &recognizer,
        sample_rate,
        total_ms,
        &cancel,
        &on_progress,
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
        // 16 kHz: 16 000 samples == 1000 ms, exactly, at any absolute offset.
        assert_eq!(samples_to_ms(0, 16_000), 0);
        assert_eq!(samples_to_ms(16_000, 16_000), 1000);
        assert_eq!(samples_to_ms(8_000, 16_000), 500);
        // A far-into-a-long-file offset stays exact (no accumulation): 20 minutes in.
        assert_eq!(samples_to_ms(16_000 * 1200, 16_000), 1_200_000);
        // Guards against a zero sample rate.
        assert_eq!(samples_to_ms(16_000, 0), 16_000_000);
    }

    #[test]
    fn sherpa_output_base_is_ascii() {
        let base = sherpa_output_base();
        let text = base.to_str().expect("output base should be unicode");
        assert!(text.is_ascii(), "sherpa output base must be ASCII: {text}");
        assert!(text.contains("wonder-of-u-sherpa-"));
    }

    /// Manual end-to-end check of the real engine on a real recording. Ignored by default
    /// (needs local models + audio + ffmpeg). Run with, e.g.:
    ///   WOU_ASSET_DIR=".../assets" WOU_AUDIO=".../#57 Ani-One....mp3" \
    ///   WOU_FFMPEG=".../ffmpeg.exe" WOU_LANG=ja WOU_SHERPA_MODEL=large-v3 \
    ///   cargo test --release end_to_end_real_audio -- --ignored --nocapture
    #[test]
    #[ignore = "manual: needs local models + audio + ffmpeg"]
    fn end_to_end_real_audio() {
        use std::sync::atomic::AtomicBool;

        let asset = std::env::var("WOU_ASSET_DIR").expect("set WOU_ASSET_DIR");
        let audio = std::env::var("WOU_AUDIO").expect("set WOU_AUDIO");
        let ffmpeg = std::env::var("WOU_FFMPEG").expect("set WOU_FFMPEG");
        let model = std::env::var("WOU_SHERPA_MODEL").unwrap_or_else(|_| "large-v3".into());
        let language = std::env::var("WOU_LANG").unwrap_or_default();

        let paths = resolve_sherpa_model_paths(Path::new(&asset), &model);
        assert!(paths.all_present(), "sherpa models missing under {asset}");

        let request = SherpaTranscriptionRequest {
            vad_model_path: paths.vad_model_path,
            encoder_path: paths.encoder_path,
            decoder_path: paths.decoder_path,
            tokens_path: paths.tokens_path,
            audio_path: PathBuf::from(&audio),
            ffmpeg_path: PathBuf::from(&ffmpeg),
            language,
            num_threads: 4,
            duration_ms: 0,
        };

        let last = Arc::new(AtomicBool::new(false));
        let result = run_sherpa_transcription(
            &request,
            |percent| {
                if percent % 20 == 0 {
                    eprintln!("progress {percent}%");
                }
            },
            Arc::clone(&last),
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
        for segment in segments.iter().take(3) {
            eprintln!("  {segment}");
        }
        assert!(!segments.is_empty(), "expected at least one segment");

        let _ = fs::remove_file(&result.json_path);
        let _ = fs::remove_file(&result.transcript_path);
    }
}
