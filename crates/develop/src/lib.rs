//! The develop pipeline: the single owner of "settings + source pixels in,
//! rendered pixels out". Previews, thumbnails, the histogram and export
//! all render through [`Renderer`] (DESIGN §3.2, §5).
//!
//! # Stages
//!
//! A render runs the stages of DESIGN §5.2 in order, each skipped when its
//! settings are neutral:
//!
//! | # | Stage | How |
//! | --- | --- | --- |
//! | 1 | Heal | The pyramid level the view samples, healed and cached until the spots change. |
//! | 2 | Resample | The view (plus padding) pulled through the geometry. |
//! | 3 | Input | The colour crate's input transform, then lens vignetting. |
//! | 4 | Denoise | Wavelet soft thresholding of luminance and colour. |
//! | — | *cache A* | Stages 1 to 4, reused while only later settings change. |
//! | 5 | Exposure, highlights, shadows, base curve | Per pixel, with the edge-aware base from the whole-image guides. |
//! | 6 | Tone | Per pixel. |
//! | 7 | Colour | Per pixel. |
//! | 8 | Presence | Dehaze and clarity per pixel from the guides; texture on the view. |
//! | 9 | Sharpen | Unsharp mask on perceptual luminance. |
//! | 10 | Effects | Post-crop vignette and grain, in output coordinates. |
//! | 11 | Output | Gamut map and encode, 8 or 16 bits. |
//!
//! Masks are not a stage: each active mask is rasterized once per view and
//! every adjustable stage reads the pixel's effective value `global +
//! Σ alphaᵢ · amountᵢ · localᵢ`.
//!
//! # Region contracts
//!
//! - R1: a tile ([`View::tile`](photo_imaging::View::tile)) renders the
//!   pixels of that region of the whole: the spatial stages that run on
//!   the view (denoise, texture, sharpen) are given a region padded by
//!   their support, and the large-radius ones (the tone base, dehaze,
//!   clarity) read whole-image guides fitted on the source's analysis
//!   reduction, the same for every tile.
//! - R2: a reduced view approximates the full render scaled down: radii are
//!   defined in source pixels or fractions of the long edge and scaled by
//!   the view.
//! - R3: whole-image statistics come from the source's [`Analysis`].
//! - R4: the same bytes on any number of threads.
//!
//! See `docs/work/12_lightroom-workflow-restart/design.md`.

#![warn(missing_docs)]

mod analysis;
mod auto_tone;
mod cancel;
mod denoise;
mod effects;
mod front;
mod geometry;
mod guides;
mod keys;
mod lens;
mod locate;
mod masks;
mod plan;
mod point;
mod presence;
mod render;
mod sharpen;
mod source;
mod white_balance;

pub use analysis::{Analysis, REDUCTION_EDGE};
pub use auto_tone::{auto_tone, tone_curve_sampler};
pub use cancel::CancelToken;
pub use geometry::{actual_size_view, fit_view, geometry, output_size, scaled_view};
pub use masks::mask_alpha;
pub use photo_imaging::Histogram;
pub use render::{Quality, RenderRequest, Renderer};
pub use source::Source;
pub use white_balance::{as_shot_white_balance, auto_white_balance, pick_white_balance};

/// The histogram of a rendered image, for the histogram panel: per-channel
/// and luminance counts and the clipping fractions (see
/// [`photo_imaging::histogram`]).
pub fn histogram(image: &photo_core::image::Rgba8Image) -> Histogram {
    photo_imaging::histogram(image)
}
