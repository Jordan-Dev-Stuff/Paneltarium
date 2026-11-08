//! Window tweaks for the overlay: rounded corners, and not taking focus
//! when clicked.
//!
//! Rounded corners: Windows 11 asks DWM to round the window (anti-aliased,
//! with its own thin border). Windows 10 has no such option (the call
//! fails), so the window is clipped to a rounded-rectangle region instead.

use raw_window_handle::{HasWindowHandle, RawWindowHandle};
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Dwm::{DWMWA_WINDOW_CORNER_PREFERENCE, DWMWCP_ROUND, DwmSetWindowAttribute};
use windows::Win32::Graphics::Gdi::{CreateRoundRectRgn, SetWindowRgn};
use windows::Win32::UI::WindowsAndMessaging::{GWL_EXSTYLE, GetWindowLongPtrW, SetWindowLongPtrW, WS_EX_NOACTIVATE};

fn hwnd<W: HasWindowHandle + ?Sized>(window: &W) -> Option<HWND> {
    let handle = window.window_handle().ok()?;
    match handle.as_raw() {
        RawWindowHandle::Win32(h) => Some(HWND(h.hwnd.get() as *mut core::ffi::c_void)),
        _ => None,
    }
}

/// Clicks still reach the window, but Windows won't activate it, so the
/// game/app you were using keeps keyboard focus.
pub fn make_window_non_activating<W: HasWindowHandle + ?Sized>(window: &W) {
    let Some(hwnd) = hwnd(window) else { return };
    unsafe {
        let ex = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex | WS_EX_NOACTIVATE.0 as isize);
    }
}

pub fn round_window_corners<W: HasWindowHandle + ?Sized>(window: &W, width: u32, height: u32, radius: u32) {
    let Some(hwnd) = hwnd(window) else { return };
    let pref = DWMWCP_ROUND;
    let dwm = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_WINDOW_CORNER_PREFERENCE,
            &pref as *const _ as *const core::ffi::c_void,
            std::mem::size_of_val(&pref) as u32,
        )
    };
    if dwm.is_ok() {
        return;
    }
    // Windows 10 fallback. The ellipse size is the corner diameter.
    let d = (radius * 2) as i32;
    unsafe {
        let region = CreateRoundRectRgn(0, 0, width as i32 + 1, height as i32 + 1, d, d);
        // On success the system owns the region; don't delete it.
        SetWindowRgn(hwnd, Some(region), true);
    }
}
