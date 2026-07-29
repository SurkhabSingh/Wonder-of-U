use std::{
    path::{Path, PathBuf},
    process::Command,
    sync::Mutex,
    time::SystemTime,
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// Pulls the version out of a `--version` banner.
///
/// The three tools print three different shapes:
///
/// ```text
/// ffmpeg version 7.1-full_build-www.gyan.dev Copyright (c) 2000-2024 the FFmpeg developers
/// alass 2.0.0
/// 2026.07.04
/// ```
///
/// So: first non-empty line, drop the name token if there is one, drop a literal `version`
/// if that is what follows, and take what is left. Dropping the name by POSITION rather than
/// by matching it means a binary calling itself `alass-cli` reads the same as one calling
/// itself `alass`, and yt-dlp — which prints the bare version and no name at all — is the
/// single-token case.
///
/// Every candidate is checked for a digit before being returned, so a tool that prints only
/// its name reports no version instead of reporting its name as one.
pub(super) fn version_from_banner(stdout: &str) -> Option<String> {
    fn looks_like_a_version(token: &str) -> bool {
        token.chars().any(|character| character.is_ascii_digit())
    }

    let line = stdout
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())?;
    let mut tokens = line.split_whitespace();
    let first = tokens.next()?;
    let candidate = match tokens.next() {
        None => first,
        Some(next) if next.eq_ignore_ascii_case("version") => tokens.next()?,
        Some(next) => next,
    };
    looks_like_a_version(candidate).then(|| candidate.to_string())
}

/// Runs `<binary> <flag>` and reads the version out of what it prints.
///
/// Returns `None` for anything that did not run or did not print a version — the version is
/// decoration beside a status the caller already knows, so a tool answering in a shape we do
/// not recognise shows no version rather than blocking detection or reporting a guess.
fn probe_version(executable_path: &Path, flag: &str) -> Option<String> {
    let mut command = Command::new(executable_path);
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);
    let output = command.arg(flag).output().ok()?;
    if !output.status.success() {
        return None;
    }
    version_from_banner(&String::from_utf8_lossy(&output.stdout))
}

/// Which file a cached version describes.
///
/// Length and modified time rather than a hash: this is compared on every detection, and
/// detection already stats these paths. A download replaces the binary, so both move.
#[derive(Clone, PartialEq, Eq)]
struct FileIdentity {
    path: PathBuf,
    length: u64,
    modified: Option<SystemTime>,
}

impl FileIdentity {
    fn read(path: &Path) -> Option<Self> {
        let metadata = std::fs::metadata(path).ok()?;
        Some(Self {
            path: path.to_path_buf(),
            length: metadata.len(),
            modified: metadata.modified().ok(),
        })
    }
}

#[derive(Default)]
struct VersionState {
    describes: Option<FileIdentity>,
    version: Option<String>,
    refreshing: bool,
}

/// The version one managed binary reports, resolved off the detection path.
///
/// **`version_for` never spawns a process.** `detect_local_*` runs on every app-snapshot
/// emit, and emits fire on every download-progress tick — spawning there is what stalled the
/// import queue before, and is why the managed branch of detection trusts a binary's mere
/// presence instead of running it. Reading a version means running it, so the run happens on
/// a background thread and detection only ever reads what that thread left behind.
///
/// The cache invalidates on the file itself rather than on a timer, so a download that
/// replaces the binary is picked up because its length and modified time moved — no
/// completion hook to forget at one of the five download sites.
pub(super) struct VersionCache {
    state: Mutex<VersionState>,
    flag: &'static str,
}

impl VersionCache {
    pub(super) const fn new(flag: &'static str) -> Self {
        Self {
            state: Mutex::new(VersionState {
                describes: None,
                version: None,
                refreshing: false,
            }),
            flag,
        }
    }

    /// The version, if one is already known for the file currently at `path`.
    ///
    /// Returns `None` and starts one background probe otherwise. `None` is the same answer
    /// the UI gives for "not installed", so a version that has not resolved yet simply is
    /// not shown — it never blocks, and never reports a version belonging to a binary that
    /// has since been replaced.
    pub(super) fn version_for(&'static self, path: &Path) -> Option<String> {
        let identity = FileIdentity::read(path)?;
        let mut state = self.state.lock().ok()?;
        if state.describes.as_ref() == Some(&identity) {
            return state.version.clone();
        }
        if state.refreshing {
            return None;
        }
        state.refreshing = true;
        drop(state);

        let flag = self.flag;
        std::thread::spawn(move || {
            let version = probe_version(&identity.path, flag);
            if let Ok(mut state) = self.state.lock() {
                state.describes = Some(identity);
                state.version = version;
                state.refreshing = false;
            }
        });
        None
    }

    /// Resolves the version now, blocking until it has. For the startup warm-up, which runs
    /// on its own thread so the first snapshot already carries versions.
    pub(super) fn warm(&'static self, path: &Path) {
        let Some(identity) = FileIdentity::read(path) else {
            return;
        };
        let version = probe_version(path, self.flag);
        if let Ok(mut state) = self.state.lock() {
            state.describes = Some(identity);
            state.version = version;
            state.refreshing = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::version_from_banner;

    #[test]
    fn reads_the_version_from_each_tools_real_banner() {
        assert_eq!(
            version_from_banner(
                "ffmpeg version 7.1-full_build-www.gyan.dev Copyright (c) 2000-2024\nbuilt with gcc"
            )
            .as_deref(),
            Some("7.1-full_build-www.gyan.dev")
        );
        assert_eq!(version_from_banner("alass 2.0.0").as_deref(), Some("2.0.0"));
        // yt-dlp prints the version and nothing else.
        assert_eq!(
            version_from_banner("2026.07.04").as_deref(),
            Some("2026.07.04")
        );
    }

    #[test]
    fn the_name_is_dropped_by_position_so_a_renamed_binary_still_reads() {
        // The alass archive ships `alass-cli.exe`, and nothing guarantees the banner keeps
        // calling itself plain `alass`.
        assert_eq!(
            version_from_banner("alass-cli 2.0.0").as_deref(),
            Some("2.0.0")
        );
    }

    #[test]
    fn a_banner_with_no_version_is_none_rather_than_the_tools_own_name() {
        assert_eq!(version_from_banner("alass"), None);
        assert_eq!(version_from_banner(""), None);
        assert_eq!(version_from_banner("   \n  \n"), None);
        // "version" with nothing after it must not fall through to reporting "version".
        assert_eq!(version_from_banner("ffmpeg version"), None);
        // A name-only banner must not report the name as the version.
        assert_eq!(version_from_banner("ffmpeg built with gcc"), None);
    }

    #[test]
    fn leading_blank_lines_are_skipped() {
        assert_eq!(
            version_from_banner("\n\n  ffmpeg version 6.0 Copyright").as_deref(),
            Some("6.0")
        );
    }
}

/// Resolves every managed binary's version, blocking.
///
/// Call this on a background thread at startup so the first snapshot the settings page sees
/// already carries versions. Afterwards the caches keep themselves current: each one notices
/// when the file it described has been replaced.
pub(crate) fn warm_asset_versions(settings: &crate::app_types::AppSettings) {
    use super::{alass, ffmpeg, ytdlp};

    if let Some(path) = ffmpeg::detect_local_ffmpeg(settings).executable_path {
        ffmpeg::FFMPEG_VERSION.warm(std::path::Path::new(&path));
    }
    if let Some(path) = ytdlp::detect_local_ytdlp(settings).executable_path {
        ytdlp::YTDLP_VERSION.warm(std::path::Path::new(&path));
    }
    if let Some(path) = alass::detect_local_alass(settings).executable_path {
        alass::ALASS_VERSION.warm(std::path::Path::new(&path));
    }
}
