use std::collections::HashSet;

use tauri::{AppHandle, Manager, Runtime};
use unicode_normalization::UnicodeNormalization;

use crate::{
    app_runtime::{log_event, now_ms},
    app_types::{
        AppPathsState, KnownWordIndex, KnownWordsBuild, KnownWordsSnapshot, KnownWordsState,
        SharedPersistedState, VocabularySource,
    },
};

use super::{
    client::{
        anki_connect_health_check, anki_find_notes, anki_notes_info, anki_offline_message,
        json_array,
    },
    known_words_store::{persist_index, remove_known_words_file},
};

/// How many notes one `notesInfo` call asks for.
const NOTES_INFO_BATCH_SIZE: usize = 500;

/// Tags whose content is a furigana reading rather than the word itself.
const RUBY_READING_TAGS: [&str; 2] = ["rt", "rp"];

const BLOCK_TAGS: [&str; 6] = ["br", "div", "p", "li", "tr", "td"];

/// Reduces an expression to the one form both sides of the index are compared in.
pub(super) fn normalize_expression(value: &str) -> String {
    let value = strip_sound_tags(value);
    let value = strip_html(&value);
    let value = decode_html_entities(&value);
    let value = strip_furigana_brackets(&value);
    let value: String = value.nfkc().collect();
    collapse_whitespace(&value).to_lowercase()
}

fn strip_sound_tags(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    while let Some(start) = rest.find("[sound:") {
        let Some(end) = rest[start..].find(']') else {
            break;
        };
        out.push_str(&rest[..start]);
        rest = &rest[start + end + 1..];
    }
    out.push_str(rest);
    out
}

/// Strips HTML to its text, dropping ruby readings along with their tags.
fn strip_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    let mut reading_depth = 0usize;

    while let Some(open) = rest.find('<') {
        if reading_depth == 0 {
            out.push_str(&rest[..open]);
        }

        let after = &rest[open + 1..];
        let Some(close) = after.find('>') else {
            if reading_depth == 0 {
                out.push_str(&rest[open..]);
            }
            return out;
        };

        let (name, closing, self_closing) = parse_tag(&after[..close]);
        if RUBY_READING_TAGS.contains(&name.as_str()) {
            if closing {
                reading_depth = reading_depth.saturating_sub(1);
            } else if !self_closing {
                reading_depth += 1;
            }
        } else if reading_depth == 0 && BLOCK_TAGS.contains(&name.as_str()) {
            out.push(' ');
        }

        rest = &after[close + 1..];
    }

    if reading_depth == 0 {
        out.push_str(rest);
    }
    out
}

fn parse_tag(tag: &str) -> (String, bool, bool) {
    let tag = tag.trim();
    let self_closing = tag.ends_with('/');
    let (name, closing) = match tag.strip_prefix('/') {
        Some(name) => (name, true),
        None => (tag, false),
    };
    let name = name
        .chars()
        .take_while(char::is_ascii_alphanumeric)
        .collect::<String>()
        .to_ascii_lowercase();
    (name, closing, self_closing)
}

fn decode_html_entities(value: &str) -> String {
    value
        .replace("&nbsp;", "\u{a0}")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

/// Drops Anki furigana readings: `漢字[かんじ]` becomes `漢字`.
fn strip_furigana_brackets(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']') else {
            break;
        };
        out.push_str(&rest[..open]);

        if let Some(space) = out.rfind(' ') {
            if space + 1 < out.len() {
                out.remove(space);
            }
        }

        rest = &rest[open + close + 1..];
    }

    out.push_str(rest);
    out
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Quotes a note type name into an Anki search term, restricted to mature cards.
fn note_type_query(note_type: &str, mature_after_days: u32) -> String {
    let mut escaped = String::with_capacity(note_type.len());
    for character in note_type.chars() {
        if matches!(character, '\\' | '"' | '*' | '_' | ':') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    format!("note:\"{escaped}\" prop:ivl>={mature_after_days}")
}

/// Reads one note's chosen field. `None` when the note has no such field — the
/// note type changed under the setting — or when it normalizes away to nothing.
fn note_expression(note: &serde_json::Value, field_name: &str) -> Option<String> {
    let value = note
        .get("fields")?
        .as_object()?
        .get(field_name)?
        .get("value")?
        .as_str()?;
    let expression = normalize_expression(value);
    (!expression.is_empty()).then_some(expression)
}

/// Collects the known expressions out of one batch of notes.
fn known_words_from_notes(notes: &[serde_json::Value], field_name: &str) -> HashSet<String> {
    notes
        .iter()
        .filter_map(|note| note_expression(note, field_name))
        .collect()
}

fn collect_source_words(
    source: &VocabularySource,
    mature_after_days: u32,
    words: &mut HashSet<String>,
) -> Result<(), String> {
    let note_ids = anki_find_notes(&note_type_query(&source.note_type, mature_after_days))?;
    for batch in note_ids.chunks(NOTES_INFO_BATCH_SIZE) {
        let reply = anki_notes_info(batch)?;
        words.extend(known_words_from_notes(
            json_array(&reply, "note list")?,
            &source.field,
        ));
    }
    Ok(())
}

fn collect_known_words(build: &KnownWordsBuild) -> Result<HashSet<String>, String> {
    let mut words = HashSet::new();
    for source in &build.sources {
        collect_source_words(source, build.mature_after_days, &mut words)?;
    }
    Ok(words)
}

fn known_words_build<R: Runtime>(app: &AppHandle<R>) -> Result<KnownWordsBuild, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not read the Anki settings.".to_string())?;
    Ok(KnownWordsBuild::from_anki_settings(&persisted.settings.anki))
}

/// Describes the index as it stands now, whatever this refresh did to it.
fn known_words_snapshot<R: Runtime>(
    app: &AppHandle<R>,
    status: &str,
    message: String,
) -> KnownWordsSnapshot {
    let cached = app.state::<KnownWordsState>();
    let cached = cached.0.lock().ok();
    let index = cached.as_ref().and_then(|index| index.as_ref());
    KnownWordsSnapshot {
        status: status.into(),
        message,
        word_count: index.map(|index| index.words.len()).unwrap_or(0),
        built_at_ms: index.map(|index| index.built_at_ms),
    }
}

fn store_known_words<R: Runtime>(
    app: &AppHandle<R>,
    index: Option<KnownWordIndex>,
) -> Result<(), String> {
    let state = app.state::<KnownWordsState>();
    let mut cached = state
        .0
        .lock()
        .map_err(|_| "Could not store the known-word list.".to_string())?;
    *cached = index;
    Ok(())
}

fn known_words_file<R: Runtime>(app: &AppHandle<R>) -> std::path::PathBuf {
    app.state::<AppPathsState>().inner().known_words_file.clone()
}

fn clear_known_words<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    store_known_words(app, None)?;
    if let Err(error) = remove_known_words_file(&known_words_file(app)) {
        log_event(
            app,
            "WARN",
            "known_words.remove_failed",
            serde_json::json!({ "message": error }),
        );
    }
    Ok(())
}

fn store_and_persist_known_words<R: Runtime>(
    app: &AppHandle<R>,
    index: KnownWordIndex,
) -> Result<(), String> {
    let file = known_words_file(app);
    if let Err(error) = persist_index(&file, &index) {
        log_event(
            app,
            "WARN",
            "known_words.persist_failed",
            serde_json::json!({ "message": error }),
        );
    }
    store_known_words(app, Some(index))
}

/// Rebuilds the known-word index as the union of every configured source.
pub(crate) fn refresh_known_words_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<KnownWordsSnapshot, String> {
    let build = known_words_build(app)?;
    let sources = &build.sources;

    if sources.is_empty() {
        clear_known_words(app)?;
        return Ok(KnownWordsSnapshot {
            status: "unconfigured".into(),
            message: "Add a vocabulary note type and field to build the list.".into(),
            word_count: 0,
            built_at_ms: None,
        });
    }

    if let Err(error) = anki_connect_health_check() {
        return Ok(known_words_snapshot(
            app,
            "offline",
            anki_offline_message(&error),
        ));
    }

    let words = collect_known_words(&build)?;
    if words.is_empty() {
        clear_known_words(app)?;
        return Ok(KnownWordsSnapshot {
            status: "empty".into(),
            message: format!(
                "No settled words found across {} vocabulary {}. Words count once you have known them for {} days — check the fields you chose still hold text, or lower the setting.",
                sources.len(),
                if sources.len() == 1 { "source" } else { "sources" },
                build.mature_after_days
            ),
            word_count: 0,
            built_at_ms: None,
        });
    }

    let word_count = words.len();
    let built_at_ms = now_ms();
    store_and_persist_known_words(
        app,
        KnownWordIndex {
            words,
            built_at_ms,
            build: build.clone(),
        },
    )?;
    Ok(KnownWordsSnapshot {
        status: "ready".into(),
        message: format!(
            "Read {word_count} words from {} vocabulary {}.",
            sources.len(),
            if sources.len() == 1 { "source" } else { "sources" }
        ),
        word_count,
        built_at_ms: Some(built_at_ms),
    })
}

#[cfg(test)]
mod tests {
    use super::{known_words_from_notes, normalize_expression, note_type_query};

    #[test]
    fn plain_text_survives_untouched() {
        assert_eq!(normalize_expression("見る"), "見る");
        assert_eq!(normalize_expression("  食べ物  "), "食べ物");
        assert_eq!(normalize_expression(""), "");
        assert_eq!(normalize_expression("   "), "");
    }

    #[test]
    fn anki_furigana_brackets_keep_the_kanji_and_drop_the_reading() {
        assert_eq!(normalize_expression("漢字[かんじ]"), "漢字");
        assert_eq!(normalize_expression("見[み]る"), "見る");
        assert_eq!(normalize_expression(" 食[た]べ 物[もの]"), "食べ物");
    }

    #[test]
    fn ruby_html_keeps_the_base_text_and_drops_the_reading() {
        assert_eq!(normalize_expression("<ruby>見<rt>み</rt></ruby>"), "見");
        assert_eq!(
            normalize_expression("<ruby><rb>今日</rb><rp>(</rp><rt>きょう</rt><rp>)</rp></ruby>"),
            "今日"
        );
        assert_eq!(
            normalize_expression("<ruby>猫<rt class=\"small\">ねこ</rt></ruby>"),
            "猫"
        );
        assert_eq!(normalize_expression("<RUBY>犬<RT>いぬ</RT></RUBY>"), "犬");
    }

    #[test]
    fn sound_tags_are_dropped() {
        assert_eq!(normalize_expression("[sound:foo.mp3]見る"), "見る");
        assert_eq!(
            normalize_expression("[sound:a.mp3]<br>漢字[かんじ]"),
            "漢字"
        );
        assert_eq!(normalize_expression("[sound:only.mp3]"), "");
    }

    #[test]
    fn anki_editor_wrappers_and_entities_are_unwrapped() {
        assert_eq!(normalize_expression("<div>見る</div>"), "見る");
        assert_eq!(normalize_expression("<b>食</b><b>べる</b>"), "食べる");
        assert_eq!(normalize_expression("&nbsp;見る&nbsp;"), "見る");
        assert_eq!(normalize_expression("a&amp;b"), "a&b");
        assert_eq!(normalize_expression("&quot;猫&quot;"), "\"猫\"");
        assert_eq!(normalize_expression("&lt;b&gt;"), "<b>");
    }

    #[test]
    fn width_variants_fold_onto_one_form() {
        assert_eq!(normalize_expression("ＡＢＣ"), "abc");
        assert_eq!(normalize_expression("ABC"), "abc");
        assert_eq!(normalize_expression("ｶﾞｯｷ"), "ガッキ");
        assert_eq!(normalize_expression("ｱﾒﾘｶ"), "アメリカ");
        assert_eq!(normalize_expression("１２３"), "123");
    }

    #[test]
    fn a_whole_sentence_reduces_to_that_sentence_rather_than_to_nonsense() {
        assert_eq!(
            normalize_expression("<div>私[わたし]は 学校[がっこう]に 行[い]きます</div>"),
            "私は学校に行きます"
        );
        assert_eq!(
            normalize_expression("[sound:a.mp3]<div>今日はいい天気ですね。<br>本当に。</div>"),
            "今日はいい天気ですね。 本当に。"
        );
    }

    #[test]
    fn malformed_markup_degrades_instead_of_panicking() {
        assert_eq!(normalize_expression("見る<"), "見る<");
        assert_eq!(normalize_expression("漢字[かんじ"), "漢字[かんじ");
        assert_eq!(normalize_expression("<div>見る"), "見る");
        assert_eq!(normalize_expression("<ruby>見<rt>み"), "見");
    }

    #[test]
    fn a_note_type_name_cannot_break_out_of_the_search_query() {
        assert_eq!(note_type_query("Mining", 21), "note:\"Mining\" prop:ivl>=21");
        assert_eq!(
            note_type_query("Core \"2k\"", 21),
            "note:\"Core \\\"2k\\\"\" prop:ivl>=21"
        );
        assert_eq!(note_type_query("Core_2k", 21), "note:\"Core\\_2k\" prop:ivl>=21");
        assert_eq!(note_type_query("A*B", 21), "note:\"A\\*B\" prop:ivl>=21");
        assert_eq!(note_type_query("a\\b", 21), "note:\"a\\\\b\" prop:ivl>=21");
    }

    /// The maturity threshold is the whole difference between this index and the
    /// shelved one, which counted a word the moment a card carrying it existed.
    /// Anki applies `prop:ivl` itself, so a young note is never fetched — asserting
    /// on the query is asserting that the filter runs at all.
    #[test]
    fn only_words_held_past_the_threshold_are_asked_for() {
        assert!(note_type_query("Mining", 21).ends_with(" prop:ivl>=21"));
        // The setting is honoured rather than baked in: someone who considers a
        // word settled sooner gets a query that says so.
        assert!(note_type_query("Mining", 7).ends_with(" prop:ivl>=7"));
        assert!(note_type_query("Mining", 90).ends_with(" prop:ivl>=90"));
    }

    fn note(field: &str, value: &str) -> serde_json::Value {
        serde_json::json!({ "fields": { field: { "value": value, "order": 0 } } })
    }

    #[test]
    fn notes_are_read_into_a_deduplicated_index() {
        let notes = vec![
            note("Expression", "見[み]る"),
            note("Expression", "<ruby>見<rt>み</rt></ruby>る"),
            note("Expression", "食べる"),
        ];
        let words = known_words_from_notes(&notes, "Expression");

        // The first two are the same word wearing different markup. An index that
        // did not fold them would also fail to match the tokenizer on either.
        assert_eq!(words.len(), 2);
        assert!(words.contains("見る"));
        assert!(words.contains("食べる"));
    }

    #[test]
    fn notes_missing_or_empty_in_the_chosen_field_are_skipped() {
        let notes = vec![
            note("Expression", "見る"),
            note("Expression", ""),
            note("Expression", "[sound:a.mp3]"),
            note("Expression", "<div><br></div>"),
            // A note of another shape entirely: the field simply is not there.
            note("Word", "食べる"),
        ];
        let words = known_words_from_notes(&notes, "Expression");

        assert_eq!(words.len(), 1);
        assert!(words.contains("見る"));
    }

    #[test]
    fn an_empty_batch_yields_no_words_and_says_nothing_about_the_reply() {
        // The case this replaces asserted that a reply which was not an array yielded no
        // words, which is the bug written down as a requirement: an unreadable reply and a
        // deck of unknown words became the same answer, and the index recorded the second.
        // That case cannot be expressed here any more — the reply is checked where it
        // arrives, and this function is handed notes.
        assert!(known_words_from_notes(&[], "Expression").is_empty());
        assert!(known_words_from_notes(&[serde_json::json!({})], "Expression").is_empty());
    }
}
