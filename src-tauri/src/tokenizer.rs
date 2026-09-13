use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};

use lindera::{
    dictionary::load_fs_dictionary, mode::Mode, segmenter::Segmenter, tokenizer::Tokenizer,
};

/// The IPADIC field holding a word's dictionary form (見た → 見る).
const BASE_FORM_FIELD: &str = "base_form";

/// IPADIC's part of speech, and the first of its three subcategories.
///
/// The pair is what separates vocabulary from grammar: 名詞 alone admits a
/// character's name (名詞,固有名詞) and a counter (名詞,数) alongside a real noun, and
/// those are not words anyone studies. Both are needed, and both are looked up by
/// name through the dictionary's own schema rather than by index.
const PART_OF_SPEECH_FIELD: &str = "part_of_speech";
const PART_OF_SPEECH_SUBCATEGORY_FIELD: &str = "part_of_speech_subcategory_1";

const UNKNOWN_FIELD_VALUE: &str = "*";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct JapaneseToken {
    pub(crate) surface: String,
    pub(crate) base_form: String,
    pub(crate) known_to_dictionary: bool,
    pub(crate) part_of_speech: String,
    pub(crate) part_of_speech_subcategory: String,
}

/// Caches the loaded dictionary against the path it came from.
static TOKENIZER_CACHE: TokenizerCache = TokenizerCache::new();

struct TokenizerCache {
    state: Mutex<Option<(PathBuf, Arc<Tokenizer>)>>,
}

impl TokenizerCache {
    const fn new() -> Self {
        Self {
            state: Mutex::new(None),
        }
    }

    fn cached_for(&self, dictionary_path: &Path) -> Option<Arc<Tokenizer>> {
        let cached = self.state.lock().ok()?;
        cached
            .as_ref()
            .filter(|(path, _)| path == dictionary_path)
            .map(|(_, tokenizer)| Arc::clone(tokenizer))
    }

    fn store(&self, dictionary_path: &Path, tokenizer: &Arc<Tokenizer>) {
        if let Ok(mut cached) = self.state.lock() {
            *cached = Some((dictionary_path.to_path_buf(), Arc::clone(tokenizer)));
        }
    }

    /// Returns the tokenizer for `dictionary_path`, loading it only on a miss.
    fn tokenizer_for(&self, dictionary_path: &Path) -> Result<Arc<Tokenizer>, String> {
        if let Some(tokenizer) = self.cached_for(dictionary_path) {
            return Ok(tokenizer);
        }

        let tokenizer = Arc::new(build_tokenizer(dictionary_path)?);
        self.store(dictionary_path, &tokenizer);
        Ok(tokenizer)
    }
}

fn build_tokenizer(dictionary_path: &Path) -> Result<Tokenizer, String> {
    let dictionary = load_fs_dictionary(dictionary_path).map_err(|error| {
        format!(
            "The Japanese dictionary at {} could not be loaded: {error}",
            dictionary_path.display()
        )
    })?;
    Ok(Tokenizer::new(Segmenter::new(
        Mode::Normal,
        dictionary,
        None,
    )))
}

/// Confirms a dictionary directory actually loads, which is the only question that
/// matters about it.
pub(crate) fn dictionary_loads(dictionary_path: &Path) -> Result<(), String> {
    build_tokenizer(dictionary_path).map(|_| ())
}

/// Picks the form a word should be counted under.
fn resolve_base_form(surface: &str, base_form: Option<&str>) -> String {
    match base_form {
        Some(value) if value != UNKNOWN_FIELD_VALUE && !value.is_empty() => value.to_string(),
        _ => surface.to_string(),
    }
}

/// Splits Japanese text into words, each with the form it should be looked up under.
pub(crate) fn tokenize_japanese(
    text: &str,
    dictionary_path: &Path,
) -> Result<Vec<JapaneseToken>, String> {
    let tokenizer = TOKENIZER_CACHE.tokenizer_for(dictionary_path)?;
    let mut tokens = tokenizer
        .tokenize(text)
        .map_err(|error| format!("The text could not be tokenized: {error}"))?;

    Ok(tokens
        .iter_mut()
        .map(|token| {
            let surface = token.surface.to_string();
            let raw_base_form = token.get(BASE_FORM_FIELD);
            let known_to_dictionary = matches!(
                raw_base_form,
                Some(value) if value != UNKNOWN_FIELD_VALUE && !value.is_empty()
            );
            let base_form = resolve_base_form(&surface, raw_base_form);
            JapaneseToken {
                part_of_speech: token.get(PART_OF_SPEECH_FIELD).unwrap_or_default().to_string(),
                part_of_speech_subcategory: token
                    .get(PART_OF_SPEECH_SUBCATEGORY_FIELD)
                    .unwrap_or_default()
                    .to_string(),
                known_to_dictionary,
                base_form,
                surface,
            }
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::{resolve_base_form, tokenize_japanese, JapaneseToken};
    use std::path::PathBuf;

    #[test]
    fn an_unknown_word_falls_back_to_its_own_surface() {
        assert_eq!(resolve_base_form("hello", Some("*")), "hello");
        assert_eq!(resolve_base_form("hello", None), "hello");
        assert_eq!(resolve_base_form("hello", Some("")), "hello");
    }

    #[test]
    fn a_known_word_is_counted_under_its_dictionary_form() {
        assert_eq!(resolve_base_form("見", Some("見る")), "見る");
    }

    /// The dictionary a `#[ignore]`d test needs, or `None` when it is not installed.
    fn installed_dictionary() -> Option<PathBuf> {
        let path = PathBuf::from(std::env::var("WONDER_OF_U_IPADIC_DIR").ok()?);
        path.join("metadata.json").is_file().then_some(path)
    }

    fn base_forms(text: &str) -> Vec<JapaneseToken> {
        let dictionary = installed_dictionary()
            .expect("set WONDER_OF_U_IPADIC_DIR to an extracted lindera-ipadic directory");
        tokenize_japanese(text, &dictionary).unwrap()
    }

    /// Needs the real ~16MB IPADIC download, so it cannot run in the default suite.
    #[test]
    #[ignore = "requires an installed IPADIC dictionary; see WONDER_OF_U_IPADIC_DIR"]
    fn conjugations_are_reduced_to_their_dictionary_form() {
        // Each of these is more than one token: 見た is 見 + た. The conjugated stem
        // is the first token, and it is the one carrying the base form.
        assert_eq!(base_forms("見た")[0].base_form, "見る");
        assert_eq!(base_forms("食べました")[0].base_form, "食べる");
        assert_eq!(base_forms("面白かった")[0].base_form, "面白い");

        // The surface is kept alongside it: the stem alone is not what was said.
        assert_eq!(base_forms("見た")[0].surface, "見");
    }

    #[test]
    #[ignore = "requires an installed IPADIC dictionary; see WONDER_OF_U_IPADIC_DIR"]
    fn a_non_japanese_string_degrades_to_its_own_words() {
        let tokens = base_forms("hello world");
        // IPADIC does not know these, so they stand in as their own base form
        // rather than collapsing into `*`.
        assert_eq!(
            tokens
                .iter()
                .map(|token| token.base_form.as_str())
                .collect::<Vec<_>>(),
            vec!["hello", "world"]
        );
        assert!(base_forms("").is_empty());
    }

    #[test]
    #[ignore = "requires an installed IPADIC dictionary; see WONDER_OF_U_IPADIC_DIR"]
    fn the_dictionary_is_loaded_once_and_reused_across_calls() {
        let dictionary = installed_dictionary()
            .expect("set WONDER_OF_U_IPADIC_DIR to an extracted lindera-ipadic directory");
        // The second call must come off the cache; a reload here would be a
        // per-sentence dictionary parse.
        tokenize_japanese("本を読む", &dictionary).unwrap();
        let started_at = std::time::Instant::now();
        tokenize_japanese("本を読む", &dictionary).unwrap();
        assert!(started_at.elapsed() < std::time::Duration::from_millis(100));
    }

    #[test]
    fn a_missing_dictionary_reports_the_path_instead_of_panicking() {
        let dir = tempfile::tempdir().unwrap();
        let absent = dir.path().join("not-a-dictionary");
        let error = tokenize_japanese("見た", &absent).unwrap_err();
        assert!(error.contains("could not be loaded"), "{error}");
    }
}
