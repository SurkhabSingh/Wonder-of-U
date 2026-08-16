pub(crate) const APP_SNAPSHOT_EVENT: &str = "app://snapshot-changed";
pub(crate) const AUTOSTART_ARGUMENT: &str = "--autostart";
/// The whisper.cpp runtime a fresh install is given. **A deliberate pin, not the latest
/// release** — and not stale simply because whisper.cpp has moved on.
///
/// Transcription drives `whisper-cli` entirely through its command line, and depends on
/// eleven flags: `--file`, `--language`, `--model`, `--output-file`, `--output-json`,
/// `--output-txt`, `--print-progress`, `--suppress-nst`, `--vad`,
/// `--vad-max-speech-duration-s` and `--vad-model`. A floating "latest" would keep building
/// and keep passing its tests, and would break the first time somebody actually transcribed
/// something — `--suppress-nst` and the VAD flags are recent enough to be renamed, and
/// `--output-json` is what per-sentence playback reads. Same reasoning as the alass and
/// IPADIC pins below.
///
/// **Newer runtimes are not out of reach.** `check_whisper_runtime_update` asks GitHub for
/// the latest tag and reports it as available, and `download_whisper_runtime_version` installs
/// any version the user picks, side by side under `<asset_dir>/whisper-runtime/<version>/`.
/// This constant only decides the default.
///
/// **Bumping it is a real change, not a version-string edit.** Move this and
/// `RECOMMENDED_RUNTIME_VERSION` in `src/constants.ts` together — the test below fails
/// otherwise, because this one decides what a fresh settings file EXPECTS while that one
/// decides what gets DOWNLOADED. Then re-check the eleven flags against the new CLI and
/// transcribe something real: whisper output feeds transcripts, mining and i+1 ranking, so a
/// change in segmentation or timestamps is felt well beyond this file.
pub(crate) const RECOMMENDED_WHISPER_RUNTIME_VERSION: &str = "v1.8.4";
pub(crate) const RECOMMENDED_WHISPER_RUNTIME_FILE: &str = "whisper-bin-x64.zip";
pub(crate) const RECOMMENDED_FFMPEG_RUNTIME_FILE: &str =
    "ffmpeg-master-latest-win64-gpl-shared.zip";
pub(crate) const RECOMMENDED_FFMPEG_RUNTIME_URL: &str =
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl-shared.zip";
/// alass v2.0.0 — automatic subtitle synchronisation. Pinned to a tag rather than "latest"
/// so a release that changes the archive layout cannot silently break the extraction.
///
/// GPL-3.0, invoked as a separate process. See `runtime_assets/alass.rs`.
pub(crate) const ALASS_RELEASE_DOWNLOAD_URL: &str =
    "https://github.com/kaegi/alass/releases/download/v2.0.0/alass-windows64.zip";

pub(crate) const YTDLP_RELEASE_DOWNLOAD_URL: &str =
    "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
pub(crate) const YTDLP_RELEASES_API_URL: &str =
    "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest";
pub(crate) const IPADIC_DICTIONARY_VERSION: &str = "4.0.0";
pub(crate) const IPADIC_DICTIONARY_FILE: &str = "lindera-ipadic-4.0.0.zip";
/// Pinned to the lindera crate's own release — deliberately not a `/latest/` URL.
/// The on-disk dictionary format has to match the lindera version we compile
/// against, so a floating URL would keep building fine and only break when the
/// user tries to tokenize against a dictionary this binary cannot read.
pub(crate) const IPADIC_DICTIONARY_URL: &str =
    "https://github.com/lindera/lindera/releases/download/v4.0.0/lindera-ipadic-4.0.0.zip";

#[cfg(test)]
mod tests {
    use super::RECOMMENDED_WHISPER_RUNTIME_VERSION;

    /// The recommended runtime version is written down twice — here, and as
    /// `RECOMMENDED_RUNTIME_VERSION` in `src/constants.ts`. That is not cosmetic
    /// duplication: the frontend's copy is the version passed to
    /// `download_whisper_runtime_version`, so it decides what gets DOWNLOADED, while this
    /// one decides what a fresh settings file EXPECTS. Bump one alone and the app fetches
    /// a runtime it will not then look for.
    ///
    /// Reading the other language's source from a test is unusual, but the alternative is
    /// a build step to generate one from the other, and a version string bumped once a
    /// release does not earn one. This fails loudly the moment they disagree, which is
    /// the whole requirement.
    #[test]
    fn the_frontend_agrees_on_the_recommended_runtime_version() {
        let constants_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/constants.ts");
        let source = std::fs::read_to_string(&constants_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", constants_path.display()));

        let declaration = source
            .lines()
            .find(|line| line.contains("RECOMMENDED_RUNTIME_VERSION ="))
            .expect("constants.ts declares RECOMMENDED_RUNTIME_VERSION");
        let frontend_version = declaration
            .split('"')
            .nth(1)
            .expect("the declaration is a double-quoted string");

        assert_eq!(
            frontend_version, RECOMMENDED_WHISPER_RUNTIME_VERSION,
            "src/constants.ts and app_config.rs disagree about the recommended whisper runtime"
        );
    }
}
