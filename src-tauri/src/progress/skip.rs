use std::sync::Mutex;

use serde::Serialize;
use tauri::{AppHandle, Emitter, Runtime};

use crate::app_types::KnownWordsBuild;

use super::store::MIN_CONTENT_TOKENS;

/// What a page with no reading says when nothing stops a first one.
pub(crate) const NO_READING_YET: &str =
    "Nothing has been measured yet. Refresh your word list to take a first reading.";

/// Why an attempt took no reading.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub(crate) enum Skip {
    Unconfigured,
    NeedsDictionary,
    Unbuilt,
    Stale,
    NothingToRead,
    Insufficient { words: u32 },
    Unreadable,
}

/// What stops a first reading before any text is read. The reading and the page both ask this,
/// so the page cannot show a reason whose cause has been put right.
pub(crate) fn ready<D, L>(
    settings: &KnownWordsBuild,
    dictionary: Option<D>,
    list: Option<(&KnownWordsBuild, L)>,
    japanese: bool,
) -> Result<(D, L), Skip> {
    if settings.sources.is_empty() {
        return Err(Skip::Unconfigured);
    }
    let Some(dictionary) = dictionary else {
        return Err(Skip::NeedsDictionary);
    };
    let Some((built, list)) = list else {
        return Err(Skip::Unbuilt);
    };
    if !built.matches(settings) {
        return Err(Skip::Stale);
    }
    if !japanese {
        return Err(Skip::NothingToRead);
    }
    Ok((dictionary, list))
}

pub(crate) fn too_little(words: u32, read: usize, unread: u32) -> Option<Skip> {
    if words >= MIN_CONTENT_TOKENS {
        None
    } else if read == 0 && unread > 0 {
        Some(Skip::Unreadable)
    } else {
        Some(Skip::Insufficient { words })
    }
}

// Only what reading the text found is kept; the rest is worked out again whenever it is asked.
static FOUND: Mutex<Option<Skip>> = Mutex::new(None);

/// Keeps what the newest attempt found in the text, and tells an open page when that changed.
pub(crate) fn note<R: Runtime>(app: &AppHandle<R>, found: Skip) {
    let changed = FOUND
        .lock()
        .is_ok_and(|mut kept| remember(&mut kept, Some(found)));
    if changed {
        let _ = app.emit(crate::app_config::PROGRESS_EVENT, ());
    }
}

pub(crate) fn clear() {
    if let Ok(mut kept) = FOUND.lock() {
        remember(&mut kept, None);
    }
}

pub(crate) fn found() -> Option<Skip> {
    FOUND.lock().ok().and_then(|kept| *kept)
}

fn remember(kept: &mut Option<Skip>, found: Option<Skip>) -> bool {
    std::mem::replace(kept, found) != found
}

impl Skip {
    pub(crate) fn found_in_text(self) -> bool {
        matches!(self, Skip::Insufficient { .. } | Skip::Unreadable)
    }

    pub(crate) fn message(self) -> String {
        match self {
            Skip::Unconfigured => "Add a vocabulary note type and field in Study Picks, then refresh your word list to take a first reading.".to_string(),
            Skip::NeedsDictionary => "Download the Japanese dictionary in Study Picks, then refresh your word list to take a first reading.".to_string(),
            Skip::Unbuilt => NO_READING_YET.to_string(),
            Skip::Stale => "Your vocabulary settings changed after your word list was built. Refresh it to take a first reading.".to_string(),
            Skip::NothingToRead => "A reading measures your Japanese transcripts, and there are none yet. Transcribe something in Japanese to take a first one.".to_string(),
            Skip::Insufficient { words } => format!(
                "A first reading needs {MIN_CONTENT_TOKENS} words of Japanese transcripts, and yours hold {words}. Transcribe a little more to take it."
            ),
            Skip::Unreadable => "None of your Japanese transcripts could be read, so no reading could be taken.".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_types::VocabularySource;

    const ALL: [Skip; 7] = [
        Skip::Unconfigured,
        Skip::NeedsDictionary,
        Skip::Unbuilt,
        Skip::Stale,
        Skip::NothingToRead,
        Skip::Insufficient { words: 57 },
        Skip::Unreadable,
    ];

    fn build(note_type: &str) -> KnownWordsBuild {
        KnownWordsBuild {
            sources: vec![VocabularySource {
                note_type: note_type.to_string(),
                field: "Word".to_string(),
            }],
            mature_after_days: 21,
        }
    }

    #[test]
    fn a_first_reading_is_stopped_by_the_first_thing_missing() {
        let settings = build("Kaishi");
        let other = build("Lapis");
        let check = |settings: &KnownWordsBuild, dictionary: bool, list: Option<&KnownWordsBuild>, japanese: bool| {
            ready(settings, dictionary.then_some(()), list.map(|built| (built, ())), japanese).err()
        };
        let empty = KnownWordsBuild::default();
        assert_eq!(check(&empty, false, None, false), Some(Skip::Unconfigured));
        assert_eq!(check(&settings, false, None, false), Some(Skip::NeedsDictionary));
        assert_eq!(check(&settings, true, None, false), Some(Skip::Unbuilt));
        assert_eq!(check(&settings, true, Some(&other), false), Some(Skip::Stale));
        assert_eq!(check(&settings, true, Some(&settings), false), Some(Skip::NothingToRead));
        assert_eq!(check(&settings, true, Some(&settings), true), None, "nothing stops it");
    }

    #[test]
    fn too_little_text_tells_unreadable_files_from_short_ones() {
        assert_eq!(too_little(MIN_CONTENT_TOKENS, 3, 0), None);
        assert_eq!(too_little(57, 2, 1), Some(Skip::Insufficient { words: 57 }));
        assert_eq!(too_little(0, 0, 3), Some(Skip::Unreadable), "every file failed to read");
        assert_eq!(too_little(0, 1, 0), Some(Skip::Insufficient { words: 0 }));
    }

    #[test]
    fn only_a_change_is_announced() {
        let mut kept = None;
        assert!(remember(&mut kept, Some(Skip::Unreadable)));
        assert!(!remember(&mut kept, Some(Skip::Unreadable)), "the same reason again");
        assert!(remember(&mut kept, Some(Skip::Insufficient { words: 57 })));
        assert!(remember(&mut kept, None), "a reading taken clears it");
        assert_eq!(kept, None);
    }

    #[test]
    fn only_what_the_text_turned_up_is_kept_between_attempts() {
        let kept: Vec<Skip> = ALL.into_iter().filter(|skip| skip.found_in_text()).collect();
        assert_eq!(kept, [Skip::Insufficient { words: 57 }, Skip::Unreadable]);
    }

    #[test]
    fn each_reason_says_what_takes_the_first_reading() {
        let says = [
            "vocabulary note type",
            "Japanese dictionary",
            "Refresh your word list",
            "settings changed",
            "Transcribe something in Japanese",
            "yours hold 57",
            "could be read",
        ];
        for (skip, says) in ALL.into_iter().zip(says) {
            assert!(skip.message().contains(says), "{skip:?}: {}", skip.message());
        }
        let message = Skip::Insufficient { words: 57 }.message();
        assert!(message.contains(&MIN_CONTENT_TOKENS.to_string()), "{message}");
    }

    /// The page picks its button by these names, and a renamed one would leave it blank.
    #[test]
    fn every_reason_reaches_the_page_under_a_name_it_knows() {
        let types = std::fs::read_to_string(
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/types.ts"),
        )
        .expect("src/types.ts is readable");
        let types = types.replace("\r\n", "\n");
        let declared = types
            .split("export type SkipReason")
            .nth(1)
            .and_then(|rest| rest.split("\n\n").next())
            .expect("types.ts declares SkipReason");
        for skip in ALL {
            let wire = serde_json::to_value(skip).expect("serialise");
            let kind = wire["kind"].as_str().expect("a kind");
            assert!(declared.contains(&format!("\"{kind}\"")), "{kind} is not in SkipReason");
        }
        assert_eq!(
            serde_json::to_value(Skip::Insufficient { words: 57 }).expect("serialise"),
            serde_json::json!({ "kind": "insufficient", "words": 57 })
        );
    }
}
