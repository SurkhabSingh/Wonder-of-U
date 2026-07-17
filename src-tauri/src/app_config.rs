pub(crate) const APP_SNAPSHOT_EVENT: &str = "app://snapshot-changed";
pub(crate) const AUTOSTART_ARGUMENT: &str = "--autostart";
pub(crate) const RECOMMENDED_WHISPER_RUNTIME_VERSION: &str = "v1.8.4";
pub(crate) const RECOMMENDED_WHISPER_RUNTIME_FILE: &str = "whisper-bin-x64.zip";
pub(crate) const RECOMMENDED_FFMPEG_RUNTIME_FILE: &str =
    "ffmpeg-master-latest-win64-gpl-shared.zip";
pub(crate) const RECOMMENDED_FFMPEG_RUNTIME_URL: &str =
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl-shared.zip";
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
