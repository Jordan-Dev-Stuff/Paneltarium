use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Error, Result};
use crate::gamepad::GamepadChord;
use crate::hotkey::Hotkey;
use crate::paths::write_atomic;
use crate::profile::ProfileStore;
use crate::startup::StartupProfile;
use crate::theme::{Color, Theme};

/// App-wide settings, stored in `settings.toml`.
///
/// A setting that is switched off is written as `""` rather than left out:
/// a missing key means "use the default", so leaving it out would bring the
/// default back on the next load.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Rescue hotkey: duplicate the picture on every monitor.
    #[serde(with = "empty_is_none")]
    pub duplicate_hotkey: Option<Hotkey>,
    /// Optional hotkey: extend the desktop across every monitor.
    #[serde(with = "empty_is_none")]
    pub extend_hotkey: Option<Hotkey>,
    /// Hotkey that opens the numbered profile overlay.
    #[serde(with = "empty_is_none")]
    pub overlay_hotkey: Option<Hotkey>,
    /// Controller buttons held together to open the overlay.
    #[serde(with = "empty_is_none")]
    pub gamepad_overlay: Option<GamepadChord>,
    /// Master switch for game controllers. Off = controllers aren't read at
    /// all; `gamepad_overlay` is kept for when it's switched back on.
    pub gamepad_enabled: bool,
    /// What to do with the monitors when Paneltarium starts at sign-in.
    pub startup_profile: StartupProfile,
    /// Accent colour of the overlay and profile manager, `#RRGGBB`.
    pub accent_color: Color,
    /// How the keyboard-opened overlay stays open.
    pub overlay_mode: OverlayMode,
}

/// How the profile overlay stays open after its keyboard hotkey.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OverlayMode {
    /// Keep holding the hotkey's modifier keys (e.g. Ctrl+Alt); letting go
    /// cancels. The last key (e.g. D) can be released.
    #[default]
    Hold,
    /// Stays open without holding anything until a profile is chosen, Esc is
    /// pressed, the hotkey is pressed again, or it times out.
    Toggle,
}

impl std::fmt::Display for OverlayMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            OverlayMode::Hold => "hold",
            OverlayMode::Toggle => "toggle",
        })
    }
}

impl std::str::FromStr for OverlayMode {
    type Err = String;

    fn from_str(s: &str) -> std::result::Result<Self, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "hold" => Ok(OverlayMode::Hold),
            "toggle" | "stay" | "stay-open" => Ok(OverlayMode::Toggle),
            _ => Err(format!("'{s}' isn't an overlay mode; use hold or toggle")),
        }
    }
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            duplicate_hotkey: Some(Hotkey::default_duplicate()),
            extend_hotkey: None,
            overlay_hotkey: Some(Hotkey::default_overlay()),
            gamepad_overlay: Some(GamepadChord::default_overlay()),
            gamepad_enabled: true,
            startup_profile: StartupProfile::None,
            accent_color: Color::DEFAULT_ACCENT,
            overlay_mode: OverlayMode::Hold,
        }
    }
}

impl Settings {
    /// Load settings; a missing file means defaults.
    pub fn load(path: &Path) -> Result<Self> {
        match std::fs::read_to_string(path) {
            Ok(text) => toml::from_str(&text).map_err(|e| Error::Parse {
                path: path.to_path_buf(),
                message: e.to_string(),
            }),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Settings::default()),
            Err(e) => Err(Error::io(path, e)),
        }
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self).expect("settings always serialise");
        write_atomic(path, &text)
    }

    /// The overlay/manager colours for the chosen accent.
    pub fn theme(&self) -> Theme {
        Theme::new(self.accent_color)
    }

    /// The controller combination to listen for, or `None` if controllers
    /// are switched off or no combination is set.
    pub fn active_gamepad_chord(&self) -> Option<&GamepadChord> {
        self.gamepad_overlay.as_ref().filter(|_| self.gamepad_enabled)
    }

    /// Hotkeys reserved by the app itself, with a label for conflict messages.
    pub fn reserved_hotkeys(&self) -> Vec<(Hotkey, &'static str)> {
        [
            (self.duplicate_hotkey, "the duplicate-all hotkey"),
            (self.extend_hotkey, "the extend-all hotkey"),
            (self.overlay_hotkey, "the overlay hotkey"),
        ]
        .into_iter()
        .filter_map(|(hk, label)| hk.map(|h| (h, label)))
        .collect()
    }
}

impl Settings {
    /// App hotkeys must not collide with each other or with profile hotkeys.
    pub fn check_conflicts(&self, store: &ProfileStore) -> Result<()> {
        let reserved = self.reserved_hotkeys();
        for (i, (a, la)) in reserved.iter().enumerate() {
            if let Some((_, lb)) = reserved[i + 1..].iter().find(|(b, _)| b == a) {
                return Err(Error::HotkeyConflict {
                    hotkey: a.to_string(),
                    used_by: format!("both {la} and {lb}"),
                });
            }
            if let Some(p) = store.find_by_hotkey(*a)? {
                return Err(Error::HotkeyConflict {
                    hotkey: a.to_string(),
                    used_by: format!("profile '{}'", p.name),
                });
            }
        }
        Ok(())
    }
}

/// `Option<T>` stored as text, with `""` for `None` (TOML has no null).
mod empty_is_none {
    use std::fmt::Display;
    use std::str::FromStr;

    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<T: Display, S: Serializer>(v: &Option<T>, s: S) -> Result<S::Ok, S::Error> {
        match v {
            Some(v) => s.collect_str(v),
            None => s.serialize_str(""),
        }
    }

    pub fn deserialize<'de, T, D>(d: D) -> Result<Option<T>, D::Error>
    where
        T: FromStr,
        T::Err: Display,
        D: Deserializer<'de>,
    {
        let text = String::deserialize(d)?;
        match text.trim() {
            "" => Ok(None),
            t => t.parse().map(Some).map_err(serde::de::Error::custom),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_file_gives_defaults_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let s = Settings::load(&path).unwrap();
        assert_eq!(s, Settings::default());
        assert_eq!(s.reserved_hotkeys().len(), 2);

        let changed = Settings {
            extend_hotkey: Some("Ctrl+Alt+Shift+E".parse().unwrap()),
            ..s
        };
        changed.save(&path).unwrap();
        assert_eq!(Settings::load(&path).unwrap(), changed);
    }

    #[test]
    fn conflicts_are_detected() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(dir.path());
        let mut p = crate::Profile::new("A", crate::testing::sample_layout()).unwrap();
        p.hotkey = Some("Ctrl+Alt+1".parse().unwrap());
        store.save(&p, false, &[]).unwrap();

        assert!(Settings::default().check_conflicts(&store).is_ok());
        let clash_profile = Settings {
            extend_hotkey: Some("Ctrl+Alt+1".parse().unwrap()),
            ..Settings::default()
        };
        let e = clash_profile.check_conflicts(&store).unwrap_err().to_string();
        assert!(e.contains("profile 'A'"), "{e}");
        let clash_self = Settings {
            extend_hotkey: Settings::default().overlay_hotkey,
            ..Settings::default()
        };
        let e = clash_self.check_conflicts(&store).unwrap_err().to_string();
        assert!(e.contains("both"), "{e}");
    }

    #[test]
    fn cleared_settings_stay_cleared() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let cleared = Settings {
            duplicate_hotkey: None,
            overlay_hotkey: None,
            gamepad_overlay: None,
            ..Settings::default()
        };
        cleared.save(&path).unwrap();
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("duplicate_hotkey = \"\""), "{text}");
        assert_eq!(Settings::load(&path).unwrap(), cleared);
    }

    #[test]
    fn old_settings_files_get_the_controller_default() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "overlay_hotkey = \"Ctrl+Alt+D\"").unwrap();
        let s = Settings::load(&path).unwrap();
        assert_eq!(s.gamepad_overlay, Some(GamepadChord::default_overlay()));
        assert!(s.gamepad_enabled);
    }

    #[test]
    fn switching_controllers_off_keeps_the_combination() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        let off = Settings {
            gamepad_enabled: false,
            gamepad_overlay: Some("A+B".parse().unwrap()),
            ..Settings::default()
        };
        off.save(&path).unwrap();
        let back = Settings::load(&path).unwrap();
        assert!(!back.gamepad_enabled);
        assert_eq!(back.gamepad_overlay.as_ref().unwrap().to_string(), "A+B");
        assert_eq!(back.active_gamepad_chord(), None);
    }

    #[test]
    fn accent_colour_round_trips_and_defaults() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "gamepad_enabled = true").unwrap();
        assert_eq!(Settings::load(&path).unwrap().accent_color, Color::DEFAULT_ACCENT);
        let green = Settings {
            accent_color: "#22C55E".parse().unwrap(),
            ..Settings::default()
        };
        green.save(&path).unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("accent_color = \"#22C55E\"")
        );
        assert_eq!(Settings::load(&path).unwrap(), green);
        std::fs::write(&path, "accent_color = \"blue-ish\"").unwrap();
        assert!(Settings::load(&path).is_err());
    }

    #[test]
    fn overlay_mode_defaults_to_hold_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "gamepad_enabled = true").unwrap();
        assert_eq!(Settings::load(&path).unwrap().overlay_mode, OverlayMode::Hold);
        let s = Settings {
            overlay_mode: OverlayMode::Toggle,
            ..Settings::default()
        };
        s.save(&path).unwrap();
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("overlay_mode = \"toggle\"")
        );
        assert_eq!(Settings::load(&path).unwrap(), s);
        assert_eq!("Stay-Open".parse::<OverlayMode>().unwrap(), OverlayMode::Toggle);
        assert!("sometimes".parse::<OverlayMode>().is_err());
    }

    #[test]
    fn bad_file_reports_path() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("settings.toml");
        std::fs::write(&path, "duplicate_hotkey = \"Banana\"").unwrap();
        let err = Settings::load(&path).unwrap_err().to_string();
        assert!(err.contains("settings.toml"), "{err}");
    }
}
