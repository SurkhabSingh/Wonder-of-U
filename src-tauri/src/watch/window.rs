use std::sync::atomic::{AtomicIsize, Ordering};

use windows_sys::core::BOOL;
use windows_sys::Win32::{
    Foundation::{HWND, LPARAM, POINT, RECT, TRUE},
    Graphics::Gdi::ClientToScreen,
    UI::HiDpi::GetDpiForWindow,
    UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_ESCAPE, VK_MENU, VK_SHIFT},
    UI::WindowsAndMessaging::{
        EnumWindows, GetClassNameW, GetClientRect, GetForegroundWindow,
        GetWindowThreadProcessId, IsWindowVisible, USER_DEFAULT_SCREEN_DPI,
    },
};

const MPV_WINDOW_CLASS: &str = "mpv";

const MINIMUM_VIDEO_EXTENT: i32 = 120;

/// Where mpv's video is, in physical screen pixels, plus what it takes to place a window
/// there correctly on a mixed-DPI desktop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VideoWindowRect {
    pub(crate) left: i32,
    pub(crate) top: i32,
    pub(crate) width: i32,
    pub(crate) height: i32,
    pub(crate) dpi: u32,
}

struct Search {
    pid: u32,
    found: HWND,
}

/// `EnumWindows` hands the callback an `LPARAM`, so the search state travels as a pointer.
unsafe extern "system" fn collect(window: HWND, state: LPARAM) -> BOOL {
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

    let mut origin = POINT { x: 0, y: 0 };
    if unsafe { ClientToScreen(window, &mut origin) } == 0 {
        return None;
    }

    let width = client.right - client.left;
    let height = client.bottom - client.top;
    if width < MINIMUM_VIDEO_EXTENT || height < MINIMUM_VIDEO_EXTENT {
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

pub(crate) fn foreground_window() -> HWND {
    unsafe { GetForegroundWindow() }
}

/// The process that owns `window`, or 0.
pub(crate) fn window_process_id(window: HWND) -> u32 {
    if window.is_null() {
        return 0;
    }
    let mut owner = 0u32;
    unsafe { GetWindowThreadProcessId(window, &mut owner) };
    owner
}

/// Whether Escape is down.
pub(crate) fn escape_is_held() -> bool {
    (unsafe { GetAsyncKeyState(VK_ESCAPE as i32) } as u16 & 0x8000) != 0
}

/// Which scanner modifier is held right now.
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

    pub(crate) fn is_held(self) -> bool {
        let key = match self {
            Self::None => return true,
            Self::Shift => VK_SHIFT,
            Self::Control => VK_CONTROL,
            Self::Alt => VK_MENU,
        };
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
    fn a_dead_process_never_owns_the_foreground_window() {
        // Guards the overlay's show/hide rule: an unknown process must never read as the one
        // in front, or the overlay would sit over other apps.
        assert_ne!(window_process_id(foreground_window()), u32::MAX);
        // A null handle is what the desktop reports mid-switch; it must not resolve to a
        // real process either.
        assert_eq!(window_process_id(std::ptr::null_mut()), 0);
    }

    #[test]
    fn a_dead_pid_has_no_window() {
        // u32::MAX is not a live process, so this exercises the miss path — and the cache
        // must not hand back a stale handle for it.
        assert!(video_window_rect(u32::MAX).is_none());
    }

}
