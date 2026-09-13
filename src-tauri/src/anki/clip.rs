use std::{path::Path, process::Command};

use super::mine::{hide_command_window, ClipPadding};

/// The clip is capped at 720p, not left at the source resolution.
const MAX_CLIP_WIDTH: u32 = 1280;
const MAX_CLIP_HEIGHT: u32 = 720;

const CLIP_QUALITY: &str = "34";

pub(super) fn clip_ffmpeg_args(
    start_ms: u64,
    end_ms: u64,
    padding: ClipPadding,
    input: &str,
    output: &str,
) -> Vec<String> {
    let start = start_ms.saturating_sub(padding.before_ms);
    let end = end_ms.saturating_add(padding.after_ms);
    vec![
        "-y".into(),
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-ss".into(),
        format!("{}.{:03}", start / 1000, start % 1000),
        "-to".into(),
        format!("{}.{:03}", end / 1000, end % 1000),
        "-i".into(),
        input.into(),
        "-map".into(),
        "0:v:0".into(),
        "-map".into(),
        "0:a:0".into(),
        "-vf".into(),
        format!(
            "scale='min({width},iw)':'min({height},ih)':force_original_aspect_ratio=decrease,\
             scale=trunc(iw/2)*2:trunc(ih/2)*2",
            width = MAX_CLIP_WIDTH,
            height = MAX_CLIP_HEIGHT
        ),
        "-c:v".into(),
        "libvpx-vp9".into(),
        "-crf".into(),
        CLIP_QUALITY.into(),
        "-b:v".into(),
        "0".into(),
        "-deadline".into(),
        "good".into(),
        "-cpu-used".into(),
        "4".into(),
        "-row-mt".into(),
        "1".into(),
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-c:a".into(),
        "libopus".into(),
        "-b:a".into(),
        "96k".into(),
        output.into(),
    ]
}

/// Writes a short video of the line to `output_path`.
pub(super) fn capture_clip(
    ffmpeg_path: &Path,
    video_path: &Path,
    start_ms: u64,
    end_ms: u64,
    padding: ClipPadding,
    output_path: &Path,
) -> Result<(), String> {
    if !video_path.exists() {
        return Err(format!(
            "The source video is no longer at {}.",
            video_path.display()
        ));
    }

    let mut command = Command::new(ffmpeg_path);
    hide_command_window(&mut command);
    command.args(clip_ffmpeg_args(
        start_ms,
        end_ms,
        padding,
        &video_path.display().to_string(),
        &output_path.display().to_string(),
    ));

    let output = command
        .output()
        .map_err(|error| format!("Could not run ffmpeg for the video clip: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "ffmpeg could not cut the video clip: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    match std::fs::metadata(output_path) {
        Ok(metadata) if metadata.len() > 0 => Ok(()),
        _ => Err("ffmpeg produced an empty video clip.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::clip_ffmpeg_args;
    use crate::anki::mine::ClipPadding;

    const NO_PADDING: ClipPadding = ClipPadding {
        before_ms: 0,
        after_ms: 0,
    };

    fn args() -> Vec<String> {
        clip_ffmpeg_args(4_200, 7_500, NO_PADDING, "input.mkv", "clip.webm")
    }

    fn index_of(args: &[String], value: &str) -> usize {
        args.iter().position(|arg| arg == value).unwrap()
    }

    #[test]
    fn seeks_before_the_input_so_a_long_episode_does_not_decode_from_zero() {
        let args = args();
        assert!(index_of(&args, "-ss") < index_of(&args, "-i"));
        assert!(index_of(&args, "-to") < index_of(&args, "-i"));
        assert_eq!(args[index_of(&args, "-ss") + 1], "4.200");
        assert_eq!(args[index_of(&args, "-to") + 1], "7.500");
    }

    #[test]
    fn takes_the_first_video_and_audio_stream_rather_than_letting_ffmpeg_guess() {
        let args = args();
        let maps: Vec<&String> = args
            .iter()
            .enumerate()
            .filter(|(index, _)| *index > 0 && args[index - 1] == "-map")
            .map(|(_, value)| value)
            .collect();
        assert_eq!(maps, vec!["0:v:0", "0:a:0"]);
    }

    #[test]
    fn scaling_only_shrinks_and_lands_on_even_dimensions() {
        let args = args();
        let filter = &args[index_of(&args, "-vf") + 1];
        // `min(cap, iw)` is what makes it downscale-only: a smaller source keeps its size.
        assert!(filter.contains("min(1280,iw)"));
        assert!(filter.contains("min(720,ih)"));
        assert!(filter.contains("force_original_aspect_ratio=decrease"));
        // yuv420p cannot encode an odd width or height.
        assert!(filter.contains("trunc(iw/2)*2:trunc(ih/2)*2"));
    }

    #[test]
    fn encodes_the_open_codecs_the_anki_webview_can_actually_decode() {
        // H.264/AAC render a player that shows nothing: Anki's webview is Chromium without
        // the proprietary decoders. This assertion is the reason the clip is a WebM.
        let args = args();
        assert_eq!(args[index_of(&args, "-c:v") + 1], "libvpx-vp9");
        assert_eq!(args[index_of(&args, "-c:a") + 1], "libopus");
        assert_eq!(args[index_of(&args, "-pix_fmt") + 1], "yuv420p");
    }

    #[test]
    fn vp9_is_told_to_use_constant_quality_rather_than_a_bitrate() {
        // `-crf` alone is a ceiling for VP9; without `-b:v 0` it targets a default bitrate
        // and the quality setting does nothing.
        let args = args();
        assert_eq!(args[index_of(&args, "-b:v") + 1], "0");
    }

    #[test]
    fn sub_second_offsets_keep_three_digits() {
        let args = clip_ffmpeg_args(250, 1_005, NO_PADDING, "input.mkv", "clip.webm");
        assert_eq!(args[index_of(&args, "-ss") + 1], "0.250");
        assert_eq!(args[index_of(&args, "-to") + 1], "1.005");
    }

    #[test]
    fn the_window_is_padded_on_each_side_the_way_the_audio_slicer_pads_it() {
        // The clip and the audio must cover the same span, or the card shows one thing and
        // plays another. Both builders take the padding and apply it themselves.
        let args = clip_ffmpeg_args(
            4_200,
            7_500,
            ClipPadding {
                before_ms: 200,
                after_ms: 500,
            },
            "input.mkv",
            "clip.webm",
        );
        assert_eq!(args[index_of(&args, "-ss") + 1], "4.000");
        assert_eq!(args[index_of(&args, "-to") + 1], "8.000");
    }

    #[test]
    fn padding_cannot_seek_before_the_start_of_the_file() {
        let args = clip_ffmpeg_args(
            100,
            900,
            ClipPadding {
                before_ms: 250,
                after_ms: 0,
            },
            "input.mkv",
            "clip.webm",
        );
        assert_eq!(args[index_of(&args, "-ss") + 1], "0.000");
    }

    #[test]
    fn the_output_path_is_last_so_ffmpeg_reads_it_as_the_destination() {
        let args = args();
        assert_eq!(args.last().unwrap(), "clip.webm");
    }
}
