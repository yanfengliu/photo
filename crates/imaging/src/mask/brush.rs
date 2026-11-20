//! Brush strokes, stamped into a bitmap in source space and then sampled.
//!
//! A stroke is the set of points within the brush radius of its path, with a
//! falloff toward the rim; its coverage at a point depends on the distance
//! to the nearest point of the path. Evaluating that for every view pixel
//! against every segment would cost the product of the two, so the strokes
//! are rasterized into square cells in source space, and each view pixel
//! reads the cells at its source position.
//!
//! The cells are `2^k` source pixels wide, `k` chosen from the view's scale
//! alone, and lie on one lattice anchored at the source's top-left corner.
//! A bitmap holds the cells of that lattice that its view reads and the
//! strokes reach, nothing more, so its size follows the view, not the
//! strokes. Every cell's coverage is computed from its own lattice position
//! with the same arithmetic whatever else the bitmap holds: the tiles of a
//! render agree bit for bit (contract R1), and a stroke renders the same
//! however many other strokes there are elsewhere.

use photo_core::develop::BrushStroke;

use crate::buffer::Plane;
use crate::slider::fraction;

use super::path::{SIMPLIFY_TOLERANCE, simplify};

/// Cells of margin around what the strokes reach (half a cell for the
/// antialiased rim, one for bilinear sampling, half to spare) and around
/// what the view reads (one for bilinear sampling, one to spare).
const MARGIN_CELLS: f64 = 2.0;
/// Largest cell size, as a power of two of source pixels.
const MAX_CELL_LOG2: f64 = 16.0;
/// Brush radii are held at this many long edges of the source: a brush
/// that large covers the whole picture from anywhere on it.
const MAX_RADIUS: f64 = 2.0;
/// Path points are held within this many widths and heights of the source.
/// A point farther out only bends its segment by a hair, and the limit
/// keeps every coordinate a reasonable `f32`.
const MAX_REACH: f64 = 1.0e4;

/// One straight piece of a stroke's path, in cells from the source's
/// top-left corner. A dot is a segment of no length.
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
    /// Raise `coverage[i]` to this stroke's coverage of lattice cell
    /// `(first_column + i, row)`, and return the range of `i` touched.
    ///
    /// A cell's value depends only on its lattice position: the centre of
    /// cell `c` is `c + 0.5`, formed exactly (whole numbers and halves are
    /// exact in `f32` this far from zero), so a cell gets the same bits
    /// whichever window of the lattice it is computed in.
    fn cover_row(
        &self,
        row: i64,
        first_column: i64,
        coverage: &mut [f32],
    ) -> Option<(usize, usize)> {
        let y = row as f32 + 0.5;
        let lowest = first_column as f32;
        let highest = (first_column + coverage.len() as i64 - 1) as f32;
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
            let low = (x_from.min(x_to) - self.outer - 0.5).ceil().max(lowest);
            let high = (x_from.max(x_to) + self.outer - 0.5).floor().min(highest);
            if from > to || high < low {
                continue;
            }
            let (first, last) = ((low - lowest) as usize, (high - lowest) as usize);
            // Straight-line arithmetic only, so the compiler can do several
            // cells at once: distance to the segment, then the falloff.
            let (centre, dy) = (low + 0.5, y - start.1);
            for (offset, value) in coverage[first..=last].iter_mut().enumerate() {
                let dx = (centre + offset as f32) - start.0;
                let along = ((dx * delta.0 + dy * delta.1) * segment.inverse_length_squared)
                    .clamp(0.0, 1.0);
                let (off_x, off_y) = (dx - along * delta.0, dy - along * delta.1);
                let distance = (off_x * off_x + off_y * off_y).sqrt();
                let t = ((distance - self.inner) * inverse_band).clamp(0.0, 1.0);
                *value = value.max(1.0 - t * t * (3.0 - 2.0 * t));
            }
            touched = Some(touched.map_or((first, last), |(l, h)| (l.min(first), h.max(last))));
        }
        touched
    }
}

/// A stroke in source pixels, before a cell size has been chosen.
struct Outline {
    points: Vec<(f64, f64)>,
    radius: f64,
    hardness: f64,
    strength: f32,
    erase: bool,
}

impl Outline {
    /// `None` for a stroke that paints nothing: no finite point, no radius,
    /// no flow.
    fn new(stroke: &BrushStroke, width: f64, height: f64) -> Option<Self> {
        let radius = f64::from(stroke.radius);
        let strength = fraction(stroke.flow) as f32;
        let reach = |value: f32| f64::from(value).clamp(-MAX_REACH, MAX_REACH);
        let points: Vec<(f64, f64)> = stroke
            .points
            .iter()
            .filter(|p| p.x.is_finite() && p.y.is_finite())
            .map(|p| (reach(p.x) * width, reach(p.y) * height))
            .collect();
        // NaN fails the comparison, so it paints nothing too.
        let usable = radius > 0.0 && strength > 0.0 && !points.is_empty();
        usable.then(|| Self {
            points,
            radius: radius.min(MAX_RADIUS) * width.max(height),
            hardness: 1.0 - fraction(stroke.feather),
            strength,
            erase: stroke.erase,
        })
    }

    fn stamp(&self, cell: f64) -> Stamp {
        let radius = self.radius / cell;
        let (mut inner, mut outer) = (radius * self.hardness, radius);
        if outer - inner < 1.0 {
            // A rim at least one cell wide, centred on the nominal edge,
            // keeps hard brushes antialiased on this grid.
            let middle = (inner + outer) / 2.0;
            (inner, outer) = (middle - 0.5, middle + 0.5);
        }
        let path: Vec<(f64, f64)> = self
            .points
            .iter()
            .map(|&(x, y)| (x / cell, y / cell))
            .collect();
        let path = simplify(&path, SIMPLIFY_TOLERANCE);
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

/// The lattice cells, as `first..end` global indices along one axis, that
/// lie within a margin of both `reach` (what the strokes cover) and `view`
/// (what the view reads), both in source pixels. `None` when they do not
/// meet.
fn cell_range(reach: (f64, f64), view: (f64, f64), cell: f64) -> Option<(i64, i64)> {
    let first = (reach.0 / cell).floor().max((view.0 / cell).floor()) - MARGIN_CELLS;
    let end = (reach.1 / cell).ceil().min((view.1 / cell).ceil()) + MARGIN_CELLS;
    (first < end).then_some((first as i64, end as i64))
}

/// The accumulated coverage of a list of strokes, over the window of the
/// cell lattice that one view reads.
pub(crate) struct BrushBitmap {
    cell: f64,
    /// Lattice column and row of the window's first cell.
    first: (i64, i64),
    coverage: Plane,
}

impl BrushBitmap {
    /// Stamp `strokes`, in order, for a source of `source_size` pixels seen
    /// with `footprint` source pixels per view pixel, covering what a view
    /// reads: `view` is `[x0, y0, x1, y1]`, in source pixels, around every
    /// position the view will [`sample`](Self::sample). `None` when the
    /// strokes paint nothing there.
    ///
    /// Within one stroke coverage is the maximum over its path, scaled by
    /// the stroke's flow, so a stroke never builds on itself. Across strokes
    /// paint accumulates (`a + c * (1 - a)`) and erasers remove
    /// (`a * (1 - c)`).
    ///
    /// Cells are `2^k` source pixels wide: no finer than a source pixel and
    /// no finer than the view can show, so there are fewer than four per
    /// view pixel.
    pub(crate) fn stamp(
        strokes: &[BrushStroke],
        source_size: (u32, u32),
        footprint: f64,
        view: [f64; 4],
    ) -> Option<Self> {
        let (width, height) = (f64::from(source_size.0), f64::from(source_size.1));
        let outlines: Vec<Outline> = strokes
            .iter()
            .filter_map(|s| Outline::new(s, width, height))
            .collect();
        let footprint = if footprint.is_finite() && footprint > 0.0 {
            footprint
        } else {
            1.0
        };
        let cell = 2f64.powi((footprint.log2() + 1e-3).floor().clamp(0.0, MAX_CELL_LOG2) as i32);

        // What the strokes can cover, in source pixels.
        let mut reach = [
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        ];
        for outline in &outlines {
            for &(x, y) in &outline.points {
                reach[0] = reach[0].min(x - outline.radius);
                reach[1] = reach[1].min(y - outline.radius);
                reach[2] = reach[2].max(x + outline.radius);
                reach[3] = reach[3].max(y + outline.radius);
            }
        }
        let (first_column, end_column) =
            cell_range((reach[0], reach[2]), (view[0], view[2]), cell)?;
        let (first_row, end_row) = cell_range((reach[1], reach[3]), (view[1], view[3]), cell)?;
        let columns = (end_column - first_column) as usize;
        let rows = (end_row - first_row) as usize;

        let stamps: Vec<Stamp> = outlines.iter().map(|outline| outline.stamp(cell)).collect();
        let mut coverage = Plane::new(columns, rows);
        coverage.par_rows_mut_with(
            || vec![0.0f32; columns],
            |scratch, row_index, row| {
                let lattice_row = first_row + row_index as i64;
                for stamp in &stamps {
                    let Some((low, high)) = stamp.cover_row(lattice_row, first_column, scratch)
                    else {
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
            cell,
            first: (first_column, first_row),
            coverage,
        })
    }

    /// Coverage at a source position, bilinear between cell centres and 0
    /// outside the window.
    #[inline]
    pub(crate) fn sample(&self, x: f64, y: f64) -> f32 {
        let (u, v) = (x / self.cell - 0.5, y / self.cell - 0.5);
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
        let (column, row) = (column as i64 - self.first.0, row as i64 - self.first.1);
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

    /// Stamp for a view that reads the whole source.
    fn whole(strokes: &[BrushStroke], size: (u32, u32), footprint: f64) -> Option<BrushBitmap> {
        let view = [0.0, 0.0, f64::from(size.0), f64::from(size.1)];
        BrushBitmap::stamp(strokes, size, footprint, view)
    }

    #[test]
    fn the_cell_size_follows_the_view_and_is_a_power_of_two() {
        let strokes = [line(0.05, 0.0)];
        let cell = |footprint: f64| whole(&strokes, (1000, 1000), footprint).unwrap().cell;
        assert_eq!(cell(0.25), 1.0, "never finer than a source pixel");
        assert_eq!(cell(1.0), 1.0);
        assert_eq!(cell(1.99), 1.0);
        assert_eq!(cell(2.0), 2.0);
        assert_eq!(cell(7.3), 4.0);
        assert_eq!(cell(f64::NAN), 1.0);
    }

    #[test]
    fn the_bitmap_holds_only_what_its_view_reads() {
        // A big stroke on a big source, read through a small window at 1:1:
        // full resolution, and only the window's cells (plus margins).
        let strokes = [line(0.4, 0.0)];
        let view = [9_900.0, 9_950.0, 10_100.0, 10_050.0];
        let bitmap = BrushBitmap::stamp(&strokes, (20_000, 20_000), 1.0, view).unwrap();
        assert_eq!(bitmap.cell, 1.0);
        assert_eq!(
            (bitmap.coverage.width(), bitmap.coverage.height()),
            (204, 104)
        );
        assert_eq!(bitmap.sample(10_000.0, 10_000.0), 1.0);
        // A window the strokes do not reach needs no bitmap at all.
        assert!(
            BrushBitmap::stamp(&strokes, (20_000, 20_000), 1.0, [10.0, 10.0, 50.0, 50.0]).is_none()
        );
    }

    #[test]
    fn a_window_holds_the_same_cells_as_the_whole() {
        let mut stroke = line(0.03, 40.0);
        stroke.points = (0..40)
            .map(|i| {
                let t = i as f32 / 39.0;
                Point::new(0.1 + 0.8 * t, 0.5 + 0.3 * (t * 9.0).sin())
            })
            .collect();
        let strokes = [stroke];
        for footprint in [1.0, 2.5, 6.0] {
            let all = whole(&strokes, (1000, 800), footprint).unwrap();
            let part = BrushBitmap::stamp(
                &strokes,
                (1000, 800),
                footprint,
                [310.0, 205.0, 520.0, 460.0],
            )
            .unwrap();
            for step in 0..500 {
                let (x, y) = (310.0 + step as f64 * 0.42, 205.0 + step as f64 * 0.51);
                assert_eq!(
                    all.sample(x, y).to_bits(),
                    part.sample(x, y).to_bits(),
                    "{footprint} at ({x}, {y})"
                );
            }
        }
    }

    #[test]
    fn strokes_outside_the_source_paint_nothing() {
        let mut outside = line(0.01, 0.0);
        outside.points = vec![Point::new(2.0, 2.0), Point::new(3.0, 2.5)];
        assert!(whole(&[outside], (500, 400), 1.0).is_none());
        let mut flowless = line(0.05, 0.0);
        flowless.flow = 0.0;
        assert!(whole(&[flowless], (500, 400), 1.0).is_none());
        let mut broken = line(0.05, 0.0);
        broken.points = vec![Point::new(f32::NAN, 0.5)];
        assert!(whole(&[broken], (500, 400), 1.0).is_none());
        let mut shapeless = line(f32::NAN, 0.0);
        assert!(whole(&[shapeless.clone()], (500, 400), 1.0).is_none());
        shapeless.radius = f32::INFINITY;
        let everywhere = whole(&[shapeless], (500, 400), 1.0).unwrap();
        assert_eq!(everywhere.sample(1.0, 1.0), 1.0);
    }

    #[test]
    fn a_brush_thinner_than_a_cell_is_faint_rather_than_missing() {
        // 2 px radius seen through 16 px cells.
        let bitmap = whole(&[line(0.002, 0.0)], (1000, 1000), 16.0).unwrap();
        let on_path = bitmap.sample(500.0, 500.0);
        assert!(on_path > 0.05 && on_path < 0.9, "{on_path}");
    }

    #[test]
    fn densely_sampled_straight_paths_are_simplified_without_moving_their_ends() {
        let mut dense = line(0.05, 0.0);
        dense.points = (0..=2000)
            .map(|i| Point::new(0.25 + 0.5 * i as f32 / 2000.0, 0.5))
            .collect();
        let stamp = Outline::new(&dense, 1000.0, 1000.0).unwrap().stamp(1.0);
        // 2000 collinear segments, at most 64 per drawn segment.
        assert!(stamp.segments.len() <= 32, "{}", stamp.segments.len());
        assert_eq!(stamp.segments[0].start, (250.0, 500.0));
        assert_eq!(stamp.segments.last().unwrap().end(), (750.0, 500.0));
    }

    #[test]
    fn a_dot_is_one_segment_of_no_length() {
        let mut dot = line(0.05, 0.0);
        dot.points.truncate(1);
        let stamp = Outline::new(&dot, 1000.0, 1000.0).unwrap().stamp(1.0);
        assert_eq!(stamp.segments.len(), 1);
        assert_eq!(stamp.segments[0].inverse_length_squared, 0.0);
        let mut row = [0.0f32; 1000];
        // Centred at x = 250 with a 50-cell radius: the rim is the cell at 300.
        assert_eq!(stamp.cover_row(500, 0, &mut row), Some((199, 300)));
        assert_eq!((row[250], row[298], row[300]), (1.0, 1.0, 0.0));
        // The same cells through a window that starts elsewhere.
        let mut window = [0.0f32; 200];
        assert_eq!(stamp.cover_row(500, 150, &mut window), Some((49, 150)));
        assert_eq!(window[100].to_bits(), row[250].to_bits());
        assert_eq!(window[149].to_bits(), row[299].to_bits());
    }
}
