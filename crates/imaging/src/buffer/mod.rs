//! `f32` pixel buffers and the row-parallel helpers the rest of the crate is
//! built on.
//!
//! Every parallel helper here hands each row to the closure on its own, so a
//! result never depends on how many threads ran or on how rayon split the
//! work: same inputs, same bits (DESIGN §5.9 R4).

mod plane;
mod rgb;

pub use plane::Plane;
pub use rgb::RgbImage;

pub(crate) use plane::{par_column_strips, par_rows2_mut};
