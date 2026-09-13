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
    escape_is_held, foreground_window, video_window_rect, window_process_id, ScanModifier,
    VideoWindowRect,
};
use crate::watch::watch_session_pid;

pub(crate) const SCANNER_WINDOW_LABEL: &str = "scanner";
/// Geometry + modifier state, pushed to the overlay bundle.
const SCANNER_STATE_EVENT: &str = "scanner-overlay-state";

/// How often the tracker samples mpv's rectangle and the modifier key.
const TRACK_INTERVAL: Duration = Duration::from_millis(16);

/// How long the tracker sleeps while the overlay is switched off. 
const IDLE_INTERVAL: Duration = Duration::from_millis(250);

/// How often the configured modifier is re-read from settings, in ticks.
const MODIFIER_REFRESH_TICKS: u32 = 60;

/// Whether the overlay is switched on. Off by default: mpv keeps its own styled `.ass`
/// rendering unless the user asks for the scanner, so nothing that works today changes.
static ENABLED: AtomicBool = AtomicBool::new(false);
/// Set once, so a second enable does not start a second tracker.
static TRACKER_RUNNING: AtomicBool = AtomicBool::new(false);
/// True while a dictionary popup is open.
static POPUP_OPEN: AtomicBool = AtomicBool::new(false);
/// Sampled by the tracker, because a window that never takes focus never sees a key event.
static MODIFIER_HELD: AtomicBool = AtomicBool::new(false);
static ESCAPE_HELD: AtomicBool = AtomicBool::new(false);

static APPLIED: Mutex<Applied> = Mutex::new(Applied::new());

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ScannerState {
    pub(crate) tracking: bool,
    pub(crate) scanning: bool,
    pub(crate) escape_pressed: bool,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) dpi: u32,
}

impl ScannerState {
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

/// Everything the overlay's appearance is derived from — inputs only. Nothing here records
/// what has already been applied.
#[derive(Clone, Copy)]
struct Inputs {
    enabled: bool,
    popup_open: bool,
    modifier_held: bool,
    escape_held: bool,
    placement: Option<VideoWindowRect>,
}

/// What those inputs mean for the window. Derived on demand, never stored as a source of
/// truth.
#[derive(Clone, Copy, PartialEq, Eq)]
struct Desired {
    visible: bool,
    interactive: bool,
}

impl Inputs {
    fn desired(self) -> Desired {
        let visible = self.enabled && self.placement.is_some();
        Desired {
            visible,
            interactive: visible && (self.modifier_held || self.popup_open),
        }
    }

    /// What the overlay bundle needs.
    fn frontend_state(self) -> ScannerState {
        match self.placement {
            Some(rect) if self.enabled => ScannerState {
                tracking: true,
                scanning: self.modifier_held,
                escape_pressed: self.escape_held,
                width: rect.width,
                height: rect.height,
                dpi: rect.dpi,
            },
            _ => ScannerState::hidden(),
        }
    }
}

/// The applied side of the world, so each effect is issued only when it actually changes.
struct Applied {
    visible: bool,
    interactive: bool,
    origin: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
    emitted: Option<ScannerState>,
}

impl Applied {
    const fn new() -> Self {
        Self {
            // Matches how the window is built: hidden and click-through.
            visible: false,
            interactive: false,
            origin: None,
            size: None,
            emitted: None,
        }
    }
}

/// Turns the overlay on or off, switching mpv's own subtitles the other way.
pub(crate) fn set_scanner_popup_open<R: Runtime>(app: &AppHandle<R>, open: bool) {
    POPUP_OPEN.store(open, Ordering::Relaxed);
    reconcile(app);
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
    } else {
        POPUP_OPEN.store(false, Ordering::Relaxed);
    }
    reconcile(app);
    Ok(())
}

/// Builds the overlay window once, hidden and click-through, exactly like the recording indicator.
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
#[cfg(target_os = "windows")]
fn start_tracker<R: Runtime>(app: &AppHandle<R>) {
    if TRACKER_RUNNING.swap(true, Ordering::SeqCst) {
        return;
    }
    let app = app.clone();
    std::thread::spawn(move || {
        let mut modifier = ScanModifier::from_setting(&configured_modifier(&app));
        let mut ticks: u32 = 0;
        loop {
        if !ENABLED.load(Ordering::Relaxed) {
            std::thread::sleep(IDLE_INTERVAL);
            continue;
        }
        std::thread::sleep(TRACK_INTERVAL);
        ticks = ticks.wrapping_add(1);
        if ticks.is_multiple_of(MODIFIER_REFRESH_TICKS) {
            modifier = ScanModifier::from_setting(&configured_modifier(&app));
        }
        MODIFIER_HELD.store(modifier.is_held(), Ordering::Relaxed);
        ESCAPE_HELD.store(escape_is_held(), Ordering::Relaxed);
        reconcile(&app);
        }
    });
}

/// Whether the overlay has any business being on screen right now.
///
/// It is always-on-top and follows mpv's rectangle, which says nothing about whether mpv is
/// in FRONT — without a check, alt-tabbing away leaves a subtitle line floating over
/// whatever the user switched to.
#[cfg(target_os = "windows")]
fn overlay_should_draw<R: Runtime>(app: &AppHandle<R>, mpv_pid: u32) -> bool {
    let foreground = foreground_window();
    if foreground.is_null() {
        return false;
    }
    if window_process_id(foreground) == mpv_pid {
        return true;
    }
    app.get_webview_window(SCANNER_WINDOW_LABEL)
        .and_then(|window| window.hwnd().ok())
        .is_some_and(|handle| {
            std::ptr::eq(
                handle.0 as *const std::ffi::c_void,
                foreground as *const std::ffi::c_void,
            )
        })
}

/// Reads every input the overlay's appearance depends on, in one place.
#[cfg(target_os = "windows")]
fn sample_inputs<R: Runtime>(app: &AppHandle<R>) -> Inputs {
    let enabled = ENABLED.load(Ordering::Relaxed);
    let placement = enabled
        .then(watch_session_pid)
        .flatten()
        .filter(|pid| overlay_should_draw(app, *pid))
        .and_then(video_window_rect);
    Inputs {
        enabled,
        popup_open: POPUP_OPEN.load(Ordering::Relaxed),
        modifier_held: MODIFIER_HELD.load(Ordering::Relaxed),
        escape_held: ESCAPE_HELD.load(Ordering::Relaxed),
        placement,
    }
}

/// Brings the window in line with the inputs. **The only place any of this is applied.**
///
/// The module used to carry the same knowledge in several places — a visibility flag, a
/// click-through mirror, a last-emitted snapshot — each updated by hand at every call site.
/// Six bugs came out of that, all the same shape: one path changed the window without
/// updating a mirror, the next comparison saw "no change", and the window stayed wrong.
/// Two of them were introduced by the fix for a third.
#[cfg(target_os = "windows")]
fn reconcile<R: Runtime>(app: &AppHandle<R>) {
    let Some(window) = app.get_webview_window(SCANNER_WINDOW_LABEL) else {
        return;
    };
    let inputs = sample_inputs(app);
    let desired = inputs.desired();
    let state = inputs.frontend_state();

    let Ok(mut applied) = APPLIED.lock() else {
        return;
    };

    // Geometry before visibility, so the window is never painted at a stale position. Both
    // are compared rather than set every tick: this runs 60 times a second and the window
    // usually has not moved.
    if let Some(rect) = inputs.placement {
        let origin = (rect.left, rect.top);
        if applied.origin != Some(origin) {
            let _ = window.set_position(PhysicalPosition::new(rect.left, rect.top));
            applied.origin = Some(origin);
        }
        let size = (rect.width, rect.height);
        if applied.size != Some(size) {
            let _ = window.set_size(PhysicalSize::new(rect.width, rect.height));
            applied.size = Some(size);
        }
    }

    // Click-through before visibility on the way out, so the window can never be left
    // taking the mouse after it disappears.
    if applied.interactive != desired.interactive {
        let _ = window.set_ignore_cursor_events(!desired.interactive);
        applied.interactive = desired.interactive;
    }

    if applied.visible != desired.visible {
        let _ = if desired.visible {
            window.show()
        } else {
            window.hide()
        };
        applied.visible = desired.visible;
    }

    let should_emit = applied.emitted != Some(state);
    if should_emit {
        applied.emitted = Some(state);
    }
    // Never emit while holding a lock: an emit re-enters app state, and this module's
    // sibling overlay documents that as a deadlock.
    drop(applied);
    if should_emit {
        let _ = app.emit_to(SCANNER_WINDOW_LABEL, SCANNER_STATE_EVENT, state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(target_os = "windows")]
    fn placed() -> Option<VideoWindowRect> {
        Some(VideoWindowRect {
            left: 100,
            top: 50,
            width: 1280,
            height: 720,
            dpi: 96,
        })
    }

    #[cfg(target_os = "windows")]
    fn inputs(enabled: bool, popup_open: bool, modifier_held: bool) -> Inputs {
        Inputs {
            enabled,
            popup_open,
            modifier_held,
            escape_held: false,
            placement: placed(),
        }
    }

    /// The invariant the whole module exists to hold. An invisible window that still takes
    /// the mouse is how clicks meant for mpv were swallowed, twice, and it is now impossible
    /// to express rather than merely avoided.
    #[cfg(target_os = "windows")]
    #[test]
    fn an_invisible_overlay_is_never_interactive() {
        for popup_open in [false, true] {
            for modifier_held in [false, true] {
                // Switched off...
                let off = inputs(false, popup_open, modifier_held).desired();
                assert!(!off.visible);
                assert!(!off.interactive);

                // ...and nowhere to draw, which is mpv gone, minimised, or not in front.
                let nowhere = Inputs {
                    placement: None,
                    ..inputs(true, popup_open, modifier_held)
                }
                .desired();
                assert!(!nowhere.visible);
                assert!(!nowhere.interactive);
            }
        }
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn the_overlay_takes_the_mouse_only_while_scanning_or_reading() {
        let idle = inputs(true, false, false).desired();
        assert!(idle.visible);
        assert!(!idle.interactive);

        // Holding the modifier is what lets a word be hovered...
        assert!(inputs(true, false, true).desired().interactive);
        // ...and an open popup is what lets the entry be read after it is released.
        assert!(inputs(true, true, false).desired().interactive);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn the_frontend_is_told_to_stand_down_whenever_the_overlay_is_not_drawn() {
        // `tracking: false` is the overlay bundle's cue to drop a popup anchored to a line it can no longer show — the fix for a popup that survived an alt-tab.
        assert!(!inputs(false, true, true).frontend_state().tracking);
        assert!(!Inputs {
            placement: None,
            ..inputs(true, true, true)
        }
        .frontend_state()
        .tracking);

        let live = inputs(true, false, true).frontend_state();
        assert!(live.tracking);
        assert!(live.scanning);
        assert_eq!(live.width, 1280);
    }

    #[test]
    fn the_overlay_starts_disabled() {
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
