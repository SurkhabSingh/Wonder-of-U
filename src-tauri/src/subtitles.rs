//! Writing a subtitle file from our own transcript.
//!
//! Crate level rather than under `watch/` or `recording_library/` because both sides use it:
//! the segments come from a transcription and the file is consumed by a watch session.
//!
//! Serialised from the CLEANED segments, never from whisper's `--output-srt`. Whisper's own
//! output carries what cleaning exists to remove — the runaway repeats, the stock
//! hallucination phrases, the out-of-bounds tail, and whisper's unclamped cue ends — so a
//! `-osrt` file would disagree with both the list the app displays and the sidecar it mines
//! from. Two answers to "what does this recording say" is the divergence
//! `store_segments_sidecar` already rewrites the `.txt` to avoid.

use crate::app_types::RecordingSegment;

/// Milliseconds → `HH:MM:SS,mmm`, the SRT timestamp form (comma, not the period VTT uses).
///
/// Hours are not capped: a long recording is rare but a wrapped hour would silently move a
/// cue to the start of the file.
fn format_srt_timestamp(total_ms: u64) -> String {
    let hours = total_ms / 3_600_000;
    let minutes = (total_ms % 3_600_000) / 60_000;
    let seconds = (total_ms % 60_000) / 1_000;
    let millis = total_ms % 1_000;
    format!("{hours:02}:{minutes:02}:{seconds:02},{millis:03}")
}

/// The smallest cue this will emit, for a segment whose end did not survive being made
/// greater than its start. Long enough to be readable, short enough not to overlap the
/// next line at a normal speaking rate.
const MIN_CUE_MS: u64 = 400;

/// Serialise segments into SRT.
///
/// Three properties are enforced here because the reader on the other side is strict in ways
/// that fail silently:
///
/// - A cue whose end is not after its start is dropped by `src/lib/subtitles.ts`, so a
///   degenerate segment would vanish from the very list the user mines from. Such a cue is
///   given `MIN_CUE_MS` instead of being discarded — the sentence is real even when its
///   timing is not.
/// - A blank line inside a cue's text terminates that cue, which shifts every following
///   index by one and corrupts the rest of the file. Interior newlines are collapsed.
/// - Indices are 1-based and contiguous *after* skips, because a gap in the numbering is
///   tolerated by some players and rejected by others.
pub(crate) fn segments_to_srt(segments: &[RecordingSegment]) -> String {
    let mut srt = String::new();
    let mut index = 0usize;

    for segment in segments {
        // Collapsing rather than replacing with a space: SRT allows a multi-line cue, and a
        // transcript line that already contains a break should keep it. Only *blank* lines
        // are the hazard.
        let text = segment
            .text
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
            .collect::<Vec<_>>()
            .join("\n");
        if text.is_empty() {
            continue;
        }

        let end_ms = segment.end_ms.max(segment.start_ms + MIN_CUE_MS);
        index += 1;
        srt.push_str(&format!(
            "{index}\n{} --> {}\n{text}\n\n",
            format_srt_timestamp(segment.start_ms),
            format_srt_timestamp(end_ms)
        ));
    }

    srt
}

#[cfg(test)]
mod tests {
    use super::*;

    fn segment(start_ms: u64, end_ms: u64, text: &str) -> RecordingSegment {
        RecordingSegment {
            text: text.to_string(),
            start_ms,
            end_ms,
        }
    }

    #[test]
    fn timestamps_use_the_srt_comma_form() {
        assert_eq!(format_srt_timestamp(0), "00:00:00,000");
        assert_eq!(format_srt_timestamp(1_920), "00:00:01,920");
        assert_eq!(format_srt_timestamp(31_410), "00:00:31,410");
        // 23m40s, a real episode length.
        assert_eq!(format_srt_timestamp(1_419_970), "00:23:39,970");
        // Past an hour, where a wrapped field would move the cue to the file's start.
        assert_eq!(format_srt_timestamp(3_661_001), "01:01:01,001");
    }

    #[test]
    fn a_transcript_becomes_a_well_formed_srt() {
        let srt = segments_to_srt(&[
            segment(1_920, 4_830, "生まれ変わる今"),
            segment(30_060, 31_410, "大丈夫ですか"),
        ]);

        assert_eq!(
            srt,
            "1\n00:00:01,920 --> 00:00:04,830\n生まれ変わる今\n\n\
             2\n00:00:30,060 --> 00:00:31,410\n大丈夫ですか\n\n"
        );
    }

    /// `src/lib/subtitles.ts` drops a cue whose end is not after its start, so one would
    /// disappear from the mining list rather than merely look wrong.
    #[test]
    fn a_cue_that_would_be_dropped_by_the_reader_is_given_a_readable_length() {
        let srt = segments_to_srt(&[segment(5_000, 5_000, "はい")]);

        assert!(srt.contains("00:00:05,000 --> 00:00:05,400"), "{srt}");
    }

    /// A blank line inside the text would end the cue early and shift every later index.
    #[test]
    fn interior_blank_lines_cannot_split_a_cue() {
        let srt = segments_to_srt(&[
            segment(0, 1_000, "first\n\n  \nsecond"),
            segment(2_000, 3_000, "next"),
        ]);

        assert_eq!(
            srt,
            "1\n00:00:00,000 --> 00:00:01,000\nfirst\nsecond\n\n\
             2\n00:00:02,000 --> 00:00:03,000\nnext\n\n"
        );
    }

    #[test]
    fn empty_text_is_skipped_and_indices_stay_contiguous() {
        let srt = segments_to_srt(&[
            segment(0, 1_000, "one"),
            segment(2_000, 3_000, "   "),
            segment(4_000, 5_000, "two"),
        ]);

        assert!(srt.starts_with("1\n"), "{srt}");
        assert!(srt.contains("\n2\n00:00:04,000"), "no gap in the numbering: {srt}");
        assert_eq!(srt.matches(" --> ").count(), 2);
    }

    #[test]
    fn no_segments_is_an_empty_file_rather_than_a_malformed_one() {
        assert_eq!(segments_to_srt(&[]), "");
    }
}
