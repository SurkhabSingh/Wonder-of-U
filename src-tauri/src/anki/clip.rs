use std::{path::Path, process::Command};

use super::mine::hide_command_window;

/// The clip is capped at 720p, not left at the source resolution.
///
/// Anki syncs its media collection to AnkiWeb and down to phones, so a mined line is not a
/// local file — it is something the user carries around and pays for in sync time. A 4K
/// source cut without re-encoding would also keep whatever codec it was in, and HEVC is
/// exactly what AnkiDroid and AnkiMobile refuse to play.
const MAX_CLIP_WIDTH: u32 = 1280;
const MAX_CLIP_HEIGHT: u32 = 720;

/// Constant-quality rather than a target bitrate: a still shot and a busy action scene are
/// worth different numbers of bits, and CRF spends them where they are needed. 28 lands a
/// few-second line in the low hundreds of kilobytes on typical animation.
const CLIP_QUALITY: &str = "28";

/// Builds the ffmpeg arguments for cutting `[start_ms, end_ms]` out of `input` as a small
/// H.264/AAC MP4. Kept pure, like `screenshot_ffmpeg_args` and `slice_ffmpeg_args`, so the
/// ordering and the filter string can be tested without spawning anything.
///
/// `-ss`/`-to` come before `-i`, matching the audio slicer: ffmpeg seeks by keyframe before
/// it starts decoding, which is what keeps this fast on a long episode.
pub(super) fn clip_ffmpeg_args(start_ms: u64, end_ms: u64, input: &str, output: &str) -> Vec<String> {
    vec![
        "-y".into(),
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-ss".into(),
        format!("{}.{:03}", start_ms / 1000, start_ms % 1000),
        "-to".into(),
        format!("{}.{:03}", end_ms / 1000, end_ms % 1000),
        "-i".into(),
        input.into(),
        // First video and first audio stream. Without this an MKV with several audio tracks
        // or a subtitle stream would have ffmpeg guessing, and a subtitle stream in an MP4
        // container is an error rather than a warning.
        "-map".into(),
        "0:v:0".into(),
        "-map".into(),
        "0:a:0".into(),
        "-vf".into(),
        format!(
            // Two scales on purpose. The first shrinks to fit inside the cap and never
            // enlarges — a 480p source stays 480p. The second rounds to even dimensions,
            // which yuv420p requires; without it an odd height fails the encode outright.
            "scale='min({width},iw)':'min({height},ih)':force_original_aspect_ratio=decrease,\
             scale=trunc(iw/2)*2:trunc(ih/2)*2",
            width = MAX_CLIP_WIDTH,
            height = MAX_CLIP_HEIGHT
        ),
        "-c:v".into(),
        "libx264".into(),
        "-preset".into(),
        // This runs while the user waits for a card, so encode speed is worth more than the
        // few percent of file size a slower preset would save.
        "veryfast".into(),
        "-crf".into(),
        CLIP_QUALITY.into(),
        // The pixel format every Anki client can decode. libx264 would otherwise keep a
        // source's 10-bit or 4:2:2 format and produce a file that plays nowhere but a desktop.
        "-pix_fmt".into(),
        "yuv420p".into(),
        "-c:a".into(),
        "aac".into(),
        "-b:a".into(),
        "96k".into(),
        // Moves the index to the front so a player can start without reading the whole file.
        "-movflags".into(),
        "+faststart".into(),
        output.into(),
    ]
}

/// Writes a short video of the line to `output_path`.
///
/// Every failure is the caller's cue to mine WITHOUT a clip, never to fail the mine — the
/// same contract `capture_screenshot` has. The audio is what a card cannot do without.
pub(super) fn capture_clip(
    ffmpeg_path: &Path,
    video_path: &Path,
    start_ms: u64,
    end_ms: u64,
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
    // ffmpeg can exit 0 having written nothing — seeking past the end of the video is the
    // usual way — so the file itself is the proof, not the exit code.
    match std::fs::metadata(output_path) {
        Ok(metadata) if metadata.len() > 0 => Ok(()),
        _ => Err("ffmpeg produced an empty video clip.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::clip_ffmpeg_args;

    fn args() -> Vec<String> {
        clip_ffmpeg_args(4_200, 7_500, "input.mkv", "clip.mp4")
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
    fn encodes_what_every_anki_client_can_play() {
        let args = args();
        assert_eq!(args[index_of(&args, "-c:v") + 1], "libx264");
        assert_eq!(args[index_of(&args, "-pix_fmt") + 1], "yuv420p");
        assert_eq!(args[index_of(&args, "-c:a") + 1], "aac");
    }

    #[test]
    fn sub_second_offsets_keep_three_digits() {
        let args = clip_ffmpeg_args(250, 1_005, "input.mkv", "clip.mp4");
        assert_eq!(args[index_of(&args, "-ss") + 1], "0.250");
        assert_eq!(args[index_of(&args, "-to") + 1], "1.005");
    }

    #[test]
    fn the_output_path_is_last_so_ffmpeg_reads_it_as_the_destination() {
        let args = args();
        assert_eq!(args.last().unwrap(), "clip.mp4");
    }
}
