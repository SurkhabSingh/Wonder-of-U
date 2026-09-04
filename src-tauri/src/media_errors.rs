//! Reading the media tools' stderr for conditions the app can name.
//!
//! ffmpeg and yt-dlp both report a source with no audio track as an internal
//! malfunction, and neither of their wordings is something to show a user. This is
//! the one place that recognises the condition, so every path that maps audio
//! answers it the same way instead of each one growing its own guess.

/// True when the tool failed because the source carried no audio stream at all.
///
/// Two wordings, one condition:
///
/// * yt-dlp's extract-audio postprocessor prints `unable to obtain file audio codec
///   with ffprobe`. That reads as a broken ffprobe and is not one — ffprobe ran,
///   exited 0, and listed the streams it found; there was simply no
///   `codec_type=audio` among them.
/// * ffmpeg asked for `-map 0:a:0` prints `Stream map '' matches no streams.` before
///   it opens the output at all.
///
/// A silent video is a normal thing to be handed — plenty of clips on the web carry
/// no audio track — so it is answered in the caller's own words rather than dumped
/// as tool jargon. The caller supplies the sentence, because what cannot be done
/// (imported, mined, transcribed) differs; only the recognition is shared.
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
