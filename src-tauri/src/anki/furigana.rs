use std::time::Duration;

use serde::Deserialize;

use crate::{
    app_state::{is_japanese_transcript_language, transcript_looks_japanese},
    app_types::RecentRecording,
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
        response
            .furigana_html
            .ok_or_else(|| "Anki Lookup add-on did not return furigana HTML.".to_string())
    } else {
        Err(response
            .error
            .unwrap_or_else(|| "Anki Lookup add-on could not create furigana.".into()))
    }
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
