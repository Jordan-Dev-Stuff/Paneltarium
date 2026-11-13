//! Which global hotkey does what. The mapping is pure (and unit-tested);
//! [`HotkeyRegistry`] is the thin layer that registers it with the OS: via
//! the cross-platform `global-hotkey` crate (Windows, X11), or on Wayland by
//! handing the list to the desktop (`pt_core::ShortcutService`, the
//! GlobalShortcuts portal), whose events arrive with the same hotkey ids.

use std::str::FromStr;

use global_hotkey::GlobalHotKeyManager;
use global_hotkey::hotkey::{Code, HotKey, Modifiers};
use pt_core::{Hotkey, Profile, Settings, Shortcut, ShortcutService};

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

/// The id a hotkey's events carry, whichever way it was registered.
pub fn hotkey_id(hotkey: &Hotkey) -> Option<u32> {
    to_global(hotkey).ok().map(|h| h.id())
}

/// What the desktop is asked for on Wayland: every binding, plus `extra`
/// keys that are only meaningful while the overlay is open (they can't be
/// bound for just that moment, since the desktop may ask the user each
/// time). One entry per hotkey; bindings win.
pub fn shortcuts(bindings: &[Binding], extra: &[(Hotkey, String)]) -> Vec<Shortcut> {
    let mut out: Vec<Shortcut> = Vec::new();
    let all = bindings
        .iter()
        .map(|b| (b.hotkey, sentence(&b.action.describe())))
        .chain(extra.iter().cloned());
    for (hotkey, description) in all {
        let Some(id) = hotkey_id(&hotkey) else { continue };
        let id = id.to_string();
        if !out.iter().any(|s| s.id == id) {
            out.push(Shortcut {
                id,
                description,
                hotkey,
            });
        }
    }
    out
}

/// "profile 'TV'" → "Apply profile 'TV'", "the overlay hotkey" → "Show
/// the profile list", ...: how it reads in the desktop's shortcut list.
fn sentence(describe: &str) -> String {
    match describe {
        "the overlay hotkey" => "Show the profile list".into(),
        "the rescue hotkey" => "Same picture on every monitor (rescue)".into(),
        "the extend-all hotkey" => "Turn every monitor on, side by side".into(),
        other => format!("Apply {other}"),
    }
}

enum Os {
    /// Grab keys directly (Windows, X11).
    Grab(GlobalHotKeyManager),
    /// Ask the desktop (Wayland).
    Desktop(Box<dyn ShortcutService>),
    /// Neither works here.
    Off,
}

/// Owns the OS registrations. Must live on the thread running the event loop.
pub struct HotkeyRegistry {
    os: Os,
    registered: Vec<(HotKey, Action)>,
}

impl HotkeyRegistry {
    /// `desktop`: the desktop's shortcut service where apps can't grab keys.
    pub fn new(desktop: Option<Box<dyn ShortcutService>>) -> Result<Self, String> {
        let os = match desktop {
            Some(service) => Os::Desktop(service),
            None => Os::Grab(GlobalHotKeyManager::new().map_err(|e| format!("could not start hotkeys: {e}"))?),
        };
        Ok(HotkeyRegistry {
            os,
            registered: Vec::new(),
        })
    }

    /// No hotkeys at all (the tray and controllers still work).
    pub fn disabled() -> Self {
        HotkeyRegistry {
            os: Os::Off,
            registered: Vec::new(),
        }
    }

    /// Replace every registration. Returns warnings for hotkeys that could
    /// not be registered (usually because another app already owns them).
    /// `overlay_keys` are the overlay's digits etc.: registered only while
    /// it's open where keys can be grabbed, but bound up front with a desktop
    /// shortcut service.
    pub fn replace(&mut self, bindings: &[Binding], overlay_keys: &[(Hotkey, String)]) -> Vec<String> {
        let manager = match &self.os {
            Os::Grab(m) => m,
            Os::Desktop(service) => {
                self.registered = bindings
                    .iter()
                    .filter_map(|b| Some((to_global(&b.hotkey).ok()?, b.action.clone())))
                    .collect();
                let list = shortcuts(bindings, overlay_keys);
                tracing::info!("asking the desktop for {} shortcut(s)", list.len());
                service.bind(list);
                return Vec::new(); // the outcome arrives as a ShortcutEvent
            }
            Os::Off => return Vec::new(),
        };
        for (hk, _) in self.registered.drain(..) {
            if let Err(e) = manager.unregister(hk) {
                tracing::warn!("unregister {hk:?}: {e}");
            }
        }
        let mut warnings = Vec::new();
        for b in bindings {
            let result = to_global(&b.hotkey).and_then(|hk| {
                manager
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
        let Os::Grab(manager) = &self.os else {
            return Vec::new(); // bound up front, see `replace`
        };
        let mut added = Vec::new();
        for hk in hotkeys {
            if self.registered.iter().any(|(r, _)| r.id() == hk.id())
                || added.iter().any(|a: &HotKey| a.id() == hk.id())
            {
                continue;
            }
            match manager.register(*hk) {
                Ok(()) => added.push(*hk),
                Err(e) => tracing::debug!("temporary hotkey {hk:?} unavailable: {e}"),
            }
        }
        added
    }

    pub fn unregister_temporary(&mut self, hotkeys: &[HotKey]) {
        let Os::Grab(manager) = &self.os else { return };
        for hk in hotkeys {
            if let Err(e) = manager.unregister(*hk) {
                tracing::warn!("unregister temporary {hk:?}: {e}");
            }
        }
    }

    pub fn action_for(&self, id: u32) -> Option<&Action> {
        self.registered.iter().find(|(hk, _)| hk.id() == id).map(|(_, a)| a)
    }
}

/// Turns a held hotkey into one press. Windows repeats a held hotkey
/// (about 30×/s after half a second) and those repeats must not toggle the
/// overlay or re-apply a profile. A hotkey counts again once its key was let
/// go; if the release is never reported, after a second without repeats.
#[derive(Debug, Default)]
pub struct RepeatFilter {
    down: std::collections::HashMap<u32, std::time::Instant>,
}

impl RepeatFilter {
    const STUCK: std::time::Duration = std::time::Duration::from_secs(1);

    /// A press of hotkey `id` at `now`: should it do anything?
    pub fn press(&mut self, id: u32, now: std::time::Instant) -> bool {
        match self.down.insert(id, now) {
            Some(prev) => now.duration_since(prev) >= Self::STUCK,
            None => true,
        }
    }

    pub fn release(&mut self, id: u32) {
        self.down.remove(&id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pt_core::testing::sample_layout;
    use std::time::{Duration, Instant};

    #[test]
    fn held_hotkey_acts_once() {
        let mut f = RepeatFilter::default();
        let t0 = Instant::now();
        let ms = |n| t0 + Duration::from_millis(n);
        assert!(f.press(7, t0), "first press acts");
        // Auto-repeat while held: 500 ms delay, then every ~33 ms for 3 s.
        assert!(!f.press(7, ms(500)));
        for i in 0..90 {
            assert!(!f.press(7, ms(533 + i * 33)), "repeat {i} ignored");
        }
        // Another hotkey isn't blocked by the held one.
        assert!(f.press(8, ms(3600)));
        // Let go, press again: acts.
        f.release(7);
        assert!(f.press(7, ms(3700)));
    }

    #[test]
    fn missed_release_recovers_after_a_quiet_second() {
        let mut f = RepeatFilter::default();
        let t0 = Instant::now();
        assert!(f.press(1, t0));
        assert!(!f.press(1, t0 + Duration::from_millis(900)));
        assert!(f.press(1, t0 + Duration::from_millis(2000)));
    }

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
            "Enter",
            "Escape",
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
    fn desktop_shortcuts_cover_bindings_and_overlay_keys_once() {
        let profiles = [profile("TV only", Some("Ctrl+Alt+2"))];
        let (b, _) = build(&Settings::default(), &profiles);
        let extra = [
            (hk("Ctrl+Alt+2"), "Profile list: 2".to_string()),
            (hk("Ctrl+Alt+Esc"), "Profile list: close".to_string()),
        ];
        let s = shortcuts(&b, &extra);
        assert_eq!(s.len(), 4, "{s:?}");
        let by_key = |k: &str| s.iter().find(|x| x.hotkey == hk(k)).unwrap();
        assert_eq!(
            by_key("Ctrl+Alt+2").description,
            "Apply profile 'TV only'",
            "the binding wins"
        );
        assert_eq!(by_key("Ctrl+Alt+D").description, "Show the profile list");
        assert_eq!(
            by_key("Ctrl+Alt+Shift+D").description,
            "Same picture on every monitor (rescue)"
        );
        assert_eq!(by_key("Ctrl+Alt+Esc").description, "Profile list: close");
        // Ids are the ones global-hotkey events carry, so either source
        // reaches the same handler.
        assert_eq!(
            by_key("Ctrl+Alt+D").id,
            hotkey_id(&hk("Ctrl+Alt+D")).unwrap().to_string()
        );
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
