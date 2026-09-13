use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::app_state::write_file_atomically;
use crate::app_types::KnownWordsBuild;

use super::day::{DayKey, DAY_ROLLOVER_HOUR};
use super::ledger::{DayTotals, Ledger};

const RECORD_VERSION: u32 = 1;

/// Serialises read-modify-write against itself: writers share one temp path, so a second
/// `File::create` would truncate the first's half-written file.
static WRITE: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Header {
    pub(crate) v: u32,
    pub(crate) first_run_day: DayKey,
    pub(crate) rollover_hour: i64,
    pub(crate) history_lost_at_ms: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SampleItem {
    pub(crate) key: String,
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
    pub(crate) build: KnownWordsBuild,
    pub(crate) index_built_at_ms: u64,
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
    pub(crate) fn totals(&self) -> (u32, u32) {
        self.items.iter().fold((0, 0), |(content, known), item| {
            (content + item.content_tokens, known + item.known_tokens)
        })
    }
}

const KIND_SAMPLE: &str = "sample";
const KIND_DAY: &str = "day";

/// One day's immersion, as it sits on the line.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DayRow {
    v: u32,
    kind: String,
    day: DayKey,
    #[serde(flatten)]
    totals: DayTotals,
}

#[derive(Debug, Default)]
pub(crate) struct ProgressStore {
    pub(crate) samples: Vec<Sample>,
    pub(crate) days: Ledger,
    pub(crate) passthrough: Vec<String>,
    pub(crate) newer: usize,
    pub(crate) damaged: usize,
}

#[derive(Debug)]
pub(crate) enum Loaded {
    Missing,
    Unreadable(String),
    Present {
        header: Header,
        store: ProgressStore,
    },
}

/// Only a missing file means nothing to keep.
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
            store.passthrough.push(line.to_string());
            continue;
        };
        let version = value.get("v").and_then(serde_json::Value::as_u64);
        if version != Some(u64::from(RECORD_VERSION)) {
            store.newer += 1;
            store.passthrough.push(line.to_string());
            continue;
        }
        match value.get("kind").and_then(serde_json::Value::as_str) {
            Some(KIND_SAMPLE) => match serde_json::from_value::<Sample>(value) {
                Ok(sample) => store.samples.push(sample),
                Err(_) => {
                    store.damaged += 1;
                    store.passthrough.push(line.to_string());
                }
            },
            Some(KIND_DAY) => match serde_json::from_value::<DayRow>(value) {
                // Added, not inserted: a file holding two rows for one date must total
                // them rather than let the later one silently win.
                Ok(row) => {
                    let mut one = Ledger::default();
                    one.set(row.day, row.totals);
                    store.days.merge(&one);
                }
                Err(_) => {
                    store.damaged += 1;
                    store.passthrough.push(line.to_string());
                }
            },
            _ => store.passthrough.push(line.to_string()),
        }
    }

    store.days.floor_at(&header.first_run_day);
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

/// Adds `delta` to the days already on disk. Read-modify-write, so two writers of the
/// same day accumulate rather than one overwriting the other.
#[allow(dead_code)]
pub(crate) fn merge_days(path: &Path, delta: &Ledger) -> Result<(), String> {
    let _guard = WRITE.lock();
    let (header, mut store) = match load(path) {
        Loaded::Present { header, store } => (header, store),
        Loaded::Missing => return Err("the progress file is not there".to_string()),
        Loaded::Unreadable(reason) => return Err(reason),
    };
    store.days.merge(delta);
    write_all(path, &header, &store)
}

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

fn write_all(path: &Path, header: &Header, store: &ProgressStore) -> Result<(), String> {
    let mut out = serde_json::to_string(header).map_err(|error| error.to_string())?;
    for sample in &store.samples {
        out.push('\n');
        out.push_str(&serde_json::to_string(sample).map_err(|error| error.to_string())?);
    }
    for (day, totals) in store.days.iter() {
        let row = DayRow {
            v: RECORD_VERSION,
            kind: KIND_DAY.to_string(),
            day: day.clone(),
            totals: *totals,
        };
        out.push('\n');
        out.push_str(&serde_json::to_string(&row).map_err(|error| error.to_string())?);
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

    fn day_delta(day: &str, listening_ms: u64, watching_ms: u64) -> Ledger {
        let mut ledger = Ledger::default();
        ledger.set(
            key(day),
            DayTotals {
                listening_ms,
                watching_ms,
                ..DayTotals::default()
            },
        );
        ledger
    }

    #[test]
    fn a_day_survives_a_write_and_reads_back_the_same() {
        let dir = temp_dir("day-round-trip");
        let path = dir.join("progress.jsonl");
        ensure(&path, key("2026-09-01"), 1000).expect("create");

        merge_days(&path, &day_delta("2026-09-02", 60_000, 30_000)).expect("merge");

        match load(&path) {
            Loaded::Present { store, .. } => {
                let totals = store.days.get(&key("2026-09-02")).expect("the day is there");
                assert_eq!(totals.listening_ms, 60_000);
                assert_eq!(totals.watching_ms, 30_000);
                assert_eq!(store.damaged, 0);
            }
            other => panic!("expected a present store, got {other:?}"),
        }
    }

    #[test]
    fn two_merges_on_one_day_add_rather_than_replace() {
        let dir = temp_dir("day-add");
        let path = dir.join("progress.jsonl");
        ensure(&path, key("2026-09-01"), 1000).expect("create");

        merge_days(&path, &day_delta("2026-09-02", 60_000, 0)).expect("first");
        merge_days(&path, &day_delta("2026-09-02", 30_000, 0)).expect("second");

        match load(&path) {
            Loaded::Present { store, .. } => {
                assert_eq!(store.days.len(), 1, "still one row for the date");
                assert_eq!(
                    store.days.get(&key("2026-09-02")).unwrap().listening_ms,
                    90_000
                );
            }
            other => panic!("expected a present store, got {other:?}"),
        }
    }

    #[test]
    fn a_merge_after_the_rollover_opens_its_own_row() {
        let dir = temp_dir("day-rollover");
        let path = dir.join("progress.jsonl");
        ensure(&path, key("2026-09-01"), 1000).expect("create");

        merge_days(&path, &day_delta("2026-09-02", 60_000, 0)).expect("first");
        merge_days(&path, &day_delta("2026-09-03", 60_000, 0)).expect("second");

        match load(&path) {
            Loaded::Present { store, .. } => assert_eq!(store.days.len(), 2),
            other => panic!("expected a present store, got {other:?}"),
        }
    }

    /// Two rows for one date is what a restore or a second writer can leave behind. The
    /// later one must not be read as the whole truth.
    #[test]
    fn two_rows_for_one_date_are_totalled_rather_than_letting_the_last_win() {
        let dir = temp_dir("day-duplicate");
        let path = dir.join("progress.jsonl");
        ensure(&path, key("2026-09-01"), 1000).expect("create");

        let existing = fs::read_to_string(&path).expect("read back");
        fs::write(
            &path,
            format!(
                "{existing}{{\"v\":1,\"kind\":\"day\",\"day\":\"2026-09-02\",\"listeningMs\":60000}}\n\
                 {{\"v\":1,\"kind\":\"day\",\"day\":\"2026-09-02\",\"listeningMs\":30000}}\n"
            ),
        )
        .expect("write two rows for one date");

        match load(&path) {
            Loaded::Present { store, .. } => {
                assert_eq!(store.days.len(), 1);
                assert_eq!(
                    store.days.get(&key("2026-09-02")).unwrap().listening_ms,
                    90_000,
                    "both rows count"
                );
            }
            other => panic!("expected a present store, got {other:?}"),
        }
    }

    /// The header stamps the first day the store existed; anything earlier is a number
    /// nobody measured.
    #[test]
    fn a_day_before_the_first_run_is_refused() {
        let dir = temp_dir("day-floor");
        let path = dir.join("progress.jsonl");
        ensure(&path, key("2026-09-10"), 1000).expect("create");

        merge_days(&path, &day_delta("2026-09-09", 60_000, 0)).expect("merge");

        match load(&path) {
            Loaded::Present { store, .. } => {
                assert!(store.days.is_empty(), "the earlier day did not land");
            }
            other => panic!("expected a present store, got {other:?}"),
        }
    }

    /// A row can reach the file from a restore or an older build, not only from a merge,
    /// so the read is what has to refuse it.
    #[test]
    fn a_day_row_already_in_the_file_from_before_the_first_run_is_refused_on_read() {
        let dir = temp_dir("day-floor-read");
        let path = dir.join("progress.jsonl");
        ensure(&path, key("2026-09-10"), 1000).expect("create");

        let existing = fs::read_to_string(&path).expect("read back");
        fs::write(
            &path,
            format!(
                "{existing}{{\"v\":1,\"kind\":\"day\",\"day\":\"2026-09-09\",\"listeningMs\":60000}}
"
            ),
        )
        .expect("write a day the store cannot speak for");

        match load(&path) {
            Loaded::Present { store, .. } => {
                assert!(store.days.is_empty(), "the earlier day is not read");
                assert_eq!(store.damaged, 0, "it is refused, not damaged");
            }
            other => panic!("expected a present store, got {other:?}"),
        }
    }

    #[test]
    fn days_and_samples_and_unknown_rows_all_survive_one_write() {
        let dir = temp_dir("day-beside");
        let path = dir.join("progress.jsonl");
        ensure(&path, key("2026-09-01"), 1000).expect("create");
        append_sample(&path, sample(1, "build-a")).expect("sample");

        let existing = fs::read_to_string(&path).expect("read back");
        fs::write(&path, format!("{existing}{{\"v\":2,\"kind\":\"future\"}}\n"))
            .expect("add a newer row");

        merge_days(&path, &day_delta("2026-09-02", 60_000, 0)).expect("merge");

        match load(&path) {
            Loaded::Present { store, .. } => {
                assert_eq!(store.samples.len(), 1, "the sample is still there");
                assert_eq!(store.days.len(), 1, "the day is there");
                assert_eq!(store.newer, 1, "the newer row is still counted");
                assert_eq!(store.damaged, 0);
            }
            other => panic!("expected a present store, got {other:?}"),
        }
    }

    #[test]
    fn a_day_row_this_build_cannot_model_is_counted_and_kept() {
        let dir = temp_dir("day-damaged");
        let path = dir.join("progress.jsonl");
        ensure(&path, key("2026-09-01"), 1000).expect("create");

        let existing = fs::read_to_string(&path).expect("read back");
        let orphan = "{\"v\":1,\"kind\":\"day\",\"listeningMs\":60000}";
        fs::write(&path, format!("{existing}{orphan}\n")).expect("a day row with no date");

        match load(&path) {
            Loaded::Present { store, .. } => {
                assert_eq!(store.damaged, 1);
                assert!(store.days.is_empty());
            }
            other => panic!("expected a present store, got {other:?}"),
        }

        merge_days(&path, &day_delta("2026-09-02", 60_000, 0)).expect("merge");
        let after = fs::read_to_string(&path).expect("read back");
        assert!(after.contains(orphan), "the unreadable row was rewritten away");
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
