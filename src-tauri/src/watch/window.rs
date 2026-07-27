//! Finding mpv's video window on screen.
//!
//! The scanner overlay has to sit exactly on top of the video, and mpv offers no way to ask
//! where it is — `--wid` embedding was rejected (it would make mpv a child of our window and
//! break its own fullscreen and OSC), so the window is located the way any other process
//! would do it: enumerate top-level windows, keep the ones owned by mpv's pid, and pick the
//! visible one.
//!
//! Measured in the spike: mpv owns **five** top-level windows — the video window plus
//! `mpv-smtc` (the media-transport-controls helper, permanently hidden) and three IME
//! windows with zero-sized rects. Only one is both visible and real, which is what
//! `video_window_for_pid` filters on. Matching on the window *title* would have been the
//! obvious alternative and is wrong: mpv's title is the filename by default and fully
//! user-configurable via `--title`.

use std::sync::atomic::{AtomicIsize, Ordering};

use windows_sys::core::BOOL;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, POINT, RECT, TRUE},
    // `ClientToScreen` lives with the GDI bindings rather than the windowing ones.
    Graphics::Gdi::ClientToScreen,
    UI::HiDpi::GetDpiForWindow,
    UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_MENU, VK_SHIFT},
    UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetClientRect, GetForegroundWindow,
        GetWindowThreadProcessId, IsWindowVisible, USER_DEFAULT_SCREEN_DPI,
    },
};

/// mpv's window class. Stable across builds and, unlike the title, not user-settable.
const MPV_WINDOW_CLASS: &str = "mpv";

/// A window narrower or shorter than this is a helper, not a video surface.
const MINIMUM_VIDEO_EXTENT: i32 = 120;

/// Where mpv's video is, in physical screen pixels, plus what it takes to place a window
/// there correctly on a mixed-DPI desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VideoWindowRect {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
    /// Physical pixels per logical pixel × 100. Tauri positions in physical pixels but sizes
    /// webview content in logical ones, so the overlay needs this to agree with mpv when the
    /// two windows are on monitors with different scaling.
    pub(crate) dpi: u32,
}

struct Search {
    pid: u32,
    found: HWND,
}

/// `EnumWindows` hands the callback an `LPARAM`, so the search state travels as a pointer.
unsafe extern "system" fn collect(window: HWND, state: LPARAM) -> BOOL {
    // SAFETY: `state` is the `&mut Search` handed to EnumWindows below, alive for the
    // duration of that call, and the callback is only ever invoked from inside it.
    let search = unsafe { &mut *(state as *mut Search) };

    let mut owner = 0u32;
    unsafe { GetWindowThreadProcessId(window, &mut owner) };
    if owner != search.pid {
        return TRUE;
    }
    if unsafe { IsWindowVisible(window) } == 0 {
        return TRUE;
    }

    let mut class = [0u16; 64];
    let written = unsafe { GetClassNameW(window, class.as_mut_ptr(), class.len() as i32) };
    if written <= 0 {
        return TRUE;
    }
    let class_name = String::from_utf16_lossy(&class[..written as usize]);
    if class_name != MPV_WINDOW_CLASS {
        return TRUE;
    }

    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if unsafe { GetClientRect(window, &mut rect) } == 0 {
        return TRUE;
    }
    if rect.right - rect.left < MINIMUM_VIDEO_EXTENT
        || rect.bottom - rect.top < MINIMUM_VIDEO_EXTENT
    {
        return TRUE;
    }

    search.found = window;
    // Stop enumerating: the first visible, correctly-classed, real-sized window is it.
    0
}

/// Caches the handle so the common case is one `GetClientRect` rather than a full enumeration.
/// Re-validated on every read, and discarded the moment mpv's window stops answering.
static CACHED_WINDOW: AtomicIsize = AtomicIsize::new(0);

fn window_is_still_mpv(window: HWND, pid: u32) -> bool {
    if window.is_null() || unsafe { IsWindowVisible(window) } == 0 {
        return false;
    }
    let mut owner = 0u32;
    unsafe { GetWindowThreadProcessId(window, &mut owner) };
    owner == pid
}

fn find_window(pid: u32) -> Option<HWND> {
    let cached = CACHED_WINDOW.load(Ordering::Relaxed) as HWND;
    if window_is_still_mpv(cached, pid) {
        return Some(cached);
    }

    let mut search = Search {
        pid,
        found: std::ptr::null_mut(),
    };
    unsafe { EnumWindows(Some(collect), &mut search as *mut Search as LPARAM) };
    if search.found.is_null() {
        CACHED_WINDOW.store(0, Ordering::Relaxed);
        return None;
    }
    CACHED_WINDOW.store(search.found as isize, Ordering::Relaxed);
    Some(search.found)
}

/// mpv's video area, or `None` while its window does not exist — during startup, after it
/// quits, and while it is minimised.
///
/// The **client** rect is used rather than the window rect: the window rect includes the
/// title bar and borders, and an overlay aligned to it would sit a title bar too high.
pub(crate) fn video_window_rect(pid: u32) -> Option<VideoWindowRect> {
    let window = find_window(pid)?;

    let mut client = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    if unsafe { GetClientRect(window, &mut client) } == 0 {
        return None;
    }

    // GetClientRect is window-relative and always starts at (0,0); ClientToScreen turns the
    // origin into a desktop coordinate.
    let mut origin = POINT { x: 0, y: 0 };
    if unsafe { ClientToScreen(window, &mut origin) } == 0 {
        return None;
    }

    let width = client.right - client.left;
    let height = client.bottom - client.top;
    if width < MINIMUM_VIDEO_EXTENT || height < MINIMUM_VIDEO_EXTENT {
        // Minimised windows report a degenerate client rect. Report "not placeable" rather
        // than parking the overlay in a corner.
        return None;
    }

    let dpi = match unsafe { GetDpiForWindow(window) } {
        0 => USER_DEFAULT_SCREEN_DPI,
        value => value,
    };

    Some(VideoWindowRect {
        left: origin.x,
        top: origin.y,
        width,
        height,
        dpi,
    })
}

/// Whether mpv is the window the user is actually looking at.
///
/// The overlay is always-on-top and follows mpv's rectangle, which says nothing about
/// whether mpv is in FRONT. Without this check, alt-tabbing to another app leaves a
/// subtitle line floating over it — the overlay is doing exactly what it was told, in a
/// place it has no business being.
///
/// Compared by process rather than by handle because mpv owns several top-level windows and
/// the foreground one is not guaranteed to be the same handle we track. The overlay itself
/// can never be the foreground window: it carries `WS_EX_NOACTIVATE`.
pub(crate) fn mpv_is_foreground(pid: u32) -> bool {
    let foreground = unsafe { GetForegroundWindow() };
    if foreground.is_null() {
        return false;
    }
    let mut owner = 0u32;
    unsafe { GetWindowThreadProcessId(foreground, &mut owner) };
    owner == pid
}

/// Which scanner modifier is held right now.
///
/// Polled rather than bound as a shortcut, deliberately: `tauri-plugin-global-shortcut`
/// registers accelerators (a modifier *plus* a key) and cannot report a bare modifier being
/// held. The scanner needs the held state while **mpv** has focus, so no DOM listener can
/// see it either.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ScanModifier {
    None,
    Shift,
    Control,
    Alt,
}

impl ScanModifier {
    pub(crate) fn from_setting(value: &str) -> Self {
        match value {
            "ctrl" | "control" => Self::Control,
            "alt" => Self::Alt,
            "none" => Self::None,
            _ => Self::Shift,
        }
    }

    /// `None` means "always scanning", so it reads as permanently held.
    pub(crate) fn is_held(self) -> bool {
        let key = match self {
            Self::None => return true,
            Self::Shift => VK_SHIFT,
            Self::Control => VK_CONTROL,
            Self::Alt => VK_MENU,
        };
        // The high bit is the down state; the low bit is "pressed since last call" and is
        // deliberately ignored — this is a level, not an edge.
        (unsafe { GetAsyncKeyState(key as i32) } as u16 & 0x8000) != 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_defaults_to_shift_for_anything_unrecognised() {
        assert_eq!(ScanModifier::from_setting("shift"), ScanModifier::Shift);
        assert_eq!(ScanModifier::from_setting("ctrl"), ScanModifier::Control);
        assert_eq!(ScanModifier::from_setting("control"), ScanModifier::Control);
        assert_eq!(ScanModifier::from_setting("alt"), ScanModifier::Alt);
        assert_eq!(ScanModifier::from_setting("none"), ScanModifier::None);
        // A hand-edited state.json must not disable scanning by typo.
        assert_eq!(ScanModifier::from_setting("meta"), ScanModifier::Shift);
        assert_eq!(ScanModifier::from_setting(""), ScanModifier::Shift);
    }

    #[test]
    fn no_modifier_reads_as_always_held() {
        assert!(ScanModifier::None.is_held());
    }

    #[test]
    fn a_dead_process_is_never_the_foreground_one() {
        // Guards the overlay's show/hide rule: an unknown pid must read as "not in front",
        // never as "in front", or the overlay would sit over other apps.
        assert!(!mpv_is_foreground(u32::MAX));
    }

    #[test]
    fn a_dead_pid_has_no_window() {
        // u32::MAX is not a live process, so this exercises the miss path — and the cache
        // must not hand back a stale handle for it.
        assert!(video_window_rect(u32::MAX).is_none());
    }

}
