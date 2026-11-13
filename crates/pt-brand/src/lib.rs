//! Paneltarium's app icon: a white ringed planet centred in a display, on a
//! rounded purple-gradient tile. Drawn in code from the open-source Phosphor icon font,
//! so it is sharp at every size (16 px tray icon up to 256 px `.ico`) and
//! there are no image files to keep in sync.
//!
//! Used by the tray icon, the overlay header and the profile manager's
//! window icon and header.

use ab_glyph::{Font, FontRef, PxScale, point};

/// Top and bottom colours of the tile (deep violet → Paneltarium purple).
pub const SPACE_TOP: [u8; 3] = [0x2E, 0x10, 0x65];
pub const SPACE_BOTTOM: [u8; 3] = [0x7C, 0x3A, 0xED];
/// The display outline and the planet.
pub const WHITE: [u8; 3] = [0xFF, 0xFF, 0xFF];
/// The one-colour icon on light panels.
pub const BLACK: [u8; 3] = [0x00, 0x00, 0x00];

/// Where the screen opening is inside Phosphor's bold "monitor" glyph, as
/// fractions of the glyph's height (measured from the font: the opening
/// spans 11.8 %–70.6 % top to bottom, and is centred left-to-right).
const SCREEN_TOP: f32 = 0.118;
const SCREEN_BOTTOM: f32 = 0.706;
/// How much of the opening's height the planet fills (the rest is a margin
/// so it never touches the frame, even at 16 px).
const PLANET_FILL: f32 = 0.72;

/// Straight (not premultiplied) RGBA, 0..=255 colour, 0..=1 alpha.
type Px = [f32; 4];

/// The app icon as `size`×`size` straight RGBA8 (row-major).
pub fn icon_rgba(size: u32) -> Vec<u8> {
    let mut img = Image::new(size as usize);
    let s = size as f32;
    img.tile(s * 0.22);
    img.display_with_planet(Box2::centered(s / 2.0, s * 0.52, s * 0.80), WHITE, WHITE);
    img.into_rgba8()
}

/// One-colour ("symbolic") icon: display and planet both in `color`, no
/// tile, like the other icons in GNOME's and KDE's top bars and panels.
pub fn symbolic_rgba(size: u32, color: [u8; 3]) -> Vec<u8> {
    let mut img = Image::new(size as usize);
    let s = size as f32;
    img.display_with_planet(Box2::centered(s / 2.0, s / 2.0, s), color, color);
    img.into_rgba8()
}

/// Same icon, for drawing onto an existing dark background (overlay
/// header, manager header): just the display (in `display` colour) with
/// the white planet, no tile.
pub fn glyph_rgba(size: u32, display: [u8; 3]) -> Vec<u8> {
    let mut img = Image::new(size as usize);
    let s = size as f32;
    img.display_with_planet(Box2::centered(s / 2.0, s / 2.0, s), display, WHITE);
    img.into_rgba8()
}

struct Glyph {
    font: FontRef<'static>,
    ch: char,
}

fn glyph(text: &'static str, bytes: &'static [u8]) -> Glyph {
    Glyph {
        font: FontRef::try_from_slice(bytes).expect("Phosphor font is valid"),
        ch: text.chars().next().expect("glyph string is not empty"),
    }
}

#[derive(Debug, Clone, Copy)]
struct Box2 {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
}

impl Box2 {
    /// A square of side `side` centred on (cx, cy).
    fn centered(cx: f32, cy: f32, side: f32) -> Self {
        Box2 {
            x0: cx - side / 2.0,
            y0: cy - side / 2.0,
            x1: cx + side / 2.0,
            y1: cy + side / 2.0,
        }
    }
}

struct Image {
    n: usize,
    px: Vec<Px>,
}

impl Image {
    fn new(n: usize) -> Self {
        Image {
            n,
            px: vec![[0.0; 4]; n * n],
        }
    }

    /// Paint `color` at (x, y) with `coverage` (0..1), "over" compositing.
    fn blend(&mut self, x: i32, y: i32, color: [u8; 3], coverage: f32) {
        let n = self.n as i32;
        if !(0..n).contains(&x) || !(0..n).contains(&y) {
            return;
        }
        let a = coverage.clamp(0.0, 1.0);
        if a <= 0.0 {
            return;
        }
        let p = &mut self.px[(y * n + x) as usize];
        let out_a = a + p[3] * (1.0 - a);
        for i in 0..3 {
            p[i] = (color[i] as f32 * a + p[i] * p[3] * (1.0 - a)) / out_a;
        }
        p[3] = out_a;
    }

    /// Rounded square with a vertical gradient and anti-aliased edges.
    fn tile(&mut self, radius: f32) {
        let s = self.n as f32;
        for y in 0..self.n {
            let t = y as f32 / (s - 1.0).max(1.0);
            let color: [u8; 3] = std::array::from_fn(|i| {
                (SPACE_TOP[i] as f32 + (SPACE_BOTTOM[i] as f32 - SPACE_TOP[i] as f32) * t) as u8
            });
            for x in 0..self.n {
                let (fx, fy) = (x as f32 + 0.5, y as f32 + 0.5);
                let dx = (radius - fx).max(fx - (s - radius)).max(0.0);
                let dy = (radius - fy).max(fy - (s - radius)).max(0.0);
                let dist = (dx * dx + dy * dy).sqrt() - radius;
                self.blend(x as i32, y as i32, color, 0.5 - dist);
            }
        }
    }

    /// The display outline fitted into `target`, with the planet centred in
    /// its screen. Returns (screen opening, planet) boxes, for tests.
    fn display_with_planet(&mut self, target: Box2, display: [u8; 3], planet: [u8; 3]) -> Option<(Box2, Box2)> {
        // Bold weight, so the outline survives 16 px.
        let monitor = glyph(egui_phosphor::bold::MONITOR, egui_phosphor::bytes::bold::FONT);
        let b = self.glyph(&monitor, target, display)?;
        let h = b.y1 - b.y0;
        let screen = Box2 {
            x0: b.x0,
            y0: b.y0 + h * SCREEN_TOP,
            x1: b.x1,
            y1: b.y0 + h * SCREEN_BOTTOM,
        };
        let (cx, cy) = ((screen.x0 + screen.x1) / 2.0, (screen.y0 + screen.y1) / 2.0);
        let planet_glyph = glyph(egui_phosphor::fill::PLANET, egui_phosphor::bytes::fill::FONT);
        let side = (screen.y1 - screen.y0) * PLANET_FILL;
        let p = self.glyph(&planet_glyph, Box2::centered(cx, cy, side), planet)?;
        Some((screen, p))
    }

    /// Draw `g` scaled to fit `target` (keeping its shape), centred there.
    /// Returns the pixel box it was drawn in.
    fn glyph(&mut self, g: &Glyph, target: Box2, color: [u8; 3]) -> Option<Box2> {
        // Measure at a reference size, then scale to fit.
        const REF: f32 = 100.0;
        let id = g.font.glyph_id(g.ch);
        let probe = g.font.outline_glyph(id.with_scale(PxScale::from(REF)))?;
        let pb = probe.px_bounds();
        let fit = ((target.x1 - target.x0) / pb.width()).min((target.y1 - target.y0) / pb.height());
        let outline = g
            .font
            .outline_glyph(id.with_scale_and_position(PxScale::from(REF * fit), point(0.0, 0.0)))?;
        let b = outline.px_bounds();
        let cx = (target.x0 + target.x1) / 2.0;
        let cy = (target.y0 + target.y1) / 2.0;
        // Whole-pixel offsets keep thin strokes crisp at small sizes.
        let ox = (cx - b.width() / 2.0).round() as i32;
        let oy = (cy - b.height() / 2.0).round() as i32;
        let mut cov = Vec::new();
        outline.draw(|gx, gy, c| cov.push((gx as i32 + ox, gy as i32 + oy, c)));
        for (x, y, c) in cov {
            self.blend(x, y, color, c);
        }
        Some(Box2 {
            x0: ox as f32,
            y0: oy as f32,
            x1: ox as f32 + b.width(),
            y1: oy as f32 + b.height(),
        })
    }

    fn into_rgba8(self) -> Vec<u8> {
        self.px
            .iter()
            .flat_map(|p| {
                [
                    p[0].round() as u8,
                    p[1].round() as u8,
                    p[2].round() as u8,
                    (p[3] * 255.0).round() as u8,
                ]
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(rgba: &[u8], size: u32, x: u32, y: u32) -> [u8; 4] {
        let i = ((y * size + x) * 4) as usize;
        [rgba[i], rgba[i + 1], rgba[i + 2], rgba[i + 3]]
    }

    #[test]
    fn every_size_has_display_and_planet() {
        for size in [16, 20, 24, 32, 48, 64, 256] {
            let rgba = icon_rgba(size);
            assert_eq!(rgba.len(), (size * size * 4) as usize);
            assert_eq!(at(&rgba, size, 0, 0)[3], 0, "{size}px: corner is transparent");
            let px: Vec<&[u8; 4]> = rgba.as_chunks::<4>().0.iter().collect();
            let white = px.iter().filter(|p| p[0] > 200 && p[1] > 200 && p[2] > 200).count();
            let area = (size * size) as usize;
            assert!(white * 16 > area, "{size}px: display and planet drawn ({white} px)");
        }
    }

    #[test]
    fn tile_is_a_gradient() {
        let rgba = icon_rgba(64);
        let (top, bottom) = (at(&rgba, 64, 32, 1), at(&rgba, 64, 32, 62));
        assert_eq!(top[3], 255);
        assert!(
            bottom[0] > top[0] + 50,
            "lighter purple at the bottom: {top:?} → {bottom:?}"
        );
        assert!(top[2] > top[1] && bottom[2] > bottom[1], "purple, not blue-green");
    }

    #[test]
    fn glyph_only_version_is_transparent_around() {
        let rgba = glyph_rgba(48, [255, 255, 255]);
        assert_eq!(at(&rgba, 48, 0, 47)[3], 0);
        assert!(
            rgba.as_chunks::<4>()
                .0
                .iter()
                .any(|p| p[3] == 255 && p[..3] == [255, 255, 255]),
            "white planet"
        );
    }

    #[test]
    fn symbolic_version_uses_one_colour() {
        for color in [WHITE, BLACK] {
            let rgba = symbolic_rgba(32, color);
            let px = rgba.as_chunks::<4>().0;
            assert!(px.iter().all(|p| p[3] == 0 || p[..3] == color), "only {color:?}");
            assert!(
                px.iter().filter(|p| p[3] == 255).count() > 100,
                "display and planet are drawn"
            );
            assert_eq!(at(&rgba, 32, 0, 0)[3], 0, "no tile");
        }
    }

    #[test]
    fn planet_is_centred_in_the_screen() {
        for size in [16, 24, 32, 64, 256] {
            let s = size as f32;
            let mut img = Image::new(size as usize);
            let (screen, planet) = img
                .display_with_planet(Box2::centered(s / 2.0, s * 0.52, s * 0.80), WHITE, WHITE)
                .unwrap();
            let centre = |b: &Box2| ((b.x0 + b.x1) / 2.0, (b.y0 + b.y1) / 2.0);
            let ((sx, sy), (px, py)) = (centre(&screen), centre(&planet));
            // Whole-pixel snapping can shift it by up to a pixel.
            assert!(
                (sx - px).abs() <= 1.0 && (sy - py).abs() <= 1.0,
                "{size}px: screen ({sx}, {sy}) vs planet ({px}, {py})"
            );
            assert!(
                planet.y0 > screen.y0 && planet.y1 < screen.y1,
                "{size}px: planet clear of the frame"
            );
        }
    }

    #[test]
    fn screen_opening_matches_the_font() {
        // The constants are measured from the font; check the opening's
        // middle is empty and the frame just above it is drawn.
        let size = 256;
        let s = size as f32;
        let mut img = Image::new(size);
        let monitor = glyph(egui_phosphor::bold::MONITOR, egui_phosphor::bytes::bold::FONT);
        let b = img.glyph(&monitor, Box2::centered(s / 2.0, s / 2.0, s), WHITE).unwrap();
        let h = b.y1 - b.y0;
        let alpha = |x: f32, y: f32| img.px[y as usize * size + x as usize][3];
        let cx = (b.x0 + b.x1) / 2.0;
        assert!(
            alpha(cx, b.y0 + h * (SCREEN_TOP + SCREEN_BOTTOM) / 2.0) < 0.01,
            "opening is empty"
        );
        assert!(alpha(cx, b.y0 + h * SCREEN_TOP * 0.5) > 0.9, "top of the frame");
        assert!(
            alpha(cx, b.y0 + h * (SCREEN_BOTTOM + 0.04)) > 0.9,
            "bottom of the frame"
        );
        assert!(alpha(cx, b.y0 + h * (SCREEN_TOP + 0.02)) < 0.1, "just inside the top");
        assert!(
            alpha(cx, b.y0 + h * (SCREEN_BOTTOM - 0.02)) < 0.1,
            "just inside the bottom"
        );
    }
}
