use serde::Serialize;

use crate::app_types::RecentRecording;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LibraryReport {
    pub(crate) items: usize,
    pub(crate) total_ms: u64,
    pub(crate) items_without_length: usize,
    pub(crate) recorded: usize,
    pub(crate) imported_from_a_link: usize,
    pub(crate) imported_from_a_file: usize,
    pub(crate) unknown_origin: usize,
    pub(crate) transcribed: usize,
    pub(crate) japanese: usize,
    pub(crate) translated: usize,
}

pub(crate) fn summarise(recordings: &[RecentRecording]) -> LibraryReport {
    let mut library = LibraryReport {
        items: recordings.len(),
        total_ms: 0,
        items_without_length: 0,
        recorded: 0,
        imported_from_a_link: 0,
        imported_from_a_file: 0,
        unknown_origin: 0,
        transcribed: 0,
        japanese: 0,
        translated: 0,
    };

    for recording in recordings {
        if recording.duration_ms == 0 {
            library.items_without_length += 1;
        } else {
            library.total_ms += recording.duration_ms;
        }

        match recording.source.as_deref() {
            Some("recording") => library.recorded += 1,
            Some("import") => library.imported_from_a_file += 1,
            Some("youtube") => library.imported_from_a_link += 1,
            _ => library.unknown_origin += 1,
        }

        if !recording.transcripts.is_empty() {
            library.transcribed += 1;
        }
        if recording.transcripts.iter().any(is_japanese) {
            library.japanese += 1;
        }
        if recording.translation_path.is_some() {
            library.translated += 1;
        }
    }

    library
}

fn is_japanese(transcript: &crate::app_types::RecordingTranscript) -> bool {
    let language = transcript.language.trim().to_ascii_lowercase();
    let detected = transcript
        .detected_language
        .as_deref()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    language == "ja" || (language == "auto" && detected == "ja")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_types::RecordingTranscript;

    fn recording(
        created_at_ms: u64,
        duration_ms: u64,
        source: Option<&str>,
        transcripts: Vec<RecordingTranscript>,
        translated: bool,
    ) -> RecentRecording {
        RecentRecording {
            file_name: "a.wav".to_string(),
            file_path: format!("C:/{created_at_ms}.wav"),
            transcript_path: None,
            transcript_language: None,
            transcripts,
            translation_path: translated.then(|| "C:/a.txt".to_string()),
            anki_note_id: None,
            anki_deck_name: None,
            anki_note_type: None,
            anki_pushes: Vec::new(),
            furigana_applied: false,
            audio_deleted: false,
            duration_ms,
            bytes_written: 0,
            created_at_ms,
            source: source.map(str::to_string),
            source_url: None,
            title: None,
        }
    }

    fn ja() -> RecordingTranscript {
        RecordingTranscript {
            language: "ja".to_string(),
            file_path: "C:/a.ja.txt".to_string(),
            detected_language: Some("ja".to_string()),
            segments_path: None,
        }
    }

    #[test]
    fn an_item_with_no_origin_is_counted_as_unknown_and_not_as_recorded() {
        let recordings = vec![
            recording(1_700_000_000_000, 1000, Some("recording"), vec![], false),
            recording(1_700_000_000_000, 1000, Some("youtube"), vec![], false),
            recording(1_700_000_000_000, 1000, Some("import"), vec![], false),
            recording(1_700_000_000_000, 1000, None, vec![], false),
            recording(1_700_000_000_000, 1000, Some("something-new"), vec![], false),
        ];
        let library = summarise(&recordings);
        assert_eq!(library.recorded, 1);
        assert_eq!(library.imported_from_a_link, 1);
        assert_eq!(library.imported_from_a_file, 1);
        assert_eq!(
            library.unknown_origin, 2,
            "an absent origin and one this build does not know are both unknown"
        );
        assert_eq!(
            library.recorded + library.imported_from_a_link + library.imported_from_a_file
                + library.unknown_origin,
            library.items,
            "every item lands in exactly one bucket"
        );
    }

    #[test]
    fn a_total_says_how_many_items_it_could_not_measure() {
        let recordings = vec![
            recording(1_700_000_000_000, 60_000, Some("recording"), vec![], false),
            recording(1_700_000_000_000, 0, Some("recording"), vec![], false),
        ];
        let library = summarise(&recordings);
        assert_eq!(library.total_ms, 60_000);
        assert_eq!(library.items_without_length, 1);
    }

    #[test]
    fn transcribed_japanese_and_translated_are_counted_per_item() {
        let recordings = vec![
            recording(1_700_000_000_000, 1000, None, vec![ja()], true),
            recording(1_700_000_000_000, 1000, None, vec![ja(), ja()], false),
            recording(1_700_000_000_000, 1000, None, vec![], false),
        ];
        let library = summarise(&recordings);
        assert_eq!(library.transcribed, 2);
        assert_eq!(library.japanese, 2, "an item is Japanese once, not once per transcript");
        assert_eq!(library.translated, 1);
    }

    #[test]
    fn an_empty_library_reports_nothing_rather_than_guessing() {
        let library = summarise(&[]);
        assert_eq!(library.items, 0);
        assert_eq!(library.total_ms, 0);
    }
}
