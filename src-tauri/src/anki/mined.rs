use tauri::{AppHandle, Manager, Runtime};

use crate::app_types::{MinedSentences, SharedPersistedState};

use super::client::{
    anki_connect_health_check, anki_connect_request, anki_offline_message, json_array,
    json_i64_array, json_string_array,
};

/// How many notes to ask `notesInfo` about at a time. See `collect_mined_sentences`.
const NOTES_INFO_BATCH: usize = 500;

/// Escapes a value for use inside a quoted Anki search term.
fn escape_anki_search(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(character, '\\' | '"' | '*' | '_') {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

const BOUNDARY_TAGS: [&str; 8] = ["br", "div", "p", "li", "tr", "td", "th", "blockquote"];

/// Reduces an Anki field's stored HTML back to the plain sentence the transcript
/// holds, so the two can be compared.
fn normalize_mined_text(raw: &str) -> String {
    let characters = raw.chars().collect::<Vec<_>>();
    let mut text = String::with_capacity(raw.len());
    let mut index = 0;
    let mut in_reading = false;

    while index < characters.len() {
        if characters[index] != '<' {
            if !in_reading {
                text.push(characters[index]);
            }
            index += 1;
            continue;
        }

        if characters[index..].starts_with(&['<', '!', '-', '-']) {
            index = match find_sequence(&characters, index + 4, &['-', '-', '>']) {
                Some(end) => end + 3,
                None => characters.len(),
            };
            continue;
        }

        let Some(end) = find_tag_end(&characters, index) else {
            if !in_reading {
                text.push('<');
            }
            index += 1;
            continue;
        };

        let tag = characters[index + 1..end].iter().collect::<String>();
        index = end + 1;

        let closing = tag.starts_with('/');
        let self_closing = tag.ends_with('/');
        let name = tag
            .trim_start_matches('/')
            .trim_end_matches('/')
            .split(char::is_whitespace)
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();

        if in_reading {
            if closing && matches!(name.as_str(), "rt" | "rp" | "ruby") {
                in_reading = false;
            }
        } else if !closing && !self_closing && matches!(name.as_str(), "rt" | "rp") {
            in_reading = true;
        } else if BOUNDARY_TAGS.contains(&name.as_str()) {
            text.push(' ');
        }
    }

    collapse_whitespace(&strip_furigana_brackets(&decode_html_entities(
        &strip_media_references(&text),
    )))
}

/// Removes Anki furigana bracket notation, so `これは 漢字[かんじ] です` compares equal to
/// the transcript's `これは漢字です`.
fn strip_furigana_brackets(value: &str) -> String {
    let mut without_readings = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(open) = remaining.find('[') {
        let Some(close) = remaining[open..].find(']') else {
            break;
        };
        without_readings.push_str(&remaining[..open]);
        remaining = &remaining[open + close + 1..];
    }
    without_readings.push_str(remaining);

    let characters = without_readings.chars().collect::<Vec<_>>();
    let mut cleaned = String::with_capacity(without_readings.len());
    for (index, character) in characters.iter().enumerate() {
        if *character == ' ' && index > 0 && index + 1 < characters.len() {
            let before = characters[index - 1];
            let after = characters[index + 1];
            if !before.is_ascii() && !after.is_ascii() {
                continue;
            }
        }
        cleaned.push(*character);
    }
    cleaned
}

fn find_tag_end(characters: &[char], start: usize) -> Option<usize> {
    let mut quote: Option<char> = None;
    for (offset, character) in characters.iter().enumerate().skip(start + 1) {
        match quote {
            Some(open) if *character == open => quote = None,
            Some(_) => {}
            None => match character {
                '"' | '\'' => quote = Some(*character),
                '>' => return Some(offset),
                _ => {}
            },
        }
    }
    None
}

fn find_sequence(characters: &[char], start: usize, needle: &[char]) -> Option<usize> {
    (start..characters.len().saturating_sub(needle.len() - 1))
        .find(|&index| characters[index..index + needle.len()] == *needle)
}

/// Removes Anki's `[sound:…]` media references.
fn strip_media_references(value: &str) -> String {
    let mut stripped = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(start) = remaining.find("[sound:") {
        stripped.push_str(&remaining[..start]);
        let candidate = &remaining[start..];
        match candidate.find(']') {
            Some(end) => remaining = &candidate[end + 1..],
            None => {
                stripped.push_str(candidate);
                remaining = "";
            }
        }
    }
    stripped.push_str(remaining);
    stripped
}

/// Decodes the entities that reach a field. `&amp;` is resolved last so `&amp;lt;`
/// decodes to the literal `&lt;`, not to `<`.
fn decode_html_entities(value: &str) -> String {
    decode_numeric_references(value)
        .replace("&nbsp;", " ")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&amp;", "&")
}

fn decode_numeric_references(value: &str) -> String {
    if !value.contains("&#") {
        return value.to_string();
    }

    let mut decoded = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(start) = remaining.find("&#") {
        decoded.push_str(&remaining[..start]);
        let body = &remaining[start + 2..];
        let Some(end) = body.find(';') else {
            decoded.push_str(&remaining[start..]);
            return decoded;
        };

        let digits = &body[..end];
        let parsed = match digits.strip_prefix(['x', 'X']) {
            Some(hexadecimal) => u32::from_str_radix(hexadecimal, 16).ok(),
            None => digits.parse::<u32>().ok(),
        }
        .and_then(char::from_u32);

        match parsed {
            Some(character) => decoded.push(character),
            None => decoded.push_str(&remaining[start..start + 2 + end + 1]),
        }
        remaining = &body[end + 1..];
    }
    decoded.push_str(remaining);
    decoded
}

fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every sentence already mined into the configured deck + note type, normalized for
/// comparison against transcript segments.
fn collect_mined_sentences(deck: &str, note_type: &str, field: &str) -> Result<Vec<String>, String> {
    let query = format!(
        "\"note:{}\" \"deck:{}\"",
        escape_anki_search(note_type),
        escape_anki_search(deck)
    );
    let note_ids = json_i64_array(
        anki_connect_request("findNotes", serde_json::json!({ "query": query }))?,
        "note id list",
    )?;
    if note_ids.is_empty() {
        return Ok(Vec::new());
    }

    let mut sentences = Vec::with_capacity(note_ids.len());
    for batch in note_ids.chunks(NOTES_INFO_BATCH) {
        let reply = anki_connect_request("notesInfo", serde_json::json!({ "notes": batch }))?;
        for note in json_array(&reply, "note list")? {
            let value = note
                .get("fields")
                .and_then(|fields| fields.as_object())
                .and_then(|fields| fields.get(field))
                .and_then(|field| field.get("value"))
                .and_then(serde_json::Value::as_str)
                .unwrap_or_default();
            let normalized = normalize_mined_text(value);
            if !normalized.is_empty() {
                sentences.push(normalized);
            }
        }
    }
    sentences.sort_unstable();
    sentences.dedup();
    Ok(sentences)
}

pub(crate) fn load_mined_sentences_inner<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<MinedSentences, String> {
    let (deck, note_type, field) = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the Anki settings.".to_string())?;
        let anki = &persisted.settings.anki;
        (
            anki.deck_name.trim().to_string(),
            anki.note_type.trim().to_string(),
            anki.fields.transcription.trim().to_string(),
        )
    };

    // An incomplete mapping is not an error — the viewer simply has nothing to mark,
    // and mining itself already explains what is missing.
    if deck.is_empty() || note_type.is_empty() || field.is_empty() {
        return Ok(MinedSentences {
            status: "unmapped".into(),
            message: "Choose an Anki deck, note type, and sentence field to see which sentences you have already mined."
                .into(),
            sentences: Vec::new(),
        });
    }

    if let Err(error) = anki_connect_health_check() {
        return Ok(MinedSentences {
            status: "offline".into(),
            message: anki_offline_message(&error),
            sentences: Vec::new(),
        });
    }

    // Confirm the mapping still points at something real before trusting a count.
    match anki_connect_request(
        "modelFieldNames",
        serde_json::json!({ "modelName": note_type }),
    ) {
        Ok(value) => match json_string_array(value, "field list") {
            Err(error) => {
                return Ok(MinedSentences {
                    status: "stale".into(),
                    message: format!("Mined sentences cannot be matched right now. {error}"),
                    sentences: Vec::new(),
                });
            }
            Ok(names) => {
                if !names.iter().any(|name| name.as_str() == field) {
                    return Ok(MinedSentences {
                        status: "stale".into(),
                        message: format!(
                            "The note type \"{note_type}\" has no field called \"{field}\" any more,                          so mined sentences cannot be matched. Re-map the sentence field in Settings."
                        ),
                        sentences: Vec::new(),
                    });
                }
            }
        },
        Err(error) => {
            return Ok(MinedSentences {
                status: "stale".into(),
                message: format!(
                    "The note type \"{note_type}\" is no longer in Anki, so mined sentences                      cannot be matched. Choose one in Settings. ({error})"
                ),
                sentences: Vec::new(),
            });
        }
    }

    match collect_mined_sentences(&deck, &note_type, &field) {
        Ok(sentences) => Ok(MinedSentences {
            status: "ready".into(),
            message: format!("{} mined sentences found in {deck}.", sentences.len()),
            sentences,
        }),
        Err(error) => Ok(MinedSentences {
            status: "error".into(),
            message: format!("Anki could not list the notes in {deck}. {error}"),
            sentences: Vec::new(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn furigana_ruby_normalizes_back_to_the_plain_sentence() {
        assert_eq!(
            normalize_mined_text("<ruby>今日<rt>きょう</rt></ruby>は<ruby>猫<rt>ねこ</rt></ruby>だ"),
            "今日は猫だ"
        );
    }

    #[test]
    fn ruby_fallback_parens_are_dropped_with_their_readings() {
        assert_eq!(
            normalize_mined_text("<ruby><rb>漢字</rb><rp>(</rp><rt>かんじ</rt><rp>)</rp></ruby>"),
            "漢字"
        );
    }

    #[test]
    fn line_breaks_become_a_word_boundary_and_whitespace_collapses() {
        assert_eq!(
            normalize_mined_text("  the   cat<br>sat  "),
            "the cat sat"
        );
    }

    #[test]
    fn entities_decode_without_re_decoding_the_ampersand() {
        assert_eq!(normalize_mined_text("a &amp; b"), "a & b");
        assert_eq!(normalize_mined_text("&amp;lt;"), "&lt;");
        assert_eq!(normalize_mined_text("&quot;hi&quot;&nbsp;there"), "\"hi\" there");
    }

    #[test]
    fn a_self_closing_or_unbalanced_reading_tag_does_not_eat_the_sentence() {
        assert_eq!(
            normalize_mined_text("<ruby>猫<rt/></ruby>がすきです"),
            "猫がすきです"
        );
        assert_eq!(
            normalize_mined_text("<ruby>漢字<rt>かんじ</ruby>です"),
            "漢字です"
        );
    }

    #[test]
    fn a_literal_less_than_is_kept_instead_of_swallowing_the_rest() {
        assert_eq!(normalize_mined_text("1<2 is true"), "1<2 is true");
    }

    #[test]
    fn a_quoted_angle_bracket_inside_an_attribute_does_not_end_the_tag() {
        assert_eq!(
            normalize_mined_text("<a href=\"https://x/?a=1>2\">見る</a>"),
            "見る"
        );
    }

    #[test]
    fn comments_are_dropped_whole() {
        assert_eq!(normalize_mined_text("<!-- a > b -->猫"), "猫");
        // Unterminated: drop to the end rather than spilling markup into the sentence.
        assert_eq!(normalize_mined_text("猫<!-- dangling"), "猫");
    }

    #[test]
    fn block_tags_separate_words_in_space_delimited_text() {
        assert_eq!(
            normalize_mined_text("<div>the cat</div><div>sat down</div>"),
            "the cat sat down"
        );
    }

    #[test]
    fn a_sound_tag_sharing_the_sentence_field_is_stripped() {
        // The audio and transcript roles can point at one field, which stores both.
        assert_eq!(
            normalize_mined_text("[sound:wonder_of_u_x.mp3]<br>今日は猫だ"),
            "今日は猫だ"
        );
        // Unterminated, so not a media tag — keep it.
        assert_eq!(normalize_mined_text("[sound:broken"), "[sound:broken");
    }

    #[test]
    fn numeric_references_decode_in_both_bases() {
        assert_eq!(normalize_mined_text("it&#x27;s fine"), "it's fine");
        assert_eq!(normalize_mined_text("it&#39;s fine"), "it's fine");
        assert_eq!(normalize_mined_text("don&#8217;t"), "don\u{2019}t");
        // Malformed references stay as written rather than vanishing.
        assert_eq!(normalize_mined_text("a &#zz; b"), "a &#zz; b");
        assert_eq!(normalize_mined_text("50% off &#"), "50% off &#");
    }

    #[test]
    fn furigana_bracket_notation_normalizes_back_to_the_sentence() {
        assert_eq!(
            normalize_mined_text("これは 漢字[かんじ]です"),
            "これは漢字です"
        );
        assert_eq!(
            normalize_mined_text("日本語[にほんご]の 勉強[べんきょう]をしています"),
            "日本語の勉強をしています"
        );
    }

    #[test]
    fn english_keeps_its_word_spacing() {
        assert_eq!(normalize_mined_text("the cat sat"), "the cat sat");
        assert_eq!(normalize_mined_text("a 猫[ねこ] here"), "a 猫 here");
    }

    #[test]
    fn plain_text_survives_untouched() {
        assert_eq!(normalize_mined_text("これは普通の文です。"), "これは普通の文です。");
    }

    #[test]
    fn search_terms_escape_wildcards_but_leave_subdeck_separators_alone() {
        assert_eq!(escape_anki_search("Japanese::Mining"), "Japanese::Mining");
        assert_eq!(escape_anki_search("a*b_c"), "a\\*b\\_c");
        assert_eq!(escape_anki_search("say \"hi\""), "say \\\"hi\\\"");
        assert_eq!(escape_anki_search("back\\slash"), "back\\\\slash");
    }
}
