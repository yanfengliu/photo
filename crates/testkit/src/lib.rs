//! Shared test fixtures: synthetic images written to temporary folders.
//!
//! Dev-dependency only. Everything is rendered from one procedural
//! photograph ([`scene`]) whose content is known exactly, so tests can check
//! what a decoder, a colour operator or an exporter did to it.
//!
//! - [`scene`], [`scene16`] and [`scene_linear`] render the photograph;
//!   [`patches`] and [`grey_ramp`] locate its flat, measurable regions.
//! - The `write_*` functions save it as JPEG, PNG, TIFF and DNG, optionally
//!   with EXIF ([`ExifSpec`]) or an embedded ICC profile.
//! - [`TempLibrary`] is a small ready-made folder tree for import tests.
//!
//! Nothing here ever reads or writes real user data: every writer refuses a
//! destination outside the system temporary directory.

mod dng;
mod exif;
mod icc;
mod library;
mod orient;
mod scene;
mod tiff;
mod writers;

pub use dng::{DngCamera, DngSpec, dng_camera, dng_preview_size, write_dng, write_dng_with};
pub use exif::ExifSpec;
pub use icc::icc_profile;
pub use library::{LibraryEntry, TempLibrary};
pub use scene::{
    LinearScene, MARKER_RGB, Patch, grey_ramp, marker_size, patches, radiance, scene, scene_linear,
    scene_with_marker, scene16,
};
pub use writers::{
    write_jpeg, write_jpeg_with_exif, write_png8, write_png16, write_tagged, write_tiff16,
    write_tiff16_with_exif,
};
