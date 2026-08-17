//! How many words in a line are new to you — the number the whole feature exists
//! to show.
//!
//! The count has to mean what a learner means by it, and the gap between those two
//! is entirely part of speech. Every Japanese sentence contains は, が, を, に and
//! です; counting those would put a floor of four or five under every line and make
//! the number say nothing. So only content words count, and `is_content_word` is
//! where that judgement lives.

use std::{collections::HashSet, path::Path};

use tauri::{AppHandle, Manager, Runtime};

use crate::{
    app_types::{
        KnownWordsBuild, KnownWordsState, LineRanking, SharedPersistedState, TranscriptRanking,
    },
    runtime_assets::find_managed_dictionary_root,
    tokenizer::{tokenize_japanese, JapaneseToken},
};

use super::known_words::normalize_expression;

/// Parts of speech that are vocabulary — things you look up and learn.
///
/// 連体詞 (adnominal: この, あの) and 接続詞 (conjunction: しかし, でも) are in because
/// they are closed classes a learner genuinely studies and would not know at the
/// start. 感動詞 (interjection) is out: it is はい, ええ, あっ — meaningful in speech,
/// not vocabulary, and disproportionately common in the conversational audio this
/// app transcribes.
const CONTENT_PARTS_OF_SPEECH: [&str; 6] = [
    "名詞",   // noun
    "動詞",   // verb
    "形容詞", // i-adjective
    "副詞",   // adverb
    "連体詞", // adnominal
    "接続詞", // conjunction
];

/// Noun subcategories that are not vocabulary, however much they look like nouns.
///
/// - 固有名詞 is a name. A character called 田中 is not a word to learn, and counting
///   names would make every line of dialogue naming someone read as harder.
/// - 数 is a numeral. 三 and 百 are not what anyone means by an unknown word.
/// - 非自立 is a bound noun — こと, もの, ため — which cannot stand alone and is
///   grammar wearing a noun's clothes.
/// - 接尾 is a suffix: さん, 的, 化. Same reason.
/// - 代名詞 is これ, それ, 私: closed-class, and known from the first week.
const EXCLUDED_NOUN_SUBCATEGORIES: [&str; 5] =
    ["固有名詞", "数", "非自立", "接尾", "代名詞"];

/// Verb and adjective subcategories that are grammar rather than vocabulary:
/// the いる of ている, the くる of てくる, the ない of 食べない.
const EXCLUDED_DEPENDENT_SUBCATEGORY: &str = "非自立";

/// Whether this token is a word someone would study.
pub(super) fn is_content_word(token: &JapaneseToken) -> bool {
    // A word IPADIC has never seen is not evidence of vocabulary. It is a
    // mistranscription, a name, a stutter, or a foreign word the transcriber spelled
    // out — and counting it would make the worst-transcribed lines look like the
    // most advanced ones, which is exactly backwards for a feature that recommends
    // what to study.
    if !token.known_to_dictionary {
        return false;
    }
    if !CONTENT_PARTS_OF_SPEECH.contains(&token.part_of_speech.as_str()) {
        return false;
    }
    if token.part_of_speech == "名詞"
        && EXCLUDED_NOUN_SUBCATEGORIES.contains(&token.part_of_speech_subcategory.as_str())
    {
        return false;
    }
    token.part_of_speech_subcategory != EXCLUDED_DEPENDENT_SUBCATEGORY
}

/// Whether a line is one word away: everything known but one.
///
/// Nothing about whether that word can be *learned* from the line — see
/// [`has_context`] for that. The two were one test until 2026-08-10, and separating
/// them is what lets a bare line be listed and filtered while still being told apart
/// from a real sentence.
fn is_within_reach(unknown_words: usize) -> bool {
    unknown_words == 1
}

/// Whether there is anything around the new word to infer it from.
///
/// **i+1 needs an i.** A line whose only content word is the new one gives the reader
/// nothing to work with — just a word on its own, which a flashcard does better than a
/// sentence card. That is not a tuned threshold, it is the definition, and it was
/// measured before being believed: on a real 598-line episode, 242 lines had exactly
/// one unknown content word and **99 of those had no other content word at all**.
///
/// It no longer excludes those lines, because the user asked for them — they are shown
/// and filtered like any other line one word away. What it still does is let the badge
/// say which kind a line is, and let "Mine all" leave them out unless told otherwise.
/// Two of every five one-word-away lines are bare, so that is a 40% difference in how
/// many cards a batch produces.
fn has_context(content_word_count: usize) -> bool {
    content_word_count >= 2
}

/// The distinct content words in one line, in the form the index stores.
///
/// Distinct, because a line repeating 元気 three times is one word to learn, not
/// three — and an i+1 line that says a word twice is still i+1.
///
/// Normalized through `normalize_expression` for the same reason the index is: the
/// two sides have to agree, and a comparison is only as good as its least
/// normalized operand.
fn line_content_words(line: &str, dictionary_path: &Path) -> Result<Vec<String>, String> {
    let mut seen = HashSet::new();
    Ok(tokenize_japanese(line, dictionary_path)?
        .into_iter()
        .filter(is_content_word)
        .map(|token| normalize_expression(&token.base_form))
        .filter(|word| !word.is_empty() && seen.insert(word.clone()))
        .collect())
}

/// The words in one line the user does not know yet.
///
/// The same derivation the badge uses, exposed for the card enricher so a card's
/// definitions cannot describe a different set of words than the badge that
/// recommended the line. Empty whenever the feature is not set up — no sources, no
/// dictionary, no index — because "nothing to add" is the right answer then, not an
/// error a mine should carry.
pub(super) fn line_unknown_words<R: Runtime>(
    app: &AppHandle<R>,
    line: &str,
) -> Result<Vec<String>, String> {
    let asset_directory = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the app settings.".to_string())?;
        persisted.settings.asset_directory.clone()
    };
    let Some(dictionary_path) = find_managed_dictionary_root(Path::new(&asset_directory)) else {
        return Ok(Vec::new());
    };
    let words = line_content_words(line, &dictionary_path)?;

    let state = app.state::<KnownWordsState>();
    let index = state
        .0
        .lock()
        .map_err(|_| "Could not read your known-word list.".to_string())?;
    let Some(index) = index.as_ref() else {
        return Ok(Vec::new());
    };
    Ok(words
        .into_iter()
        .filter(|word| !index.words.contains(word))
        .collect())
}

fn nothing_to_rank(status: &str, message: &str, line_count: usize) -> TranscriptRanking {
    TranscriptRanking {
        status: status.into(),
        message: message.into(),
        // Still one entry per line, so the caller can render rows the same way
        // whatever the status is — a shorter list would be a second shape for
        // every consumer to handle, and the one they forget.
        lines: vec![LineRanking::default(); line_count],
    }
}

/// Counts the words in each line that are not yet known.
///
/// Takes the lines rather than a recording path, because the rows on screen are
/// not always the rows on disk: the viewer lets a sentence be merged or split, and
/// a ranking keyed to the sidecar's indices would quietly describe the previous
/// shape of the transcript. Whatever is displayed is what gets ranked.
pub(crate) fn rank_transcript_lines_inner<R: Runtime>(
    app: &AppHandle<R>,
    lines: &[String],
) -> Result<TranscriptRanking, String> {
    let (asset_directory, has_sources) = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read the app settings.".to_string())?;
        (
            persisted.settings.asset_directory.clone(),
            // Through the one constructor, so a half-filled row in settings does not
            // read as "configured" and promise a ranking that cannot be built.
            !KnownWordsBuild::from_anki_settings(&persisted.settings.anki)
                .sources
                .is_empty(),
        )
    };

    if !has_sources {
        return Ok(nothing_to_rank(
            "unconfigured",
            "Choose where your vocabulary lives to see which lines are within reach.",
            lines.len(),
        ));
    }
    let Some(dictionary_path) = find_managed_dictionary_root(Path::new(&asset_directory)) else {
        return Ok(nothing_to_rank(
            "needsDictionary",
            "Download the Japanese dictionary to count the words in each line.",
            lines.len(),
        ));
    };

    // Tokenized before the index is locked, and deliberately so: this is the slow
    // half — it can load the dictionary on a cold cache — and holding the index
    // across it would stall a Refresh behind a transcript being opened.
    let content_words = lines
        .iter()
        .map(|line| line_content_words(line, &dictionary_path))
        .collect::<Result<Vec<_>, _>>()?;

    let state = app.state::<KnownWordsState>();
    let index = state
        .0
        .lock()
        .map_err(|_| "Could not read your known-word list.".to_string())?;
    let Some(index) = index.as_ref() else {
        return Ok(nothing_to_rank(
            "unbuilt",
            "Refresh your known-word list to see which lines are within reach.",
            lines.len(),
        ));
    };

    let ranked: Vec<LineRanking> = content_words
        .into_iter()
        .map(|words| {
            let content_word_count = words.len();
            let unknown_words: Vec<String> = words
                .into_iter()
                .filter(|word| !index.words.contains(word))
                .collect();
            LineRanking {
                within_reach: is_within_reach(unknown_words.len()),
                has_context: has_context(content_word_count),
                unknown_words,
                content_word_count,
            }
        })
        .collect();

    let within_reach = ranked.iter().filter(|line| line.within_reach).count();
    Ok(TranscriptRanking {
        status: "ready".into(),
        message: format!(
            "{within_reach} {} one new word.",
            if within_reach == 1 { "line has" } else { "lines have" }
        ),
        lines: ranked,
    })
}

#[cfg(test)]
mod tests {
    use super::is_content_word;
    use crate::tokenizer::JapaneseToken;

    fn token(part_of_speech: &str, subcategory: &str) -> JapaneseToken {
        JapaneseToken {
            surface: "x".into(),
            base_form: "x".into(),
            known_to_dictionary: true,
            part_of_speech: part_of_speech.into(),
            part_of_speech_subcategory: subcategory.into(),
        }
    }

    /// "One word away" is now only about the count. Whether the word can be LEARNED from the
    /// line is a separate question — see the next test — and keeping them apart is what lets a
    /// bare line be listed while still being told apart from a sentence.
    #[test]
    fn one_word_away_is_decided_by_the_count_alone() {
        use super::is_within_reach;
        assert!(is_within_reach(1));
        // Everything known, or more than one gap: neither is the case this finds.
        assert!(!is_within_reach(0));
        assert!(!is_within_reach(2));
    }

    /// i+1 needs an i. A line whose only content word is the new one has no surrounding
    /// meaning to infer it from — measured at two of every five "one word away" lines on a
    /// real episode. It no longer hides those lines, but it is still what decides whether the
    /// badge calls one a sentence and whether "Mine all" takes it by default.
    #[test]
    fn a_line_whose_only_word_is_the_new_one_has_no_context() {
        use super::has_context;
        assert!(!has_context(1));
        assert!(has_context(2));
        assert!(has_context(5));
        // No content words at all is the emptiest case, not a sentence.
        assert!(!has_context(0));
    }

    /// The two together, which is how every caller reads them: a bare word is one word away
    /// AND without context, and that pairing is what the amber badge and the "Mine all"
    /// setting both key on. If these ever collapsed back into one test, that pairing would
    /// stop being pinned.
    #[test]
    fn a_bare_word_is_within_reach_but_has_no_context() {
        use super::{has_context, is_within_reach};
        // 刑期が — one new word, nothing else in the line.
        assert!(is_within_reach(1) && !has_context(1));
        // この刑務所に来た記憶もほとんどない — two new words, plenty of context.
        assert!(!is_within_reach(2) && has_context(4));
        // A real i+1 sentence: both true.
        assert!(is_within_reach(1) && has_context(3));
    }

    #[test]
    fn vocabulary_counts() {
        assert!(is_content_word(&token("名詞", "一般")));
        assert!(is_content_word(&token("動詞", "自立")));
        assert!(is_content_word(&token("形容詞", "自立")));
        assert!(is_content_word(&token("副詞", "一般")));
    }

    /// The correctness requirement behind the whole feature: は, が and を appear in
    /// nearly every sentence, so counting them would put the same floor under every
    /// line and the number would mean nothing.
    #[test]
    fn grammar_does_not_count() {
        assert!(!is_content_word(&token("助詞", "格助詞")));
        assert!(!is_content_word(&token("助動詞", "*")));
        assert!(!is_content_word(&token("記号", "句点")));
        assert!(!is_content_word(&token("フィラー", "*")));
        assert!(!is_content_word(&token("感動詞", "*")));
    }

    #[test]
    fn nouns_that_are_not_vocabulary_do_not_count() {
        // A character's name would make every line naming someone read as harder.
        assert!(!is_content_word(&token("名詞", "固有名詞")));
        assert!(!is_content_word(&token("名詞", "数")));
        // こと, もの — grammar wearing a noun's clothes.
        assert!(!is_content_word(&token("名詞", "非自立")));
        assert!(!is_content_word(&token("名詞", "接尾")));
        assert!(!is_content_word(&token("名詞", "代名詞")));
    }

    #[test]
    fn the_dependent_verb_of_a_grammar_pattern_does_not_count() {
        // The いる of 食べている is the pattern, not a word being used.
        assert!(!is_content_word(&token("動詞", "非自立")));
    }

    /// A word IPADIC has never seen is a mistranscription, a name or a stutter far
    /// more often than it is vocabulary — and counting it would make the worst
    /// transcribed lines look like the most advanced ones.
    #[test]
    fn a_word_the_dictionary_does_not_know_does_not_count() {
        let mut unknown = token("名詞", "一般");
        unknown.known_to_dictionary = false;
        assert!(!is_content_word(&unknown));
    }

    /// Ranks a real transcript against a real `known_words.txt` and prints it.
    ///
    /// The end-to-end check the unit tests cannot be: whether the counts a user
    /// actually sees are sensible. It reads the same file the app writes and calls
    /// the same `line_content_words` the command does, so the only thing it leaves
    /// out is the Tauri plumbing.
    ///
    ///   WONDER_OF_U_IPADIC_DIR=<dir> WONDER_OF_U_SRT=<file.srt> \
    ///     WONDER_OF_U_KNOWN_WORDS=<known_words.txt> \
    ///     cargo test rank_a_real_transcript -- --ignored --nocapture
    #[test]
    #[ignore = "requires a dictionary, a transcript and a built known-word list"]
    fn rank_a_real_transcript() {
        use super::line_content_words;
        use std::{collections::HashSet, path::PathBuf};

        let dictionary = PathBuf::from(
            std::env::var("WONDER_OF_U_IPADIC_DIR")
                .expect("set WONDER_OF_U_IPADIC_DIR to an extracted lindera-ipadic directory"),
        );
        let known_words_file = std::fs::read_to_string(
            std::env::var("WONDER_OF_U_KNOWN_WORDS")
                .expect("set WONDER_OF_U_KNOWN_WORDS to a known_words.txt"),
        )
        .expect("the known-word list must be readable");
        // Same rule as the store: line 1 is the header, every other line is a word.
        let known: HashSet<&str> = known_words_file.lines().skip(1).collect();

        let srt = std::fs::read_to_string(
            std::env::var("WONDER_OF_U_SRT").expect("set WONDER_OF_U_SRT to a .srt file"),
        )
        .expect("the transcript must be readable");
        let lines: Vec<&str> = srt
            .lines()
            .map(str::trim)
            .filter(|line| {
                !line.is_empty() && !line.contains("-->") && line.parse::<u32>().is_err()
            })
            .collect();

        println!("  {} known words, {} lines\n", known.len(), lines.len());
        // How many content words a line has at all decides what "one word away"
        // is worth: a fragment with a single content word is i+1 by arithmetic and
        // teaches nothing, while a full sentence with one new word is the thing
        // this feature exists to find. Bucketed so the difference is visible.
        let mut by_content_words = [0usize; 4];
        let mut within_reach_by_content_words = [0usize; 4];
        let mut fully_known = 0;
        for line in &lines {
            let words = line_content_words(line, &dictionary).unwrap();
            if words.is_empty() {
                continue;
            }
            let unknown = words.iter().filter(|word| !known.contains(word.as_str())).count();
            // Both halves, deliberately: this measurement is what showed that two of every
            // five one-word-away lines are bare, and that number is still the reason the
            // badge and the "Mine all" setting tell them apart.
            let counted = super::is_within_reach(unknown) && super::has_context(words.len());
            let bucket = match words.len() {
                1 => 0,
                2 => 1,
                3..=4 => 2,
                _ => 3,
            };
            by_content_words[bucket] += 1;
            if unknown == 0 {
                fully_known += 1;
            }
            if counted {
                within_reach_by_content_words[bucket] += 1;
                // Listed, not just counted. Which lines qualify is the question every
                // "why did N go in and M come out" needs answering, and a histogram
                // cannot answer it.
                let new_word = words.iter().find(|word| !known.contains(word.as_str()));
                println!("  +1  {line}
        new: {}", new_word.map(String::as_str).unwrap_or(""));
            }
        }
        let labels = ["1 word", "2 words", "3-4 words", "5+ words"];
        println!("  content words per line   lines   of which one-word-away");
        for bucket in 0..4 {
            println!(
                "  {:<22} {:>6} {:>16}",
                labels[bucket], by_content_words[bucket], within_reach_by_content_words[bucket]
            );
        }
        println!(
            "\n  {} lines one word away, {fully_known} fully known, out of {}",
            within_reach_by_content_words.iter().sum::<usize>(),
            lines.len()
        );
    }

    /// Prints what a real transcript reduces to, line by line.
    ///
    /// The lists above were chosen by reading this against real recordings, not by
    /// reasoning about IPADIC's tag set: the decision to build this said the exact
    /// set "only real transcripts will tell you". Anything changed there should be
    /// changed by looking at this again.
    ///
    ///   WONDER_OF_U_IPADIC_DIR=<dir> WONDER_OF_U_SRT=<file.srt> \
    ///     cargo test explain_a_real_transcript -- --ignored --nocapture
    #[test]
    #[ignore = "requires an installed dictionary and a transcript"]
    fn explain_a_real_transcript() {
        use crate::tokenizer::tokenize_japanese;
        use std::path::PathBuf;

        let dictionary = PathBuf::from(
            std::env::var("WONDER_OF_U_IPADIC_DIR")
                .expect("set WONDER_OF_U_IPADIC_DIR to an extracted lindera-ipadic directory"),
        );
        let srt = std::fs::read_to_string(
            std::env::var("WONDER_OF_U_SRT").expect("set WONDER_OF_U_SRT to a .srt file"),
        )
        .expect("the transcript must be readable");

        let lines: Vec<&str> = srt
            .lines()
            .map(str::trim)
            .filter(|line| {
                !line.is_empty()
                    && !line.contains("-->")
                    && line.parse::<u32>().is_err()
            })
            .take(25)
            .collect();

        for line in lines {
            let tokens = tokenize_japanese(line, &dictionary).unwrap();
            let kept: Vec<String> = tokens
                .iter()
                .filter(|token| is_content_word(token))
                .map(|token| token.base_form.clone())
                .collect();
            let dropped: Vec<String> = tokens
                .iter()
                .filter(|token| !is_content_word(token))
                .map(|token| {
                    format!(
                        "{}({}/{})",
                        token.surface, token.part_of_speech, token.part_of_speech_subcategory
                    )
                })
                .collect();
            println!("\n  {line}");
            println!("    counts : {}", kept.join(" "));
            println!("    dropped: {}", dropped.join(" "));
        }
    }
}
