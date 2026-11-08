//! Linux implementations. So far only autostart; the display backend comes
//! later (X11 RandR / Wayland protocols, see the roadmap).

mod autostart;

pub use autostart::XdgAutostart;
