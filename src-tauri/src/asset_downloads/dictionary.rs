use std::{
    fs,
    path::{Path, PathBuf},
};

use tauri::{AppHandle, Runtime};

use crate::{
    app_config::{IPADIC_DICTIONARY_FILE, IPADIC_DICTIONARY_URL},
    runtime_assets::{find_managed_dictionary_root, managed_dictionary_install_directory},
    tokenizer::dictionary_loads,
};

use super::asset::AssetKind;
use super::envelope::{AssetDownloadPlan, Installed};
use super::transfer::{
    asset_directory, ensure_directory_exists, extract_zip_archive_to_directory,
    verify_managed_directory_or_remove, PartialInstallGuard,
};

/// Where the dictionary's archive is staged and where it unpacks to.
struct DictionaryPaths {
    archive: PathBuf,
    install: PathBuf,
}

fn dictionary_paths(asset_directory: &Path) -> DictionaryPaths {
    DictionaryPaths {
        archive: asset_directory
            .join("downloads")
            .join(IPADIC_DICTIONARY_FILE),
        install: managed_dictionary_install_directory(asset_directory),
    }
}

/// Proves the extracted dictionary is one lindera can actually read.
fn verify_extracted_dictionary(
    install_directory: &Path,
    asset_directory: &Path,
) -> Result<PathBuf, String> {
    let dictionary_path = find_managed_dictionary_root(asset_directory).ok_or_else(|| {
        format!(
            "The dictionary was downloaded, but no dictionary was found under {}.",
            install_directory.display()
        )
    })?;
    dictionary_loads(&dictionary_path)?;
    Ok(dictionary_path)
}

/// Downloads the pinned IPADIC dictionary into `<asset_dir>/lindera-ipadic/<version>/`.
pub(super) fn dictionary_plan<R: Runtime>(
    app: &AppHandle<R>,
) -> Result<AssetDownloadPlan<R>, String> {
    let asset_directory = asset_directory(app)?;
    let paths = dictionary_paths(&asset_directory);
    ensure_directory_exists(
        paths
            .archive
            .parent()
            .ok_or_else(|| "The downloads directory has no parent.".to_string())?,
    )?;
    ensure_directory_exists(&paths.install)?;

    let shell_start_text = format!(
        "Downloading the Japanese dictionary to {}...",
        paths.install.display()
    );

    let starting_target_path = paths.archive.clone();

    Ok(AssetDownloadPlan {
        kind: AssetKind::Dictionary,
        slot_busy_message: "Another download is already in progress.".into(),
        shell_start_text,
        starting_message: "Preparing the Japanese dictionary download...".into(),
        starting_target_path,
        cancelled_message: "Japanese dictionary download cancelled.".into(),
        cancelled_shell_text: "Japanese dictionary download cancelled.".into(),
        failed_message_prefix: "Japanese dictionary download failed".into(),
        failed_shell_prefix: "Japanese dictionary download failed".into(),
        success_log_event: "dictionary.downloaded",
        failure_log_event: "dictionary.download_failed",
        install: Box::new(move |context| {
            context.fetch(
                IPADIC_DICTIONARY_URL,
                &paths.archive,
                "the Japanese dictionary",
            )?;

            let mut install_guard = PartialInstallGuard::new(paths.install.clone());
            extract_zip_archive_to_directory(&paths.archive, &paths.install)?;
            install_guard.disarm();

            let dictionary_path = verify_managed_directory_or_remove(&paths.install, |_| {
                verify_extracted_dictionary(&paths.install, &asset_directory)
            })?;

            let log_details = serde_json::json!({
                "archivePath": paths.archive.display().to_string(),
                "dictionaryPath": dictionary_path.display().to_string()
            });
            // Success only. A failed run leaves the archive in `downloads/` on purpose —
            // unlike alass, whose 26 MB is mostly ffmpeg and worth nothing to anyone.
            let _ = fs::remove_file(&paths.archive);

            Ok(Installed {
                completed_message:
                    "The Japanese dictionary is ready. Sentences can be analysed word by word."
                        .into(),
                shell_success_text: format!(
                    "The Japanese dictionary is ready at {}.",
                    dictionary_path.display()
                ),
                target_path: dictionary_path,
                log_details,
            })
        }),
    })
}

#[cfg(test)]
mod tests {
    use super::dictionary_paths;
    use crate::app_config::IPADIC_DICTIONARY_VERSION;
    use std::path::Path;

    /// The dictionary stages through the shared `downloads/` directory — the opposite of
    /// alass, which keeps its archive beside the binary. Getting these two the same way round
    /// is the mistake a conversion invites, and neither would fail to compile.
    #[test]
    fn the_archive_stages_in_downloads_not_beside_the_install() {
        let paths = dictionary_paths(Path::new("C:/assets"));

        assert!(
            paths
                .archive
                .components()
                .any(|part| part.as_os_str() == "downloads"),
            "expected downloads/ staging: {:?}",
            paths.archive
        );
        assert_ne!(paths.archive.parent(), Some(paths.install.as_path()));
    }

    /// The install directory is version-pinned, because the on-disk format has to match the
    /// lindera the binary was compiled against — see `IPADIC_DICTIONARY_URL`'s comment.
    #[test]
    fn the_install_directory_is_pinned_to_the_dictionary_version() {
        let paths = dictionary_paths(Path::new("C:/assets"));

        assert!(
            paths
                .install
                .components()
                .any(|part| part.as_os_str() == IPADIC_DICTIONARY_VERSION),
            "expected the version in the path: {:?}",
            paths.install
        );
    }
}
