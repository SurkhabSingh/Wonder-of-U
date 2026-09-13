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
const CONTENT_PARTS_OF_SPEECH: [&str; 6] = [
    "名詞",   // noun
    "動詞",   // verb
    "形容詞", // i-adjective
    "副詞",   // adverb
    "連体詞", // adnominal
    "接続詞", // conjunction
];

/// Noun subcategories that are not vocabulary, however much they look like nouns.
const EXCLUDED_NOUN_SUBCATEGORIES: [&str; 5] =
    ["固有名詞", "数", "非自立", "接尾", "代名詞"];

const EXCLUDED_DEPENDENT_SUBCATEGORY: &str = "非自立";

/// Whether this token is a word someone would study.
pub(super) fn is_content_word(token: &JapaneseToken) -> bool {
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
fn is_within_reach(unknown_words: usize) -> bool {
    unknown_words == 1
}

/// Whether there is anything around the new word to infer it from.
fn has_context(content_word_count: usize) -> bool {
    content_word_count >= 2
}

/// The distinct content words in one line, in the form the index stores.
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
        lines: vec![LineRanking::default(); line_count],
    }
}

/// Counts the words in each line that are not yet known.
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

    #[test]
    fn one_word_away_is_decided_by_the_count_alone() {
        use super::is_within_reach;
        assert!(is_within_reach(1));
        assert!(!is_within_reach(0));
        assert!(!is_within_reach(2));
    }

    #[test]
    fn a_line_whose_only_word_is_the_new_one_has_no_context() {
        use super::has_context;
        assert!(!has_context(1));
        assert!(has_context(2));
        assert!(has_context(5));
        assert!(!has_context(0));
    }

    #[test]
    fn a_bare_word_is_within_reach_but_has_no_context() {
        use super::{has_context, is_within_reach};
        assert!(is_within_reach(1) && !has_context(1));
        assert!(!is_within_reach(2) && has_context(4));
        assert!(is_within_reach(1) && has_context(3));
    }

    #[test]
    fn vocabulary_counts() {
        assert!(is_content_word(&token("名詞", "一般")));
        assert!(is_content_word(&token("動詞", "自立")));
        assert!(is_content_word(&token("形容詞", "自立")));
        assert!(is_content_word(&token("副詞", "一般")));
    }

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
        assert!(!is_content_word(&token("名詞", "固有名詞")));
        assert!(!is_content_word(&token("名詞", "数")));
        assert!(!is_content_word(&token("名詞", "非自立")));
        assert!(!is_content_word(&token("名詞", "接尾")));
        assert!(!is_content_word(&token("名詞", "代名詞")));
    }

    #[test]
    fn the_dependent_verb_of_a_grammar_pattern_does_not_count() {
        assert!(!is_content_word(&token("動詞", "非自立")));
    }

    #[test]
    fn a_word_the_dictionary_does_not_know_does_not_count() {
        let mut unknown = token("名詞", "一般");
        unknown.known_to_dictionary = false;
        assert!(!is_content_word(&unknown));
    }

    /// Ranks a real transcript against a real `known_words.txt` and prints it.
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
        let mut by_content_words = [0usize; 4];
        let mut within_reach_by_content_words = [0usize; 4];
        let mut fully_known = 0;
        for line in &lines {
            let words = line_content_words(line, &dictionary).unwrap();
            if words.is_empty() {
                continue;
            }
            let unknown = words.iter().filter(|word| !known.contains(word.as_str())).count();
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
