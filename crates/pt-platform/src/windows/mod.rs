//! Windows implementation, built on the CCD ("Connecting and Configuring
//! Displays") API: `QueryDisplayConfig` / `SetDisplayConfig`.
//!
//! - [`ccd`]: thin, safe wrappers around the raw Win32 calls
//! - [`display`]: [`WindowsDisplayBackend`], the `DisplayBackend` implementation
//! - [`autostart`]: "Start with Windows" (HKCU Run key)
//! - [`keyboard`]: [`WindowsKeyboard`], live modifier-key state
//! - [`window`]: rounded corners and no-focus-on-click for the overlay
//! - [`steam`]: whether the Steam client is running

mod autostart;
mod ccd;
mod display;
mod keyboard;
mod steam;
mod window;

pub use autostart::WindowsAutostart;
pub use display::WindowsDisplayBackend;
pub use keyboard::WindowsKeyboard;
pub use steam::steam_running;
pub use window::{make_window_non_activating, round_window_corners};
