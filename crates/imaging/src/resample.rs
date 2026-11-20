//! Stage 2 of the develop pipeline: pull a view of the output out of the
//! source pyramid through the geometry.

use crate::buffer::RgbImage;
use crate::geometry::Geometry;
use crate::pyramid::SourcePyramid;
use crate::sample::{Kernel, sample4};
use crate::view::View;

/// Tolerance, in octaves, when deciding that a view has reached the next
/// pyramid level: an exact half-size view must land on level 1 even after
/// its scale has been through `f32`.
const LEVEL_SLACK: f64 = 1e-3;

/// Which pyramid level a view is sampled from, and with which kernel.
struct Plan {
    level: usize,
    kernel: Kernel,
}

impl Plan {
    fn new(pyramid: &SourcePyramid, geometry: &Geometry, view: &View) -> Self {
        // Source pixels per view pixel. Depends only on the view's pixel
        // pitch, which every tile of a view shares exactly, and on a
        // constant of the whole geometry; never on where the view is. So
        // every tile of a render makes the same choice.
        let footprint = geometry.footprint() * view.grid(0).pitch();
        let coarsest = pyramid.level_count() - 1;
        let level = (footprint.log2() + LEVEL_SLACK)
            .floor()
            .clamp(0.0, coarsest as f64) as usize;
        let kernel = Kernel::for_footprint(footprint / f64::from(1u32 << level));
        Self { level, kernel }
    }
}

/// The pyramid level [`resample`] reads for `view`: the coarsest one whose
/// scale is still at or above the view's (DESIGN §5.2), taking the
/// geometry's own magnification into account. Level 0 for 1:1 and magnified
/// views.
///
/// It depends on the view's scale and the geometry, not on the view's
/// position, so it is the same for every tile of a view made with
/// [`View::tile`]. Heal that level (see [`heal_level`](crate::heal_level))
/// and swap it in with [`SourcePyramid::with_level`] before resampling.
pub fn resample_level(pyramid: &SourcePyramid, geometry: &Geometry, view: &View) -> usize {
    Plan::new(pyramid, geometry, view).level
}

/// Render `view` of the output from `pyramid`, surrounded by `padding` extra
/// view pixels on every side, as linear RGB in the source's own colour space
/// (`0..=1`).
///
/// The result is `view.width() + 2 * padding` by `view.height() + 2 *
/// padding`; pixel `(padding, padding)` is the view's top-left pixel.
/// Padding pixels are real renders of the output positions they stand for,
/// so a spatial filter that needs `n` pixels of support can be given
/// `padding = n` and its result cropped back.
///
/// Each pixel's output position is mapped to the source with
/// [`Geometry::out_to_src`] and read from the level [`resample_level`]
/// returns. How it is read depends on the footprint: how many pixels of that
/// level one view pixel covers, always below 2 except on the coarsest level.
///
/// - Footprint up to 1 (1:1 on the level, or magnified): a Catmull-Rom
///   cubic whose result is clamped to the range of the four nearest pixels,
///   so it is as sharp as a cubic in texture but cannot ring next to an
///   edge.
/// - Footprint 1.5 and up: a triangle filter as wide as the view pixel,
///   which averages the level instead of point sampling it.
/// - In between, a mix of the two that follows the footprint continuously.
///
/// That continuity holds within a level. Where a view crosses into the
/// next level (at each halving of its scale) the triangle on the finer
/// level gives way to the cubic on the coarser one, and fine-texture
/// contrast steps by roughly 15%: the price of reading one level rather
/// than blending two, which would also mean healing two.
///
/// The footprint is one number for the whole view (the view's pitch times
/// [`Geometry::source_pixels_per_output_pixel`]), so the same kernel is
/// used everywhere, including where perspective or lens distortion locally
/// magnify more or less than they do at the centre.
///
/// Either way the support is 4 x 4 level pixels, and anything outside the
/// source reads its nearest edge pixel.
///
/// Guarantees:
///
/// - At a footprint of 1 or less, a pixel whose source position is exactly
///   a pixel centre of the level returns that pixel exactly (`code /
///   65535`). With identity geometry at scale 1 the output is therefore the
///   source, bit for bit; the same holds for quarter turns, mirrors and
///   upright crops.
/// - R1: every pixel is a function of its own output position only, and a
///   tile made with [`View::tile`] has exactly its parent's pixel positions
///   and scale, so it is the same bits as that region of the whole render.
/// - R2: a half-size view approximates the full-size render averaged over
///   2 x 2 blocks. Where the view's pixels coincide with the pyramid's own
///   2 x 2 blocks (identity geometry, or an upright crop at an even offset)
///   the two are equal to 16-bit rounding.
/// - R4: the same bytes on any number of threads.
///
/// `geometry` should have been built for the pyramid's source size; if it
/// was not, the mapping is still applied as given.
pub fn resample(
    pyramid: &SourcePyramid,
    geometry: &Geometry,
    view: &View,
    padding: u32,
) -> RgbImage {
    let plan = Plan::new(pyramid, geometry, view);
    let image = pyramid.level(plan.level);
    let to_level = 1.0 / f64::from(1u32 << plan.level);
    let grid = view.grid(padding);

    let mut out = RgbImage::new(grid.width, grid.height);
    out.par_rows_mut(|row_index, row| {
        let y = grid.y(row_index);
        for (column, pixel) in row.iter_mut().enumerate() {
            let (source_x, source_y) = geometry.map_out_to_src(grid.x(column), y);
            *pixel = sample4(image, source_x * to_level, source_y * to_level, plan.kernel);
        }
    });
    out
}
