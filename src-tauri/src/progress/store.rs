use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::app_state::write_file_atomically;
use crate::app_types::KnownWordsBuild;

use super::day::{DayKey, DAY_ROLLOVER_HOUR};

/// Carried per row, not once in the header, so a file written by two builds stays
/// readable row by row.
const RECORD_VERSION: u32 = 1;

/// Serialises read-modify-write against itself: writers share one temp path, so a second
/// `File::create` would truncate the first's half-written file.
static WRITE: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Header {
    pub(crate) v: u32,
    /// The first day this store existed. Series measured only from here are floored at it.
    pub(crate) first_run_day: DayKey,
    /// The rollover these day keys were written under, so a later build can tell.
    pub(crate) rollover_hour: i64,
    /// Set only where a file that existed could not be read at all.
    pub(crate) history_lost_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SampleItem {
    pub(crate) key: String,
    /// What the text was when counted. A re-transcribe rewrites in place, so without this
    /// the same key names different words on two dates and the difference reads as learning.
    pub(crate) fingerprint: String,
    pub(crate) content_tokens: u32,
    pub(crate) known_tokens: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Sample {
    pub(crate) v: u32,
    pub(crate) kind: String,
    pub(crate) taken_at_ms: u64,
    pub(crate) day: DayKey,
    /// The settings this was measured under. Samples from different builds are not
    /// comparable; stored whole so a reader can say which setting moved.
    pub(crate) build: KnownWordsBuild,
    pub(crate) index_built_at_ms: u64,
    /// In-scope documents that could not be read. Kept on the row, because a denominator
    /// that quietly shrank moves the share with no visible cause.
    #[serde(default)]
    pub(crate) unread_items: u32,
    pub(crate) items: Vec<SampleItem>,
}

impl Sample {
    pub(crate) fn new(
        taken_at_ms: u64,
        day: DayKey,
        build: KnownWordsBuild,
        index_built_at_ms: u64,
        unread_items: u32,
        items: Vec<SampleItem>,
    ) -> Self {
        Self {
            v: RECORD_VERSION,
            kind: KIND_SAMPLE.to_string(),
            taken_at_ms,
            day,
            build,
            index_built_at_ms,
            unread_items,
            items,
        }
    }

    /// Pooled, not averaged per item, so a two-word clip cannot outvote an episode.
    /// Playback count is not a weight: looping one easy sentence must not move it.
    pub(crate) fn totals(&self) -> (u32, u32) {
        self.items.iter().fold((0, 0), |(content, known), item| {
            (content + item.content_tokens, known + item.known_tokens)
        })
    }
}

const KIND_SAMPLE: &str = "sample";

#[derive(Debug, Default)]
pub(crate) struct ProgressStore {
    pub(crate) samples: Vec<Sample>,
    /// Rows this build did not model, written back untouched: rebuilding from the parsed
    /// model alone deletes what a newer build wrote, on the first write after a rollback.
    pub(crate) passthrough: Vec<String>,
    /// Rows from a newer version. Counted, so the reader is told the answer is incomplete.
    pub(crate) newer: usize,
    /// Rows this build could not read. Kept in `passthrough` too, so a write cannot erase
    /// what it could not parse.
    pub(crate) damaged: usize,
}

#[derive(Debug)]
pub(crate) enum Loaded {
    Missing,
    /// Present but unreadable. Never treated as empty: that overwrites months of history.
    Unreadable(String),
    Present {
        header: Header,
        store: ProgressStore,
    },
}

/// Only a missing file means nothing to keep. A caller that collapses missing and
/// unreadable will rewrite a file it merely failed to open.
pub(crate) fn load(path: &Path) -> Loaded {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => return Loaded::Missing,
        Err(error) => return Loaded::Unreadable(error.to_string()),
    };

    let mut lines = contents.lines();
    let Some(header_line) = lines.next() else {
        return Loaded::Unreadable("the progress file is empty".to_string());
    };
    let header: Header = match serde_json::from_str(header_line) {
        Ok(header) => header,
        Err(error) => return Loaded::Unreadable(error.to_string()),
    };

    let mut store = ProgressStore::default();
    for line in lines {
        if line.trim().is_empty() {
            continue;
        }
        let Ok(value) = serde_json::from_str::<serde_json::Value>(line) else {
            // Counted and kept: every write rebuilds from what was parsed, so dropping it
            // here erases it on the next reading taken.
            store.damaged += 1;
            store.passthrough.push(line.to_string());
            continue;
        };
        let version = value.get("v").and_then(serde_json::Value::as_u64);
        if version != Some(u64::from(RECORD_VERSION)) {
            // Kept verbatim: read under this build's assumptions it is confidently wrong.
            store.newer += 1;
            store.passthrough.push(line.to_string());
            continue;
        }
        match value.get("kind").and_then(serde_json::Value::as_str) {
            Some(KIND_SAMPLE) => match serde_json::from_value::<Sample>(value) {
                Ok(sample) => store.samples.push(sample),
                Err(_) => {
                    // The branch that matters most: a row refused by the model is what a
                    // change to `Sample` looks like, and dropping it takes the history.
                    store.damaged += 1;
                    store.passthrough.push(line.to_string());
                }
            },
            // This version, a kind this build does not handle. Not damage, so it survives.
            _ => store.passthrough.push(line.to_string()),
        }
    }

    Loaded::Present { header, store }
}

/// Creates the file if absent and reports whether the store is writable. Never fatal:
/// the app has to start without a statistics file.
pub(crate) fn ensure(path: &Path, today: DayKey, now_ms: u64) -> Result<(), String> {
    let _guard = WRITE.lock();
    match load(path) {
        Loaded::Present { .. } => Ok(()),
        Loaded::Missing => {
            let header = Header {
                v: RECORD_VERSION,
                first_run_day: today,
                rollover_hour: DAY_ROLLOVER_HOUR,
                history_lost_at_ms: None,
            };
            write_all(path, &header, &ProgressStore::default())
        }
        Loaded::Unreadable(reason) => {
            // Moved aside, not overwritten, so a recovery by hand is still possible.
            let moved = path.with_file_name(format!(
                "{}.corrupt-{now_ms}",
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("progress.jsonl")
            ));
            let _ = fs::rename(path, &moved);
            let header = Header {
                v: RECORD_VERSION,
                first_run_day: today,
                rollover_hour: DAY_ROLLOVER_HOUR,
                history_lost_at_ms: Some(now_ms),
            };
            write_all(path, &header, &ProgressStore::default())?;
            Err(reason)
        }
    }
}

/// Adds one sample, keeping the rest. A failed read aborts without writing: rewriting
/// after one replaces the whole history with a single row.
pub(crate) fn append_sample(path: &Path, sample: Sample) -> Result<(), String> {
    let _guard = WRITE.lock();
    let (header, mut store) = match load(path) {
        Loaded::Present { header, store } => (header, store),
        Loaded::Missing => return Err("the progress file is not there".to_string()),
        Loaded::Unreadable(reason) => return Err(reason),
    };
    store.samples.push(sample);
    write_all(path, &header, &store)
}

/// Serialises the header, then every row this build models, then every row it did not.
fn write_all(path: &Path, header: &Header, store: &ProgressStore) -> Result<(), String> {
    let mut out = serde_json::to_string(header).map_err(|error| error.to_string())?;
    for sample in &store.samples {
        out.push('\n');
        out.push_str(&serde_json::to_string(sample).map_err(|error| error.to_string())?);
    }
    for line in &store.passthrough {
        out.push('\n');
        out.push_str(line);
    }
    out.push('\n');
    write_file_atomically(path, &out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(day: &str) -> DayKey {
        serde_json::from_str(&format!("\"{day}\"")).expect("a day key is a string")
    }

    fn build_named(note_type: &str) -> KnownWordsBuild {
        KnownWordsBuild {
            sources: vec![crate::app_types::VocabularySource {
                note_type: note_type.to_string(),
                field: "Word".to_string(),
            }],
            mature_after_days: 21,
        }
    }

    fn sample(taken_at_ms: u64, build: &str) -> Sample {
        Sample::new(
            taken_at_ms,
            key("2026-09-10"),
            build_named(build),
            1_700_000_000_000,
            0,
            vec![SampleItem {
                key: "C:/audio.wav|ja".to_string(),
                fingerprint: "abc123".to_string(),
                content_tokens: 100,
                known_tokens: 58,
            }],
        )
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("wou-progress-{name}"));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("a temp directory");
        dir
    }

    #[test]
    fn a_missing_file_is_a_first_run_and_nothing_else_is() {
        let dir = temp_dir("missing");
        let path = dir.join("progress.jsonl");
        assert!(matches!(load(&path), Loaded::Missing));

        ensure(&path, key("2026-09-10"), 1000).expect("a first run creates the file");
        match load(&path) {
            Loaded::Present { header, store } => {
                assert_eq!(header.first_run_day.as_str(), "2026-09-10");
                assert_eq!(header.rollover_hour, DAY_ROLLOVER_HOUR);
                assert_eq!(header.history_lost_at_ms, None);
                assert!(store.samples.is_empty());
            }
            other => panic!("expected a present store, got {other:?}"),
        }
    }

    /// The expensive failure: an unreadable file rewritten as if it were empty.
    #[test]
    fn an_unreadable_file_is_never_mistaken_for_an_empty_one() {
        let dir = temp_dir("unreadable");
        let path = dir.join("progress.jsonl");
        fs::write(&path, "this is not json\n").expect("write the file");

        assert!(matches!(load(&path), Loaded::Unreadable(_)));
        assert!(
            append_sample(&path, sample(1, "build-a")).is_err(),
            "a write over an unreadable file must refuse"
        );
        assert_eq!(
            fs::read_to_string(&path).expect("read back"),
            "this is not json\n",
            "the bytes must be left exactly as they were"
        );
    }

    /// The other half of unreadable: the read itself fails, not the header. A directory
    /// in the file's place reproduces what a backup agent holding it open would do.
    #[test]
    fn a_file_that_cannot_be_read_at_all_is_not_an_empty_one_either() {
        let dir = temp_dir("unopenable");
        let path = dir.join("progress.jsonl");
        fs::create_dir_all(&path).expect("stand a directory where the file goes");

        match load(&path) {
            Loaded::Unreadable(_) => {}
            other => panic!("a failed read must not read as empty, got {other:?}"),
        }
        assert!(
            append_sample(&path, sample(1, "build-a")).is_err(),
            "a write over a file that cannot be read must refuse"
        );
    }

    /// A rollback must not delete what a newer build wrote, so those rows are carried.
    #[test]
    fn a_row_from_a_newer_build_survives_a_write_by_this_one() {
        let dir = temp_dir("newer");
        let path = dir.join("progress.jsonl");
        ensure(&path, key("2026-09-01"), 1000).expect("create");

        let future = "{\"v\":99,\"kind\":\"whatever-comes-next\",\"payload\":41}";
        let existing = fs::read_to_string(&path).expect("read back");
        fs::write(&path, format!("{}{future}\n", existing)).expect("append a future row");

        match load(&path) {
            Loaded::Present { store, .. } => {
                assert_eq!(store.newer, 1);
                assert_eq!(store.passthrough.len(), 1);
            }
            other => panic!("expected a present store, got {other:?}"),
        }

        append_sample(&path, sample(2, "build-a")).expect("append");

        let after = fs::read_to_string(&path).expect("read back");
        assert!(
            after.contains(future),
            "the newer row must still be there:\n{after}"
        );
        match load(&path) {
            Loaded::Present { store, .. } => {
                assert_eq!(store.samples.len(), 1);
                assert_eq!(store.newer, 1);
            }
            other => panic!("expected a present store, got {other:?}"),
        }
    }

    #[test]
    fn a_torn_row_is_counted_and_kept_while_the_rest_is_read() {
        let dir = temp_dir("damaged");
        let path = dir.join("progress.jsonl");
        ensure(&path, key("2026-09-01"), 1000).expect("create");
        append_sample(&path, sample(1, "build-a")).expect("append");

        let existing = fs::read_to_string(&path).expect("read back");
        fs::write(&path, format!("{existing}{{\"v\":1,\"kind\":\"sam\n")).expect("tear a row");

        match load(&path) {
            Loaded::Present { store, .. } => {
                assert_eq!(store.damaged, 1);
                assert_eq!(store.samples.len(), 1, "the good row survives");
            }
            other => panic!("expected a present store, got {other:?}"),
        }

        // The half that matters: a damaged row left out of the model is erased by the
        // next reading taken, not merely skipped.
        append_sample(&path, sample(2, "build-a")).expect("append after the tear");
        let after = fs::read_to_string(&path).expect("read back");
        assert!(
            after.contains("\"kind\":\"sam"),
            "the torn row was deleted by the next write:
{after}"
        );

        match load(&path) {
            Loaded::Present { store, .. } => {
                assert_eq!(store.damaged, 1, "still counted, not quietly forgotten");
                assert_eq!(store.samples.len(), 2, "and the new reading was stored");
            }
            other => panic!("expected a present store, got {other:?}"),
        }
    }

    /// Well-formed JSON claiming this version and kind, still refused by the model: what
    /// removing a field looks like. Kept, so it costs one reading rather than every one.
    #[test]
    fn a_reading_this_build_cannot_model_is_kept_rather_than_rewritten_away() {
        let dir = temp_dir("unmodellable");
        let path = dir.join("progress.jsonl");
        ensure(&path, key("2026-09-01"), 1000).expect("create");
        append_sample(&path, sample(1, "build-a")).expect("append");

        // Valid JSON, this version, this kind — and missing everything a Sample needs.
        let existing = fs::read_to_string(&path).expect("read back");
        let orphan = format!("{{\"v\":{RECORD_VERSION},\"kind\":\"{KIND_SAMPLE}\",\"takenAtMs\":7}}");
        fs::write(&path, format!("{existing}{orphan}
")).expect("write the orphan");

        match load(&path) {
            Loaded::Present { store, .. } => {
                assert_eq!(store.damaged, 1, "it has to be noticed");
                assert_eq!(store.samples.len(), 1, "and not mistaken for a reading");
            }
            other => panic!("expected a present store, got {other:?}"),
        }

        append_sample(&path, sample(2, "build-a")).expect("append after the orphan");
        let after = fs::read_to_string(&path).expect("read back");
        assert!(
            after.contains(&orphan),
            "the unreadable reading was rewritten away:
{after}"
        );
    }

    #[test]
    fn samples_round_trip_with_their_fingerprints() {
        let dir = temp_dir("round-trip");
        let path = dir.join("progress.jsonl");
        ensure(&path, key("2026-09-01"), 1000).expect("create");
        append_sample(&path, sample(10, "build-a")).expect("first");
        append_sample(&path, sample(20, "build-b")).expect("second");

        match load(&path) {
            Loaded::Present { store, .. } => {
                assert_eq!(store.samples.len(), 2);
                assert_eq!(store.samples[0].taken_at_ms, 10);
                assert_eq!(store.samples[1].build.sources[0].note_type, "build-b");
                assert_eq!(store.samples[0].items[0].fingerprint, "abc123");
                assert_eq!(store.samples[0].items[0].content_tokens, 100);
                assert_eq!(store.samples[0].items[0].known_tokens, 58);
            }
            other => panic!("expected a present store, got {other:?}"),
        }
    }

    /// The rows carry the fingerprint under the name the comparison reads. A rename here
    /// silently turns every item into a new one and every delta into nothing.
    #[test]
    fn the_stored_row_uses_the_names_the_reader_expects() {
        let row = serde_json::to_value(sample(5, "build-a")).expect("serialize");
        assert_eq!(row["v"], serde_json::json!(RECORD_VERSION));
        assert_eq!(row["kind"], serde_json::json!("sample"));
        assert_eq!(row["takenAtMs"], serde_json::json!(5));
        assert_eq!(row["items"][0]["fingerprint"], serde_json::json!("abc123"));
        assert_eq!(row["items"][0]["contentTokens"], serde_json::json!(100));
        assert_eq!(row["items"][0]["knownTokens"], serde_json::json!(58));
        assert_eq!(row["indexBuiltAtMs"], serde_json::json!(1_700_000_000_000_u64));
        assert_eq!(row["unreadItems"], serde_json::json!(0));
    }

    /// Pooled, so one short clip cannot outvote an episode. The check is deliberately
    /// against hand arithmetic rather than against another call to the same function.
    #[test]
    fn totals_pool_across_items_rather_than_averaging_them() {
        let mut pooled = sample(1, "build-a");
        pooled.items = vec![
            SampleItem {
                key: "short".to_string(),
                fingerprint: "a".to_string(),
                content_tokens: 2,
                known_tokens: 2,
            },
            SampleItem {
                key: "long".to_string(),
                fingerprint: "b".to_string(),
                content_tokens: 998,
                known_tokens: 500,
            },
        ];
        // Averaging the two items would give 75%. Pooling gives 502/1000.
        assert_eq!(pooled.totals(), (1000, 502));
    }
}
