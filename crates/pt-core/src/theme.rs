//! The accent colour (user-configurable) and the colours derived from it,
//! shared by the overlay and the profile manager. The rest of the dark theme
//! is fixed.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// An sRGB colour, written as `#RRGGBB` in `settings.toml`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Color {
    pub r: u8,
    pub g: u8,
    pub b: u8,
}

impl Color {
    pub const fn rgb(r: u8, g: u8, b: u8) -> Self {
        Color { r, g, b }
    }

    /// Paneltarium purple.
    pub const DEFAULT_ACCENT: Color = Color::rgb(0x7C, 0x3A, 0xED);
    /// Window background of the dark theme.
    pub const BACKGROUND: Color = Color::rgb(0x16, 0x17, 0x1B);
    pub const WHITE: Color = Color::rgb(0xFF, 0xFF, 0xFF);
    pub const BLACK: Color = Color::rgb(0x00, 0x00, 0x00);

    pub fn to_array(self) -> [u8; 3] {
        [self.r, self.g, self.b]
    }

    /// `self` blended towards `other` by `t` (0 = self, 1 = other).
    pub fn mix(self, other: Color, t: f32) -> Color {
        let t = t.clamp(0.0, 1.0);
        let m = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t).round() as u8;
        Color::rgb(m(self.r, other.r), m(self.g, other.g), m(self.b, other.b))
    }

    /// WCAG relative luminance, 0 (black) ..= 1 (white).
    pub fn luminance(self) -> f32 {
        let lin = |c: u8| {
            let c = c as f32 / 255.0;
            if c <= 0.04045 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
    }

    /// WCAG contrast ratio between two colours (1 ..= 21).
    pub fn contrast(self, other: Color) -> f32 {
        let (a, b) = (self.luminance(), other.luminance());
        (a.max(b) + 0.05) / (a.min(b) + 0.05)
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{:02X}{:02X}{:02X}", self.r, self.g, self.b)
    }
}

impl FromStr for Color {
    type Err = String;

    /// `#RRGGBB`, `RRGGBB` or the short `#RGB`.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let hex = s.trim().trim_start_matches('#');
        let bad = || format!("'{s}' is not a colour; use #RRGGBB, e.g. #7C3AED");
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let byte = |i: usize| u8::from_str_radix(&hex[i..i + 2], 16).map_err(|_| bad());
        match hex.len() {
            6 => Ok(Color::rgb(byte(0)?, byte(2)?, byte(4)?)),
            3 => {
                let nib = |i: usize| u8::from_str_radix(&hex[i..=i], 16).map(|v| v * 17).map_err(|_| bad());
                Ok(Color::rgb(nib(0)?, nib(1)?, nib(2)?))
            }
            _ => Err(bad()),
        }
    }
}

impl Serialize for Color {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for Color {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        String::deserialize(d)?.parse().map_err(serde::de::Error::custom)
    }
}

/// The colours that follow the accent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    /// Buttons, number badges, toggles, the selected row's outline.
    pub accent: Color,
    /// Text and icons drawn on the accent: white, or black on light accents
    /// (e.g. yellow) so labels stay readable.
    pub on_accent: Color,
    /// Background of the selected overlay row / open profile card: a dark
    /// tint of the accent.
    pub selected: Color,
}

impl Theme {
    pub fn new(accent: Color) -> Self {
        let on_accent = if accent.contrast(Color::WHITE) >= accent.contrast(Color::BLACK) {
            Color::WHITE
        } else {
            Color::BLACK
        };
        Theme {
            accent,
            on_accent,
            selected: Color::BACKGROUND.mix(accent, 0.2),
        }
    }
}

impl Default for Theme {
    fn default() -> Self {
        Theme::new(Color::DEFAULT_ACCENT)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_prints_hex() {
        assert_eq!("#7C3AED".parse::<Color>().unwrap(), Color::DEFAULT_ACCENT);
        assert_eq!("7c3aed".parse::<Color>().unwrap(), Color::DEFAULT_ACCENT);
        assert_eq!(" #f80 ".parse::<Color>().unwrap(), Color::rgb(0xFF, 0x88, 0x00));
        assert_eq!(Color::rgb(1, 2, 255).to_string(), "#0102FF");
        for bad in ["", "#12345", "#GGGGGG", "purple", "#7C3AED00", "#é1"] {
            assert!(bad.parse::<Color>().is_err(), "{bad:?}");
        }
    }

    #[test]
    fn text_on_the_accent_stays_readable() {
        assert_eq!(Theme::new(Color::DEFAULT_ACCENT).on_accent, Color::WHITE);
        assert_eq!(Theme::new(Color::rgb(0x25, 0x63, 0xEB)).on_accent, Color::WHITE);
        // Light accents get black text.
        assert_eq!(Theme::new(Color::rgb(0xFA, 0xCC, 0x15)).on_accent, Color::BLACK);
        assert_eq!(Theme::new(Color::WHITE).on_accent, Color::BLACK);
        for c in [
            Color::rgb(0xFA, 0xCC, 0x15),
            Color::DEFAULT_ACCENT,
            Color::rgb(0x22, 0xC5, 0x5E),
        ] {
            let t = Theme::new(c);
            assert!(c.contrast(t.on_accent) >= 3.0, "{c}: {}", c.contrast(t.on_accent));
        }
    }

    #[test]
    fn selected_tint_is_dark_and_follows_the_accent() {
        let purple = Theme::default().selected;
        assert_eq!(purple, Color::rgb(0x2A, 0x1E, 0x45));
        let green = Theme::new(Color::rgb(0x22, 0xC5, 0x5E)).selected;
        assert!(green.g > green.r && green.g > green.b, "{green}");
        assert!(green.luminance() < 0.08, "still dark: {green}");
    }
}
