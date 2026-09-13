use std::fs;
use std::io;
use std::path::Path;

/// Read a file produced by something other than this app, replacing invalid UTF-8 rather
/// than failing on it. `Err` is reserved for the file genuinely not being readable.
pub(crate) fn read_external_text(path: impl AsRef<Path>) -> io::Result<String> {
    Ok(String::from_utf8_lossy(&fs::read(path)?).into_owned())
}

#[cfg(test)]
mod tests {
    use super::read_external_text;
    use std::fs;

    /// The exact shape whisper produced: a 3-byte character truncated to its first 2 bytes.
    /// `fs::read_to_string` rejects this file outright; this reader must not.
    #[test]
    fn a_truncated_multi_byte_character_costs_one_character_not_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("whisper.json");

        // "농" is EB 86 8D. Write it whole, then again missing its final byte.
        let mut bytes = Vec::new();
        bytes.extend_from_slice("{\"text\":\"".as_bytes());
        bytes.extend_from_slice(&[0xEB, 0x86, 0x8D]);
        bytes.extend_from_slice(&[0xEB, 0x86]); // truncated — the whole bug
        bytes.extend_from_slice("\"}".as_bytes());
        fs::write(&path, &bytes).unwrap();

        assert!(
            fs::read_to_string(&path).is_err(),
            "the fixture must be invalid UTF-8, or this test proves nothing",
        );

        let text = read_external_text(&path).expect("a bad byte must not fail the read");
        assert!(text.contains('\u{FFFD}'), "the bad byte becomes a replacement character");
        assert!(text.contains('농'), "every valid character around it survives");
        assert!(text.ends_with("\"}"), "the rest of the file is intact");
    }

    #[test]
    fn a_missing_file_is_still_an_error() {
        let dir = tempfile::tempdir().unwrap();
        assert!(read_external_text(dir.path().join("absent.txt")).is_err());
    }
}
