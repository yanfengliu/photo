//! Mask rasterization: a [`Mask`]'s coverage as an alpha plane for a view.

mod brush;
mod path;

use photo_core::develop::{Mask, MaskComponent, MaskShape};
use rayon::prelude::*;

use crate::buffer::Plane;
use crate::geometry::Geometry;
use crate::slider::fraction;
use crate::view::{PixelGrid, View};

use brush::BrushBitmap;

/// Hermite smoothstep: 0 at or below `low`, 1 at or above `high`. A band
/// of no width (or a NaN one) is a step at `low`.
fn smoothstep(low: f64, high: f64, x: f64) -> f64 {
    if low.is_nan() || high.is_nan() || high <= low {
        return if x < low { 0.0 } else { 1.0 };
    }
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// `(low, high)`, or the band one `pixel` wide around its middle when it is
/// narrower than that: how hard edges keep one view pixel of antialiasing.
fn at_least(low: f64, high: f64, pixel: f64) -> (f64, f64) {
    if high - low < pixel {
        let middle = (low + high) / 2.0;
        (middle - pixel / 2.0, middle + pixel / 2.0)
    } else {
        (low, high)
    }
}

/// One component's shape, converted to source pixels once per call.
enum Shape {
    /// `t = (p - start) . along`; coverage is `1 - smoothstep(low, high, t)`.
    Linear {
        start: (f64, f64),
        along: (f64, f64),
        low: f64,
        high: f64,
    },
    Radial {
        centre: (f64, f64),
        /// Rotation into the ellipse's own axes.
        cos: f64,
        sin: f64,
        inverse_radii: (f64, f64),
        /// Normalized radii between which coverage falls from 1 to 0, as
        /// the feather sets them.
        inner: f64,
        outer: f64,
        /// Source pixels per view pixel.
        footprint: f64,
        invert: bool,
    },
    Brush(BrushBitmap),
    /// A shape that covers nothing (or everything, for an inverted radial
    /// gradient with no extent).
    Constant(f32),
}

impl Shape {
    /// `footprint` is the size of a view pixel in source pixels, which sets
    /// the width of the antialiased rim on hard edges. `extent` is
    /// `[x0, y0, x1, y1]` around every source position the shape will be
    /// evaluated at, which bounds what a brush needs to stamp.
    fn new(shape: &MaskShape, source_size: (u32, u32), footprint: f64, extent: [f64; 4]) -> Self {
        let (width, height) = (f64::from(source_size.0), f64::from(source_size.1));
        let long_edge = width.max(height);
        match shape {
            MaskShape::Linear { start, end } => {
                let start = (f64::from(start.x) * width, f64::from(start.y) * height);
                let end = (f64::from(end.x) * width, f64::from(end.y) * height);
                let (dx, dy) = (end.0 - start.0, end.1 - start.1);
                let length_squared = dx * dx + dy * dy;
                let usable = start.0.is_finite() && start.1.is_finite();
                if !usable || !length_squared.is_finite() || length_squared < 1e-12 {
                    return Self::Constant(0.0);
                }
                // A gradient shorter than a view pixel is an edge, and gets
                // a pixel of antialiasing like any other.
                let (low, high) = at_least(0.0, 1.0, footprint / length_squared.sqrt());
                Self::Linear {
                    start,
                    along: (dx / length_squared, dy / length_squared),
                    low,
                    high,
                }
            }
            MaskShape::Radial {
                center,
                radius_x,
                radius_y,
                angle,
                feather,
                invert,
            } => {
                let radii = (
                    f64::from(*radius_x) * long_edge,
                    f64::from(*radius_y) * long_edge,
                );
                let usable = [
                    radii.0,
                    radii.1,
                    f64::from(center.x),
                    f64::from(center.y),
                    f64::from(*angle),
                ]
                .iter()
                .all(|v| v.is_finite());
                if !usable || radii.0 <= 0.0 || radii.1 <= 0.0 {
                    return Self::Constant(if *invert { 1.0 } else { 0.0 });
                }
                let (sin, cos) = f64::from(*angle).to_radians().sin_cos();
                Self::Radial {
                    centre: (f64::from(center.x) * width, f64::from(center.y) * height),
                    cos,
                    sin,
                    inverse_radii: (1.0 / radii.0, 1.0 / radii.1),
                    inner: 1.0 - fraction(*feather),
                    outer: 1.0,
                    footprint,
                    invert: *invert,
                }
            }
            MaskShape::Brush { strokes } => {
                BrushBitmap::stamp(strokes, source_size, footprint, extent)
                    .map_or(Self::Constant(0.0), Self::Brush)
            }
        }
    }

    /// Coverage, `0..=1`, at a source position in pixels.
    #[inline]
    fn coverage(&self, x: f64, y: f64) -> f32 {
        match self {
            Self::Linear {
                start,
                along,
                low,
                high,
            } => {
                let t = (x - start.0) * along.0 + (y - start.1) * along.1;
                (1.0 - smoothstep(*low, *high, t)) as f32
            }
            Self::Radial {
                centre,
                cos,
                sin,
                inverse_radii,
                inner,
                outer,
                footprint,
                invert,
            } => {
                let (dx, dy) = (x - centre.0, y - centre.1);
                let u = (dx * cos + dy * sin) * inverse_radii.0;
                let v = (dy * cos - dx * sin) * inverse_radii.1;
                let distance = (u * u + v * v).sqrt();
                // One view pixel, in units of the normalized distance, here:
                // the footprint times the length of that distance's gradient.
                // Measured per pixel, so a long thin ellipse is as sharp
                // along its length as across it.
                let pixel = if distance > 0.0 {
                    let (gu, gv) = (u * inverse_radii.0, v * inverse_radii.1);
                    footprint * (gu * gu + gv * gv).sqrt() / distance
                } else {
                    // The centre itself: the limit along the ellipse's
                    // shorter axis, so a disc smaller than a pixel has no
                    // spike there.
                    footprint * inverse_radii.0.max(inverse_radii.1)
                };
                let (low, high) = at_least(*inner, *outer, pixel);
                let inside = 1.0 - smoothstep(low, high, distance);
                (if *invert { 1.0 - inside } else { inside }) as f32
            }
            Self::Brush(bitmap) => bitmap.sample(x, y),
            Self::Constant(value) => *value,
        }
    }
}

struct Component {
    shape: Shape,
    subtract: bool,
}

/// Every source position a padded view grid is evaluated at, mapped and
/// clamped exactly as [`rasterize_mask`] does, reduced to its bounding box
/// `[x0, y0, x1, y1]`. Rows are scanned in parallel; minima and maxima do
/// not depend on the order they are taken in.
fn source_extent(geometry: &Geometry, grid: &PixelGrid, source: (f64, f64)) -> [f64; 4] {
    let empty = [
        f64::INFINITY,
        f64::INFINITY,
        f64::NEG_INFINITY,
        f64::NEG_INFINITY,
    ];
    let merge = |a: [f64; 4], b: [f64; 4]| {
        [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]
    };
    (0..grid.height)
        .into_par_iter()
        .map(|row| {
            let y = grid.y(row);
            (0..grid.width).fold(empty, |extent, column| {
                let (x, y) = geometry.map_out_to_src(grid.x(column), y);
                let (x, y) = (x.clamp(0.0, source.0), y.clamp(0.0, source.1));
                merge(extent, [x, y, x, y])
            })
        })
        .reduce(|| empty, merge)
}

/// The coverage of `mask` over `view`, as an alpha plane in `0..=1`,
/// surrounded by `padding` extra view pixels on every side (the same layout
/// as [`resample`](crate::resample())).
///
/// Only the mask's shape is rasterized: `components` and `invert`. Its
/// `enabled`, `amount` and `adjust` fields are the caller's to apply.
///
/// # Shapes
///
/// Shapes are defined in normalized source coordinates and evaluated at the
/// source position of each view pixel ([`Geometry::out_to_src`]), so a mask
/// stays on the image content it was drawn on through any orientation,
/// crop, perspective or lens setting, at any view scale. Positions outside
/// the source take the coverage of the nearest point on its edge, matching
/// the edge replication of [`resample`](crate::resample()). Lengths given as
/// fractions of the long edge are in source pixels, so circles stay round.
///
/// - **Linear**: 1 on the `start` side, 0 beyond `end`, eased between them
///   with a smoothstep (0.5 at the midpoint), constant across the direction
///   from `start` to `end` in source pixels. A gradient of zero length
///   covers nothing.
/// - **Radial**: an ellipse with half-axes `radius_x` and `radius_y`,
///   turned clockwise on screen by `angle` degrees in the source frame
///   (the convention of `CropRect::corners`). The ellipse is the outer
///   limit of the effect at any feather: coverage is 1 out to
///   `1 - feather / 100` of the way from the centre to the ellipse and
///   falls with a smoothstep to 0 on it. Feather 0 is a hard edge, 100 is
///   soft from the centre. `invert` gives the complement. An ellipse with
///   no extent covers nothing.
/// - **Brush**: each stroke covers everything within `radius` of its path
///   (round ends and joins), fully out to `1 - feather / 100` of the radius
///   and falling with a smoothstep to 0 at the radius, times `flow / 100`.
///   A stroke never builds on itself, however its path overlaps. Strokes
///   apply in order: paint accumulates as `a + c * (1 - a)`, an erasing
///   stroke removes as `a * (1 - c)`. Radii are held at twice the long
///   edge.
///
///   Strokes are stamped into square cells in source space, a power-of-two
///   number of source pixels wide: as fine as the view can show (never
///   finer than a source pixel), on one lattice for every view at that
///   scale. Only the cells inside the bounding box of what this view reads
///   are stamped, so the work and memory follow the view rather than the
///   strokes, and a stroke renders the same however many others there are.
///   A view that reads the source upright (quarter turns included) needs
///   fewer than four cells per view pixel; a straightened or keystoned view
///   reads a slanted region whose bounding box is larger, about twice for a
///   whole frame at 45 degrees and many times for a thin strip of one.
///
///   A stroke's path is drawn simplified, never more than a quarter of a
///   cell from where it was painted or, under a soft edge, 2% of the edge's
///   width: either way coverage moves by at most 3% of the stroke's
///   strength.
///
/// Edges are antialiased over one view pixel (one cell for brushes): a
/// feather or gradient narrower than that is widened to it, centred on the
/// edge, so a hard edge reaches half a view pixel beyond its nominal place.
///
/// # Combination
///
/// Starting from 0, components apply in list order: an adding component
/// with coverage `s` gives `a + s * (1 - a)` (the union of independent
/// coverages, never above 1), a subtracting one gives `a * (1 - s)`. A
/// subtraction therefore only removes what earlier components added.
/// Finally `invert` replaces `a` with `1 - a`. A mask without components is
/// 0 everywhere (1 when inverted).
///
/// # Guarantees
///
/// Every pixel depends only on its own output position, the mask, the
/// geometry and the view's scale, so a tile made with [`View::tile`] gets
/// the same bits as that region of the whole view (R1), and the result is
/// the same on any number of threads (R4). Non-finite shape parameters
/// never produce a NaN.
pub fn rasterize_mask(mask: &Mask, geometry: &Geometry, view: &View, padding: u32) -> Plane {
    let source_size = geometry.source_size();
    let source = (f64::from(source_size.0), f64::from(source_size.1));
    let grid = view.grid(padding);
    // Source pixels per view pixel: the same for every tile of a view.
    let footprint = geometry.footprint() * grid.pitch();
    let has_brush = mask
        .components
        .iter()
        .any(|component| matches!(component.shape, MaskShape::Brush { .. }));
    // Only a brush needs to know where the view reads; finding out costs a
    // pass over the view.
    let extent = if has_brush {
        source_extent(geometry, &grid, source)
    } else {
        [0.0, 0.0, source.0, source.1]
    };
    let components: Vec<Component> = mask
        .components
        .iter()
        .map(|component: &MaskComponent| Component {
            shape: Shape::new(&component.shape, source_size, footprint, extent),
            subtract: component.subtract,
        })
        .collect();

    let background = if mask.invert { 1.0 } else { 0.0 };
    let mut out = Plane::filled(grid.width, grid.height, background);
    if components.is_empty() {
        return out;
    }
    out.par_rows_mut(|row_index, row| {
        let y = grid.y(row_index);
        for (column, value) in row.iter_mut().enumerate() {
            let (source_x, source_y) = geometry.map_out_to_src(grid.x(column), y);
            let (source_x, source_y) =
                (source_x.clamp(0.0, source.0), source_y.clamp(0.0, source.1));
            let mut alpha = 0.0f32;
            for component in &components {
                let coverage = component.shape.coverage(source_x, source_y);
                alpha = if component.subtract {
                    alpha * (1.0 - coverage)
                } else {
                    alpha + coverage * (1.0 - alpha)
                };
            }
            *value = if mask.invert { 1.0 - alpha } else { alpha };
        }
    });
    out
}
