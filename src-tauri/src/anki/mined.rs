use tauri::{AppHandle, Manager, Runtime};

use crate::app_types::{MinedSentences, SharedPersistedState};

use super::client::{anki_connect_health_check, anki_connect_request, anki_offline_message};

/// How many notes to ask `notesInfo` about at a time. See `collect_mined_sentences`.
const NOTES_INFO_BATCH: usize = 500;

/// Escapes a value for use inside a quoted Anki search term.
///
/// `*` and `_` are wildcards and `"` would close the term early, so a deck literally
/// named `Japanese_Core` must not silently match `JapaneseXCore`. The backslash is
/// handled first, or it would re-escape the escapes just added.
///
/// Colons are deliberately NOT escaped: Anki splits a term on its FIRST colon only, so
/// every later colon is already literal — and `::` is the subdeck separator, so
/// escaping it would break every nested deck.
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

/// Tags that end a line of text. They unwrap to a space rather than to nothing, so
/// `<div>the cat</div><div>sat</div>` does not fuse into "the catsat" in a
/// space-delimited language. The trailing whitespace collapse absorbs the extras.
const BOUNDARY_TAGS: [&str; 8] = ["br", "div", "p", "li", "tr", "td", "th", "blockquote"];

/// Reduces an Anki field's stored HTML back to the plain sentence the transcript
/// holds, so the two can be compared.
///
/// The field is not plain text: pushing with furigana rewrites the very same field
/// into ruby markup (`<ruby>漢字<rt>かんじ</rt></ruby>`), so a naive comparison would
/// miss every furigana'd card — which is most of them. The readings inside `<rt>` and
/// the fallback parens inside `<rp>` are *additions*, so their content is dropped;
/// every other tag is unwrapped and its text kept.
///
/// This has to survive markup this app never wrote: the deck also holds hand-made and
/// imported notes carrying links, images, comments and inline styles, and the furigana
/// writer itself accepts unbalanced ruby (see `validate_furigana_html`). So every
/// malformed shape has to degrade to "keep the sentence text", never to a truncation.
fn normalize_mined_text(raw: &str) -> String {
    let characters = raw.chars().collect::<Vec<_>>();
    let mut text = String::with_capacity(raw.len());
    let mut index = 0;
    // Set while inside <rt>/<rp>, whose text is a reading rather than the sentence.
    let mut in_reading = false;

    while index < characters.len() {
        if characters[index] != '<' {
            if !in_reading {
                text.push(characters[index]);
            }
            index += 1;
            continue;
        }

        // A comment is dropped whole. It must be handled before the tag scan, whose
        // `>` search would otherwise stop inside the comment body and spill the rest
        // of it into the sentence.
        if characters[index..].starts_with(&['<', '!', '-', '-']) {
            index = match find_sequence(&characters, index + 4, &['-', '-', '>']) {
                Some(end) => end + 3,
                None => characters.len(),
            };
            continue;
        }

        let Some(end) = find_tag_end(&characters, index) else {
            // No closing `>` anywhere: this is a literal less-than ("1<2"), not a tag.
            // Emit it and carry on, rather than swallowing the rest of the sentence.
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
            // Any of these closes the reading. Accepting `</ruby>` matters: the
            // furigana writer permits unbalanced ruby, and keying only on `</rt>`
            // would leave the skip latched and drop the rest of the sentence.
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
///
/// Both halves matter. The reading itself is obviously not part of the sentence — but the
/// SPACE in front of the group is not either: it is inserted purely so Anki's
/// `{{furigana:}}` filter knows where the base text starts. Dropping the reading without
/// dropping that space would leave `これは 漢字 です`, which still would not match, and
/// "already mined" would quietly stop recognising every furigana'd card.
///
/// Only a space between two non-ASCII characters is removed, so an English sentence keeps
/// its word spacing.
fn strip_furigana_brackets(value: &str) -> String {
    let mut without_readings = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(open) = remaining.find('[') {
        // An unterminated `[` is not a reading — it is text that happens to contain a
        // bracket, such as a malformed `[sound:` tag. Keeping the rest verbatim is what
        // stops one stray character erasing the whole sentence.
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

/// Index of the `>` that ends the tag opening at `start`, or None when there is none.
/// Quoted attribute values are skipped, so a `>` inside `href="…?a=1>2"` does not cut
/// the tag short and leak its tail into the sentence.
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
///
/// The audio and transcript roles can be mapped to the SAME field — `insert_furigana_field`
/// has a branch for exactly that — in which case the stored value is
/// `[sound:clip.mp3]<br>今日は猫だ` and a raw comparison would never match its own sentence.
fn strip_media_references(value: &str) -> String {
    let mut stripped = String::with_capacity(value.len());
    let mut remaining = value;
    while let Some(start) = remaining.find("[sound:") {
        stripped.push_str(&remaining[..start]);
        let candidate = &remaining[start..];
        match candidate.find(']') {
            Some(end) => remaining = &candidate[end + 1..],
            // Unterminated, so not really a media tag — keep it as text.
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
///
/// Numeric references are covered too. Our own writer never emits them, but a note
/// pasted into or edited inside Anki routinely carries `&#x27;` or `&#8217;`, and an
/// undecoded one silently costs that sentence its match.
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

/// Resolves `&#39;` / `&#x27;` style references. Anything that isn't a well-formed
/// reference is left exactly as it stands, so stray ampersands survive untouched.
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
            // Malformed or out of range — keep the original text rather than losing it.
            None => decoded.push_str(&remaining[start..start + 2 + end + 1]),
        }
        remaining = &body[end + 1..];
    }
    decoded.push_str(remaining);
    decoded
}

/// Collapses every run of whitespace to a single space and trims. Whisper segments
/// and Anki fields disagree about incidental spacing, and that difference must not
/// decide whether a sentence counts as mined.
fn collapse_whitespace(value: &str) -> String {
    value.split_whitespace().collect::<Vec<_>>().join(" ")
}

/// Every sentence already mined into the configured deck + note type, normalized for
/// comparison against transcript segments.
fn collect_mined_sentences(deck: &str, note_type: &str, field: &str) -> Result<Vec<String>, String> {
    // Scoping the search to the deck AND note type is what keeps this cheap: a large
    // collection stays untouched, only the mining destination is read.
    let query = format!(
        "\"note:{}\" \"deck:{}\"",
        escape_anki_search(note_type),
        escape_anki_search(deck)
    );
    let note_ids = anki_connect_request("findNotes", serde_json::json!({ "query": query }))?;
    let note_ids = note_ids
        .as_array()
        .map(|ids| ids.iter().filter_map(serde_json::Value::as_i64).collect::<Vec<_>>())
        .unwrap_or_default();
    if note_ids.is_empty() {
        return Ok(Vec::new());
    }

    // `notesInfo` returns each note whole — every field, plus tags and card ids — so
    // asking for a 25k-note deck in one call means megabytes of JSON that Anki builds
    // on its UI thread, blowing the 15s request timeout and freezing Anki with it.
    // Chunking bounds both, and makes the timeout a per-batch budget.
    let mut sentences = Vec::with_capacity(note_ids.len());
    for batch in note_ids.chunks(NOTES_INFO_BATCH) {
        let notes = anki_connect_request("notesInfo", serde_json::json!({ "notes": batch }))?;
        // A batch that does not come back as an array is a read that did not happen, and
        // skipping it would quietly shrink the answer: sentences genuinely in the deck would
        // come back unmarked, and the count would still be reported as a fact. Same shape as
        // the Jimaku `unwrap_or_default` — a failure wearing an empty result's clothes.
        let Some(notes) = notes.as_array() else {
            return Err(
                "Anki's note list could not be read — its API may have changed.".to_string(),
            );
        };
        for note in notes {
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

    // Anki being closed must never block reading a transcript, so offline degrades to
    // "no marks" rather than to an error the viewer would have to render.
    if let Err(error) = anki_connect_health_check() {
        return Ok(MinedSentences {
            status: "offline".into(),
            message: anki_offline_message(&error),
            sentences: Vec::new(),
        });
    }

    // Confirm the mapping still points at something real before trusting a count.
    //
    // Every note is read through `fields[field]`, so a field name Anki does not have
    // yields an empty string for EVERY note — and the result is a confident "0 mined
    // sentences found", which reads as "you have mined nothing" rather than "I could not
    // look". `note:` behaves the same way: a deleted note type matches nothing and errors
    // on nobody. Renaming a field in Anki is all it takes to get here, since the mapping
    // stores the name.
    match anki_connect_request(
        "modelFieldNames",
        serde_json::json!({ "modelName": note_type }),
    ) {
        Ok(value) => {
            let known = value
                .as_array()
                .map(|names| {
                    names
                        .iter()
                        .filter_map(serde_json::Value::as_str)
                        .any(|name| name == field)
                })
                .unwrap_or(false);
            if !known {
                return Ok(MinedSentences {
                    status: "unmapped".into(),
                    message: format!(
                        "The note type \"{note_type}\" has no field called \"{field}\" any more,                          so mined sentences cannot be matched. Re-map the sentence field in Settings."
                    ),
                    sentences: Vec::new(),
                });
            }
        }
        Err(error) => {
            return Ok(MinedSentences {
                status: "unmapped".into(),
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
        // The health check passed, so this is the read itself failing — a renamed deck,
        // a timeout, a dropped socket. It is neither "Anki is closed" nor "you haven't
        // configured this", and calling it either would send the user somewhere useless.
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
        // The furigana writer accepts both shapes (see `allows_unbalanced_ruby_tags`),
        // so the reader must not latch into skip mode and truncate the rest.
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
        // Furigana is now stored as Anki bracket notation rather than ruby HTML, so the
        // matcher has to undo BOTH the reading and the space that separates the group —
        // without the second, no furigana'd card would ever match its transcript again.
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
        // Only a space BETWEEN two non-ASCII characters is furigana separation.
        assert_eq!(normalize_mined_text("the cat sat"), "the cat sat");
        assert_eq!(normalize_mined_text("a 猫[ねこ] here"), "a 猫 here");
    }

    #[test]
    fn plain_text_survives_untouched() {
        assert_eq!(normalize_mined_text("これは普通の文です。"), "これは普通の文です。");
    }

    #[test]
    fn search_terms_escape_wildcards_but_leave_subdeck_separators_alone() {
        // `::` is the subdeck separator — escaping it would break nested decks.
        assert_eq!(escape_anki_search("Japanese::Mining"), "Japanese::Mining");
        assert_eq!(escape_anki_search("a*b_c"), "a\\*b\\_c");
        assert_eq!(escape_anki_search("say \"hi\""), "say \\\"hi\\\"");
        assert_eq!(escape_anki_search("back\\slash"), "back\\\\slash");
    }
}
