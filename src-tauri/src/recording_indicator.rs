use std::sync::Mutex;

use tauri::{
    image::Image, tray::TrayIcon, AppHandle, Emitter, Manager, PhysicalPosition, Runtime,
    WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

use crate::{
    app_runtime::log_event, app_state::normalize_indicator_position,
    app_types::SharedPersistedState,
};

/// Overlay window label and the event it listens for. Kept next to each other so
/// the backend show/emit and the tiny overlay bundle cannot drift apart.
const INDICATOR_WINDOW_LABEL: &str = "indicator";
const INDICATOR_EVENT: &str = "recording-indicator";

/// Logical size of the toast window. Sized like a ShadowPlay-style notification —
/// a card with an icon and two lines of text — and left taller than the card so it
/// can slide down from the top edge without being clipped.
const INDICATOR_WIDTH: f64 = 340.0;
const INDICATOR_HEIGHT: f64 = 112.0;
/// Gap the toast keeps from the top of the monitor work area.
const INDICATOR_MARGIN: f64 = 22.0;

/// The moments the global indicator reacts to. `Recording` flips the tray to its
/// red dot; the two terminal states restore the tray and flash a confirmation.
#[derive(Copy, Clone)]
pub(crate) enum IndicatorSignal {
    Recording,
    Saved,
    Failed,
}

/// Managed for the app's life so the tray icon can be swapped to the recording
/// variant and back long after the shell was built. Tauri keeps the overlay
/// window alive itself, so only the tray handle and the two icons live here.
///
/// The tray sits behind a `Mutex` purely to hand out `&TrayIcon` from shared
/// state — see the lock discipline note on [`signal_recording_indicator`].
pub(crate) struct RecordingIndicatorState<R: Runtime> {
    tray: Mutex<Option<TrayIcon<R>>>,
    default_icon: Option<Image<'static>>,
    recording_icon: Image<'static>,
}

/// Creates the click-through overlay window and stores the tray so recording can
/// drive both later. Called during shell setup, before the hotkeys register, so a
/// start or stop can always find the managed state.
pub(crate) fn configure_recording_indicator<R: Runtime>(
    app: &AppHandle<R>,
    tray: TrayIcon<R>,
    default_icon: Option<Image<'static>>,
) -> Result<(), String> {
    build_indicator_window(app)?;

    // Decoded once at startup rather than on every start: `Image::from_bytes`
    // needs the `image-png` feature, which is enabled in Cargo.toml for this.
    let recording_icon = Image::from_bytes(include_bytes!("../icons/tray-recording.png"))
        .map_err(|error| format!("Could not load the recording tray icon: {error}"))?;

    app.manage(RecordingIndicatorState {
        tray: Mutex::new(Some(tray)),
        default_icon,
        recording_icon,
    });

    Ok(())
}

/// Builds the overlay window once, hidden, and makes it click-through.
fn build_indicator_window<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    // Built hidden and reused for the app's life: `visible(false)` keeps it from
    // flashing on launch, and `signal_recording_indicator` shows it on demand.
    let window = WebviewWindowBuilder::new(
        app,
        INDICATOR_WINDOW_LABEL,
        WebviewUrl::App("overlay.html".into()),
    )
    .transparent(true)
    .decorations(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .resizable(false)
    .shadow(false)
    .focused(false)
    .visible(false)
    .inner_size(INDICATOR_WIDTH, INDICATOR_HEIGHT)
    .build()
    .map_err(|error| format!("Could not create the recording indicator window: {error}"))?;

    // The pill floats over whatever the user is watching, so it must never eat a
    // click meant for the content beneath it.
    window
        .set_ignore_cursor_events(true)
        .map_err(|error| format!("Could not make the recording indicator click-through: {error}"))?;

    let placement = position_indicator_window(app, &window);
    log_event(
        app,
        "INFO",
        "indicator.built",
        serde_json::json!({
            "anchor": placement.as_ref().map(|placement| placement.anchor.clone()),
            "x": placement.as_ref().map(|placement| placement.x),
            "y": placement.as_ref().map(|placement| placement.y),
            "monitorWidth": placement.as_ref().map(|placement| placement.monitor_width),
            "monitorHeight": placement.as_ref().map(|placement| placement.monitor_height),
            "positionError": placement.as_ref().and_then(|placement| placement.error.clone())
        }),
    );

    Ok(())
}

/// Where a toast was anchored, and what refused it if anything.
///
/// Returned rather than logged in place so one toast produces one log line: an
/// anchor that lands off-screen and a toast that never fired look identical from
/// outside the app, and telling them apart is the whole reason any of this is
/// recorded. `None` means no monitor was reported, so nothing was moved.
struct IndicatorPlacement {
    anchor: String,
    x: i32,
    y: i32,
    monitor_width: u32,
    monitor_height: u32,
    error: Option<String>,
}

/// Parks the toast at the user's chosen anchor within the primary monitor's work
/// area (default top-center — the spot the eye lands on and clear of the video
/// controls along the bottom). If no monitor is reported we leave the window
/// where it landed rather than guess a position that could push it off-screen.
fn position_indicator_window<R: Runtime>(
    app: &AppHandle<R>,
    window: &WebviewWindow<R>,
) -> Option<IndicatorPlacement> {
    let monitor = match app.primary_monitor() {
        Ok(Some(monitor)) => monitor,
        _ => return None,
    };

    let position = indicator_position_setting(app);

    // `work_area` is physical pixels, but the window size and margin were booked
    // in logical units, so scale them up to match before anchoring.
    let scale = monitor.scale_factor();
    let work_area = monitor.work_area();
    let window_width = INDICATOR_WIDTH * scale;
    let window_height = INDICATOR_HEIGHT * scale;
    let margin = INDICATOR_MARGIN * scale;

    let left = work_area.position.x as f64;
    let top = work_area.position.y as f64;
    let area_width = work_area.size.width as f64;
    let area_height = work_area.size.height as f64;

    let x = match position.as_str() {
        "top-left" | "bottom-left" => left + margin,
        "top-right" | "bottom-right" => left + area_width - window_width - margin,
        // top-center / bottom-center, and the normalized fallback.
        _ => left + (area_width - window_width) / 2.0,
    };
    let y = match position.as_str() {
        "bottom-left" | "bottom-center" | "bottom-right" => {
            top + area_height - window_height - margin
        }
        // Every top-* anchor, and the fallback.
        _ => top + margin,
    };

    let placed = PhysicalPosition::new(x.round() as i32, y.round() as i32);
    let error = window.set_position(placed).err().map(|error| error.to_string());

    Some(IndicatorPlacement {
        anchor: position,
        x: placed.x,
        y: placed.y,
        monitor_width: work_area.size.width,
        monitor_height: work_area.size.height,
        error,
    })
}

/// The user's chosen toast anchor, normalized to one of the six known values so
/// a bad stored value can never place the window off-screen. Defaults to
/// top-center if the settings state is missing or unreadable.
fn indicator_position_setting<R: Runtime>(app: &AppHandle<R>) -> String {
    app.try_state::<SharedPersistedState>()
        .and_then(|state| {
            state.0.lock().ok().map(|guard| {
                normalize_indicator_position(&guard.settings.indicator_position).to_string()
            })
        })
        .unwrap_or_else(|| "top-center".to_string())
}

/// Drives the global indicator for one lifecycle moment: swaps the tray icon and
/// tooltip, then flashes the corner pill.
///
/// Payload shape (plain keys, no rename): `{ "state": "recording" | "saved" |
/// "failed", "label": "Recording" | "Saved" | "Recording failed" }`.
///
/// Lock discipline: the tray guard is dropped before any window op or emit.
/// `update_shell_snapshot` deadlocks the app when a `std::sync::Mutex` is held
/// across an emit — the emit re-locks state to rebuild the bootstrap — and the
/// tray lock is no exception, so we never straddle the show/emit with it held.
/// A missing state (setup failed) is a no-op rather than a panic.
pub(crate) fn signal_recording_indicator<R: Runtime>(app: &AppHandle<R>, signal: IndicatorSignal) {
    let state = match app.try_state::<RecordingIndicatorState<R>>() {
        Some(state) => state,
        // Setup failed, so there is no tray handle and no overlay. The reason was
        // reported once at startup; without this the toast simply never appears and
        // nothing says why.
        None => {
            log_event(
                app,
                "WARN",
                "indicator.not_configured",
                serde_json::json!({}),
            );
            return;
        }
    };

    {
        let tray = match state.tray.lock() {
            Ok(tray) => tray,
            Err(_) => return,
        };
        if let Some(tray) = tray.as_ref() {
            let (icon, tooltip) = match signal {
                IndicatorSignal::Recording => (
                    Some(state.recording_icon.clone()),
                    "Wonder of U — ● Recording",
                ),
                IndicatorSignal::Saved | IndicatorSignal::Failed => {
                    (state.default_icon.clone(), "Wonder of U")
                }
            };
            let _ = tray.set_icon(icon);
            let _ = tray.set_tooltip(Some(tooltip));
        }
    }

    let (indicator_state, label) = match signal {
        IndicatorSignal::Recording => ("recording", "Recording started"),
        IndicatorSignal::Saved => ("saved", "Recording saved"),
        IndicatorSignal::Failed => ("failed", "Recording failed"),
    };

    let window = app.get_webview_window(INDICATOR_WINDOW_LABEL);
    let mut placement = None;
    let shown = match &window {
        Some(window) => {
            // Re-anchor on every appearance so a changed position setting takes
            // effect on the next toast without an app restart.
            placement = position_indicator_window(app, window);
            window.show().map(|()| true).map_err(|error| error.to_string())
        }
        None => Ok(false),
    };
    let emitted = app.emit_to(
        INDICATOR_WINDOW_LABEL,
        INDICATOR_EVENT,
        serde_json::json!({ "state": indicator_state, "label": label }),
    );

    // Most reasons the toast fails to appear are silent otherwise: the managed state
    // missing, the window never built, `show` refused, or the anchor landing off the
    // screen. Each is a different fix and none could be told apart from outside.
    //
    // What this CANNOT tell you: `emit_to` reports that the event was dispatched, not
    // that anything was listening for it. A webview that failed to load its script
    // answers exactly like one that painted the card, so an INFO here means every
    // step the backend owns succeeded — not that anyone saw a toast.
    log_event(
        app,
        if shown.as_ref().is_ok_and(|shown| *shown) && emitted.is_ok() {
            "INFO"
        } else {
            "WARN"
        },
        "indicator.signalled",
        serde_json::json!({
            "state": indicator_state,
            "windowFound": window.is_some(),
            "shown": shown.as_ref().ok().copied().unwrap_or(false),
            "showError": shown.err(),
            "emitError": emitted.err().map(|error| error.to_string()),
            "anchor": placement.as_ref().map(|placement| placement.anchor.clone()),
            "x": placement.as_ref().map(|placement| placement.x),
            "y": placement.as_ref().map(|placement| placement.y),
            "monitorWidth": placement.as_ref().map(|placement| placement.monitor_width),
            "monitorHeight": placement.as_ref().map(|placement| placement.monitor_height),
            "positionError": placement.as_ref().and_then(|placement| placement.error.clone())
        }),
    );
}
