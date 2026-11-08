//! Overlay numbers ("slots" 1–9). Shared by the overlay and the profile
//! manager so both always show the same numbers.
//!
//! A profile whose hotkey is "overlay modifiers + digit N" sits in slot N.
//! The remaining profiles fill the free slots in list (name) order.
//! Moving a profile to a slot therefore just means giving it that hotkey.

use crate::error::Result;
use crate::hotkey::{Hotkey, Key, Modifiers};
use crate::profile::{Profile, ProfileStore};

/// `(slot number, index into profiles)` sorted by slot, plus how many
/// profiles didn't fit into slots 1–9.
pub fn assign(profiles: &[Profile], overlay_mods: Modifiers) -> (Vec<(u8, usize)>, usize) {
    let mut slots: [Option<usize>; 10] = [None; 10];
    let mut placed = vec![false; profiles.len()];

    for (i, p) in profiles.iter().enumerate() {
        if let Some(n) = slot_of_hotkey(p.hotkey, overlay_mods)
            && slots[n as usize].is_none()
        {
            slots[n as usize] = Some(i);
            placed[i] = true;
        }
    }
    let mut overflow = 0;
    for (i, _) in profiles.iter().enumerate() {
        if placed[i] {
            continue;
        }
        match (1..=9).find(|&n| slots[n].is_none()) {
            Some(n) => slots[n] = Some(i),
            None => overflow += 1,
        }
    }
    let list = (1..=9u8).filter_map(|n| slots[n as usize].map(|i| (n, i))).collect();
    (list, overflow)
}

/// The slot a hotkey pins its profile to, if it is "overlay modifiers + 1–9".
pub fn slot_of_hotkey(hotkey: Option<Hotkey>, overlay_mods: Modifiers) -> Option<u8> {
    match hotkey {
        Some(Hotkey {
            modifiers,
            key: Key::Digit(n),
        }) if modifiers == overlay_mods && (1..=9).contains(&n) => Some(n),
        _ => None,
    }
}

pub fn slot_hotkey(overlay_mods: Modifiers, n: u8) -> Hotkey {
    Hotkey::new(overlay_mods, Key::Digit(n))
}

/// Pin `name` to slot `n` by giving it the slot's hotkey. A profile that
/// currently holds that hotkey swaps: it gets `name`'s old slot hotkey (if
/// it had one) or no hotkey.
pub fn move_to_slot(
    store: &ProfileStore,
    name: &str,
    n: u8,
    overlay_mods: Modifiers,
    reserved: &[(Hotkey, &str)],
) -> Result<()> {
    let target = slot_hotkey(overlay_mods, n);
    let me = store.get(name)?;
    if me.hotkey == Some(target) {
        return Ok(());
    }
    let my_old_slot = slot_of_hotkey(me.hotkey, overlay_mods).map(|s| slot_hotkey(overlay_mods, s));
    if let Some(other) = store.find_by_hotkey(target)? {
        // Clear first so the two hotkeys never collide mid-swap.
        store.set_hotkey(&other.name, None, reserved)?;
        store.set_hotkey(&me.name, Some(target), reserved)?;
        store.set_hotkey(&other.name, my_old_slot, reserved)?;
    } else {
        store.set_hotkey(&me.name, Some(target), reserved)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::sample_layout;

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

    fn names<'a>(profiles: &'a [Profile], slots: &[(u8, usize)]) -> Vec<(u8, &'a str)> {
        slots.iter().map(|(n, i)| (*n, profiles[*i].name.as_str())).collect()
    }

    #[test]
    fn hotkey_digits_pin_slots_and_others_fill_in() {
        let profiles = [
            p("Both", None),
            p("LG only", Some("Ctrl+Alt+9")),
            p("TV only", Some("Ctrl+Alt+2")),
            p("Other mods", Some("Ctrl+Shift+1")),
        ];
        let (slots, overflow) = assign(&profiles, ctrl_alt());
        assert_eq!(overflow, 0);
        assert_eq!(
            names(&profiles, &slots),
            [(1, "Both"), (2, "TV only"), (3, "Other mods"), (9, "LG only")]
        );
    }

    #[test]
    fn overflow_beyond_nine() {
        let profiles: Vec<_> = (0..12).map(|i| p(&format!("P{i:02}"), None)).collect();
        let (slots, overflow) = assign(&profiles, ctrl_alt());
        assert_eq!((slots.len(), overflow), (9, 3));
    }

    #[test]
    fn move_to_slot_swaps() {
        let dir = tempfile::tempdir().unwrap();
        let store = ProfileStore::new(dir.path());
        for prof in [p("A", Some("Ctrl+Alt+1")), p("B", Some("Ctrl+Alt+2")), p("C", None)] {
            store.save(&prof, false, &[]).unwrap();
        }
        // A takes slot 2: B gets A's old slot 1.
        move_to_slot(&store, "A", 2, ctrl_alt(), &[]).unwrap();
        assert_eq!(store.get("A").unwrap().hotkey.unwrap().to_string(), "Ctrl+Alt+2");
        assert_eq!(store.get("B").unwrap().hotkey.unwrap().to_string(), "Ctrl+Alt+1");
        // C (no slot hotkey) takes slot 1: B ends up with no hotkey.
        move_to_slot(&store, "C", 1, ctrl_alt(), &[]).unwrap();
        assert_eq!(store.get("C").unwrap().hotkey.unwrap().to_string(), "Ctrl+Alt+1");
        assert_eq!(store.get("B").unwrap().hotkey, None);
        // Free slot: no swap needed.
        move_to_slot(&store, "B", 5, ctrl_alt(), &[]).unwrap();
        assert_eq!(store.get("B").unwrap().hotkey.unwrap().to_string(), "Ctrl+Alt+5");
    }
}
