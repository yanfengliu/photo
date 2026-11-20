//! Whole-image statistics, computed once per source from a fixed-size
//! reduction (DESIGN §5.9, contract R3).

use photo_color::PixelPipeline;
use photo_core::develop::{DevelopSettings, WhiteBalance};
use photo_core::image::SourceImage;
use photo_imaging::filter::min_filter;
use photo_imaging::{Geometry, Plane, SourcePyramid, View, resample};

use crate::source::unit_pixel;
use crate::white_balance::grey_world;

// ---- Tunable constants ----------------------------------------------------

/// Long edge of the reduction every whole-image statistic is taken from.
pub const REDUCTION_EDGE: u32 = 512;
/// A source-native channel at or above this counts as clipped.
pub(crate) const CLIPPED: f32 = 0.98;
/// The dark channel's window, as a fraction of the long edge: He, Sun and
/// Tang's 15 x 15 patch on their roughly 600-pixel images.
pub(crate) const DARK_CHANNEL_RADIUS: f32 = 0.012;
/// Fraction of the pixels, the haziest by the dark channel, whose mean
/// colour is the atmospheric light (He, Sun and Tang take the top 0.1%).
const HAZE_FRACTION: f32 = 0.001;

// ---------------------------------------------------------------------------

/// What every render of a source shares, computed once in
/// [`Source::new`](crate::Source::new) from a reduction of the whole image
/// whose long edge is [`REDUCTION_EDGE`] (or the image itself when it is
/// smaller). Because every tile and every view reads the same analysis,
/// they agree on the dehaze atmospheric light, on auto tone and on auto
/// white balance (contract R3).
#[derive(Clone, Debug)]
pub struct Analysis {
    reduction: SourceImage,
    haze: Vec<u32>,
    auto_white_balance: WhiteBalance,
}

impl Analysis {
    pub(crate) fn new(pyramid: &SourcePyramid) -> Self {
        let (width, height) = pyramid.source_size();
        let geometry = Geometry::identity(width, height);
        let reduced = resample(pyramid, &geometry, &View::fit(&geometry, REDUCTION_EDGE), 0);
        let reduction = SourceImage {
            width: reduced.width() as u32,
            height: reduced.height() as u32,
            pixels: reduced
                .as_slice()
                .iter()
                .map(|v| (v.clamp(0.0, 1.0) * f32::from(SourceImage::WHITE)).round() as u16)
                .collect(),
            color: pyramid.level(0).color,
        };
        let haze = haze_pixels(&reduction);
        let auto_white_balance = grey_world(&reduction);
        Self {
            reduction,
            haze,
            auto_white_balance,
        }
    }

    /// The reduction itself: source-native linear RGB, long edge at most
    /// [`REDUCTION_EDGE`], area-averaged from the pyramid.
    pub fn reduction(&self) -> &SourceImage {
        &self.reduction
    }

    /// The grey-world white balance estimate (see
    /// [`auto_white_balance`](crate::auto_white_balance)).
    pub fn auto_white_balance(&self) -> WhiteBalance {
        self.auto_white_balance
    }

    /// Row-major indices into [`Self::reduction`] of the haziest pixels:
    /// the brightest 0.1% of the dark channel of a neutral rendering,
    /// clipped pixels excluded. Their mean colour, as the current settings
    /// render it, is the dehaze atmospheric light.
    pub fn haze_pixels(&self) -> &[u32] {
        &self.haze
    }
}

/// The dark channel prior's choice of atmospheric light (He, Sun and Tang,
/// "Single Image Haze Removal Using Dark Channel Prior", CVPR 2009): the
/// pixels whose dark channel (the minimum over the colour channels and over
/// a window) is brightest, on a rendering with default settings. Clipped
/// pixels are left out, so a light source is not mistaken for haze; when
/// every pixel is clipped they are all candidates.
fn haze_pixels(reduction: &SourceImage) -> Vec<u32> {
    let (width, height) = (reduction.width as usize, reduction.height as usize);
    let pipeline = PixelPipeline::new(&DevelopSettings::default(), &reduction.color);
    let dark = Plane::from_fn(width, height, |x, y| {
        let source = unit_pixel(reduction, y * width + x);
        let shown = pipeline.scene_to_display(pipeline.input(source, None), 0.0, None);
        shown[0].min(shown[1]).min(shown[2])
    });
    let radius = (DARK_CHANNEL_RADIUS * width.max(height) as f32)
        .round()
        .max(1.0) as usize;
    let dark = min_filter(&dark, radius);
    let clipped = |index: usize| {
        let pixel = unit_pixel(reduction, index);
        pixel.iter().any(|&channel| channel >= CLIPPED)
    };
    let mut candidates: Vec<u32> = (0..width * height)
        .filter(|&index| !clipped(index))
        .map(|index| index as u32)
        .collect();
    if candidates.is_empty() {
        candidates = (0..(width * height) as u32).collect();
    }
    let values = dark.as_slice();
    candidates.sort_by(|&a, &b| {
        values[b as usize]
            .total_cmp(&values[a as usize])
            .then(a.cmp(&b))
    });
    let keep = ((candidates.len() as f32 * HAZE_FRACTION).ceil() as usize).max(1);
    candidates.truncate(keep);
    candidates.sort_unstable();
    candidates
}
