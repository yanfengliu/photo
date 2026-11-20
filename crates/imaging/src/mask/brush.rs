//! Brush strokes, stamped into a bitmap in source space and then sampled.
//!
//! A stroke is the set of points within the brush radius of its path, with a
//! falloff toward the rim; its coverage at a point depends on the distance
//! to the nearest point of the path. Evaluating that for every view pixel
//! against every segment would cost the product of the two, so the strokes
//! are rasterized once into a grid of cells aligned with the source, and
//! each view pixel reads the grid at its source position.
//!
//! The grid's resolution and extent depend only on the strokes, the source
//! size and the view's scale, never on which part of the output a view
//! covers, so every tile of a render reads the same bitmap (contract R1).

use photo_core::develop::BrushStroke;

use crate::buffer::Plane;
use crate::slider::fraction;

/// Most cells a bitmap may have, whatever the strokes cover. Reached only
/// when strokes span a large part of a large source seen at close to 1:1.
const MAX_CELLS: f64 = 4.0 * 1024.0 * 1024.0;
/// Path points closer together than this fraction of the brush radius (or
/// than [`MIN_SEGMENT_CELLS`]) are merged: they cannot change the outline
/// by more than a sliver, and every segment costs time.
const MIN_SEGMENT_RADII: f64 = 0.25;
const MIN_SEGMENT_CELLS: f64 = 0.25;
/// Cells of margin around the strokes: half for the antialiased rim, half
/// for bilinear sampling, one to spare.
const MARGIN_CELLS: f64 = 2.0;
/// Largest cell size, as a power of two of source pixels.
const MAX_CELL_LOG2: f64 = 16.0;

/// One straight piece of a stroke's path, in cell units. A dot is a segment
/// of no length.
struct Segment {
    start: (f32, f32),
    delta: (f32, f32),
    /// `1 / |delta|^2`, or 0 for a dot, so that the nearest point of a dot
    /// is always its start.
    inverse_length_squared: f32,
}

impl Segment {
    fn new(from: (f64, f64), to: (f64, f64)) -> Self {
        let delta = ((to.0 - from.0) as f32, (to.1 - from.1) as f32);
        let length_squared = delta.0 * delta.0 + delta.1 * delta.1;
        Self {
            start: (from.0 as f32, from.1 as f32),
            delta,
            inverse_length_squared: if length_squared > 0.0 {
                1.0 / length_squared
            } else {
                0.0
            },
        }
    }

    fn end(&self) -> (f32, f32) {
        (self.start.0 + self.delta.0, self.start.1 + self.delta.1)
    }
}

/// One stroke, ready to stamp: everything in cell units.
struct Stamp {
    segments: Vec<Segment>,
    /// Coverage is 1 within `inner` of the path and 0 beyond `outer`.
    inner: f32,
    outer: f32,
    strength: f32,
    erase: bool,
}

impl Stamp {
    /// Raise `coverage[i]` to this stroke's coverage of the cells of row
    /// `row`, and return the range of cells touched.
    fn cover_row(&self, row: usize, coverage: &mut [f32]) -> Option<(usize, usize)> {
        let y = row as f32 + 0.5;
        let last = coverage.len() as f32 - 1.0;
        let inverse_band = 1.0 / (self.outer - self.inner);
        let mut touched: Option<(usize, usize)> = None;
        for segment in &self.segments {
            let (start, delta, end) = (segment.start, segment.delta, segment.end());
            if y < start.1.min(end.1) - self.outer || y > start.1.max(end.1) + self.outer {
                continue;
            }
            // The stretch of the segment within reach of this row.
            let (from, to) = if delta.1.abs() < 1e-6 {
                (0.0, 1.0)
            } else {
                let (u, v) = (
                    (y - self.outer - start.1) / delta.1,
                    (y + self.outer - start.1) / delta.1,
                );
                (u.min(v).max(0.0), u.max(v).min(1.0))
            };
            let (x_from, x_to) = (start.0 + from * delta.0, start.0 + to * delta.0);
            let low = (x_from.min(x_to) - self.outer - 0.5).ceil().max(0.0);
            let high = (x_from.max(x_to) + self.outer - 0.5).floor().min(last);
            if from > to || high < low {
                continue;
            }
            let (low, high) = (low as usize, high as usize);
            // Straight-line arithmetic only, so the compiler can do several
            // cells at once: distance to the segment, then the falloff.
            let (first_dx, dy) = (low as f32 + 0.5 - start.0, y - start.1);
            for (offset, value) in coverage[low..=high].iter_mut().enumerate() {
                let dx = first_dx + offset as f32;
                let along = ((dx * delta.0 + dy * delta.1) * segment.inverse_length_squared)
                    .clamp(0.0, 1.0);
                let (off_x, off_y) = (dx - along * delta.0, dy - along * delta.1);
                let distance = (off_x * off_x + off_y * off_y).sqrt();
                let t = ((distance - self.inner) * inverse_band).clamp(0.0, 1.0);
                *value = value.max(1.0 - t * t * (3.0 - 2.0 * t));
            }
            touched = Some(touched.map_or((low, high), |(l, h)| (l.min(low), h.max(high))));
        }
        touched
    }
}

/// A stroke in source pixels, before a grid has been chosen.
struct Outline {
    points: Vec<(f64, f64)>,
    radius: f64,
    hardness: f64,
    strength: f32,
    erase: bool,
}

impl Outline {
    fn new(stroke: &BrushStroke, width: f64, height: f64) -> Option<Self> {
        let radius = f64::from(stroke.radius) * width.max(height);
        let strength = fraction(stroke.flow) as f32;
        let points: Vec<(f64, f64)> = stroke
            .points
            .iter()
            .filter(|p| p.x.is_finite() && p.y.is_finite())
            .map(|p| (f64::from(p.x) * width, f64::from(p.y) * height))
            .collect();
        let usable = radius.is_finite() && radius > 0.0 && strength > 0.0 && !points.is_empty();
        usable.then(|| Self {
            points,
            radius,
            hardness: 1.0 - fraction(stroke.feather),
            strength,
            erase: stroke.erase,
        })
    }

    fn stamp(&self, origin: (f64, f64), cell: f64) -> Stamp {
        let radius = self.radius / cell;
        let (mut inner, mut outer) = (radius * self.hardness, radius);
        if outer - inner < 1.0 {
            // A rim at least one cell wide, centred on the nominal edge,
            // keeps hard brushes antialiased on this grid.
            let middle = (inner + outer) / 2.0;
            (inner, outer) = (middle - 0.5, middle + 0.5);
        }
        let step = (MIN_SEGMENT_RADII * radius).max(MIN_SEGMENT_CELLS);
        let mut path: Vec<(f64, f64)> = Vec::with_capacity(self.points.len());
        for (index, &(x, y)) in self.points.iter().enumerate() {
            let point = ((x - origin.0) / cell, (y - origin.1) / cell);
            let is_last = index + 1 == self.points.len();
            let far_enough = path
                .last()
                .is_none_or(|&(px, py)| (point.0 - px).hypot(point.1 - py) >= step);
            if far_enough || is_last {
                path.push(point);
            }
        }
        let segments = if path.len() == 1 {
            vec![Segment::new(path[0], path[0])]
        } else {
            path.windows(2)
                .map(|pair| Segment::new(pair[0], pair[1]))
                .collect()
        };
        Stamp {
            segments,
            inner: inner as f32,
            outer: outer as f32,
            strength: self.strength,
            erase: self.erase,
        }
    }
}

/// The accumulated coverage of a list of strokes on a grid of square cells
/// in source space.
pub(crate) struct BrushBitmap {
    origin: (f64, f64),
    cell: f64,
    coverage: Plane,
}

impl BrushBitmap {
    /// Stamp `strokes`, in order, for a source of `source_size` pixels seen
    /// with `footprint` source pixels per view pixel. `None` when nothing
    /// would be painted inside the source.
    ///
    /// Within one stroke coverage is the maximum over its path, scaled by
    /// the stroke's flow, so a stroke never builds on itself. Across strokes
    /// paint accumulates (`a + c * (1 - a)`) and erasers remove
    /// (`a * (1 - c)`).
    ///
    /// Cells are `2^k` source pixels wide: no finer than a source pixel, no
    /// finer than the view can show, and coarse enough that the bitmap
    /// stays under [`MAX_CELLS`].
    pub(crate) fn stamp(
        strokes: &[BrushStroke],
        source_size: (u32, u32),
        footprint: f64,
    ) -> Option<Self> {
        let (width, height) = (f64::from(source_size.0), f64::from(source_size.1));
        let outlines: Vec<Outline> = strokes
            .iter()
            .filter_map(|s| Outline::new(s, width, height))
            .collect();

        // What the strokes can cover, clipped to the source.
        let mut bounds = (
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for outline in &outlines {
            for &(x, y) in &outline.points {
                bounds.0 = bounds.0.min(x - outline.radius);
                bounds.1 = bounds.1.min(y - outline.radius);
                bounds.2 = bounds.2.max(x + outline.radius);
                bounds.3 = bounds.3.max(y + outline.radius);
            }
        }
        let (x0, y0) = (bounds.0.max(0.0), bounds.1.max(0.0));
        let (x1, y1) = (bounds.2.min(width), bounds.3.min(height));
        if x1 <= x0 || y1 <= y0 {
            return None;
        }

        let footprint = if footprint.is_finite() && footprint > 0.0 {
            footprint
        } else {
            1.0
        };
        let for_view = (footprint.log2() + 1e-3).floor();
        let for_size = (0.5 * ((x1 - x0) * (y1 - y0) / MAX_CELLS).log2()).ceil();
        let cell = 2f64.powi(for_view.max(for_size).clamp(0.0, MAX_CELL_LOG2) as i32);

        let origin = (
            ((x0 / cell).floor() - MARGIN_CELLS) * cell,
            ((y0 / cell).floor() - MARGIN_CELLS) * cell,
        );
        let columns = ((x1 - origin.0) / cell).ceil() + MARGIN_CELLS;
        let rows = ((y1 - origin.1) / cell).ceil() + MARGIN_CELLS;

        let stamps: Vec<Stamp> = outlines
            .iter()
            .map(|outline| outline.stamp(origin, cell))
            .collect();
        let mut coverage = Plane::new(columns as usize, rows as usize);
        coverage.par_rows_mut_with(
            || vec![0.0f32; columns as usize],
            |scratch, row_index, row| {
                for stamp in &stamps {
                    let Some((low, high)) = stamp.cover_row(row_index, scratch) else {
                        continue;
                    };
                    for (value, covered) in row[low..=high].iter_mut().zip(&mut scratch[low..=high])
                    {
                        let amount = stamp.strength * *covered;
                        *value = if stamp.erase {
                            *value * (1.0 - amount)
                        } else {
                            *value + amount * (1.0 - *value)
                        };
                        *covered = 0.0;
                    }
                }
            },
        );
        Some(Self {
            origin,
            cell,
            coverage,
        })
    }

    /// Coverage at a source position, bilinear between cell centres and 0
    /// outside the bitmap.
    #[inline]
    pub(crate) fn sample(&self, x: f64, y: f64) -> f32 {
        let (u, v) = (
            (x - self.origin.0) / self.cell - 0.5,
            (y - self.origin.1) / self.cell - 0.5,
        );
        let (column, row) = (u.floor(), v.floor());
        let (tx, ty) = ((u - column) as f32, (v - row) as f32);
        let (width, height) = (self.coverage.width() as i64, self.coverage.height() as i64);
        let at = |column: i64, row: i64| {
            if column >= 0 && row >= 0 && column < width && row < height {
                self.coverage.get(column as usize, row as usize)
            } else {
                0.0
            }
        };
        let (column, row) = (column as i64, row as i64);
        let upper = at(column, row) + (at(column + 1, row) - at(column, row)) * tx;
        let lower = at(column, row + 1) + (at(column + 1, row + 1) - at(column, row + 1)) * tx;
        upper + (lower - upper) * ty
    }
}

#[cfg(test)]
mod tests {
    use photo_core::geom::Point;

    use super::*;

    fn line(radius: f32, feather: f32) -> BrushStroke {
        BrushStroke {
            points: vec![Point::new(0.25, 0.5), Point::new(0.75, 0.5)],
            radius,
            feather,
            flow: 100.0,
            erase: false,
        }
    }

    #[test]
    fn the_cell_size_follows_the_view_and_is_a_power_of_two() {
        let strokes = [line(0.05, 0.0)];
        let cell = |footprint: f64| {
            BrushBitmap::stamp(&strokes, (1000, 1000), footprint)
                .unwrap()
                .cell
        };
        assert_eq!(cell(0.25), 1.0, "never finer than a source pixel");
        assert_eq!(cell(1.0), 1.0);
        assert_eq!(cell(1.99), 1.0);
        assert_eq!(cell(2.0), 2.0);
        assert_eq!(cell(7.3), 4.0);
        assert_eq!(cell(f64::NAN), 1.0);
    }

    #[test]
    fn a_huge_painted_area_is_capped_in_cells() {
        let strokes = [line(0.4, 0.0)];
        let bitmap = BrushBitmap::stamp(&strokes, (20_000, 20_000), 1.0).unwrap();
        assert!(bitmap.cell >= 8.0, "{}", bitmap.cell);
        let cells = bitmap.coverage.width() * bitmap.coverage.height();
        assert!(cells as f64 <= MAX_CELLS * 1.01, "{cells}");
        // Still correct: solid on the path, empty in the far corner.
        assert_eq!(bitmap.sample(10_000.0, 10_000.0), 1.0);
        assert_eq!(bitmap.sample(100.0, 100.0), 0.0);
    }

    #[test]
    fn strokes_outside_the_source_paint_nothing() {
        let mut outside = line(0.01, 0.0);
        outside.points = vec![Point::new(2.0, 2.0), Point::new(3.0, 2.5)];
        assert!(BrushBitmap::stamp(&[outside], (500, 400), 1.0).is_none());
        let mut flowless = line(0.05, 0.0);
        flowless.flow = 0.0;
        assert!(BrushBitmap::stamp(&[flowless], (500, 400), 1.0).is_none());
        let mut broken = line(0.05, 0.0);
        broken.points = vec![Point::new(f32::NAN, 0.5)];
        assert!(BrushBitmap::stamp(&[broken], (500, 400), 1.0).is_none());
    }

    #[test]
    fn a_brush_thinner_than_a_cell_is_faint_rather_than_missing() {
        // 2 px radius seen through 16 px cells.
        let bitmap = BrushBitmap::stamp(&[line(0.002, 0.0)], (1000, 1000), 16.0).unwrap();
        let on_path = bitmap.sample(500.0, 500.0);
        assert!(on_path > 0.05 && on_path < 0.9, "{on_path}");
    }

    #[test]
    fn densely_sampled_paths_are_thinned_without_moving_their_ends() {
        let mut dense = line(0.05, 0.0);
        dense.points = (0..=2000)
            .map(|i| Point::new(0.25 + 0.5 * i as f32 / 2000.0, 0.5))
            .collect();
        let outline = Outline::new(&dense, 1000.0, 1000.0).unwrap();
        let stamp = outline.stamp((0.0, 0.0), 1.0);
        // 500 px of path with a 50 px brush: a point every 12.5 px.
        assert!(stamp.segments.len() <= 41, "{}", stamp.segments.len());
        assert_eq!(stamp.segments[0].start, (250.0, 500.0));
        assert_eq!(stamp.segments.last().unwrap().end(), (750.0, 500.0));
    }

    #[test]
    fn a_dot_is_one_segment_of_no_length() {
        let mut dot = line(0.05, 0.0);
        dot.points.truncate(1);
        let stamp = Outline::new(&dot, 1000.0, 1000.0)
            .unwrap()
            .stamp((0.0, 0.0), 1.0);
        assert_eq!(stamp.segments.len(), 1);
        assert_eq!(stamp.segments[0].inverse_length_squared, 0.0);
        let mut row = [0.0f32; 1000];
        // Centred at x = 250 with a 50-cell radius: the rim is the cell at 300.
        assert_eq!(stamp.cover_row(500, &mut row), Some((199, 300)));
        assert_eq!((row[250], row[298], row[300]), (1.0, 1.0, 0.0));
    }
}
