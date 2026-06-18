use std::{
    env, fs,
    path::{Path, PathBuf},
    process::Command,
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
}

#[derive(Debug, Clone)]
pub struct WhisperTranscriptionResult {
    pub transcript_path: PathBuf,
}

fn transcript_output_base(audio_path: &Path) -> PathBuf {
    let stem = audio_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("recording");
    let unique_suffix = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis())
        .unwrap_or(0);

    env::temp_dir().join(format!(
        "wonder-of-u-{stem}-{}-{unique_suffix}.transcript",
        std::process::id()
    ))
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

pub fn run_whisper_transcription(
    request: &WhisperTranscriptionRequest,
) -> Result<WhisperTranscriptionResult, String> {
    verify_whisper_cli(&request.cli_path)?;
    verify_whisper_model(&request.model_path)?;

    let output_base = transcript_output_base(&request.audio_path);
    let transcript_path = PathBuf::from(format!("{}.txt", output_base.display()));

    let mut command = Command::new(&request.cli_path);
    hide_command_window(&mut command);
    command
        .arg("--model")
        .arg(&request.model_path)
        .arg("--file")
        .arg(&request.audio_path)
        .arg("--output-txt")
        .arg("--output-file")
        .arg(&output_base)
        .arg("--no-prints");

    if !request.language.trim().is_empty() {
        command.arg("--language").arg(request.language.trim());
    }

    let output = command.output().map_err(|error| error.to_string())?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let details = if !stderr.is_empty() { stderr } else { stdout };
        return Err(if details.is_empty() {
            "whisper-cli failed to transcribe the recording.".into()
        } else {
            details
        });
    }

    if !transcript_path.exists() {
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        let stdout = String::from_utf8_lossy(&output.stdout).trim().to_string();
        let details = [stderr, stdout]
            .into_iter()
            .filter(|value| !value.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
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

    Ok(WhisperTranscriptionResult { transcript_path })
}
