//! Per-pixel colour science: white balance, tone, curves and colour operators.
//!
//! Everything here is scalar math on `[f32; 3]` values. No image buffers, no
//! threading, no I/O. See
//! `docs/work/12_lightroom-workflow-restart/design.md` §5.
//!
//! Most callers want [`PixelPipeline`]: build it once per render from the
//! settings and the source's colour description, then call its stage
//! methods per pixel. The modules underneath expose the individual
//! operators and the pure curve functions for testing, plotting and reuse:
//!
//! - [`temperature`]: temperature and tint to and from chromaticity.
//! - [`camera`], [`relative_wb`]: white balance for camera and raster sources.
//! - [`base`]: exposure and base rendering (scene to display).
//! - [`tone`]: highlights, shadows, whites, blacks, contrast and curves.
//! - [`oklab`], [`color`]: the colour mixer, vibrance, saturation, grading
//!   and calibration.
//! - [`output`]: gamut mapping and encoding.

#![warn(missing_docs)]

pub mod base;
pub mod camera;
pub mod color;
mod lut;
mod math;
pub mod oklab;
pub mod output;
mod pipeline;
pub mod relative_wb;
pub mod temperature;
pub mod tone;

pub use pipeline::{LocalDelta, PixelPipeline, displayed_white_balance, pick_white_balance};
pub use tone::tone_base_value;
