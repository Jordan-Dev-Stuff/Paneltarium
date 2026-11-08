//! Game-controller input, OS- and library-neutral.
//!
//! - [`Button`]: Xbox-style names, with PlayStation/Nintendo-style aliases
//!   (`R1` = `RB`, `Cross` = `A`, ...).
//! - [`GamepadChord`]: a set of buttons held together, e.g. `A+LB+RB+LT+RT`.
//!   Stored in settings as text.
//! - [`ChordTracker`]: turns button presses/releases into "chord held" /
//!   "chord let go" events.
//! - [`ChordRecorder`]: records a chord by "hold the buttons, then let go".

use std::collections::BTreeSet;
use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::error::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Button {
    A,
    B,
    X,
    Y,
    LB,
    RB,
    LT,
    RT,
    Back,
    Start,
    Guide,
    LeftStick,
    RightStick,
    DpadUp,
    DpadDown,
    DpadLeft,
    DpadRight,
}

/// (button, canonical name, aliases)
const NAMES: &[(Button, &str, &[&str])] = &[
    (Button::A, "A", &["Cross"]),
    (Button::B, "B", &["Circle"]),
    (Button::X, "X", &["Square"]),
    (Button::Y, "Y", &["Triangle"]),
    (Button::LB, "LB", &["L1", "L"]),
    (Button::RB, "RB", &["R1", "R"]),
    (Button::LT, "LT", &["L2", "ZL"]),
    (Button::RT, "RT", &["R2", "ZR"]),
    (Button::Back, "Back", &["Select", "View", "Share", "Minus"]),
    (Button::Start, "Start", &["Menu", "Options", "Plus"]),
    (Button::Guide, "Guide", &["Home", "Xbox", "PS"]),
    (Button::LeftStick, "LS", &["L3"]),
    (Button::RightStick, "RS", &["R3"]),
    (Button::DpadUp, "Up", &["DpadUp"]),
    (Button::DpadDown, "Down", &["DpadDown"]),
    (Button::DpadLeft, "Left", &["DpadLeft"]),
    (Button::DpadRight, "Right", &["DpadRight"]),
];

impl Button {
    pub fn name(self) -> &'static str {
        NAMES
            .iter()
            .find(|(b, ..)| *b == self)
            .map(|(_, n, _)| *n)
            .expect("every button has a name")
    }

    pub fn is_dpad(self) -> bool {
        matches!(
            self,
            Button::DpadUp | Button::DpadDown | Button::DpadLeft | Button::DpadRight
        )
    }
}

impl fmt::Display for Button {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.name())
    }
}

impl FromStr for Button {
    type Err = ();
    fn from_str(s: &str) -> Result<Self, ()> {
        NAMES
            .iter()
            .find(|(_, n, aliases)| n.eq_ignore_ascii_case(s) || aliases.iter().any(|a| a.eq_ignore_ascii_case(s)))
            .map(|(b, ..)| *b)
            .ok_or(())
    }
}

/// Buttons held together. At least two, no D-pad (the D-pad moves the
/// overlay's selection).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct GamepadChord {
    buttons: BTreeSet<Button>,
}

impl GamepadChord {
    pub fn new(buttons: impl IntoIterator<Item = Button>) -> Result<Self, Error> {
        let buttons: BTreeSet<Button> = buttons.into_iter().collect();
        let text = buttons.iter().map(|b| b.name()).collect::<Vec<_>>().join("+");
        let invalid = |reason: &str| Error::InvalidGamepadChord {
            input: text.clone(),
            reason: reason.into(),
        };
        if buttons.len() < 2 {
            return Err(invalid("use at least two buttons so it can't be pressed by accident"));
        }
        if buttons.iter().any(|b| b.is_dpad()) {
            return Err(invalid("the D-pad is used to move in the overlay; pick other buttons"));
        }
        Ok(GamepadChord { buttons })
    }

    /// A + LB + RB + LT + RT (A + R1 + R2 + L1 + L2 on PlayStation pads).
    pub fn default_overlay() -> Self {
        GamepadChord::new([Button::A, Button::LB, Button::RB, Button::LT, Button::RT]).expect("valid default")
    }

    pub fn buttons(&self) -> impl Iterator<Item = Button> + '_ {
        self.buttons.iter().copied()
    }

    pub fn contains(&self, b: Button) -> bool {
        self.buttons.contains(&b)
    }
}

impl fmt::Display for GamepadChord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let names: Vec<&str> = self.buttons.iter().map(|b| b.name()).collect();
        f.write_str(&names.join("+"))
    }
}

impl FromStr for GamepadChord {
    type Err = Error;
    fn from_str(input: &str) -> Result<Self, Error> {
        let mut buttons = Vec::new();
        for part in input.split('+').map(str::trim) {
            let b = part.parse::<Button>().map_err(|()| Error::InvalidGamepadChord {
                input: input.to_string(),
                reason: format!(
                    "'{part}' is not a controller button (A, B, X, Y, LB, RB, LT, RT, Back, Start, LS, RS)"
                ),
            })?;
            buttons.push(b);
        }
        GamepadChord::new(buttons)
    }
}

impl Serialize for GamepadChord {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for GamepadChord {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChordEvent {
    /// Every button of the chord is now held.
    Activated,
    /// One of the chord's buttons was let go after it had been activated.
    Released,
}

/// Tracks one controller's held buttons against a chord.
#[derive(Debug, Default, Clone)]
pub struct ChordTracker {
    held: BTreeSet<Button>,
    active: bool,
}

impl ChordTracker {
    pub fn update(&mut self, chord: &GamepadChord, button: Button, pressed: bool) -> Option<ChordEvent> {
        if pressed {
            self.held.insert(button);
        } else {
            self.held.remove(&button);
        }
        if !self.active && chord.buttons.is_subset(&self.held) {
            self.active = true;
            Some(ChordEvent::Activated)
        } else if self.active && !pressed && chord.contains(button) {
            self.active = false;
            Some(ChordEvent::Released)
        } else {
            None
        }
    }

    pub fn is_held(&self, button: Button) -> bool {
        self.held.contains(&button)
    }
}

/// "Hold the buttons you want, then let go": the largest set held at once
/// becomes the chord once every button is released.
#[derive(Debug, Default, Clone)]
pub struct ChordRecorder {
    held: BTreeSet<Button>,
    peak: BTreeSet<Button>,
}

impl ChordRecorder {
    pub fn update(&mut self, button: Button, pressed: bool) -> Option<Result<GamepadChord, Error>> {
        if pressed {
            self.held.insert(button);
            if self.held.len() >= self.peak.len() {
                self.peak = self.held.clone();
            }
            None
        } else {
            self.held.remove(&button);
            if self.held.is_empty() && !self.peak.is_empty() {
                Some(GamepadChord::new(std::mem::take(&mut self.peak)))
            } else {
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use Button::*;

    #[test]
    fn parse_display_and_aliases() {
        let c: GamepadChord = "a + r1 + R2 + l1 + L2".parse().unwrap();
        assert_eq!(c.to_string(), "A+LB+RB+LT+RT");
        assert_eq!(c, GamepadChord::default_overlay());
        assert_eq!("Cross+Options".parse::<GamepadChord>().unwrap().to_string(), "A+Start");
        assert!("A".parse::<GamepadChord>().is_err(), "one button is too easy to hit");
        assert!("A+Up".parse::<GamepadChord>().is_err(), "D-pad is for moving");
        assert!("A+Banana".parse::<GamepadChord>().is_err());
    }

    #[test]
    fn serde_round_trip() {
        #[derive(Serialize, Deserialize, PartialEq, Debug)]
        struct W {
            c: GamepadChord,
        }
        let w = W {
            c: GamepadChord::default_overlay(),
        };
        let t = toml::to_string(&w).unwrap();
        assert_eq!(t.trim(), r#"c = "A+LB+RB+LT+RT""#);
        assert_eq!(toml::from_str::<W>(&t).unwrap(), w);
    }

    #[test]
    fn tracker_activates_when_all_held_and_releases_on_first_let_go() {
        let chord = GamepadChord::default_overlay();
        let mut t = ChordTracker::default();
        for b in [LB, RB, LT, RT] {
            assert_eq!(t.update(&chord, b, true), None);
        }
        assert_eq!(t.update(&chord, X, true), None, "unrelated button");
        assert_eq!(t.update(&chord, A, true), Some(ChordEvent::Activated));
        assert_eq!(t.update(&chord, DpadDown, true), None, "D-pad while held");
        assert_eq!(t.update(&chord, DpadDown, false), None);
        assert_eq!(t.update(&chord, X, false), None, "unrelated release");
        assert_eq!(t.update(&chord, LT, false), Some(ChordEvent::Released));
        assert_eq!(t.update(&chord, RT, false), None, "already released");
        assert_eq!(t.update(&chord, LT, true), None, "RT is still up");
        assert_eq!(
            t.update(&chord, RT, true),
            Some(ChordEvent::Activated),
            "can re-activate"
        );
    }

    #[test]
    fn recorder_uses_the_biggest_set_held() {
        let mut r = ChordRecorder::default();
        assert!(r.update(A, true).is_none());
        assert!(r.update(LB, true).is_none());
        assert!(r.update(RB, true).is_none());
        assert!(r.update(A, false).is_none(), "still holding others");
        assert!(r.update(LB, false).is_none());
        let c = r.update(RB, false).unwrap().unwrap();
        assert_eq!(c.to_string(), "A+LB+RB");

        let mut r = ChordRecorder::default();
        r.update(A, true);
        assert!(r.update(A, false).unwrap().is_err(), "single button rejected");
    }
}
