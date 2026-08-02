//! Dictionary definitions for the words a mined card is meant to teach.
//!
//! A card made from an i+1 line exists because of one word. This looks that word
//! up in the dictionary the popup already uses and writes what it finds onto the
//! card, so the answer is on the card rather than one lookup away at review time.
//!
//! Which words those are is not asked of the caller — it is re-derived here from
//! the line, the tokenizer and the known-word index, exactly as the badge derives
//! it. Passing them in would mean a card whose definitions could disagree with the
//! badge that recommended it.

use std::sync::Mutex;

use tauri::{AppHandle, Runtime};

use crate::app_runtime::now_ms;

use super::{
    lookup::{lookup_term_inner, LookupEntry},
    sentence_ranking::line_unknown_words,
};

/// How many dictionary entries to keep per word.
///
/// The popup shows twenty because it is being read interactively and scrolled.
/// A card is glanced at during a review, and eighteen senses from seven
/// dictionaries is not a card anyone reads — it is a wall that gets skipped.
const ENTRIES_PER_WORD: usize = 3;

/// How many glosses to keep from one entry, for the same reason.
const GLOSSES_PER_ENTRY: usize = 4;

/// How long to stop attempting lookups after one fails.
///
/// The lookup waits up to four seconds, and the usual reason for failing is that
/// the add-on is not running — which will still be true for the next line. Without
/// this, mining forty lines with the add-on down would spend nearly three minutes
/// discovering the same thing forty times. One failure answers for the next minute.
const BACKOFF_MS: u64 = 60_000;

static UNAVAILABLE_UNTIL_MS: Mutex<Option<u64>> = Mutex::new(None);

fn lookups_are_worth_attempting() -> bool {
    let Ok(until) = UNAVAILABLE_UNTIL_MS.lock() else {
        // A poisoned lock degrades to trying, not to silently never enriching a
        // card again for the life of the process.
        return true;
    };
    match *until {
        Some(until_ms) => now_ms() >= until_ms,
        None => true,
    }
}

fn note_lookups_unavailable() {
    if let Ok(mut until) = UNAVAILABLE_UNTIL_MS.lock() {
        *until = Some(now_ms() + BACKOFF_MS);
    }
}

fn note_lookups_available() {
    if let Ok(mut until) = UNAVAILABLE_UNTIL_MS.lock() {
        *until = None;
    }
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Renders one word's entries as the card will show them.
///
/// The reading rides with the headword rather than in its own column, because a
/// card field is read as prose and 修理【しゅうり】 is how a dictionary prints it.
/// The dictionary's name is kept: with several installed, which one a gloss came
/// from is part of judging it.
fn entries_html(word: &str, entries: &[LookupEntry]) -> Option<String> {
    let rendered: Vec<String> = entries
        .iter()
        .take(ENTRIES_PER_WORD)
        .filter_map(|entry| {
            let glosses: Vec<String> = entry
                .definitions
                .iter()
                .map(|gloss| gloss.trim())
                .filter(|gloss| !gloss.is_empty())
                .take(GLOSSES_PER_ENTRY)
                .map(escape)
                .collect();
            if glosses.is_empty() {
                return None;
            }
            let headword = if entry.reading.is_empty() || entry.reading == entry.expression {
                escape(&entry.expression)
            } else {
                format!("{}【{}】", escape(&entry.expression), escape(&entry.reading))
            };
            let source = if entry.dictionary.is_empty() {
                String::new()
            } else {
                format!(
                    " <span class=\"wou-dict\">({})</span>",
                    escape(&entry.dictionary)
                )
            };
            Some(format!(
                "<li><b>{headword}</b>{source}<br>{}</li>",
                glosses.join("; ")
            ))
        })
        .collect();

    (!rendered.is_empty()).then(|| {
        format!(
            "<div class=\"wou-definition\"><b>{}</b><ul>{}</ul></div>",
            escape(word),
            rendered.join("")
        )
    })
}

/// Looks up every word this line is meant to teach and renders them for the card.
///
/// `None` whenever there is nothing worth writing — no new words, no dictionary,
/// no add-on running. **Never an error**: a definition is something added to a
/// card, and failing to add it must never be a reason the card is not made. That
/// is the whole contract of this module.
pub(super) fn definitions_html<R: Runtime>(app: &AppHandle<R>, line: &str) -> Option<String> {
    if !lookups_are_worth_attempting() {
        return None;
    }

    let words = line_unknown_words(app, line).ok()?;
    if words.is_empty() {
        return None;
    }

    let mut sections = Vec::new();
    for word in words {
        match lookup_term_inner(word.clone(), 0, Some(ENTRIES_PER_WORD as u32)) {
            Ok(result) if result.status == "ready" => {
                note_lookups_available();
                if let Some(html) = entries_html(&word, &result.entries) {
                    sections.push(html);
                }
            }
            // `empty` is a working dictionary with nothing for this word — a real
            // answer, and no reason to stop asking about the next one.
            Ok(result) if result.status == "empty" => note_lookups_available(),
            // Anything else means the add-on did not answer. Stop for a while
            // rather than paying the timeout again on every remaining line.
            _ => {
                note_lookups_unavailable();
                break;
            }
        }
    }

    (!sections.is_empty()).then(|| sections.join(""))
}

#[cfg(test)]
mod tests {
    use super::{entries_html, escape, ENTRIES_PER_WORD, GLOSSES_PER_ENTRY};
    use crate::anki::lookup::LookupEntry;

    fn entry(expression: &str, reading: &str, dictionary: &str, glosses: &[&str]) -> LookupEntry {
        LookupEntry {
            expression: expression.into(),
            reading: reading.into(),
            dictionary: dictionary.into(),
            definitions: glosses.iter().map(|gloss| gloss.to_string()).collect(),
            inflection_reasons: Vec::new(),
            frequencies: Vec::new(),
            pitch_accents: Vec::new(),
        }
    }

    #[test]
    fn a_word_renders_with_its_reading_and_dictionary() {
        let html = entries_html("修理", &[entry("修理", "しゅうり", "JMdict", &["repair"])])
            .expect("an entry with a gloss must render");
        assert!(html.contains("修理【しゅうり】"), "{html}");
        assert!(html.contains("repair"), "{html}");
        assert!(html.contains("JMdict"), "{html}");
    }

    /// A kana word's reading IS its headword, and printing 見る【見る】 reads as a
    /// bug on the card.
    #[test]
    fn a_reading_identical_to_the_headword_is_not_repeated() {
        let html = entries_html("あの", &[entry("あの", "あの", "JMdict", &["that"])]).unwrap();
        assert!(!html.contains("【"), "{html}");
    }

    #[test]
    fn an_entry_with_no_glosses_is_dropped_rather_than_rendered_empty() {
        assert!(entries_html("本", &[entry("本", "ほん", "JMdict", &[])]).is_none());
        assert!(entries_html("本", &[entry("本", "ほん", "JMdict", &["  "])]).is_none());
        assert!(entries_html("本", &[]).is_none());
    }

    /// A card is glanced at, not scrolled. Eighteen senses from seven dictionaries
    /// is a wall that gets skipped rather than a definition that gets read.
    #[test]
    fn entries_and_glosses_are_capped() {
        let many: Vec<LookupEntry> = (0..10)
            .map(|index| {
                entry(
                    "本",
                    "ほん",
                    "JMdict",
                    &["one", "two", "three", "four", "five", "six"],
                )
                .tap_index(index)
            })
            .collect();
        let html = entries_html("本", &many).unwrap();
        assert_eq!(html.matches("<li>").count(), ENTRIES_PER_WORD);
        // Four glosses joined by "; " is three separators per entry.
        assert_eq!(
            html.matches(';').count(),
            (GLOSSES_PER_ENTRY - 1) * ENTRIES_PER_WORD
        );
    }

    /// A gloss is dictionary text going into an HTML field. Left raw, an entry
    /// containing `<` would break the card's markup.
    #[test]
    fn dictionary_text_cannot_inject_markup_into_the_card() {
        assert_eq!(escape("a < b & c"), "a &lt; b &amp; c");
        let html = entries_html(
            "本",
            &[entry("本", "ほん", "<script>", &["<b>bold</b> claim"])],
        )
        .unwrap();
        assert!(!html.contains("<script>"), "{html}");
        assert!(!html.contains("<b>bold"), "{html}");
    }

    trait TapIndex {
        fn tap_index(self, index: usize) -> Self;
    }
    impl TapIndex for LookupEntry {
        /// Distinct expressions, so a cap test cannot pass by deduplication it does
        /// not actually do.
        fn tap_index(mut self, index: usize) -> Self {
            self.expression = format!("本{index}");
            self
        }
    }
}
