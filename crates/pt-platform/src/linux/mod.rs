//! Linux implementations.
//!
//! Linux has no single display API: each desktop has its own. They all
//! share one [`outputs::LinuxDisplayBackend`] (the logic), with a driver per
//! display stack, picked at start-up by [`display_driver`]:
//!
//! | Desktop | Driver |
//! |---|---|
//! | GNOME (Wayland or X11), and other Mutter desktops | [`mutter`]: Mutter's D-Bus API |
//! | KDE Plasma on Wayland | [`kscreen`]: `kscreen-doctor` |
//! | Sway, Hyprland, river, labwc, Wayfire… | [`wlroots`]: `wlr-randr` |
//! | Any X11 session (Xfce, MATE, Cinnamon, i3, Plasma on X11…) | [`xrandr`]: `xrandr` |
//!
//! `PANELTARIUM_DISPLAY_BACKEND=gnome|kde|wlroots|x11` forces one.
//!
//! Also here: start at sign-in ([`autostart`]), global hotkeys through the
//! desktop on Wayland ([`portal`]), modifier keys and window shapes on X11
//! ([`x11`]), and whether Steam is running.

mod autostart;
mod command;
mod coords;
mod edid;
mod kscreen;
mod mutter;
mod outputs;
mod portal;
mod wlroots;
mod x11;
mod xrandr;

pub use autostart::XdgAutostart;
pub use outputs::LinuxDisplayBackend;
pub use portal::PortalShortcuts;
pub use x11::{X11Keyboard, round_corners};

use pt_core::{Error, Result};

use command::{System, on_path};
use outputs::{Coords, Driver};

/// Forces a display driver (`gnome`, `kde`, `wlroots` or `x11`).
pub const BACKEND_ENV: &str = "PANELTARIUM_DISPLAY_BACKEND";

/// What the session looks like, from its environment.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Session {
    pub wayland: bool,
    pub x11: bool,
    /// `XDG_CURRENT_DESKTOP`, split on ':' and upper-cased.
    pub desktops: Vec<String>,
    pub forced: Option<String>,
}

impl Session {
    pub fn from_env() -> Self {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.is_empty());
        let kind = var("XDG_SESSION_TYPE").unwrap_or_default();
        Session {
            wayland: kind == "wayland" || var("WAYLAND_DISPLAY").is_some(),
            x11: kind == "x11" || (kind.is_empty() && var("WAYLAND_DISPLAY").is_none() && var("DISPLAY").is_some()),
            desktops: var("XDG_CURRENT_DESKTOP")
                .unwrap_or_default()
                .split(':')
                .map(|d| d.trim().to_ascii_uppercase())
                .filter(|d| !d.is_empty())
                .collect(),
            forced: var(BACKEND_ENV).map(|v| v.to_ascii_lowercase()),
        }
    }

    fn is(&self, desktop: &str) -> bool {
        self.desktops.iter().any(|d| d == desktop)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DriverKind {
    Gnome,
    Kde,
    Wlroots,
    X11,
}

/// Which driver fits `session`. `gnome_running` and `has` (is a program on
/// PATH) are passed in so this is testable.
pub fn choose_driver(session: &Session, gnome_running: bool, has: &dyn Fn(&str) -> bool) -> Option<DriverKind> {
    match session.forced.as_deref() {
        Some("gnome" | "mutter") => return Some(DriverKind::Gnome),
        Some("kde" | "kscreen" | "plasma") => return Some(DriverKind::Kde),
        Some("wlroots" | "wlr" | "sway" | "hyprland") => return Some(DriverKind::Wlroots),
        Some("x11" | "xrandr") => return Some(DriverKind::X11),
        _ => {}
    }
    if gnome_running {
        Some(DriverKind::Gnome)
    } else if session.wayland && session.is("KDE") && has(kscreen::PROGRAM) {
        Some(DriverKind::Kde)
    } else if session.wayland && has(wlroots::PROGRAM) {
        Some(DriverKind::Wlroots)
    } else if session.x11 && has(xrandr::PROGRAM) {
        Some(DriverKind::X11)
    } else {
        None
    }
}

/// The display driver for this session.
pub fn display_driver() -> Result<Box<dyn Driver>> {
    let session = Session::from_env();
    // Mutter's D-Bus service is the best signal for GNOME (it's also there
    // on GNOME-based desktops that don't say "GNOME").
    let gnome = match session.forced.as_deref() {
        None | Some("gnome" | "mutter") => mutter::MutterDriver::connect(),
        Some(_) => Err(Error::Unsupported),
    };
    let gnome_running = gnome.is_ok();
    match choose_driver(&session, gnome_running, &on_path) {
        Some(DriverKind::Gnome) => Ok(Box::new(gnome?)),
        Some(DriverKind::Kde) => Ok(Box::new(kscreen::KScreenDriver::new(
            Box::new(System),
            if session.wayland {
                Coords::Logical
            } else {
                Coords::Physical
            },
        ))),
        Some(DriverKind::Wlroots) => Ok(Box::new(wlroots::WlrootsDriver::new(Box::new(System)))),
        Some(DriverKind::X11) => Ok(Box::new(xrandr::XrandrDriver::new(Box::new(System)))),
        None => Err(Error::Os {
            call: "display backend",
            code: None,
            message: no_driver_hint(&session),
        }),
    }
}

fn no_driver_hint(session: &Session) -> String {
    let what = if session.is("KDE") {
        "install kscreen-doctor (package: kscreen or libkscreen)"
    } else if session.wayland {
        "on wlroots compositors (Sway, Hyprland, …) install wlr-randr"
    } else {
        "install xrandr (package: xrandr or x11-xserver-utils)"
    };
    format!(
        "can't change monitors on this desktop: {what}. Supported: GNOME, KDE Plasma, wlroots compositors and X11 \
         sessions (set {BACKEND_ENV} to choose one)"
    )
}

/// The tray icon's library (what `libappindicator-sys` looks for).
pub fn tray_support() -> std::result::Result<(), String> {
    const LIBS: [&str; 2] = ["libayatana-appindicator3.so.1", "libappindicator3.so.1"];
    // Only checks the library can be found; it's loaded for real (and kept)
    // by the tray itself.
    if LIBS.iter().any(|l| unsafe { libloading::Library::new(l) }.is_ok()) {
        Ok(())
    } else {
        Err(
            "no tray icon: install libayatana-appindicator3 (Fedora: libayatana-appindicator-gtk3, \
             Debian/Ubuntu: libayatana-appindicator3-1, Arch: libayatana-appindicator)"
                .into(),
        )
    }
}

/// Tray icons on Linux go to whoever owns `org.kde.StatusNotifierWatcher`
/// on the session bus (KDE, most panels; on GNOME only the AppIndicator
/// extension). Without one the icon is created but shown nowhere.
pub fn tray_host_missing() -> Option<String> {
    let conn = zbus::blocking::Connection::session().ok()?;
    let dbus = zbus::blocking::fdo::DBusProxy::new(&conn).ok()?;
    let name = "org.kde.StatusNotifierWatcher".try_into().ok()?;
    if dbus.name_has_owner(name).unwrap_or(true) {
        return None;
    }
    Some(if Session::from_env().is("GNOME") {
        "the tray icon is hidden: GNOME needs the \"AppIndicator and KStatusNotifierItem Support\" extension \
         (Fedora: gnome-shell-extension-appindicator, then `gnome-extensions enable appindicatorsupport@rgcjonas.gmail.com`)"
            .into()
    } else {
        "the tray icon is hidden: this desktop has no tray (StatusNotifier) area".into()
    })
}

/// Panel icons on Linux desktops are one colour (white on GNOME's always
/// dark top bar; on other desktops whatever contrasts with the theme).
pub fn tray_icon_style() -> crate::TrayIconStyle {
    style_for(&Session::from_env(), color_scheme())
}

fn style_for(session: &Session, color_scheme: Option<u32>) -> crate::TrayIconStyle {
    use crate::TrayIconStyle::{Black, White};
    if session.is("GNOME") {
        return White; // the top bar is dark in light mode too
    }
    // XDG `color-scheme`: 0 no preference, 1 prefer dark, 2 prefer light.
    match color_scheme {
        Some(2) => Black,
        _ => White,
    }
}

/// The desktop's light/dark preference, from the XDG Settings portal
/// (GNOME, KDE, Cinnamon, Xfce, … all publish it).
fn color_scheme() -> Option<u32> {
    use zbus::zvariant::{OwnedValue, Value};
    let conn = zbus::blocking::Connection::session().ok()?;
    let read = |method: &str| -> Option<OwnedValue> {
        conn.call_method(
            Some("org.freedesktop.portal.Desktop"),
            "/org/freedesktop/portal/desktop",
            Some("org.freedesktop.portal.Settings"),
            method,
            &("org.freedesktop.appearance", "color-scheme"),
        )
        .ok()?
        .body()
        .deserialize::<OwnedValue>()
        .ok()
    };
    // `ReadOne` (newer portals) returns the value; the older `Read` wraps
    // it in one more variant.
    let value = read("ReadOne").or_else(|| read("Read"))?;
    match &*value {
        Value::U32(n) => Some(*n),
        Value::Value(inner) => u32::try_from(&**inner).ok(),
        _ => None,
    }
}

/// Is the Steam client running? Steam writes its process ID to
/// `~/.steam/steam.pid`; check that process still exists.
pub fn steam_running() -> bool {
    let Some(home) = dirs::home_dir() else { return false };
    steam_running_in(&home.join(".steam/steam.pid"), std::path::Path::new("/proc"))
}

fn steam_running_in(pid_file: &std::path::Path, proc_dir: &std::path::Path) -> bool {
    std::fs::read_to_string(pid_file)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
        .filter(|&pid| pid != 0)
        .is_some_and(|pid| proc_dir.join(pid.to_string()).exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn steam_pid_file_must_point_at_a_live_process() {
        let dir = tempfile::tempdir().unwrap();
        let (pid_file, proc_dir) = (dir.path().join("steam.pid"), dir.path().join("proc"));
        std::fs::create_dir_all(proc_dir.join("4242")).unwrap();
        assert!(!super::steam_running_in(&pid_file, &proc_dir), "no pid file");
        std::fs::write(&pid_file, "4242\n").unwrap();
        assert!(super::steam_running_in(&pid_file, &proc_dir));
        std::fs::write(&pid_file, "999").unwrap();
        assert!(!super::steam_running_in(&pid_file, &proc_dir), "stale pid");
        std::fs::write(&pid_file, "garbage").unwrap();
        assert!(!super::steam_running_in(&pid_file, &proc_dir));
    }

    fn session(wayland: bool, desktop: &str) -> Session {
        Session {
            wayland,
            x11: !wayland,
            desktops: desktop
                .split(':')
                .map(str::to_string)
                .filter(|d| !d.is_empty())
                .collect(),
            forced: None,
        }
    }

    #[test]
    fn picks_the_driver_for_the_desktop() {
        let all = |_: &str| true;
        let none = |_: &str| false;
        assert_eq!(
            choose_driver(&session(true, "GNOME"), true, &none),
            Some(DriverKind::Gnome)
        );
        assert_eq!(
            choose_driver(&session(false, "UBUNTU:GNOME"), true, &none),
            Some(DriverKind::Gnome),
            "GNOME on X11 also uses Mutter"
        );
        assert_eq!(choose_driver(&session(true, "KDE"), false, &all), Some(DriverKind::Kde));
        assert_eq!(
            choose_driver(&session(false, "KDE"), false, &all),
            Some(DriverKind::X11),
            "Plasma on X11: xrandr"
        );
        assert_eq!(
            choose_driver(&session(true, "sway"), false, &all),
            Some(DriverKind::Wlroots)
        );
        assert_eq!(
            choose_driver(&session(true, "Hyprland"), false, &|p| p == "wlr-randr"),
            Some(DriverKind::Wlroots)
        );
        assert_eq!(
            choose_driver(&session(false, "XFCE"), false, &all),
            Some(DriverKind::X11)
        );
        assert_eq!(choose_driver(&session(true, "sway"), false, &none), None);
        assert_eq!(choose_driver(&session(false, "XFCE"), false, &none), None);
    }

    #[test]
    fn the_environment_can_force_a_driver() {
        let mut s = session(true, "GNOME");
        s.forced = Some("x11".into());
        assert_eq!(choose_driver(&s, true, &|_| false), Some(DriverKind::X11));
        s.forced = Some("kde".into());
        assert_eq!(choose_driver(&s, true, &|_| false), Some(DriverKind::Kde));
        s.forced = Some("nonsense".into());
        assert_eq!(choose_driver(&s, true, &|_| false), Some(DriverKind::Gnome));
    }

    #[test]
    fn tray_icon_contrasts_with_the_panel() {
        use crate::TrayIconStyle::{Black, White};
        assert_eq!(
            style_for(&session(true, "GNOME"), Some(2)),
            White,
            "GNOME's bar is dark"
        );
        assert_eq!(style_for(&session(true, "ubuntu:GNOME"), None), White);
        assert_eq!(style_for(&session(true, "KDE"), Some(2)), Black);
        assert_eq!(style_for(&session(true, "KDE"), Some(1)), White);
        assert_eq!(style_for(&session(false, "XFCE"), Some(0)), White);
        assert_eq!(style_for(&session(false, "XFCE"), None), White);
    }

    #[test]
    fn missing_tools_get_a_useful_hint() {
        assert!(no_driver_hint(&session(true, "KDE")).contains("kscreen-doctor"));
        assert!(no_driver_hint(&session(true, "sway")).contains("wlr-randr"));
        assert!(no_driver_hint(&session(false, "XFCE")).contains("xrandr"));
    }
}
