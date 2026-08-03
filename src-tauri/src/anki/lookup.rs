//! Dictionary lookup for the subtitle scanner.
//!
//! The dictionary is not a file we read — it is a service. The 1.36M-term database, its
//! deinflection rule graph (食べた → 食べる) and its search ranking all live inside the
//! Anki add-on's Python process, reachable only over its local HTTP bridge. **Anki must
//! be running.** In practice that costs nothing: mining already needs Anki for
//! AnkiConnect. The alternative — reimplementing deinflection and ranking in Rust
//! against the SQLite file — would be a second copy of the hardest part, free to drift
//! from the original.
//!
//! This goes through Rust rather than straight from the webview because the app's CSP
//! `connect-src` forbids the webview reaching any host, exactly as the furigana call does.

use std::time::Duration;

use serde::{Deserialize, Serialize};

const ANKI_LOOKUP_URL: &str = "http://127.0.0.1:8766/lookup";
const ANKI_DICTIONARIES_URL: &str = "http://127.0.0.1:8766/dictionaries";
/// Longer than the furigana call: a lookup deinflects several candidates and ranks
/// entries across seven dictionaries, and it runs on Anki's UI thread.
const LOOKUP_TIMEOUT: Duration = Duration::from_millis(4000);

/// The longest run of characters offered as a single term. Matches the reviewer
/// scanner's own cap; beyond this the candidate is never a word.
const MAX_TERM_LENGTH: usize = 20;

/// Note the asymmetry on every multi-word field below: the add-on speaks snake_case and
/// the frontend speaks camelCase, so `rename` is scoped to `deserialize` only. A plain
/// `rename` would apply to both directions and quietly hand the webview snake_case keys
/// that its types say do not exist — which reads as an empty popup, not as an error.
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

/// One dictionary entry. Deliberately a SUBSET of what the add-on returns: the popup
/// shows the headword, reading, glosses and why the form matched, and carrying the rest
/// would mean tracking a schema we do not use.
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
    /// Why a conjugated form matched its dictionary form, e.g. ["past"]. Shown so a
    /// learner can see 食べた came from 食べる rather than guessing.
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
    /// "ready" | "empty" | "unavailable". `unavailable` means Anki is not running, which
    /// is an ordinary state rather than an error — the panel says so and moves on.
    pub(crate) status: String,
    pub(crate) message: String,
    /// The candidate that actually matched, which is what should be highlighted in the
    /// sentence — it is usually longer than the single character that was clicked.
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
///
/// This is how Yomitan and the add-on's own reviewer scanner work, and it is why no
/// morphological analyser is needed: the backend deinflects each candidate and the
/// longest dictionary hit wins. Splitting the sentence into words first would be a
/// second, worse segmenter.
/// One dictionary installed in the add-on.
///
/// `priority` is the order lookups consult them in, which is why it is carried
/// rather than dropped: it is what explains why an answer came from one dictionary
/// rather than another.
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

/// Lists the dictionaries the add-on can answer from.
///
/// Read-only. Which are enabled, and in what order, belongs to the add-on's own
/// dictionary manager; this only reports it so the app can offer a subset for
/// mined cards without changing what the reading popup sees.
pub(crate) fn lookup_dictionaries_inner() -> Result<LookupDictionaries, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(LOOKUP_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;

    let response = match client.get(ANKI_DICTIONARIES_URL).send() {
        Ok(response) => response,
        Err(_) => {
            return Ok(LookupDictionaries {
                status: "unavailable".into(),
                message: "Open Anki to see your dictionaries — they live in the add-on.".into(),
                dictionaries: Vec::new(),
            })
        }
    };

    let body = response
        .text()
        .map_err(|error| format!("The dictionary list could not be read. {error}"))?;
    let parsed = serde_json::from_str::<DictionariesBridgeResponse>(&body).map_err(|error| {
        // An add-on too old to have the endpoint answers 404 with a body this cannot
        // parse, so say what to do rather than showing a parse error.
        format!("Your Anki add-on is too old to list dictionaries. Update it and restart Anki. ({error})")
    })?;
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
///
/// The scanner cannot know where a word ends — someone clicked into the middle of a
/// sentence — so it hands over every prefix and lets the add-on pick. The card
/// enricher is in the opposite position: the word came out of the tokenizer, so it
/// is already exactly one word.
///
/// Sending prefixes there was a bug with a visible symptom. Measured against the
/// real add-on: asking about カフェ WITH prefixes answers カフェ, カフ and カ — the
/// middle one a manga character — while asking with the word alone answers カフェ
/// five times over and nothing else.
///
/// `dictionary_ids` narrows the answer to chosen dictionaries; empty means every
/// enabled one, exactly as before this existed.
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

    // No dictionary filter: the scanner reads what the user reads while immersing,
    // which is what the add-on's own priority order is for.
    post_lookup(&term, &candidates, &text, limit.unwrap_or(20), &[])
}

/// The one request both callers make, so the two can never answer in different
/// shapes — the same reason the add-on serializes both its consumers through
/// `lookup_result`.
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
    // Omitted entirely when empty rather than sent as `[]`. Both mean the same thing
    // to the add-on, but a request that carries no filter is one that provably cannot
    // be filtered — and this is the request the verified scanner makes.
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
        // Anki closed is the common case, not a failure worth an error dialog.
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
    // The add-on already decides ready-vs-empty; take its answer rather than deriving a
    // second one that could disagree with it.
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
    ///
    /// The client had never been exercised end to end — the settings toggle that
    /// triggers it was off, so the first "it does not work" report could not tell a
    /// broken client from a hidden UI. Needs Anki running.
    ///
    ///   cargo test dictionaries_from_the_real_addon -- --ignored --nocapture
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
