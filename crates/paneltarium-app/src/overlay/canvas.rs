//! A tiny software canvas: anti-aliased rounded rectangles and text into a
//! `0x00RRGGBB` pixel buffer (the format `softbuffer` presents). Pure Rust,
//! identical on every OS.

use ab_glyph::{Font, FontRef, PxScale, ScaleFont, point};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub const fn hex(v: u32) -> Rgb {
        Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    fn pack(self) -> u32 {
        (self.0 as u32) << 16 | (self.1 as u32) << 8 | self.2 as u32
    }
}

/// A rectangle in pixels.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }

    fn inset(self, d: f32) -> Rect {
        Rect::new(self.x + d, self.y + d, self.w - 2.0 * d, self.h - 2.0 * d)
    }
}

pub struct Fonts {
    pub text: FontRef<'static>,
    pub icons: FontRef<'static>,
}

impl Fonts {
    /// egui's default UI font (Ubuntu) and Phosphor icons (fill).
    pub fn load() -> Fonts {
        Fonts {
            text: FontRef::try_from_slice(epaint_default_fonts::UBUNTU_LIGHT).expect("bundled font is valid"),
            icons: FontRef::try_from_slice(egui_phosphor::bytes::fill::FONT).expect("bundled font is valid"),
        }
    }
}

pub struct Canvas {
    pub width: usize,
    pub height: usize,
    pub pixels: Vec<u32>,
}

impl Canvas {
    pub fn new(width: usize, height: usize, fill: Rgb) -> Canvas {
        Canvas {
            width,
            height,
            pixels: vec![fill.pack(); width * height],
        }
    }

    #[cfg(test)]
    pub fn get(&self, x: usize, y: usize) -> Rgb {
        let v = self.pixels[y * self.width + x];
        Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8)
    }

    fn blend(&mut self, x: i32, y: i32, c: Rgb, alpha: f32) {
        if x < 0 || y < 0 || x as usize >= self.width || y as usize >= self.height || alpha <= 0.0 {
            return;
        }
        let a = alpha.min(1.0);
        let i = y as usize * self.width + x as usize;
        let old = self.pixels[i];
        let mix = |o: u32, n: u8| ((o & 0xFF) as f32 * (1.0 - a) + n as f32 * a).round() as u32;
        self.pixels[i] = mix(old >> 16, c.0) << 16 | mix(old >> 8, c.1) << 8 | mix(old, c.2);
    }

    /// Draw a square straight-RGBA8 image (e.g. the app icon) at (x, y).
    pub fn image(&mut self, rgba: &[u8], size: u32, x: f32, y: f32) {
        let (x0, y0) = (x.round() as i32, y.round() as i32);
        for (i, px) in rgba.chunks_exact(4).enumerate() {
            let (ix, iy) = ((i as u32 % size) as i32, (i as u32 / size) as i32);
            self.blend(x0 + ix, y0 + iy, Rgb(px[0], px[1], px[2]), px[3] as f32 / 255.0);
        }
    }

    /// Filled rounded rectangle with anti-aliased edges.
    pub fn rounded_rect(&mut self, rect: Rect, radius: f32, c: Rgb) {
        let Rect { x, y, w, h } = rect;
        let r = radius.min(w / 2.0).min(h / 2.0);
        let (x0, y0) = (x.floor() as i32, y.floor() as i32);
        let (x1, y1) = ((x + w).ceil() as i32, (y + h).ceil() as i32);
        for py in y0..y1 {
            for px in x0..x1 {
                let (fx, fy) = (px as f32 + 0.5, py as f32 + 0.5);
                let dx = (x + r - fx).max(fx - (x + w - r)).max(0.0);
                let dy = (y + r - fy).max(fy - (y + h - r)).max(0.0);
                let outside_corner = (dx * dx + dy * dy).sqrt() - r;
                let outside_edge = (x - fx).max(fx - (x + w)).max(y - fy).max(fy - (y + h));
                let dist = if dx > 0.0 && dy > 0.0 {
                    outside_corner
                } else {
                    outside_edge
                };
                self.blend(px, py, c, (0.5 - dist).clamp(0.0, 1.0));
            }
        }
    }

    /// A rounded outline `width` px thick: outer shape in `c`, inside in `inner`.
    pub fn rounded_outline(&mut self, rect: Rect, radius: f32, width: f32, c: Rgb, inner: Rgb) {
        self.rounded_rect(rect, radius, c);
        self.rounded_rect(rect.inset(width), (radius - width).max(0.0), inner);
    }

    /// Draw `text` with its baseline at `baseline`. Returns the advance width.
    pub fn text(&mut self, font: &FontRef<'_>, size: f32, x: f32, baseline: f32, text: &str, c: Rgb) -> f32 {
        let scaled = font.as_scaled(PxScale::from(size));
        let mut caret = x;
        let mut prev = None;
        for ch in text.chars() {
            let id = scaled.glyph_id(ch);
            if let Some(p) = prev {
                caret += scaled.kern(p, id);
            }
            let glyph = id.with_scale_and_position(PxScale::from(size), point(caret, baseline));
            if let Some(outline) = font.outline_glyph(glyph) {
                let b = outline.px_bounds();
                outline.draw(|gx, gy, cov| {
                    // Slightly thicken the light font so it reads on dark backgrounds.
                    let a = cov.clamp(0.0, 1.0).powf(0.75);
                    self.blend(b.min.x as i32 + gx as i32, b.min.y as i32 + gy as i32, c, a);
                });
            }
            caret += scaled.h_advance(id);
            prev = Some(id);
        }
        caret - x
    }

    /// Like [`Canvas::text`] (at `pos` = x, baseline) but cut with "…" to fit `max_width`.
    pub fn text_fit(&mut self, font: &FontRef<'_>, size: f32, pos: (f32, f32), text: &str, max_width: f32, c: Rgb) {
        let fitted = fit(font, size, text, max_width);
        self.text(font, size, pos.0, pos.1, &fitted, c);
    }
}

/// Width of `text` in pixels at `size`.
pub fn measure(font: &FontRef<'_>, size: f32, text: &str) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    let mut w = 0.0;
    let mut prev = None;
    for ch in text.chars() {
        let id = scaled.glyph_id(ch);
        if let Some(p) = prev {
            w += scaled.kern(p, id);
        }
        w += scaled.h_advance(id);
        prev = Some(id);
    }
    w
}

/// Shorten `text` with a trailing "…" until it fits `max_width`.
pub fn fit(font: &FontRef<'_>, size: f32, text: &str, max_width: f32) -> String {
    if measure(font, size, text) <= max_width {
        return text.to_string();
    }
    let mut chars: Vec<char> = text.chars().collect();
    while !chars.is_empty() {
        chars.pop();
        let candidate: String = chars.iter().collect::<String>().trim_end().to_string() + "…";
        if measure(font, size, &candidate) <= max_width {
            return candidate;
        }
    }
    "…".into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rect_and_text_draw_where_expected() {
        let fonts = Fonts::load();
        let bg = Rgb::hex(0x101010);
        let red = Rgb::hex(0xFF0000);
        let mut c = Canvas::new(100, 40, bg);
        c.rounded_rect(Rect::new(10.0, 10.0, 30.0, 20.0), 6.0, red);
        assert_eq!(c.get(25, 20), red, "inside");
        assert_eq!(c.get(10, 10), bg, "rounded corner stays background");
        assert_eq!(c.get(50, 20), bg, "outside");

        let w = c.text(&fonts.text, 16.0, 45.0, 30.0, "Hi", Rgb::hex(0xFFFFFF));
        assert!(w > 8.0 && w < 30.0, "{w}");
        // The light font never reaches full coverage at this size; look for bright pixels.
        assert!(c.pixels.iter().any(|&p| (p & 0xFF) > 0xC0), "text drew bright pixels");
    }

    #[test]
    fn fit_adds_ellipsis() {
        let fonts = Fonts::load();
        let long = "A very long profile name that will not fit";
        let short = fit(&fonts.text, 16.0, long, 100.0);
        assert!(short.ends_with('…') && short.len() < long.len(), "{short}");
        assert!(measure(&fonts.text, 16.0, &short) <= 100.0);
        assert_eq!(fit(&fonts.text, 16.0, "Desk", 100.0), "Desk");
    }
}
