//! X11 calls, through `x11rb` (pure Rust; talks to `$DISPLAY`, which is also
//! XWayland, where the overlay windows live on Wayland desktops).
//!
//! - [`X11Keyboard`]: which modifier keys are held, for closing the overlay
//!   when you let go. Only on real X11 sessions: under XWayland the X server
//!   only sees keys while one of its own windows has focus.
//! - [`round_corners`]: clip a window to a rounded rectangle (SHAPE extension).

use std::sync::Mutex;

use pt_core::{KeyboardState, Modifiers};
use x11rb::connection::Connection;
use x11rb::protocol::shape::{self, ConnectionExt as _};
use x11rb::protocol::xproto::{ClipOrdering, ConnectionExt as _, Rectangle};
use x11rb::rust_connection::RustConnection;

/// Keycodes behind each modifier we care about.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct ModMap {
    shift: Vec<u8>,
    ctrl: Vec<u8>,
    alt: Vec<u8>,
    logo: Vec<u8>,
}

impl ModMap {
    /// From `GetModifierMapping`: 8 rows (Shift, Lock, Control, Mod1–Mod5)
    /// of `per` keycodes each, 0 = unused. Alt is Mod1, Super is Mod4.
    fn from_rows(keycodes: &[u8], per: usize) -> Self {
        let row = |i: usize| -> Vec<u8> {
            keycodes
                .get(i * per..(i + 1) * per)
                .unwrap_or_default()
                .iter()
                .copied()
                .filter(|&k| k != 0)
                .collect()
        };
        ModMap {
            shift: row(0),
            ctrl: row(2),
            alt: row(3),
            logo: row(6),
        }
    }

    /// Which modifiers are down in a `QueryKeymap` bit vector.
    fn held(&self, keys: &[u8; 32]) -> Modifiers {
        let down = |codes: &[u8]| codes.iter().any(|&k| keys[usize::from(k / 8)] & (1 << (k % 8)) != 0);
        Modifiers {
            ctrl: down(&self.ctrl),
            alt: down(&self.alt),
            shift: down(&self.shift),
            win: down(&self.logo),
        }
    }
}

pub struct X11Keyboard {
    conn: Mutex<RustConnection>,
    map: ModMap,
}

impl X11Keyboard {
    pub fn connect() -> Option<Self> {
        let (conn, _) = x11rb::connect(None).ok()?;
        let reply = conn.get_modifier_mapping().ok()?.reply().ok()?;
        let per = usize::from(reply.keycodes_per_modifier());
        let map = ModMap::from_rows(&reply.keycodes, per);
        Some(X11Keyboard {
            conn: Mutex::new(conn),
            map,
        })
    }
}

impl KeyboardState for X11Keyboard {
    fn modifiers_held(&self) -> Option<Modifiers> {
        let conn = self.conn.lock().ok()?;
        let keys = conn.query_keymap().ok()?.reply().ok()?.keys;
        Some(self.map.held(&keys))
    }
}

/// Rows that make up a `w`×`h` rectangle with corners of radius `r`:
/// one rectangle per pixel row inside the corners, one for the middle.
fn rounded_rows(w: u16, h: u16, r: u16) -> Vec<Rectangle> {
    let r = r.min(w / 2).min(h / 2);
    let mut rows = Vec::with_capacity(usize::from(r) * 2 + 1);
    for i in 0..r {
        let dy = f64::from(r) - f64::from(i) - 0.5;
        let inset = (f64::from(r) - (f64::from(r).powi(2) - dy * dy).max(0.0).sqrt()).round() as u16;
        let width = w.saturating_sub(inset * 2);
        for y in [i, h - 1 - i] {
            rows.push(Rectangle {
                x: inset as i16,
                y: y as i16,
                width,
                height: 1,
            });
        }
    }
    if h > r * 2 {
        rows.push(Rectangle {
            x: 0,
            y: r as i16,
            width: w,
            height: h - r * 2,
        });
    }
    rows
}

/// Clip X11 window `window` to a rounded rectangle. Best effort.
pub fn round_corners(window: u32, width: u32, height: u32, radius: u32) {
    let clamp = |v: u32| v.min(u32::from(u16::MAX)) as u16;
    let rows = rounded_rows(clamp(width), clamp(height), clamp(radius));
    let result = x11rb::connect(None).map_err(|e| e.to_string()).and_then(|(conn, _)| {
        conn.shape_rectangles(
            shape::SO::SET,
            shape::SK::BOUNDING,
            ClipOrdering::UNSORTED,
            window,
            0,
            0,
            &rows,
        )
        .map_err(|e| e.to_string())?;
        conn.flush().map_err(|e| e.to_string())
    });
    if let Err(e) = result {
        eprintln!("paneltarium: couldn't round the overlay's corners: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn modifier_rows_and_keymap_bits() {
        // 2 keycodes per modifier: Shift 50/62, Lock 66, Control 37/105,
        // Mod1 64/108, Mod2 77, Mod3 -, Mod4 133/134, Mod5 92.
        let rows = [50, 62, 66, 0, 37, 105, 64, 108, 77, 0, 0, 0, 133, 134, 92, 0];
        let map = ModMap::from_rows(&rows, 2);
        assert_eq!(map.ctrl, [37, 105]);
        assert_eq!(map.alt, [64, 108]);
        assert_eq!(map.logo, [133, 134]);

        let mut keys = [0u8; 32];
        assert_eq!(map.held(&keys), Modifiers::default());
        let press = |keys: &mut [u8; 32], k: u8| keys[usize::from(k / 8)] |= 1 << (k % 8);
        press(&mut keys, 105); // right Ctrl
        press(&mut keys, 64); // left Alt
        let held = map.held(&keys);
        assert!(held.ctrl && held.alt && !held.shift && !held.win);
        press(&mut keys, 134);
        assert!(map.held(&keys).win);
    }

    #[test]
    fn short_modifier_rows_are_tolerated() {
        let map = ModMap::from_rows(&[50, 0, 37], 1);
        assert_eq!(map.ctrl, [37]);
        assert!(map.alt.is_empty() && map.logo.is_empty());
    }

    #[test]
    fn rounded_rows_cover_a_rounded_rectangle() {
        let rows = rounded_rows(100, 60, 10);
        assert_eq!(rows.len(), 21, "10 rows top, 10 bottom, one middle");
        let area: u32 = rows.iter().map(|r| u32::from(r.width) * u32::from(r.height)).sum();
        // Each corner cuts away about (1 - π/4)·r² ≈ 21.5 px.
        let cut = 100 * 60 - area;
        assert!((80..=95).contains(&cut), "cut {cut}");
        // Top row is the narrowest, rows widen towards the middle, and
        // every row is centred.
        assert!(rows[0].width < rows[18].width);
        assert!(rows.iter().all(|r| i32::from(r.x) * 2 + i32::from(r.width) == 100));
        let middle = rows.last().unwrap();
        assert_eq!((middle.y, middle.height, middle.width), (10, 40, 100));
        // Bottom rows mirror the top ones.
        assert_eq!(rows[1].y, 59);
        assert_eq!(rows[0].width, rows[1].width);
    }

    #[test]
    fn radius_is_limited_to_the_window() {
        let rows = rounded_rows(10, 10, 50);
        assert!(rows.iter().all(|r| r.y >= 0 && r.y < 10));
        let square = rounded_rows(10, 10, 0);
        assert_eq!(square.len(), 1);
        assert_eq!(
            (square[0].x, square[0].y, square[0].width, square[0].height),
            (0, 0, 10, 10)
        );
    }
}
