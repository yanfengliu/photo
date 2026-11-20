//! Healing and cloning spots: stage 1 of the develop pipeline.
//!
//! A spot replaces a disc around its target with the disc around its source.
//! **Clone** copies the pixels as they are. **Heal** copies them too, then
//! multiplies each channel by a smooth field that makes the copy's tone and
//! colour agree with what surrounds the target, so only the source's texture
//! arrives. This is seamless cloning in the sense of Perez, Gangnet and
//! Blake, "Poisson Image Editing" (SIGGRAPH 2003): the source plus a
//! harmonic interpolant of the mismatch along the boundary. Two things are
//! specific to this implementation:
//!
//! - the region is a disc, so the harmonic interpolant has a closed form
//!   (the Poisson integral over the boundary circle) and no linear system is
//!   solved;
//! - the mismatch is a **ratio** per channel rather than a difference, since
//!   in linear light shading multiplies texture: a source patch twice as
//!   bright as the target has texture twice as strong, and a ratio corrects
//!   both at once. A small floor keeps the ratio stable near black.

mod membrane;
mod suggest;

use photo_core::develop::{Spot, SpotMode};
use photo_core::image::SourceImage;
use rayon::prelude::*;

use crate::pyramid::SourcePyramid;
use crate::slider::fraction;

use membrane::Membrane;

pub use suggest::suggest_heal_source;

const CHANNELS: usize = SourceImage::CHANNELS;
const WHITE: f32 = SourceImage::WHITE as f32;
/// Spot radii are held at this many long edges: a spot that large covers
/// the whole picture from anywhere on it.
const MAX_RADIUS: f64 = 1.0;

/// How normalized spot coordinates turn into pixels of the image being
/// healed (the source itself, or one level of its pyramid).
#[derive(Clone, Copy)]
struct Frame {
    /// Pixels per unit of normalized x and y.
    per_unit: (f64, f64),
    /// Pixels per unit of normalized length (the long edge).
    long_edge: f64,
}

impl Frame {
    fn new(source_size: (u32, u32), pixels_per_source_pixel: f64) -> Self {
        let (width, height) = (f64::from(source_size.0), f64::from(source_size.1));
        Self {
            per_unit: (
                width * pixels_per_source_pixel,
                height * pixels_per_source_pixel,
            ),
            long_edge: width.max(height) * pixels_per_source_pixel,
        }
    }
}

/// A spot in pixels of the image it is applied to.
struct Stamp {
    mode: SpotMode,
    centre: (f64, f64),
    radius: f64,
    /// Coverage is 1 within `inner` of the centre and 0 beyond `outer`.
    inner: f64,
    outer: f64,
    /// Whole-pixel displacement from a target pixel to its source pixel.
    offset: (i64, i64),
    opacity: f32,
}

impl Stamp {
    /// `None` for a spot that cannot change anything.
    fn new(spot: &Spot, frame: Frame) -> Option<Self> {
        let centre = (
            f64::from(spot.target.x) * frame.per_unit.0,
            f64::from(spot.target.y) * frame.per_unit.1,
        );
        let offset = (
            (f64::from(spot.source.x) - f64::from(spot.target.x)) * frame.per_unit.0,
            (f64::from(spot.source.y) - f64::from(spot.target.y)) * frame.per_unit.1,
        );
        let opacity = fraction(spot.opacity) as f32;
        let finite = [centre.0, centre.1, offset.0, offset.1]
            .iter()
            .all(|v| v.is_finite());
        if !finite || spot.radius.is_nan() || spot.radius <= 0.0 || opacity <= 0.0 {
            return None;
        }
        let radius = f64::from(spot.radius).min(MAX_RADIUS) * frame.long_edge;
        // Any source farther away than this reads nothing but the
        // replicated edge, for every pixel the spot and its sampling ring
        // touch; holding the offset here keeps it a reasonable integer.
        let reach = frame.per_unit.0 + frame.per_unit.1 + 4.0 * radius + 8.0;
        let offset = (offset.0.clamp(-reach, reach), offset.1.clamp(-reach, reach));
        let hardness = 1.0 - fraction(spot.feather);
        let (mut inner, mut outer) = (radius * hardness, radius);
        if outer - inner < 1.0 {
            // A one-pixel antialiased rim centred on the nominal edge.
            let middle = (inner + outer) / 2.0;
            (inner, outer) = (middle - 0.5, middle + 0.5);
        }
        Some(Self {
            mode: spot.mode,
            centre,
            radius,
            inner,
            outer,
            offset: (offset.0.round() as i64, offset.1.round() as i64),
            opacity,
        })
    }

    /// Blend weight of the repair at `distance` pixels from the centre.
    fn alpha(&self, distance: f64) -> f32 {
        let t = ((distance - self.inner) / (self.outer - self.inner)).clamp(0.0, 1.0);
        self.opacity * (1.0 - t * t * (3.0 - 2.0 * t)) as f32
    }

    /// The pixels whose centres can lie within `outer` of the spot's
    /// centre, as inclusive ranges clipped to the image. `None` when the
    /// spot misses the image entirely.
    fn bounds(&self, width: usize, height: usize) -> Option<(usize, usize, usize, usize)> {
        let span = |centre: f64, len: usize| {
            let low = (centre - self.outer - 0.5).ceil().max(0.0);
            let high = (centre + self.outer - 0.5).floor().min(len as f64 - 1.0);
            (low <= high).then_some((low as usize, high as usize))
        };
        let (x0, x1) = span(self.centre.0, width)?;
        let (y0, y1) = span(self.centre.1, height)?;
        Some((x0, y0, x1, y1))
    }
}

/// The pixel at `(x, y)`, replicating the edge for positions outside.
fn pixel_clamped(image: &SourceImage, x: i64, y: i64) -> [u16; 3] {
    let x = x.clamp(0, i64::from(image.width) - 1) as usize;
    let y = y.clamp(0, i64::from(image.height) - 1) as usize;
    let index = (y * image.width as usize + x) * CHANNELS;
    [
        image.pixels[index],
        image.pixels[index + 1],
        image.pixels[index + 2],
    ]
}

fn apply(image: &mut SourceImage, stamp: &Stamp) {
    let (width, height) = (image.width as usize, image.height as usize);
    let Some((x0, y0, x1, y1)) = stamp.bounds(width, height) else {
        return;
    };
    // Read everything this spot needs before writing anything: the source
    // disc may overlap the target disc. Kept as `u16`, so even a spot the
    // size of the picture costs no more memory than the picture.
    let patch_width = x1 - x0 + 1;
    let patch: Vec<[u16; 3]> = (y0..=y1)
        .flat_map(|y| (x0..=x1).map(move |x| (x, y)))
        .map(|(x, y)| pixel_clamped(image, x as i64 + stamp.offset.0, y as i64 + stamp.offset.1))
        .collect();
    let membrane = (stamp.mode == SpotMode::Heal).then(|| Membrane::new(image, stamp));

    let stride = width * CHANNELS;
    image.pixels[y0 * stride..(y1 + 1) * stride]
        .par_chunks_exact_mut(stride)
        .enumerate()
        .for_each(|(row_index, row)| {
            let dy = (y0 + row_index) as f64 + 0.5 - stamp.centre.1;
            let sources = &patch[row_index * patch_width..(row_index + 1) * patch_width];
            let targets = row[x0 * CHANNELS..(x1 + 1) * CHANNELS].chunks_exact_mut(CHANNELS);
            for (column, (target, &source)) in targets.zip(sources).enumerate() {
                let dx = (x0 + column) as f64 + 0.5 - stamp.centre.0;
                let alpha = stamp.alpha(dx.hypot(dy));
                if alpha <= 0.0 {
                    continue;
                }
                let source = source.map(f32::from);
                let repair = match &membrane {
                    Some(membrane) => membrane.matched(source, dx, dy),
                    None => source,
                };
                for (value, wanted) in target.iter_mut().zip(repair) {
                    let current = f32::from(*value);
                    *value = (current + alpha * (wanted - current))
                        .round()
                        .clamp(0.0, WHITE) as u16;
                }
            }
        });
}

fn heal(image: &SourceImage, frame: Frame, spots: &[Spot]) -> SourceImage {
    // A parallel copy: a full-resolution source is hundreds of megabytes.
    const CHUNK: usize = 1 << 20;
    let mut pixels = vec![0u16; image.pixels.len()];
    pixels
        .par_chunks_mut(CHUNK)
        .zip(image.pixels.par_chunks(CHUNK))
        .for_each(|(to, from)| to.copy_from_slice(from));
    let mut out = SourceImage {
        width: image.width,
        height: image.height,
        pixels,
        color: image.color,
    };
    for stamp in spots.iter().filter_map(|spot| Stamp::new(spot, frame)) {
        apply(&mut out, &stamp);
    }
    out
}

/// Apply `spots`, in order, to a full-resolution source, returning the
/// repaired copy. `image` itself is never modified.
///
/// For each spot, every pixel within `radius` of `target` is blended toward
/// a repair taken from the same position relative to `source`:
///
/// - **Clone**: the source pixel itself.
/// - **Heal**: the source pixel scaled, per channel, by a factor that
///   varies smoothly across the spot: `(source + f) * gain - f`, with `f`
///   1/256 of white so that near black the correction becomes an offset
///   rather than a large gain on noise. The gain is the harmonic
///   interpolation, over the disc, of the ratio between the target's
///   surroundings and the source's surroundings, each averaged in windows
///   on a ring just outside the spot. The repair therefore has the source's
///   texture at the target's own low-frequency tone and colour, and meets
///   its surroundings without a seam. Gains are limited to 16x either way,
///   and each window mean reads at most about a thousand pixels however
///   large the spot.
///
/// The blend weight is `opacity / 100` times a falloff that is 1 out to
/// `1 - feather / 100` of the radius and eases to 0 at the radius (feather 0
/// is a hard edge, antialiased over one pixel; 100 is soft from the centre).
///
/// Details a caller can rely on:
///
/// - Positions are normalized to the image and `radius` to its long edge,
///   as `photo_core` defines them. Radii above 1 (the whole long edge) are
///   held at 1, which bounds the work a spot can ask for.
/// - The source offset is rounded to whole pixels, so cloned texture is
///   never softened by interpolation. With opacity 100, pixels at least half
///   a pixel inside the hard edge are exact copies of their source pixels;
///   pixels at least half a pixel outside the radius are untouched.
/// - A later spot sees the result of earlier ones, and a spot whose source
///   overlaps its own target reads the pixels as they were before it.
/// - Source positions outside the image read the replicated edge, however
///   far out. Spots with a non-finite position, no radius or no opacity
///   are skipped.
/// - The result does not depend on the thread count.
///
/// # Panics
///
/// When `image.pixels` does not match its dimensions.
pub fn heal_image(image: &SourceImage, spots: &[Spot]) -> SourceImage {
    assert_eq!(
        image.pixels.len(),
        image.width as usize * image.height as usize * CHANNELS,
        "source image buffer does not match its dimensions"
    );
    heal(image, Frame::new((image.width, image.height), 1.0), spots)
}

/// [`heal_image`] for one level of a pyramid: the same spots, scaled to that
/// level, so a reduced preview shows the repair the full-resolution render
/// will have. The pyramid is not modified; put the result back with
/// [`SourcePyramid::with_level`].
///
/// Level 0 gives exactly `heal_image(pyramid.level(0), spots)`. At coarser
/// levels the radius and offset shrink with the level (the offset is again
/// rounded to whole pixels of that level), so the result approximates the
/// full-resolution repair averaged down.
///
/// # Panics
///
/// When `level` is not below [`SourcePyramid::level_count`].
pub fn heal_level(pyramid: &SourcePyramid, level: usize, spots: &[Spot]) -> SourceImage {
    let scale = 0.5f64.powi(level as i32);
    heal(
        pyramid.level(level),
        Frame::new(pyramid.source_size(), scale),
        spots,
    )
}
