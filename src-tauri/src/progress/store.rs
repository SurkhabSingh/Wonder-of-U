use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::app_state::write_file_atomically;

use super::day::{DayKey, DAY_ROLLOVER_HOUR};

/// The record shape this build writes and reads.
///
/// Carried on every row rather than once in the header, so a file written by two builds is
/// still readable row by row instead of being all-or-nothing.
const RECORD_VERSION: u32 = 1;

/// Serialises every read-modify-write of the progress file against itself.
///
/// [`write_file_atomically`] derives one temp path per target, so two writers of the same
/// file share it: the second `File::create` truncates the first's half-written temp, and
/// either one's failure cleanup deletes whatever is sitting there. The store has more than
/// one writer by design — a sample taken on a settings refresh, and later the playback and
/// mining paths — so the file needs what the log file already gives itself.
static WRITE: Mutex<()> = Mutex::new(());

/// The first line of the file. Everything the rows are keyed against lives here.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Header {
    pub(crate) v: u32,
    /// The first day this store existed. A series that could only have been measured from
    /// here is floored at it; a series backed by dated evidence from before it is not.
    pub(crate) first_run_day: DayKey,
    /// The rollover the day keys below were written under, so a future build can tell that
    /// history was bucketed under a different rule rather than silently re-reading it.
    pub(crate) rollover_hour: i64,
    /// When history was last declared lost, if it ever was. Set only where a file that
    /// existed could not be read at all.
    pub(crate) history_lost_at_ms: Option<u64>,
}

/// One item inside a comprehension sample.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SampleItem {
    /// Stable identity across re-transcription and transcript edits.
    pub(crate) key: String,
    /// What the text WAS when it was counted.
    ///
    /// Without it the intersection between two samples is a lie: a transcript is rewritten
    /// in place by a re-transcribe, so the same key can name different words on two dates
    /// and the difference reads as learning. An item whose fingerprint moved is treated
    /// like a new one instead of compared against its own past.
    pub(crate) fingerprint: String,
    pub(crate) content_tokens: u32,
    pub(crate) known_tokens: u32,
}

/// One dated measurement of how much of the library the word list covers.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Sample {
    pub(crate) v: u32,
    pub(crate) kind: String,
    pub(crate) taken_at_ms: u64,
    pub(crate) day: DayKey,
    /// The vocabulary settings this was measured under. Two samples taken under different
    /// builds are not comparable, and saying so is the difference between a trend and a
    /// coincidence.
    pub(crate) build: String,
    pub(crate) items: Vec<SampleItem>,
}

impl Sample {
    pub(crate) fn new(taken_at_ms: u64, day: DayKey, build: String, items: Vec<SampleItem>) -> Self {
        Self {
            v: RECORD_VERSION,
            kind: KIND_SAMPLE.to_string(),
            taken_at_ms,
            day,
            build,
            items,
        }
    }
}

const KIND_SAMPLE: &str = "sample";

/// Everything the file held, and what could not be read of it.
#[derive(Debug, Default)]
pub(crate) struct ProgressStore {
    pub(crate) samples: Vec<Sample>,
    /// Rows this build did not model, kept exactly as they were read.
    ///
    /// Written back untouched so that running an older build over a newer file cannot
    /// delete what the newer one wrote. Rebuilding the file from the parsed model alone
    /// would do exactly that, silently, on the first write after a rollback — which is the
    /// one moment the history matters most.
    pub(crate) passthrough: Vec<String>,
    /// Rows carrying a version this build does not read. Counted so the reader can be told
    /// its answer is incomplete rather than shown a smaller number as if it were whole.
    pub(crate) newer: usize,
    /// Rows that were not JSON at all. Dropped rather than re-emitted: a torn line is
    /// damage, and writing it back would preserve the damage forever.
    pub(crate) damaged: usize,
}

/// What a read of the file found.
#[derive(Debug)]
pub(crate) enum Loaded {
    /// No file. A genuine first run, and the only outcome that means "empty".
    Missing,
    /// A file that exists and whose header could not be read. Never treated as empty —
    /// that is how months of history get overwritten by one bad read.
    Unreadable(String),
    Present {
        header: Header,
        store: ProgressStore,
    },
}

/// Reads the file at `path`.
///
/// The three outcomes are kept apart on purpose. Only a missing file means there is nothing
/// to keep; a file that is present but unreadable is a problem to report, and a caller that
/// collapses the two will rewrite a file it simply failed to open.
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
            store.damaged += 1;
            continue;
        };
        let version = value.get("v").and_then(serde_json::Value::as_u64);
        if version != Some(u64::from(RECORD_VERSION)) {
            // A row from a build that knows more than this one. Kept verbatim, never
            // guessed at: reading it under this build's assumptions is how a number
            // becomes confidently wrong.
            store.newer += 1;
            store.passthrough.push(line.to_string());
            continue;
        }
        match value.get("kind").and_then(serde_json::Value::as_str) {
            Some(KIND_SAMPLE) => match serde_json::from_value::<Sample>(value) {
                Ok(sample) => store.samples.push(sample),
                Err(_) => {
                    store.damaged += 1;
                }
            },
            // This version, a kind this build does not handle. Not damage, so it survives.
            _ => store.passthrough.push(line.to_string()),
        }
    }

    Loaded::Present { header, store }
}

/// Creates the file with a header if it is not there. Never fails a caller.
///
/// Returns whether the store is writable. Nothing downstream may treat this as fatal — the
/// app has to start without a statistics file, so this reports rather than propagates.
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
            // The file is there and cannot be read. Its bytes are moved aside rather than
            // overwritten, so a recovery by hand is still possible, and the replacement
            // says history was lost instead of pretending this is a first run.
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

/// Adds one sample, keeping everything already in the file.
///
/// A read that fails for any reason other than the file being absent aborts without
/// writing. Rewriting after a failed read would replace the whole history with one row,
/// and a transient lock — an indexer, a backup agent — is enough to cause it.
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

    fn sample(taken_at_ms: u64, build: &str) -> Sample {
        Sample::new(
            taken_at_ms,
            key("2026-09-10"),
            build.to_string(),
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

    /// The failure this guards is the expensive one: a file that could not be READ being
    /// rewritten as if it were empty. A lock held for 200 ms by an indexer is enough.
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

    /// The other half of "unreadable": the file exists and the READ itself fails, which is
    /// what an indexer or a backup agent holding it open looks like. A directory standing
    /// where the file should be reproduces that without needing one. Distinct from a bad
    /// header, and the branch that decides it is a different line.
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

    /// Rolling back to an older build must not delete what a newer one wrote. The rows are
    /// unreadable to this build by design, so they are carried rather than modelled.
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
    fn a_torn_row_is_counted_and_dropped_while_the_rest_is_kept() {
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
                assert_eq!(store.samples[1].build, "build-b");
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
    }
}
