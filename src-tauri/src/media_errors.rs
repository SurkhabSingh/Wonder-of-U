pub(crate) fn stderr_indicates_no_audio(stderr: &str) -> bool {
    let lower = stderr.to_ascii_lowercase();
    ["unable to obtain file audio codec", "matches no streams"]
        .iter()
        .any(|needle| lower.contains(needle))
}

#[cfg(test)]
mod tests {
    use super::stderr_indicates_no_audio;

    #[test]
    fn both_tools_wordings_for_a_missing_audio_track_are_recognised() {
        // yt-dlp's extract-audio postprocessor, verbatim.
        assert!(stderr_indicates_no_audio(
            "ERROR: Postprocessing: WARNING: unable to obtain file audio codec with ffprobe"
        ));
        // ffmpeg with `-map 0:a:0` on a video that has only a video stream, verbatim.
        assert!(stderr_indicates_no_audio(
            "Stream map '' matches no streams.\nTo ignore this, add a trailing '?' to the map."
        ));
        // Matching is case-insensitive: neither tool promises its casing.
        assert!(stderr_indicates_no_audio("STREAM MAP '' MATCHES NO STREAMS."));
    }

    #[test]
    fn other_failures_are_not_mistaken_for_a_missing_audio_track() {
        // Saying "no sound" about any of these would be a lie the user acts on.
        assert!(!stderr_indicates_no_audio(
            "ERROR: [youtube] abc: This video is unavailable"
        ));
        assert!(!stderr_indicates_no_audio(
            "Error opening input file /nope.mkv."
        ));
        assert!(!stderr_indicates_no_audio(
            "Unknown encoder 'libmp3lame'"
        ));
        assert!(!stderr_indicates_no_audio(""));
    }
}
