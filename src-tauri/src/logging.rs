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

struct LogContext {
    /// Identifies one launch. Every line carries it, so a whole run can be isolated from a file
    /// that holds many.
    run: String,
    /// Why logging last failed, if it has. Read by the bootstrap so the app can say so instead
    /// of handing over a file that is silently short.
    failure: Mutex<Option<String>>,
}

static CONTEXT: OnceLock<LogContext> = OnceLock::new();

fn context() -> &'static LogContext {
    CONTEXT.get_or_init(|| LogContext {
        run: format!("{:x}", now_ms()),
        failure: Mutex::new(None),
    })
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

/// This launch's id, for the line that reports what the app is running on.
pub(crate) fn run_id() -> &'static str {
    &context().run
}

/// The reason logging is not working, if it is not.
pub(crate) fn failure() -> Option<String> {
    context()
        .failure
        .lock()
        .ok()
        .and_then(|failure| failure.clone())
}

fn record_failure(reason: String) {
    // Printed as well as stored: during development the terminal is where it will be seen, and
    // a logger that cannot report its own failure is the one component that has nowhere else
    // to go.
    eprintln!("wonder-of-u: log write failed: {reason}");
    if let Ok(mut failure) = context().failure.lock() {
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
/// is too.
fn redact_string(text: &str, home: &str) -> String {
    let haystack = text.to_lowercase();
    let needle = home.to_lowercase();
    if !haystack.contains(&needle) {
        return text.to_string();
    }

    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(index) = rest.to_lowercase().find(&needle) {
        out.push_str(&rest[..index]);
        out.push_str(HOME_PLACEHOLDER);
        rest = &rest[index + home.len()..];
    }
    out.push_str(rest);
    out
}

/// Rewrites the home directory out of every string anywhere in the payload.
///
/// Walks the whole value rather than naming fields, so a field added later is covered without
/// anyone remembering to add it here — 84% of lines carried the account name before this, and
/// they carried it in twenty-seven differently named fields plus free text.
fn redact(value: &mut Value, home: &str) {
    match value {
        Value::String(text) => *text = redact_string(text, home),
        Value::Array(items) => items.iter_mut().for_each(|item| redact(item, home)),
        Value::Object(fields) => fields.values_mut().for_each(|field| redact(field, home)),
        _ => {}
    }
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
    *ENABLED.get_or_init(|| {
        std::env::var("WONDER_OF_U_LOG")
            .map(|value| value.eq_ignore_ascii_case("debug"))
            .unwrap_or(false)
    })
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
        line.msg = redact_string(&line.msg, &home);
        redact(&mut line.details, &home);
    }

    let Ok(encoded) = serde_json::to_string(&line) else {
        record_failure(format!("event {event} could not be encoded"));
        return;
    };

    rotate_if_needed(path);

    let opened = OpenOptions::new().create(true).append(true).open(path);
    match opened {
        Ok(mut file) => {
            if let Err(error) = writeln!(file, "{encoded}") {
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
        redact(&mut details, home);

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
        let order: Vec<&str> = ["ts", "level", "event", "msg", "run", "details"]
            .into_iter()
            .filter(|key| encoded.contains(&format!("\"{key}\":")))
            .collect();
        assert_eq!(order, ["ts", "level", "event", "msg", "run", "details"]);
        assert!(encoded.starts_with(r#"{"ts":"#), "{encoded}");
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
