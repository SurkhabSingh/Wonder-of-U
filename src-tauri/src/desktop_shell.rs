use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    App, AppHandle, Manager, Runtime, WindowEvent,
};
use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};

#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{CloseHandle, GetLastError, ERROR_ALREADY_EXISTS, HANDLE},
    System::Threading::CreateMutexW,
    UI::WindowsAndMessaging::{
        FindWindowW, IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW,
    },
};

use crate::{
    anki::mine_watched_line_inner,
    app_config::AUTOSTART_ARGUMENT,
    app_runtime::{log_event, update_shell_snapshot},
    watch::watch_snapshot,
    app_types::{
        SharedPersistedState, SharedShellState, MINE_SHORTCUT, SHOW_SHORTCUT, START_SHORTCUT,
        STOP_SHORTCUT,
    },
    recording_indicator::{
        configure_recording_indicator, signal_recording_indicator, IndicatorSignal,
    },
    recording_session::{start_recording_inner, stop_recording_inner},
};

const APP_TITLE: &str = "Wonder of U";
const START_SHORTCUT_CANDIDATES: [&str; 3] = [START_SHORTCUT, "Ctrl+Alt+Shift+R", "Ctrl+Alt+F8"];
const STOP_SHORTCUT_CANDIDATES: [&str; 3] = [STOP_SHORTCUT, "Ctrl+Alt+Shift+S", "Ctrl+Alt+F9"];
const SHOW_SHORTCUT_CANDIDATES: [&str; 3] = [SHOW_SHORTCUT, "Ctrl+Alt+Shift+W", "Ctrl+Alt+F10"];
const MINE_SHORTCUT_CANDIDATES: [&str; 3] = [MINE_SHORTCUT, "Ctrl+Alt+Shift+M", "Ctrl+Alt+F11"];

#[derive(Default)]
pub(crate) struct StartupVisibility {
    initialized: AtomicBool,
    page_loaded: AtomicBool,
    resolved: AtomicBool,
    start_minimized: AtomicBool,
}

#[derive(Copy, Clone)]
enum HotkeyAction {
    Start,
    Stop,
    ShowWindow,
    Mine,
}

pub(crate) fn show_main_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        window.show()?;
        window.unminimize()?;
        window.set_focus()?;
    }

    Ok(())
}

pub(crate) fn hide_main_window<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window("main") {
        window.hide()?;
    }

    Ok(())
}

fn resolve_startup_visibility<R: Runtime>(
    app: &AppHandle<R>,
    startup_visibility: &StartupVisibility,
) {
    if !startup_visibility.initialized.load(Ordering::Acquire)
        || !startup_visibility.page_loaded.load(Ordering::Acquire)
        || startup_visibility.resolved.swap(true, Ordering::AcqRel)
    {
        return;
    }

    if !startup_visibility.start_minimized.load(Ordering::Acquire) {
        let _ = show_main_window(app);
    }
}

pub(crate) fn mark_main_page_loaded<R: Runtime>(
    app: &AppHandle<R>,
    startup_visibility: &StartupVisibility,
) {
    startup_visibility
        .page_loaded
        .store(true, Ordering::Release);
    resolve_startup_visibility(app, startup_visibility);
}

fn handle_shortcut<R: Runtime>(app: &AppHandle<R>, action: HotkeyAction, shortcut: &str) {
    let _ = update_shell_snapshot(app, |shell| {
        shell.last_shortcut = Some(shortcut.to_string());
    });

    let action_result = match action {
        HotkeyAction::Start => start_recording_inner(app, None),
        HotkeyAction::Stop => stop_recording_inner(app),
        HotkeyAction::ShowWindow => show_main_window(app).map_err(|error| error.to_string()),
        // Mining reads mpv fresh, so the card is made from the line being heard right
        // now. Doing nothing when no video is playing is correct rather than an error:
        // the hotkey is global and will get pressed by accident.
        HotkeyAction::Mine => match mine_watched_line_from_player(app) {
            Ok(()) => Ok(()),
            Err(error) if error == NO_WATCH_SESSION => Ok(()),
            Err(error) => Err(error),
        },
    };

    if let Err(error) = action_result {
        log_event(
            app,
            "ERROR",
            "hotkey.failed",
            serde_json::json!({
                "shortcut": shortcut,
                "message": error
            }),
        );
        // A hotkey-triggered start or stop fails while the window is hidden, so the
        // in-app toast never surfaces — flash the global pill instead. `ShowWindow`
        // has nothing to do with recording, so it stays silent.
        if matches!(action, HotkeyAction::Start | HotkeyAction::Stop) {
            signal_recording_indicator(app, IndicatorSignal::Failed);
        }
        let _ = update_shell_snapshot(app, |shell| {
            shell.phase = "error".into();
            shell.status_text = error.clone();
            shell.started_at_ms = None;
            shell.current_recording_name = None;
        });
    }
}

/// The one "failure" that is really just "nothing to do", so the hotkey can stay silent
/// rather than logging an error every time it is pressed outside a watch session.
const NO_WATCH_SESSION: &str = "No video is playing.";

/// Mines whatever line mpv has on screen right now.
///
/// Reads the player rather than any cached UI state: the user presses the key because of
/// what they are hearing, and the watch panel's poll can be a quarter-second behind.
fn mine_watched_line_from_player<R: Runtime>(app: &AppHandle<R>) -> Result<(), String> {
    let snapshot = watch_snapshot()?;
    if !snapshot.connected {
        return Err(NO_WATCH_SESSION.into());
    }
    let Some(video_path) = snapshot.path else {
        return Err(NO_WATCH_SESSION.into());
    };
    let Some(text) = snapshot
        .subtitle_text
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
    else {
        return Err("There is no subtitle on screen to mine.".into());
    };
    // Both bounds come from mpv. Without them there is nothing to cut, and guessing a
    // window around the position would produce a clip that does not match the card.
    let (Some(start_ms), Some(end_ms)) = (snapshot.subtitle_start_ms, snapshot.subtitle_end_ms)
    else {
        return Err("mpv did not report this line's timing.".into());
    };

    let result = mine_watched_line_inner(app, video_path, text, start_ms, end_ms, None, None)?;
    // The status line is the only feedback available when the app window is behind mpv,
    // so a hotkey mine has to say what happened there.
    let message = result.message.clone();
    update_shell_snapshot(app, |shell| {
        shell.status_text = message.clone();
    })?;
    Ok(())
}

fn register_hotkey<R: Runtime>(
    app: &AppHandle<R>,
    action: HotkeyAction,
    label: &str,
    candidates: &[&'static str],
) -> Result<(String, Option<String>), String> {
    let global_shortcut = app.global_shortcut();
    let mut last_error = None;

    for candidate in candidates {
        let registered_shortcut = *candidate;
        match global_shortcut.on_shortcut(registered_shortcut, move |app, _shortcut, event| {
            if event.state != ShortcutState::Pressed {
                return;
            }

            handle_shortcut(app, action, registered_shortcut);
        }) {
            Ok(()) => {
                let warning = if registered_shortcut == candidates[0] {
                    None
                } else {
                    Some(format!(
                        "{label} hotkey moved to {registered_shortcut} because {primary} was unavailable.",
                        primary = candidates[0]
                    ))
                };

                return Ok((registered_shortcut.to_string(), warning));
            }
            Err(error) => last_error = Some(error.to_string()),
        }
    }

    Ok((
        "Unavailable".into(),
        Some(format!(
            "{label} hotkey could not be registered. Tried: {}. {}",
            candidates.join(", "),
            last_error.unwrap_or_else(|| "The operating system rejected every candidate.".into())
        )),
    ))
}

pub(crate) fn configure_desktop_shell<R: Runtime>(
    app: &mut App<R>,
    startup_visibility: &Arc<StartupVisibility>,
    mut warnings: Vec<String>,
) -> Result<(), String> {
    let show_item = MenuItem::with_id(app, "show", "Open Wonder of U", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let hide_item = MenuItem::with_id(app, "hide", "Hide Window", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let quit_item = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)
        .map_err(|error| error.to_string())?;
    let menu = Menu::with_items(app, &[&show_item, &hide_item, &quit_item])
        .map_err(|error| error.to_string())?;

    // Kept so the recording indicator can restore it after flashing the red dot.
    // Owned as `Image<'static>` because it outlives this call inside managed state.
    let default_icon = app.default_window_icon().cloned().map(Image::to_owned);

    let mut tray_builder = TrayIconBuilder::new().tooltip(APP_TITLE).menu(&menu);
    if let Some(icon) = default_icon.clone() {
        tray_builder = tray_builder.icon(icon);
    }

    // The returned handle used to be dropped; the indicator keeps it so recording
    // can swap the tray icon between the default and the red dot.
    let tray = tray_builder
        .on_menu_event(|app, event| match event.id.as_ref() {
            "show" => {
                let _ = show_main_window(app);
            }
            "hide" => {
                let _ = hide_main_window(app);
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                let _ = show_main_window(tray.app_handle());
            }
        })
        .build(app)
        .map_err(|error| error.to_string())?;

    // Set up before the hotkeys register, so the first start or stop can already
    // find the managed state. A failure here is non-fatal: the app still records,
    // just without the global indicator, and the reason lands in the warnings.
    if let Err(error) = configure_recording_indicator(app.handle(), tray, default_icon) {
        warnings.push(format!(
            "The recording indicator could not be initialized. {error}"
        ));
    }

    if let Some(window) = app.get_webview_window("main") {
        let app_handle = app.handle().clone();
        window.on_window_event(move |event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = hide_main_window(&app_handle);
            }
        });
    }

    let (start_binding, start_warning) = register_hotkey(
        app.handle(),
        HotkeyAction::Start,
        "Start",
        &START_SHORTCUT_CANDIDATES,
    )?;
    let (stop_binding, stop_warning) = register_hotkey(
        app.handle(),
        HotkeyAction::Stop,
        "Stop",
        &STOP_SHORTCUT_CANDIDATES,
    )?;
    let (show_binding, show_warning) = register_hotkey(
        app.handle(),
        HotkeyAction::ShowWindow,
        "Show window",
        &SHOW_SHORTCUT_CANDIDATES,
    )?;
    let (mine_binding, mine_warning) = register_hotkey(
        app.handle(),
        HotkeyAction::Mine,
        "Mine line",
        &MINE_SHORTCUT_CANDIDATES,
    )?;

    warnings.extend(
        [start_warning, stop_warning, show_warning, mine_warning]
            .into_iter()
            .flatten(),
    );

    {
        let shell_state = app.state::<SharedShellState>();
        let mut shell = shell_state
            .0
            .lock()
            .map_err(|_| "Could not initialize shell state.".to_string())?;
        shell.hotkeys.start = start_binding;
        shell.hotkeys.stop = stop_binding;
        shell.hotkeys.show_window = show_binding;
        shell.hotkeys.mine = mine_binding;
        if !warnings.is_empty() {
            shell.status_text = format!(
                "Tray shell is ready with fallback hotkeys. {}",
                warnings.join(" ")
            );
        }
    }

    let start_minimized = {
        let persisted_state = app.state::<SharedPersistedState>();
        let persisted = persisted_state
            .0
            .lock()
            .map_err(|_| "Could not read minimized startup preference.".to_string())?;
        persisted.settings.start_minimized
    };

    startup_visibility
        .start_minimized
        .store(start_minimized, Ordering::Release);
    startup_visibility
        .initialized
        .store(true, Ordering::Release);
    resolve_startup_visibility(app.handle(), startup_visibility);

    Ok(())
}

#[cfg(target_os = "windows")]
pub(crate) struct SingleInstanceGuard {
    handle: HANDLE,
}

#[cfg(target_os = "windows")]
impl Drop for SingleInstanceGuard {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe {
                CloseHandle(self.handle);
            }
        }
    }
}

#[cfg(target_os = "windows")]
fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(target_os = "windows")]
fn current_launch_should_focus_existing_instance() -> bool {
    !std::env::args().any(|argument| argument == AUTOSTART_ARGUMENT)
}

#[cfg(target_os = "windows")]
fn focus_existing_instance_window() {
    let window_title = wide_null(APP_TITLE);

    unsafe {
        let window = FindWindowW(std::ptr::null(), window_title.as_ptr());
        if window.is_null() {
            return;
        }

        if IsIconic(window) != 0 {
            ShowWindow(window, SW_RESTORE);
        } else {
            ShowWindow(window, SW_SHOW);
        }

        SetForegroundWindow(window);
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn acquire_single_instance_or_exit() -> Option<SingleInstanceGuard> {
    let mutex_name = wide_null("Local\\com.wonderofu.desktop.single-instance");
    let handle = unsafe { CreateMutexW(std::ptr::null(), 0, mutex_name.as_ptr()) };
    if handle.is_null() {
        return None;
    }

    if unsafe { GetLastError() } == ERROR_ALREADY_EXISTS {
        if current_launch_should_focus_existing_instance() {
            focus_existing_instance_window();
        }

        unsafe {
            CloseHandle(handle);
        }
        std::process::exit(0);
    }

    Some(SingleInstanceGuard { handle })
}

#[cfg(not(target_os = "windows"))]
pub(crate) fn acquire_single_instance_or_exit() {}
