use std::collections::HashSet;
use std::path::Path;
use std::sync::Mutex;

use sha2::{Digest, Sha256};
use tauri::{AppHandle, Manager, Runtime};

use crate::app_types::{KnownWordsBuild, KnownWordsState, RecentRecording, SharedPersistedState};
use crate::progress::day;
use crate::progress::store::{Sample, SampleItem, MIN_CONTENT_TOKENS};
use crate::runtime_assets::find_managed_dictionary_root;
use crate::tokenizer::tokenize_japanese;

use super::known_words::normalize_expression;
use super::sentence_ranking::is_content_word;

/// The transcripts and word list a reading was last attempted for, so a skipped reading is not
/// retried until one of them moves. Held across the attempt, so two cannot run at once.
static ATTEMPTED: Mutex<Option<(String, Option<u64>)>> = Mutex::new(None);

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
    let digest = library_digest(&japanese);

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

    let sample = Sample::new(
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
    );
    Ok(Sampled::Taken(sample.with_source_digest(digest)))
}

pub(crate) fn record_sample<R: Runtime>(app: &AppHandle<R>) {
    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<String, String> {
        let now_ms = crate::app_runtime::now_ms();
        match sample_comprehension(app, now_ms)? {
            Sampled::Skipped(skip) => Ok(format!("{skip:?}")),
            Sampled::Taken(sample) => {
                let items = sample.items_with_words().count();
                let (content, known) = sample.totals();
                crate::progress::add_sample(app, sample)?;
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

/// Takes a reading when the transcripts or the word list moved since the newest one.
pub(crate) fn keep_reading_current<R: Runtime>(app: &AppHandle<R>) {
    let Ok(mut attempted) = ATTEMPTED.try_lock() else {
        return;
    };
    let recordings = {
        let persisted_state = app.state::<SharedPersistedState>();
        let Ok(persisted) = persisted_state.0.lock() else {
            return;
        };
        persisted.recent_recordings.clone()
    };
    let index_built_at_ms = app
        .state::<KnownWordsState>()
        .0
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|index| index.built_at_ms));
    let now = (
        library_digest(&japanese_transcripts(&recordings)),
        index_built_at_ms,
    );
    let newest = crate::progress::newest_reading(app);
    if reading_is_current(newest.as_ref(), &now) || attempted.as_ref() == Some(&now) {
        return;
    }
    crate::app_runtime::log_event(
        app,
        "INFO",
        "progress.reading_due",
        serde_json::json!({ "because": due_because(newest.as_ref(), &now) }),
    );
    *attempted = Some(now);
    record_sample(app);
}

fn reading_is_current(newest: Option<&(Option<String>, u64)>, now: &(String, Option<u64>)) -> bool {
    newest.is_some_and(|(digest, index_built_at_ms)| {
        digest.as_deref() == Some(now.0.as_str()) && Some(*index_built_at_ms) == now.1
    })
}

fn due_because(
    newest: Option<&(Option<String>, u64)>,
    now: &(String, Option<u64>),
) -> &'static str {
    match newest {
        None => "no reading yet",
        Some((_, index_built_at_ms)) if Some(*index_built_at_ms) != now.1 => "word list",
        Some(_) => "transcripts",
    }
}

/// Names, sizes and times, hashed. No text is read, so asking whether a reading is due costs
/// one metadata call per transcript.
fn library_digest(transcripts: &[(String, String)]) -> String {
    let mut lines: Vec<String> = transcripts
        .iter()
        .map(|(key, path)| {
            let stamp = std::fs::metadata(path).map_or_else(
                |_| "missing".to_string(),
                |meta| {
                    let modified = meta
                        .modified()
                        .ok()
                        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |since| since.as_millis());
                    format!("{}:{modified}", meta.len())
                },
            );
            format!("{key}\t{path}\t{stamp}")
        })
        .collect();
    lines.sort_unstable();
    fingerprint(&lines.join("\n"))
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
    use super::{
        due_because, fingerprint, item_key, japanese_transcripts, library_digest,
        reading_is_current,
    };
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
    fn the_digest_moves_with_the_transcripts_and_with_nothing_else() {
        let dir = tempfile::tempdir().expect("a temp directory");
        let first = dir.path().join("a.ja.txt");
        let second = dir.path().join("b.ja.txt");
        std::fs::write(&first, "こんにちは").expect("write");
        std::fs::write(&second, "さようなら").expect("write");
        let entry =
            |key: &str, path: &std::path::Path| (key.to_string(), path.display().to_string());
        let both = [entry("a|ja", &first), entry("b|ja", &second)];
        let digest = library_digest(&both);

        assert_eq!(library_digest(&both), digest, "nothing moved");
        let reversed = [entry("b|ja", &second), entry("a|ja", &first)];
        assert_eq!(library_digest(&reversed), digest, "order is not a change");
        assert_ne!(library_digest(&both[..1]), digest, "a transcript went away");

        std::fs::write(&second, "さようなら、またね").expect("rewrite");
        assert_ne!(library_digest(&both), digest, "a transcript was rewritten");

        std::fs::remove_file(&second).expect("remove");
        let missing = library_digest(&both);
        assert_ne!(
            missing,
            library_digest(&both[..1]),
            "a missing file still counts"
        );
    }

    #[test]
    fn a_reading_is_due_when_the_transcripts_or_the_word_list_moved() {
        let now = ("ab".to_string(), Some(20));
        let newest = |digest: Option<&str>, index: u64| (digest.map(str::to_string), index);

        assert!(reading_is_current(Some(&newest(Some("ab"), 20)), &now));
        assert!(!reading_is_current(Some(&newest(Some("cd"), 20)), &now));
        assert!(!reading_is_current(Some(&newest(Some("ab"), 10)), &now));
        assert!(
            !reading_is_current(Some(&newest(None, 20)), &now),
            "an older reading"
        );
        assert!(!reading_is_current(None, &now), "no reading at all");
        assert!(!reading_is_current(
            Some(&newest(Some("ab"), 20)),
            &("ab".to_string(), None)
        ));

        assert_eq!(due_because(None, &now), "no reading yet");
        assert_eq!(
            due_because(Some(&newest(Some("ab"), 10)), &now),
            "word list"
        );
        assert_eq!(
            due_because(Some(&newest(Some("cd"), 20)), &now),
            "transcripts"
        );
    }
}
