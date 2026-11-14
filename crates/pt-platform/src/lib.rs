//! # pt-platform
//!
//! The only crate that calls OS APIs. Each OS gets its own module that
//! implements the `pt-core` traits; [`display_backend`] picks the right one.
//!
//! - `windows/`: the CCD display API, registry, Win32 window styles
//! - `linux/`: one display driver per desktop (GNOME, KDE, wlroots, X11),
//!   the GlobalShortcuts portal for hotkeys on Wayland, XDG autostart
//! - `unsupported.rs`: stand-ins so everything builds and tests anywhere

use pt_core::mock::{MOCK_LAYOUT_ENV, MockDisplayBackend};
use pt_core::{Autostart, DisplayBackend, KeyboardState, Result, ShortcutEvent, ShortcutService};
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
    #[cfg(target_os = "linux")]
    {
        // No usable driver isn't fatal: the manager and CLI still open (to
        // edit hotkeys and settings) and every monitor call explains why.
        match linux::display_driver() {
            Ok(driver) => Ok(Box::new(linux::LinuxDisplayBackend::new(driver))),
            Err(e) => Ok(Box::new(unsupported::Unavailable(e.to_string()))),
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
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
    #[cfg(target_os = "linux")]
    {
        // Only a real X11 session can see the keyboard; on Wayland nobody
        // can (the overlay then stays open until a choice, Esc or timeout).
        let session = linux::Session::from_env();
        match (!session.wayland && session.x11)
            .then(linux::X11Keyboard::connect)
            .flatten()
        {
            Some(k) => Box::new(k),
            None => Box::new(unsupported::UnsupportedKeyboard),
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        Box::new(unsupported::UnsupportedKeyboard)
    }
}

/// Global hotkeys owned by the desktop, for sessions where apps can't grab
/// keys themselves (Wayland). `Ok(None)`: not needed here, grab keys
/// directly (`global-hotkey`). `Err`: needed, but the desktop doesn't offer
/// it.
pub fn shortcut_service(
    on_event: impl Fn(ShortcutEvent) + Send + Sync + 'static,
) -> Result<Option<Box<dyn ShortcutService>>> {
    #[cfg(target_os = "linux")]
    {
        if linux::Session::from_env().wayland {
            let portal = linux::PortalShortcuts::start(pt_core::text::APP_ID, on_event)?;
            return Ok(Some(Box::new(portal)));
        }
    }
    let _ = on_event;
    Ok(None)
}

/// Is the Steam client running? (Steam's own controllers are only read
/// while it is; see `pt-gamepad`.)
pub fn steam_running() -> bool {
    #[cfg(windows)]
    {
        windows::steam_running()
    }
    #[cfg(target_os = "linux")]
    {
        linux::steam_running()
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
        false
    }
}

/// Give a borderless window rounded corners of `radius` physical pixels.
/// Best effort: does nothing where the OS can't (yet).
pub fn round_window_corners<W: HasWindowHandle + ?Sized>(window: &W, width: u32, height: u32, radius: u32) {
    #[cfg(windows)]
    {
        windows::round_window_corners(window, width, height, radius);
    }
    #[cfg(target_os = "linux")]
    {
        // X11 / XWayland windows get a SHAPE mask; plain Wayland windows
        // can't be shaped (they stay square).
        use raw_window_handle::RawWindowHandle;
        let id = match window.window_handle().map(|h| h.as_raw()) {
            Ok(RawWindowHandle::Xlib(h)) => Some(h.window as u32),
            Ok(RawWindowHandle::Xcb(h)) => Some(h.window.get()),
            _ => None,
        };
        if let Some(id) = id {
            linux::round_corners(id, width, height, radius);
        }
    }
    #[cfg(not(any(windows, target_os = "linux")))]
    {
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
        // Linux: the overlay is an X11 override-redirect window (set when
        // it's created), which the window manager never focuses.
        let _ = window;
    }
}

/// Can a tray icon be shown? On Linux it needs the AppIndicator library
/// (`libayatana-appindicator3`), which `tray-icon` loads when the icon is
/// created and panics without. `Err` says what to install.
pub fn tray_support() -> std::result::Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        linux::tray_support()
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(())
    }
}

/// How the tray icon should look to fit in with the desktop's own icons.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrayIconStyle {
    /// The full-colour app icon (Windows, where tray icons are colourful).
    Colour,
    /// One-colour white, for dark panels (GNOME's top bar, dark themes).
    White,
    /// One-colour black, for light panels.
    Black,
}

pub fn tray_icon_style() -> TrayIconStyle {
    #[cfg(target_os = "linux")]
    {
        linux::tray_icon_style()
    }
    #[cfg(not(target_os = "linux"))]
    {
        TrayIconStyle::Colour
    }
}

/// Why a tray icon that was created won't be visible, if it won't
/// (Linux desktops without a tray area, e.g. GNOME without its extension).
pub fn tray_hint() -> Option<String> {
    #[cfg(target_os = "linux")]
    {
        linux::tray_host_missing()
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
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
