//! Spatial filters on [`Plane`](crate::Plane)s.
//!
//! What they have in common:
//!
//! - **Radii are in pixels of the plane they are given.** A parameter
//!   defined at full resolution is multiplied by the view scale before it
//!   gets here. Radii are fractional wherever that has a meaning, so a
//!   result changes smoothly with the scale instead of in steps.
//! - **Cost does not grow with the radius.**
//! - **Edges are replicated**: a window that hangs over the plane's edge
//!   reads the nearest edge sample.
//! - **Every filter states its support radius**, as a number or a
//!   `*_support` function: the distance beyond which input cannot affect an
//!   output pixel. Give a filter a region padded by its support and crop the
//!   result, and it equals the same region of the filtered whole to well
//!   under `1e-4` (contract R1). Running sums are kept in `f64` so that this
//!   holds no matter where the padded region starts.
//! - **Output depends only on the input**, never on the thread count
//!   (contract R4).
//!
//! To filter an [`RgbImage`](crate::RgbImage), split it with
//! [`RgbImage::channel`](crate::RgbImage::channel) or
//! [`RgbImage::map_to_plane`](crate::RgbImage::map_to_plane).

mod box_blur;
mod gaussian;
mod guided;
mod min;
mod sharpen;
mod wavelet;

pub use box_blur::{box_blur, box_support};
pub use gaussian::{gaussian_blur, gaussian_support};
pub use guided::{
    fast_guided_filter, fast_guided_subsample, fast_guided_support, fast_self_guided_filter,
    guided_filter, guided_support, self_guided_filter,
};
pub use min::min_filter;
pub use sharpen::{gradient_magnitude, high_pass};
pub use wavelet::{
    MAX_WAVELET_LEVELS, WaveletDecomposition, soft_threshold, wavelet_decompose, wavelet_denoise,
    wavelet_support,
};
