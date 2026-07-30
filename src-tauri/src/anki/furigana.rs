use std::time::Duration;

use serde::Deserialize;

use super::fields::{html_escape, preserve_anki_sound_tags};
use crate::{
    app_state::{is_japanese_transcript_language, transcript_looks_japanese},
    app_types::{AnkiSettings, RecentRecording},
};

const ANKI_LOOKUP_FURIGANA_URL: &str = "http://127.0.0.1:8766/furigana";
const ANKI_LOOKUP_TIMEOUT: Duration = Duration::from_millis(2500);

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct FuriganaBridgeResponse {
    ok: bool,
    #[serde(default)]
    furigana_html: Option<String>,
    #[serde(default)]
    error: Option<String>,
}

pub(super) fn request_furigana_html(text: &str) -> Result<String, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(ANKI_LOOKUP_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;

    let response_text = client
        .post(ANKI_LOOKUP_FURIGANA_URL)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(serde_json::json!({ "text": text }).to_string())
        .send()
        .map_err(|error| {
            format!(
                "Anki Lookup add-on is not running or did not respond. Open Anki with the Wonder of U/Anki Lookup add-on installed, then try again. {error}"
            )
        })?
        .error_for_status()
        .map_err(|error| format!("Anki Lookup add-on rejected the furigana request. {error}"))?
        .text()
        .map_err(|error| format!("Anki Lookup add-on response could not be read. {error}"))?;
    let response = serde_json::from_str::<FuriganaBridgeResponse>(&response_text)
        .map_err(|error| format!("Anki Lookup add-on returned invalid furigana data. {error}"))?;

    if response.ok {
        let furigana_html = response
            .furigana_html
            .ok_or_else(|| "Anki Lookup add-on did not return furigana HTML.".to_string())?;
        // Converted to bracket notation rather than validated as markup. The old strict
        // tag allowlist existed because ruby HTML from an unauthenticated port went
        // straight into a field Anki renders; converting drops every tag instead, so
        // there is no markup left to allow or reject. It also means an add-on that
        // changes its wrapper — which is exactly what silently broke this — can no longer
        // break furigana.
        let brackets = ruby_html_to_furigana_brackets(&furigana_html);
        if brackets.trim().is_empty() {
            return Err("Anki Lookup add-on returned furigana with no readable text.".into());
        }
        Ok(brackets)
    } else {
        Err(response
            .error
            .unwrap_or_else(|| "Anki Lookup add-on could not create furigana.".into()))
    }
}


/// Converts the bridge's ruby HTML into Anki's bracket notation:
/// `<ruby>漢字<rt>かんじ</rt></ruby>` becomes `漢字[かんじ]`.
///
/// This is what the Lapis note type does, and what Yomitan and mpvacious emit. The point
/// is not cosmetic — it removes the security problem rather than managing it. Storing
/// ruby HTML meant accepting markup from an unauthenticated localhost port and rendering
/// it inside Anki's QtWebEngine, which is why a strict tag allowlist existed at all.
/// Bracket notation is PLAIN TEXT: the field is escaped like every other field, and
/// Anki's own `{{furigana:}}` filter builds the ruby at render time. Nothing the bridge
/// sends can be markup any more, because every tag is dropped here.
///
/// A space is inserted before a reading group when the preceding character is not
/// already one. Anki's filter matches `([^ >]+?)\[(.+?)\]`, so without that separator
/// `これは漢字[かんじ]` makes the WHOLE run the base text and renders the reading over
/// `これは漢字`. Yomitan inserts the same space for the same reason.
pub(super) fn ruby_html_to_furigana_brackets(html: &str) -> String {
    let characters = html.chars().collect::<Vec<_>>();
    let mut output = String::with_capacity(html.len());
    // Text collected inside the current <ruby>, i.e. the base the reading belongs to.
    let mut base = String::new();
    let mut reading = String::new();
    let mut index = 0;
    let mut in_ruby = false;
    // <rt> is the reading; <rp> is fallback parens for renderers without ruby support and
    // must be dropped whole; <style>/<script> content is not text at all.
    let mut in_reading = false;
    let mut skip_depth: Option<&'static str> = None;

    while index < characters.len() {
        let character = characters[index];
        if character != '<' {
            if skip_depth.is_some() {
                // Inside <rp>/<style>/<script>: swallowed.
            } else if in_reading {
                reading.push(character);
            } else if in_ruby {
                base.push(character);
            } else {
                output.push(character);
            }
            index += 1;
            continue;
        }

        let Some(end) = characters[index..].iter().position(|value| *value == '>') else {
            // An unterminated `<` is literal text, not a tag.
            if skip_depth.is_none() && !in_reading {
                if in_ruby {
                    base.push('<');
                } else {
                    output.push('<');
                }
            }
            index += 1;
            continue;
        };
        let tag = characters[index + 1..index + end].iter().collect::<String>();
        index += end + 1;

        let closing = tag.starts_with('/');
        let name = tag
            .trim_start_matches('/')
            .trim_end_matches('/')
            .split(char::is_whitespace)
            .next()
            .unwrap_or_default()
            .to_ascii_lowercase();

        if let Some(open) = skip_depth {
            if closing && open == name {
                skip_depth = None;
            }
            continue;
        }

        match (name.as_str(), closing) {
            ("ruby", false) => {
                in_ruby = true;
                base.clear();
                reading.clear();
            }
            ("ruby", true) => {
                let base_text = base.trim();
                let reading_text = reading.trim();
                if !base_text.is_empty() {
                    if !reading_text.is_empty() {
                        // Separate the group from whatever precedes it, or Anki's filter
                        // swallows that text into the base.
                        //
                        // The test is against a LITERAL SPACE, not `is_whitespace`. Anki's
                        // pattern is ` ?([^ >]+?)\[(.+?)\]`, and that class excludes only
                        // a space and `>` — a NEWLINE is fair game, so the match crosses
                        // it and swallows the end of the previous line into the base. On a
                        // multi-line transcript that renders the reading over the wrong
                        // words entirely.
                        if output.chars().last().is_some_and(|last| last != ' ') {
                            output.push(' ');
                        }
                        output.push_str(base_text);
                        output.push('[');
                        output.push_str(reading_text);
                        output.push(']');
                    } else {
                        output.push_str(base_text);
                    }
                }
                in_ruby = false;
                in_reading = false;
                base.clear();
                reading.clear();
            }
            ("rt", false) => in_reading = true,
            ("rt", true) => in_reading = false,
            // Fallback parens, and anything whose contents are not text.
            ("rp", false) => skip_depth = Some("rp"),
            ("style", false) => skip_depth = Some("style"),
            ("script", false) => skip_depth = Some("script"),
            // A line break is real content the transcript carried over.
            ("br", _) => {
                if in_ruby {
                    base.push('\n');
                } else {
                    output.push('\n');
                }
            }
            // <rb>, <span>, and anything else: unwrap, keep the text.
            _ => {}
        }
    }

    // An unclosed <ruby> still has text worth keeping.
    let trailing = base.trim();
    if !trailing.is_empty() {
        output.push_str(trailing);
    }
    output
}

/// Writes validated furigana over the mapped transcription field, keeping any
/// `[sound:...]` tag the field already carries. Shared by the mine and push flows,
/// which differ only in how they report the outcome.
pub(super) fn insert_furigana_field(
    settings: &AnkiSettings,
    furigana_brackets: &str,
    media_file_name: &str,
    fields: &mut serde_json::Map<String, serde_json::Value>,
) {
    // Bracket notation is plain text, so it is escaped like every other field value —
    // the reason the old ruby-HTML path could not be.
    let furigana_html = html_escape(furigana_brackets);
    let furigana_html = furigana_html.as_str();
    let target_field = settings.fields.transcription.as_str();
    let existing_value = fields.get(target_field).and_then(|value| value.as_str());
    let fallback_sound_tag =
        if !settings.fields.audio.is_empty() && settings.fields.audio == target_field {
            Some(format!("[sound:{media_file_name}]"))
        } else {
            None
        };
    let merged = preserve_anki_sound_tags(existing_value, furigana_html, fallback_sound_tag.as_deref());
    fields.insert(target_field.to_string(), serde_json::Value::String(merged));
}

pub(crate) fn recording_transcript_supports_furigana(
    recording: &RecentRecording,
    transcript: &str,
) -> bool {
    if is_japanese_transcript_language(recording.transcript_language.as_deref()) {
        return true;
    }

    if recording.transcript_language.is_some() {
        return false;
    }

    transcript_looks_japanese(transcript)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_types::AnkiSettings;

    #[test]
    fn the_field_writer_escapes_markup_that_survived_the_conversion() {
        // The bracket text is plain text by construction, but the SENTENCE inside it is
        // whatever was transcribed — a bare `<` or `&` reaches here intact, and the field
        // it lands in is rendered by Anki as HTML. Escaping is the whole reason bracket
        // notation replaced ruby markup, so it is pinned rather than assumed.
        let mut settings = AnkiSettings::default();
        settings.fields.transcription = "Sentence".into();
        let mut fields = serde_json::Map::new();
        insert_furigana_field(&settings, "a < b & c", "clip.mp3", &mut fields);

        let written = fields["Sentence"].as_str().unwrap();
        assert!(!written.contains("< b"), "a bare < would open a tag: {written}");
        assert!(written.contains("&lt;"));
        assert!(written.contains("&amp;"));
    }

    #[test]
    fn an_existing_sound_tag_survives_the_furigana_rewrite() {
        // Furigana overwrites the transcript field, and on a note type where the audio is
        // mapped to that same field the `[sound:]` tag lives there too — losing it would
        // silently mute a card that had audio a moment earlier.
        let mut settings = AnkiSettings::default();
        settings.fields.transcription = "Sentence".into();
        let mut fields = serde_json::Map::new();
        fields.insert(
            "Sentence".into(),
            serde_json::Value::String("[sound:old.mp3] previous text".into()),
        );
        insert_furigana_field(&settings, "kanji[reading]", "clip.mp3", &mut fields);

        let written = fields["Sentence"].as_str().unwrap();
        assert!(
            written.contains("[sound:old.mp3]"),
            "audio was dropped: {written}"
        );
    }
    use super::ruby_html_to_furigana_brackets;

    #[test]
    fn ruby_becomes_anki_bracket_notation() {
        assert_eq!(
            ruby_html_to_furigana_brackets("<ruby>漢字<rt>かんじ</rt></ruby>"),
            "漢字[かんじ]"
        );
    }

    #[test]
    fn a_space_separates_a_group_from_the_text_before_it() {
        // Anki's filter matches `([^ >]+?)\[(.+?)\]`, so without the space the base
        // becomes the WHOLE preceding run and the reading renders over これは漢字.
        assert_eq!(
            ruby_html_to_furigana_brackets("これは<ruby>漢字<rt>かんじ</rt></ruby>です"),
            "これは 漢字[かんじ]です"
        );
        // No leading space when the group starts the string or already follows one.
        assert_eq!(
            ruby_html_to_furigana_brackets("<ruby>猫<rt>ねこ</rt></ruby>だ"),
            "猫[ねこ]だ"
        );
    }

    #[test]
    fn the_addons_style_and_span_wrapper_is_dropped() {
        // The exact shape that silently broke furigana: a <style> block plus a class'd
        // <span>. The CSS must not leak into the card as text.
        let html = concat!(
            "<style>.wonder-of-u-furigana rt{visibility:hidden;}</style>",
            "<span class=\"wonder-of-u-furigana\">",
            "<ruby>日本語<rt>にほんご</rt></ruby>の<ruby>勉強<rt>べんきょう</rt></ruby>",
            "</span>"
        );
        assert_eq!(
            ruby_html_to_furigana_brackets(html),
            "日本語[にほんご]の 勉強[べんきょう]"
        );
    }

    #[test]
    fn rp_fallback_parens_are_dropped_and_rb_is_unwrapped() {
        assert_eq!(
            ruby_html_to_furigana_brackets(
                "<ruby><rb>今日</rb><rp>(</rp><rt>きょう</rt><rp>)</rp></ruby>は"
            ),
            "今日[きょう]は"
        );
    }

    #[test]
    fn markup_can_never_survive_the_conversion() {
        // This is what replaced the old tag allowlist: there is no markup left to allow
        // or reject, because every tag is dropped and the result is escaped by the caller.
        let converted = ruby_html_to_furigana_brackets(
            "<script>steal()</script><ruby>猫<rt>ねこ</rt></ruby><img src=x onerror=alert(1)>"
        );
        assert!(!converted.contains('<'), "got {converted:?}");
        assert!(!converted.contains("steal"), "got {converted:?}");
        assert!(!converted.contains("onerror"), "got {converted:?}");
        assert_eq!(converted, "猫[ねこ]");
    }

    #[test]
    fn plain_text_and_line_breaks_survive() {
        assert_eq!(ruby_html_to_furigana_brackets("ただの文です"), "ただの文です");
        assert_eq!(ruby_html_to_furigana_brackets("一行目<br>二行目"), "一行目
二行目");
    }

    #[test]
    fn a_ruby_without_a_reading_keeps_its_base_text() {
        // The add-on emits bare <ruby> for words it has no reading for; those must not
        // vanish from the sentence.
        assert_eq!(
            ruby_html_to_furigana_brackets("<ruby>ねこ</ruby>がいる"),
            "ねこがいる"
        );
    }

    #[test]
    fn converts_a_real_addon_response() {
        let html = concat!(
            "<span class=\"wonder-of-u-furigana\">これを1つお",
            "<ruby>願<rt>ねが</rt></ruby>いします。",
            "<ruby>単品<rt>たんぴん</rt></ruby></span>"
        );
        assert_eq!(
            ruby_html_to_furigana_brackets(html),
            "これを1つお 願[ねが]いします。 単品[たんぴん]"
        );
    }

    #[test]
    fn a_group_after_a_newline_still_gets_its_separating_space() {
        // The bug this pins. Anki's pattern is ` ?([^ >]+?)\[(.+?)\]` and that class
        // excludes only a SPACE and `>` — a newline is fair game. Without a space the
        // match crosses the line break, the base becomes the end of the PREVIOUS line,
        // and the reading renders over the wrong words. Multi-line transcripts hit this
        // on nearly every line, which is exactly what was seen on a real card.
        let html = concat!(
            "お<ruby>願<rt>ねが</rt></ruby>いします。\n",
            "<ruby>単品<rt>たんぴん</rt></ruby>です"
        );
        let got = ruby_html_to_furigana_brackets(html);
        assert_eq!(got, "お 願[ねが]いします。\n 単品[たんぴん]です");

        // Every group's base must start after a literal space (or the string start), and
        // must not reach back across a newline.
        for (bracket, _) in got.match_indices('[') {
            let base_start = got[..bracket].rfind(' ').map(|at| at + 1).unwrap_or(0);
            assert!(
                base_start == 0 || got.as_bytes()[base_start - 1] == b' ',
                "group at {bracket} is not space-delimited in {got:?}"
            );
            assert!(
                !got[base_start..bracket].contains('\n'),
                "base text crosses a newline in {got:?}"
            );
        }
    }
}
