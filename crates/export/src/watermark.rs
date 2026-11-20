//! The text watermark: a line of the built-in stroke font laid out at an
//! anchor, sized relative to the picture, and blended in linear light.
//!
//! Every pixel's coverage is computed from its distance to the strokes, so
//! edges are antialiased at any size and a pixel's value depends only on its
//! position in the picture, never on which strip it was rendered in.

use photo_core::colorimetry::{RgbSpace, mat3_apply, rgb_to_rgb};
use photo_core::geom::Point;
use rayon::prelude::*;

use crate::font::{self, CAP_HEIGHT, GAP, Glyph, HALF_STROKE};
use crate::settings::Watermark;

/// Margin between the text and the picture's edges, as a fraction of the
/// shorter side.
const INSET: f32 = 0.025;
/// Highest and lowest font units any glyph reaches (brackets, the
/// underscore), for the region a watermark can touch.
const REACH_UP: f32 = -1.0;
const REACH_DOWN: f32 = 14.0;

/// A watermark laid out on one picture.
#[derive(Clone, Debug)]
pub(crate) struct WatermarkLayer {
    glyphs: Vec<(f32, &'static Glyph)>,
    /// Picture position of the font's origin (left end of the text, top of
    /// the capitals), in pixels.
    origin: Point,
    /// Pixels per font unit.
    unit: f32,
    /// Columns and rows the text can touch, clamped to the picture.
    columns: (u32, u32),
    rows: (u32, u32),
    alpha: f32,
    /// The colour, linear, in the picture's colour space.
    color: [f32; 3],
    space: RgbSpace,
}

impl WatermarkLayer {
    /// `watermark` laid out on a picture of `size` encoded in `space`, or
    /// `None` when it would draw nothing (no text, no opacity).
    pub(crate) fn new(watermark: &Watermark, size: (u32, u32), space: RgbSpace) -> Option<Self> {
        let text = font::printable(&watermark.text);
        let text = text.trim();
        let alpha = finite_or(watermark.opacity, 0.0).clamp(0.0, 100.0) / 100.0;
        if text.is_empty() || alpha == 0.0 {
            return None;
        }
        let mut glyphs = Vec::with_capacity(text.len());
        let mut pen = 0.0;
        for c in text.chars() {
            let glyph = font::glyph(c);
            glyphs.push((pen, glyph));
            pen += glyph.advance;
        }
        let text_width = (pen - GAP).max(0.0);

        let (width, height) = (size.0 as f32, size.1 as f32);
        let short = width.min(height);
        let inset = INSET * short;
        let cap = finite_or(watermark.size, 3.0).clamp(0.1, 100.0) / 100.0 * short;
        let room = (width - 2.0 * inset).max(1.0);
        let unit = (cap / CAP_HEIGHT).min(room / text_width.max(1.0));
        let (fx, fy) = watermark.anchor.fractions();
        let origin = Point::new(
            inset + fx * (width - 2.0 * inset - text_width * unit),
            inset + fy * (height - 2.0 * inset - CAP_HEIGHT * unit),
        );
        let margin = HALF_STROKE * unit + 1.0;
        let span = |from: f32, to: f32, limit: u32| {
            let low = from.floor().clamp(0.0, limit as f32) as u32;
            let high = to.ceil().clamp(0.0, limit as f32) as u32;
            (low, high)
        };
        let decode = |code: u8| RgbSpace::Srgb.decode(f32::from(code) / 255.0);
        let color = mat3_apply(
            &rgb_to_rgb(RgbSpace::Srgb, space),
            watermark.color.map(decode),
        );
        Some(Self {
            glyphs,
            origin,
            unit,
            columns: span(
                origin.x - margin,
                origin.x + text_width * unit + margin,
                size.0,
            ),
            rows: span(
                origin.y + REACH_UP * unit - margin,
                origin.y + REACH_DOWN * unit + margin,
                size.1,
            ),
            alpha,
            color: color.map(|c| c.max(0.0)),
            space,
        })
    }

    /// Blends the watermark into `rows`, picture rows `first..` of a
    /// picture `width` pixels wide (16-bit RGB in the layer's colour space).
    pub(crate) fn draw(&self, rows: &mut [u16], width: u32, first: u32) {
        let stride = width as usize * 3;
        let count = (rows.len() / stride.max(1)) as u32;
        let (top, bottom) = (self.rows.0.max(first), self.rows.1.min(first + count));
        if top >= bottom {
            return;
        }
        let start = (top - first) as usize * stride;
        let end = (bottom - first) as usize * stride;
        rows[start..end]
            .par_chunks_exact_mut(stride)
            .enumerate()
            .for_each(|(index, row)| self.draw_row(row, top + index as u32));
    }

    fn draw_row(&self, row: &mut [u16], y: u32) {
        let (left, right) = self.columns;
        let mut coverage = vec![0.0f32; (right - left) as usize];
        let v = (y as f32 + 0.5 - self.origin.y) / self.unit;
        for &(pen, glyph) in &self.glyphs {
            let reach = HALF_STROKE + 1.0 / self.unit;
            let from = self.origin.x + (pen - reach) * self.unit;
            let to = self.origin.x + (pen + glyph.width + reach) * self.unit;
            let first = (from.floor().max(left as f32) as u32).min(right);
            let last = (to.ceil().max(0.0) as u32).min(right);
            for x in first..last {
                let u = (x as f32 + 0.5 - self.origin.x) / self.unit - pen;
                let distance = glyph
                    .segments
                    .iter()
                    .map(|&(a, b)| segment_distance(Point::new(u, v), a, b))
                    .fold(f32::INFINITY, f32::min);
                // A one-pixel ramp across the stroke's edge.
                let cover = ((HALF_STROKE - distance) * self.unit + 0.5).clamp(0.0, 1.0);
                let slot = &mut coverage[(x - left) as usize];
                *slot = slot.max(cover);
            }
        }
        for (offset, &cover) in coverage.iter().enumerate() {
            if cover <= 0.0 {
                continue;
            }
            let weight = cover * self.alpha;
            let pixel = &mut row[(left as usize + offset) * 3..][..3];
            for (sample, target) in pixel.iter_mut().zip(self.color) {
                let linear = self.space.decode(f32::from(*sample) / 65535.0);
                let mixed = linear + (target - linear) * weight;
                *sample = (self.space.encode(mixed) * 65535.0)
                    .round()
                    .clamp(0.0, 65535.0) as u16;
            }
        }
    }
}

fn finite_or(value: f32, fallback: f32) -> f32 {
    if value.is_finite() { value } else { fallback }
}

/// Distance from `p` to the segment `a`-`b` (a point when they coincide).
fn segment_distance(p: Point, a: Point, b: Point) -> f32 {
    let (dx, dy) = (b.x - a.x, b.y - a.y);
    let length = dx * dx + dy * dy;
    let t = if length > 0.0 {
        (((p.x - a.x) * dx + (p.y - a.y) * dy) / length).clamp(0.0, 1.0)
    } else {
        0.0
    };
    p.distance(Point::new(a.x + t * dx, a.y + t * dy))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::Anchor;

    fn layer(text: &str, anchor: Anchor, size: (u32, u32)) -> WatermarkLayer {
        let watermark = Watermark {
            text: text.into(),
            anchor,
            size: 5.0,
            ..Watermark::default()
        };
        WatermarkLayer::new(&watermark, size, RgbSpace::Srgb).unwrap()
    }

    #[test]
    fn nothing_to_draw_is_no_layer() {
        let blank = Watermark {
            text: "  ".into(),
            ..Watermark::default()
        };
        assert!(WatermarkLayer::new(&blank, (100, 100), RgbSpace::Srgb).is_none());
        let invisible = Watermark {
            text: "x".into(),
            opacity: 0.0,
            ..Watermark::default()
        };
        assert!(WatermarkLayer::new(&invisible, (100, 100), RgbSpace::Srgb).is_none());
    }

    #[test]
    fn the_text_sits_inside_the_picture_at_its_anchor() {
        let size = (1200, 800);
        for anchor in Anchor::ALL {
            let layer = layer("Anchored", anchor, size);
            let (fx, fy) = anchor.fractions();
            let centre = (
                (layer.columns.0 + layer.columns.1) as f32 / 2.0 / size.0 as f32,
                (layer.rows.0 + layer.rows.1) as f32 / 2.0 / size.1 as f32,
            );
            assert!(
                (centre.0 - (0.1 + 0.8 * fx)).abs() < 0.12,
                "{anchor:?} {centre:?}"
            );
            assert!(
                (centre.1 - (0.05 + 0.9 * fy)).abs() < 0.08,
                "{anchor:?} {centre:?}"
            );
        }
    }

    #[test]
    fn text_too_wide_for_the_picture_is_made_to_fit() {
        let layer = layer(&"W".repeat(200), Anchor::Center, (300, 200));
        assert!(
            layer.columns.0 > 0 && layer.columns.1 < 300,
            "{:?}",
            layer.columns
        );
    }

    #[test]
    fn segment_distance_measures_to_the_nearest_point() {
        let (a, b) = (Point::new(0.0, 0.0), Point::new(4.0, 0.0));
        assert_eq!(segment_distance(Point::new(2.0, 3.0), a, b), 3.0);
        assert_eq!(segment_distance(Point::new(7.0, 4.0), a, b), 5.0);
        assert_eq!(segment_distance(Point::new(1.0, 1.0), a, a), 2f32.sqrt());
    }
}
