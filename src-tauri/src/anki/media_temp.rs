use std::{
    fs,
    path::{Path, PathBuf},
};

/// Where a mine builds the files it hands to Anki.
///
/// These used to be written beside the source media — a mined line left `Ep01_seg4200.mp3`
/// and `Ep01_shot4200.jpg` sitting in the user's own video folder until the code got around
/// to deleting them, and left them there for good if anything went wrong first. They are
/// scratch files that exist for the few hundred milliseconds between ffmpeg writing them and
/// Anki copying them into its media collection, so they belong in the OS temp directory.
///
/// The directory is created on demand and never cleaned up as a whole: it is shared with
/// whatever else the OS puts there, and the individual files remove themselves (see
/// `TempMedia`).
pub(crate) fn mining_temp_dir() -> Result<PathBuf, String> {
    let directory = std::env::temp_dir().join("wonder-of-u");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create a temporary folder for the clip: {error}"))?;
    Ok(directory)
}

/// A file that deletes itself when it goes out of scope.
///
/// A mine can end at a dozen points — bad settings, ffmpeg failing, Anki being closed, the
/// note being a duplicate — and every one of them used to need its own `remove_file`. There
/// were four, which is four chances to add a fifth exit and forget. Ownership answers it
/// instead: the file is deleted when the value holding it dies, on every path, including a
/// panic. Storing the file with Anki does not consume the guard, because Anki copies the
/// bytes into its own collection and the original is scratch either way.
pub(super) struct TempMedia {
    /// `None` once `keep` has taken it, which is what disarms the drop.
    path: Option<PathBuf>,
}

impl TempMedia {
    pub(super) fn new(path: PathBuf) -> Self {
        Self { path: Some(path) }
    }

    pub(super) fn path(&self) -> &Path {
        // Only `keep` empties this, and it consumes the guard, so a caller cannot hold a
        // reference to a taken path.
        self.path.as_deref().unwrap_or(Path::new(""))
    }

    /// Hand the file to a caller that outlives the mine, and stop guarding it.
    ///
    /// A mined clip is scratch — Anki copies the bytes and the original is disposable. A
    /// preview is not: the webview has to still be able to read it after this returns, so
    /// that one caller takes ownership and is responsible for the cleanup instead.
    pub(super) fn keep(mut self) -> PathBuf {
        self.path.take().unwrap_or_default()
    }
}

impl Drop for TempMedia {
    fn drop(&mut self) {
        // Best effort by design. A file that cannot be removed is a stray in the OS temp
        // directory, which is not worth failing a mine that has otherwise succeeded.
        if let Some(path) = &self.path {
            let _ = fs::remove_file(path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TempMedia;

    #[test]
    fn the_file_is_gone_once_the_guard_is_dropped() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("clip.mp3");
        std::fs::write(&path, b"audio").unwrap();

        {
            let _guard = TempMedia::new(path.clone());
            assert!(path.exists());
        }

        assert!(!path.exists(), "the guard should have removed the file");
    }

    #[test]
    fn a_file_that_was_never_written_is_not_an_error_to_drop() {
        // ffmpeg failing before it writes anything is an ordinary outcome, and the guard is
        // built before ffmpeg runs.
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("never-created.mp3");
        drop(TempMedia::new(path.clone()));
        assert!(!path.exists());
    }
}
