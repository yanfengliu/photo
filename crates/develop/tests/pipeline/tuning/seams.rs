//! Two checks across views: a render assembled from tiles against the
//! whole (with their difference amplified), and fine-texture contrast on
//! either side of a pyramid level.

use std::path::Path;

use photo_core::geom::Rect;
use photo_core::image::Rgba8Image;
use photo_develop::Renderer;
use photo_imaging::View;

use super::{Scenes, Sheet, shot};
use crate::fixtures::{full, luma};
use crate::region::everything;

pub fn sheets(scenes: &Scenes, dir: &Path) {
    tiles(scenes, dir);
    pyramid_crossing(scenes);
}

/// The whole 1:1 window, the same window assembled from four tiles, and
/// their difference times 64 (any seam would show as a line).
fn tiles(scenes: &Scenes, dir: &Path) {
    let mut renderer = Renderer::new();
    let settings = everything();
    let source = &scenes.raster;
    let view = full(source, &settings);
    let window = (500u32, 400u32, 360u32, 240u32);
    let whole = shot(
        &mut renderer,
        source,
        &settings,
        view.tile(window.0, window.1, window.2, window.3),
    );
    let mut assembled = whole.clone();
    let (half_w, half_h) = (window.2 / 2 + 13, window.3 / 2 - 7);
    for (x, y, w, h) in [
        (0, 0, half_w, half_h),
        (half_w, 0, window.2 - half_w, half_h),
        (0, half_h, half_w, window.3 - half_h),
        (half_w, half_h, window.2 - half_w, window.3 - half_h),
    ] {
        let tile = shot(
            &mut renderer,
            source,
            &settings,
            view.tile(window.0 + x, window.1 + y, w, h),
        );
        for ty in 0..h {
            for tx in 0..w {
                let at = (((y + ty) * window.2 + x + tx) * 4) as usize;
                assembled.pixels[at..at + 4].copy_from_slice(&tile.pixel(tx, ty));
            }
        }
    }
    let mut difference = whole.clone();
    let mut worst = 0u8;
    let pairs = whole.pixels.chunks(4).zip(assembled.pixels.chunks(4));
    for (out, (a, b)) in difference.pixels.chunks_mut(4).zip(pairs) {
        for c in 0..3 {
            let d = a[c].abs_diff(b[c]);
            worst = worst.max(d);
            out[c] = d.saturating_mul(64);
        }
    }
    println!("tiles: largest difference from the whole render {worst} codes");
    let mut sheet = Sheet::new("R1: WHOLE / FOUR TILES / DIFFERENCE X64 (EVERYTHING ON)", 3);
    sheet.add("WHOLE", whole);
    sheet.add("FOUR TILES", assembled);
    sheet.add(format!("DIFFERENCE X64 (MAX {worst})"), difference);
    sheet.save(dir, "15_tiles");
}

/// The spread of fine detail over the textured ground at view scales just
/// either side of each pyramid level, printed.
fn pyramid_crossing(scenes: &Scenes) {
    let mut renderer = Renderer::new();
    let settings = photo_core::develop::DevelopSettings::default();
    let source = &scenes.raster;
    // A window of the ground, 600 x 300 output pixels.
    let roi = Rect::new(200.0, 620.0, 800.0, 920.0);
    for scale in [0.27f32, 0.2499, 0.52, 0.4999, 1.0] {
        let (w, h) = (
            (600.0 * scale).round() as u32,
            (300.0 * scale).round() as u32,
        );
        let image = shot(&mut renderer, source, &settings, View::new(roi, w, h));
        println!("scale {scale:.4}: fine detail {:.3}", fine(&image));
    }
}

/// Spread of the difference between each pixel and its neighbours' mean.
fn fine(image: &Rgba8Image) -> f32 {
    let mut values = Vec::new();
    for y in 1..image.height - 1 {
        for x in 1..image.width - 1 {
            let around = (luma(image, x - 1, y - 1, 3, 3) * 9.0 - luma(image, x, y, 1, 1)) / 8.0;
            values.push(luma(image, x, y, 1, 1) - around);
        }
    }
    let mean = values.iter().sum::<f32>() / values.len() as f32;
    (values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32).sqrt()
}
