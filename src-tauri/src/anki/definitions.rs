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
    lookup::{lookup_exact_word, LookupEntry},
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

/// How many entries to ASK for. Higher than the cap so that entries dropped by
/// `entry_is_about`, or by a dictionary the user has excluded, leave enough behind.
const LOOKUP_LIMIT: u32 = 12;

/// How much of one gloss to keep.
///
/// A monolingual dictionary hands back the whole article — 旺文社's entry for a
/// common kanji runs to stroke order, compounds and several senses, newlines and
/// all. Whole, it is a page; cut, it is the definition.
const MAX_GLOSS_CHARS: usize = 220;

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

/// Flattens one gloss to a single line and cuts it to length.
///
/// Newlines first: the add-on joins a dictionary's senses with them, and left
/// alone they collapse in HTML anyway, welding 「① 一個…」「② セット…」 into one
/// run of text without even a space between.
fn tidy_gloss(gloss: &str) -> String {
    let flattened = gloss.split_whitespace().collect::<Vec<_>>().join(" ");
    if flattened.chars().count() <= MAX_GLOSS_CHARS {
        return flattened;
    }
    let cut: String = flattened.chars().take(MAX_GLOSS_CHARS).collect();
    format!("{cut}…")
}

/// Renders one word's entries as the card will show them.
///
/// The reading rides with the headword rather than in its own column, because a
/// card field is read as prose and 修理【しゅうり】 is how a dictionary prints it.
/// The dictionary's name is kept: with several installed, which one a gloss came
/// from is part of judging it.
fn entries_html(word: &str, entries: &[LookupEntry]) -> Option<String> {
    // Taken as the add-on returned them, in its priority order.
    //
    // There used to be a filter here rejecting any entry whose headword was not the
    // word asked for. It was written when the request carried every PREFIX of the
    // word and half the answers were about カフ; sending the exact word solved that,
    // and the filter then did nothing but harm — the add-on DEINFLECTS, so asking
    // about 出会える correctly answers 出会う【であう】 with `inflection_reasons:
    // ["potential"]`, and the filter threw both entries away and left the card blank.
    // The add-on knows better than this module which entries are about the word.
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
                .map(|gloss| escape(&tidy_gloss(gloss)))
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
                    " <span class=\"wu-dict\">({})</span>",
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
            "<div class=\"wu-definition\"><b>{}</b><ul>{}</ul></div>",
            escape(word),
            rendered.join("")
        )
    })
}

/// What a definitions attempt produced.
///
/// Three states, not two, because "there was nothing to add" and "it could not be
/// fetched" are different things to tell the user. The first is an ordinary card;
/// the second is a card missing something they switched on and expect to be there.
pub(super) enum Definitions {
    /// Nothing to write, and nothing wrong: no new words in the line, or the
    /// feature is not set up far enough to know.
    NothingToAdd,
    /// Something to write, and possibly some words the dictionaries had nothing for.
    /// Both, because a card can carry one meaning and be missing another, and only
    /// saying which is missing lets the reader judge whether to widen their choice.
    Ready {
        html: String,
        missing: Vec<String>,
    },
    /// Asked for and not obtained.
    Unavailable(String),
}

/// Looks up every word this line is meant to teach and renders them for the card.
///
/// Never returns an error. A definition is something added to a card, and failing
/// to add it must not be a reason the card is not made — but it IS a reason to say
/// so, which is what `Unavailable` is for. The first version of this collapsed
/// every outcome into `None`, and a card silently missing what the toggle promised
/// is indistinguishable from a toggle that does nothing.
pub(super) fn definitions_for<R: Runtime>(
    app: &AppHandle<R>,
    line: &str,
    dictionary_ids: &[i64],
) -> Definitions {
    let words = match line_unknown_words(app, line) {
        Ok(words) => words,
        Err(error) => return Definitions::Unavailable(error),
    };
    if words.is_empty() {
        return Definitions::NothingToAdd;
    }
    if !lookups_are_worth_attempting() {
        return Definitions::Unavailable(
            "the dictionary was not answering a moment ago".into(),
        );
    }

    let mut sections = Vec::new();
    let mut missing = Vec::new();
    let mut problem = None;
    for word in words {
        match lookup_exact_word(&word, LOOKUP_LIMIT, dictionary_ids) {
            Ok(result) if result.status == "ready" || result.status == "empty" => {
                note_lookups_available();
                match entries_html(&word, &result.entries) {
                    Some(html) => sections.push(html),
                    // A word the chosen dictionaries simply do not have. Recorded
                    // rather than passed over: "no entry for this" and "the lookup
                    // failed" are different answers, and until this was reported the
                    // only symptom of either was a card that looked untouched.
                    None => missing.push(word.clone()),
                }
            }
            // Anything else means the add-on did not answer. Stop for a while
            // rather than paying the timeout again on every remaining line.
            Ok(result) => {
                note_lookups_unavailable();
                problem = Some(result.message);
                break;
            }
            Err(error) => {
                note_lookups_unavailable();
                problem = Some(error);
                break;
            }
        }
    }

    match (sections.is_empty(), problem) {
        (false, _) => Definitions::Ready {
            html: sections.join(""),
            missing,
        },
        (true, Some(problem)) => Definitions::Unavailable(problem),
        // Nothing rendered and nothing broke: the chosen dictionaries have no entry
        // for any of these words. Said out loud, because the alternative is a card
        // that looks exactly like the feature being switched off.
        (true, None) if !missing.is_empty() => Definitions::Unavailable(format!(
            "your chosen dictionaries have no entry for {}",
            missing.join("、")
        )),
        (true, None) => Definitions::NothingToAdd,
    }
}

#[cfg(test)]
mod tests {
    use super::{
        entries_html, escape, tidy_gloss, ENTRIES_PER_WORD, GLOSSES_PER_ENTRY,
        MAX_GLOSS_CHARS,
    };
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

    /// The bug that left cards blank, and the reason there is no headword filter.
    ///
    /// The add-on deinflects: asking about 出会える answers 出会う【であう】 with
    /// `inflection_reasons: ["potential"]`. A filter comparing the headword to the
    /// word asked for threw exactly those entries away — the useful ones — and the
    /// card came out empty with nothing to say why.
    #[test]
    fn a_deinflected_entry_is_kept() {
        let html = entries_html(
            "出会える",
            &[entry("出会う", "であう", "JMdict", &["to meet"])],
        )
        .expect("the add-on deinflected to this entry, so it is about the word");
        assert!(html.contains("出会う"), "{html}");
        assert!(html.contains("to meet"), "{html}");
        // Titled by the word that was looked up, so the card names the word being
        // learned rather than the dictionary's spelling of its base form.
        assert!(html.contains("<b>出会える</b>"), "{html}");
    }

    /// A kana word's entry is filed under its kanji, and that is the entry wanted.
    #[test]
    fn an_entry_found_under_its_kanji_is_kept() {
        let html = entries_html(
            "わかる",
            &[entry("分かる", "わかる", "JMdict", &["to understand"])],
        )
        .unwrap();
        assert!(html.contains("to understand"), "{html}");
    }

    /// Which entries are about the word is the add-on's judgement, not this
    /// module's — it deinflects and ranks, and second-guessing it is what produced
    /// the blank cards. Narrowing is the dictionary picker's job instead.
    #[test]
    fn entries_are_taken_in_the_order_the_addon_returned_them() {
        let html = entries_html(
            "本",
            &[
                entry("本", "ほん", "First", &["book"]),
                entry("本", "ほん", "Second", &["origin"]),
            ],
        )
        .unwrap();
        assert!(html.find("First") < html.find("Second"), "{html}");
    }

    /// A monolingual entry arrives as a whole article    /// A monolingual entry arrives as a whole article, newlines and all. Left as
    /// they are, HTML collapses them and welds the senses into one run of text.
    #[test]
    fn a_gloss_is_flattened_and_cut_to_length() {
        assert_eq!(tidy_gloss("① one。
② two。"), "① one。 ② two。");
        assert_eq!(tidy_gloss("  spaced 

 out  "), "spaced out");

        let long = "あ".repeat(MAX_GLOSS_CHARS + 50);
        let cut = tidy_gloss(&long);
        assert_eq!(cut.chars().count(), MAX_GLOSS_CHARS + 1, "cut plus the ellipsis");
        assert!(cut.ends_with('…'));
    }

    /// A card is glanced at, not scrolled. Eighteen senses from seven dictionaries
    /// is a wall that gets skipped rather than a definition that gets read.
    #[test]
    fn entries_and_glosses_are_capped() {
        // All about 本, as the prefix filter now requires, but from distinct
        // dictionaries — so a cap test cannot pass by deduplication it does not do.
        let many: Vec<LookupEntry> = (0..10)
            .map(|index| {
                entry(
                    "本",
                    "ほん",
                    &format!("Dictionary {index}"),
                    &["one", "two", "three", "four", "five", "six"],
                )
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

    /// Asks the real add-on for real words and prints what comes back.
    ///
    /// The unit tests above cover the rendering, which is not the half that can be
    /// wrong about the outside world. Needs Anki running with the lookup add-on.
    ///
    ///   cargo test explain_a_real_lookup -- --ignored --nocapture
    #[test]
    #[ignore = "requires Anki running with the lookup add-on"]
    fn explain_a_real_lookup() {
        use crate::anki::lookup::lookup_exact_word;

        // Through the card path, not the scanner's. The scanner sends every prefix
        // and its answers include them; a card never does, and a diagnostic that
        // showed カフ under カフェ would be reporting a problem cards do not have.
        for word in ["単品", "カフェ", "卵", "わかる", "おすすめ", "出会える"] {
            match lookup_exact_word(word, super::LOOKUP_LIMIT, &[]) {
                Ok(result) => {
                    println!(
                        "  {word:<8} status={:<12} term={:<10} entries={}",
                        result.status,
                        result.term,
                        result.entries.len()
                    );
                    for entry in result.entries.iter().take(2) {
                        println!(
                            "        {} [{}] ({}) {:?}",
                            entry.expression,
                            entry.reading,
                            entry.dictionary,
                            entry.definitions.first()
                        );
                    }
                    println!("        rendered: {:?}", super::entries_html(word, &result.entries));
                }
                Err(error) => println!("  {word:<8} ERROR {error}"),
            }
        }
    }

}
