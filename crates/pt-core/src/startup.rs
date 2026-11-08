//! What happens at sign-in.
//!
//! - [`StartupProfile`]: the setting — leave the monitors alone, load the
//!   last used profile, or load a specific profile.
//! - [`AppState`]: small bits the app remembers between runs (the last
//!   applied profile), kept in `state.toml`, separate from the settings so
//!   applying a profile never looks like a settings change.

use std::fmt;
use std::path::Path;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::{Error, Result};
use crate::paths::write_atomic;

/// Which profile (if any) to apply when Paneltarium starts at sign-in.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum StartupProfile {
    /// Leave the monitors as Windows set them up.
    #[default]
    None,
    /// The profile applied most recently through Paneltarium.
    Last,
    /// Always this profile.
    Profile(String),
}

impl fmt::Display for StartupProfile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            StartupProfile::None => f.write_str("none"),
            StartupProfile::Last => f.write_str("last"),
            StartupProfile::Profile(n) => write!(f, "profile:{n}"),
        }
    }
}

impl FromStr for StartupProfile {
    type Err = Error;
    /// `none`, `last`, or `profile:<name>`.
    fn from_str(s: &str) -> Result<Self> {
        let t = s.trim();
        if t.is_empty() || t.eq_ignore_ascii_case("none") {
            Ok(StartupProfile::None)
        } else if t.eq_ignore_ascii_case("last") {
            Ok(StartupProfile::Last)
        } else if let Some(name) = t.strip_prefix("profile:") {
            crate::profile::validate_name(name.trim())?;
            Ok(StartupProfile::Profile(name.trim().to_string()))
        } else {
            Err(Error::InvalidProfileName(t.to_string()))
        }
    }
}

impl Serialize for StartupProfile {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for StartupProfile {
    fn deserialize<D: Deserializer<'de>>(d: D) -> std::result::Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

/// Remembered between runs; `state.toml` in the config folder.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct AppState {
    /// The profile applied most recently through Paneltarium.
    pub last_profile: Option<String>,
}

impl AppState {
    /// Missing or unreadable file = empty state (it's only a convenience).
    pub fn load(path: &Path) -> Self {
        std::fs::read_to_string(path)
            .ok()
            .and_then(|t| toml::from_str(&t).ok())
            .unwrap_or_default()
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        write_atomic(path, &toml::to_string_pretty(self).expect("state serialises"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn startup_profile_text_round_trip() {
        for (text, v) in [
            ("none", StartupProfile::None),
            ("last", StartupProfile::Last),
            ("profile:TV only", StartupProfile::Profile("TV only".into())),
        ] {
            assert_eq!(text.parse::<StartupProfile>().unwrap(), v);
            assert_eq!(v.to_string(), text);
        }
        assert_eq!("".parse::<StartupProfile>().unwrap(), StartupProfile::None);
        assert_eq!("LAST".parse::<StartupProfile>().unwrap(), StartupProfile::Last);
        assert!("bogus".parse::<StartupProfile>().is_err());
        assert!("profile:a/b".parse::<StartupProfile>().is_err());
    }

    #[test]
    fn state_round_trip_and_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state.toml");
        assert_eq!(AppState::load(&path), AppState::default());
        let s = AppState {
            last_profile: Some("Desk".into()),
        };
        s.save(&path).unwrap();
        assert_eq!(AppState::load(&path), s);
        std::fs::write(&path, "garbage = [").unwrap();
        assert_eq!(AppState::load(&path), AppState::default(), "corrupt file is ignored");
    }
}
