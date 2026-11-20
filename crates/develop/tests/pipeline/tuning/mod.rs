//! The tuning grid: contact sheets of the pipeline's output across its
//! settings, for a person (or an agent) to look at. It does nothing unless
//! the environment variable `PHOTO_TUNING_GRID` is set; then it writes PNG
//! sheets into `target/tuning/` at the workspace root:
//!
//! `PHOTO_TUNING_GRID=1 cargo test -p photo-develop --test pipeline tuning -- --nocapture`
//!
//! The camera sheets use the testkit camera (the colour matrix, as-shot
//! neutral and baseline exposure of the testkit DNG) applied to the scene
//! directly: the DNG itself needs a decoder, which this crate does not
//! depend on.

mod basic;
mod font;
mod local;
mod looks;
mod seams;
mod sheet;

use std::path::PathBuf;

use photo_core::develop::DevelopSettings;
use photo_core::geom::Rect;
use photo_core::image::Rgba8Image;
use photo_develop::{CancelToken, Renderer, Source, actual_size_view, fit_view};
use photo_imaging::View;

use crate::fixtures::{camera_scene, camera_source, raster, request};

pub use sheet::Sheet;

/// Size of the scenes the sheets render, and of each cell.
pub const SCENE: (u32, u32) = (1536, 1024);
pub const CELL: (u32, u32) = (360, 240);

/// Where the sheets go.
fn directory() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../target/tuning")
}

/// The sources the sheets are made from.
pub struct Scenes {
    pub raster: Source,
    pub camera: Source,
    pub noisy: Source,
}

/// Deterministic noise in `[-0.5, 0.5)` per pixel and channel.
pub fn noise(x: u32, y: u32, channel: u32) -> f32 {
    let mut h = x.wrapping_mul(0x9e37_79b1)
        ^ y.wrapping_mul(0x85eb_ca77)
        ^ channel.wrapping_mul(0x27d4_eb2f);
    h ^= h >> 15;
    h = h.wrapping_mul(0x2c1b_3c6d);
    h ^= h >> 12;
    h = h.wrapping_mul(0x297a_2d39);
    h ^= h >> 15;
    (h >> 8) as f32 / 16_777_216.0 - 0.5
}

impl Scenes {
    fn new() -> Self {
        let (width, height) = SCENE;
        let (_, raster) = raster(width, height);
        // The camera scene with shot-like noise: 5% at middle grey, more in
        // the shadows, independent per channel.
        let noisy = camera_source(width, height, 0.25, |x, y| {
            let clean = photo_testkit::radiance(i64::from(x), i64::from(y), width, height);
            std::array::from_fn(|c| {
                let level = clean[c].max(0.0);
                let sigma = 0.05 * (level * 0.18).sqrt() + 0.002;
                (level + sigma * 3.4 * noise(x, y, c as u32)).max(0.0)
            })
        });
        Self {
            raster,
            camera: camera_scene(width, height),
            noisy,
        }
    }
}

/// A render of the whole output fitted into a cell.
pub fn thumb(renderer: &mut Renderer, source: &Source, settings: &DevelopSettings) -> Rgba8Image {
    let view = fit_view(source, settings, CELL.0, CELL.1);
    shot(renderer, source, settings, view)
}

/// A 1:1 render of the cell-sized window whose top-left is at `(x, y)` of
/// the output.
pub fn actual(
    renderer: &mut Renderer,
    source: &Source,
    settings: &DevelopSettings,
    x: f32,
    y: f32,
) -> Rgba8Image {
    let view = actual_size_view(Rect::new(x, y, x + CELL.0 as f32, y + CELL.1 as f32));
    shot(renderer, source, settings, view)
}

pub fn shot(
    renderer: &mut Renderer,
    source: &Source,
    settings: &DevelopSettings,
    view: View,
) -> Rgba8Image {
    renderer
        .render_rgba8(&request(source, settings, view), &CancelToken::new())
        .expect("not cancelled")
}

pub fn edit(change: impl FnOnce(&mut DevelopSettings)) -> DevelopSettings {
    crate::fixtures::edit(change)
}

#[test]
fn tuning_grid() {
    if std::env::var_os("PHOTO_TUNING_GRID").is_none() {
        return;
    }
    let dir = directory();
    let scenes = Scenes::new();
    basic::sheets(&scenes, &dir);
    looks::sheets(&scenes, &dir);
    local::sheets(&scenes, &dir);
    seams::sheets(&scenes, &dir);
    println!("tuning sheets written to {}", dir.display());
}
