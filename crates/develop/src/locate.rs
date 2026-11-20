//! Where a view's pixels come from in the source, for the stages that read
//! something defined in source coordinates per pixel (lens vignetting, the
//! whole-image guides).

use photo_core::develop::DevelopSettings;
use photo_imaging::{Geometry, View};

/// Grid pixels between the anchors an affine locator is fitted at.
const SPAN: f64 = 1024.0;

/// Source positions (in source pixels) of the pixels of a view's padded
/// buffer.
///
/// When the geometry is affine (no lens distortion and no perspective) the
/// position is an affine function of the pixel's index in the view's pixel
/// grid, fitted at three anchors of that grid; otherwise every pixel is
/// mapped exactly. The anchors are the same grid pixels whatever window of
/// the grid a view is, so a tile computes the same numbers for a pixel as
/// the whole view does.
pub(crate) enum Locator {
    Exact {
        view: View,
        geometry: Box<Geometry>,
    },
    Affine {
        origin: (f64, f64),
        across: (f64, f64),
        down: (f64, f64),
        first: (i64, i64),
    },
}

impl Locator {
    pub(crate) fn new(view: &View, geometry: &Geometry, settings: &DevelopSettings) -> Self {
        let affine = settings.lens.distortion == 0.0
            && settings.transform.vertical == 0.0
            && settings.transform.horizontal == 0.0;
        if !affine {
            return Self::Exact {
                view: *view,
                geometry: Box::new(geometry.clone()),
            };
        }
        let first = view.grid_position();
        // The source position of grid pixel (gx, gy), whatever window of the
        // grid `view` is.
        let at = |gx: f64, gy: f64| {
            let x = (gx - first.0 as f64 + 0.5) as f32;
            let y = (gy - first.1 as f64 + 0.5) as f32;
            let p = geometry.out_to_src(view.to_output(x, y));
            (f64::from(p.x), f64::from(p.y))
        };
        let origin = at(0.0, 0.0);
        let (right, below) = (at(SPAN, 0.0), at(0.0, SPAN));
        Self::Affine {
            origin,
            across: ((right.0 - origin.0) / SPAN, (right.1 - origin.1) / SPAN),
            down: ((below.0 - origin.0) / SPAN, (below.1 - origin.1) / SPAN),
            first,
        }
    }

    /// Fill `out` with the source positions of row `row` of a buffer padded
    /// by `padding`, one per column.
    pub(crate) fn row(&self, row: usize, padding: u32, out: &mut [(f32, f32)]) {
        let padding = i64::from(padding);
        match self {
            Self::Exact { view, geometry } => {
                let y = row as f32 - padding as f32 + 0.5;
                for (column, position) in out.iter_mut().enumerate() {
                    let p = geometry
                        .out_to_src(view.to_output(column as f32 - padding as f32 + 0.5, y));
                    *position = (p.x, p.y);
                }
            }
            Self::Affine {
                origin,
                across,
                down,
                first,
            } => {
                let gy = (first.1 - padding + row as i64) as f64;
                let start = (origin.0 + down.0 * gy, origin.1 + down.1 * gy);
                for (column, position) in out.iter_mut().enumerate() {
                    let gx = (first.0 - padding + column as i64) as f64;
                    *position = (
                        (start.0 + across.0 * gx) as f32,
                        (start.1 + across.1 * gx) as f32,
                    );
                }
            }
        }
    }
}
