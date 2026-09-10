//! The tags every card this app makes carries.
//!
//! One module rather than a literal beside each `addNote`. These tags are the only thing
//! telling a card made here from the rest of a collection, so whatever writes them and
//! whatever counts them have to agree on the exact string — and a count looking for one
//! spelling while the writers put down another reads zero for a healthy collection, which
//! is a wrong answer wearing the clothes of an empty one.
//!
//! The kinds are built FROM the identifying tag rather than spelled out beside it, so
//! renaming it moves them with it and a kind that is not a child of it cannot be written.

/// `mined_tag!()` is the tag on everything; `mined_tag!("word")` is a kind under it.
///
/// Anki reads `::` as a hierarchy separator, so the kinds group under the parent in its
/// tag list and `tag:wonder-of-u::word` selects one of them.
macro_rules! mined_tag {
    () => {
        "wonder-of-u"
    };
    ($kind:literal) => {
        concat!(mined_tag!(), "::", $kind)
    };
}

/// On every card this app has ever made, whatever made it.
///
/// Written alongside a kind rather than left to be inferred from one: `tag:wonder-of-u`
/// matches this tag itself, so a count of everything mined here does not rest on how Anki
/// happens to treat a parent tag in a search.
pub(crate) const MINED: &str = mined_tag!();

/// Mined from the lookup popup, for one word the reader pointed at. The only kind whose
/// card names a single word, and so the only one a count of words can be built on.
pub(crate) const MINED_WORD: &str = mined_tag!("word");

/// Mined from a transcript row, or from a subtitle during a watch session: a whole line,
/// with no one word behind it.
pub(crate) const MINED_LINE: &str = mined_tag!("line");

/// A whole recording pushed as one card, rather than a line taken out of one.
pub(crate) const MINED_TRANSCRIPT: &str = mined_tag!("transcript");

#[cfg(test)]
mod tests {
    use super::{MINED, MINED_LINE, MINED_TRANSCRIPT, MINED_WORD};

    /// The exact strings, pinned.
    ///
    /// These go into the user's own collection and stay there. A card tagged a year ago
    /// has to be found by the same string today, so renaming any of these does not rename
    /// anything already written — it orphans every card carrying the old one, in a
    /// collection this app cannot see to migrate.
    #[test]
    fn the_tags_are_the_strings_already_written_into_collections() {
        assert_eq!(MINED, "wonder-of-u");
        assert_eq!(MINED_WORD, "wonder-of-u::word");
        assert_eq!(MINED_LINE, "wonder-of-u::line");
        assert_eq!(MINED_TRANSCRIPT, "wonder-of-u::transcript");
    }

    /// The macro is what makes a stray kind impossible, so this checks the macro rather
    /// than the constants: a kind that does not sit under the identifying tag is a tag
    /// nothing counting this app's cards would ever find.
    #[test]
    fn every_kind_sits_under_the_tag_that_identifies_the_card() {
        for kind in [MINED_WORD, MINED_LINE, MINED_TRANSCRIPT] {
            assert!(
                kind.starts_with(&format!("{MINED}::")),
                "{kind} is not under {MINED}"
            );
            assert_ne!(kind, MINED, "a kind has to say more than the parent does");
        }
    }

    /// Two paths sharing a kind would make the split these exist to draw invisible.
    #[test]
    fn the_kinds_are_told_apart() {
        let mut kinds = [MINED_WORD, MINED_LINE, MINED_TRANSCRIPT];
        kinds.sort_unstable();
        let unique = {
            let mut seen = kinds.to_vec();
            seen.dedup();
            seen.len()
        };
        assert_eq!(unique, kinds.len(), "two kinds share a tag");
    }

    /// The tags have to reach Anki from here rather than from a literal at each call site.
    /// A second copy is a second thing to update, and the copy that gets missed is
    /// invisible until a count that should be thousands reads zero.
    #[test]
    fn the_mining_paths_write_these_tags_rather_than_their_own() {
        for (name, source) in [
            ("anki/mine.rs", include_str!("mine.rs")),
            ("anki/push.rs", include_str!("push.rs")),
        ] {
            assert!(
                // Any literal STARTING with the tag, not only the tag alone. A kind written
                // out by hand reads "wonder-of-u::word", which does not contain the
                // tag followed by its closing quote, so it slipped past the narrower
                // test while being exactly the drift that test exists to catch.
                !source.contains(&format!("\"{MINED}")),
                "{name} writes a tag as its own literal instead of using this module"
            );
            assert!(
                source.contains("tags::MINED"),
                "{name} must take its tags from this module"
            );
        }
    }
}
