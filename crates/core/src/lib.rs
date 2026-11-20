//! Shared vocabulary for the photo workspace: plain data types and
//! colorimetric constants that every other crate agrees on.
//!
//! Nothing in here does I/O or loops over pixels. If two crates need to pass a
//! value to each other, its type lives here.

pub mod colorimetry;
pub mod develop;
pub mod geom;
pub mod image;
pub mod meta;
pub mod time;
