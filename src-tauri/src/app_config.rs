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
/// The **LGPL** build, deliberately, not the GPL one.
///
/// BtbN publishes both. The GPL variant exists to carry libx264 and libx265, which this app
/// has never used: it encodes with libmp3lame, libvpx-vp9 and libopus, and only ever *decodes*
/// H.264 and AAC — and those decoders are native to ffmpeg, present in every build. So the GPL
/// variant bought nothing and cost the strongest copyleft terms available.
///
/// Verified against the real binary rather than the build scripts: `-buildconf` carries
/// `--enable-version3` with no `--enable-gpl`, and explicit `--disable-libx264
/// --disable-libx265`; all five codecs the app names are present; a WAV to MP3 and a
/// VP9+Opus WebM both encode.
///
/// Why it matters even though the app only *fetches* this and never ships it: fetching
/// conveys nothing and carries no obligation either way, but a GPL binary invites the argument
/// that app and binary are one combined work — an argument that, if it ever landed, would
/// reach this app's own licence. Under LGPL the same argument's worst case is a notice
/// requirement. It removes the argument, not an obligation we actually had.
///
/// Note the URL floats: `latest` rebuilds daily from FFmpeg master. That is fine while we
/// fetch. It would have to be pinned to a dated build before anyone bundles this, because
/// the source has to correspond exactly to the binary.
pub(crate) const RECOMMENDED_FFMPEG_RUNTIME_FILE: &str =
    "ffmpeg-master-latest-win64-lgpl-shared.zip";
pub(crate) const RECOMMENDED_FFMPEG_RUNTIME_URL: &str =
    "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-lgpl-shared.zip";
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

/// mpv, the player Watch & Mine drives over its IPC channel.
///
/// The project's OWN Windows build, pinned to a release tag. The commonly linked third-party
/// nightly builds were measured and rejected: their filenames carry a date and a build hash so
/// no permanent link exists, and both builders delete releases after about a month, which turns
/// a saved URL into a download that silently starts failing. This tag's assets do not expire.
///
/// The msvc archive rather than the mingw one, though it downloads larger: mingw unpacks to
/// 101 MiB across 27 loose libraries and is a zip nested inside a zip, while this is 55.7 MiB
/// and five files once the debug symbols are skipped. It also ships `vulkan-1.dll`, which
/// `mpv.exe` imports at load time — the third-party builds omit it and rely on a graphics
/// driver having placed one in the system folder, so they fail outright on a machine that has
/// none.
///
/// Fetched, never bundled. mpv is GPL and has no LGPL build for the player — verified on the
/// binary, which exposes `direct3d`, a GPL-gated output. Fetching at the user's request carries
/// no obligation; shipping it would mean supplying the corresponding source for mpv and every
/// GPL library inside it, and the archive carries no licence file at all.
pub(crate) const MPV_RELEASE_FILE: &str = "mpv-v0.41.0-x86_64-pc-windows-msvc.zip";
pub(crate) const MPV_RELEASE_URL: &str = "https://github.com/mpv-player/mpv/releases/download/v0.41.0/mpv-v0.41.0-x86_64-pc-windows-msvc.zip";

/// The published digest of the archive above, checked after it is fetched.
///
/// The URL is pinned to an immutable tag, so this constant can be too. It is the only integrity
/// check any asset has: every other download is trusted on the strength of having produced a
/// file at all, which cannot tell a truncated transfer from a complete one.
pub(crate) const MPV_RELEASE_SHA256: &str =
    "4e197f729f5071c6772f35fffd96e0f36e3e8a044bd9479b136bb09b7c6a80ff";

/// Debug symbols. Four fifths of the unpacked archive and of no use to anyone running the
/// player, so they are skipped rather than written and deleted.
pub(crate) const MPV_SKIPPED_ENTRY: &str = "mpv.pdb";

#[cfg(test)]
mod tests {
    use super::{
        RECOMMENDED_FFMPEG_RUNTIME_FILE, RECOMMENDED_FFMPEG_RUNTIME_URL,
        RECOMMENDED_WHISPER_RUNTIME_VERSION,
    };

    /// The FFmpeg build is a licensing decision, not a URL.
    ///
    /// BtbN's `gpl` and `lgpl` variants differ by one word in the filename and are otherwise
    /// interchangeable to anyone reading quickly — which is exactly how the stronger copyleft
    /// terms could come back by accident, in a commit that looks like a typo fix. The app uses
    /// no libx264 or libx265, so the GPL variant has nothing to offer it.
    #[test]
    fn the_ffmpeg_build_is_the_lgpl_variant() {
        for value in [RECOMMENDED_FFMPEG_RUNTIME_FILE, RECOMMENDED_FFMPEG_RUNTIME_URL] {
            assert!(
                value.contains("lgpl"),
                "the LGPL build is a deliberate licensing choice: {value}"
            );
            // `contains("gpl")` would match "lgpl" too, so test the boundary the name turns on.
            assert!(
                !value.contains("-gpl"),
                "this is the GPL variant, which the app has no use for: {value}"
            );
        }
    }

    /// The installer's compressor is a licensing decision, and `tauri.conf.json` cannot hold a
    /// comment saying so.
    ///
    /// `makensis` is not only a build tool: it writes its own ~38 KB exehead into the front of
    /// every installer it produces, so that code is redistributed rather than merely run.
    /// Measured on the real artifact — 90% of the first 38 KB matches the stub byte for byte,
    /// in one contiguous run, and NSIS's own UI strings are in there.
    ///
    /// Almost all of that stub is zlib/libpng, which imposes nothing on a binary. The exception
    /// is the LZMA decoder, which is Common Public License 1.0 — and LZMA is Tauri's default.
    /// A linking exception in NSIS's COPYING keeps CPL off *our* code, but is silent on CPL's
    /// own source-availability duty for the decoder it embeds, and no primary source resolves
    /// that. bzip2 and zlib stubs contain no CPL code at all, so the question simply does not
    /// arise. It cost 1.6 MB, against the 543 MiB a new install downloads anyway.
    ///
    /// Read from the config rather than restated, because restating it is how the file and the
    /// reason for it drift apart.
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

    /// The URL has to name the same archive the downloader stages and unpacks, or ffmpeg is
    /// fetched under one name and looked for under another.
    #[test]
    fn the_ffmpeg_url_ends_with_the_file_it_stages() {
        assert!(
            RECOMMENDED_FFMPEG_RUNTIME_URL.ends_with(RECOMMENDED_FFMPEG_RUNTIME_FILE),
            "{RECOMMENDED_FFMPEG_RUNTIME_URL} does not end with {RECOMMENDED_FFMPEG_RUNTIME_FILE}"
        );
    }

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
