use std::collections::HashSet;
use std::path::Path;

use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, Runtime};

use crate::app_types::{KnownWordsBuild, KnownWordsState, RecentRecording, SharedPersistedState};
use crate::progress::day;
use crate::progress::store::{Sample, SampleItem};
use crate::runtime_assets::find_managed_dictionary_root;
use crate::tokenizer::tokenize_japanese;

use super::known_words::normalize_expression;
use super::sentence_ranking::is_content_word;

const MIN_CONTENT_TOKENS: u32 = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Skip {
    Unconfigured,
    NeedsDictionary,
    Unbuilt,
    Stale,
    NothingToRead,
    Insufficient,
}

#[derive(Debug)]
pub(crate) enum Sampled {
    Skipped(Skip),
    Taken(Sample),
}

struct Counted {
    key: String,
    fingerprint: String,
    content_tokens: u32,
    known_tokens: u32,
}

/// Coverage in content-word tokens, so the share does not move when the row split does.
/// Reuses `is_content_word`, so this and the transcript badge agree on what is a word.
pub(crate) fn sample_comprehension<R: Runtime>(
    app: &AppHandle<R>,
    now_ms: u64,
) -> Result<Sampled, String> {
    let (asset_directory, build, recordings) = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the app settings.".to_string())?;
        (
            persisted.settings.asset_directory.clone(),
            KnownWordsBuild::from_anki_settings(&persisted.settings.anki),
            persisted.recent_recordings.clone(),
        )
    };

    if build.sources.is_empty() {
        return Ok(Sampled::Skipped(Skip::Unconfigured));
    }
    let Some(dictionary_path) = find_managed_dictionary_root(Path::new(&asset_directory)) else {
        return Ok(Sampled::Skipped(Skip::NeedsDictionary));
    };

    let (known_words, built_at_ms, word_count) = {
        let state = app.state::<KnownWordsState>();
        let guard = state
            .0
            .lock()
            .map_err(|_| "Could not read your known-word list.".to_string())?;
        let Some(index) = guard.as_ref() else {
            return Ok(Sampled::Skipped(Skip::Unbuilt));
        };
        if !index.build.matches(&build) {
            return Ok(Sampled::Skipped(Skip::Stale));
        }
        let word_count = u32::try_from(index.words.len()).unwrap_or(u32::MAX);
        (index.words.clone(), index.built_at_ms, word_count)
    };

    let japanese = japanese_transcripts(&recordings);
    if japanese.is_empty() {
        return Ok(Sampled::Skipped(Skip::NothingToRead));
    }

    let mut counted = Vec::new();
    let mut unread = 0_u32;
    for (key, path) in japanese {
        let Ok(text) = crate::text_files::read_external_text(Path::new(&path)) else {
            unread += 1;
            continue;
        };
        match count_document(&key, &text, &dictionary_path, &known_words) {
            Ok(document) => counted.push(document),
            Err(_) => unread += 1,
        }
    }

    let content_tokens: u32 = counted.iter().map(|item| item.content_tokens).sum();
    if content_tokens < MIN_CONTENT_TOKENS {
        return Ok(Sampled::Skipped(Skip::Insufficient));
    }

    Ok(Sampled::Taken(Sample::new(
        now_ms,
        day::today(),
        build,
        built_at_ms,
        unread,
        counted
            .into_iter()
            .map(|item| SampleItem {
                key: item.key,
                fingerprint: item.fingerprint,
                content_tokens: item.content_tokens,
                known_tokens: item.known_tokens,
            })
            .collect(),
        word_count,
    )))
}

pub(crate) fn record_sample<R: Runtime>(app: &AppHandle<R>) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<String, String> {
        let now_ms = crate::app_runtime::now_ms();
        match sample_comprehension(app, now_ms)? {
            Sampled::Skipped(skip) => Ok(format!("{skip:?}")),
            Sampled::Taken(sample) => {
                let items = sample.items.len();
                let (content, known) = sample.totals();
                let path = app
                    .state::<crate::app_types::AppPathsState>()
                    .progress_file
                    .clone();
                crate::progress::store::append_sample(&path, sample)?;
                Ok(format!("taken: {items} items, {known}/{content} words"))
            }
        }
    }));

    match outcome {
        Ok(Ok(detail)) => crate::app_runtime::log_event(
            app,
            "INFO",
            "progress.sampled",
            serde_json::json!({ "outcome": detail }),
        ),
        Ok(Err(message)) => crate::app_runtime::log_event(
            app,
            "WARN",
            "progress.sample_failed",
            serde_json::json!({ "message": message }),
        ),
        Err(_) => crate::app_runtime::log_event(
            app,
            "ERROR",
            "progress.sample_panicked",
            serde_json::json!({}),
        ),
    }
}

fn japanese_transcripts(recordings: &[RecentRecording]) -> Vec<(String, String)> {
    let mut found = Vec::new();
    for recording in recordings {
        for transcript in &recording.transcripts {
            let language = transcript.language.trim().to_ascii_lowercase();
            let detected = transcript
                .detected_language
                .as_deref()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            if language == "ja" || (language == "auto" && detected == "ja") {
                found.push((
                    item_key(&recording.file_path, &transcript.language),
                    transcript.file_path.clone(),
                ));
            }
        }
    }
    found
}

fn item_key(recording_path: &str, language: &str) -> String {
    format!("{recording_path}|{}", language.trim().to_ascii_lowercase())
}

fn fingerprint(text: &str) -> String {
    Sha256::digest(text.as_bytes())
        .iter()
        .take(8)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

fn count_document(
    key: &str,
    text: &str,
    dictionary_path: &Path,
    known_words: &HashSet<String>,
) -> Result<Counted, String> {
    let mut content_tokens = 0_u32;
    let mut known_tokens = 0_u32;
    for token in tokenize_japanese(text, dictionary_path)? {
        if !is_content_word(&token) {
            continue;
        }
        let word = normalize_expression(&token.base_form);
        if word.is_empty() {
            continue;
        }
        content_tokens += 1;
        if known_words.contains(&word) {
            known_tokens += 1;
        }
    }
    Ok(Counted {
        key: key.to_string(),
        fingerprint: fingerprint(text),
        content_tokens,
        known_tokens,
    })
}

#[cfg(test)]
mod tests {
    use super::{fingerprint, item_key, japanese_transcripts, MIN_CONTENT_TOKENS};
    use crate::app_types::{RecentRecording, RecordingTranscript};

    fn transcript(language: &str, detected: Option<&str>, path: &str) -> RecordingTranscript {
        RecordingTranscript {
            language: language.to_string(),
            file_path: path.to_string(),
            detected_language: detected.map(str::to_string),
            segments_path: None,
        }
    }

    fn recording(path: &str, transcripts: Vec<RecordingTranscript>) -> RecentRecording {
        RecentRecording {
            file_name: "a.wav".to_string(),
            file_path: path.to_string(),
            transcript_path: None,
            transcript_language: None,
            transcripts,
            translation_path: None,
            anki_note_id: None,
            anki_deck_name: None,
            anki_note_type: None,
            anki_pushes: Vec::new(),
            furigana_applied: false,
            audio_deleted: false,
            duration_ms: 0,
            bytes_written: 0,
            created_at_ms: 0,
            source: None,
            source_url: None,
            title: None,
        }
    }

    #[test]
    fn only_japanese_transcripts_are_counted() {
        let recordings = vec![recording(
            "C:/a.wav",
            vec![
                transcript("ja", Some("ja"), "C:/a.ja.txt"),
                transcript("en", Some("en"), "C:/a.en.txt"),
                transcript("es", Some("es"), "C:/a.es.txt"),
            ],
        )];
        let found = japanese_transcripts(&recordings);
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].1, "C:/a.ja.txt");
    }

    #[test]
    fn auto_counts_only_once_the_language_is_known() {
        let undetected = vec![recording("C:/a.wav", vec![transcript("auto", None, "C:/a.txt")])];
        assert!(japanese_transcripts(&undetected).is_empty());

        let detected = vec![recording(
            "C:/a.wav",
            vec![transcript("auto", Some("ja"), "C:/a.txt")],
        )];
        assert_eq!(japanese_transcripts(&detected).len(), 1);

        let other = vec![recording(
            "C:/a.wav",
            vec![transcript("auto", Some("es"), "C:/a.txt")],
        )];
        assert!(japanese_transcripts(&other).is_empty());
    }

    #[test]
    fn the_key_survives_a_retranscribe_and_separates_languages() {
        assert_eq!(item_key("C:/a.wav", "ja"), item_key("C:/a.wav", "JA"));
        assert_ne!(item_key("C:/a.wav", "ja"), item_key("C:/a.wav", "en"));
        assert_ne!(item_key("C:/a.wav", "ja"), item_key("C:/b.wav", "ja"));
    }

    #[test]
    fn the_fingerprint_moves_only_when_the_text_moves() {
        assert_eq!(fingerprint("こんにちは"), fingerprint("こんにちは"));
        assert_ne!(fingerprint("こんにちは"), fingerprint("こんにちわ"));
        assert_ne!(fingerprint(""), fingerprint(" "));
        assert_eq!(fingerprint("x").len(), 16);
    }

    #[test]
    fn the_floor_is_the_documented_two_hundred() {
        assert_eq!(MIN_CONTENT_TOKENS, 200);
    }
}
