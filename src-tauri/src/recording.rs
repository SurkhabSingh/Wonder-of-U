use std::{
    collections::VecDeque,
    fs::{self, OpenOptions},
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug)]
pub struct RecordingCaptureResult {
    pub output_path: PathBuf,
    pub display_name: String,
    pub duration_ms: u64,
    pub bytes_written: u64,
    pub created_at_ms: u64,
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as u64)
        .unwrap_or(0)
}

fn write_worker_log(log_path: &Path, level: &str, event: &str, message: &str) {
    let payload = serde_json::json!({
        "tsMs": now_ms(),
        "level": level,
        "event": event,
        "message": message,
        "source": "recording-worker"
    });

    if let Ok(mut file) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log_path)
    {
        let _ = writeln!(file, "{payload}");
    }
}

#[cfg(target_os = "windows")]
pub fn capture_system_audio_loopback(
    output_path: PathBuf,
    display_name: String,
    stop_signal: Arc<AtomicBool>,
    log_path: PathBuf,
    started_at_ms: u64,
) -> Result<RecordingCaptureResult, String> {
    use wasapi::{
        initialize_mta, DeviceEnumerator, Direction, SampleType, StreamMode, WaveFormat,
    };

    const SAMPLE_RATE: u32 = 48_000;
    const CHANNELS: u16 = 2;

    let cleanup_path = output_path.clone();
    let run_capture = || -> Result<RecordingCaptureResult, String> {
        initialize_mta().ok().map_err(|error| error.to_string())?;

        let enumerator = DeviceEnumerator::new().map_err(|error| error.to_string())?;
        let device = enumerator
            .get_default_device(&Direction::Render)
            .map_err(|error| error.to_string())?;
        let mut audio_client = device
            .get_iaudioclient()
            .map_err(|error| error.to_string())?;
        let desired_format = WaveFormat::new(
            32,
            32,
            &SampleType::Float,
            SAMPLE_RATE as usize,
            CHANNELS as usize,
            None,
        );

        let mode = StreamMode::EventsShared {
            autoconvert: true,
            buffer_duration_hns: 0,
        };

        audio_client
            .initialize_client(&desired_format, &Direction::Capture, &mode)
            .map_err(|error| error.to_string())?;

        let h_event = audio_client
            .set_get_eventhandle()
            .map_err(|error| error.to_string())?;
        let capture_client = audio_client
            .get_audiocaptureclient()
            .map_err(|error| error.to_string())?;

        let wav_spec = hound::WavSpec {
            channels: CHANNELS,
            sample_rate: SAMPLE_RATE,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut writer =
            hound::WavWriter::create(&output_path, wav_spec).map_err(|error| error.to_string())?;
        let mut sample_queue = VecDeque::<u8>::new();

        audio_client
            .start_stream()
            .map_err(|error| error.to_string())?;
        write_worker_log(
            &log_path,
            "INFO",
            "recording.started",
            &format!("Loopback capture started for {}", output_path.display()),
        );

        loop {
            let new_frames = capture_client
                .get_next_packet_size()
                .map_err(|error| error.to_string())?
                .unwrap_or(0);

            if new_frames > 0 {
                let additional = (new_frames as usize * 4).saturating_sub(
                    sample_queue.capacity().saturating_sub(sample_queue.len()),
                );
                sample_queue.reserve(additional);
                capture_client
                    .read_from_device_to_deque(&mut sample_queue)
                    .map_err(|error| error.to_string())?;
                drain_float32_queue_to_wav(&mut sample_queue, &mut writer)?;
            }

            if stop_signal.load(Ordering::SeqCst) {
                break;
            }

            let _ = h_event.wait_for_event(200);
        }

        for _ in 0..6 {
            let new_frames = capture_client
                .get_next_packet_size()
                .map_err(|error| error.to_string())?
                .unwrap_or(0);
            if new_frames == 0 {
                break;
            }

            capture_client
                .read_from_device_to_deque(&mut sample_queue)
                .map_err(|error| error.to_string())?;
            drain_float32_queue_to_wav(&mut sample_queue, &mut writer)?;
        }

        audio_client
            .stop_stream()
            .map_err(|error| error.to_string())?;
        writer.finalize().map_err(|error| error.to_string())?;

        let bytes_written = fs::metadata(&output_path)
            .map(|metadata| metadata.len())
            .map_err(|error| error.to_string())?;
        let duration_ms = now_ms().saturating_sub(started_at_ms);

        write_worker_log(
            &log_path,
            "INFO",
            "recording.saved",
            &format!("Saved {}", output_path.display()),
        );

        Ok(RecordingCaptureResult {
            output_path,
            display_name,
            duration_ms,
            bytes_written,
            created_at_ms: now_ms(),
        })
    };

    match run_capture() {
        Ok(result) => Ok(result),
        Err(error) => {
            let _ = fs::remove_file(&cleanup_path);
            write_worker_log(
                &log_path,
                "ERROR",
                "recording.failed",
                &format!("Capture failed: {error}"),
            );
            Err(error)
        }
    }
}

#[cfg(target_os = "windows")]
fn drain_float32_queue_to_wav(
    queue: &mut VecDeque<u8>,
    writer: &mut hound::WavWriter<std::io::BufWriter<std::fs::File>>,
) -> Result<(), String> {
    while queue.len() >= 4 {
        let bytes = [
            queue.pop_front().unwrap_or_default(),
            queue.pop_front().unwrap_or_default(),
            queue.pop_front().unwrap_or_default(),
            queue.pop_front().unwrap_or_default(),
        ];
        let sample = f32::from_le_bytes(bytes).clamp(-1.0, 1.0);
        let pcm = (sample * i16::MAX as f32).round() as i16;
        writer.write_sample(pcm).map_err(|error| error.to_string())?;
    }

    Ok(())
}

#[cfg(not(target_os = "windows"))]
pub fn capture_system_audio_loopback(
    _output_path: PathBuf,
    _display_name: String,
    _stop_signal: Arc<AtomicBool>,
    _log_path: PathBuf,
    _started_at_ms: u64,
) -> Result<RecordingCaptureResult, String> {
    Err("System-audio loopback capture is only implemented for Windows right now.".into())
}
