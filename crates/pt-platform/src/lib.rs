//! # pt-platform
//!
//! The only crate that calls OS APIs. Each OS gets its own module that
//! implements the `pt-core` traits; [`display_backend`] picks the right one.
//!
//! Adding Linux later means adding `src/linux/` (e.g. a Wayland
//! `wlr-output-management` or X11 RandR backend) and one more `cfg` arm below.
//! Nothing outside this crate needs to change.

use pt_core::mock::{MOCK_LAYOUT_ENV, MockDisplayBackend};
use pt_core::{Autostart, DisplayBackend, KeyboardState, Result};
use raw_window_handle::HasWindowHandle;

#[cfg(windows)]
pub mod windows;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(not(windows))]
mod unsupported;

/// The display backend for the OS this binary was built for.
pub fn display_backend() -> Result<Box<dyn DisplayBackend>> {
    #[cfg(windows)]
    {
        Ok(Box::new(windows::WindowsDisplayBackend::new()))
    }
    #[cfg(not(windows))]
    {
        Ok(Box::new(unsupported::UnsupportedBackend))
    }
}

/// The backend every front end should use: the real one, or a file-backed
/// fake when `PANELTARIUM_MOCK_LAYOUT` points at a layout TOML file
/// (used by tests, and handy for trying things without touching monitors).
pub fn backend_from_env() -> Result<Box<dyn DisplayBackend>> {
    match std::env::var_os(MOCK_LAYOUT_ENV) {
        Some(path) if !path.is_empty() => Ok(Box::new(MockDisplayBackend::from_file(path)?)),
        _ => display_backend(),
    }
}

/// Live keyboard modifier state for this OS.
pub fn keyboard_state() -> Box<dyn KeyboardState> {
    #[cfg(windows)]
    {
        Box::new(windows::WindowsKeyboard)
    }
    #[cfg(not(windows))]
    {
        Box::new(unsupported::UnsupportedKeyboard)
    }
}

/// Give a borderless window rounded corners of `radius` physical pixels.
/// Best effort: does nothing where the OS can't (yet).
pub fn round_window_corners<W: HasWindowHandle + ?Sized>(window: &W, width: u32, height: u32, radius: u32) {
    #[cfg(windows)]
    {
        windows::round_window_corners(window, width, height, radius);
    }
    #[cfg(not(windows))]
    {
        // Linux (future): needs a transparent visual; X11/Wayland compositors
        // don't round undecorated windows on their own.
        let _ = (window, width, height, radius);
    }
}

/// Let a window receive clicks without becoming the active window (so the
/// overlay never takes keyboard focus from the game/app underneath).
pub fn make_window_non_activating<W: HasWindowHandle + ?Sized>(window: &W) {
    #[cfg(windows)]
    {
        windows::make_window_non_activating(window);
    }
    #[cfg(not(windows))]
    {
        // Linux (future): X11 `_NET_WM_WINDOW_TYPE_NOTIFICATION`/input hints.
        let _ = window;
    }
}

/// "Start at sign-in" for this OS.
pub fn autostart() -> Result<Box<dyn Autostart>> {
    #[cfg(windows)]
    {
        Ok(Box::new(windows::WindowsAutostart))
    }
    #[cfg(target_os = "linux")]
    {
        Ok(Box::new(linux::XdgAutostart::for_user()?))
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Ok(Box::new(unsupported::UnsupportedAutostart))
    }
}
