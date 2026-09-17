pub(crate) const APP_SNAPSHOT_EVENT: &str = "app://snapshot-changed";
pub(crate) const PROGRESS_EVENT: &str = "progress://changed";
pub(crate) const CARD_MADE_EVENT: &str = "progress://card-made";
pub(crate) const AUTOSTART_ARGUMENT: &str = "--autostart";

pub(crate) const RECOMMENDED_WHISPER_RUNTIME_VERSION: &str = "v1.8.4";
pub(crate) const RECOMMENDED_WHISPER_RUNTIME_FILE: &str = "whisper-bin-x64.zip";

pub(crate) const RECOMMENDED_FFMPEG_RUNTIME_FILE: &str =
    "ffmpeg-master-latest-win64-lgpl-shared.zip";
pub(crate) const RECOMMENDED_FFMPEG_RUNTIME_URL: &str =
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-lgpl-shared.zip";

pub(crate) const ALASS_RELEASE_DOWNLOAD_URL: &str =
    "https://github.com/kaegi/alass/releases/download/v2.0.0/alass-windows64.zip";

pub(crate) const YTDLP_RELEASE_DOWNLOAD_URL: &str =
    "https://github.com/yt-dlp/yt-dlp/releases/latest/download/yt-dlp.exe";
pub(crate) const YTDLP_RELEASES_API_URL: &str =
    "https://api.github.com/repos/yt-dlp/yt-dlp/releases/latest";
pub(crate) const IPADIC_DICTIONARY_VERSION: &str = "4.0.0";
pub(crate) const IPADIC_DICTIONARY_FILE: &str = "lindera-ipadic-4.0.0.zip";

pub(crate) const IPADIC_DICTIONARY_URL: &str =
    "https://github.com/lindera/lindera/releases/download/v4.0.0/lindera-ipadic-4.0.0.zip";

pub(crate) const MPV_RELEASE_FILE: &str = "mpv-v0.41.0-x86_64-pc-windows-msvc.zip";
pub(crate) const MPV_RELEASE_URL: &str = "https://github.com/mpv-player/mpv/releases/download/v0.41.0/mpv-v0.41.0-x86_64-pc-windows-msvc.zip";

/// The published digest of the archive above, checked after it is fetched.
pub(crate) const MPV_RELEASE_SHA256: &str =
    "4e197f729f5071c6772f35fffd96e0f36e3e8a044bd9479b136bb09b7c6a80ff";

pub(crate) const MPV_SKIPPED_ENTRY: &str = "mpv.pdb";

#[cfg(test)]
mod tests {
    use super::{
        CARD_MADE_EVENT, PROGRESS_EVENT, RECOMMENDED_FFMPEG_RUNTIME_FILE,
        RECOMMENDED_FFMPEG_RUNTIME_URL, RECOMMENDED_WHISPER_RUNTIME_VERSION,
    };

    #[test]
    fn the_ffmpeg_build_is_the_lgpl_variant() {
        for value in [RECOMMENDED_FFMPEG_RUNTIME_FILE, RECOMMENDED_FFMPEG_RUNTIME_URL] {
            assert!(
                value.contains("lgpl"),
                "the LGPL build is a deliberate licensing choice: {value}"
            );
            assert!(
                !value.contains("-gpl"),
                "this is the GPL variant, which the app has no use for: {value}"
            );
        }
    }

    #[test]
    fn the_installer_uses_a_compressor_with_no_copyleft_stub() {
        let config_path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tauri.conf.json");
        let source = std::fs::read_to_string(&config_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", config_path.display()));
        let config: serde_json::Value =
            serde_json::from_str(&source).expect("tauri.conf.json is valid JSON");

        let compression = config["bundle"]["windows"]["nsis"]["compression"].as_str();

        assert!(
            matches!(compression, Some("bzip2") | Some("zlib")),
            "lzma pulls a CPL-1.0 decoder into the shipped installer; got {compression:?}"
        );
    }

    #[test]
    fn the_ffmpeg_url_ends_with_the_file_it_stages() {
        assert!(
            RECOMMENDED_FFMPEG_RUNTIME_URL.ends_with(RECOMMENDED_FFMPEG_RUNTIME_FILE),
            "{RECOMMENDED_FFMPEG_RUNTIME_URL} does not end with {RECOMMENDED_FFMPEG_RUNTIME_FILE}"
        );
    }

    fn frontend_constant(name: &str) -> String {
        let constants_path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../src/constants.ts");
        let source = std::fs::read_to_string(&constants_path)
            .unwrap_or_else(|error| panic!("read {}: {error}", constants_path.display()));

        let declaration = source
            .lines()
            .find(|line| line.contains(&format!("{name} =")))
            .unwrap_or_else(|| panic!("constants.ts declares {name}"));
        declaration
            .split('"')
            .nth(1)
            .expect("the declaration is a double-quoted string")
            .to_string()
    }

    #[test]
    fn the_frontend_agrees_on_the_recommended_runtime_version() {
        assert_eq!(
            frontend_constant("RECOMMENDED_RUNTIME_VERSION"),
            RECOMMENDED_WHISPER_RUNTIME_VERSION,
            "src/constants.ts and app_config.rs disagree about the recommended whisper runtime"
        );
    }

    #[test]
    fn the_frontend_listens_for_progress_under_the_names_it_is_sent() {
        assert_eq!(frontend_constant("PROGRESS_EVENT"), PROGRESS_EVENT);
        assert_eq!(frontend_constant("CARD_MADE_EVENT"), CARD_MADE_EVENT);
    }
}
