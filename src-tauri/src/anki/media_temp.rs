use std::{
    fs,
    path::{Path, PathBuf},
};

/// Where a mine builds the files it hands to Anki.
pub(super) fn mining_temp_dir() -> Result<PathBuf, String> {
    let directory = std::env::temp_dir().join("wonder-of-u");
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create a temporary folder for the clip: {error}"))?;
    Ok(directory)
}

/// A file that deletes itself when it goes out of scope.
pub(super) struct TempMedia {
    path: PathBuf,
}

impl TempMedia {
    pub(super) fn new(path: PathBuf) -> Self {
        Self { path }
    }

    pub(super) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempMedia {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
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
