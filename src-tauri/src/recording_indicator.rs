use std::sync::Mutex;

use tauri::{
    image::Image, tray::TrayIcon, AppHandle, Emitter, Manager, PhysicalPosition, Runtime,
    WebviewUrl, WebviewWindow, WebviewWindowBuilder,
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

    position_indicator_window(app, &window);

    Ok(())
}

/// Parks the toast at the top-center of the primary monitor's work area — the
/// spot the eye naturally lands on, and clear of the video controls that usually
/// sit along the bottom. If no monitor is reported we leave the window where it
/// landed rather than guess a position that could push it off-screen.
fn position_indicator_window<R: Runtime>(app: &AppHandle<R>, window: &WebviewWindow<R>) {
    let monitor = match app.primary_monitor() {
        Ok(Some(monitor)) => monitor,
        _ => return,
    };

    // `work_area` is physical pixels, but the window size was booked in logical
    // units, so scale it up to match before centering.
    let scale = monitor.scale_factor();
    let work_area = monitor.work_area();
    let window_width = INDICATOR_WIDTH * scale;
    let margin = INDICATOR_MARGIN * scale;

    let x =
        work_area.position.x as f64 + (work_area.size.width as f64 - window_width) / 2.0;
    let y = work_area.position.y as f64 + margin;

    let _ = window.set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32));
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
        None => return,
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

    if let Some(window) = app.get_webview_window(INDICATOR_WINDOW_LABEL) {
        let _ = window.show();
    }
    let _ = app.emit_to(
        INDICATOR_WINDOW_LABEL,
        INDICATOR_EVENT,
        serde_json::json!({ "state": indicator_state, "label": label }),
    );
}
