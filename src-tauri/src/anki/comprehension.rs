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

/// Below this many content words there is nothing to say.
///
/// A share is only as meaningful as the text under it: two transcribed sentences can put
/// the figure anywhere, and a headline that swings twenty points because one clip arrived
/// teaches the reader to distrust it. Reported as "not enough yet" rather than as a number.
const MIN_CONTENT_TOKENS: u32 = 200;

/// Why no sample was taken.
///
/// Each is a state the user can act on, and none of them is a number. The whole reason this
/// is an enum rather than an empty sample is that a sample of nothing sums to `0 / 0`, and
/// the page would render a confident zero for "I could not look".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Skip {
    /// No vocabulary sources chosen, so there is no word list to compare against.
    Unconfigured,
    /// The Japanese dictionary is not installed, so nothing can be split into words.
    NeedsDictionary,
    /// No word list has been built yet.
    Unbuilt,
    /// A word list exists, built under vocabulary settings that have since changed.
    /// Measuring against it would date the answer to a rule the user has left behind.
    Stale,
    /// Nothing Japanese has been transcribed.
    NothingToRead,
    /// Too little text to draw a share from.
    Insufficient,
}

#[derive(Debug)]
pub(crate) enum Sampled {
    Skipped(Skip),
    Taken(Sample),
}

/// One document's worth of counted words.
struct Counted {
    key: String,
    fingerprint: String,
    content_tokens: u32,
    known_tokens: u32,
}

/// Measures how much of the Japanese library the current word list covers.
///
/// Counted in content-word TOKENS, not in distinct words per line. Tokens make the number
/// independent of how a transcript happens to be split into rows — the viewer splits on
/// timed segments and the scanner on newlines, and an unreadable segments sidecar changes
/// the row set without changing a word of the text. A share that moves when the rows move
/// is not a measure of the reader.
///
/// The content-word judgement itself is not re-implemented: `is_content_word` and
/// `normalize_expression` are the same ones the transcript badge uses, so the two can
/// disagree about a total but never about what counts as a word.
pub(crate) fn sample_comprehension<R: Runtime>(
    app: &AppHandle<R>,
    now_ms: u64,
) -> Result<Sampled, String> {
    // Settings under one lock, released before anything slow. The lock discipline here is
    // the module's, not a preference: never hold this across a file read, a tokenize pass,
    // or the known-word index.
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

    // The index is copied out under its lock and the lock released immediately. The
    // tokenize pass below can load a 58 MB dictionary on a cold cache, and holding the
    // index across that would stall a Refresh behind a transcript being opened.
    //
    // Read BEFORE the counting rather than after, so the two states that mean "do not
    // measure" are found before the expensive work rather than paid for and thrown away.
    let (known_words, built_at_ms) = {
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
        (index.words.clone(), index.built_at_ms)
    };

    let japanese = japanese_transcripts(&recordings);
    if japanese.is_empty() {
        return Ok(Sampled::Skipped(Skip::NothingToRead));
    }

    let mut counted = Vec::new();
    let mut unread = 0_u32;
    for (key, path) in japanese {
        let Ok(text) = crate::text_files::read_external_text(Path::new(&path)) else {
            // Counted, never skipped silently. A denominator that quietly shrinks moves the
            // headline with no visible cause, which is the failure this whole feature is
            // built to avoid.
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
    )))
}

/// Takes a sample and stores it. Cannot fail its caller, by signature.
///
/// Returns nothing, so no future contributor can propagate it with `?` out of a command
/// whose own work already succeeded. Every outcome goes to the log instead: a reading is a
/// by-product of the refresh the user asked for, and it must never be able to turn a
/// refresh that worked into an error, nor leave a button disabled behind an unsettled
/// promise.
///
/// The `catch_unwind` is for the same reason and covers the whole body. It is honest about
/// what it does: it protects this caller's thread. It cannot un-poison a lock a panic
/// crossed, which is why the sampler holds no lock across any slow work.
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

/// Every Japanese transcript in the library, as `(item key, path)`.
///
/// `auto` counts only once whisper has said what it heard. An undetected `auto` is not
/// evidence of Japanese, and counting it would put an unknown language in the denominator.
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

/// Identity for one measured document.
///
/// The recording's path rather than the transcript's, so re-transcribing — which rewrites
/// the transcript in place — keeps the item comparable with its own past instead of
/// arriving as a new one.
fn item_key(recording_path: &str, language: &str) -> String {
    format!("{recording_path}|{}", language.trim().to_ascii_lowercase())
}

/// What the text WAS when it was counted.
///
/// Two samples are only comparable over items whose text did not change between them.
/// Transcripts are rewritten in place by a re-transcribe, so without this a change of
/// speech model reads as the reader having learned something.
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

    /// `auto` is a request, not an answer. Counting an undetected one would put an unknown
    /// language into the denominator and quietly lower the share.
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

    /// The key is the RECORDING's path, so a re-transcribe — which rewrites the transcript
    /// file in place — leaves the item comparable with its own past.
    #[test]
    fn the_key_survives_a_retranscribe_and_separates_languages() {
        assert_eq!(item_key("C:/a.wav", "ja"), item_key("C:/a.wav", "JA"));
        assert_ne!(item_key("C:/a.wav", "ja"), item_key("C:/a.wav", "en"));
        assert_ne!(item_key("C:/a.wav", "ja"), item_key("C:/b.wav", "ja"));
    }

    /// The fingerprint is what stops a model change reading as learning. Same text, same
    /// value; one character different, different value.
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
