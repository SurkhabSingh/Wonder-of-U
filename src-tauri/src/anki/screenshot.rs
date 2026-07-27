use std::path::Path;
use std::process::Command;

use super::mine::hide_command_window;

/// Longest edge of a mined screenshot, in pixels. Anki syncs its media collection, and a
/// sentence card is read on a phone — a full 1080p frame per card would bloat the
/// collection for detail nobody reads. Downscaling only: `force_original_aspect_ratio`
/// plus the `min()` guard leaves an already-smaller frame alone rather than upscaling it
/// into blur.
const MAX_SCREENSHOT_EDGE: u32 = 640;

/// JPEG quality for `-q:v` (2–31, lower is better). 4 is visually clean on video stills
/// while staying a fraction of the size of the 2 the audio path uses for clips.
const SCREENSHOT_QUALITY: &str = "4";

/// Builds the ffmpeg arguments that grab a single frame at `at_ms`.
///
/// `-ss` goes BEFORE `-i` deliberately: that seeks by keyframe before decoding, which
/// turns a frame grab from a whole-file decode into a near-instant one. The cost is
/// landing on the nearest preceding keyframe rather than the exact millisecond, which for
/// a still illustrating a sentence is not a cost at all — and mining has to stay
/// interactive, since the user is pressing Enter on one row after another.
///
/// Kept pure so the profile can be asserted without spawning ffmpeg, matching
/// `slice_ffmpeg_args` in mine.rs.
pub(super) fn screenshot_ffmpeg_args(at_ms: u64, input: &str, output: &str) -> Vec<String> {
    vec![
        "-y".into(),
        "-nostdin".into(),
        "-hide_banner".into(),
        "-loglevel".into(),
        "error".into(),
        "-ss".into(),
        format!("{}.{:03}", at_ms / 1000, at_ms % 1000),
        "-i".into(),
        input.into(),
        // One frame, and never a stream of them.
        "-frames:v".into(),
        "1".into(),
        // Drop any audio/subtitle stream so a container with several does not confuse the
        // single-frame output.
        "-an".into(),
        "-sn".into(),
        "-vf".into(),
        format!(
            "scale='min({max},iw)':'min({max},ih)':force_original_aspect_ratio=decrease"
        ,
            max = MAX_SCREENSHOT_EDGE
        ),
        "-q:v".into(),
        SCREENSHOT_QUALITY.into(),
        output.into(),
    ]
}

/// Writes a single still from `video_path` at `at_ms` to `output_path`.
///
/// Every failure is the caller's cue to mine WITHOUT a picture, never to fail the mine:
/// the source video is a reference to the user's own file and may have been moved,
/// renamed, or deleted since the import, and a missing screenshot must not cost them the
/// card.
pub(super) fn capture_screenshot(
    ffmpeg_path: &Path,
    video_path: &Path,
    at_ms: u64,
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
    command.args(screenshot_ffmpeg_args(
        at_ms,
        &video_path.display().to_string(),
        &output_path.display().to_string(),
    ));

    let output = command
        .output()
        .map_err(|error| format!("Could not run ffmpeg for the screenshot: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "ffmpeg could not take a screenshot: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    // ffmpeg can exit 0 having written nothing — seeking past the end of the video is the
    // usual way — so the file itself is the proof, not the exit code.
    match std::fs::metadata(output_path) {
        Ok(metadata) if metadata.len() > 0 => Ok(()),
        _ => Err("ffmpeg produced an empty screenshot.".into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_args_seek_before_input_and_take_one_frame() {
        let args = screenshot_ffmpeg_args(65_400, "in.mp4", "out.jpg");
        let seek = args.iter().position(|arg| arg == "-ss").expect("-ss present");
        let input = args.iter().position(|arg| arg == "-i").expect("-i present");
        assert!(
            seek < input,
            "-ss must precede -i so ffmpeg keyframe-seeks instead of decoding the whole file"
        );
        assert_eq!(args[seek + 1], "65.400");
        assert_eq!(args[input + 1], "in.mp4");
        let frames = args
            .iter()
            .position(|arg| arg == "-frames:v")
            .expect("-frames:v present");
        assert_eq!(args[frames + 1], "1");
        assert_eq!(args.last().expect("output last"), "out.jpg");
    }

    #[test]
    fn screenshot_timestamps_pad_sub_second_offsets() {
        // A bare "4" would be read as four seconds, not four milliseconds.
        assert_eq!(screenshot_ffmpeg_args(4, "in.mp4", "out.jpg")[6], "0.004");
        assert_eq!(screenshot_ffmpeg_args(0, "in.mp4", "out.jpg")[6], "0.000");
        assert_eq!(screenshot_ffmpeg_args(1_000, "in.mp4", "out.jpg")[6], "1.000");
    }

    #[test]
    fn screenshot_scaling_only_shrinks() {
        let args = screenshot_ffmpeg_args(0, "in.mp4", "out.jpg");
        let filter = args
            .iter()
            .position(|arg| arg == "-vf")
            .map(|index| args[index + 1].clone())
            .expect("-vf present");
        // The min() against the input dimensions is what keeps a small frame from being
        // upscaled into blur.
        assert!(filter.contains("min(640,iw)"), "got {filter}");
        assert!(filter.contains("min(640,ih)"), "got {filter}");
        assert!(filter.contains("force_original_aspect_ratio=decrease"));
    }
}
