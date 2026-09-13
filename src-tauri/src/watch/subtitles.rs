use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

use serde::Serialize;

use crate::{
    anki::hide_command_window, app_types::AppSettings, runtime_assets::detect_local_ffmpeg,
};

/// A subtitle track inside a video container.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubtitleTrack {
    pub(crate) index: u32,
    pub(crate) language: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) codec: Option<String>,
}

/// The cue text for a video, plus which tracks it has.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SubtitleSource {
    pub(crate) content: String,
    pub(crate) name: String,
    pub(crate) tracks: Vec<SubtitleTrack>,
}

pub(crate) fn ffprobe_path_for(ffmpeg_executable: &str) -> PathBuf {
    let ffmpeg = Path::new(ffmpeg_executable);
    let extension = ffmpeg
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();
    ffmpeg
        .parent()
        .map(|parent| parent.join(format!("ffprobe{extension}")))
        .unwrap_or_else(|| PathBuf::from(format!("ffprobe{extension}")))
}

pub(crate) fn list_subtitle_tracks(
    settings: &AppSettings,
    video_path: &Path,
) -> Vec<SubtitleTrack> {
    let detection = detect_local_ffmpeg(settings);
    let Some(executable_path) = detection.executable_path.clone() else {
        return Vec::new();
    };
    let ffprobe = ffprobe_path_for(&executable_path);

    let mut command = Command::new(&ffprobe);
    hide_command_window(&mut command);
    let output = command
        .arg("-v")
        .arg("error")
        .arg("-select_streams")
        .arg("s")
        .arg("-show_entries")
        .arg("stream=index,codec_name:stream_tags=language,title")
        .arg("-of")
        .arg("json")
        .arg(video_path)
        .output();

    let Ok(output) = output else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    let Ok(parsed) = serde_json::from_slice::<serde_json::Value>(&output.stdout) else {
        return Vec::new();
    };
    let Some(streams) = parsed.get("streams").and_then(|value| value.as_array()) else {
        return Vec::new();
    };

    streams
        .iter()
        .enumerate()
        .map(|(position, stream)| {
            let tags = stream.get("tags");
            SubtitleTrack {
                index: position as u32,
                language: tags
                    .and_then(|tags| tags.get("language"))
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                title: tags
                    .and_then(|tags| tags.get("title"))
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
                codec: stream
                    .get("codec_name")
                    .and_then(|value| value.as_str())
                    .map(str::to_string),
            }
        })
        .collect()
}

pub(super) fn extract_subtitle_args(track_index: u32, input: &str, output: &str) -> Vec<String> {
    vec![
        "-y".into(),
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-i".into(),
        input.into(),
        "-map".into(),
        format!("0:s:{track_index}"),
        output.into(),
    ]
}

fn extract_embedded_track(
    settings: &AppSettings,
    video_path: &Path,
    track_index: u32,
) -> Result<String, String> {
    let detection = detect_local_ffmpeg(settings);
    let executable_path = detection
        .executable_path
        .clone()
        .ok_or_else(|| "FFmpeg is required to read embedded subtitles.".to_string())?;

    let output_path = std::env::temp_dir().join(format!(
        "wonder-of-u-subs-{}-{track_index}.ass",
        std::process::id()
    ));
    let _ = fs::remove_file(&output_path);

    let mut command = Command::new(&executable_path);
    hide_command_window(&mut command);
    command.args(extract_subtitle_args(
        track_index,
        &video_path.display().to_string(),
        &output_path.display().to_string(),
    ));

    let output = command
        .output()
        .map_err(|error| format!("FFmpeg could not read the embedded subtitles: {error}"))?;
    if !output.status.success() {
        let _ = fs::remove_file(&output_path);
        let stderr = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(if stderr.is_empty() {
            "FFmpeg could not read the embedded subtitles.".to_string()
        } else {
            format!("FFmpeg could not read the embedded subtitles: {stderr}")
        });
    }

    let content = crate::text_files::read_external_text(&output_path)
        .map_err(|error| format!("The extracted subtitles could not be read: {error}"))?;
    let _ = fs::remove_file(&output_path);
    Ok(content)
}

/// Resolves the cue text for a watch session.
pub(crate) fn load_subtitle_source(
    settings: &AppSettings,
    video_path: &Path,
    subtitle_path: Option<&Path>,
    track_index: Option<u32>,
) -> Result<SubtitleSource, String> {
    let tracks = list_subtitle_tracks(settings, video_path);

    if let Some(subtitle_path) = subtitle_path {
        let content = fs::read_to_string(subtitle_path)
            .map_err(|error| format!("That subtitle file could not be read: {error}"))?;
        return Ok(SubtitleSource {
            content,
            name: subtitle_path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("subtitles.srt")
                .to_string(),
            tracks,
        });
    }

    let Some(selected) = track_index.or_else(|| tracks.first().map(|track| track.index)) else {
        return Ok(SubtitleSource {
            content: String::new(),
            name: String::new(),
            tracks,
        });
    };

    let content = extract_embedded_track(settings, video_path, selected)?;
    Ok(SubtitleSource {
        content,
        name: format!("embedded-{selected}.ass"),
        tracks,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extract_args_select_the_nth_subtitle_stream() {
        let args = extract_subtitle_args(2, "in.mkv", "out.ass");
        let map = args.iter().position(|arg| arg == "-map").expect("-map present");
        assert_eq!(args[map + 1], "0:s:2");
        let input = args.iter().position(|arg| arg == "-i").expect("-i present");
        assert_eq!(args[input + 1], "in.mkv");
        assert_eq!(args.last().expect("output last"), "out.ass");
    }

    #[test]
    fn ffprobe_is_resolved_beside_ffmpeg() {
        assert_eq!(
            ffprobe_path_for("C:\\assets\\ffmpeg\\bin\\ffmpeg.exe"),
            PathBuf::from("C:\\assets\\ffmpeg\\bin\\ffprobe.exe")
        );
        assert_eq!(
            ffprobe_path_for("/usr/bin/ffmpeg"),
            PathBuf::from("/usr/bin/ffprobe")
        );
    }
}
