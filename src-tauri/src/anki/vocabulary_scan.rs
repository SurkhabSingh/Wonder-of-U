use std::{collections::HashSet, path::Path};

use tauri::{AppHandle, Manager, Runtime};

use crate::app_runtime::log_event;

use crate::{
    app_types::{
        KnownWordsBuild, SharedPersistedState, VocabularySource, VocabularySuggestion,
        VocabularySuggestions,
    },
    runtime_assets::find_managed_dictionary_root,
    tokenizer::tokenize_japanese,
};

use super::{
    client::{
        anki_connect_health_check, anki_connect_request, anki_find_notes, anki_notes_info,
        anki_offline_message, json_array, json_string_array,
    },
    known_words::normalize_expression,
};

const MIN_MATURE_NOTES: usize = 20;

const SAMPLE_SIZE: usize = 60;

const MIN_FILL_PERCENT: usize = 90;

const MIN_JAPANESE_CHARS_PERCENT: usize = 70;

const MIN_JAPANESE_VALUES_PERCENT: usize = 70;

const MIN_WORD_PERCENT: usize = 80;

const MAX_WORD_CHARS: usize = 24;

/// How many real values ride along with each suggestion for the user to judge by.
const SHOWN_SAMPLES: usize = 3;

const WORD_FIELD_HINTS: [&str; 9] = [
    "word",
    "expression",
    "vocabulary",
    "vocab",
    "term",
    "headword",
    "単語",
    "語彙",
    "front",
];

fn is_japanese(character: char) -> bool {
    matches!(character,
        '\u{3040}'..='\u{309f}'   // hiragana
        | '\u{30a0}'..='\u{30ff}' // katakana, and the ー it lengthens with
        | '\u{4e00}'..='\u{9fff}' // kanji
        | '\u{3005}'              // 々, the repeat mark
        | '\u{ff66}'..='\u{ff9f}' // half-width katakana
    )
}

fn japanese_percent(value: &str) -> usize {
    let total = value.chars().filter(|c| !c.is_whitespace()).count();
    if total == 0 {
        return 0;
    }
    100 * value.chars().filter(|c| is_japanese(*c)).count() / total
}

fn percent(part: usize, whole: usize) -> usize {
    if whole == 0 {
        0
    } else {
        100 * part / whole
    }
}

/// How often this field's contents are exactly one word the dictionary knows.
fn single_known_word_percent(values: &[String], dictionary_path: &Path) -> usize {
    let mut words = 0;
    for value in values {
        if value.chars().count() > MAX_WORD_CHARS {
            continue;
        }
        if matches!(
            tokenize_japanese(value, dictionary_path).as_deref(),
            Ok([token]) if token.known_to_dictionary
        ) {
            words += 1;
        }
    }
    percent(words, values.len())
}

fn field_name_is_a_hint(field_name: &str) -> bool {
    let lowered = field_name.trim().to_lowercase();
    WORD_FIELD_HINTS.contains(&lowered.as_str())
}

/// Reads one field across the sampled notes, as the index itself would read it.
fn sampled_field_values(notes: &[serde_json::Value], field_name: &str) -> Vec<String> {
    notes
        .iter()
        .map(|note| {
            note.get("fields")
                .and_then(|fields| fields.get(field_name))
                .and_then(|field| field.get("value"))
                .and_then(|value| value.as_str())
                .map(normalize_expression)
                .unwrap_or_default()
        })
        .collect()
}

fn field_names(notes: &[serde_json::Value]) -> Vec<String> {
    notes
        .first()
        .and_then(|note| note.get("fields"))
        .and_then(|fields| fields.as_object())
        .map(|fields| fields.keys().cloned().collect())
        .unwrap_or_default()
}

/// One field's case for being the word field, or `None` if it fails a test.
struct FieldScore {
    field: String,
    word_percent: usize,
    is_hinted: bool,
    samples: Vec<String>,
}

fn score_field(
    notes: &[serde_json::Value],
    field_name: &str,
    dictionary_path: &Path,
) -> Option<FieldScore> {
    let values = sampled_field_values(notes, field_name);
    let filled: Vec<String> = values.iter().filter(|v| !v.is_empty()).cloned().collect();
    if percent(filled.len(), values.len()) < MIN_FILL_PERCENT {
        return None;
    }

    let japanese: Vec<String> = filled
        .into_iter()
        .filter(|value| japanese_percent(value) >= MIN_JAPANESE_CHARS_PERCENT)
        .collect();
    if percent(japanese.len(), values.len()) < MIN_JAPANESE_VALUES_PERCENT {
        return None;
    }

    let word_percent = single_known_word_percent(&japanese, dictionary_path);
    if word_percent < MIN_WORD_PERCENT {
        return None;
    }

    Some(FieldScore {
        field: field_name.to_string(),
        word_percent,
        is_hinted: field_name_is_a_hint(field_name),
        samples: japanese.into_iter().take(SHOWN_SAMPLES).collect(),
    })
}

/// Spreads the sample across the whole note type rather than taking the first N.
fn spread_sample(mut note_ids: Vec<i64>) -> Vec<i64> {
    note_ids.sort_unstable();
    if note_ids.len() <= SAMPLE_SIZE {
        return note_ids;
    }
    let stride = note_ids.len() / SAMPLE_SIZE;
    note_ids
        .into_iter()
        .step_by(stride.max(1))
        .take(SAMPLE_SIZE)
        .collect()
}

/// Quotes a note type into a search term, restricted to mature cards.
fn mature_notes_query(note_type: &str, mature_after_days: u32) -> String {
    let mut escaped = String::with_capacity(note_type.len());
    for character in note_type.chars() {
        if matches!(character, '\\' | '"' | '*' | '_' | ':') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    format!("note:\"{escaped}\" prop:ivl>={mature_after_days}")
}

/// `Ok(None)` is "no suggestion for this note type" — the ordinary answer for most of them.
/// `Err` is a query that FAILED, which used to be the same `None` and so was invisible.
fn examine_note_type(
    note_type: &str,
    mature_after_days: u32,
    dictionary_path: &Path,
    already_configured: &HashSet<(String, String)>,
) -> Result<Option<VocabularySuggestion>, String> {
    let mature = anki_find_notes(&mature_notes_query(note_type, mature_after_days))
        .map_err(|error| format!("findNotes: {error}"))?;
    if mature.len() < MIN_MATURE_NOTES {
        return Ok(None);
    }

    let sample = spread_sample(mature.clone());
    let reply = anki_notes_info(&sample).map_err(|error| format!("notesInfo: {error}"))?;
    let notes = json_array(&reply, "note list")?;

    let best = field_names(notes)
        .into_iter()
        .enumerate()
        .filter_map(|(index, field)| {
            score_field(notes, &field, dictionary_path).map(|score| (index, score))
        })
        .max_by_key(|(index, score)| {
            (
                score.word_percent,
                score.is_hinted,
                std::cmp::Reverse(*index),
            )
        })
        .map(|(_, score)| score);
    let Some(best) = best else {
        return Ok(None);
    };

    Ok(Some(VocabularySuggestion {
        already_added: already_configured
            .contains(&(note_type.to_string(), best.field.clone())),
        note_type: note_type.to_string(),
        field: best.field,
        mature_note_count: mature.len(),
        samples: best.samples,
    }))
}

fn scan_settings<R: Runtime>(app: &AppHandle<R>) -> Result<(u32, String, Vec<VocabularySource>), String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not read the Anki settings.".to_string())?;
    let build = KnownWordsBuild::from_anki_settings(&persisted.settings.anki);
    Ok((
        build.mature_after_days,
        persisted.settings.asset_directory.clone(),
        build.sources,
    ))
}

/// Looks through the collection for note types that hold vocabulary.
pub(crate) fn scan_vocabulary_sources_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<VocabularySuggestions, String> {
    let (mature_after_days, asset_directory, configured) = scan_settings(app)?;

    let Some(dictionary_path) = find_managed_dictionary_root(Path::new(&asset_directory)) else {
        return Ok(VocabularySuggestions {
            status: "needsDictionary".into(),
            message: "Download the Japanese dictionary above first — finding your vocabulary decks means reading the words on your cards.".into(),
            suggestions: Vec::new(),
        });
    };

    if let Err(error) = anki_connect_health_check() {
        return Ok(VocabularySuggestions {
            status: "offline".into(),
            message: anki_offline_message(&error),
            suggestions: Vec::new(),
        });
    }

    let already_configured: HashSet<(String, String)> = configured
        .into_iter()
        .map(|source| (source.note_type, source.field))
        .collect();

    let note_types = json_string_array(
        anki_connect_request("modelNames", serde_json::json!({}))?,
        "note type list",
    )?;
    let examined = note_types.len();

    let mut suggestions: Vec<VocabularySuggestion> = note_types
        .iter()
        .filter_map(|note_type| {
            match examine_note_type(
                note_type,
                mature_after_days,
                &dictionary_path,
                &already_configured,
            ) {
                Ok(suggestion) => suggestion,
                Err(message) => {
                    log_event(
                        app,
                        "WARN",
                        "anki.note_type_scan_failed",
                        serde_json::json!({ "noteType": note_type, "message": message }),
                    );
                    None
                }
            }
        })
        .collect();
    suggestions.sort_by_key(|suggestion| std::cmp::Reverse(suggestion.mature_note_count));

    let found = suggestions.len();
    Ok(VocabularySuggestions {
        status: if found == 0 { "none".into() } else { "ready".into() },
        message: if found == 0 {
            format!(
                "Looked through {examined} note types and none of them read like a vocabulary deck. Add one by hand below."
            )
        } else {
            format!(
                "Looked through {examined} note types; {found} {} like vocabulary.",
                if found == 1 { "reads" } else { "read" }
            )
        },
        suggestions,
    })
}

#[cfg(test)]
mod tests {
    use super::{
        field_name_is_a_hint, field_names, japanese_percent, mature_notes_query,
        sampled_field_values, spread_sample, SAMPLE_SIZE,
    };

    #[test]
    fn japanese_text_is_told_apart_from_everything_else() {
        assert_eq!(japanese_percent("修理"), 100);
        assert_eq!(japanese_percent("こんにちは"), 100);
        assert_eq!(japanese_percent("コーヒー"), 100);
        assert_eq!(japanese_percent("14"), 0);
        assert_eq!(japanese_percent("9999999"), 0);
        assert_eq!(japanese_percent("konnichiha"), 0);
        assert_eq!(japanese_percent("walking stick"), 0);
        assert!(japanese_percent("修理 (repair)") < 70);
    }

    #[test]
    fn an_empty_value_is_not_japanese_rather_than_dividing_by_zero() {
        assert_eq!(japanese_percent(""), 0);
        assert_eq!(japanese_percent("   "), 0);
    }

    #[test]
    fn the_note_type_query_matches_the_index_and_cannot_break_out() {
        assert_eq!(
            mature_notes_query("Kaishi 1.5k", 21),
            "note:\"Kaishi 1.5k\" prop:ivl>=21"
        );
        assert_eq!(mature_notes_query("Core_2k", 30), "note:\"Core\\_2k\" prop:ivl>=30");
        assert_eq!(mature_notes_query("a\\b", 21), "note:\"a\\\\b\" prop:ivl>=21");
    }

    #[test]
    fn field_name_hints_are_matched_loosely_but_not_wrongly() {
        assert!(field_name_is_a_hint("Word"));
        assert!(field_name_is_a_hint("expression"));
        assert!(field_name_is_a_hint(" Vocabulary "));
        assert!(field_name_is_a_hint("単語"));
        assert!(!field_name_is_a_hint("Word Reading"));
        assert!(!field_name_is_a_hint("Sentence"));
    }

    #[test]
    fn the_sample_is_spread_across_the_whole_note_type() {
        let ids: Vec<i64> = (0..6000).collect();
        let sample = spread_sample(ids);

        assert_eq!(sample.len(), SAMPLE_SIZE);
        assert_eq!(sample[0], 0);
        assert!(
            *sample.last().unwrap() > 5000,
            "sample stopped at {:?}",
            sample.last()
        );
    }

    #[test]
    fn a_small_note_type_is_sampled_whole_and_stays_sorted() {
        assert_eq!(spread_sample(vec![3, 1, 2]), vec![1, 2, 3]);
        assert!(spread_sample(Vec::new()).is_empty());
    }

    fn notes_json() -> Vec<serde_json::Value> {
        vec![
            serde_json::json!({ "fields": { "Word": { "value": "見[み]る", "order": 0 },
                                            "Sentence": { "value": "本を読む。", "order": 1 } } }),
            serde_json::json!({ "fields": { "Word": { "value": "<b>食べる</b>", "order": 0 },
                                            "Sentence": { "value": "", "order": 1 } } }),
        ]
    }

    #[test]
    fn field_values_are_read_the_way_the_index_reads_them() {
        assert_eq!(
            sampled_field_values(&notes_json(), "Word"),
            vec!["見る".to_string(), "食べる".to_string()]
        );
    }

    #[test]
    fn a_missing_field_reads_as_empty_rather_than_shifting_the_others() {
        let values = sampled_field_values(&notes_json(), "Sentence");
        assert_eq!(values.len(), 2);
        assert_eq!(values[1], "");
        assert!(sampled_field_values(&notes_json(), "Nonexistent")
            .iter()
            .all(String::is_empty));
    }

    #[test]
    fn field_names_come_from_the_notes_themselves() {
        let mut names = field_names(&notes_json());
        names.sort();
        assert_eq!(names, vec!["Sentence".to_string(), "Word".to_string()]);
        assert!(field_names(&[]).is_empty());
    }

    /// Prints how IPADIC analyses specific values.
    #[test]
    #[ignore = "requires an installed dictionary"]
    fn explain_tokenization() {
        use crate::tokenizer::tokenize_japanese;
        use std::path::PathBuf;

        let dictionary = PathBuf::from(
            std::env::var("WONDER_OF_U_IPADIC_DIR")
                .expect("set WONDER_OF_U_IPADIC_DIR to an extracted lindera-ipadic directory"),
        );
        for value in ["丨", "攵", "昜", "本", "私", "修理", "盗む", "あの", "〜あとで"] {
            let tokens = tokenize_japanese(value, &dictionary).unwrap();
            println!(
                "  {value:<10} {} token(s)  known={:?}  base={:?}",
                tokens.len(),
                tokens
                    .iter()
                    .map(|token| token.known_to_dictionary)
                    .collect::<Vec<_>>(),
                tokens
                    .iter()
                    .map(|token| token.base_form.as_str())
                    .collect::<Vec<_>>()
            );
        }
    }

    /// Runs the scan against a real collection and prints what it proposes.
    #[test]
    #[ignore = "requires a running Anki and an installed dictionary"]
    fn scan_a_real_collection() {
        use super::{examine_note_type, json_string_array};
        use std::{collections::HashSet, path::PathBuf};

        let dictionary = PathBuf::from(
            std::env::var("WONDER_OF_U_IPADIC_DIR")
                .expect("set WONDER_OF_U_IPADIC_DIR to an extracted lindera-ipadic directory"),
        );
        let note_types = json_string_array(
            super::anki_connect_request("modelNames", serde_json::json!({}))
                .expect("Anki must be running with AnkiConnect"),
            "note type list",
        )
        .expect("Anki's note type list should be a list of strings");

        let empty = HashSet::new();
        for note_type in &note_types {
            match examine_note_type(note_type, 21, &dictionary, &empty) {
                Ok(Some(found)) => println!(
                    "  PROPOSE  {:<28} -> {:<22} {:>5} mature   {}",
                    found.note_type,
                    found.field,
                    found.mature_note_count,
                    found.samples.join(" | ")
                ),
                Ok(None) => println!("  skip     {note_type}"),
                Err(message) => println!("  FAILED   {note_type}: {message}"),
            }
        }
    }
}
