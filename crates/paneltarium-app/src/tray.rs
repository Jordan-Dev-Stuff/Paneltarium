//! The tray icon: shows the app is running, reminds you of the overlay and
//! rescue hotkeys, has a "Listen to game controllers" switch, and offers
//! "Open profile manager" and "Quit".
//!
//! The menu contents are built by the pure [`lines`] function (tested);
//! [`Tray`] turns them into a real `tray-icon` menu.
//!
//! Linux: the icon and menu are GTK, and GTK needs its main loop running to
//! answer the desktop at any time (open the menu, notice a tray area that
//! appears later, e.g. GNOME's extension being switched on). So the tray
//! lives on its own thread running `gtk::main`, and [`Tray`] sends it
//! updates. Clicks still arrive through `MenuEvent`'s handler.

use pt_core::{AutostartStatus, Hotkey, text};
use pt_platform::TrayIconStyle;
use tray_icon::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tray_icon::{Icon, TrayIcon, TrayIconBuilder};

pub const QUIT_ID: &str = "quit";
pub const MANAGER_ID: &str = "manager";
pub const GAMEPAD_ID: &str = "gamepad";
pub const AUTOSTART_ID: &str = "autostart";

/// Everything the tray displays.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Status {
    pub overlay: Option<Hotkey>,
    pub rescue: Option<Hotkey>,
    /// Result of the last hotkey action, e.g. "Applied 'TV only'".
    pub last: Option<(bool, String)>,
    /// Hotkeys that could not be registered, config problems, ...
    pub warnings: Vec<String>,
    /// Game controllers switched on?
    pub gamepad_enabled: bool,
    /// Controller combination that opens the overlay, e.g. "A+LB+RB+LT+RT".
    pub gamepad_chord: Option<String>,
    /// "Start with Windows" state (`None` = unavailable on this OS).
    pub autostart: Option<AutostartStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Line {
    /// Grey, not clickable.
    Info(String),
    Separator,
    /// "Open profile manager…".
    Manager,
    /// Checkbox: listen to game controllers.
    Gamepad {
        checked: bool,
    },
    /// Checkbox: start with Windows.
    Autostart {
        checked: bool,
    },
    Quit,
}

const MAX_LINE: usize = 90;

fn clip(s: &str) -> String {
    if s.chars().count() <= MAX_LINE {
        s.to_string()
    } else {
        s.chars().take(MAX_LINE - 1).collect::<String>() + "…"
    }
}

fn hotkey_text(h: Option<Hotkey>) -> String {
    h.map(|h| h.to_string()).unwrap_or_else(|| "not set".into())
}

pub fn lines(status: &Status) -> Vec<Line> {
    let mut out = vec![
        Line::Info("Paneltarium is running".into()),
        Line::Info(format!("Profile overlay: {}", hotkey_text(status.overlay))),
        Line::Info(format!("Rescue (duplicate all): {}", hotkey_text(status.rescue))),
        Line::Info(match (&status.gamepad_chord, status.gamepad_enabled) {
            (Some(c), true) => format!("Controller overlay: hold {c}"),
            (Some(_), false) => "Controller overlay: off".into(),
            (None, _) => "Controller overlay: not set".into(),
        }),
    ];
    if let Some((ok, msg)) = &status.last {
        let mark = if *ok { "Last" } else { "⚠ Failed" };
        out.push(Line::Info(clip(&format!("{mark}: {msg}"))));
    }
    out.extend(status.warnings.iter().map(|w| Line::Info(clip(&format!("⚠ {w}")))));
    if status.autostart == Some(AutostartStatus::DisabledInSystem) {
        out.push(Line::Info(format!(
            "⚠ {} is switched off in {}",
            text::START_AT_SIGN_IN,
            text::STARTUP_SETTINGS
        )));
    }
    out.extend([
        Line::Separator,
        Line::Gamepad {
            checked: status.gamepad_enabled,
        },
    ]);
    if status.autostart.is_some() {
        out.push(Line::Autostart {
            checked: status.autostart == Some(AutostartStatus::On),
        });
    }
    out.extend([Line::Manager, Line::Separator, Line::Quit]);
    out
}

/// Windows limits tray tooltips to 127 characters.
pub fn tooltip(status: &Status) -> String {
    let mut t = format!("Paneltarium\nOverlay: {}", hotkey_text(status.overlay));
    if !status.warnings.is_empty() {
        t += &format!("\n⚠ {} problem(s), see menu", status.warnings.len());
    }
    t.chars().take(127).collect()
}

fn create(status: &Status, style: TrayIconStyle) -> Result<TrayIcon, String> {
    TrayIconBuilder::new()
        .with_icon(app_icon(style))
        .with_tooltip(tooltip(status))
        .with_menu(Box::new(build_menu(status)?))
        .build()
        .map_err(|e| format!("could not create tray icon: {e}"))
}

fn update(icon: &TrayIcon, status: &Status) {
    if let Err(e) = icon.set_tooltip(Some(tooltip(status))) {
        tracing::warn!("tray tooltip: {e}");
    }
    match build_menu(status) {
        Ok(menu) => icon.set_menu(Some(Box::new(menu))),
        Err(e) => tracing::warn!("tray menu: {e}"),
    }
}

/// The icon; dropping it removes it.
#[cfg(not(target_os = "linux"))]
pub struct Tray {
    icon: TrayIcon,
}

#[cfg(not(target_os = "linux"))]
impl Tray {
    /// Must be called on the event-loop thread (Windows requires it).
    pub fn new(status: &Status) -> Result<Self, String> {
        Ok(Tray {
            icon: create(status, pt_platform::tray_icon_style())?,
        })
    }

    pub fn update(&self, status: &Status) {
        update(&self.icon, status);
    }
}

/// The icon, on the GTK thread; dropping this removes it.
#[cfg(target_os = "linux")]
pub struct Tray {
    _private: (),
}

#[cfg(target_os = "linux")]
thread_local! {
    /// The icon, owned by the GTK thread.
    static ICON: std::cell::RefCell<Option<TrayIcon>> = const { std::cell::RefCell::new(None) };
}

#[cfg(target_os = "linux")]
impl Tray {
    /// Starts the GTK thread with the icon on it.
    pub fn new(status: &Status) -> Result<Self, String> {
        use std::sync::mpsc;
        let (tx, rx) = mpsc::channel();
        let status = status.clone();
        let style = pt_platform::tray_icon_style();
        tracing::info!("tray icon: {style:?}");
        std::thread::Builder::new()
            .name("tray (gtk)".into())
            .spawn(move || {
                // This thread owns GTK (and GLib's default main context).
                let created = gtk::init()
                    .map_err(|e| format!("GTK: {e}"))
                    .and_then(|()| create(&status, style));
                match created {
                    Ok(icon) => {
                        ICON.with(|i| *i.borrow_mut() = Some(icon));
                        let _ = tx.send(Ok(()));
                        gtk::main();
                    }
                    Err(e) => {
                        let _ = tx.send(Err(e));
                    }
                }
            })
            .map_err(|e| format!("tray thread: {e}"))?;
        rx.recv()
            .unwrap_or_else(|_| Err("the tray thread stopped".into()))
            .map(|()| Tray { _private: () })
    }

    pub fn update(&self, status: &Status) {
        let status = status.clone();
        gtk::glib::MainContext::default().invoke(move || {
            ICON.with(|i| {
                if let Some(icon) = i.borrow().as_ref() {
                    update(icon, &status);
                }
            });
        });
    }
}

#[cfg(target_os = "linux")]
impl Drop for Tray {
    /// Remove the icon and stop GTK; waits briefly so the icon is gone
    /// before the app exits.
    fn drop(&mut self) {
        let (tx, rx) = std::sync::mpsc::channel();
        gtk::glib::MainContext::default().invoke(move || {
            ICON.with(|i| i.borrow_mut().take());
            gtk::main_quit();
            let _ = tx.send(());
        });
        let _ = rx.recv_timeout(std::time::Duration::from_millis(500));
    }
}

fn build_menu(status: &Status) -> Result<Menu, String> {
    let menu = Menu::new();
    for line in lines(status) {
        let result = match line {
            Line::Info(text) => menu.append(&MenuItem::new(text, false, None)),
            Line::Separator => menu.append(&PredefinedMenuItem::separator()),
            Line::Manager => menu.append(&MenuItem::with_id(MANAGER_ID, "Open profile manager…", true, None)),
            Line::Autostart { checked } => menu.append(&CheckMenuItem::with_id(
                AUTOSTART_ID,
                text::START_AT_SIGN_IN,
                true,
                checked,
                None,
            )),
            Line::Gamepad { checked } => menu.append(&CheckMenuItem::with_id(
                GAMEPAD_ID,
                "Listen to game controllers",
                true,
                checked,
                None,
            )),
            Line::Quit => menu.append(&MenuItem::with_id(QUIT_ID, "Quit Paneltarium", true, None)),
        };
        result.map_err(|e| e.to_string())?;
    }
    Ok(menu)
}

const ICON_SIZE: u32 = 32;

/// The tray icon: the app icon (a planet inside a display on a deep-space
/// tile), which reads on both light and dark taskbars; or, where panel
/// icons are one colour (Linux desktops), the display and planet alone in
/// white or black.
pub fn app_icon(style: TrayIconStyle) -> Icon {
    let rgba = match style {
        TrayIconStyle::Colour => pt_brand::icon_rgba(ICON_SIZE),
        TrayIconStyle::White => pt_brand::symbolic_rgba(ICON_SIZE, pt_brand::WHITE),
        TrayIconStyle::Black => pt_brand::symbolic_rgba(ICON_SIZE, pt_brand::BLACK),
    };
    Icon::from_rgba(rgba, ICON_SIZE, ICON_SIZE).expect("icon buffer has the right size")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn status() -> Status {
        Status {
            overlay: Some("Ctrl+Alt+D".parse().unwrap()),
            rescue: Some("Ctrl+Alt+Shift+D".parse().unwrap()),
            ..Default::default()
        }
    }

    #[test]
    fn menu_shows_hotkeys_and_quit() {
        let l = lines(&status());
        assert_eq!(l[0], Line::Info("Paneltarium is running".into()));
        assert_eq!(l[1], Line::Info("Profile overlay: Ctrl+Alt+D".into()));
        assert_eq!(l[2], Line::Info("Rescue (duplicate all): Ctrl+Alt+Shift+D".into()));
        assert_eq!(l.last(), Some(&Line::Quit));
        assert!(l.contains(&Line::Manager));
        assert!(l.contains(&Line::Gamepad { checked: false }));
        assert_eq!(l[3], Line::Info("Controller overlay: not set".into()));
    }

    #[test]
    fn menu_shows_autostart_state() {
        let mut s = status();
        assert!(
            !lines(&s).iter().any(|l| matches!(l, Line::Autostart { .. })),
            "hidden if unavailable"
        );
        s.autostart = Some(AutostartStatus::On);
        assert!(lines(&s).contains(&Line::Autostart { checked: true }));
        s.autostart = Some(AutostartStatus::DisabledInSystem);
        let l = lines(&s);
        assert!(l.contains(&Line::Autostart { checked: false }));
        assert!(
            l.iter()
                .any(|x| matches!(x, Line::Info(t) if t.contains(text::STARTUP_SETTINGS)))
        );
    }

    #[test]
    fn menu_shows_controller_state() {
        let mut s = status();
        s.gamepad_chord = Some("A+LB+RB+LT+RT".into());
        s.gamepad_enabled = true;
        let l = lines(&s);
        assert_eq!(l[3], Line::Info("Controller overlay: hold A+LB+RB+LT+RT".into()));
        assert!(l.contains(&Line::Gamepad { checked: true }));
        s.gamepad_enabled = false;
        let l = lines(&s);
        assert_eq!(l[3], Line::Info("Controller overlay: off".into()));
        assert!(l.contains(&Line::Gamepad { checked: false }));
    }

    #[test]
    fn menu_shows_last_result_and_warnings() {
        let mut s = status();
        s.overlay = None;
        s.last = Some((false, "Couldn't apply 'TV only': boom".into()));
        s.warnings = vec![
            "Ctrl+Alt+1 (profile 'LG only') is taken by another app".into(),
            "x".repeat(200),
        ];
        let l = lines(&s);
        assert_eq!(l[1], Line::Info("Profile overlay: not set".into()));
        assert_eq!(l[4], Line::Info("⚠ Failed: Couldn't apply 'TV only': boom".into()));
        assert!(matches!(&l[5], Line::Info(t) if t.contains("taken by another app")));
        assert!(matches!(&l[6], Line::Info(t) if t.chars().count() == MAX_LINE && t.ends_with('…')));
    }

    #[test]
    fn tooltip_fits_windows_limit() {
        let mut s = status();
        assert_eq!(tooltip(&s), "Paneltarium\nOverlay: Ctrl+Alt+D");
        s.warnings = vec!["a".into(), "b".into()];
        assert!(tooltip(&s).contains("2 problem(s)"));
        assert!(tooltip(&s).chars().count() <= 127);
    }

    #[test]
    fn tray_icon_builds() {
        for style in [TrayIconStyle::Colour, TrayIconStyle::White, TrayIconStyle::Black] {
            let _ = app_icon(style);
        }
    }
}
