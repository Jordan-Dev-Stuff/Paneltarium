//! Everything the manager window can do, without any UI. Each action
//! returns a message for the status bar (`Ok`) or an error message (`Err`).
//! The UI is a thin layer over this, and this is what the tests exercise.
//!
//! The manager only edits the profile/settings files (through `pt-core`);
//! the running background app notices and re-registers its hotkeys.

use pt_core::gamepad::GamepadChord;
use std::path::Path;

use pt_core::startup::StartupProfile;
use pt_core::{
    Autostart, AutostartStatus, Color, DisplayLayout, Hotkey, Modifiers, MonitorService, OverlayMode, Profile,
    Settings, slots,
};

pub type Msg = Result<String, String>;

/// One profile row, in overlay order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Row {
    /// Overlay number, or `None` if it didn't fit into 1–9.
    pub slot: Option<u8>,
    pub name: String,
    pub hotkey: Option<Hotkey>,
    pub summary: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppHotkey {
    Overlay,
    Rescue,
    Extend,
}

impl AppHotkey {
    pub const ALL: [AppHotkey; 3] = [AppHotkey::Overlay, AppHotkey::Rescue, AppHotkey::Extend];

    pub fn label(self) -> &'static str {
        match self {
            AppHotkey::Overlay => "Profile overlay",
            AppHotkey::Rescue => "Rescue (duplicate all)",
            AppHotkey::Extend => "Extend all",
        }
    }

    pub fn get(self, s: &Settings) -> Option<Hotkey> {
        match self {
            AppHotkey::Overlay => s.overlay_hotkey,
            AppHotkey::Rescue => s.duplicate_hotkey,
            AppHotkey::Extend => s.extend_hotkey,
        }
    }

    fn set(self, s: &mut Settings, h: Option<Hotkey>) {
        match self {
            AppHotkey::Overlay => s.overlay_hotkey = h,
            AppHotkey::Rescue => s.duplicate_hotkey = h,
            AppHotkey::Extend => s.extend_hotkey = h,
        }
    }
}

pub struct Model {
    svc: MonitorService,
    pub profiles: Vec<Profile>,
    pub settings: Settings,
    /// Current monitors, or why they couldn't be read.
    pub layout: Result<DisplayLayout, String>,
    autostart: Option<Box<dyn Autostart>>,
    /// "Start with Windows" state; `None` = not available here.
    pub autostart_status: Option<AutostartStatus>,
    /// Files that couldn't be read (shown above the profile list).
    pub problems: Vec<String>,
}

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// TOML errors span several lines (with a code snippet); one is enough here.
fn first_line(e: impl std::fmt::Display) -> String {
    e.to_string().lines().next().unwrap_or_default().to_string()
}

impl Model {
    /// `autostart` is `None` where start-at-sign-in isn't available.
    pub fn new(svc: MonitorService, autostart: Option<Box<dyn Autostart>>) -> Self {
        let mut m = Model {
            svc,
            profiles: Vec::new(),
            settings: Settings::default(),
            layout: Err("not loaded yet".into()),
            autostart,
            autostart_status: None,
            problems: Vec::new(),
        };
        m.refresh();
        m
    }

    /// Re-read profiles, settings and the current monitors.
    pub fn refresh(&mut self) {
        self.refresh_files();
        self.layout = self.svc.current_layout().map_err(err);
        self.autostart_status = self.autostart.as_ref().and_then(|a| a.status().ok());
    }

    /// Re-read only if the files changed in a way that matters (cheap; used
    /// on a timer so CLI edits show up). Returns true if anything changed.
    pub fn refresh_files(&mut self) -> bool {
        let mut problems = Vec::new();
        let profiles = match self.svc.store().list_checked() {
            Ok((profiles, bad)) => {
                problems.extend(bad.iter().map(|e| format!("Skipped a profile: {}", first_line(e))));
                profiles
            }
            Err(e) => {
                problems.push(format!("Couldn't read the profiles: {e}"));
                Vec::new()
            }
        };
        let settings = self.svc.settings().unwrap_or_else(|e| {
            problems.push(format!("Settings file unreadable, showing defaults: {e}"));
            Settings::default()
        });
        let changed = profiles != self.profiles || settings != self.settings || problems != self.problems;
        self.problems = problems;
        self.profiles = profiles;
        self.settings = settings;
        changed
    }

    pub fn overlay_mods(&self) -> Modifiers {
        self.settings
            .overlay_hotkey
            .unwrap_or_else(Hotkey::default_overlay)
            .modifiers
    }

    /// Profiles in overlay order (1–9), then any that didn't fit.
    pub fn rows(&self) -> Vec<Row> {
        let (assigned, _) = slots::assign(&self.profiles, self.overlay_mods());
        let row = |slot, p: &Profile| Row {
            slot,
            name: p.name.clone(),
            hotkey: p.hotkey,
            summary: p.layout.summary(),
        };
        let mut rows: Vec<Row> = assigned.iter().map(|&(n, i)| row(Some(n), &self.profiles[i])).collect();
        for (i, p) in self.profiles.iter().enumerate() {
            if !assigned.iter().any(|&(_, j)| j == i) {
                rows.push(row(None, p));
            }
        }
        rows
    }

    fn reserved(&self) -> Vec<(Hotkey, &'static str)> {
        self.settings.reserved_hotkeys()
    }

    fn done(&mut self, result: Msg) -> Msg {
        self.refresh_files();
        result
    }

    pub fn save_current(&mut self, name: &str, overwrite: bool) -> Msg {
        let r = self
            .svc
            .save_current(name, None, overwrite)
            .map(|p| format!("Saved '{}': {}", p.name, p.layout.summary()))
            .map_err(err);
        self.layout = self.svc.current_layout().map_err(err);
        self.done(r)
    }

    pub fn update_from_current(&mut self, name: &str) -> Msg {
        self.save_current(name, true).map(|m| m.replacen("Saved", "Updated", 1))
    }

    pub fn apply(&mut self, name: &str) -> Msg {
        let r = self
            .svc
            .apply_profile(name, false)
            .map(|r| {
                let mut m = format!("Applied '{}'", r.profile);
                if !r.differences.is_empty() {
                    m += &format!(" (note: {})", r.differences.join("; "));
                }
                m
            })
            .map_err(err);
        self.layout = self.svc.current_layout().map_err(err);
        r
    }

    pub fn rename(&mut self, old: &str, new: &str) -> Msg {
        let r = self
            .svc
            .rename_profile(old, new)
            .map(|p| format!("Renamed '{old}' to '{}'", p.name))
            .map_err(err);
        self.done(r)
    }

    pub fn delete(&mut self, name: &str) -> Msg {
        let r = self
            .svc
            .store()
            .delete(name)
            .map(|()| format!("Deleted '{name}'"))
            .map_err(err);
        self.done(r)
    }

    pub fn set_hotkey(&mut self, name: &str, hotkey: Option<Hotkey>) -> Msg {
        let r = self
            .svc
            .set_hotkey(name, hotkey)
            .map(|p| match p.hotkey {
                Some(h) => format!("'{}' hotkey set to {h}", p.name),
                None => format!("'{}' hotkey cleared", p.name),
            })
            .map_err(err);
        self.done(r)
    }

    /// Make `name` overlay number `n` (gives it the matching hotkey,
    /// swapping with whoever had it).
    pub fn move_to_slot(&mut self, name: &str, n: u8) -> Msg {
        let reserved = self.reserved();
        let r = slots::move_to_slot(self.svc.store(), name, n, self.overlay_mods(), &reserved)
            .map(|()| {
                format!(
                    "'{name}' is now number {n} ({})",
                    slots::slot_hotkey(self.overlay_mods(), n)
                )
            })
            .map_err(err);
        self.done(r)
    }

    pub fn set_app_hotkey(&mut self, which: AppHotkey, hotkey: Option<Hotkey>) -> Msg {
        if which == AppHotkey::Overlay && hotkey.is_none() {
            return Err("The overlay needs a hotkey (it's how you open it)".into());
        }
        let mut s = self.settings.clone();
        which.set(&mut s, hotkey);
        let r = s
            .check_conflicts(self.svc.store())
            .and_then(|()| self.svc.save_settings(&s))
            .map(|()| match hotkey {
                Some(h) => format!("{} hotkey set to {h}", which.label()),
                None => format!("{} hotkey cleared", which.label()),
            })
            .map_err(err);
        self.done(r)
    }
}

impl Model {
    /// Switch reading game controllers on/off. The button combination is kept.
    pub fn set_gamepad_enabled(&mut self, on: bool) -> Msg {
        let mut s = self.settings.clone();
        s.gamepad_enabled = on;
        let r = self
            .svc
            .save_settings(&s)
            .map(|()| {
                if on {
                    "Listening to game controllers".to_string()
                } else {
                    "Game controllers are off (your button combination is kept)".to_string()
                }
            })
            .map_err(err);
        self.done(r)
    }

    pub fn set_gamepad_chord(&mut self, chord: Option<GamepadChord>) -> Msg {
        let mut s = self.settings.clone();
        s.gamepad_overlay = chord.clone();
        let r = self
            .svc
            .save_settings(&s)
            .map(|()| match chord {
                Some(c) => format!("Controller overlay: hold {c}"),
                None => "Controller overlay combination cleared".to_string(),
            })
            .map_err(err);
        self.done(r)
    }
}

impl Model {
    /// Turn "Start with Windows" on/off. `app_exe` is the background app to
    /// start (normally `paneltarium-app.exe` next to the manager).
    pub fn set_autostart(&mut self, on: bool, app_exe: &Path) -> Msg {
        let Some(a) = &self.autostart else {
            return Err("Starting at sign-in isn't available on this system".into());
        };
        let r = if on {
            if !app_exe.exists() {
                Err(format!("{} not found (run `cargo build` first)", app_exe.display()))
            } else {
                a.enable(app_exe)
                    .map(|()| "Paneltarium will start when you sign in".to_string())
                    .map_err(err)
            }
        } else {
            a.disable()
                .map(|()| "Paneltarium won't start at sign-in".to_string())
                .map_err(err)
        };
        self.autostart_status = a.status().ok();
        r
    }

    pub fn set_startup_profile(&mut self, choice: StartupProfile) -> Msg {
        let mut s = self.settings.clone();
        s.startup_profile = choice.clone();
        let r = self
            .svc
            .save_settings(&s)
            .map(|()| match choice {
                StartupProfile::None => "At sign-in: monitors are left as Windows set them up".to_string(),
                StartupProfile::Last => "At sign-in: load the last used profile".to_string(),
                StartupProfile::Profile(n) => format!("At sign-in: load '{n}'"),
            })
            .map_err(err);
        self.done(r)
    }

    pub fn last_profile(&self) -> Option<String> {
        self.svc.last_profile()
    }

    /// How the keyboard-opened overlay stays open.
    pub fn set_overlay_mode(&mut self, mode: OverlayMode) -> Msg {
        let mut s = self.settings.clone();
        s.overlay_mode = mode;
        let r = self
            .svc
            .save_settings(&s)
            .map(|()| match mode {
                OverlayMode::Hold => "The overlay now closes when you let go of the hotkey's modifier keys".to_string(),
                OverlayMode::Toggle => "The overlay now stays open until you choose or press Esc".to_string(),
            })
            .map_err(err);
        self.done(r)
    }

    /// Save the accent colour (overlay and this window).
    pub fn set_accent(&mut self, color: Color) -> Msg {
        let mut s = self.settings.clone();
        s.accent_color = color;
        let r = self
            .svc
            .save_settings(&s)
            .map(|()| {
                if color == Color::DEFAULT_ACCENT {
                    format!("Accent colour reset to {color}")
                } else {
                    format!("Accent colour set to {color}")
                }
            })
            .map_err(err);
        self.done(r)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pt_core::AppPaths;
    use pt_core::mock::MockDisplayBackend;
    use pt_core::testing::sample_layout;

    fn model() -> (tempfile::TempDir, Model) {
        let dir = tempfile::tempdir().unwrap();
        let svc = MonitorService::new(
            Box::new(MockDisplayBackend::new(sample_layout())),
            AppPaths::new(dir.path()),
        );
        (dir, Model::new(svc, Some(Box::new(FakeAutostart::default()))))
    }

    /// Stands in for the registry / ~/.config in tests.
    #[derive(Default)]
    struct FakeAutostart(std::sync::Mutex<Option<std::path::PathBuf>>);

    impl Autostart for FakeAutostart {
        fn status(&self) -> pt_core::Result<AutostartStatus> {
            Ok(match *self.0.lock().unwrap() {
                Some(_) => AutostartStatus::On,
                None => AutostartStatus::Off,
            })
        }
        fn enable(&self, exe: &Path) -> pt_core::Result<()> {
            *self.0.lock().unwrap() = Some(exe.to_path_buf());
            Ok(())
        }
        fn disable(&self) -> pt_core::Result<()> {
            *self.0.lock().unwrap() = None;
            Ok(())
        }
    }

    #[test]
    fn autostart_and_startup_profile() {
        let (d, mut m) = model();
        assert_eq!(m.autostart_status, Some(AutostartStatus::Off));
        assert!(
            m.set_autostart(true, &d.path().join("missing.exe")).is_err(),
            "exe must exist"
        );
        let exe = d.path().join("paneltarium-app.exe");
        std::fs::write(&exe, "").unwrap();
        m.set_autostart(true, &exe).unwrap();
        assert_eq!(m.autostart_status, Some(AutostartStatus::On));
        m.set_autostart(false, &exe).unwrap();
        assert_eq!(m.autostart_status, Some(AutostartStatus::Off));

        m.save_current("Desk", false).unwrap();
        m.set_startup_profile(StartupProfile::Last).unwrap();
        assert_eq!(m.settings.startup_profile, StartupProfile::Last);
        assert_eq!(m.last_profile(), None);
        m.apply("Desk").unwrap();
        assert_eq!(m.last_profile().as_deref(), Some("Desk"));
        m.rename("Desk", "Work").unwrap();
        assert_eq!(m.last_profile().as_deref(), Some("Work"), "rename keeps 'last used'");
        m.set_startup_profile(StartupProfile::Profile("Work".into())).unwrap();
        assert_eq!(m.settings.startup_profile, StartupProfile::Profile("Work".into()));
    }

    fn hk(s: &str) -> Option<Hotkey> {
        Some(s.parse().unwrap())
    }

    #[test]
    fn save_apply_rename_delete() {
        let (_d, mut m) = model();
        assert!(m.layout.is_ok());
        assert!(m.save_current("Desk", false).unwrap().starts_with("Saved 'Desk'"));
        assert!(m.save_current("Desk", false).is_err(), "no silent overwrite");
        assert!(m.update_from_current("Desk").unwrap().starts_with("Updated 'Desk'"));
        assert_eq!(m.apply("Desk").unwrap(), "Applied 'Desk'");
        assert!(m.rename("Desk", "Work").is_ok());
        assert_eq!(m.rows()[0].name, "Work");
        assert!(m.delete("Work").is_ok());
        assert!(m.rows().is_empty());
        assert!(m.apply("Work").is_err());
    }

    #[test]
    fn rows_follow_overlay_numbers_and_move_swaps() {
        let (_d, mut m) = model();
        for n in ["Both", "LG only", "TV only"] {
            m.save_current(n, false).unwrap();
        }
        m.set_hotkey("TV only", hk("Ctrl+Alt+1")).unwrap();
        let order: Vec<_> = m.rows().into_iter().map(|r| (r.slot, r.name)).collect();
        assert_eq!(
            order,
            [
                (Some(1), "TV only".to_string()),
                (Some(2), "Both".to_string()),
                (Some(3), "LG only".to_string())
            ]
        );

        let msg = m.move_to_slot("LG only", 1).unwrap();
        assert!(msg.contains("Ctrl+Alt+1"), "{msg}");
        let rows = m.rows();
        assert_eq!((rows[0].slot, rows[0].name.as_str()), (Some(1), "LG only"));
        assert_eq!(rows[0].hotkey, hk("Ctrl+Alt+1"));
    }

    #[test]
    fn app_hotkeys_are_validated() {
        let (_d, mut m) = model();
        m.save_current("Desk", false).unwrap();
        m.set_hotkey("Desk", hk("Ctrl+Alt+1")).unwrap();

        assert!(m.set_app_hotkey(AppHotkey::Extend, hk("Ctrl+Alt+E")).is_ok());
        assert_eq!(m.settings.extend_hotkey, hk("Ctrl+Alt+E"));
        let e = m.set_app_hotkey(AppHotkey::Extend, hk("Ctrl+Alt+1")).unwrap_err();
        assert!(e.contains("profile 'Desk'"), "{e}");
        assert!(m.set_app_hotkey(AppHotkey::Overlay, None).is_err());
        assert!(m.set_app_hotkey(AppHotkey::Extend, None).is_ok());

        // Profile hotkeys can't take an app hotkey either.
        assert!(m.set_hotkey("Desk", hk("Ctrl+Alt+D")).is_err());
    }

    #[test]
    fn controller_toggle_keeps_combination() {
        let (_d, mut m) = model();
        assert!(m.settings.gamepad_enabled);
        m.set_gamepad_chord(Some("A+Start".parse().unwrap())).unwrap();
        let msg = m.set_gamepad_enabled(false).unwrap();
        assert!(msg.contains("kept"), "{msg}");
        assert!(!m.settings.gamepad_enabled);
        assert_eq!(m.settings.gamepad_overlay.as_ref().unwrap().to_string(), "A+Start");
        m.set_gamepad_enabled(true).unwrap();
        assert_eq!(
            m.settings.active_gamepad_chord().map(|c| c.to_string()).as_deref(),
            Some("A+Start")
        );
        m.set_gamepad_chord(None).unwrap();
        assert_eq!(m.settings.active_gamepad_chord(), None);
    }

    #[test]
    fn changing_overlay_modifiers_renumbers() {
        let (_d, mut m) = model();
        m.save_current("A", false).unwrap();
        m.save_current("B", false).unwrap();
        m.set_hotkey("B", hk("Ctrl+Shift+1")).unwrap();
        assert_eq!(
            m.rows()[0].name,
            "A",
            "Ctrl+Shift+1 doesn't pin with a Ctrl+Alt overlay"
        );
        m.set_app_hotkey(AppHotkey::Overlay, hk("Ctrl+Shift+D")).unwrap();
        assert_eq!(m.rows()[0].name, "B", "now it does");
    }
}
