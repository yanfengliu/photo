//! Image-buffer primitives for the develop pipeline: planes, the source mip
//! pyramid, geometry mapping, resampling, spatial filters, mask
//! rasterization, healing, histograms, grain noise and thumbnail scaling.
//!
//! This crate knows nothing about the order of the pipeline's stages or
//! about what a tone or colour slider means; `photo-develop` composes these
//! primitives. See `docs/work/12_lightroom-workflow-restart/design.md`
//! (§5.1, §5.2, §5.7, §5.9).
//!
//! Conventions shared by everything here:
//!
//! - Pixel coordinates have pixel centres at `+0.5`.
//! - Spatial parameters are in pixels of the buffer they are applied to; the
//!   caller scales full-resolution values by the view scale (DESIGN §3.4).
//! - Buffers made for a [`View`] ([`resample`], [`rasterize_mask`],
//!   [`grain_plane`]) share one layout: the view's pixels surrounded by
//!   `padding` extra pixels on every side, each of which is a real render of
//!   the output position it stands for.
//! - Tile a render with [`View::tile`]: a tile is a window onto its parent's
//!   pixel grid, so [`resample`], [`rasterize_mask`] and [`grain_plane`]
//!   give it exactly the bits of that region of the whole (contract R1).
//! - Every spatial filter documents its support radius, so a caller can pad
//!   a region by that much and crop the result (contract R1).
//! - Settings arriving from outside are read defensively: values beyond
//!   their range are clamped, and no finite or non-finite setting produces
//!   a NaN in a buffer.
//! - Results are bit-identical on any number of threads (contract R4).
//!
//! What the pipeline's stages find here:
//!
//! | Stage (DESIGN §5.2) | Primitive |
//! | --- | --- |
//! | Heal | [`resample_level`], [`heal_level`], [`SourcePyramid::with_level`], [`suggest_heal_source`] |
//! | Resample | [`SourcePyramid`], [`Geometry`], [`View`], [`resample`] |
//! | Denoise | [`filter::wavelet_denoise`], [`filter::wavelet_decompose`] |
//! | Tone, presence | [`filter::self_guided_filter`], [`filter::fast_self_guided_filter`], [`filter::guided_filter`], [`filter::min_filter`], [`filter::gaussian_blur`], [`filter::box_blur`] |
//! | Sharpen | [`filter::high_pass`], [`filter::gradient_magnitude`], [`filter::soft_threshold`] |
//! | Effects | [`grain_noise`], [`grain_plane`] |
//! | Masks | [`rasterize_mask`] |
//! | After output | [`histogram`], [`downscale_rgba8`] |

#![warn(missing_docs)]

pub mod buffer;
pub mod downscale;
pub mod filter;
pub mod geometry;
pub mod grain;
pub mod heal;
pub mod histogram;
pub mod mask;
pub mod pyramid;
pub mod resample;
pub mod view;

mod sample;
mod slider;

pub use buffer::{Plane, RgbImage};
pub use downscale::downscale_rgba8;
pub use geometry::Geometry;
pub use grain::{grain_noise, grain_plane};
pub use heal::{heal_image, heal_level, suggest_heal_source};
pub use histogram::{Histogram, histogram};
pub use mask::rasterize_mask;
pub use pyramid::SourcePyramid;
pub use resample::{resample, resample_level};
pub use view::View;
