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
/// The dictionary listing's own budget, far longer than a lookup's.
///
/// A lookup is interactive: a reading popup that waits longer than four seconds has
/// already failed the reader. The listing is a one-off settings call nobody is waiting
/// on mid-sentence, and it can legitimately take much longer than the read itself
/// suggests — it shares one SQLite file with the dictionary importer, and a reader
/// blocks behind the importer's write lock for as long as an import runs. Sharing the
/// popup's budget made "an import is in progress" indistinguishable from "Anki is
/// closed", and the settings page reported the second.
const DICTIONARIES_TIMEOUT: Duration = Duration::from_secs(15);
/// How long to wait for the port to accept at all. A refused connection is instant; a
/// bound-but-dead socket — Anki shutting down with a large collection — would
/// otherwise hold the full listing budget before saying anything.
const DICTIONARIES_CONNECT_TIMEOUT: Duration = Duration::from_millis(750);

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

/// Nobody answered on the port. True whether Anki is shut or running without the
/// add-on, which the app cannot tell apart and should not pretend to — so the sentence
/// covers both and gives a next step for each.
const DICTIONARIES_SILENT: &str =
    "Anki isn't answering. Open Anki, and check that the Anki Lookup add-on is installed and enabled.";
/// Anki is there and did not finish in time — an import or a sync holding the
/// dictionary file. Names the control that retries, because the section has none of
/// its own and the one that works is at the top of the page.
const DICTIONARIES_BUSY: &str =
    "Anki is busy and didn't answer in time. Use Refresh Anki above to try again.";
/// The add-on predates this endpoint. Reached by its 404 rather than by a parse
/// failure: the versions that lack it answer 404 with a perfectly parseable body.
const DICTIONARIES_ADDON_TOO_OLD: &str =
    "Your Anki Lookup add-on is too old to list dictionaries. Update it, then restart Anki.";
/// Something answered on the port and it was not the add-on, or the add-on failed
/// inside itself. Either way there is nothing the user can do about the detail, and the
/// detail is tool text.
const DICTIONARIES_UNREADABLE: &str =
    "Your dictionaries couldn't be listed. Restart Anki and try again.";

/// A listing that carries a reason instead of dictionaries.
///
/// Every message the settings page can show for a failed listing is built here, from
/// the constants above, and never from a tool's own words. The page renders whatever
/// `message` holds as the section's only copy, so a serde parse error or the add-on's
/// Python exception text would otherwise land under "Meanings come from" — against the
/// rule that this app's copy says what the app does, not how it is built.
fn dictionaries_unavailable(message: &str) -> LookupDictionaries {
    LookupDictionaries {
        status: "unavailable".into(),
        message: message.into(),
        dictionaries: Vec::new(),
    }
}

/// Lists the dictionaries the add-on can answer from.
///
/// Read-only. Which are enabled, and in what order, belongs to the add-on's own
/// dictionary manager; this only reports it so the app can offer a subset for
/// mined cards without changing what the reading popup sees.
///
/// Not answering is an ordinary state here, so every way of not answering resolves as
/// an `unavailable` listing rather than an error. The one exception is an add-on that
/// answered and reported its own failure, which is a real fault worth an `Err`.
pub(crate) fn lookup_dictionaries_inner() -> Result<LookupDictionaries, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(DICTIONARIES_TIMEOUT)
        .connect_timeout(DICTIONARIES_CONNECT_TIMEOUT)
        .build()
        .map_err(|error| error.to_string())?;

    let response = match client.get(ANKI_DICTIONARIES_URL).send() {
        Ok(response) => response,
        // `is_connect` is tested first: reqwest reports a connect TIMEOUT as a timeout
        // too, and a port that never accepted is a silent Anki, not a busy one.
        Err(error) if error.is_connect() => return Ok(dictionaries_unavailable(DICTIONARIES_SILENT)),
        Err(error) if error.is_timeout() => return Ok(dictionaries_unavailable(DICTIONARIES_BUSY)),
        Err(_) => return Ok(dictionaries_unavailable(DICTIONARIES_SILENT)),
    };

    // Checked before the body is parsed. An add-on without this endpoint answers 404
    // with a body that parses perfectly well, so a parse failure never identifies it.
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(dictionaries_unavailable(DICTIONARIES_ADDON_TOO_OLD));
    }

    let body = match response.text() {
        Ok(body) => body,
        // The budget covers the body too, so the same "Anki is busy" can land here.
        Err(error) if error.is_timeout() => return Ok(dictionaries_unavailable(DICTIONARIES_BUSY)),
        Err(_) => return Ok(dictionaries_unavailable(DICTIONARIES_UNREADABLE)),
    };
    let Ok(parsed) = serde_json::from_str::<DictionariesBridgeResponse>(&body) else {
        return Ok(dictionaries_unavailable(DICTIONARIES_UNREADABLE));
    };
    if !parsed.ok {
        // The add-on answered and said it failed. That is a genuine fault rather than
        // an ordinary state, so it stays an error — and its own wording never reaches
        // the page, which shows fixed copy for a rejected listing.
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
/// `dictionary_ids` narrows the answer to chosen dictionaries. Empty sends no
/// filter at all, which the add-on reads as every enabled one — the card enricher
/// never calls it that way, because for a card nothing chosen means nothing.
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
