//! Reading the pipes of a child process.

use std::io::{BufRead, BufReader, Read};

/// Reads a child pipe line by line, and never stops early on a decode error.
///
/// Both obvious spellings of this loop are wrong, in opposite directions, and both are
/// wrong in the same expensive way: the thread that drains stdout usually owns the EOF
/// signal, so how this iterator ends decides whether the run ends at all.
///
/// `lines().map_while(Result::ok)` ends the whole iterator at the first line that is not
/// valid UTF-8 — and one non-ASCII byte is enough, since these children report a console
/// codepage rather than UTF-8. The drain then reports EOF while the child is still
/// writing: nothing drains the pipe, the child blocks on a full one, and `wait()` blocks
/// on the child — after the loop that polls for Cancel has already been left, so the run
/// cannot even be cancelled.
///
/// `lines().filter_map(Result::ok)` fixes that and breaks the other end. A decode error
/// has already consumed its line, so skipping it does advance; but a pipe in a genuine
/// error state returns the same error without consuming anything, and the loop spins on
/// it forever. The thread never returns, its sender is never dropped, and the same wedge
/// arrives with a burned core attached.
///
/// So the decision is removed instead of made. Splitting on newline bytes never decodes,
/// so there is no `InvalidData` to handle; the only `Err` left is the pipe itself
/// failing, and ending the drain there is simply correct. A bad byte becomes `U+FFFD` in
/// the text rather than costing the line, which matters because these lines are what a
/// failure gets explained from.
pub(crate) fn drain_lines(pipe: impl Read) -> impl Iterator<Item = String> {
    BufReader::new(pipe)
        .split(b'\n')
        .map_while(Result::ok)
        .map(|raw| {
            String::from_utf8_lossy(&raw)
                .trim_end_matches('\r')
                .to_string()
        })
}

#[cfg(test)]
mod tests {
    use super::drain_lines;
    use std::io::{self, Read};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;

    #[test]
    fn a_bad_byte_costs_its_own_characters_and_nothing_else() {
        // A single undecodable byte used to end the whole iterator. On a stdout drain
        // that silently truncates the stream and wedges the run, so the decode is lossy
        // and every later line still has to arrive.
        let raw: &[u8] = b"first\n\xff\xfe bad\nthird\n";
        let lines = drain_lines(raw).collect::<Vec<_>>();
        assert_eq!(lines.len(), 3, "every line should survive, got {lines:?}");
        assert_eq!(lines[0], "first");
        assert_eq!(lines[2], "third");
        assert!(lines[1].ends_with(" bad"), "{:?}", lines[1]);
    }

    #[test]
    fn carriage_returns_are_left_behind() {
        let raw: &[u8] = b"one\r\ntwo\r\n";
        assert_eq!(drain_lines(raw).collect::<Vec<_>>(), vec!["one", "two"]);
    }

    #[test]
    fn a_last_line_without_a_newline_still_arrives() {
        let raw: &[u8] = b"one\ntwo";
        assert_eq!(drain_lines(raw).collect::<Vec<_>>(), vec!["one", "two"]);
    }

    /// A pipe that has failed for good: every read reports the same error and consumes
    /// nothing, which is the shape a closed or broken handle actually has.
    struct DeadPipe {
        reads: Arc<AtomicUsize>,
    }

    impl Read for DeadPipe {
        fn read(&mut self, _buffer: &mut [u8]) -> io::Result<usize> {
            let reads = self.reads.fetch_add(1, Ordering::Relaxed) + 1;
            // Turns the failure being guarded against into a fast one. A drain that
            // retries this error would otherwise hang the test rather than fail it,
            // which is exactly how it would present in the app.
            assert!(reads < 1000, "the drain retried a dead pipe {reads} times");
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "the pipe is gone"))
        }
    }

    #[test]
    fn a_pipe_that_only_ever_errors_ends_the_drain_rather_than_spinning() {
        // The cost of getting this wrong is not a lost line. It is a drain thread that
        // never returns, so its sender is never dropped, the wait never completes, and
        // the import hangs with a core at full tilt.
        let reads = Arc::new(AtomicUsize::new(0));
        let lines = drain_lines(DeadPipe {
            reads: Arc::clone(&reads),
        })
        .collect::<Vec<_>>();

        assert!(lines.is_empty(), "a dead pipe has no lines, got {lines:?}");
        assert_eq!(
            reads.load(Ordering::Relaxed),
            1,
            "the drain must stop at the first failure rather than retrying it"
        );
    }
}
