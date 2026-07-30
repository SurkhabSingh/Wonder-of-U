pub(crate) const APP_SNAPSHOT_EVENT: &str = "app://snapshot-changed";
pub(crate) const AUTOSTART_ARGUMENT: &str = "--autostart";
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
