//! Mask rasterization: a [`Mask`]'s coverage as an alpha plane for a view.

mod brush;

use photo_core::develop::{Mask, MaskComponent, MaskShape};

use crate::buffer::Plane;
use crate::geometry::Geometry;
use crate::slider::fraction;
use crate::view::View;

use brush::BrushBitmap;

/// Hermite smoothstep: 0 at or below `low`, 1 at or above `high`.
fn smoothstep(low: f64, high: f64, x: f64) -> f64 {
    let t = ((x - low) / (high - low)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// One component's shape, converted to source pixels once per call.
enum Shape {
    /// `t = (p - start) . along`; coverage is `1 - smoothstep(t)`.
    Linear {
        start: (f64, f64),
        along: (f64, f64),
    },
    Radial {
        centre: (f64, f64),
        /// Rotation into the ellipse's own axes.
        cos: f64,
        sin: f64,
        inverse_radii: (f64, f64),
        /// Normalized radii between which coverage falls from 1 to 0.
        inner: f64,
        outer: f64,
        invert: bool,
    },
    Brush(BrushBitmap),
    /// A shape that covers nothing (or everything, for an inverted radial
    /// gradient with no extent).
    Constant(f32),
}

impl Shape {
    /// `footprint` is the size of a view pixel in source pixels, which sets
    /// the width of the antialiased rim on hard edges.
    fn new(shape: &MaskShape, source_size: (u32, u32), footprint: f64) -> Self {
        let (width, height) = (f64::from(source_size.0), f64::from(source_size.1));
        let long_edge = width.max(height);
        match shape {
            MaskShape::Linear { start, end } => {
                let start = (f64::from(start.x) * width, f64::from(start.y) * height);
                let end = (f64::from(end.x) * width, f64::from(end.y) * height);
                let (dx, dy) = (end.0 - start.0, end.1 - start.1);
                let length_squared = dx * dx + dy * dy;
                if !length_squared.is_finite() || length_squared < 1e-12 {
                    return Self::Constant(0.0);
                }
                Self::Linear {
                    start,
                    along: (dx / length_squared, dy / length_squared),
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
                let (mut inner, mut outer) = (1.0 - fraction(*feather), 1.0);
                // Keep the transition at least one view pixel wide.
                let rim = 0.5 * footprint / radii.0.min(radii.1);
                if outer - inner < 2.0 * rim {
                    let middle = (inner + outer) / 2.0;
                    (inner, outer) = (middle - rim, middle + rim);
                }
                Self::Radial {
                    centre: (f64::from(center.x) * width, f64::from(center.y) * height),
                    cos,
                    sin,
                    inverse_radii: (1.0 / radii.0, 1.0 / radii.1),
                    inner,
                    outer,
                    invert: *invert,
                }
            }
            MaskShape::Brush { strokes } => BrushBitmap::stamp(strokes, source_size, footprint)
                .map_or(Self::Constant(0.0), Self::Brush),
        }
    }

    /// Coverage, `0..=1`, at a source position in pixels.
    #[inline]
    fn coverage(&self, x: f64, y: f64) -> f32 {
        match self {
            Self::Linear { start, along } => {
                let t = (x - start.0) * along.0 + (y - start.1) * along.1;
                (1.0 - smoothstep(0.0, 1.0, t)) as f32
            }
            Self::Radial {
                centre,
                cos,
                sin,
                inverse_radii,
                inner,
                outer,
                invert,
            } => {
                let (dx, dy) = (x - centre.0, y - centre.1);
                let u = (dx * cos + dy * sin) * inverse_radii.0;
                let v = (dy * cos - dx * sin) * inverse_radii.1;
                let inside = 1.0 - smoothstep(*inner, *outer, (u * u + v * v).sqrt());
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

impl Component {
    fn new(component: &MaskComponent, source_size: (u32, u32), footprint: f64) -> Self {
        Self {
            shape: Shape::new(&component.shape, source_size, footprint),
            subtract: component.subtract,
        }
    }
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
///   stroke removes as `a * (1 - c)`. Strokes are stamped into a bitmap in
///   source space whose cells are a power-of-two number of source pixels:
///   as fine as the view (never finer than the source), and at most about
///   four million cells.
///
/// Hard edges are antialiased over one view pixel (one bitmap cell for
/// brushes).
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
/// geometry and the view's scale, so a tile equals the same region of the
/// whole view (R1), and the result is the same on any number of threads
/// (R4).
pub fn rasterize_mask(mask: &Mask, geometry: &Geometry, view: &View, padding: u32) -> Plane {
    let source_size = geometry.source_size();
    let grid = view.grid(padding);
    // Source pixels per view pixel: the same for every tile of a view.
    let footprint = geometry.footprint() * grid.pitch();
    let components: Vec<Component> = mask
        .components
        .iter()
        .map(|component| Component::new(component, source_size, footprint))
        .collect();

    let background = if mask.invert { 1.0 } else { 0.0 };
    let mut out = Plane::filled(grid.width, grid.height, background);
    if components.is_empty() {
        return out;
    }
    let (source_w, source_h) = (f64::from(source_size.0), f64::from(source_size.1));
    out.par_rows_mut(|row_index, row| {
        let y = grid.y(row_index);
        for (column, value) in row.iter_mut().enumerate() {
            let (source_x, source_y) = geometry.map_out_to_src(grid.x(column), y);
            let (source_x, source_y) =
                (source_x.clamp(0.0, source_w), source_y.clamp(0.0, source_h));
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
