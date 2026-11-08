//! The overlay's look. `draw` turns a [`View`] into pixels; it is pure, so
//! tests (and the PNG previews in the docs) use exactly the code the app does.

use super::canvas::{Canvas, Fonts, Rect, Rgb, measure};
use super::session::{Entry, EntryKind};
use pt_core::{Color, Theme};

pub const BG: Rgb = Rgb::hex(0x16171B);
const BORDER: Rgb = Rgb::hex(0x30333C);
const ROW: Rgb = Rgb::hex(0x1F2127);
const BADGE_RESCUE: Rgb = Rgb::hex(0x3A3F4B);
const TEXT: Rgb = Rgb::hex(0xF3F4F6);
const MUTED: Rgb = Rgb::hex(0x9CA3AF);
const WHITE: Rgb = Rgb::hex(0xFFFFFF);

// Layout in logical pixels (multiplied by the monitor's scale factor).
const WIDTH: f32 = 460.0;
const PAD: f32 = 20.0;
const HEADER: f32 = 86.0;
const ROW_H: f32 = 52.0;
const ROW_GAP: f32 = 6.0;
const OVERFLOW_H: f32 = 26.0;
const BOTTOM: f32 = 18.0;

pub struct View<'a> {
    pub entries: &'a [Entry],
    pub overflow: usize,
    pub hint: &'a str,
    /// Index of the highlighted row.
    pub selected: usize,
    /// Accent colour (from settings) and the colours that follow it.
    pub theme: Theme,
}

fn rgb(c: Color) -> Rgb {
    Rgb(c.r, c.g, c.b)
}

/// Corner radius of the panel in logical px. Matches Windows 11's rounded
/// window corners; the platform layer clips the window to the same shape.
pub const CORNER_RADIUS: f32 = 8.0;

/// Size in physical pixels for a monitor with this scale factor.
pub fn size(view: &View, scale: f64) -> (u32, u32) {
    let n = view.entries.len() as f32;
    let mut h = HEADER + n * ROW_H + (n - 1.0).max(0.0) * ROW_GAP + BOTTOM;
    if view.overflow > 0 {
        h += OVERFLOW_H;
    }
    let s = scale as f32;
    ((WIDTH * s).round() as u32, (h * s).round() as u32)
}

/// The scale to draw at on a monitor: its own scale factor, shrunk if the
/// panel wouldn't fit in 90 % of the monitor (e.g. ten profiles on a
/// 1366×768 laptop at 125 %, or a small portrait screen).
pub fn fit_scale(view: &View, monitor_scale: f64, monitor_w: u32, monitor_h: u32) -> f64 {
    let (w1, h1) = size(view, 1.0);
    let room = |px: u32, need: u32| (px as f64 * 0.9) / need.max(1) as f64;
    let fit = room(monitor_w, w1).min(room(monitor_h, h1));
    // Never below half size: past that the text is unreadable anyway.
    monitor_scale.min(fit).max(0.5)
}

/// Which row is at physical pixel (`x`, `y`) in a window `width` px wide,
/// if any. Uses the same layout constants as [`draw`].
pub fn row_at(rows: usize, scale: f64, width: u32, x: f64, y: f64) -> Option<usize> {
    let s = scale as f32;
    let (x, y) = (x as f32 / s, y as f32 / s);
    let w = width as f32 / s;
    if x < PAD || x > w - PAD || y < HEADER {
        return None;
    }
    let offset = y - HEADER;
    let i = (offset / (ROW_H + ROW_GAP)) as usize;
    let within = offset - i as f32 * (ROW_H + ROW_GAP);
    (i < rows && within <= ROW_H).then_some(i)
}

pub fn draw(view: &View, fonts: &Fonts, width: u32, height: u32, scale: f64) -> Canvas {
    let s = scale as f32;
    let (w, h) = (width as f32, height as f32);
    let mut c = Canvas::new(width as usize, height as usize, BG);
    // Rounded border; the window is clipped to the same shape by the OS.
    c.rounded_outline(Rect::new(0.0, 0.0, w, h), CORNER_RADIUS * s, s.max(1.0), BORDER, BG);

    // Header: icon + title, then the hint line.
    // App icon (display + planet) in white, then the name.
    let icon_px = (26.0 * s).round() as u32;
    c.image(
        &pt_brand::glyph_rgba(icon_px, [WHITE.0, WHITE.1, WHITE.2]),
        icon_px,
        PAD * s,
        21.0 * s,
    );
    let icon_w = icon_px as f32;
    c.text(
        &fonts.text,
        21.0 * s,
        PAD * s + icon_w + 10.0 * s,
        42.0 * s,
        "Paneltarium",
        TEXT,
    );
    c.text_fit(
        &fonts.text,
        13.5 * s,
        (PAD * s, 68.0 * s),
        view.hint,
        w - 2.0 * PAD * s,
        MUTED,
    );

    let mut y = HEADER * s;
    for (i, e) in view.entries.iter().enumerate() {
        draw_row(&mut c, fonts, e, (i == view.selected, &view.theme), y, s, w);
        y += (ROW_H + ROW_GAP) * s;
    }
    if view.overflow > 0 {
        let more = format!(
            "+{} more profile(s) — give them a hotkey with a number to choose their slot",
            view.overflow
        );
        c.text_fit(
            &fonts.text,
            12.5 * s,
            (PAD * s, y + 12.0 * s),
            &more,
            w - 2.0 * PAD * s,
            MUTED,
        );
    }
    c
}

/// `state`: whether this row is highlighted, and the colours to use.
fn draw_row(c: &mut Canvas, fonts: &Fonts, e: &Entry, state: (bool, &Theme), y: f32, s: f32, w: f32) {
    let (selected, theme) = state;
    let (x, row_w, row_h) = (PAD * s, w - 2.0 * PAD * s, ROW_H * s);
    let rescue = e.kind == EntryKind::Duplicate;
    let row = Rect::new(x, y, row_w, row_h);
    if selected {
        c.rounded_outline(row, 10.0 * s, 1.5 * s, rgb(theme.accent), rgb(theme.selected));
    } else {
        c.rounded_rect(row, 10.0 * s, ROW);
    }

    // Number badge.
    let badge = 32.0 * s;
    let (bx, by) = (x + 10.0 * s, y + (row_h - badge) / 2.0);
    let (badge_color, number_color) = if rescue {
        (BADGE_RESCUE, WHITE)
    } else {
        (rgb(theme.accent), rgb(theme.on_accent))
    };
    c.rounded_rect(Rect::new(bx, by, badge, badge), 8.0 * s, badge_color);
    let num = e.number.to_string();
    let nsize = 18.0 * s;
    let nw = measure(&fonts.text, nsize, &num);
    c.text(
        &fonts.text,
        nsize,
        bx + (badge - nw) / 2.0,
        by + badge * 0.70,
        &num,
        number_color,
    );

    // Right side, drawn right-to-left: hotkey (or the duplicate icon), then a
    // check mark on the current profile. The title avoids whatever is there.
    let mut right = x + row_w - 14.0 * s;
    let mut text_right = right;
    if let Some(hk) = &e.hotkey {
        let size = 12.5 * s;
        let hw = measure(&fonts.text, size, hk);
        c.text(&fonts.text, size, right - hw, y + row_h * 0.60, hk, MUTED);
        text_right = right - hw - 12.0 * s;
    } else if rescue {
        let size = 18.0 * s;
        let iw = measure(&fonts.icons, size, egui_phosphor::fill::COPY);
        c.text(
            &fonts.icons,
            size,
            right - iw,
            y + row_h * 0.66,
            egui_phosphor::fill::COPY,
            MUTED,
        );
        text_right = right - iw - 12.0 * s;
    }
    if e.current {
        right = text_right;
        let size = 18.0 * s;
        let icon = egui_phosphor::fill::CHECK_CIRCLE;
        let iw = measure(&fonts.icons, size, icon);
        c.text(
            &fonts.icons,
            size,
            right - iw,
            y + row_h * 0.66,
            icon,
            rgb(theme.accent),
        );
        text_right = right - iw - 10.0 * s;
    }

    // Title and one-line summary.
    let tx = bx + badge + 12.0 * s;
    let avail = text_right - tx;
    c.text_fit(&fonts.text, 16.5 * s, (tx, y + 23.0 * s), &e.title, avail, TEXT);
    c.text_fit(&fonts.text, 12.0 * s, (tx, y + 41.0 * s), &e.detail, avail, MUTED);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::overlay::session::entries;
    use pt_core::testing::sample_layout;
    use pt_core::{Modifiers, Profile};

    fn view_entries() -> Vec<Entry> {
        let mut a = Profile::new("LG only", sample_layout()).unwrap();
        a.hotkey = Some("Ctrl+Alt+1".parse().unwrap());
        let b = Profile::new("TV only", sample_layout()).unwrap();
        let mods = Modifiers {
            ctrl: true,
            alt: true,
            ..Default::default()
        };
        entries(&[a, b], mods, &pt_core::current::Current::Profile("TV only".into())).0
    }

    #[test]
    fn size_scales_with_dpi() {
        let e = view_entries();
        let v = View {
            entries: &e,
            overflow: 0,
            hint: "x",
            selected: 0,
            theme: Theme::default(),
        };
        let (w1, h1) = size(&v, 1.0);
        let (w2, h2) = size(&v, 2.0);
        assert_eq!((w2, h2), (w1 * 2, h1 * 2));
        assert_eq!(w1, 460);
    }

    #[test]
    fn draws_badges_and_selected_row() {
        let fonts = Fonts::load();
        let e = view_entries();
        let v = View {
            entries: &e,
            overflow: 2,
            hint: "Hold Ctrl+Alt · press a number or arrows + Enter · let go to cancel",
            selected: 1,
            theme: Theme::default(),
        };
        let (w, h) = size(&v, 1.0);
        let c = draw(&v, &fonts, w, h, 1.0);
        assert_eq!(c.pixels.len(), (w * h) as usize);
        assert_eq!(c.get(w as usize / 2, 0), BORDER, "top border");
        assert_eq!(c.get(0, 0), BG, "outside the rounded corner");
        // Middle of the first row's badge edge is accent purple.
        let badge_y = (HEADER + ROW_H / 2.0) as usize;
        assert_eq!(c.get((PAD + 12.0) as usize, badge_y), rgb(Color::DEFAULT_ACCENT));
        // Second row is selected: tinted background.
        let row2_y = (HEADER + ROW_H + ROW_GAP + 4.0) as usize;
        assert_eq!(c.get(200, row2_y), rgb(Theme::default().selected));
        // First row is not.
        assert_eq!(c.get(200, (HEADER + 4.0) as usize), ROW);
    }

    #[test]
    fn accent_follows_the_setting_and_numbers_stay_readable() {
        let fonts = Fonts::load();
        let e = view_entries();
        let yellow = Color::rgb(0xFA, 0xCC, 0x15);
        let v = View {
            entries: &e,
            overflow: 0,
            hint: "",
            selected: 1,
            theme: Theme::new(yellow),
        };
        let (w, h) = size(&v, 1.0);
        let c = draw(&v, &fonts, w, h, 1.0);
        let badge_y = (HEADER + ROW_H / 2.0) as usize;
        assert_eq!(c.get((PAD + 12.0) as usize, badge_y), rgb(yellow));
        let row2_y = (HEADER + ROW_H + ROW_GAP + 4.0) as usize;
        assert_eq!(c.get(200, row2_y), rgb(Theme::new(yellow).selected));
        // The "1" on the yellow badge is drawn in black, not white.
        let badge = (PAD + 10.0, HEADER + (ROW_H - 32.0) / 2.0);
        let dark = (0..32)
            .flat_map(|dy| (0..32).map(move |dx| (dx, dy)))
            .filter(|&(dx, dy)| c.get((badge.0 + dx as f32) as usize, (badge.1 + dy as f32) as usize).0 < 60)
            .count();
        assert!(dark > 10, "black digit on the yellow badge ({dark} px)");
    }

    #[test]
    fn hints_fit_without_truncation() {
        use crate::overlay::session::{Input, Opener, Session};
        use pt_core::current::Current;
        let fonts = Fonts::load();
        let mods = Modifiers {
            ctrl: true,
            alt: true,
            shift: true,
            win: false,
        };
        let mut sessions = vec![
            Session::new(&[], mods, &Current::Unknown, Opener::Keyboard, true),
            Session::new(&[], mods, &Current::Unknown, Opener::Keyboard, false),
            Session::new(&[], mods, &Current::Unknown, Opener::Gamepad, true),
        ];
        let mut pointer = Session::new(&[], mods, &Current::Unknown, Opener::Keyboard, true);
        pointer.handle(Input::Hover(0));
        sessions.push(pointer);
        for s in sessions {
            let hint = s.hint();
            let w = measure(&fonts.text, 13.5, &hint);
            assert!(w <= WIDTH - 2.0 * PAD, "hint too wide ({w}px): {hint}");
        }
    }

    #[test]
    fn hit_testing_matches_layout() {
        let w = (WIDTH * 1.5) as u32;
        let row_y = |i: usize| (HEADER + i as f32 * (ROW_H + ROW_GAP) + ROW_H / 2.0) as f64;
        assert_eq!(row_at(4, 1.0, WIDTH as u32, 100.0, row_y(0)), Some(0));
        assert_eq!(row_at(4, 1.0, WIDTH as u32, 100.0, row_y(3)), Some(3));
        assert_eq!(
            row_at(4, 1.0, WIDTH as u32, 100.0, row_y(4)),
            None,
            "below the last row"
        );
        assert_eq!(row_at(4, 1.0, WIDTH as u32, 100.0, 40.0), None, "header");
        assert_eq!(row_at(4, 1.0, WIDTH as u32, 5.0, row_y(1)), None, "left margin");
        let gap = (HEADER + ROW_H + ROW_GAP / 2.0) as f64;
        assert_eq!(row_at(4, 1.0, WIDTH as u32, 100.0, gap), None, "gap between rows");
        assert_eq!(row_at(4, 1.5, w, 150.0, row_y(2) * 1.5), Some(2), "scaled");
    }

    #[test]
    fn panel_shrinks_to_fit_small_monitors() {
        let entries: Vec<Entry> = (0..10u8)
            .map(|i| Entry {
                number: i,
                kind: crate::overlay::session::EntryKind::Profile(format!("P{i}")),
                title: format!("P{i}"),
                detail: "LG 2560x1080 @ 75 Hz".into(),
                hotkey: None,
                current: false,
            })
            .collect();
        let view = View {
            entries: &entries,
            overflow: 3,
            hint: "",
            selected: 0,
            theme: Theme::default(),
        };
        // Big monitor: its own scale is used.
        assert_eq!(fit_scale(&view, 1.5, 3840, 2160), 1.5);
        // 1366x768 at 125 %: shrunk until the panel fits with a margin.
        let s = fit_scale(&view, 1.25, 1366, 768);
        assert!(s < 1.25, "{s}");
        let (w, h) = size(&view, s);
        assert!(h as f64 <= 768.0 * 0.9 + 1.0 && w <= 1366, "{w}x{h}");
        // Portrait 1080x1920 at 100 %: fits as is.
        assert_eq!(fit_scale(&view, 1.0, 1080, 1920), 1.0);
        // Absurdly small (e.g. a 640x480 capture device): stops at half size.
        assert_eq!(fit_scale(&view, 1.0, 320, 240), 0.5);
    }

    #[test]
    fn hint_glyphs_exist_in_font() {
        use ab_glyph::Font;
        let fonts = Fonts::load();
        for ch in ['·', '…', '—', '+'] {
            assert_ne!(fonts.text.glyph_id(ch).0, 0, "{ch} missing from the UI font");
        }
        for icon in [egui_phosphor::fill::COPY, egui_phosphor::fill::CHECK_CIRCLE] {
            let ch = icon.chars().next().unwrap();
            assert_ne!(fonts.icons.glyph_id(ch).0, 0);
        }
    }
}
