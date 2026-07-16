use std::time::Duration;

use serde::Deserialize;

use super::fields::preserve_anki_sound_tags;
use crate::{
    app_state::{is_japanese_transcript_language, transcript_looks_japanese},
    app_types::{AnkiSettings, RecentRecording},
};

const ANKI_LOOKUP_FURIGANA_URL: &str = "http://127.0.0.1:8766/furigana";
const ANKI_LOOKUP_TIMEOUT: Duration = Duration::from_millis(2500);

/// The only markup furigana needs: a `<ruby>` wrapper, `<rt>`/`<rb>` for the
/// reading and base text, `<rp>` fallback parens, and `<br>` for the line breaks
/// a multi-line transcript carries over.
const ALLOWED_FURIGANA_TAGS: [&str; 5] = ["ruby", "rt", "rb", "rp", "br"];

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
        validate_furigana_html(&furigana_html)?;
        Ok(furigana_html)
    } else {
        Err(response
            .error
            .unwrap_or_else(|| "Anki Lookup add-on could not create furigana.".into()))
    }
}

/// Rejects anything the furigana endpoint returns that is not plain ruby markup.
///
/// Unlike every other Anki field, furigana cannot be `html_escape`d — the ruby
/// tags are the whole point of the feature. But port 8766 is unauthenticated, so
/// whatever local process binds it first is what we are reading, and Anki renders
/// card fields in QtWebEngine. A strict allowlist is what keeps that channel from
/// being stored XSS in the user's collection. Both callers treat a rejection as a
/// lookup miss and fall back to the plain text, so being over-strict costs a user
/// their furigana on one card; being under-strict costs them script execution
/// inside Anki.
fn validate_furigana_html(html: &str) -> Result<(), String> {
    let mut remaining = html;
    while let Some(tag_start) = remaining.find('<') {
        remaining = &remaining[tag_start + 1..];
        let tag_end = remaining
            .find('>')
            .ok_or_else(|| {
                "Anki Lookup add-on returned furigana with an unterminated tag.".to_string()
            })?;
        validate_furigana_tag(&remaining[..tag_end])?;
        remaining = &remaining[tag_end + 1..];
    }

    Ok(())
}

/// Validates one tag's innards (everything between `<` and `>`). Requiring the
/// name to be nothing but ASCII letters is what does the real work here: it
/// leaves no room for an attribute, so `onerror=`, `javascript:` hrefs, comments
/// and processing instructions are all rejected before the name is even checked.
fn validate_furigana_tag(tag: &str) -> Result<(), String> {
    let name = tag.trim();
    let name = name.strip_prefix('/').unwrap_or(name);
    let name = name.strip_suffix('/').unwrap_or(name).trim();

    if name.is_empty() || !name.chars().all(|value| value.is_ascii_alphabetic()) {
        return Err(format!(
            "Anki Lookup add-on returned furigana with unexpected markup: <{tag}>"
        ));
    }

    if !ALLOWED_FURIGANA_TAGS.contains(&name.to_ascii_lowercase().as_str()) {
        return Err(format!(
            "Anki Lookup add-on returned furigana with a disallowed <{name}> tag."
        ));
    }

    Ok(())
}

/// Writes validated furigana over the mapped transcription field, keeping any
/// `[sound:...]` tag the field already carries. Shared by the mine and push flows,
/// which differ only in how they report the outcome.
pub(super) fn insert_furigana_field(
    settings: &AnkiSettings,
    furigana_html: &str,
    media_file_name: &str,
    fields: &mut serde_json::Map<String, serde_json::Value>,
) {
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
    use super::validate_furigana_html;

    #[test]
    fn accepts_plain_ruby_markup() {
        assert!(validate_furigana_html("<ruby>漢字<rt>かんじ</rt></ruby>").is_ok());
        assert!(validate_furigana_html(
            "<ruby><rb>今日</rb><rp>(</rp><rt>きょう</rt><rp>)</rp></ruby>は"
        )
        .is_ok());
        // Multi-line transcripts reach Anki with <br> between the lines.
        assert!(validate_furigana_html("一行目<br>二行目").is_ok());
        assert!(validate_furigana_html("<ruby>猫<rt>ねこ</rt></ruby><br/>").is_ok());
        // Text with no markup at all is what an all-kana sentence returns.
        assert!(validate_furigana_html("ねこがすきです").is_ok());
    }

    #[test]
    fn rejects_script_payloads_from_the_unauthenticated_endpoint() {
        assert!(validate_furigana_html("<script>alert(1)</script>").is_err());
        assert!(validate_furigana_html("<ruby>漢字<rt>かんじ</rt></ruby><script>steal()</script>")
            .is_err());
        assert!(validate_furigana_html("<SCRIPT>alert(1)</SCRIPT>").is_err());
    }

    #[test]
    fn rejects_attributes_event_handlers_and_javascript_urls() {
        assert!(validate_furigana_html("<ruby onclick=\"alert(1)\">漢字</ruby>").is_err());
        assert!(validate_furigana_html("<img src=x onerror=alert(1)>").is_err());
        assert!(validate_furigana_html("<a href=\"javascript:alert(1)\">x</a>").is_err());
        assert!(validate_furigana_html("<iframe src=\"http://evil\"></iframe>").is_err());
        assert!(validate_furigana_html("<!-- <ruby>x</ruby> -->").is_err());
        assert!(validate_furigana_html("<ruby").is_err());
    }

    #[test]
    fn allows_unbalanced_ruby_tags() {
        // Nesting is a rendering concern, not a safety one, so the allowlist
        // deliberately does not track open/close pairs.
        assert!(validate_furigana_html("<ruby>漢字<rt>かんじ</rt>").is_ok());
    }
}
