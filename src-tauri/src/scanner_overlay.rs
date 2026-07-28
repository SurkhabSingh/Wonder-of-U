//! The scanner overlay: a transparent window pinned over mpv's video, so a word can be
//! looked up without leaving the player.
//!
//! **Why an overlay at all.** mpv draws subtitles with libass, which exposes no glyph
//! positions — asked directly, an mpv maintainer answered "There's no way to get the
//! position of words in the subtitle", and called fixing it unfeasible. So hovering a word
//! over the video is only possible if we draw the line ourselves, with mpv's own subtitle
//! layer switched off. SubMiner and asbplayer arrived at the same design; Memento avoids it
//! only by embedding libmpv in-process, which is closed to us because mpv is a separate
//! process.
//!
//! **What the spike established** (all measured, none assumed):
//! - mpv owns five top-level windows; exactly one is visible with class `mpv`.
//! - A move is reflected in `GetClientRect` within 0.02 ms, and the call costs ~3 µs, so
//!   polling the rectangle is cheap enough to do at frame rate.
//! - Toggling `WS_EX_TRANSPARENT` routes the pointer to mpv or to the overlay exactly as
//!   needed — this is what makes "hold Shift to scan" both the gesture and the mechanism.
//! - `WS_EX_NOACTIVATE` keeps focus with mpv when the overlay is raised.
//! - The overlay draws **over mpv's fullscreen**, because the app never passes `--ontop`
//!   and mpv's plain `--fs` stays composited by the DWM.
//! - `sub-text` / `sub-start` / `sub-end` keep updating with `sub-visibility=no`, which is
//!   why the overlay renders from mpv's own properties and needs no cue list of its own.

use std::sync::{
    atomic::{AtomicBool, Ordering},
    Mutex,
};
use std::time::Duration;

use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Runtime, WebviewUrl,
    WebviewWindow, WebviewWindowBuilder,
};

use crate::app_types::SharedPersistedState;
#[cfg(target_os = "windows")]
use crate::watch::window::{
    escape_is_held, mpv_is_foreground, video_window_rect, ScanModifier, VideoWindowRect,
};
use crate::watch::watch_session_pid;

pub(crate) const SCANNER_WINDOW_LABEL: &str = "scanner";
/// Geometry + modifier state, pushed to the overlay bundle.
const SCANNER_STATE_EVENT: &str = "scanner-overlay-state";

/// How often the tracker samples mpv's rectangle and the modifier key.
///
/// 16 ms, not the watch panel's 250 ms: this drives whether the pointer belongs to the
/// overlay or to mpv, and a quarter-second of lag there is the difference between "hold
/// Shift and hover" and "hold Shift, wait, then hover". It costs nothing that matters —
/// the sampled calls are ~3 µs and, crucially, **none of them touch mpv's IPC socket or the
/// session mutex**, so the mine hotkey and the watch poll are unaffected.
const TRACK_INTERVAL: Duration = Duration::from_millis(16);

/// Whether the overlay is switched on. Off by default: mpv keeps its own styled `.ass`
/// rendering unless the user asks for the scanner, so nothing that works today changes.
static ENABLED: AtomicBool = AtomicBool::new(false);
/// Set once, so a second enable does not start a second tracker.
static TRACKER_RUNNING: AtomicBool = AtomicBool::new(false);
/// True while a dictionary popup is open.
///
/// The overlay has to stay interactive for as long as there is a popup to read, or the
/// default "leave it open on release" would produce a popup nobody can scroll or click:
/// the moment the modifier came up, the window would go click-through underneath it.
static POPUP_OPEN: AtomicBool = AtomicBool::new(false);
/// Mirrors the window's current click-through state so it is only ever set on a change.
/// Only ever written through [`apply_interactive`] — see the note there.
static INTERACTIVE: AtomicBool = AtomicBool::new(false);

/// The last state pushed, so the tracker only emits on change rather than 60 times a second.
static LAST_STATE: Mutex<Option<ScannerState>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScannerState {
    /// False while mpv has no placeable window — starting up, minimised, or gone.
    pub(crate) tracking: bool,
    /// True while the scan modifier is held, which is also when the overlay stops being
    /// click-through.
    pub(crate) scanning: bool,
    /// True while Escape is down. Polled rather than listened for: a window that never takes
    /// focus never receives a key event, so this is the overlay's only keyboard input.
    pub(crate) escape_pressed: bool,
    pub(crate) width: i32,
    pub(crate) height: i32,
    /// Physical pixels per logical pixel × 100, so the overlay can size text against the
    /// monitor mpv is actually on rather than the primary one.
    pub(crate) dpi: u32,
}

impl ScannerState {
    /// What the overlay is told whenever it goes off screen. Chiefly `tracking: false`,
    /// which is the frontend's cue to drop a popup anchored to a line it can no longer show.
    fn hidden() -> Self {
        Self {
            tracking: false,
            scanning: false,
            escape_pressed: false,
            width: 0,
            height: 0,
            dpi: 96,
        }
    }
}

/// Turns the overlay on or off, switching mpv's own subtitles the other way.
///
/// The two are opposites on purpose: two subtitle layers drawn at once would double every
/// line. Failing to switch mpv is **not** fatal — the user would see both, which is ugly but
/// recoverable, and refusing to open the scanner over it would be worse.
/// Told by the overlay when a popup opens or closes, so the tracker can keep the window
/// taking the mouse for as long as there is something to interact with.
pub(crate) fn set_scanner_popup_open(open: bool) {
    POPUP_OPEN.store(open, Ordering::Relaxed);
}

pub(crate) fn set_scanner_overlay_enabled<R: Runtime>(
    app: &AppHandle<R>,
    enabled: bool,
) -> Result<(), String> {
    ENABLED.store(enabled, Ordering::Relaxed);

    if let Err(error) = crate::watch::set_watch_subtitle_visibility(!enabled) {
        crate::app_runtime::log_event(
            app,
            "WARN",
            "scanner.subtitles",
            serde_json::json!({
                "enabled": enabled,
                "message": format!("Could not switch mpv's own subtitles: {error}"),
            }),
        );
    }

    if enabled {
        start_tracker(app);
    } else if let Some(window) = app.get_webview_window(SCANNER_WINDOW_LABEL) {
        POPUP_OPEN.store(false, Ordering::Relaxed);
        let _ = window.hide();
        // Leave it click-through so a stranded window can never eat a click.
        #[cfg(target_os = "windows")]
        apply_interactive(&window, false);
        if let Ok(mut last) = LAST_STATE.lock() {
            *last = None;
        }
        // Same reason as `hide_overlay`: tell the frontend, or a popup opened before the
        // toggle survives in its state and reappears the next time the overlay is shown.
        let _ = app.emit_to(SCANNER_WINDOW_LABEL, SCANNER_STATE_EVENT, ScannerState::hidden());
    }
    Ok(())
}

/// Builds the overlay window once, hidden and click-through, exactly like the recording
/// indicator. Non-fatal: a failure here costs the scanner, not the app.
pub(crate) fn configure_scanner_overlay<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let window = WebviewWindowBuilder::new(
        app,
        SCANNER_WINDOW_LABEL,
        WebviewUrl::App("scanner.html".into()),
    )
    .transparent(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .shadow(false)
    .focused(false)
    .visible(false)
    .inner_size(640.0, 360.0)
    .build()
    .map_err(|error| format!("Could not create the scanner overlay window: {error}"))?;

    window
        .set_ignore_cursor_events(true)
        .map_err(|error| format!("Could not make the scanner overlay click-through: {error}"))?;

    #[cfg(target_os = "windows")]
    apply_no_activate(&window);

    Ok(())
}

/// Adds `WS_EX_NOACTIVATE` so clicking the popup never pulls focus off mpv — without it,
/// the first click on a definition would silently kill mpv's own space/arrow bindings.
///
/// Tauri exposes `focused(false)` only at build time, which covers creation and nothing
/// after it, so this is set directly on the handle.
#[cfg(target_os = "windows")]
fn apply_no_activate<R: Runtime>(window: &WebviewWindow<R>) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, GWL_EXSTYLE, WS_EX_NOACTIVATE,
    };

    let Ok(handle) = window.hwnd() else {
        return;
    };
    // `WebviewWindow::hwnd` hands back the `windows` crate's newtype while every other Win32
    // call in this app uses `windows-sys`' raw pointer. Same handle, different wrapper.
    let handle = handle.0 as windows_sys::Win32::Foundation::HWND;
    // SAFETY: `handle` is a live top-level window owned by this process for as long as the
    // Tauri window exists, and GWL_EXSTYLE is a valid index for it.
    unsafe {
        let style = GetWindowLongPtrW(handle, GWL_EXSTYLE);
        SetWindowLongPtrW(handle, GWL_EXSTYLE, style | WS_EX_NOACTIVATE as isize);
    }
}

fn configured_modifier<R: Runtime>(app: &AppHandle<R>) -> String {
    app.try_state::<SharedPersistedState>()
        .and_then(|state| {
            state
                .0
                .lock()
                .ok()
                .map(|persisted| persisted.settings.scanner.modifier.clone())
        })
        .unwrap_or_else(|| "shift".to_string())
}

#[cfg(not(target_os = "windows"))]
fn start_tracker<R: Runtime>(_app: &AppHandle<R>) {}

/// One thread, for the app's life, following mpv's window and the modifier key.
///
/// Deliberately not a second mpv poller: it reads the window rectangle and the keyboard
/// through Win32 only. The subtitle text the overlay draws rides the watch page's existing
/// 250 ms snapshot, so watching costs mpv exactly what it costs today.
#[cfg(target_os = "windows")]
fn start_tracker<R: Runtime>(app: &AppHandle<R>) {
    if TRACKER_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || loop {
        std::thread::sleep(TRACK_INTERVAL);
        if !ENABLED.load(Ordering::Relaxed) {
            continue;
        }
        let Some(pid) = watch_session_pid() else {
            hide_overlay(&app);
            continue;
        };
        // Only draw while mpv is the window in front. The overlay is always-on-top and
        // follows mpv's rectangle, so without this it would keep painting a subtitle line
        // over whatever the user alt-tabbed to.
        if !mpv_is_foreground(pid) {
            hide_overlay(&app);
            continue;
        }
        let Some(rect) = video_window_rect(pid) else {
            hide_overlay(&app);
            continue;
        };
        let scanning = ScanModifier::from_setting(&configured_modifier(&app)).is_held();
        track_once(&app, rect, scanning, escape_is_held());
    });
}

/// Sets click-through and records it in the same breath.
///
/// `INTERACTIVE` is a mirror of the window's real state and is only consulted to avoid
/// redundant calls, which makes any path that changes the window WITHOUT updating the
/// mirror silently poisonous: the two drift, the next comparison sees "no change", and the
/// window is left in whatever state the drift produced. That is not hypothetical — hiding
/// the overlay used to make it click-through directly, so alt-tabbing away and back left
/// the mirror claiming "interactive" while the window ignored the mouse, and the popup's
/// close button stopped working. Every caller goes through here now.
#[cfg(target_os = "windows")]
fn apply_interactive<R: Runtime>(window: &WebviewWindow<R>, interactive: bool) -> bool {
    if INTERACTIVE.swap(interactive, Ordering::Relaxed) == interactive {
        return false;
    }
    let _ = window.set_ignore_cursor_events(!interactive);
    true
}

/// Takes the overlay off screen — mpv is not in front, has no placeable window, or is gone.
///
/// Also tells the frontend, which is what lets it drop a popup that is anchored to a line
/// the user can no longer see. Without that the popup survives the round trip and comes
/// back attached to stale text.
#[cfg(target_os = "windows")]
fn hide_overlay<R: Runtime>(app: &AppHandle<R>) {
    let mut last = match LAST_STATE.lock() {
        Ok(last) => last,
        Err(_) => return,
    };
    if last.is_none() {
        return;
    }
    *last = None;
    drop(last);
    if let Some(window) = app.get_webview_window(SCANNER_WINDOW_LABEL) {
        let _ = window.hide();
        apply_interactive(&window, false);
        let _ = app.emit_to(
            SCANNER_WINDOW_LABEL,
            SCANNER_STATE_EVENT,
            ScannerState::hidden(),
        );
    }
}

#[cfg(target_os = "windows")]
fn track_once<R: Runtime>(
    app: &AppHandle<R>,
    rect: VideoWindowRect,
    scanning: bool,
    escape_pressed: bool,
) {
    let state = ScannerState {
        tracking: true,
        scanning,
        escape_pressed,
        width: rect.width,
        height: rect.height,
        dpi: rect.dpi,
    };

    let Some(window) = app.get_webview_window(SCANNER_WINDOW_LABEL) else {
        return;
    };

    let geometry_changed = {
        let Ok(last) = LAST_STATE.lock() else {
            return;
        };
        match last.as_ref() {
            None => true,
            Some(previous) => {
                previous.width != state.width
                    || previous.height != state.height
                    || previous.dpi != state.dpi
            }
        }
    };

    // Position every tick regardless: the rectangle's ORIGIN is not part of the compared
    // state, so a window dragged without resizing still has to be followed.
    let _ = window.set_position(PhysicalPosition::new(rect.left, rect.top));
    if geometry_changed {
        let _ = window.set_size(PhysicalSize::new(rect.width, rect.height));
        let _ = window.show();
    }

    // Interactive while the modifier is down OR while a popup is open. The first is what
    // lets a word be hovered; the second is what lets the resulting entry be read.
    let interactive = scanning || POPUP_OPEN.load(Ordering::Relaxed);
    let scanning_changed = {
        let Ok(last) = LAST_STATE.lock() else {
            return;
        };
        last.as_ref().map(|previous| previous.scanning) != Some(scanning)
    };
    let escape_changed = {
        let Ok(last) = LAST_STATE.lock() else {
            return;
        };
        last.as_ref().map(|previous| previous.escape_pressed) != Some(escape_pressed)
    };
    // The whole interaction, in one call: while it is on the overlay takes the pointer;
    // the instant it goes off mpv gets every click back, including its OSC controls.
    let interactive_changed = apply_interactive(&window, interactive);

    let changed = geometry_changed || scanning_changed || interactive_changed || escape_changed;
    if let Ok(mut last) = LAST_STATE.lock() {
        *last = Some(state);
    }
    if changed {
        // Emitted only on change — 60 unchanged events a second would wake the webview for
        // nothing. Never emitted while a lock is held: `update_shell_snapshot` deadlocks
        // that way, as the recording indicator's note warns.
        let _ = app.emit_to(SCANNER_WINDOW_LABEL, SCANNER_STATE_EVENT, state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_overlay_starts_disabled() {
        // mpv's own styled subtitles are the default; ours replace them only when asked.
        assert!(!ENABLED.load(Ordering::Relaxed));
    }

    #[test]
    fn state_serializes_as_camel_case_for_the_overlay_bundle() {
        let json = serde_json::to_string(&ScannerState {
            tracking: true,
            scanning: false,
            escape_pressed: false,
            width: 1920,
            height: 1080,
            dpi: 144,
        })
        .expect("serializable");
        assert!(json.contains("\"tracking\":true"));
        assert!(json.contains("\"scanning\":false"));
        assert!(json.contains("\"dpi\":144"));
    }
}
