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

const INDICATOR_WIDTH: f64 = 340.0;
const INDICATOR_HEIGHT: f64 = 112.0;

const INDICATOR_MARGIN: f64 = 22.0;

#[derive(Copy, Clone)]
pub(crate) enum IndicatorSignal {
    Recording,
    Saved,
    Failed,
}

pub(crate) struct RecordingIndicatorState<R: Runtime> {
    tray: Mutex<Option<TrayIcon<R>>>,
    default_icon: Option<Image<'static>>,
    recording_icon: Image<'static>,
}

pub(crate) fn configure_recording_indicator<R: Runtime>(
    app: &AppHandle<R>,
    tray: TrayIcon<R>,
    default_icon: Option<Image<'static>>,
) -> Result<(), String> {
    build_indicator_window(app)?;

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


struct IndicatorPlacement {
    anchor: String,
    x: i32,
    y: i32,
    monitor_width: u32,
    monitor_height: u32,
    error: Option<String>,
}

fn position_indicator_window<R: Runtime>(
    app: &AppHandle<R>,
    window: &WebviewWindow<R>,
) -> Option<IndicatorPlacement> {
    let monitor = match app.primary_monitor() {
        Ok(Some(monitor)) => monitor,
        _ => return None,
    };

    let position = indicator_position_setting(app);

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
pub(crate) fn signal_recording_indicator<R: Runtime>(app: &AppHandle<R>, signal: IndicatorSignal) {
    let state = match app.try_state::<RecordingIndicatorState<R>>() {
        Some(state) => state,
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
