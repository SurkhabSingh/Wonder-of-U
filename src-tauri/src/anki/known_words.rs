//! The index of words the user already knows, read from their Anki collection.
//!
//! Sentence ranking asks one question of this — "have I seen this word before?" —
//! and the answer is only as good as the agreement between the two sides asking
//! and answering it. `normalize_expression` is where that agreement is made, and
//! it is the part of this module worth reading.

use std::collections::HashSet;

use tauri::{AppHandle, Manager, Runtime};
use unicode_normalization::UnicodeNormalization;

use crate::{
    app_runtime::now_ms,
    app_types::{
        KnownWordIndex, KnownWordsSnapshot, KnownWordsState, SharedPersistedState, VocabularySource,
    },
};

use super::client::{
    anki_connect_health_check, anki_find_notes, anki_notes_info, anki_offline_message,
};

/// How many notes one `notesInfo` call asks for.
///
/// `findNotes` on a serious collection hands back 20k-50k ids at once, and
/// `notesInfo` answers with every field of every note it is given — a mined
/// sentence note carrying a sentence, a translation and a `[sound:...]` tag runs
/// to several KB. Passing the whole id list to one call would build a response
/// measured in hundreds of megabytes, resident twice over (the body, then the
/// parsed tree), and would hold Anki's own UI thread for the length of that
/// serialization.
///
/// 500 keeps a batch's response in the low megabytes and costs ~100 round trips
/// for a 50k collection. That trade is one-sided on localhost: a round trip is
/// sub-millisecond, a hundred-megabyte allocation is not.
const NOTES_INFO_BATCH_SIZE: usize = 500;

/// Tags whose content is a furigana reading rather than the word itself.
const RUBY_READING_TAGS: [&str; 2] = ["rt", "rp"];

/// Tags that end a line. They reduce to a space so a two-line field does not
/// weld its lines into one false word.
const BLOCK_TAGS: [&str; 6] = ["br", "div", "p", "li", "tr", "td"];

/// Reduces an expression to the one form both sides of the index are compared in.
///
/// The two sides do not arrive looking anything alike. lindera hands over a bare
/// dictionary form (見る); an Anki field hands over whatever the user's note type,
/// the Japanese Support add-on, this app's own furigana writer, and Anki's editor
/// have layered onto it. Everything here strips that layering back off.
///
/// BOTH sides must be passed through this — the transcript token as well as the
/// Anki field. A comparison is only as honest as its least normalized operand, and
/// the cost of forgetting is silent: words simply look new forever.
pub(super) fn normalize_expression(value: &str) -> String {
    let value = strip_sound_tags(value);
    let value = strip_html(&value);
    // After the tags are gone, so an escaped `&lt;b&gt;` stays the literal text it
    // was rather than becoming a tag that the stripper already had its chance at.
    let value = decode_html_entities(&value);
    let value = strip_furigana_brackets(&value);
    // NFKC is what folds ＡＢＣ onto ABC, half-width katakana ｶ onto カ (recomposing
    // the ﾞ dakuten that half-width forms split off), and the &nbsp; decoded above
    // onto a plain space. Hand-rolling the fullwidth-ASCII offset would be a few
    // lines; hand-rolling the katakana composition tables would not, and this is
    // not a new dependency — lindera already compiles unicode-normalization into
    // the binary, so taking it directly costs nothing but the `use`.
    let value: String = value.nfkc().collect();
    // Lowercased so an Anki field reading "Apple" answers for a transcript's
    // "apple". A no-op on Japanese, which is the whole point of the feature; the
    // gain is on the Latin words that turn up inside Japanese sentences.
    collapse_whitespace(&value).to_lowercase()
}

/// Drops `[sound:foo.mp3]` tags. Runs first so that every bracket left for
/// `strip_furigana_brackets` is a reading.
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
///
/// The direction is the whole point and it is the one thing here that is easy to
/// get backwards: `<ruby>見<rt>み</rt></ruby>` must reduce to 見, never み. The
/// transcript side deals in kanji dictionary forms, so an index built out of
/// readings would match nothing and report every kanji word the user knows as new
/// — a feature that looks like it works and is wrong on every row.
///
/// This app writes exactly this markup itself (`insert_furigana_field`), so the
/// shape is not hypothetical.
fn strip_html(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;
    // Depth rather than a flag: `<rp>(</rp>` pairs nest inside `<ruby>` alongside
    // `<rt>`, and a malformed field can open one twice.
    let mut reading_depth = 0usize;

    while let Some(open) = rest.find('<') {
        if reading_depth == 0 {
            out.push_str(&rest[..open]);
        }

        let after = &rest[open + 1..];
        let Some(close) = after.find('>') else {
            // An unterminated `<` is a literal the user typed, not a tag.
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

/// Pulls the name out of a tag's innards (everything between `<` and `>`), plus
/// whether it closes or self-closes. Attributes are ignored rather than parsed:
/// nothing here renders the markup, it only needs to know which element it is in.
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
    // `&amp;` decodes last: doing it first would turn the literal text `&amp;lt;`
    // into `<`, inventing markup out of an escaped ampersand.
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
///
/// Same direction as ruby, for the same reason: the base text is what the
/// transcript will ask about, the reading is not.
///
/// The space the Japanese Support add-on puts before each base run goes with the
/// reading. Without that, ` 食[た]べ 物[もの]` would reduce to `食べ 物` and never
/// match the tokenizer's 食べ物 — the separator is the add-on's punctuation, not
/// the user's word.
fn strip_furigana_brackets(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(open) = rest.find('[') {
        let Some(close) = rest[open..].find(']') else {
            break;
        };
        out.push_str(&rest[..open]);

        // `out` now ends with the base run this reading belongs to; the space in
        // front of that run is the add-on's separator.
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

/// Quotes a note type name into an Anki search term.
///
/// The name is user data crossing into a query language. `\` and `"` would break
/// out of the quoted term, and `*`/`_` are wildcards even inside quotes — a note
/// type called `Core_2k` would otherwise also match a `Core 2k`, quietly indexing
/// someone else's deck.
fn note_type_query(note_type: &str) -> String {
    let mut escaped = String::with_capacity(note_type.len());
    for character in note_type.chars() {
        if matches!(character, '\\' | '"' | '*' | '_' | ':') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    format!("note:\"{escaped}\"")
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

/// Collects the known expressions out of one `notesInfo` response.
fn known_words_from_notes(notes: &serde_json::Value, field_name: &str) -> HashSet<String> {
    notes
        .as_array()
        .map(|notes| {
            notes
                .iter()
                .filter_map(|note| note_expression(note, field_name))
                .collect()
        })
        .unwrap_or_default()
}

/// Walks one note type over AnkiConnect and folds its words into `words`. Takes no
/// locks and holds none: every caller runs it between reading the settings and
/// storing the result.
fn collect_source_words(
    source: &VocabularySource,
    words: &mut HashSet<String>,
) -> Result<(), String> {
    let note_ids = anki_find_notes(&note_type_query(&source.note_type))?;
    for batch in note_ids.chunks(NOTES_INFO_BATCH_SIZE) {
        words.extend(known_words_from_notes(&anki_notes_info(batch)?, &source.field));
    }
    Ok(())
}

/// The union of every configured source. One `HashSet` across all of them, so a
/// word in both a Kaishi and a Lapis deck is counted once for free — the whole
/// point of reading them together rather than letting the last Refresh win.
fn collect_known_words(sources: &[VocabularySource]) -> Result<HashSet<String>, String> {
    let mut words = HashSet::new();
    for source in sources {
        collect_source_words(source, &mut words)?;
    }
    Ok(words)
}

fn vocabulary_sources<R: Runtime>(app: &AppHandle<R>) -> Result<Vec<VocabularySource>, String> {
    let persisted_state = app.state::<SharedPersistedState>();
    let persisted = persisted_state
        .0
        .lock()
        .map_err(|_| "Could not read the Anki settings.".to_string())?;
    Ok(persisted.settings.anki.vocabulary_sources.clone())
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

/// Fails rather than degrading, unlike the read above: a store we swallowed would
/// leave us reporting a word count that is not in the cache, and ranking silently
/// off with the UI saying otherwise.
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

/// Rebuilds the known-word index as the union of every configured source.
///
/// Manual by design. Anki is edited outside this app and AnkiConnect has no change
/// notification, so there is no moment we could honestly call the index stale —
/// anything automatic would either be silently out of date or walk the whole
/// collection on a timer. The user presses the button; the timestamp we return
/// lets them judge for themselves.
pub(crate) fn refresh_known_words_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<KnownWordsSnapshot, String> {
    let sources = vocabulary_sources(app)?;

    // No sources is "off", not "broken". Nothing to build and nothing to
    // apologize for. `normalize_settings` has already dropped half-filled rows, so
    // an empty list here really is "the user has added none".
    if sources.is_empty() {
        store_known_words(app, None)?;
        return Ok(KnownWordsSnapshot {
            status: "unconfigured".into(),
            message: "Add a vocabulary note type and field to build the list.".into(),
            word_count: 0,
            built_at_ms: None,
        });
    }

    if let Err(error) = anki_connect_health_check() {
        // The index we already have is still the best answer available, so it
        // survives an offline refresh; only the message changes.
        return Ok(known_words_snapshot(
            app,
            "offline",
            anki_offline_message(&error),
        ));
    }

    let words = collect_known_words(&sources)?;
    if words.is_empty() {
        // Every configured source came back empty — the note types hold nothing we
        // can read. Clearing matters: any index still cached belongs to a previous
        // selection.
        store_known_words(app, None)?;
        return Ok(KnownWordsSnapshot {
            status: "empty".into(),
            message: format!(
                "No words found across {} vocabulary {}. Check the fields you chose still hold text.",
                sources.len(),
                if sources.len() == 1 { "source" } else { "sources" }
            ),
            word_count: 0,
            built_at_ms: None,
        });
    }

    // Reported from what was built rather than read back out of the cache, so the
    // count and the timestamp are the ones actually stored.
    let word_count = words.len();
    let built_at_ms = now_ms();
    store_known_words(
        app,
        Some(KnownWordIndex {
            words,
            built_at_ms,
        }),
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
        // The direction that matters: getting this backwards indexes readings and
        // makes every kanji word the user knows look new.
        assert_eq!(normalize_expression("漢字[かんじ]"), "漢字");
        assert_eq!(normalize_expression("見[み]る"), "見る");
        // The add-on's leading separator goes with the reading, or a multi-run
        // word never matches the tokenizer's single token.
        assert_eq!(normalize_expression(" 食[た]べ 物[もの]"), "食べ物");
    }

    #[test]
    fn ruby_html_keeps_the_base_text_and_drops_the_reading() {
        assert_eq!(normalize_expression("<ruby>見<rt>み</rt></ruby>"), "見");
        // The shape this app writes itself, via insert_furigana_field.
        assert_eq!(
            normalize_expression("<ruby><rb>今日</rb><rp>(</rp><rt>きょう</rt><rp>)</rp></ruby>"),
            "今日"
        );
        // A reading tag carrying attributes still has to be recognized as one --
        // stripping the tag but keeping its text would index the reading.
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
        // html_escape writes this for a literal `<b>` the user typed; decoding
        // after tag-stripping is what keeps it literal instead of resurrecting it
        // as markup that then gets stripped.
        assert_eq!(normalize_expression("&lt;b&gt;"), "<b>");
    }

    #[test]
    fn width_variants_fold_onto_one_form() {
        assert_eq!(normalize_expression("ＡＢＣ"), "abc");
        assert_eq!(normalize_expression("ABC"), "abc");
        // Half-width katakana, including the dakuten NFKC has to recompose.
        assert_eq!(normalize_expression("ｶﾞｯｷ"), "ガッキ");
        assert_eq!(normalize_expression("ｱﾒﾘｶ"), "アメリカ");
        assert_eq!(normalize_expression("１２３"), "123");
    }

    #[test]
    fn a_whole_sentence_reduces_to_that_sentence_rather_than_to_nonsense() {
        // The real case behind this: the user points the setting at their sentence
        // note type. It must not match any single token -- it should index one long
        // useless string and leave the ranking honestly finding nothing, not
        // silently mark words known.
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
        // Unterminated tags and brackets are user data too.
        assert_eq!(normalize_expression("見る<"), "見る<");
        assert_eq!(normalize_expression("漢字[かんじ"), "漢字[かんじ");
        assert_eq!(normalize_expression("<div>見る"), "見る");
        // An unclosed reading tag swallows the rest rather than leaking a reading.
        assert_eq!(normalize_expression("<ruby>見<rt>み"), "見");
    }

    #[test]
    fn a_note_type_name_cannot_break_out_of_the_search_query() {
        assert_eq!(note_type_query("Mining"), "note:\"Mining\"");
        assert_eq!(
            note_type_query("Core \"2k\""),
            "note:\"Core \\\"2k\\\"\""
        );
        // Wildcards are live inside quotes, so an underscore must not silently
        // widen the search to a neighbouring note type.
        assert_eq!(note_type_query("Core_2k"), "note:\"Core\\_2k\"");
        assert_eq!(note_type_query("A*B"), "note:\"A\\*B\"");
        assert_eq!(note_type_query("a\\b"), "note:\"a\\\\b\"");
    }

    fn note(field: &str, value: &str) -> serde_json::Value {
        serde_json::json!({ "fields": { field: { "value": value, "order": 0 } } })
    }

    #[test]
    fn notes_are_read_into_a_deduplicated_index() {
        let notes = serde_json::json!([
            note("Expression", "見[み]る"),
            note("Expression", "<ruby>見<rt>み</rt></ruby>る"),
            note("Expression", "食べる"),
        ]);
        let words = known_words_from_notes(&notes, "Expression");

        // The first two are the same word wearing different markup. An index that
        // did not fold them would also fail to match the tokenizer on either.
        assert_eq!(words.len(), 2);
        assert!(words.contains("見る"));
        assert!(words.contains("食べる"));
    }

    #[test]
    fn notes_missing_or_empty_in_the_chosen_field_are_skipped() {
        let notes = serde_json::json!([
            note("Expression", "見る"),
            note("Expression", ""),
            note("Expression", "[sound:a.mp3]"),
            note("Expression", "<div><br></div>"),
            // A note of another shape entirely: the field simply is not there.
            note("Word", "食べる"),
        ]);
        let words = known_words_from_notes(&notes, "Expression");

        assert_eq!(words.len(), 1);
        assert!(words.contains("見る"));
    }

    #[test]
    fn a_response_that_is_not_a_note_array_yields_no_words() {
        assert!(known_words_from_notes(&serde_json::Value::Null, "Expression").is_empty());
        assert!(known_words_from_notes(&serde_json::json!([]), "Expression").is_empty());
        assert!(known_words_from_notes(&serde_json::json!([{}]), "Expression").is_empty());
    }
}
