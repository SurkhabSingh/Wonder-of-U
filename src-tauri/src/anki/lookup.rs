use std::time::Duration;

use serde::{Deserialize, Serialize};

const ANKI_LOOKUP_URL: &str = "http://127.0.0.1:8766/lookup";
const ANKI_DICTIONARIES_URL: &str = "http://127.0.0.1:8766/dictionaries";
const LOOKUP_TIMEOUT: Duration = Duration::from_millis(4000);
/// The dictionary listing's own budget, far longer than a lookup's.
const DICTIONARIES_TIMEOUT: Duration = Duration::from_secs(15);

const DICTIONARIES_CONNECT_TIMEOUT: Duration = Duration::from_millis(750);

const MAX_TERM_LENGTH: usize = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LookupFrequency {
    #[serde(default)]
    pub(crate) dictionary: String,
    #[serde(default, rename(deserialize = "display_value"))]
    pub(crate) display_value: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct LookupPitch {
    #[serde(default)]
    pub(crate) position: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LookupEntry {
    #[serde(default)]
    pub(crate) expression: String,
    #[serde(default)]
    pub(crate) reading: String,
    #[serde(default)]
    pub(crate) dictionary: String,
    #[serde(default)]
    pub(crate) definitions: Vec<String>,
    #[serde(default, rename(deserialize = "inflection_reasons"))]
    pub(crate) inflection_reasons: Vec<String>,
    #[serde(default)]
    pub(crate) frequencies: Vec<LookupFrequency>,
    #[serde(default, rename(deserialize = "pitch_accents"))]
    pub(crate) pitch_accents: Vec<LookupPitch>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LookupResult {
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) term: String,
    pub(crate) entries: Vec<LookupEntry>,
}

#[derive(Debug, Deserialize)]
struct LookupBridgeResponse {
    ok: bool,
    #[serde(default)]
    status: Option<String>,
    #[serde(default)]
    term: Option<String>,
    #[serde(default)]
    entries: Vec<LookupEntry>,
    #[serde(default)]
    error: Option<String>,
}

/// Every prefix of `text` from `offset`, longest first.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LookupDictionary {
    pub(crate) id: i64,
    #[serde(default)]
    pub(crate) title: String,
    #[serde(default)]
    pub(crate) revision: String,
    #[serde(default)]
    pub(crate) enabled: bool,
    #[serde(default)]
    pub(crate) priority: i64,
    #[serde(default)]
    pub(crate) term_count: i64,
}

/// What the add-on has installed. `status` is `ready` or `unavailable` — Anki being
/// closed is an ordinary state here, exactly as it is for a lookup.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct LookupDictionaries {
    pub(crate) status: String,
    pub(crate) message: String,
    pub(crate) dictionaries: Vec<LookupDictionary>,
}

#[derive(Debug, Deserialize)]
struct DictionariesBridgeResponse {
    ok: bool,
    #[serde(default)]
    dictionaries: Vec<LookupDictionary>,
    #[serde(default)]
    error: Option<String>,
}

const DICTIONARIES_SILENT: &str =
    "Anki isn't answering. Open Anki, and check that the Anki Lookup add-on is installed and enabled.";
const DICTIONARIES_BUSY: &str =
    "Anki is busy and didn't answer in time. Use Refresh Anki above to try again.";
const DICTIONARIES_ADDON_TOO_OLD: &str =
    "Your Anki Lookup add-on is too old to list dictionaries. Update it, then restart Anki.";
const DICTIONARIES_UNREADABLE: &str =
    "Your dictionaries couldn't be listed. Restart Anki and try again.";

/// A listing that carries a reason instead of dictionaries.
fn dictionaries_unavailable(message: &str) -> LookupDictionaries {
    LookupDictionaries {
        status: "unavailable".into(),
        message: message.into(),
        dictionaries: Vec::new(),
    }
}

/// Lists the dictionaries the add-on can answer from.
pub(crate) fn lookup_dictionaries_inner() -> Result<LookupDictionaries, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(DICTIONARIES_TIMEOUT)
        .connect_timeout(DICTIONARIES_CONNECT_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;

    let response = match client.get(ANKI_DICTIONARIES_URL).send() {
        Ok(response) => response,
        Err(error) if error.is_connect() => return Ok(dictionaries_unavailable(DICTIONARIES_SILENT)),
        Err(error) if error.is_timeout() => return Ok(dictionaries_unavailable(DICTIONARIES_BUSY)),
        Err(_) => return Ok(dictionaries_unavailable(DICTIONARIES_SILENT)),
    };

    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(dictionaries_unavailable(DICTIONARIES_ADDON_TOO_OLD));
    }

    let body = match response.text() {
        Ok(body) => body,
        Err(error) if error.is_timeout() => return Ok(dictionaries_unavailable(DICTIONARIES_BUSY)),
        Err(_) => return Ok(dictionaries_unavailable(DICTIONARIES_UNREADABLE)),
    };
    let Ok(parsed) = serde_json::from_str::<DictionariesBridgeResponse>(&body) else {
        return Ok(dictionaries_unavailable(DICTIONARIES_UNREADABLE));
    };
    if !parsed.ok {
        return Err(parsed
            .error
            .unwrap_or_else(|| "The dictionaries could not be listed.".into()));
    }

    Ok(LookupDictionaries {
        status: "ready".into(),
        message: String::new(),
        dictionaries: parsed.dictionaries,
    })
}

pub(crate) fn lookup_candidates(text: &str, offset: usize) -> Vec<String> {
    let characters = text.chars().collect::<Vec<_>>();
    if offset >= characters.len() {
        return Vec::new();
    }
    let end = characters.len().min(offset + MAX_TERM_LENGTH);
    let mut candidates = Vec::with_capacity(end - offset);
    for length in (1..=end - offset).rev() {
        let candidate: String = characters[offset..offset + length].iter().collect();
        if !candidate.trim().is_empty() {
            candidates.push(candidate);
        }
    }
    candidates
}

/// Looks a word up directly, without offering the add-on any prefixes.
pub(super) fn lookup_exact_word(
    word: &str,
    limit: u32,
    dictionary_ids: &[i64],
) -> Result<LookupResult, String> {
    post_lookup(word, &[word.to_string()], word, limit, dictionary_ids)
}

pub(crate) fn lookup_term_inner(
    text: String,
    offset: usize,
    limit: Option<u32>,
) -> Result<LookupResult, String> {
    let candidates = lookup_candidates(&text, offset);
    let Some(term) = candidates.last().cloned() else {
        return Ok(LookupResult {
            status: "empty".into(),
            message: "Nothing to look up there.".into(),
            term: String::new(),
            entries: Vec::new(),
        });
    };

    post_lookup(&term, &candidates, &text, limit.unwrap_or(20), &[])
}

fn post_lookup(
    term: &str,
    candidates: &[String],
    sentence: &str,
    limit: u32,
    dictionary_ids: &[i64],
) -> Result<LookupResult, String> {
    let term = term.to_string();
    let client = reqwest::blocking::Client::builder()
        .timeout(LOOKUP_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;

    let mut payload = serde_json::json!({
        "term": term,
        "candidates": candidates,
        "sentence": sentence,
        "limit": limit,
    });
    if !dictionary_ids.is_empty() {
        payload["dictionaryIds"] = serde_json::json!(dictionary_ids);
    }

    let response = match client
        .post(ANKI_LOOKUP_URL)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(payload.to_string())
        .send()
    {
        Ok(response) => response,
        Err(_) => {
            return Ok(LookupResult {
                status: "unavailable".into(),
                message: "Open Anki to look words up — the dictionary lives in the add-on."
                    .into(),
                term,
                entries: Vec::new(),
            })
        }
    };

    let response_text = response
        .text()
        .map_err(|error| format!("The dictionary response could not be read. {error}"))?;
    let parsed = serde_json::from_str::<LookupBridgeResponse>(&response_text)
        .map_err(|error| format!("The dictionary returned unreadable data. {error}"))?;

    if !parsed.ok {
        return Err(parsed
            .error
            .unwrap_or_else(|| "The dictionary could not look that up.".into()));
    }

    let matched = parsed.term.unwrap_or(term);
    let entries = parsed.entries;
    let status = parsed
        .status
        .unwrap_or_else(|| if entries.is_empty() { "empty" } else { "ready" }.into());
    Ok(LookupResult {
        status,
        message: if entries.is_empty() {
            format!("No dictionary entry for {matched}.")
        } else {
            String::new()
        },
        term: matched,
        entries,
    })
}

#[cfg(test)]
mod tests {
    use super::{lookup_candidates, LookupBridgeResponse, LookupEntry};

    #[test]
    fn every_reason_a_listing_failed_is_copy_this_app_wrote() {
        // The settings page renders `message` as the section's only text, so none of
        // these may carry tool wording — no serde parse errors, no reqwest internals,
        // no Python exception text from the add-on.
        let reasons = [
            super::DICTIONARIES_SILENT,
            super::DICTIONARIES_BUSY,
            super::DICTIONARIES_ADDON_TOO_OLD,
            super::DICTIONARIES_UNREADABLE,
        ];
        for reason in reasons {
            assert!(
                !reason.is_empty() && reason.ends_with('.'),
                "a reason shown to a user is a sentence: {reason}"
            );
            for jargon in ["error", "Err", "reqwest", "serde", "http", "404", "None"] {
                assert!(
                    !reason.contains(jargon),
                    "{reason:?} leaks the word {jargon:?} at the user"
                );
            }
        }
        // Each names a different situation, so none is a copy-paste of another.
        for (index, reason) in reasons.iter().enumerate() {
            for other in &reasons[index + 1..] {
                assert_ne!(reason, other);
            }
        }
        // A busy Anki must never be told to open Anki — that was the original fault.
        assert!(!super::DICTIONARIES_BUSY.contains("Open Anki"));
        // And a silent one has to cover BOTH ways of being silent, since the app
        // cannot tell a closed Anki from a running one with no add-on.
        assert!(super::DICTIONARIES_SILENT.contains("add-on"));
    }

    #[test]
    fn an_unavailable_listing_carries_the_reason_and_no_dictionaries() {
        let listing = super::dictionaries_unavailable(super::DICTIONARIES_BUSY);
        assert_eq!(listing.status, "unavailable");
        assert_eq!(listing.message, super::DICTIONARIES_BUSY);
        assert!(listing.dictionaries.is_empty());
    }

    /// The two ends of this type speak different conventions — the add-on sends
    /// snake_case, the webview reads camelCase — and getting it wrong shows up as an
    /// entry that renders blank rather than as any kind of failure. Pin both directions.
    #[test]
    fn entries_read_snake_case_and_are_sent_as_camel_case() {
        let from_addon = r#"{
            "ok": true,
            "status": "ready",
            "term": "単品",
            "entries": [{
                "expression": "単品",
                "reading": "たんぴん",
                "dictionary": "JMdict",
                "definitions": ["single item"],
                "inflection_reasons": ["past"],
                "frequencies": [{"dictionary": "大辞泉", "display_value": "1234"}],
                "pitch_accents": [{"position": 0}]
            }]
        }"#;

        let parsed = serde_json::from_str::<LookupBridgeResponse>(from_addon)
            .expect("the add-on's own snake_case payload must deserialize");
        let entry = parsed.entries.first().expect("one entry");
        assert_eq!(entry.inflection_reasons, vec!["past".to_string()]);
        assert_eq!(entry.pitch_accents.len(), 1);
        assert_eq!(
            entry.frequencies[0].display_value.as_deref(),
            Some("1234")
        );

        let to_webview = serde_json::to_string(entry).expect("serializable");
        assert!(to_webview.contains("\"inflectionReasons\""));
        assert!(to_webview.contains("\"pitchAccents\""));
        assert!(to_webview.contains("\"displayValue\""));
        assert!(!to_webview.contains("\"inflection_reasons\""));
        assert!(!to_webview.contains("\"display_value\""));
    }

    /// Entries the add-on sends without the optional lists must not fail the whole lookup.
    #[test]
    fn missing_optional_fields_default_rather_than_erroring() {
        let entry = serde_json::from_str::<LookupEntry>(r#"{"expression": "犬"}"#)
            .expect("a bare entry is still an entry");
        assert_eq!(entry.expression, "犬");
        assert!(entry.definitions.is_empty());
        assert!(entry.pitch_accents.is_empty());
    }

    #[test]
    fn candidates_are_prefixes_longest_first() {
        // Longest first is what makes the backend's longest match win: 単品 beats 単.
        let candidates = lookup_candidates("単品でよかった", 0);
        assert_eq!(candidates.first().map(String::as_str), Some("単品でよかった"));
        assert_eq!(candidates.last().map(String::as_str), Some("単"));
        assert_eq!(candidates.len(), 7);
    }

    #[test]
    fn candidates_start_at_the_clicked_character() {
        let candidates = lookup_candidates("これは単品です", 3);
        assert_eq!(candidates.last().map(String::as_str), Some("単"));
        assert!(candidates.iter().all(|candidate| candidate.starts_with('単')));
    }

    #[test]
    fn candidates_are_capped_and_bounds_checked() {
        // Past the end is a normal thing to ask (a click lands after the last character).
        assert!(lookup_candidates("短い", 99).is_empty());
        assert!(lookup_candidates("", 0).is_empty());
        // Nothing longer than the cap is ever a word.
        let long = "あ".repeat(50);
        assert_eq!(lookup_candidates(&long, 0).len(), 20);
    }

    #[test]
    fn multibyte_text_is_sliced_by_character_not_byte() {
        // Byte slicing would panic or produce mojibake on Japanese.
        let candidates = lookup_candidates("日本語", 1);
        assert_eq!(candidates, vec!["本語".to_string(), "本".to_string()]);
    }
}

#[cfg(test)]
mod bridge_tests {
    /// Asks the real add-on for its dictionaries.
    #[test]
    #[ignore = "requires Anki running with the lookup add-on"]
    fn dictionaries_from_the_real_addon() {
        let listing = super::lookup_dictionaries_inner().expect("the bridge should answer");
        println!("  status={} count={}", listing.status, listing.dictionaries.len());
        for entry in &listing.dictionaries {
            println!(
                "    id={:<3} prio={:<3} enabled={:<5} terms={:<9} {}",
                entry.id, entry.priority, entry.enabled, entry.term_count, entry.title
            );
        }
        assert_eq!(listing.status, "ready");
        assert!(!listing.dictionaries.is_empty());

        // The field the desktop keys everything on. A zero here would mean the
        // camelCase mapping silently dropped it and every id would collide.
        assert!(listing.dictionaries.iter().all(|entry| entry.id > 0));
        assert!(listing.dictionaries.iter().any(|entry| entry.term_count > 0));
    }
}

#[cfg(test)]
mod serialization_tests {
    /// What the webview actually receives. The struct is deserialized from the
    /// add-on AND serialized to the frontend through the same `rename_all`, so a
    /// key that is wrong in one direction is invisible in the other.
    #[test]
    fn a_dictionary_reaches_the_frontend_with_every_field() {
        let listing = super::LookupDictionaries {
            status: "ready".into(),
            message: String::new(),
            dictionaries: vec![super::LookupDictionary {
                id: 3,
                title: "JMdict [2025-11-01]".into(),
                revision: "r1".into(),
                enabled: true,
                priority: 2,
                term_count: 513033,
            }],
        };
        let json = serde_json::to_string(&listing).unwrap();
        println!("  {json}");
        assert!(json.contains(r#""title":"JMdict [2025-11-01]""#), "{json}");
        assert!(json.contains(r#""termCount":513033"#), "{json}");
    }
}
