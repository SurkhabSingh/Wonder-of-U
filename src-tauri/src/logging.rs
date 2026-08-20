//! The one place a log line is written.
//!
//! Every line goes through [`write`], which is what makes the redaction below reliable: a
//! second writer would be fail-open on whatever it forgot, and a log file is only shareable if
//! nothing can leak into it by a path nobody checked.
//!
//! The file is JSON Lines — one object per line — so it can be grepped, opened in a text
//! editor, and parsed without a schema.

use std::{
    fs::{self, OpenOptions},
    io::Write,
    path::Path,
    sync::{Mutex, OnceLock},
    time::{SystemTime, UNIX_EPOCH},
};

use serde::Serialize;
use serde_json::Value;

/// Rotate once the active file passes this, keeping [`KEPT_FILES`] in total.
///
/// Measured against real use at roughly 8 KiB a day, so three 2 MB files hold well over a
/// year. The cap exists to bound disk and to bound how much history one shared file exposes,
/// not because the volume is a problem.
const MAX_BYTES: u64 = 2 * 1024 * 1024;
const KEPT_FILES: usize = 3;

/// What a path is rewritten to before it reaches the file.
///
/// A real environment variable rather than an opaque token: it still resolves in a shell, so
/// nothing is lost for debugging, while the account name it expands to never leaves the
/// machine.
const HOME_PLACEHOLDER: &str = "%USERPROFILE%";

/// One record. Field order is declaration order, which is why this is a struct and not a
/// `json!` literal — `serde_json::json!` builds a `BTreeMap` and sorts keys alphabetically, so
/// every line used to open with `details` and bury the timestamp at the end.
#[derive(Serialize)]
struct LogLine<'a> {
    ts: String,
    level: &'a str,
    event: &'a str,
    msg: String,
    run: &'a str,
    details: Value,
}

/// Identifies one launch. Every line carries it, so a whole run can be isolated from a file
/// that holds many.
static RUN: OnceLock<String> = OnceLock::new();

/// Why logging last failed, if it has. Read by the bootstrap so the app can say so instead of
/// handing over a file that is silently short.
static FAILURE: Mutex<Option<String>> = Mutex::new(None);

/// Serialises rotation and the append against each other.
static FILE: Mutex<()> = Mutex::new(());

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// This launch's id, for the line that reports what the app is running on.
pub(crate) fn run_id() -> &'static str {
    RUN.get_or_init(|| format!("{:x}", now_ms()))
}

/// The reason logging is not working, if it is not.
pub(crate) fn failure() -> Option<String> {
    FAILURE.lock().ok().and_then(|failure| failure.clone())
}

fn record_failure(reason: String) {
    // Printed as well as stored: during development the terminal is where it will be seen, and
    // a logger that cannot report its own failure is the one component that has nowhere else
    // to go.
    eprintln!("wonder-of-u: log write failed: {reason}");
    if let Ok(mut failure) = FAILURE.lock() {
        *failure = Some(reason);
    }
}

/// The user's home directory, or `None` when the environment does not say.
fn home_directory() -> Option<String> {
    for key in ["USERPROFILE", "HOME"] {
        if let Ok(value) = std::env::var(key) {
            let trimmed = value.trim_end_matches(['\\', '/']);
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

/// Replaces the home directory wherever it appears in a string.
///
/// Windows paths are case-insensitive and reach the log in more than one casing, so the search
/// is too — but only over ASCII. Full Unicode lowering changes byte lengths (the Kelvin sign is
/// three bytes and lowers to one), and an offset found in a lowered copy is then not an offset
/// into the original: the cut lands in the wrong place, leaving part of the account name behind
/// or slicing mid-character and panicking. ASCII lowering is length-preserving, which makes
/// that misalignment impossible rather than something to check for, and the casing that varies
/// on Windows — the drive letter and `Users` — is ASCII.
fn redact_string(text: &str, home: &str) -> String {
    let haystack = text.to_ascii_lowercase();
    let needle = home.to_ascii_lowercase();
    if !haystack.contains(&needle) {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.to_ascii_lowercase().find(&needle) {
        out.push_str(&rest[..index]);
        out.push_str(HOME_PLACEHOLDER);
        rest = &rest[index + home.len()..];
    }
    out.push_str(rest);
    out
}

/// The folder recordings live in, so their names can be replaced before they are written.
///
/// Set at startup and again whenever the setting changes. `None` means the folder is not known
/// yet, and a name is left alone rather than guessed at.
static RECORDINGS: Mutex<Option<String>> = Mutex::new(None);

/// Tells the writer where recordings live.
pub(crate) fn set_recordings_directory(directory: &str) {
    if let Ok(mut recordings) = RECORDINGS.lock() {
        *recordings = Some(directory.trim_end_matches(['\\', '/']).to_string());
    }
}

fn recordings_directory() -> Option<String> {
    RECORDINGS.lock().ok().and_then(|value| value.clone())
}

/// What a recording is called in the log.
///
/// A recording's name is the first words of its transcript, so it is a sample of what was said;
/// an imported one is named after the video. Neither belongs in a file meant to be handed to a
/// stranger, and neither has a shape a redactor can find, so the name is replaced wholesale.
///
/// The recording id is kept when the name carries one — recordings are saved as
/// `{stem}_{id}{ext}` and the id survives the rename that happens when a transcript lands, so
/// it is the one part that identifies a recording across its whole life. Names without an id
/// (imports keep the source's own name) get a short digest instead, which links records about
/// the same file without saying what the file is. Every extension is kept, because which
/// sidecar a line is about is the useful half.
fn recording_reference(file_name: &str) -> String {
    if let Some(id_start) = recording_id_start(file_name) {
        return file_name[id_start..].to_string();
    }

    let cut = file_name.find('.').unwrap_or(file_name.len());
    let (stem, extensions) = file_name.split_at(cut);

    use std::hash::{Hash, Hasher};
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    stem.hash(&mut hasher);
    format!("{:x}{extensions}", hasher.finish())
}

/// Where the trailing `_{id}` starts, if the name carries one.
///
/// Ten digits or more, because the id is a millisecond timestamp and a shorter run of digits is
/// far more likely to be part of a title.
fn recording_id_start(file_name: &str) -> Option<usize> {
    let bytes = file_name.as_bytes();
    for (index, _) in file_name.match_indices('_') {
        let digits = bytes[index + 1..]
            .iter()
            .take_while(|byte| byte.is_ascii_digit())
            .count();
        if digits >= 10 {
            return Some(index + 1);
        }
    }
    None
}

/// Replaces the name of any recording mentioned in a string.
fn redact_recording_names(text: &str, recordings: &str) -> String {
    let lowered = text.to_ascii_lowercase();
    let needle = recordings.to_ascii_lowercase();
    if !lowered.contains(&needle) {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.to_ascii_lowercase().find(&needle) {
        let after_directory = index + recordings.len();
        out.push_str(&rest[..after_directory]);
        rest = &rest[after_directory..];

        // Only the segment straight after the folder is a recording name; anything beyond the
        // next separator is a different path and is left to the next pass.
        let separator = if rest.starts_with(['\\', '/']) { 1 } else { 0 };
        out.push_str(&rest[..separator]);
        rest = &rest[separator..];

        let end = rest.find(['"', '\\', '/']).unwrap_or(rest.len());
        let (name, remainder) = rest.split_at(end);
        if name.is_empty() {
            continue;
        }
        out.push_str(&recording_reference(name));
        rest = remainder;
    }
    out.push_str(rest);
    out
}

/// Rewrites the home directory out of every string anywhere in the payload.
///
/// Walks the whole value rather than naming fields, so a field added later is covered without
/// anyone remembering to add it here — 84% of lines carried the account name before this, and
/// they carried it in twenty-seven differently named fields plus free text.
fn redact(value: &mut Value, home: &str, recordings: Option<&str>) {
    match value {
        Value::String(text) => *text = redact_one(text, home, recordings),
        Value::Array(items) => items
            .iter_mut()
            .for_each(|item| redact(item, home, recordings)),
        Value::Object(fields) => fields
            .values_mut()
            .for_each(|field| redact(field, home, recordings)),
        _ => {}
    }
}

/// Recording names first, then the home directory.
///
/// Order matters: the recordings folder is stored as the real path, and rewriting the home
/// directory first would leave nothing for it to match.
fn redact_one(text: &str, home: &str, recordings: Option<&str>) -> String {
    let named = match recordings {
        Some(directory) => redact_recording_names(text, directory),
        None => text.to_string(),
    };
    redact_string(&named, home)
}

/// Moves `details.message` up to the record's own `msg`.
///
/// Call sites carry the human sentence inside `details` today. Lifting it gives every line the
/// same shape without editing them all, and stops the sentence being duplicated once they are.
fn take_message(details: &mut Value) -> String {
    let Value::Object(fields) = details else {
        return String::new();
    };
    match fields.remove("message") {
        Some(Value::String(text)) => text,
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

/// Renames the active file out of the way once it grows past [`MAX_BYTES`].
///
/// Size rather than a daily file because volume here follows use, not the clock: a heavy day
/// would overrun a daily file while a quiet week would leave empty ones behind.
fn rotate_if_needed(path: &Path) {
    let too_big = fs::metadata(path).map(|meta| meta.len() >= MAX_BYTES).unwrap_or(false);
    if !too_big {
        return;
    }

    let numbered = |index: usize| path.with_extension(format!("{index}.log"));

    // Oldest first, so a rename never lands on a file that has not moved yet. The oldest is
    // dropped by being renamed over, which is what bounds the set at KEPT_FILES.
    for index in (1..KEPT_FILES - 1).rev() {
        let _ = fs::rename(numbered(index), numbered(index + 1));
    }
    let _ = fs::rename(path, numbered(1));
}

/// Whether routine detail is written.
///
/// Four levels, and DEBUG is the one that is normally dropped: two events accounted for 44% of
/// the file, and a log whose signal is a twentieth of its lines is one nobody reads. Set
/// `WONDER_OF_U_LOG=debug` to keep them while working on something.
fn debug_enabled() -> bool {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    *ENABLED.get_or_init(|| debug_requested(std::env::var("WONDER_OF_U_LOG").ok().as_deref()))
}

/// The decision, separated from the latch that caches it.
///
/// `debug_enabled` reads the environment once per process, so a test can only ever observe
/// whichever branch the ambient environment selected. Both answers are testable here.
fn debug_requested(setting: Option<&str>) -> bool {
    setting.is_some_and(|value| value.eq_ignore_ascii_case("debug"))
}

/// Writes one line. The only function in the app that appends to the log.
pub(crate) fn write(path: &Path, level: &str, event: &str, mut details: Value) {
    if level == "DEBUG" && !debug_enabled() {
        return;
    }
    let msg = take_message(&mut details);
    let mut line = LogLine {
        ts: timestamp(),
        level,
        event,
        msg,
        run: run_id(),
        details,
    };

    if let Some(home) = home_directory() {
        let recordings = recordings_directory();
        line.msg = redact_one(&line.msg, &home, recordings.as_deref());
        redact(&mut line.details, &home, recordings.as_deref());
    }

    let Ok(encoded) = serde_json::to_string(&line) else {
        record_failure(format!("event {event} could not be encoded"));
        return;
    };

    // Rotation and the append are one critical section. Without it two threads that both see
    // a full file both run the rename chain and one generation is renamed over; and `writeln!`
    // issues the payload and the newline as separate appends, which another thread can split.
    // The recorder, the download worker and the main thread all log.
    let _guard = FILE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());

    rotate_if_needed(path);

    let opened = OpenOptions::new().create(true).append(true).open(path);
    match opened {
        // One `write_all` of payload-plus-newline, so a record is a single append.
        Ok(mut file) => {
            if let Err(error) = file.write_all(format!("{encoded}\n").as_bytes()) {
                record_failure(error.to_string());
            }
        }
        Err(error) => record_failure(error.to_string()),
    }
}

/// Local time as RFC 3339 with milliseconds, e.g. `2026-08-19T15:04:05.123+05:30`.
///
/// Local rather than UTC so that "it stopped working around three" lines up with the file
/// without anyone converting anything, and offset-qualified so it is still unambiguous when the
/// file is read on another machine.
fn timestamp() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false)
}

/// What this machine is, for the first line of every run.
///
/// A handed-over log answers "what went wrong" only if it also answers "on what". Without this
/// every question about a report starts with a round trip asking for the version.
///
/// The run id is not here: every record already carries it, and describing the machine is a
/// different question from identifying the launch.
pub(crate) fn environment() -> serde_json::Value {
    serde_json::json!({
        "app": env!("CARGO_PKG_VERSION"),
        "os": windows_release(),
        "arch": std::env::consts::ARCH,
        "cpus": std::thread::available_parallelism()
            .map(|count| count.get())
            .unwrap_or(0),
        "webview": tauri::webview_version().unwrap_or_else(|_| "unknown".into()),
    })
}

/// The Windows edition and build, read from the registry.
///
/// The registry rather than an API call because `winreg` is already a dependency and this needs
/// no unsafe block. `CurrentBuild` is what distinguishes the releases that actually behave
/// differently; the marketing name alone does not.
#[cfg(windows)]
fn windows_release() -> String {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};

    let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey(r"SOFTWARE\Microsoft\Windows NT\CurrentVersion")
    else {
        return "windows (unreadable)".into();
    };
    let read = |name: &str| key.get_value::<String, _>(name).unwrap_or_default();

    let product = read("ProductName");
    let display = read("DisplayVersion");
    let build = read("CurrentBuild");
    let ubr: u32 = key.get_value("UBR").unwrap_or(0);

    let mut description = if product.is_empty() { "Windows".into() } else { product };
    if !display.is_empty() {
        description.push_str(&format!(" {display}"));
    }
    if !build.is_empty() {
        description.push_str(&format!(" (build {build}.{ubr})"));
    }
    description
}

#[cfg(not(windows))]
fn windows_release() -> String {
    std::env::consts::OS.to_string()
}

/// Writes a panic to the log before the process goes.
///
/// A crash is the one case where nothing was recorded at all: the default hook prints to a
/// stderr no user sees. Installed once at startup, and it chains to the previous hook so the
/// usual console output still happens while developing.
pub(crate) fn install_panic_hook(path: std::path::PathBuf) {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // `info.payload()` is the panic's own message; the location is where it was raised.
        let message = info
            .payload()
            .downcast_ref::<&str>()
            .map(|text| (*text).to_string())
            .or_else(|| info.payload().downcast_ref::<String>().cloned())
            .unwrap_or_else(|| "panic".to_string());
        let location = info
            .location()
            .map(|at| format!("{}:{}:{}", at.file(), at.line(), at.column()))
            .unwrap_or_else(|| "unknown".into());

        write(
            &path,
            "ERROR",
            "app.panicked",
            serde_json::json!({
                "message": message,
                "location": location,
                "thread": std::thread::current().name().unwrap_or("unnamed").to_string(),
                "backtrace": std::backtrace::Backtrace::force_capture().to_string(),
            }),
        );

        previous(info);
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_home_directory_never_reaches_the_file() {
        let home = r"C:\Users\someone";
        let mut details = serde_json::json!({
            "audioPath": r"C:\Users\someone\Documents\a.wav",
            "nested": { "list": [r"c:\users\someone\b.txt"] },
            "unrelated": 12,
        });
        redact(&mut details, home, None);

        let rendered = details.to_string();
        assert!(!rendered.to_lowercase().contains("someone"), "{rendered}");
        assert!(rendered.contains("%USERPROFILE%"));
        // The rest of the path survives: the point is to remove the account name, not the
        // information needed to understand what happened.
        assert!(rendered.contains("Documents"));
    }

    /// Windows hands back both casings depending on who built the path.
    #[test]
    fn redaction_is_case_insensitive() {
        let redacted = redact_string(r"c:\users\someone\x", r"C:\Users\someone");
        assert_eq!(redacted, r"%USERPROFILE%\x");
    }

    /// A path can appear more than once in one string, and free text is where it usually does.
    #[test]
    fn every_occurrence_is_replaced() {
        let redacted = redact_string(
            r"copy C:\Users\someone\a to C:\Users\someone\b",
            r"C:\Users\someone",
        );
        assert_eq!(redacted, r"copy %USERPROFILE%\a to %USERPROFILE%\b");
        assert!(!redacted.contains("someone"));
    }

    /// A non-ASCII account name is still redacted.
    ///
    /// Windows usernames are not ASCII-only. If the needle and the haystack are lowered by
    /// different rules the two never match, and the account name reaches the file untouched —
    /// a silent total failure of the one thing this function exists to do.
    #[test]
    fn a_non_ascii_account_name_is_still_redacted() {
        // U+0130 lowers to two code points under full Unicode rules and is unchanged by ASCII
        // lowering, so it is exactly the character that makes the two disagree.
        let home = "C:\\Users\\\u{0130}brahim";

        assert_eq!(
            redact_string("opened C:\\Users\\\u{0130}brahim\\notes.txt", home),
            "opened %USERPROFILE%\\notes.txt"
        );
        // The drive letter still varies in case, which is the reason for lowering at all.
        assert_eq!(
            redact_string("opened c:\\users\\\u{0130}brahim\\notes.txt", home),
            "opened %USERPROFILE%\\notes.txt"
        );
    }

    /// Non-ASCII text must not shift the account name out of alignment.
    ///
    /// Lowercasing is not length-preserving over Unicode: the Kelvin sign is three bytes and
    /// lowers to one. An offset found in a lowered copy is therefore not an offset into the
    /// original, and a path after such a character is cut in the wrong place.
    #[test]
    fn a_length_changing_character_does_not_break_redaction() {
        let home = r"C:\Users\suzuki";

        // Compared whole rather than by `contains`: the leading character here is U+212A, which
        // renders identically to an ASCII K, so a substring check reads as passing when it is
        // matching nothing at all.
        assert_eq!(
            redact_string("note \u{212A}elvin at C:\\Users\\suzuki\\a.wav", home),
            "note \u{212A}elvin at %USERPROFILE%\\a.wav"
        );

        // Immediately before the path is the arrangement that slices mid-character.
        assert_eq!(
            redact_string("\u{212A} C:\\Users\\suzuki\\a.wav", home),
            "\u{212A} %USERPROFILE%\\a.wav"
        );

        // Lengthening as well as shortening: U+0130 lowers to two code points.
        assert_eq!(
            redact_string("\u{0130} C:\\Users\\suzuki\\a.wav", home),
            "\u{0130} %USERPROFILE%\\a.wav"
        );
    }

    #[test]
    fn a_string_without_the_home_directory_is_untouched() {
        let original = r"D:\media\clip.mkv";
        assert_eq!(redact_string(original, r"C:\Users\someone"), original);
    }

    /// The record has to open with the timestamp and the level, which is what makes the file
    /// readable without a tool. `json!` sorted the keys and buried both.
    #[test]
    fn fields_are_written_in_declaration_order() {
        let line = LogLine {
            ts: "2026-08-19T15:04:05.123+05:30".into(),
            level: "ERROR",
            event: "asset.download_failed",
            msg: "Could not reach the download server.".into(),
            run: "18f2c",
            details: serde_json::json!({ "url": "https://example.test" }),
        };

        let encoded = serde_json::to_string(&line).unwrap();

        // The whole line, not a filtered list of keys: filtering yields the source array's
        // order whatever serde emitted, so the comparison could only ever pass.
        assert_eq!(
            encoded,
            r#"{"ts":"2026-08-19T15:04:05.123+05:30","level":"ERROR","event":"asset.download_failed","msg":"Could not reach the download server.","run":"18f2c","details":{"url":"https://example.test"}}"#
        );
    }

    /// A recording's name is a sample of what was said, so it never reaches the file.
    ///
    /// Names taken from the user's real folder: one saved by the app, which carries its id, and
    /// one imported from YouTube, which carries the video's title instead.
    #[test]
    fn a_recording_name_is_replaced_by_its_id() {
        let recordings = r"C:\Users\me\Documents\Wonder of U Recordings";

        let redacted = redact_recording_names(
            &format!(r"{recordings}\と言っている 彼は生_1785155207130.wav"),
            recordings,
        );
        assert_eq!(
            redacted,
            format!(r"{recordings}\1785155207130.wav"),
            "the id is kept and the speech is not"
        );

        // Sidecars carry several extensions and all of them are useful.
        let redacted = redact_recording_names(
            &format!(r"{recordings}\But they'v_1784199535500.ja.segments.json"),
            recordings,
        );
        assert_eq!(redacted, format!(r"{recordings}\1784199535500.ja.segments.json"));
    }

    /// An import is named after its source, which carries no id — so it gets a digest instead
    /// of being left as the video's title.
    #[test]
    fn a_name_without_an_id_becomes_a_stable_digest() {
        let recordings = r"C:\Users\me\Documents\Wonder of U Recordings";
        let path = format!(r"{recordings}\#57 Ani-One Asia [bgWh8DR80m4].mp3");

        let redacted = redact_recording_names(&path, recordings);

        assert!(!redacted.contains("Ani-One"), "the title survived: {redacted}");
        assert!(!redacted.contains("bgWh8DR80m4"), "the video id survived: {redacted}");
        assert!(redacted.ends_with(".mp3"), "the extension is the useful half: {redacted}");
        assert_eq!(
            redacted,
            redact_recording_names(&path, recordings),
            "the same file has to read the same twice, or nothing can be followed through a log"
        );
    }

    /// Only the recordings folder is touched. A model or a binary is named by the app, and
    /// which one it is is exactly what a report needs to say.
    #[test]
    fn paths_outside_the_recordings_folder_keep_their_names() {
        let recordings = r"C:\Users\me\Documents\Wonder of U Recordings";
        let asset = r"C:\Users\me\AppData\Local\com.wonderofu.desktop\assets\models\ggml-small.bin";

        assert_eq!(redact_recording_names(asset, recordings), asset);
    }

    /// Two recordings named in one line — an ffmpeg command names its input and its output.
    #[test]
    fn every_recording_in_a_line_is_replaced() {
        let recordings = r"C:\Users\me\Documents\Wonder of U Recordings";
        let line = format!(r#"converting "{recordings}\話す_111111111111.wav" to "{recordings}\話す_111111111111.mp3""#);

        let redacted = redact_recording_names(&line, recordings);

        assert!(!redacted.contains('話'), "{redacted}");
        assert_eq!(
            redacted,
            format!(r#"converting "{recordings}\111111111111.wav" to "{recordings}\111111111111.mp3""#)
        );
    }

    #[test]
    fn the_message_is_lifted_out_of_details() {
        let mut details = serde_json::json!({ "message": "it failed", "code": 2 });
        assert_eq!(take_message(&mut details), "it failed");
        assert_eq!(details, serde_json::json!({ "code": 2 }));
    }

    #[test]
    fn a_record_without_a_message_still_writes() {
        let mut details = serde_json::json!({ "code": 2 });
        assert_eq!(take_message(&mut details), "");
        assert_eq!(details, serde_json::json!({ "code": 2 }));
    }

    #[test]
    fn the_timestamp_is_rfc_3339_with_milliseconds() {
        let stamp = timestamp();
        assert_eq!(stamp.len(), 29, "{stamp}");
        assert_eq!(&stamp[10..11], "T");
        assert_eq!(&stamp[19..20], ".");
    }

    /// Rotation keeps a bounded number of files and always frees the active name.
    #[test]
    fn rotation_shifts_the_files_along_and_drops_the_oldest() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("app.log");
        let numbered = |index: usize| path.with_extension(format!("{index}.log"));

        fs::write(&path, vec![b'x'; MAX_BYTES as usize]).unwrap();
        fs::write(numbered(1), b"previous").unwrap();
        fs::write(numbered(2), b"oldest").unwrap();

        rotate_if_needed(&path);

        assert!(!path.exists(), "the active file is renamed out of the way");
        assert_eq!(fs::metadata(numbered(1)).unwrap().len(), MAX_BYTES);
        assert_eq!(fs::read(numbered(2)).unwrap(), b"previous");
        assert!(!numbered(3).exists(), "only {KEPT_FILES} files are kept");
    }

    #[test]
    fn a_file_under_the_limit_is_left_alone() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("app.log");
        fs::write(&path, b"small").unwrap();

        rotate_if_needed(&path);

        assert_eq!(fs::read(&path).unwrap(), b"small");
        assert!(!path.with_extension("1.log").exists());
    }

    /// A write that cannot land has to be reported, because the alternative is a user handing
    /// over a file that is silently short.
    ///
    /// Deliberately only asserts that a failure IS recorded, never that none is: the context is
    /// process-wide, so a test asserting the absence would race any other test that writes.
    #[test]
    fn a_write_that_cannot_land_is_reported() {
        let unwritable = Path::new("Z:/no-such-directory/nested/app.log");

        write(unwritable, "INFO", "test.unwritable", serde_json::json!({}));

        let reported = failure().expect("a failed write records why");
        assert!(!reported.trim().is_empty(), "the reason must say something");
    }

    /// Both answers, which the latched version could never show: `debug_enabled` reads the
    /// environment once per process, so a test only ever sees the branch the environment picked.
    #[test]
    fn only_the_debug_setting_asks_for_routine_detail() {
        assert!(debug_requested(Some("debug")));
        assert!(debug_requested(Some("DEBUG")), "the setting is case-insensitive");
        assert!(!debug_requested(Some("info")));
        assert!(!debug_requested(Some("")));
        assert!(!debug_requested(None), "unset means the quiet default");
    }

    /// Routine detail stays out of the file unless it is asked for.
    ///
    /// Asserted against the default environment, which is what a user runs: two events were 44%
    /// of the file before this, and a log that is mostly routine success is one nobody reads.
    #[test]
    fn debug_records_are_dropped_unless_enabled() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("levels.log");

        write(&path, "DEBUG", "test.routine", serde_json::json!({ "message": "noise" }));
        assert!(!path.exists(), "a DEBUG record created the file");

        for level in ["INFO", "WARN", "ERROR"] {
            write(&path, level, "test.kept", serde_json::json!({ "message": level }));
        }
        let kept = fs::read_to_string(&path).unwrap();
        assert_eq!(kept.lines().count(), 3, "{kept}");
        assert!(!kept.contains("noise"));
    }

    /// A real panic, through the real hook, landing in a real file.
    ///
    /// `catch_unwind` stops the test process dying while still running the hook exactly as a
    /// crash would, so this exercises the thing rather than a stand-in for it.
    #[test]
    fn a_panic_is_written_before_the_process_goes() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("panic.log");
        install_panic_hook(path.clone());

        let result = std::panic::catch_unwind(|| panic!("a deliberate test panic"));
        // Put the default hook back before anything else can panic: the hook is process-wide
        // and holds a path inside a directory this test is about to delete.
        let _ = std::panic::take_hook();
        assert!(result.is_err(), "the panic must still propagate");

        let contents = fs::read_to_string(&path).expect("the hook wrote the file");
        let record: Value = serde_json::from_str(contents.lines().next().unwrap()).unwrap();
        assert_eq!(record["level"], "ERROR");
        assert_eq!(record["event"], "app.panicked");
        assert_eq!(record["msg"], "a deliberate test panic");
        let location = record["details"]["location"].as_str().unwrap();
        assert!(location.contains("logging.rs"), "{location}");
        assert!(
            record["details"]["backtrace"].as_str().is_some_and(|t| !t.is_empty()),
            "a panic without a backtrace is half a report"
        );
    }

    /// End to end through the real writer: a line lands, it is valid JSON, and the account name
    /// is not in it.
    #[test]
    fn a_written_line_is_json_and_carries_no_account_name() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("app.log");
        let home = home_directory().unwrap_or_else(|| r"C:\Users\nobody".into());

        write(
            &path,
            "WARN",
            "test.event",
            serde_json::json!({ "message": "x", "path": format!(r"{home}\file.txt") }),
        );

        let contents = fs::read_to_string(&path).unwrap();
        let parsed: Value = serde_json::from_str(contents.trim()).unwrap();
        // The order matters as much as the content: a line that opens with its timestamp and
        // level can be read without a tool, which is the whole reason this is a struct.
        assert!(contents.starts_with(r#"{"ts":"#), "{contents}");
        assert_eq!(parsed["level"], "WARN");
        assert_eq!(parsed["event"], "test.event");
        assert_eq!(parsed["msg"], "x");
        assert_eq!(parsed["details"]["path"], format!(r"{HOME_PLACEHOLDER}\file.txt"));
        assert!(parsed["run"].as_str().is_some_and(|run| !run.is_empty()));
    }
}
