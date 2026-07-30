//! Watch & Mine: drive an external mpv and read what it is showing.
//!
//! The app does not render video itself. WebView2 plays H.264/AAC MP4 and VP9/Opus WebM
//! and nothing else — not MKV, H.265, AC3 or 10-bit, which is most of what anyone would
//! actually want to mine from. mpv plays all of it, renders `.srt`/`.ass` natively, and
//! exposes its state over a JSON IPC socket, so the app reads the player rather than
//! being one.
//!
//! Verified against mpv v0.41.0 before this was written (see the Slice 0 spike):
//! round-trip on a Windows named pipe is 0.3–0.5ms while playing, and `sub-text` /
//! `sub-start` / `sub-end` report the on-screen line and its bounds to the millisecond.
//! That last part is why there is no subtitle parser here — mpv has already done the
//! parsing, the timing and the sync, including embedded tracks and odd encodings.

use std::{
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicU32, Ordering},
        Mutex,
    },
    time::{Duration, Instant},
};

#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;

use serde::Serialize;

pub(crate) mod subtitles;
pub(crate) mod transcribe;
pub(crate) mod sync;
#[cfg(target_os = "windows")]
pub(crate) mod window;

#[cfg(target_os = "windows")]
const CREATE_NO_WINDOW: u32 = 0x08000000;

/// How long to wait for mpv to create its IPC endpoint after launch. The spike connected
/// on the first attempt, but the pipe genuinely does not exist for a few milliseconds
/// after spawn, so this retries rather than racing.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const CONNECT_RETRY_INTERVAL: Duration = Duration::from_millis(50);

/// Ceiling on a single request. Reads are sub-millisecond in practice; this only exists
/// so a wedged player cannot hang a command thread forever.
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);

/// The IPC endpoint. A Windows named pipe, a filesystem socket elsewhere. The process id
/// is in the name so two app instances never fight over one endpoint.
fn ipc_endpoint() -> String {
    #[cfg(target_os = "windows")]
    {
        format!(r"\\.\pipe\wonder-of-u-mpv-{}", std::process::id())
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::temp_dir()
            .join(format!("wonder-of-u-mpv-{}.sock", std::process::id()))
            .display()
            .to_string()
    }
}

/// What mpv is currently showing. Everything is optional because mpv answers `null` for
/// a property that has no value right now — no file loaded, or no subtitle on screen —
/// and that is a normal state, not a failure.
#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct WatchSnapshot {
    /// True while an mpv process is running and answering.
    pub(crate) connected: bool,
    pub(crate) path: Option<String>,
    pub(crate) title: Option<String>,
    pub(crate) position_ms: Option<u64>,
    pub(crate) duration_ms: Option<u64>,
    pub(crate) paused: bool,
    /// The subtitle line on screen right now, and its exact bounds. These three are what
    /// mining reads: no parsing, no sync, no guessing which cue the user meant.
    pub(crate) subtitle_text: Option<String>,
    pub(crate) subtitle_start_ms: Option<u64>,
    pub(crate) subtitle_end_ms: Option<u64>,
    /// mpv's own subtitle offset, in milliseconds. Settable, so nudging out-of-sync subs
    /// costs one IPC call rather than a reimplementation.
    pub(crate) subtitle_delay_ms: i64,
}

/// Seconds (mpv's unit) to whole milliseconds (ours). Negative times are clamped to zero:
/// mpv can report a slightly negative `time-pos` while a file is still loading, and a
/// negative position is meaningless to every caller here.
fn seconds_to_ms(seconds: f64) -> u64 {
    if seconds.is_finite() && seconds > 0.0 {
        (seconds * 1000.0).round() as u64
    } else {
        0
    }
}

/// Signed variant, for `sub-delay`, where a negative value is meaningful (subtitles
/// ahead of the audio rather than behind).
fn seconds_to_signed_ms(seconds: f64) -> i64 {
    if seconds.is_finite() {
        (seconds * 1000.0).round() as i64
    } else {
        0
    }
}

/// A live mpv process and the connection to it.
struct MpvSession {
    child: Child,
    connection: MpvConnection,
}

/// The single running session, if any. Watch & Mine is deliberately one-player-at-a-time:
/// "mine the line I am hearing" has no meaning with two videos open.
static SESSION: Mutex<Option<MpvSession>> = Mutex::new(None);

/// mpv's process id, mirrored outside the session mutex.
///
/// The scanner overlay's tracker asks for this 60 times a second to find mpv's window, and
/// it must never block: it owns click-through, so a stall there leaves the overlay taking
/// the mouse when it should not — or refusing it when it should. Reading it through
/// `SESSION` would do exactly that, because `watch_snapshot` holds that lock across NINE
/// blocking IPC round trips, each able to wait out `REQUEST_TIMEOUT` if mpv stops
/// answering. Zero means no session.
static SESSION_PID: AtomicU32 = AtomicU32::new(0);

/// Kept in lockstep with `SESSION` — every place that assigns the session assigns this.
fn set_session_pid(pid: Option<u32>) {
    SESSION_PID.store(pid.unwrap_or(0), Ordering::Relaxed);
}

impl MpvSession {
    /// Kills mpv and drops the connection. Called on stop, and on any launch so a stale
    /// session can never linger behind a new one.
    fn shut_down(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[cfg(target_os = "windows")]
mod transport {
    use super::*;
    use std::fs::OpenOptions;

    /// On Windows the IPC endpoint is a named pipe, which opens as an ordinary file.
    pub(super) struct Pipe {
        reader: BufReader<std::fs::File>,
        writer: std::fs::File,
    }

    impl Pipe {
        pub(super) fn connect(endpoint: &str) -> Result<Self, String> {
            let writer = OpenOptions::new()
                .read(true)
                .write(true)
                .open(endpoint)
                .map_err(|error| error.to_string())?;
            let reader = writer.try_clone().map_err(|error| error.to_string())?;
            Ok(Self {
                reader: BufReader::new(reader),
                writer,
            })
        }
    }

    impl super::Transport for Pipe {
        fn write_line(&mut self, line: &str) -> Result<(), String> {
            self.writer
                .write_all(line.as_bytes())
                .and_then(|_| self.writer.write_all(b"\n"))
                .and_then(|_| self.writer.flush())
                .map_err(|error| error.to_string())
        }

        fn read_line(&mut self, buffer: &mut String) -> Result<usize, String> {
            self.reader
                .read_line(buffer)
                .map_err(|error| error.to_string())
        }
    }
}

#[cfg(not(target_os = "windows"))]
mod transport {
    use super::*;
    use std::os::unix::net::UnixStream;

    pub(super) struct Pipe {
        reader: BufReader<UnixStream>,
        writer: UnixStream,
    }

    impl Pipe {
        pub(super) fn connect(endpoint: &str) -> Result<Self, String> {
            let writer = UnixStream::connect(endpoint).map_err(|error| error.to_string())?;
            let reader = writer.try_clone().map_err(|error| error.to_string())?;
            Ok(Self {
                reader: BufReader::new(reader),
                writer,
            })
        }
    }

    impl super::Transport for Pipe {
        fn write_line(&mut self, line: &str) -> Result<(), String> {
            self.writer
                .write_all(line.as_bytes())
                .and_then(|_| self.writer.write_all(b"\n"))
                .and_then(|_| self.writer.flush())
                .map_err(|error| error.to_string())
        }

        fn read_line(&mut self, buffer: &mut String) -> Result<usize, String> {
            self.reader
                .read_line(buffer)
                .map_err(|error| error.to_string())
        }
    }
}

/// The platform-specific half of the connection, kept behind a trait so the request loop
/// below is written once.
trait Transport: Send {
    fn write_line(&mut self, line: &str) -> Result<(), String>;
    fn read_line(&mut self, buffer: &mut String) -> Result<usize, String>;
}

struct MpvConnection {
    transport: Box<dyn Transport>,
    /// Monotonic id so a reply can be matched to its request. mpv echoes `request_id`
    /// back, which is what makes this safe to use while mpv is also emitting events.
    next_request_id: u64,
}

impl MpvConnection {
    fn connect(endpoint: &str) -> Result<Self, String> {
        let deadline = Instant::now() + CONNECT_TIMEOUT;
        loop {
            match transport::Pipe::connect(endpoint) {
                Ok(pipe) => {
                    return Ok(Self {
                        transport: Box::new(pipe),
                        next_request_id: 1,
                    })
                }
                Err(error) => {
                    if Instant::now() >= deadline {
                        return Err(format!(
                            "Could not reach mpv over its control channel: {error}"
                        ));
                    }
                    std::thread::sleep(CONNECT_RETRY_INTERVAL);
                }
            }
        }
    }

    /// Sends one command and returns its `data`.
    ///
    /// mpv interleaves asynchronous events with replies on the same channel, so lines are
    /// read until one carries OUR `request_id`. Matching on the id rather than "the next
    /// line" is what keeps a property read from accidentally consuming an event — the
    /// bug this design would otherwise have.
    fn request(&mut self, command: &[&str]) -> Result<serde_json::Value, String> {
        // Every existing caller sends strings, and mpv coerces them, so the string form
        // stays the front door and simply widens into the JSON one. Keeping it means the
        // verified mine path is not touched by adding a setter.
        let command = command
            .iter()
            .map(|part| serde_json::Value::String((*part).to_string()))
            .collect::<Vec<_>>();
        self.request_json(&command)
    }

    /// The same exchange, but with arbitrary JSON arguments — `set_property` needs a real
    /// boolean for `sub-visibility`, not the string "no".
    fn request_json(&mut self, command: &[serde_json::Value]) -> Result<serde_json::Value, String> {
        let request_id = self.next_request_id;
        self.next_request_id = self.next_request_id.wrapping_add(1).max(1);

        let payload = serde_json::json!({ "command": command, "request_id": request_id });
        self.transport.write_line(&payload.to_string())?;

        let deadline = Instant::now() + REQUEST_TIMEOUT;
        loop {
            if Instant::now() >= deadline {
                return Err("mpv did not answer in time.".into());
            }
            let mut line = String::new();
            let read = self.transport.read_line(&mut line)?;
            if read == 0 {
                return Err("mpv closed its control channel.".into());
            }
            let Ok(message) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
                continue;
            };
            if message.get("request_id").and_then(serde_json::Value::as_u64) != Some(request_id) {
                // An event, or a reply to something else. Not ours.
                continue;
            }
            let error = message
                .get("error")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("success");
            if error != "success" {
                // A property that has no value right now reports `property unavailable`.
                // That is a normal state (nothing loaded, no subtitle on screen), so it
                // comes back as null rather than as an error.
                if error == "property unavailable" {
                    return Ok(serde_json::Value::Null);
                }
                return Err(format!("mpv rejected the request: {error}"));
            }
            return Ok(message
                .get("data")
                .cloned()
                .unwrap_or(serde_json::Value::Null));
        }
    }

    fn property(&mut self, name: &str) -> Result<serde_json::Value, String> {
        self.request(&["get_property", name])
    }

    fn set_property(
        &mut self,
        name: &str,
        value: serde_json::Value,
    ) -> Result<serde_json::Value, String> {
        self.request_json(&[
            serde_json::Value::String("set_property".into()),
            serde_json::Value::String(name.into()),
            value,
        ])
    }

    fn optional_seconds_ms(&mut self, name: &str) -> Option<u64> {
        self.property(name)
            .ok()
            .and_then(|value| value.as_f64())
            .map(seconds_to_ms)
    }

    fn optional_string(&mut self, name: &str) -> Option<String> {
        self.property(name).ok().and_then(|value| {
            value
                .as_str()
                .map(str::to_string)
                .filter(|text| !text.trim().is_empty())
        })
    }
}

/// Starts mpv on `video_path`, replacing any session already running.
///
/// `--no-config` is NOT passed: someone who has tuned their mpv should get their own
/// player, and the spike confirmed the IPC properties are unaffected by user config.
pub(crate) fn start_watch_session(
    mpv_path: &Path,
    video_path: &Path,
    subtitle_path: Option<&Path>,
) -> Result<(), String> {
    if !video_path.exists() {
        return Err(format!("There is no file at {}.", video_path.display()));
    }

    let mut session_guard = SESSION
        .lock()
        .map_err(|_| "Could not reach the watch session.".to_string())?;
    if let Some(existing) = session_guard.as_mut() {
        existing.shut_down();
    }
    set_session_pid(None);
    *session_guard = None;

    let endpoint = ipc_endpoint();
    #[cfg(not(target_os = "windows"))]
    let _ = std::fs::remove_file(&endpoint);

    let mut command = Command::new(mpv_path);
    #[cfg(target_os = "windows")]
    command.creation_flags(CREATE_NO_WINDOW);
    command
        .arg(format!("--input-ipc-server={endpoint}"))
        // Keep the window open at end of file rather than exiting, so the session does
        // not vanish underneath the app the moment the episode finishes.
        .arg("--keep-open=yes")
        // mpv's own terminal output is noise here; the app is the interface.
        .arg("--no-terminal")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());

    if let Some(subtitle_path) = subtitle_path {
        if subtitle_path.exists() {
            command.arg(format!("--sub-file={}", subtitle_path.display()));
        }
    }
    command.arg(video_path);

    let child = command.spawn().map_err(|error| {
        if error.kind() == std::io::ErrorKind::NotFound {
            "mpv could not be started; install it in Setup.".to_string()
        } else {
            format!("mpv could not be started: {error}")
        }
    })?;

    let connection = match MpvConnection::connect(&endpoint) {
        Ok(connection) => connection,
        Err(error) => {
            let mut child = child;
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };

    set_session_pid(Some(child.id()));
    *session_guard = Some(MpvSession { child, connection });
    Ok(())
}

/// Reads everything the watch page and the mine action need, in one pass.
///
/// A dead player is reported as a disconnected snapshot rather than an error: the user
/// closing the mpv window is an ordinary way to end a session, not a failure.
pub(crate) fn watch_snapshot() -> Result<WatchSnapshot, String> {
    let mut session_guard = SESSION
        .lock()
        .map_err(|_| "Could not reach the watch session.".to_string())?;
    let Some(session) = session_guard.as_mut() else {
        return Ok(WatchSnapshot::default());
    };

    // The user closing mpv is the normal way to stop watching.
    if matches!(session.child.try_wait(), Ok(Some(_))) {
        set_session_pid(None);
        *session_guard = None;
        return Ok(WatchSnapshot::default());
    }

    let connection = &mut session.connection;
    let snapshot = WatchSnapshot {
        connected: true,
        path: connection.optional_string("path"),
        title: connection.optional_string("media-title"),
        position_ms: connection.optional_seconds_ms("time-pos"),
        duration_ms: connection.optional_seconds_ms("duration"),
        paused: connection
            .property("pause")
            .ok()
            .and_then(|value| value.as_bool())
            .unwrap_or(false),
        subtitle_text: connection.optional_string("sub-text"),
        subtitle_start_ms: connection.optional_seconds_ms("sub-start"),
        subtitle_end_ms: connection.optional_seconds_ms("sub-end"),
        subtitle_delay_ms: connection
            .property("sub-delay")
            .ok()
            .and_then(|value| value.as_f64())
            .map(seconds_to_signed_ms)
            .unwrap_or(0),
    };

    // A snapshot that answered nothing at all means the channel died even though the
    // process is alive; drop the session so the UI stops claiming to be connected.
    if snapshot.path.is_none() && snapshot.position_ms.is_none() && snapshot.duration_ms.is_none() {
        session.shut_down();
        set_session_pid(None);
        *session_guard = None;
        return Ok(WatchSnapshot::default());
    }

    Ok(snapshot)
}

/// Jumps the player to `position_ms`.
///
/// Clicking a line in the subtitle list is the whole point of having the list, and it is
/// the one thing the read-only snapshot cannot do.
pub(crate) fn seek_watch_session(position_ms: u64) -> Result<(), String> {
    let mut session_guard = SESSION
        .lock()
        .map_err(|_| "Could not reach the watch session.".to_string())?;
    let Some(session) = session_guard.as_mut() else {
        return Err("No video is playing.".into());
    };
    let seconds = format!("{}.{:03}", position_ms / 1000, position_ms % 1000);
    session
        .connection
        .request(&["seek", &seconds, "absolute"])
        .map(|_| ())
}

/// Hands mpv's own subtitle rendering on or off.
///
/// Off is what makes the scanner overlay possible: mpv draws subtitles with libass, which
/// exposes no glyph positions, so a word can only be hovered if we draw the line ourselves.
/// Verified in the spike: `sub-text`, `sub-start` and `sub-end` keep reporting normally
/// while visibility is off, so hiding mpv's layer costs us nothing but the `.ass` styling.
pub(crate) fn set_watch_subtitle_visibility(visible: bool) -> Result<(), String> {
    let mut session_guard = SESSION
        .lock()
        .map_err(|_| "Could not reach the watch session.".to_string())?;
    let Some(session) = session_guard.as_mut() else {
        return Err("No video is playing.".into());
    };
    session
        .connection
        .set_property("sub-visibility", serde_json::Value::Bool(visible))
        .map(|_| ())
}

/// Shifts the subtitles against the audio by `delay_ms`, positive meaning "show later".
///
/// This is mpv's own `sub-delay`, so it costs one property write and applies to whatever
/// mpv is rendering — sidecar or embedded track — with no file rewritten. It is the right
/// tool for the common failure, where a subtitle file is off by a constant: alass exists
/// for the harder case where the drift varies across the episode.
///
/// mpv measures the property in **seconds**, so the millisecond figure the UI works in is
/// converted here rather than at the call site.
pub(crate) fn set_watch_subtitle_delay(delay_ms: i64) -> Result<(), String> {
    let mut session_guard = SESSION
        .lock()
        .map_err(|_| "Could not reach the watch session.".to_string())?;
    let Some(session) = session_guard.as_mut() else {
        return Err("No video is playing.".into());
    };
    let seconds = delay_ms as f64 / 1000.0;
    let Some(seconds) = serde_json::Number::from_f64(seconds) else {
        return Err("That subtitle offset is not a usable number.".into());
    };
    session
        .connection
        .set_property("sub-delay", serde_json::Value::Number(seconds))
        .map(|_| ())
}

/// Hands mpv a subtitle file and selects it, replacing whatever it was showing.
///
/// Needed because syncing writes a NEW file: without this, alass would produce a corrected
/// subtitle that the player never loads, and the user would watch the old one while the app
/// claimed success. `sub-add` with `select` both adds and switches in one command.
pub(crate) fn add_watch_subtitle_file(subtitle_path: &str) -> Result<(), String> {
    let mut session_guard = SESSION
        .lock()
        .map_err(|_| "Could not reach the watch session.".to_string())?;
    let Some(session) = session_guard.as_mut() else {
        return Err("No video is playing.".into());
    };
    session
        .connection
        .request(&["sub-add", subtitle_path, "select"])
        .map(|_| ())
}

/// mpv's process id, for finding its window.
///
/// Returned rather than the window handle itself because the handle is not stable — mpv
/// owns several top-level windows and the video one is identified by class and visibility
/// at the moment it is asked for, not once at launch.
///
/// Lock-free by design; see [`SESSION_PID`].
pub(crate) fn watch_session_pid() -> Option<u32> {
    match SESSION_PID.load(Ordering::Relaxed) {
        0 => None,
        pid => Some(pid),
    }
}

pub(crate) fn stop_watch_session() -> Result<(), String> {
    let mut session_guard = SESSION
        .lock()
        .map_err(|_| "Could not reach the watch session.".to_string())?;
    if let Some(session) = session_guard.as_mut() {
        session.shut_down();
    }
    set_session_pid(None);
    *session_guard = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex as StdMutex};

    /// A stand-in for the named pipe: records what was written and replays canned replies.
    struct FakeTransport {
        written: Arc<StdMutex<Vec<String>>>,
        replies: Vec<String>,
    }

    impl Transport for FakeTransport {
        fn write_line(&mut self, line: &str) -> Result<(), String> {
            self.written.lock().unwrap().push(line.to_string());
            Ok(())
        }

        fn read_line(&mut self, buffer: &mut String) -> Result<usize, String> {
            if self.replies.is_empty() {
                return Ok(0);
            }
            let reply = self.replies.remove(0);
            buffer.push_str(&reply);
            buffer.push('\n');
            Ok(buffer.len())
        }
    }

    fn connection_with(replies: &[&str]) -> (MpvConnection, Arc<StdMutex<Vec<String>>>) {
        let written = Arc::new(StdMutex::new(Vec::new()));
        let transport = FakeTransport {
            written: Arc::clone(&written),
            replies: replies.iter().map(|reply| (*reply).to_string()).collect(),
        };
        (
            MpvConnection {
                transport: Box::new(transport),
                next_request_id: 1,
            },
            written,
        )
    }

    /// `request` was rewritten to delegate to `request_json` so a property could be SET.
    /// Every existing caller — the mine hotkey, seek, all nine snapshot reads — goes through
    /// it, so the bytes mpv receives must be exactly what they were before.
    #[test]
    fn string_commands_are_sent_unchanged_after_the_json_split() {
        let (mut connection, written) =
            connection_with(&[r#"{"request_id":1,"error":"success","data":12.5}"#]);
        let value = connection.request(&["get_property", "time-pos"]).unwrap();

        assert_eq!(value.as_f64(), Some(12.5));
        let sent = written.lock().unwrap();
        assert_eq!(
            sent[0],
            r#"{"command":["get_property","time-pos"],"request_id":1}"#
        );
    }

    #[test]
    fn seek_still_sends_three_string_arguments() {
        let (mut connection, written) =
            connection_with(&[r#"{"request_id":1,"error":"success"}"#]);
        connection.request(&["seek", "22.400", "absolute"]).unwrap();
        assert_eq!(
            written.lock().unwrap()[0],
            r#"{"command":["seek","22.400","absolute"],"request_id":1}"#
        );
    }

    /// The reason the JSON path exists: `sub-visibility` needs a real boolean, and sending
    /// the string "false" would set the property to a truthy value instead of unsetting it.
    #[test]
    fn set_property_sends_a_real_boolean_not_a_string() {
        let (mut connection, written) =
            connection_with(&[r#"{"request_id":1,"error":"success"}"#]);
        connection
            .set_property("sub-visibility", serde_json::Value::Bool(false))
            .unwrap();
        assert_eq!(
            written.lock().unwrap()[0],
            r#"{"command":["set_property","sub-visibility",false],"request_id":1}"#
        );
    }

    /// Events interleave with replies on the same channel; matching on `request_id` is what
    /// keeps a property read from consuming one. Unchanged by the split, and worth pinning.
    #[test]
    fn an_interleaved_event_is_skipped_rather_than_returned() {
        let (mut connection, _written) = connection_with(&[
            r#"{"event":"playback-restart"}"#,
            r#"{"request_id":1,"error":"success","data":"hello"}"#,
        ]);
        let value = connection.request(&["get_property", "sub-text"]).unwrap();
        assert_eq!(value.as_str(), Some("hello"));
    }

    #[test]
    fn seconds_convert_to_whole_milliseconds() {
        assert_eq!(seconds_to_ms(22.4), 22_400);
        assert_eq!(seconds_to_ms(0.0005), 1);
        assert_eq!(seconds_to_ms(60.0), 60_000);
    }

    #[test]
    fn a_loading_players_negative_or_absent_time_reads_as_zero() {
        // mpv can report a slightly negative time-pos while a file is still loading, and
        // a negative position is meaningless to every caller.
        assert_eq!(seconds_to_ms(-0.5), 0);
        assert_eq!(seconds_to_ms(f64::NAN), 0);
        assert_eq!(seconds_to_ms(f64::INFINITY), 0);
    }

    #[test]
    fn subtitle_delay_keeps_its_sign() {
        // Unlike a position, a negative delay is meaningful: subtitles ahead of the audio.
        assert_eq!(seconds_to_signed_ms(-1.25), -1250);
        assert_eq!(seconds_to_signed_ms(0.2), 200);
        assert_eq!(seconds_to_signed_ms(f64::NAN), 0);
    }

    #[test]
    fn the_ipc_endpoint_is_unique_per_process() {
        // Two app instances must not fight over one control channel.
        let endpoint = ipc_endpoint();
        assert!(endpoint.contains(&std::process::id().to_string()));
    }
}
