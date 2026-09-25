use std::fs;
use std::io::ErrorKind;
use std::path::Path;
use std::sync::Mutex;

use serde::{Deserialize, Serialize};

use crate::app_state::write_file_atomically;
use crate::app_types::KnownWordsBuild;

use super::day::DayKey;

const HISTORY_VERSION: u32 = 1;

// Replacements share one temp path, so a second writer would truncate the first.
static WRITE: Mutex<()> = Mutex::new(());

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WordDay {
    pub(crate) day: DayKey,
    pub(crate) words: u32,
}

/// What Anki's review log says the word list held on each day, and the settings it was
/// replayed under. Another build is not a longer history but a different measurement, so a
/// file built under other settings is dropped whole rather than merged.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WordHistory {
    pub(crate) v: u32,
    pub(crate) read_at_ms: u64,
    pub(crate) read_on: DayKey,
    pub(crate) build: KnownWordsBuild,
    pub(crate) days: Vec<WordDay>,
}

impl WordHistory {
    pub(crate) fn new(
        read_at_ms: u64,
        read_on: DayKey,
        build: KnownWordsBuild,
        days: Vec<(DayKey, u32)>,
    ) -> Self {
        Self {
            v: HISTORY_VERSION,
            read_at_ms,
            read_on,
            build,
            days: days
                .into_iter()
                .map(|(day, words)| WordDay { day, words })
                .collect(),
        }
    }

    /// The days it can speak for, oldest first, and nothing at all under other settings.
    pub(crate) fn under(&self, build: &KnownWordsBuild) -> &[WordDay] {
        if self.build.matches(build) {
            &self.days
        } else {
            &[]
        }
    }
}

pub(crate) fn load(path: &Path) -> Result<Option<WordHistory>, String> {
    let contents = match fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let history: WordHistory =
        serde_json::from_str(&contents).map_err(|error| error.to_string())?;
    Ok((history.v == HISTORY_VERSION).then_some(history))
}

/// Replaces the file, which a replay may only ask for once it agrees with the live list.
pub(crate) fn keep(path: &Path, history: &WordHistory) -> Result<(), String> {
    let _guard = WRITE.lock();
    let contents = serde_json::to_string(history).map_err(|error| error.to_string())?;
    write_file_atomically(path, &contents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_types::VocabularySource;

    fn day(day: &str) -> DayKey {
        serde_json::from_str(&format!("\"{day}\"")).expect("a day key is a string")
    }

    fn build(note_type: &str) -> KnownWordsBuild {
        KnownWordsBuild {
            sources: vec![VocabularySource {
                note_type: note_type.to_string(),
                field: "Word".to_string(),
            }],
            mature_after_days: 21,
        }
    }

    fn history() -> WordHistory {
        WordHistory::new(
            1_789_041_600_000,
            day("2026-09-10"),
            build("Kaishi"),
            vec![(day("2026-09-09"), 2_500), (day("2026-09-10"), 2_507)],
        )
    }

    fn temp_dir(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("wou-vocab-{name}"));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("create a temp dir");
        dir
    }

    #[test]
    fn a_history_survives_the_round_trip() {
        let path = temp_dir("round-trip").join("word_history.json");
        keep(&path, &history()).expect("write");
        let read = load(&path).expect("read").expect("a history");
        assert_eq!(read.days, history().days);
        assert_eq!(read.read_on, day("2026-09-10"));
    }

    #[test]
    fn a_history_from_other_settings_speaks_for_no_day() {
        assert_eq!(history().under(&build("Kaishi")).len(), 2);
        assert!(history().under(&build("Lapis")).is_empty());
    }

    #[test]
    fn a_missing_file_is_no_history_rather_than_an_error() {
        let path = temp_dir("missing").join("word_history.json");
        assert!(load(&path).expect("read").is_none());
    }

    #[test]
    fn a_history_from_another_version_is_not_read() {
        let path = temp_dir("version").join("word_history.json");
        keep(&path, &history()).expect("write");
        let raw = std::fs::read_to_string(&path).expect("read back");
        std::fs::write(&path, raw.replace("\"v\":1", "\"v\":2")).expect("a newer file");
        assert!(load(&path).expect("read").is_none());
    }
}
