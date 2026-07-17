use std::{
    fs,
    path::{Path, PathBuf},
};

use crate::{
    app_config::IPADIC_DICTIONARY_VERSION,
    app_types::{AppSettings, DictionaryDetection},
};

use super::ytdlp::managed_binary_is_present;

/// The file every lindera dictionary carries, holding the schema its fields are
/// read through. Its presence is what identifies a directory as a dictionary root.
const DICTIONARY_METADATA_FILE: &str = "metadata.json";

/// How deep to search the extracted archive for the dictionary root. The v4.0.0
/// archive wraps everything in a single `lindera-ipadic/` folder, so one level is
/// all that is needed; the small allowance keeps a re-packaged archive working
/// without letting a wrong asset directory turn detection into a deep disk walk.
const DICTIONARY_SEARCH_DEPTH: usize = 3;

pub(crate) fn managed_dictionary_install_directory(asset_directory: &Path) -> PathBuf {
    asset_directory
        .join("lindera-ipadic")
        .join(IPADIC_DICTIONARY_VERSION)
}

fn is_dictionary_root(candidate: &Path) -> bool {
    managed_binary_is_present(&candidate.join(DICTIONARY_METADATA_FILE))
}

fn find_dictionary_root_within(directory: &Path, depth: usize) -> Option<PathBuf> {
    if is_dictionary_root(directory) {
        return Some(directory.to_path_buf());
    }
    if depth == 0 {
        return None;
    }

    // Sorted, so a re-packaged archive with several candidate folders resolves to
    // the same one on every call rather than following directory order.
    let mut entries: Vec<PathBuf> = fs::read_dir(directory)
        .ok()?
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .collect();
    entries.sort();

    entries
        .into_iter()
        .find_map(|entry| find_dictionary_root_within(&entry, depth - 1))
}

/// Locates the extracted dictionary inside the install directory.
///
/// The archive unpacks to a nested `lindera-ipadic/` folder rather than landing
/// flat, and lindera has to be pointed at that folder itself, so this searches for
/// the directory holding `metadata.json` instead of assuming the layout — the same
/// approach `collect_managed_ffmpeg_candidates` takes to the FFmpeg archive.
pub(crate) fn find_managed_dictionary_root(asset_directory: &Path) -> Option<PathBuf> {
    find_dictionary_root_within(
        &managed_dictionary_install_directory(asset_directory),
        DICTIONARY_SEARCH_DEPTH,
    )
}

/// Detects the app-managed dictionary by presence, without loading it.
///
/// This runs on every app-snapshot emit (via `build_app_bootstrap`), and loading
/// IPADIC reads ~57MB off disk — exactly the kind of per-emit cost that stalled
/// the app before. Presence is trustworthy here for the same reason it is for a
/// managed binary: the download path loads the dictionary once to prove it works
/// and deletes it when it does not, so anything still on disk has been verified.
pub(crate) fn detect_local_dictionary(settings: &AppSettings) -> DictionaryDetection {
    let asset_directory = PathBuf::from(&settings.asset_directory);
    match find_managed_dictionary_root(&asset_directory) {
        Some(dictionary_path) => DictionaryDetection {
            status: "ready".into(),
            dictionary_path: Some(dictionary_path.display().to_string()),
            managed: true,
            message: "The Japanese dictionary is ready. Sentences can be analysed word by word."
                .into(),
        },
        None => DictionaryDetection::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        detect_local_dictionary, find_managed_dictionary_root,
        managed_dictionary_install_directory, DICTIONARY_METADATA_FILE,
    };
    use crate::app_types::AppSettings;
    use std::path::{Path, PathBuf};

    fn write_dictionary_at(directory: &Path) {
        std::fs::create_dir_all(directory).unwrap();
        std::fs::write(directory.join(DICTIONARY_METADATA_FILE), b"{}").unwrap();
    }

    /// `AppSettings` has no `Default` — detection only reads `asset_directory`, so
    /// the rest is filled in rather than adding a trait to the shared type.
    fn settings_for(asset_directory: &Path) -> AppSettings {
        AppSettings {
            output_directory: String::new(),
            asset_directory: asset_directory.display().to_string(),
            whisper: Default::default(),
            anki: Default::default(),
            features: Default::default(),
            translation: Default::default(),
            theme: "system".into(),
            launch_at_login: false,
            start_minimized: false,
        }
    }

    #[test]
    fn the_install_directory_is_pinned_to_the_dictionary_version() {
        let root = PathBuf::from("C:\\assets");
        assert_eq!(
            managed_dictionary_install_directory(&root),
            PathBuf::from("C:\\assets\\lindera-ipadic\\4.0.0")
        );
    }

    #[test]
    fn the_dictionary_root_is_found_inside_the_archives_own_folder() {
        let temp_dir = tempfile::tempdir().unwrap();
        // The v4.0.0 archive wraps its files in `lindera-ipadic/`.
        let nested = managed_dictionary_install_directory(temp_dir.path()).join("lindera-ipadic");
        write_dictionary_at(&nested);

        assert_eq!(find_managed_dictionary_root(temp_dir.path()), Some(nested));
    }

    #[test]
    fn a_dictionary_extracted_flat_is_found_too() {
        let temp_dir = tempfile::tempdir().unwrap();
        let install_directory = managed_dictionary_install_directory(temp_dir.path());
        write_dictionary_at(&install_directory);

        assert_eq!(
            find_managed_dictionary_root(temp_dir.path()),
            Some(install_directory)
        );
    }

    #[test]
    fn a_directory_without_metadata_is_not_a_dictionary() {
        let temp_dir = tempfile::tempdir().unwrap();
        let install_directory = managed_dictionary_install_directory(temp_dir.path());
        std::fs::create_dir_all(install_directory.join("lindera-ipadic")).unwrap();

        // A half-extracted archive has files but no metadata.json yet.
        std::fs::write(
            install_directory.join("lindera-ipadic").join("dict.words"),
            b"partial",
        )
        .unwrap();
        assert_eq!(find_managed_dictionary_root(temp_dir.path()), None);
    }

    #[test]
    fn an_empty_metadata_file_is_rejected_like_a_truncated_download() {
        let temp_dir = tempfile::tempdir().unwrap();
        let nested = managed_dictionary_install_directory(temp_dir.path()).join("lindera-ipadic");
        std::fs::create_dir_all(&nested).unwrap();
        std::fs::write(nested.join(DICTIONARY_METADATA_FILE), b"").unwrap();

        assert_eq!(find_managed_dictionary_root(temp_dir.path()), None);
    }

    #[test]
    fn detection_reports_not_found_when_nothing_is_installed() {
        let temp_dir = tempfile::tempdir().unwrap();
        let settings = settings_for(temp_dir.path());

        let detection = detect_local_dictionary(&settings);
        assert_eq!(detection.status, "notFound");
        assert!(detection.dictionary_path.is_none());
        assert!(!detection.managed);
    }

    #[test]
    fn detection_reports_ready_for_an_installed_dictionary() {
        let temp_dir = tempfile::tempdir().unwrap();
        let nested = managed_dictionary_install_directory(temp_dir.path()).join("lindera-ipadic");
        write_dictionary_at(&nested);
        let settings = settings_for(temp_dir.path());

        let detection = detect_local_dictionary(&settings);
        assert_eq!(detection.status, "ready");
        assert_eq!(
            detection.dictionary_path,
            Some(nested.display().to_string())
        );
        assert!(detection.managed);
    }
}
