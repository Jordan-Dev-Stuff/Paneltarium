//! The tray icon: shows the app is running, reminds you of the overlay and
//! rescue hotkeys, has a "Listen to game controllers" switch, and offers
//! "Open profile manager" and "Quit".
//!
//! The menu contents are built by the pure [`lines`] function (tested);
//! [`Tray`] turns them into a real `tray-icon` menu.

use pt_core::{AutostartStatus, Hotkey};
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
        out.push(Line::Info(
            "⚠ Start with Windows is switched off in Task Manager".into(),
        ));
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

pub struct Tray {
    icon: TrayIcon,
}

impl Tray {
    pub fn new(status: &Status) -> Result<Self, String> {
        let icon = TrayIconBuilder::new()
            .with_icon(app_icon())
            .with_tooltip(tooltip(status))
            .with_menu(Box::new(build_menu(status)?))
            .build()
            .map_err(|e| format!("could not create tray icon: {e}"))?;
        Ok(Tray { icon })
    }

    pub fn update(&self, status: &Status) {
        if let Err(e) = self.icon.set_tooltip(Some(tooltip(status))) {
            tracing::warn!("tray tooltip: {e}");
        }
        match build_menu(status) {
            Ok(menu) => self.icon.set_menu(Some(Box::new(menu))),
            Err(e) => tracing::warn!("tray menu: {e}"),
        }
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
                "Start with Windows",
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
/// tile), which reads on both light and dark taskbars.
pub fn app_icon() -> Icon {
    Icon::from_rgba(pt_brand::icon_rgba(ICON_SIZE), ICON_SIZE, ICON_SIZE).expect("icon buffer has the right size")
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
                .any(|x| matches!(x, Line::Info(t) if t.contains("Task Manager")))
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
        let _ = app_icon();
    }
}
