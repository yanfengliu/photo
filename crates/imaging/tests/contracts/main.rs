//! Contract tests for `photo-imaging`, written against the public API only.
//!
//! Each module pins behaviour a caller (the develop pipeline, the engine, the
//! UI) relies on: geometry round trips, resolution and region independence
//! (DESIGN §5.9 R1, R2 and R4), filter references, mask and healing semantics.

mod determinism;
mod filters;
mod geometry;
mod guided;
mod heal;
mod mask;
mod mask_views;
mod misc;
mod pyramid;
mod resample;
mod sliders;
mod suggest;
mod timing;
mod util;
