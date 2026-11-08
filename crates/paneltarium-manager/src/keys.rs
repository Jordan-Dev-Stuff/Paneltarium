//! Turn a key press in the manager window into a [`Hotkey`] (for the
//! "Record" buttons). Pure, tested.

use eframe::egui::{Key, Modifiers};
use pt_core::{Error, Hotkey};

/// `None` for keys that can't be part of a hotkey (Enter, Tab, Esc, ...).
/// Uses the *physical* key when known, because that's what global hotkeys
/// match on (so the result doesn't depend on the keyboard layout).
pub fn to_hotkey(key: Key, physical: Option<Key>, mods: Modifiers) -> Option<Result<Hotkey, Error>> {
    let key = physical.unwrap_or(key);
    let name = key_name(key)?;
    let mut text = String::new();
    for (on, m) in [(mods.ctrl, "Ctrl"), (mods.alt, "Alt"), (mods.shift, "Shift")] {
        if on {
            text += m;
            text += "+";
        }
    }
    text += name;
    Some(text.parse())
}

fn key_name(key: Key) -> Option<&'static str> {
    use Key::*;
    Some(match key {
        ArrowUp => "Up",
        ArrowDown => "Down",
        ArrowLeft => "Left",
        ArrowRight => "Right",
        Home => "Home",
        End => "End",
        PageUp => "PageUp",
        PageDown => "PageDown",
        Insert => "Insert",
        Delete => "Delete",
        Space => "Space",
        Minus => "Minus",
        Equals => "Equal",
        Comma => "Comma",
        Period => "Period",
        Slash => "Slash",
        Backslash => "Backslash",
        Semicolon => "Semicolon",
        Quote => "Quote",
        Backtick => "Backquote",
        OpenBracket => "BracketLeft",
        CloseBracket => "BracketRight",
        // Letters, digits and F-keys: egui's names ("A", "1", "F5") are
        // exactly what our parser expects; anything else isn't bindable.
        other => {
            let n = other.name();
            let single = n.len() == 1 && n.chars().all(|c| c.is_ascii_alphanumeric());
            let f_key = n
                .strip_prefix('F')
                .is_some_and(|d| d.parse::<u8>().is_ok_and(|d| (1..=24).contains(&d)));
            if single || f_key {
                return Some(n);
            }
            return None;
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mods(ctrl: bool, alt: bool, shift: bool) -> Modifiers {
        Modifiers {
            ctrl,
            alt,
            shift,
            command: ctrl,
            ..Default::default()
        }
    }

    fn rec(key: Key, m: Modifiers) -> String {
        to_hotkey(key, None, m).unwrap().unwrap().to_string()
    }

    #[test]
    fn records_common_combos() {
        assert_eq!(rec(Key::Num1, mods(true, true, false)), "Ctrl+Alt+1");
        assert_eq!(rec(Key::D, mods(true, true, true)), "Ctrl+Alt+Shift+D");
        assert_eq!(rec(Key::F13, Modifiers::NONE), "F13");
        assert_eq!(rec(Key::ArrowLeft, mods(true, false, false)), "Ctrl+Left");
        assert_eq!(rec(Key::Equals, mods(false, true, false)), "Alt+Equal");
        assert_eq!(rec(Key::OpenBracket, mods(true, false, false)), "Ctrl+BracketLeft");
    }

    #[test]
    fn physical_key_wins() {
        // e.g. AZERTY: logical Q on the physical A key.
        let h = to_hotkey(Key::Q, Some(Key::A), mods(true, true, false))
            .unwrap()
            .unwrap();
        assert_eq!(h.to_string(), "Ctrl+Alt+A");
    }

    #[test]
    fn unbindable_and_invalid() {
        assert!(to_hotkey(Key::Enter, None, mods(true, false, false)).is_none());
        assert!(to_hotkey(Key::Escape, None, Modifiers::NONE).is_none());
        // A bare letter is rejected by the hotkey rules (needs a modifier).
        assert!(to_hotkey(Key::A, None, Modifiers::NONE).unwrap().is_err());
    }
}
