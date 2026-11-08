//! Platform-neutral hotkey description, parsed from strings like `Ctrl+Alt+1`.
//!
//! The app layer converts a [`Hotkey`] into whatever the hotkey library needs;
//! [`Key::code_name`] returns W3C `KeyboardEvent.code` names (`KeyA`, `Digit1`,
//! `F5`, ...) which the `global-hotkey` crate understands on every OS.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// Windows key / Super / Command.
    pub win: bool,
}

impl Modifiers {
    pub fn is_empty(&self) -> bool {
        !(self.ctrl || self.alt || self.shift || self.win)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Key {
    /// `A`..=`Z`
    Letter(char),
    /// `0`..=`9` on the main row.
    Digit(u8),
    /// `F1`..=`F24`
    F(u8),
    /// `Num0`..=`Num9`
    Numpad(u8),
    Named(NamedKey),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NamedKey {
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    Space,
    Minus,
    Equal,
    Comma,
    Period,
    Slash,
    Backslash,
    Semicolon,
    Quote,
    Backquote,
    BracketLeft,
    BracketRight,
}

/// (canonical name, accepted aliases, W3C code name)
const NAMED_KEYS: &[(NamedKey, &str, &[&str], &str)] = &[
    (NamedKey::Up, "Up", &["arrowup"], "ArrowUp"),
    (NamedKey::Down, "Down", &["arrowdown"], "ArrowDown"),
    (NamedKey::Left, "Left", &["arrowleft"], "ArrowLeft"),
    (NamedKey::Right, "Right", &["arrowright"], "ArrowRight"),
    (NamedKey::Home, "Home", &[], "Home"),
    (NamedKey::End, "End", &[], "End"),
    (NamedKey::PageUp, "PageUp", &["pgup"], "PageUp"),
    (NamedKey::PageDown, "PageDown", &["pgdn"], "PageDown"),
    (NamedKey::Insert, "Insert", &["ins"], "Insert"),
    (NamedKey::Delete, "Delete", &["del"], "Delete"),
    (NamedKey::Space, "Space", &[], "Space"),
    (NamedKey::Minus, "Minus", &["-"], "Minus"),
    (NamedKey::Equal, "Equal", &["="], "Equal"),
    (NamedKey::Comma, "Comma", &[","], "Comma"),
    (NamedKey::Period, "Period", &["."], "Period"),
    (NamedKey::Slash, "Slash", &["/"], "Slash"),
    (NamedKey::Backslash, "Backslash", &["\\"], "Backslash"),
    (NamedKey::Semicolon, "Semicolon", &[";"], "Semicolon"),
    (NamedKey::Quote, "Quote", &["'"], "Quote"),
    (NamedKey::Backquote, "Backquote", &["`", "grave"], "Backquote"),
    (NamedKey::BracketLeft, "BracketLeft", &["["], "BracketLeft"),
    (NamedKey::BracketRight, "BracketRight", &["]"], "BracketRight"),
];

fn named_entry(k: NamedKey) -> &'static (NamedKey, &'static str, &'static [&'static str], &'static str) {
    NAMED_KEYS
        .iter()
        .find(|(n, ..)| *n == k)
        .expect("every NamedKey has a table entry")
}

impl Key {
    /// W3C `KeyboardEvent.code` name, e.g. `KeyA`, `Digit1`, `Numpad3`, `ArrowUp`.
    pub fn code_name(&self) -> String {
        match self {
            Key::Letter(c) => format!("Key{c}"),
            Key::Digit(d) => format!("Digit{d}"),
            Key::F(n) => format!("F{n}"),
            Key::Numpad(d) => format!("Numpad{d}"),
            Key::Named(n) => named_entry(*n).3.to_string(),
        }
    }

    fn parse(token: &str) -> Option<Key> {
        let lower = token.to_ascii_lowercase();
        let mut chars = token.chars();
        if let (Some(c), None) = (chars.next(), chars.next()) {
            if c.is_ascii_alphabetic() {
                return Some(Key::Letter(c.to_ascii_uppercase()));
            }
            if let Some(d) = c.to_digit(10) {
                return Some(Key::Digit(d as u8));
            }
        }
        if let Some(rest) = lower.strip_prefix('f')
            && let Ok(n) = rest.parse::<u8>()
        {
            return (1..=24).contains(&n).then_some(Key::F(n));
        }
        for prefix in ["numpad", "num"] {
            if let Some(rest) = lower.strip_prefix(prefix)
                && let Ok(n) = rest.parse::<u8>()
            {
                return (n <= 9).then_some(Key::Numpad(n));
            }
        }
        NAMED_KEYS
            .iter()
            .find(|(_, name, aliases, _)| {
                name.eq_ignore_ascii_case(token) || aliases.iter().any(|a| a.eq_ignore_ascii_case(token))
            })
            .map(|(k, ..)| Key::Named(*k))
    }

    /// Keys that are fine to bind with no modifier (rarely used F13-F24).
    fn allowed_without_modifier(&self) -> bool {
        matches!(self, Key::F(n) if *n >= 13)
    }
}

impl fmt::Display for Key {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Key::Letter(c) => write!(f, "{c}"),
            Key::Digit(d) => write!(f, "{d}"),
            Key::F(n) => write!(f, "F{n}"),
            Key::Numpad(d) => write!(f, "Num{d}"),
            Key::Named(n) => f.write_str(named_entry(*n).1),
        }
    }
}

/// A global keyboard shortcut. Stored in profile files as text, e.g. `"Ctrl+Alt+1"`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Hotkey {
    pub modifiers: Modifiers,
    pub key: Key,
}

impl Hotkey {
    pub fn new(modifiers: Modifiers, key: Key) -> Self {
        Hotkey { modifiers, key }
    }

    /// The default "duplicate on all monitors" rescue hotkey.
    pub fn default_duplicate() -> Self {
        Hotkey {
            modifiers: Modifiers {
                ctrl: true,
                alt: true,
                shift: true,
                win: false,
            },
            key: Key::Letter('D'),
        }
    }

    /// The default hotkey that opens the numbered profile overlay.
    pub fn default_overlay() -> Self {
        Hotkey {
            modifiers: Modifiers {
                ctrl: true,
                alt: true,
                shift: false,
                win: false,
            },
            key: Key::Letter('D'),
        }
    }
}

impl FromStr for Hotkey {
    type Err = Error;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        let invalid = |reason: &str| Error::InvalidHotkey {
            input: input.to_string(),
            reason: reason.to_string(),
        };

        // Split on '+' but allow a trailing literal '+' is not supported; use "Equal".
        let tokens: Vec<&str> = input.split('+').map(str::trim).collect();
        if tokens.iter().any(|t| t.is_empty()) {
            return Err(invalid("empty part (did you mean 'Equal' or 'Minus'?)"));
        }
        let (key_token, mod_tokens) = tokens.split_last().ok_or_else(|| invalid("it is empty"))?;

        let mut modifiers = Modifiers::default();
        for t in mod_tokens {
            let slot = match t.to_ascii_lowercase().as_str() {
                "ctrl" | "control" => &mut modifiers.ctrl,
                "alt" | "option" => &mut modifiers.alt,
                "shift" => &mut modifiers.shift,
                "win" | "super" | "meta" | "cmd" | "command" => &mut modifiers.win,
                _ => return Err(invalid(&format!("'{t}' is not a modifier (Ctrl, Alt, Shift, Win)"))),
            };
            if *slot {
                return Err(invalid(&format!("modifier '{t}' is repeated")));
            }
            *slot = true;
        }

        let key = Key::parse(key_token).ok_or_else(|| invalid(&format!("'{key_token}' is not a supported key")))?;
        if modifiers.is_empty() && !key.allowed_without_modifier() {
            return Err(invalid("add at least one modifier (Ctrl, Alt, Shift or Win)"));
        }
        if modifiers
            == (Modifiers {
                shift: true,
                ..Default::default()
            })
            && !key.allowed_without_modifier()
        {
            return Err(invalid("Shift alone would block normal typing; add Ctrl, Alt or Win"));
        }
        Ok(Hotkey { modifiers, key })
    }
}

impl fmt::Display for Hotkey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = &self.modifiers;
        for (on, name) in [(m.ctrl, "Ctrl"), (m.alt, "Alt"), (m.shift, "Shift"), (m.win, "Win")] {
            if on {
                write!(f, "{name}+")?;
            }
        }
        write!(f, "{}", self.key)
    }
}

impl Serialize for Hotkey {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Hotkey {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        s.parse().map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hk(s: &str) -> Hotkey {
        s.parse().unwrap_or_else(|e| panic!("{s}: {e}"))
    }

    #[test]
    fn parses_and_canonicalises() {
        assert_eq!(hk("ctrl+alt+1").to_string(), "Ctrl+Alt+1");
        assert_eq!(hk("Shift + Win + F5").to_string(), "Shift+Win+F5");
        assert_eq!(hk("alt+control+d").to_string(), "Ctrl+Alt+D");
        assert_eq!(hk("Ctrl+Num3").to_string(), "Ctrl+Num3");
        assert_eq!(hk("ctrl+numpad3"), hk("Ctrl+Num3"));
        assert_eq!(hk("Ctrl+Alt+arrowleft").to_string(), "Ctrl+Alt+Left");
        assert_eq!(hk("F13").to_string(), "F13");
    }

    #[test]
    fn code_names_match_w3c() {
        assert_eq!(hk("Ctrl+a").key.code_name(), "KeyA");
        assert_eq!(hk("Ctrl+7").key.code_name(), "Digit7");
        assert_eq!(hk("Ctrl+Num0").key.code_name(), "Numpad0");
        assert_eq!(hk("Ctrl+Up").key.code_name(), "ArrowUp");
        assert_eq!(hk("Ctrl+`").key.code_name(), "Backquote");
    }

    #[test]
    fn rejects_bad_input() {
        for bad in [
            "",
            "Ctrl+",
            "A",
            "Shift+A",
            "Ctrl+Ctrl+A",
            "Hyper+A",
            "Ctrl+F25",
            "Ctrl+Num10",
            "Ctrl+Banana",
            "Ctrl++",
        ] {
            assert!(bad.parse::<Hotkey>().is_err(), "{bad} should be rejected");
        }
    }

    #[test]
    fn serde_round_trip() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W {
            k: Hotkey,
        }
        let w = W {
            k: hk("Ctrl+Alt+Shift+D"),
        };
        let text = toml::to_string(&w).unwrap();
        assert_eq!(text.trim(), r#"k = "Ctrl+Alt+Shift+D""#);
        assert_eq!(toml::from_str::<W>(&text).unwrap(), w);
        assert_eq!(Hotkey::default_duplicate(), w.k);
    }
}
