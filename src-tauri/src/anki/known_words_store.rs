//! Persistence for the known-word index: its own file beside `state.json`, loaded
//! at startup so ranking works on launch, re-written on every successful Refresh.
//!
//! Stored as plain text — one word per line under a single header line — rather
//! than as a serialized blob. The index this replaces stalled on being empty after
//! a restart with no way to see why; a file you can open, count and diff answers
//! "did it save my words?" in one look, and that is the whole reason for the format.
//!
//! The original index was memory-only, on the reasoning that a cache of a
//! collection edited since is worse than nothing. That reasoning was half right:
//! the answer to staleness is to SHOW the age and flag a source change, not to
//! throw the index away every launch and leave ranking silently unavailable until
//! the user remembers to rebuild. This module is that showing-and-flagging layer.
//!
//! It lives in its own file for blast-radius reasons: the word list can run to tens
//! of thousands of entries, and a parse failure here must never reach settings or
//! the recording library the way a bad `state.json` would.

use std::{fs, io::ErrorKind, path::Path};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_runtime::log_event,
    app_state::write_file_atomically,
    app_types::{
        AppPathsState, KnownWordIndex, KnownWordsBuild, KnownWordsSnapshot, KnownWordsState,
        SharedPersistedState,
    },
};

/// The header line: everything about the index that is not a word.
///
/// JSON on one line rather than prose, because the note type and field names are
/// arbitrary user text — a human-readable `Mining → Expression` cannot be read back
/// unambiguously once someone's field is called `Word → Reading`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct KnownWordsHeader {
    built_at_ms: u64,
    #[serde(flatten)]
    build: KnownWordsBuild,
}

/// The index as read off disk, before it becomes a `KnownWordIndex`.
#[derive(Debug, Clone)]
struct PersistedKnownWords {
    words: Vec<String>,
    header: KnownWordsHeader,
}

/// What reading `known_words.txt` found. `Missing` is a genuine first run and is
/// silent; anything present-but-unusable is `Corrupt` and gets logged, never
/// crashed on.
enum KnownWordsFile {
    Missing,
    Corrupt(String),
    Loaded(PersistedKnownWords),
}

/// Splits the file into its header and its words.
///
/// **The first line is the header and every other line is one word — decided by
/// position, not by content.** No comment marker, no sentinel: a vocabulary field
/// can hold any text at all, so any character reserved for markup is a character
/// that silently eats a real word the day someone's deck contains it. Position
/// cannot be spoofed by the data.
///
/// Blank lines are skipped rather than kept, since a trailing newline is not a
/// word; `\r` is trimmed so a file opened and saved in a Windows editor still
/// loads. Words are otherwise taken verbatim — `normalize_expression` has already
/// collapsed every whitespace run to a single space, so a word can never span two
/// lines and the format cannot lose one.
fn parse_known_words(raw: &str) -> Result<PersistedKnownWords, String> {
    let mut lines = raw.lines();
    let header_line = lines
        .next()
        .ok_or_else(|| "The file is empty.".to_string())?;
    let header: KnownWordsHeader = serde_json::from_str(header_line.trim_end_matches('\r'))
        .map_err(|error| format!("The first line is not a readable header. {error}"))?;

    Ok(PersistedKnownWords {
        words: lines
            .map(|line| line.trim_end_matches('\r').to_string())
            .filter(|word| !word.is_empty())
            .collect(),
        header,
    })
}

fn read_known_words_file(path: &Path) -> KnownWordsFile {
    match fs::read_to_string(path) {
        Ok(raw) => match parse_known_words(&raw) {
            Ok(persisted) => KnownWordsFile::Loaded(persisted),
            Err(reason) => KnownWordsFile::Corrupt(reason),
        },
        Err(error) if error.kind() == ErrorKind::NotFound => KnownWordsFile::Missing,
        // Present but unreadable (locked, permissions). Not a first run, but not
        // usable either — treat it as needing a Refresh, same as a parse failure.
        Err(error) => KnownWordsFile::Corrupt(error.to_string()),
    }
}

fn index_from_persisted(persisted: PersistedKnownWords) -> KnownWordIndex {
    KnownWordIndex {
        words: persisted.words.into_iter().collect(),
        built_at_ms: persisted.header.built_at_ms,
        build: persisted.header.build,
    }
}

/// Writes the index out atomically. Reuses `write_file_atomically` — the same
/// temp+fsync+rename `state.json` relies on — so a crash mid-write can never leave
/// a truncated file the next launch would choke on.
///
/// Words are sorted, not written in `HashSet` order. The point of a file you can
/// open is a file you can compare: unsorted, two rebuilds of an identical
/// collection produce two completely different files, and "what changed since
/// yesterday" stops being answerable.
pub(super) fn persist_index(
    known_words_file: &Path,
    index: &KnownWordIndex,
) -> Result<(), String> {
    let header = KnownWordsHeader {
        built_at_ms: index.built_at_ms,
        build: index.build.clone(),
    };
    let mut contents = serde_json::to_string(&header).map_err(|error| error.to_string())?;

    let mut words: Vec<&String> = index.words.iter().collect();
    words.sort_unstable();
    for word in words {
        contents.push('\n');
        contents.push_str(word);
    }
    contents.push('\n');

    write_file_atomically(known_words_file, &contents)
}

/// Removes the cache file, ignoring a file that is already gone.
///
/// Called when a Refresh lands on "nothing configured" or "nothing found": the
/// in-memory index is cleared to `None`, and the file has to go with it, or the
/// next launch would restore an index for a selection the user has abandoned and
/// quietly resume ranking against it.
pub(super) fn remove_known_words_file(known_words_file: &Path) -> Result<(), String> {
    match fs::remove_file(known_words_file) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.to_string()),
    }
}

fn ready_message(word_count: usize, source_count: usize) -> String {
    format!(
        "{word_count} words from {source_count} vocabulary {}.",
        if source_count == 1 { "source" } else { "sources" }
    )
}

/// Describes the index against the settings as they stand — the single place the
/// "is this index still for what I use?" question is answered, shared by startup
/// and by every app snapshot the frontend receives.
///
/// The order of checks matters. No configured sources is "off" first, whatever is
/// cached: a stale index for decks the user has removed must not present itself as
/// a working list. Then a present index either matches the current build (`ready`)
/// or does not (`stale` — still shown with its age and count so the user sees what
/// would be replaced). An absent index with sources set is `unbuilt`: a Refresh
/// will fill it, and it is the state a corrupt file degrades to.
fn snapshot_for_index(
    index: Option<&KnownWordIndex>,
    current: &KnownWordsBuild,
) -> KnownWordsSnapshot {
    if current.sources.is_empty() {
        return KnownWordsSnapshot {
            status: "unconfigured".into(),
            message: "Add a vocabulary note type and field to build the list.".into(),
            word_count: 0,
            built_at_ms: None,
        };
    }

    match index {
        None => KnownWordsSnapshot {
            status: "unbuilt".into(),
            message: "Refresh to build your known-word list from Anki.".into(),
            word_count: 0,
            built_at_ms: None,
        },
        Some(index) if index.build.matches(current) => KnownWordsSnapshot {
            status: "ready".into(),
            message: ready_message(index.words.len(), current.sources.len()),
            word_count: index.words.len(),
            built_at_ms: Some(index.built_at_ms),
        },
        Some(index) => KnownWordsSnapshot {
            status: "stale".into(),
            // Deliberately covers both a source change and a threshold change
            // without naming which: either way the list on disk was built under a
            // rule that is no longer the user's, and the action is the same.
            message: "Your vocabulary settings changed since this list was built — Refresh to update."
                .into(),
            word_count: index.words.len(),
            built_at_ms: Some(index.built_at_ms),
        },
    }
}

fn current_build<R: Runtime>(app: &AppHandle<R>) -> KnownWordsBuild {
    app.state::<SharedPersistedState>()
        .0
        .lock()
        .map(|persisted| KnownWordsBuild::from_anki_settings(&persisted.settings.anki))
        .unwrap_or_default()
}

/// The known-word snapshot for the current state, for the startup/emit bootstrap.
///
/// Takes the build it judges against as an argument rather than reading it, so
/// `build_app_bootstrap` — which already holds the settings — does not lock them a
/// second time. Only `KnownWordsState` is locked here, and never across anything
/// blocking.
pub(crate) fn known_words_snapshot_from_state<R: Runtime>(
    app: &AppHandle<R>,
    current: &KnownWordsBuild,
) -> KnownWordsSnapshot {
    let state = app.state::<KnownWordsState>();
    let guard = state.0.lock().ok();
    let index = guard.as_ref().and_then(|index| index.as_ref());
    snapshot_for_index(index, current)
}

/// Restores the index from disk into `KnownWordsState` at startup.
///
/// Best-effort and non-fatal by contract: a missing file is a silent first run, a
/// corrupt one is logged and left as "no index" (a Refresh rebuilds it), and
/// neither ever touches settings or the recording library. When the user has no
/// sources configured the feature is off, so a leftover file is not loaded into
/// memory — it must not resurrect ranking the user turned off.
pub(crate) fn restore_known_words_index<R: Runtime>(app: &AppHandle<R>) {
    if current_build(app).sources.is_empty() {
        return;
    }

    let known_words_file = app.state::<AppPathsState>().inner().known_words_file.clone();
    match read_known_words_file(&known_words_file) {
        KnownWordsFile::Missing => {}
        KnownWordsFile::Corrupt(reason) => {
            log_event(
                app,
                "WARN",
                "known_words.unreadable",
                serde_json::json!({
                    "file": known_words_file.display().to_string(),
                    "message": format!(
                        "The saved known-word list could not be read; it will be rebuilt on the next Refresh. {reason}"
                    )
                }),
            );
        }
        KnownWordsFile::Loaded(persisted) => {
            let index = index_from_persisted(persisted);
            if let Ok(mut guard) = app.state::<KnownWordsState>().0.lock() {
                *guard = Some(index);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        index_from_persisted, parse_known_words, persist_index, read_known_words_file,
        snapshot_for_index, KnownWordsFile,
    };
    use crate::app_types::{KnownWordIndex, KnownWordsBuild, VocabularySource};
    use std::collections::HashSet;

    fn source(note_type: &str, field: &str) -> VocabularySource {
        VocabularySource {
            note_type: note_type.into(),
            field: field.into(),
        }
    }

    fn build(sources: Vec<VocabularySource>) -> KnownWordsBuild {
        KnownWordsBuild {
            sources,
            mature_after_days: 21,
        }
    }

    fn index(words: &[&str], built_at_ms: u64, build: KnownWordsBuild) -> KnownWordIndex {
        KnownWordIndex {
            words: words.iter().map(|word| word.to_string()).collect(),
            built_at_ms,
            build,
        }
    }

    #[test]
    fn a_persisted_index_round_trips_words_timestamp_and_build() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_words.txt");
        let built_under = build(vec![
            source("Kaishi 1.5k", "Expression"),
            source("Lapis", "Expression"),
        ]);
        let original = index(&["見る", "食べる", "本"], 1_700_000_000_000, built_under.clone());

        persist_index(&file, &original).unwrap();

        let KnownWordsFile::Loaded(persisted) = read_known_words_file(&file) else {
            panic!("a freshly written file must read back as Loaded");
        };
        let restored = index_from_persisted(persisted);

        assert_eq!(
            restored.words,
            HashSet::from(["見る".to_string(), "食べる".to_string(), "本".to_string()])
        );
        assert_eq!(restored.built_at_ms, 1_700_000_000_000);
        assert_eq!(restored.build, built_under);
    }

    /// The format is the feature: someone must be able to open this file and see
    /// their words. A blob would round-trip just as well and answer nothing.
    #[test]
    fn the_file_is_one_word_per_line_under_a_single_header() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_words.txt");
        persist_index(
            &file,
            &index(
                &["食べる", "見る", "本"],
                1_700_000_000_000,
                build(vec![source("Kaishi 1.5k", "Expression")]),
            ),
        )
        .unwrap();

        let raw = std::fs::read_to_string(&file).unwrap();
        let lines: Vec<&str> = raw.lines().collect();
        assert_eq!(lines.len(), 4, "one header line and three words");
        assert!(lines[0].starts_with('{'), "the header is line 1");
        // Sorted, so two rebuilds of an unchanged collection produce an identical
        // file and "what changed" stays answerable by diff.
        assert_eq!(&lines[1..], ["本", "見る", "食べる"]);
    }

    /// A word is whatever is on the line. Nothing about its content can promote it
    /// to markup — there is no character a vocabulary field is guaranteed to avoid.
    #[test]
    fn a_word_that_looks_like_markup_survives_the_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_words.txt");
        let awkward = ["#hashtag", r#"{"json": 1}"#, "// comment", "-- dash"];
        persist_index(
            &file,
            &index(&awkward, 1, build(vec![source("Mining", "Expression")])),
        )
        .unwrap();

        let KnownWordsFile::Loaded(persisted) = read_known_words_file(&file) else {
            panic!("the file must read back");
        };
        let restored = index_from_persisted(persisted);
        assert_eq!(restored.words.len(), 4);
        for word in awkward {
            assert!(restored.words.contains(word), "{word} was swallowed");
        }
    }

    #[test]
    fn a_missing_file_is_a_silent_first_run() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_words.txt");
        assert!(matches!(
            read_known_words_file(&file),
            KnownWordsFile::Missing
        ));
    }

    #[test]
    fn a_file_without_a_readable_header_is_reported_not_crashed_on() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_words.txt");
        // Hand-edited down to just the words: unusable, because there is nothing
        // left to say what settings built it.
        std::fs::write(&file, "見る\n食べる\n").unwrap();

        assert!(matches!(
            read_known_words_file(&file),
            KnownWordsFile::Corrupt(_)
        ));
    }

    #[test]
    fn an_empty_file_is_corrupt_rather_than_an_empty_index() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("known_words.txt");
        std::fs::write(&file, "").unwrap();

        // "No words" and "no header" are different answers, and only the second is
        // a file we cannot judge. Treating this as an empty index would leave
        // ranking quietly reporting every word as new.
        assert!(matches!(
            read_known_words_file(&file),
            KnownWordsFile::Corrupt(_)
        ));
    }

    #[test]
    fn a_header_without_a_recorded_build_still_loads() {
        // A hand-written file: the build defaults to empty with a zero threshold,
        // which matches no real setting, so the index reads as `stale` and is
        // rebuilt rather than being trusted.
        let persisted = parse_known_words("{\"builtAtMs\":42}\n本\n").unwrap();
        let restored = index_from_persisted(persisted);
        assert!(restored.build.sources.is_empty());
        assert_eq!(restored.build.mature_after_days, 0);
        assert_eq!(restored.built_at_ms, 42);
    }

    #[test]
    fn a_file_saved_by_a_windows_editor_still_loads() {
        // CRLF and a trailing blank line are what an editor leaves behind, and this
        // file exists to be opened in one.
        let persisted =
            parse_known_words("{\"builtAtMs\":42,\"matureAfterDays\":21}\r\n見る\r\n本\r\n\r\n")
                .unwrap();
        let restored = index_from_persisted(persisted);
        assert_eq!(restored.build.mature_after_days, 21);
        assert_eq!(
            restored.words,
            HashSet::from(["見る".to_string(), "本".to_string()])
        );
    }

    #[test]
    fn a_build_matches_itself_regardless_of_row_order() {
        let a = build(vec![
            source("Kaishi 1.5k", "Expression"),
            source("Lapis", "Word"),
        ]);
        let reordered = build(vec![
            source("Lapis", "Word"),
            source("Kaishi 1.5k", "Expression"),
        ]);
        let changed_field = build(vec![
            source("Kaishi 1.5k", "Reading"),
            source("Lapis", "Word"),
        ]);
        let dropped = build(vec![source("Kaishi 1.5k", "Expression")]);

        assert!(a.matches(&reordered));
        assert!(!a.matches(&changed_field));
        assert!(!a.matches(&dropped));
    }

    /// The threshold decides which words are in the list just as much as the
    /// sources do, so changing it has to invalidate the index. Before it was part
    /// of the build, an index built at 21 days went on answering at 7.
    #[test]
    fn a_changed_threshold_does_not_match_the_same_sources() {
        let sources = vec![source("Kaishi 1.5k", "Expression")];
        let at_21 = build(sources.clone());
        let at_7 = KnownWordsBuild {
            sources,
            mature_after_days: 7,
        };

        assert!(!at_21.matches(&at_7));
    }

    #[test]
    fn snapshot_reports_a_ready_index_with_its_real_age() {
        let current = build(vec![source("Kaishi 1.5k", "Expression")]);
        let built = index(&["見る", "本"], 1_700_000_000_000, current.clone());

        let snapshot = snapshot_for_index(Some(&built), &current);

        assert_eq!(snapshot.status, "ready");
        assert_eq!(snapshot.word_count, 2);
        assert_eq!(snapshot.built_at_ms, Some(1_700_000_000_000));
    }

    #[test]
    fn snapshot_flags_a_source_mismatch_without_hiding_the_count() {
        let built_from = build(vec![source("Kaishi 1.5k", "Expression")]);
        let now_configured = build(vec![source("Lapis", "Expression")]);
        let built = index(&["見る", "本", "食べる"], 1_700_000_000_000, built_from);

        let snapshot = snapshot_for_index(Some(&built), &now_configured);

        assert_eq!(snapshot.status, "stale");
        // The count and age still show — the user should see what would be replaced.
        assert_eq!(snapshot.word_count, 3);
        assert_eq!(snapshot.built_at_ms, Some(1_700_000_000_000));
        assert!(snapshot.message.contains("Refresh"));
    }

    #[test]
    fn snapshot_flags_a_threshold_change_the_same_way() {
        let sources = vec![source("Kaishi 1.5k", "Expression")];
        let built = index(&["見る", "本"], 1_700_000_000_000, build(sources.clone()));
        let now_configured = KnownWordsBuild {
            sources,
            mature_after_days: 90,
        };

        let snapshot = snapshot_for_index(Some(&built), &now_configured);

        assert_eq!(snapshot.status, "stale");
        assert!(snapshot.message.contains("Refresh"));
    }

    #[test]
    fn snapshot_with_sources_but_no_index_asks_for_a_refresh() {
        // The state a corrupt or missing file leaves once sources are configured.
        let snapshot = snapshot_for_index(None, &build(vec![source("Kaishi 1.5k", "Expression")]));
        assert_eq!(snapshot.status, "unbuilt");
        assert_eq!(snapshot.built_at_ms, None);
    }

    #[test]
    fn snapshot_with_no_sources_is_off_even_when_an_index_is_cached() {
        // Sources removed: the feature is off and a leftover index must not present
        // itself as a working list.
        let orphan = index(
            &["見る"],
            1_700_000_000_000,
            build(vec![source("Old", "Expression")]),
        );
        let snapshot = snapshot_for_index(Some(&orphan), &build(Vec::new()));
        assert_eq!(snapshot.status, "unconfigured");
        assert_eq!(snapshot.word_count, 0);
        assert_eq!(snapshot.built_at_ms, None);
    }
}
