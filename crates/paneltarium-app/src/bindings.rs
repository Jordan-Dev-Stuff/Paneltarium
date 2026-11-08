//! Which global hotkey does what. The mapping is pure (and unit-tested);
//! [`HotkeyRegistry`] is the thin layer that registers it with the OS via the
//! cross-platform `global-hotkey` crate.

use std::str::FromStr;

use global_hotkey::GlobalHotKeyManager;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use pt_core::{Hotkey, Profile, Settings};

/// Something a hotkey can trigger.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    ApplyProfile(String),
    /// Duplicate on every monitor, falling back to extend.
    Rescue,
    Extend,
    /// Open the numbered profile overlay (roadmap step 6).
    Overlay,
}

impl Action {
    pub fn describe(&self) -> String {
        match self {
            Action::ApplyProfile(name) => format!("profile '{name}'"),
            Action::Rescue => "the rescue hotkey".into(),
            Action::Extend => "the extend-all hotkey".into(),
            Action::Overlay => "the overlay hotkey".into(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Binding {
    pub hotkey: Hotkey,
    pub action: Action,
}

/// All bindings for the current settings and profiles. App hotkeys come
/// first and win any clash; clashes are reported as warnings.
pub fn build(settings: &Settings, profiles: &[Profile]) -> (Vec<Binding>, Vec<String>) {
    let app = [
        (settings.duplicate_hotkey, Action::Rescue),
        (settings.extend_hotkey, Action::Extend),
        (settings.overlay_hotkey, Action::Overlay),
    ];
    let candidates = app
        .into_iter()
        .filter_map(|(hk, action)| hk.map(|hotkey| Binding { hotkey, action }))
        .chain(profiles.iter().filter_map(|p| {
            p.hotkey.map(|hotkey| Binding {
                hotkey,
                action: Action::ApplyProfile(p.name.clone()),
            })
        }));

    let mut bindings: Vec<Binding> = Vec::new();
    let mut warnings = Vec::new();
    for b in candidates {
        match bindings.iter().find(|existing| existing.hotkey == b.hotkey) {
            Some(existing) => warnings.push(format!(
                "{} is set for both {} and {}; using {}",
                b.hotkey,
                existing.action.describe(),
                b.action.describe(),
                existing.action.describe()
            )),
            None => bindings.push(b),
        }
    }
    (bindings, warnings)
}

/// Convert our OS-neutral hotkey into the `global-hotkey` crate's type.
pub fn to_global(hotkey: &Hotkey) -> Result<HotKey, String> {
    let code = Code::from_str(&hotkey.key.code_name())
        .map_err(|_| format!("{hotkey}: key '{}' is not supported", hotkey.key))?;
    let m = hotkey.modifiers;
    let mut mods = Modifiers::empty();
    for (on, flag) in [
        (m.ctrl, Modifiers::CONTROL),
        (m.alt, Modifiers::ALT),
        (m.shift, Modifiers::SHIFT),
        (m.win, Modifiers::META), // Win key (Super/Cmd elsewhere)
    ] {
        if on {
            mods |= flag;
        }
    }
    Ok(HotKey::new(Some(mods), code))
}

/// Owns the OS registrations. Must live on the thread running the event loop.
pub struct HotkeyRegistry {
    manager: GlobalHotKeyManager,
    registered: Vec<(HotKey, Action)>,
}

impl HotkeyRegistry {
    pub fn new() -> Result<Self, String> {
        let manager = GlobalHotKeyManager::new().map_err(|e| format!("could not start hotkeys: {e}"))?;
        Ok(HotkeyRegistry {
            manager,
            registered: Vec::new(),
        })
    }

    /// Replace every registration. Returns warnings for hotkeys that could
    /// not be registered (usually because another app already owns them).
    pub fn replace(&mut self, bindings: &[Binding]) -> Vec<String> {
        for (hk, _) in self.registered.drain(..) {
            if let Err(e) = self.manager.unregister(hk) {
                tracing::warn!("unregister {hk:?}: {e}");
            }
        }
        let mut warnings = Vec::new();
        for b in bindings {
            let result = to_global(&b.hotkey).and_then(|hk| {
                self.manager
                    .register(hk)
                    .map(|()| hk)
                    .map_err(|e| format!("{} ({}) is taken by another app: {e}", b.hotkey, b.action.describe()))
            });
            match result {
                Ok(hk) => {
                    tracing::info!("registered {} -> {:?}", b.hotkey, b.action);
                    self.registered.push((hk, b.action.clone()));
                }
                Err(w) => {
                    tracing::warn!("{w}");
                    warnings.push(w);
                }
            }
        }
        warnings
    }

    /// Register extra hotkeys for a short time (the overlay's digits).
    /// Combos this app already owns (e.g. a profile's Ctrl+Alt+1) are
    /// skipped: their events arrive anyway and the overlay claims them first.
    /// Returns the ones actually registered, to pass to `unregister_temporary`.
    pub fn register_temporary(&mut self, hotkeys: &[HotKey]) -> Vec<HotKey> {
        let mut added = Vec::new();
        for hk in hotkeys {
            if self.registered.iter().any(|(r, _)| r.id() == hk.id())
                || added.iter().any(|a: &HotKey| a.id() == hk.id())
            {
                continue;
            }
            match self.manager.register(*hk) {
                Ok(()) => added.push(*hk),
                Err(e) => tracing::debug!("temporary hotkey {hk:?} unavailable: {e}"),
            }
        }
        added
    }

    pub fn unregister_temporary(&mut self, hotkeys: &[HotKey]) {
        for hk in hotkeys {
            if let Err(e) = self.manager.unregister(*hk) {
                tracing::warn!("unregister temporary {hk:?}: {e}");
            }
        }
    }

    pub fn action_for(&self, id: u32) -> Option<&Action> {
        self.registered.iter().find(|(hk, _)| hk.id() == id).map(|(_, a)| a)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pt_core::testing::sample_layout;

    fn hk(s: &str) -> Hotkey {
        s.parse().unwrap()
    }

    fn profile(name: &str, hotkey: Option<&str>) -> Profile {
        let mut p = Profile::new(name, sample_layout()).unwrap();
        p.hotkey = hotkey.map(hk);
        p
    }

    #[test]
    fn default_settings_bind_rescue_and_overlay() {
        let (b, w) = build(&Settings::default(), &[]);
        assert!(w.is_empty());
        assert_eq!(
            b,
            [
                Binding {
                    hotkey: hk("Ctrl+Alt+Shift+D"),
                    action: Action::Rescue
                },
                Binding {
                    hotkey: hk("Ctrl+Alt+D"),
                    action: Action::Overlay
                },
            ]
        );
    }

    #[test]
    fn profiles_are_bound_and_clashes_reported() {
        let profiles = [
            profile("LG only", Some("Ctrl+Alt+1")),
            profile("TV only", Some("Ctrl+Alt+2")),
            profile("No key", None),
            profile("Clash", Some("Ctrl+Alt+D")),
        ];
        let (b, w) = build(&Settings::default(), &profiles);
        assert_eq!(b.len(), 4);
        assert!(b.contains(&Binding {
            hotkey: hk("Ctrl+Alt+2"),
            action: Action::ApplyProfile("TV only".into())
        }));
        assert_eq!(w.len(), 1);
        assert!(
            w[0].contains("profile 'Clash'") && w[0].contains("using the overlay hotkey"),
            "{w:?}"
        );
    }

    #[test]
    fn every_supported_key_converts() {
        let mut keys: Vec<String> = Vec::new();
        keys.extend(('A'..='Z').map(String::from));
        keys.extend((0..=9).map(|d| d.to_string()));
        keys.extend((1..=24).map(|n| format!("F{n}")));
        keys.extend((0..=9).map(|n| format!("Num{n}")));
        for named in [
            "Up",
            "Down",
            "Left",
            "Right",
            "Home",
            "End",
            "PageUp",
            "PageDown",
            "Insert",
            "Delete",
            "Space",
            "Minus",
            "Equal",
            "Comma",
            "Period",
            "Slash",
            "Backslash",
            "Semicolon",
            "Quote",
            "Backquote",
            "BracketLeft",
            "BracketRight",
        ] {
            keys.push(named.into());
        }
        for k in keys {
            let h = hk(&format!("Ctrl+Alt+Shift+Win+{k}"));
            let g = to_global(&h).unwrap_or_else(|e| panic!("{e}"));
            assert!(
                g.mods
                    .contains(Modifiers::CONTROL | Modifiers::ALT | Modifiers::SHIFT | Modifiers::META)
            );
        }
    }

    #[test]
    fn same_hotkey_gives_same_id() {
        let a = to_global(&hk("Ctrl+Alt+1")).unwrap();
        let b = to_global(&hk("alt+ctrl+1")).unwrap();
        let c = to_global(&hk("Ctrl+Alt+2")).unwrap();
        assert_eq!(a.id(), b.id());
        assert_ne!(a.id(), c.id());
    }
}
