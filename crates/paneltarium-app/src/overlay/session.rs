//! What the overlay shows and how it reacts to keys, the mouse and a game
//! controller. Pure logic, no OS or window code, so all of it is tested.

use pt_core::current::Current;
use pt_core::{Modifiers, Profile, slots};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EntryKind {
    Profile(String),
    /// Number 0: duplicate on every monitor (the rescue action).
    Duplicate,
}

/// One numbered row in the overlay.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub number: u8,
    pub kind: EntryKind,
    pub title: String,
    pub detail: String,
    /// The profile's own hotkey, shown on the right.
    pub hotkey: Option<String>,
    /// The profile applied most recently.
    pub current: bool,
}

/// Rows 1–9 (numbering shared with the profile manager via
/// [`pt_core::slots`]), then 0 = duplicate. Also returns how many profiles
/// didn't fit.
pub fn entries(profiles: &[Profile], overlay_mods: Modifiers, current: &Current) -> (Vec<Entry>, usize) {
    let (assigned, overflow) = slots::assign(profiles, overlay_mods);
    let mut out: Vec<Entry> = assigned
        .into_iter()
        .map(|(n, i)| {
            let p = &profiles[i];
            Entry {
                number: n,
                kind: EntryKind::Profile(p.name.clone()),
                title: p.name.clone(),
                detail: p.layout.summary(),
                hotkey: p.hotkey.map(|h| h.to_string()),
                current: current.is_profile(&p.name),
            }
        })
        .collect();
    out.push(Entry {
        number: 0,
        kind: EntryKind::Duplicate,
        title: "Duplicate on all monitors".into(),
        detail: "Rescue: same picture on every screen".into(),
        hotkey: None,
        current: *current == Current::Duplicate,
    });
    (out, overflow)
}

/// What opened the overlay; decides how it closes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Opener {
    /// Keyboard hotkey: letting go of its modifiers cancels.
    Keyboard,
    /// Controller button combination: letting go applies the highlighted
    /// row (if the D-pad moved it), B cancels.
    Gamepad,
}

/// Keys and events the overlay reacts to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Input {
    Digit(u8),
    /// Arrow keys / D-pad: -1 = previous row, +1 = next row (wraps around).
    Move(i32),
    /// Enter: apply the selected row.
    Confirm,
    /// The pointer is over row `n`.
    Hover(usize),
    /// Row `n` was clicked.
    Click(usize),
    /// Keyboard opener: (one of) the trigger's modifier keys was let go.
    ModifiersReleased,
    /// Controller opener: the button combination was let go.
    ChordReleased,
    /// Esc, controller B, or the overlay hotkey pressed again.
    Cancel,
    /// Open too long without a choice.
    Timeout,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Decision {
    KeepOpen,
    /// Still open, but something visible changed: repaint.
    Redraw,
    Close,
    Apply(String),
    Rescue,
}

#[derive(Debug, Clone)]
pub struct Session {
    pub entries: Vec<Entry>,
    pub overflow: usize,
    /// Index into `entries` of the highlighted row.
    pub selected: usize,
    pub opener: Opener,
    /// The pointer has been over the overlay: from then on it stays open
    /// when the keyboard modifiers are let go (you're using the mouse now).
    pub pointer: bool,
    /// The highlight was moved (D-pad, arrows or mouse) since opening.
    moved: bool,
    modifiers: Modifiers,
    can_track_release: bool,
}

impl Session {
    /// Starts with what's on screen now selected (or the first row).
    pub fn new(
        profiles: &[Profile],
        overlay_mods: Modifiers,
        current: &Current,
        opener: Opener,
        can_track_release: bool,
    ) -> Self {
        let (entries, overflow) = entries(profiles, overlay_mods, current);
        let selected = entries.iter().position(|e| e.current).unwrap_or(0);
        Session {
            entries,
            overflow,
            selected,
            opener,
            pointer: false,
            moved: false,
            modifiers: overlay_mods,
            can_track_release,
        }
    }

    /// Should letting go of the keyboard modifiers close the overlay now?
    pub fn closes_on_modifier_release(&self) -> bool {
        self.opener == Opener::Keyboard && self.can_track_release && !self.pointer
    }

    pub fn handle(&mut self, input: Input) -> Decision {
        match input {
            Input::Digit(n) => match self.entries.iter().position(|e| e.number == n) {
                Some(i) => self.choose(i),
                None => Decision::KeepOpen, // unused number: ignore
            },
            Input::Move(step) => {
                let len = self.entries.len() as i32;
                self.selected = (self.selected as i32 + step).rem_euclid(len) as usize;
                self.moved = true;
                Decision::Redraw
            }
            Input::Confirm => self.choose(self.selected),
            Input::Hover(i) if i < self.entries.len() => {
                let changed = !self.pointer || self.selected != i;
                self.pointer = true;
                self.moved |= self.selected != i;
                self.selected = i;
                if changed { Decision::Redraw } else { Decision::KeepOpen }
            }
            Input::Click(i) if i < self.entries.len() => self.choose(i),
            Input::Hover(_) | Input::Click(_) => Decision::KeepOpen,
            Input::ModifiersReleased if !self.closes_on_modifier_release() => Decision::KeepOpen,
            Input::ChordReleased if self.opener != Opener::Gamepad => Decision::KeepOpen,
            // Letting go applies the highlighted row once you've moved the
            // highlight, unless that row is what's already on screen.
            Input::ChordReleased if self.moved && !self.entries[self.selected].current => self.choose(self.selected),
            Input::ChordReleased | Input::ModifiersReleased | Input::Cancel | Input::Timeout => Decision::Close,
        }
    }

    fn choose(&self, i: usize) -> Decision {
        match &self.entries[i].kind {
            EntryKind::Profile(name) => Decision::Apply(name.clone()),
            EntryKind::Duplicate => Decision::Rescue,
        }
    }

    /// The instruction line under the title, for the current state.
    pub fn hint(&self) -> String {
        if self.opener == Opener::Gamepad {
            return "D-pad to choose · let go to apply · B to cancel".into();
        }
        if self.pointer || !self.can_track_release {
            return "Click a profile, press its number, or arrows + Enter · Esc to close".into();
        }
        let m = self.modifiers;
        let names: Vec<&str> = [(m.ctrl, "Ctrl"), (m.alt, "Alt"), (m.shift, "Shift"), (m.win, "Win")]
            .into_iter()
            .filter_map(|(on, n)| on.then_some(n))
            .collect();
        format!(
            "Hold {} · press a number or arrows + Enter · let go to cancel",
            names.join("+")
        )
    }
}

/// Is every modifier in `required` still held?
pub fn still_held(required: Modifiers, held: Modifiers) -> bool {
    (!required.ctrl || held.ctrl)
        && (!required.alt || held.alt)
        && (!required.shift || held.shift)
        && (!required.win || held.win)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pt_core::testing::sample_layout;

    fn ctrl_alt() -> Modifiers {
        Modifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        }
    }

    fn p(name: &str, hotkey: Option<&str>) -> Profile {
        let mut p = Profile::new(name, sample_layout()).unwrap();
        p.hotkey = hotkey.map(|h| h.parse().unwrap());
        p
    }

    fn numbers(e: &[Entry]) -> Vec<(u8, &str)> {
        e.iter().map(|e| (e.number, e.title.as_str())).collect()
    }

    fn profiles() -> [Profile; 3] {
        // Alphabetical list order, but hotkeys say TV=2, LG=9.
        [
            p("Both", None),
            p("LG only", Some("Ctrl+Alt+9")),
            p("TV only", Some("Ctrl+Alt+2")),
        ]
    }

    fn cur(name: Option<&str>) -> Current {
        name.map(|n| Current::Profile(n.into())).unwrap_or(Current::Unknown)
    }

    fn session(opener: Opener, current: Option<&str>) -> Session {
        Session::new(&profiles(), ctrl_alt(), &cur(current), opener, true)
    }

    #[test]
    fn rows_follow_shared_numbering() {
        let (e, overflow) = entries(&profiles(), ctrl_alt(), &cur(Some("tv only")));
        assert_eq!(overflow, 0);
        assert_eq!(
            numbers(&e),
            [
                (1, "Both"),
                (2, "TV only"),
                (9, "LG only"),
                (0, "Duplicate on all monitors")
            ]
        );
        assert!(e[1].current && !e[0].current);
    }

    #[test]
    fn digits_choose_directly() {
        let mut s = session(Opener::Keyboard, None);
        assert_eq!(s.handle(Input::Digit(1)), Decision::Apply("Both".into()));
        assert_eq!(s.handle(Input::Digit(0)), Decision::Rescue);
        assert_eq!(s.handle(Input::Digit(5)), Decision::KeepOpen);
        for i in [Input::ModifiersReleased, Input::Cancel, Input::Timeout] {
            assert_eq!(s.handle(i), Decision::Close);
        }
    }

    #[test]
    fn arrows_move_selection_and_enter_applies() {
        let mut s = session(Opener::Keyboard, Some("TV only"));
        assert_eq!(s.selected, 1, "starts on the current profile");
        assert_eq!(s.handle(Input::Move(1)), Decision::Redraw);
        assert_eq!(s.handle(Input::Confirm), Decision::Apply("LG only".into()));
        s.handle(Input::Move(1));
        assert_eq!(s.handle(Input::Confirm), Decision::Rescue, "row 0 is last");
        s.handle(Input::Move(1));
        assert_eq!(s.selected, 0, "wraps to the top");
        s.handle(Input::Move(-1));
        assert_eq!(s.selected, 3, "wraps to the bottom");
    }

    #[test]
    fn mouse_hover_selects_click_applies_and_keeps_it_open() {
        let mut s = session(Opener::Keyboard, None);
        assert!(s.closes_on_modifier_release());
        assert_eq!(s.handle(Input::Hover(2)), Decision::Redraw);
        assert_eq!(s.selected, 2);
        assert_eq!(
            s.handle(Input::Hover(2)),
            Decision::KeepOpen,
            "no repaint if nothing changed"
        );
        assert!(!s.closes_on_modifier_release(), "mouse mode: keys can be let go");
        assert_eq!(s.handle(Input::ModifiersReleased), Decision::KeepOpen);
        assert!(s.hint().starts_with("Click a profile"));
        assert_eq!(s.handle(Input::Hover(99)), Decision::KeepOpen, "out of range ignored");
        assert_eq!(s.handle(Input::Click(1)), Decision::Apply("TV only".into()));
        assert_eq!(s.handle(Input::Click(3)), Decision::Rescue);
        assert_eq!(s.handle(Input::Cancel), Decision::Close);
    }

    #[test]
    fn controller_release_applies_only_after_moving() {
        let mut s = session(Opener::Gamepad, Some("Both"));
        assert!(s.hint().contains("D-pad"));
        assert_eq!(
            s.handle(Input::ModifiersReleased),
            Decision::KeepOpen,
            "keyboard state ignored"
        );
        assert_eq!(
            s.handle(Input::ChordReleased),
            Decision::Close,
            "didn't move: nothing to do"
        );

        let mut s = session(Opener::Gamepad, Some("Both"));
        s.handle(Input::Move(1));
        assert_eq!(s.handle(Input::ChordReleased), Decision::Apply("TV only".into()));

        let mut s = session(Opener::Gamepad, None);
        s.handle(Input::Move(-1));
        assert_eq!(s.handle(Input::ChordReleased), Decision::Rescue);

        let mut s = session(Opener::Gamepad, None);
        s.handle(Input::Move(1));
        assert_eq!(s.handle(Input::Cancel), Decision::Close, "B cancels");
    }

    #[test]
    fn duplicate_shows_as_current_and_previous_profile_is_reachable() {
        // The reported bug: after "Duplicate on all monitors", the overlay
        // must mark duplicate as current and start there, and moving to the
        // previous profile and letting go must apply it.
        let mut s = Session::new(&profiles(), ctrl_alt(), &Current::Duplicate, Opener::Gamepad, true);
        assert_eq!(s.entries[s.selected].kind, EntryKind::Duplicate);
        assert!(s.entries[s.selected].current);
        assert!(s.entries.iter().filter(|e| e.current).count() == 1);
        s.handle(Input::Move(-1)); // up to "LG only" (number 9)
        assert_eq!(s.handle(Input::ChordReleased), Decision::Apply("LG only".into()));
    }

    #[test]
    fn controller_release_with_nothing_current() {
        // Nothing matches: starts on row 1; letting go without moving does
        // nothing (no accidental apply)...
        let mut s = session(Opener::Gamepad, None);
        assert_eq!(s.handle(Input::ChordReleased), Decision::Close);
        // ...but moving away and back to row 1 does apply it.
        let mut s = session(Opener::Gamepad, None);
        s.handle(Input::Move(1));
        s.handle(Input::Move(-1));
        assert_eq!(s.handle(Input::ChordReleased), Decision::Apply("Both".into()));
    }

    #[test]
    fn controller_release_on_current_row_does_nothing() {
        let mut s = session(Opener::Gamepad, Some("Both"));
        s.handle(Input::Move(1));
        s.handle(Input::Move(-1)); // back onto what's already on screen
        assert_eq!(s.handle(Input::ChordReleased), Decision::Close);
    }

    #[test]
    fn chord_release_ignored_for_keyboard_sessions() {
        let mut s = session(Opener::Keyboard, None);
        s.handle(Input::Move(1));
        assert_eq!(s.handle(Input::ChordReleased), Decision::KeepOpen);
    }

    #[test]
    fn release_detection_and_hints() {
        let held = Modifiers {
            ctrl: true,
            alt: true,
            shift: true,
            win: false,
        };
        assert!(still_held(ctrl_alt(), held));
        assert!(!still_held(
            ctrl_alt(),
            Modifiers {
                ctrl: true,
                ..Default::default()
            }
        ));
        assert_eq!(
            session(Opener::Keyboard, None).hint(),
            "Hold Ctrl+Alt · press a number or arrows + Enter · let go to cancel"
        );
        let no_tracking = Session::new(&profiles(), ctrl_alt(), &Current::Unknown, Opener::Keyboard, false);
        assert!(!no_tracking.closes_on_modifier_release());
        assert!(no_tracking.hint().ends_with("Esc to close"));
    }

    #[test]
    fn stay_open_mode_ignores_letting_go() {
        // "Stay open" is a keyboard session that doesn't track the release.
        let mut s = Session::new(&profiles(), ctrl_alt(), &cur(Some("TV only")), Opener::Keyboard, false);
        assert_eq!(s.handle(Input::ModifiersReleased), Decision::KeepOpen);
        assert_eq!(s.handle(Input::Move(1)), Decision::Redraw);
        assert_eq!(
            s.hint(),
            "Click a profile, press its number, or arrows + Enter · Esc to close"
        );
        assert_eq!(s.handle(Input::Digit(9)), Decision::Apply("LG only".into()));
        assert_eq!(
            s.handle(Input::Cancel),
            Decision::Close,
            "Esc or the hotkey again closes"
        );
    }
}
