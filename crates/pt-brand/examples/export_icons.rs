//! Writes the icon files kept in the repo:
//! `assets/paneltarium.ico` (16–256 px, for the Windows exe/installer),
//! `assets/paneltarium-256.png` and `docs/icon-preview.png`.
//!
//! `cargo run -p pt-brand --example export_icons`

use std::path::Path;

use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::{ExtendedColorType, RgbaImage};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    std::fs::create_dir_all(root.join("assets"))?;

    let frames = [16, 20, 24, 32, 40, 48, 64, 128, 256]
        .into_iter()
        .map(|s| IcoFrame::as_png(&pt_brand::icon_rgba(s), s, s, ExtendedColorType::Rgba8))
        .collect::<Result<Vec<_>, _>>()?;
    IcoEncoder::new(std::fs::File::create(root.join("assets/paneltarium.ico"))?).encode_images(&frames)?;

    RgbaImage::from_raw(256, 256, pt_brand::icon_rgba(256))
        .ok_or("bad buffer")?
        .save(root.join("assets/paneltarium-256.png"))?;

    // Preview sheet: each size at 1:1 on a dark and a light strip.
    let sizes = [16u32, 24, 32, 48, 64, 128];
    let (w, h) = (
        sizes.iter().sum::<u32>() + 16 * (sizes.len() as u32 + 1),
        2 * (128 + 32),
    );
    let mut sheet = RgbaImage::new(w, h);
    for (y, row) in sheet.rows_mut().enumerate() {
        let bg = if y < (h / 2) as usize {
            [22, 23, 27, 255]
        } else {
            [240, 240, 243, 255]
        };
        for p in row {
            p.0 = bg;
        }
    }
    for strip in 0..2 {
        let mut x = 16;
        for &s in &sizes {
            let icon = pt_brand::icon_rgba(s);
            let y0 = strip * (h / 2) + 16 + (128 - s) / 2;
            for (i, px) in icon.chunks(4).enumerate() {
                let (ix, iy) = (x + i as u32 % s, y0 + i as u32 / s);
                let a = px[3] as f32 / 255.0;
                let d = sheet.get_pixel_mut(ix, iy);
                for (dst, &src) in d.0.iter_mut().zip(&px[..3]) {
                    *dst = (src as f32 * a + *dst as f32 * (1.0 - a)).round() as u8;
                }
            }
            x += s + 16;
        }
    }
    sheet.save(root.join("docs/icon-preview.png"))?;
    println!("wrote assets/paneltarium.ico, assets/paneltarium-256.png, docs/icon-preview.png");
    Ok(())
}
