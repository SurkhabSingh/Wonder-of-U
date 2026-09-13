macro_rules! mined_tag {
    () => {
        "wonder-of-u"
    };
    ($kind:literal) => {
        concat!(mined_tag!(), "::", $kind)
    };
}

/// On every card this app has ever made, whatever made it.
pub(crate) const MINED: &str = mined_tag!();

pub(crate) const MINED_WORD: &str = mined_tag!("word");

pub(crate) const MINED_LINE: &str = mined_tag!("line");

pub(crate) const MINED_TRANSCRIPT: &str = mined_tag!("transcript");

#[cfg(test)]
mod tests {
    use super::{MINED, MINED_LINE, MINED_TRANSCRIPT, MINED_WORD};

    /// The exact strings, pinned.
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

    /// The tags have to reach Anki from here rather than from a literal at each call site,
    /// on the way out AND on the way back. A second copy is a second thing to update, and
    /// the copy that gets missed is invisible until a count that should be thousands reads
    /// zero — which is the failure a card-counting query drifting from the writers makes,
    /// and it looks exactly like an empty collection.
    #[test]
    fn every_path_takes_its_tags_from_here_rather_than_its_own_literal() {
        for (name, source) in [
            ("anki/mine.rs", include_str!("mine.rs")),
            ("anki/push.rs", include_str!("push.rs")),
            ("anki/mined_cards.rs", include_str!("mined_cards.rs")),
        ] {
            assert!(
                !source.contains(&format!("\"{MINED}")),
                "{name} names a tag as its own literal instead of using this module"
            );
            assert!(
                source.contains("tags::MINED"),
                "{name} must take its tags from this module"
            );
        }
    }
}
