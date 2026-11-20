//! Where every output pixel comes from: the chain of frames defined in
//! `photo_core::develop::geometry` as one invertible mapping.

mod crop;
mod distortion;
mod matrix;
mod perspective;

use photo_core::develop::{
    Crop, CropRect, DevelopSettings, LensCorrections, Orientation, Transform,
};
use photo_core::geom::Point;

use distortion::Distortion;
use matrix::{IDENTITY, Mat3, apply, invert, mul, scaled};

/// Smallest homogeneous `w` still treated as in front of the virtual camera,
/// on a scale where the centre of the output has `w = 1`.
const MIN_W: f64 = 1e-6;

/// Slack, in source pixels, when deciding whether a crop boundary point lies
/// inside the source. Covers the `f32` rounding of normalized crop corners.
const INSIDE_TOLERANCE: f64 = 1e-3;

/// The mapping between the source image and the output a set of geometry
/// settings produces: source -> corrected (lens distortion) -> oriented
/// (quarter turns, mirror) -> transformed (perspective) -> output (crop).
///
/// All coordinates are pixels with pixel centres at `+0.5`: the top-left
/// pixel covers `0..1` and its centre is `(0.5, 0.5)`. Output coordinates are
/// in full-resolution output pixels, independent of any [`View`](crate::View).
///
/// Guarantees:
///
/// - With default settings every mapping is exactly the identity.
/// - Quarter turns, mirrors and upright crops map pixel centres onto pixel
///   centres exactly, so they rearrange pixels without interpolating.
/// - [`Self::out_to_src`] and [`Self::src_to_out`] are inverses of each other
///   (far below a hundredth of a pixel) wherever the latter is defined.
///
/// # Slider signs
///
/// | Setting | Effect of a positive value |
/// | --- | --- |
/// | `lens.distortion` | Corrects barrel distortion: the middle of the picture shrinks and the middle of each edge pulls inward, while the centre and the four corners stay where they are. `+-100` is a radial coefficient of `+-0.25` (see below). |
/// | `orientation` | As `photo_core` defines it: mirror left to right first, then clockwise quarter turns. |
/// | `transform.vertical` | Magnifies the bottom of the picture relative to the top, as if the camera had been tilted up further. Negative magnifies the top, which straightens verticals that converge upward. The tilt is `atan(value / 100)`, 45 degrees at `+-100`; larger values are clamped. |
/// | `transform.horizontal` | Magnifies the right side relative to the left; negative magnifies the left. Same scale as `vertical`. |
/// | `transform.rotate` | Rotates the picture clockwise on screen about the frame centre, in degrees. |
/// | `transform.aspect` | Makes the picture taller and narrower: height times `2^(value / 200)`, width divided by the same, so the area is unchanged. |
/// | `transform.scale` | Percentage magnification about the frame centre; 100 is none. Values below 1 are treated as 1. |
/// | `transform.x_offset` | Moves the picture right by `value / 200` of the frame width. |
/// | `transform.y_offset` | Moves the picture down by `value / 200` of the frame height. |
/// | `crop.angle` | Rotates the picture counter-clockwise on screen (the crop rectangle turns clockwise in the frame), in degrees, as `photo_core` defines it. |
///
/// Lens distortion is radial about the source centre. With `r` the distance
/// from the centre divided by the half-diagonal and `a = 0.25 * distortion /
/// 100`, a corrected-frame point at radius `r` is read from source radius
/// `r * (1 + a * (1 - r^2))`, continued linearly beyond the corners. It is
/// strictly increasing in `r` for every slider value.
///
/// Perspective treats the oriented frame as the image of a camera whose
/// focal length is the frame diagonal and applies the homography `K R K^-1`
/// of rotating that camera (roll, then horizontal tilt, then vertical
/// tilt), followed by the translation and uniform scale that keep the frame
/// centre fixed with an area magnification of 1 there; then aspect, scale
/// and offsets, in that order.
#[derive(Clone, Debug, PartialEq)]
pub struct Geometry {
    source_size: (u32, u32),
    frame_size: (u32, u32),
    output_size: (u32, u32),
    output_to_corrected: Mat3,
    corrected_to_output: Mat3,
    frame_to_corrected: Mat3,
    /// True when `output_to_corrected` has no perspective row.
    affine: bool,
    distortion: Distortion,
    footprint: f64,
}

impl Geometry {
    /// Build the mapping for a source of `source_width` x `source_height`
    /// pixels. Never fails: zero dimensions are treated as 1, non-finite
    /// settings as their neutral value, and an unusable crop rectangle as the
    /// whole frame.
    pub fn new(
        source_width: u32,
        source_height: u32,
        lens: &LensCorrections,
        orientation: Orientation,
        transform: &Transform,
        crop: &Crop,
    ) -> Self {
        let source_size = (source_width.max(1), source_height.max(1));
        let (source_w, source_h) = (f64::from(source_size.0), f64::from(source_size.1));
        let orientation = Orientation::new(orientation.quarter_turns, orientation.flip_horizontal);
        let frame_size = orientation.apply_to_size(source_size.0, source_size.1);
        let (frame_w, frame_h) = (f64::from(frame_size.0), f64::from(frame_size.1));

        let orient = orientation_matrix(source_w, source_h, orientation);
        let warp = perspective::transform_matrix(frame_w, frame_h, transform);
        let (output_to_frame, output_size) = crop::crop_matrix(frame_size.0, frame_size.1, crop);

        let frame_to_corrected = unit_w_at(
            &invert(&mul(&warp, &orient)).unwrap_or(IDENTITY),
            frame_w / 2.0,
            frame_h / 2.0,
        );
        let output_to_corrected = unit_w_at(
            &mul(&frame_to_corrected, &output_to_frame),
            f64::from(output_size.0) / 2.0,
            f64::from(output_size.1) / 2.0,
        );
        let corrected_to_output = invert(&output_to_corrected).unwrap_or(IDENTITY);

        let mut geometry = Self {
            source_size,
            frame_size,
            output_size,
            output_to_corrected,
            corrected_to_output,
            frame_to_corrected,
            affine: output_to_corrected[2] == [0.0, 0.0, 1.0],
            distortion: Distortion::new(source_w, source_h, lens.distortion),
            footprint: 1.0,
        };
        geometry.footprint = geometry.footprint_at_output_centre();
        geometry
    }

    /// The mapping for the geometry fields of `settings`.
    pub fn from_settings(
        source_width: u32,
        source_height: u32,
        settings: &DevelopSettings,
    ) -> Self {
        Self::new(
            source_width,
            source_height,
            &settings.lens,
            settings.orientation,
            &settings.transform,
            &settings.crop,
        )
    }

    /// The mapping with every setting at its default: output equals source.
    pub fn identity(source_width: u32, source_height: u32) -> Self {
        Self::new(
            source_width,
            source_height,
            &LensCorrections::default(),
            Orientation::IDENTITY,
            &Transform::default(),
            &Crop::default(),
        )
    }

    /// Width and height of the source this mapping was built for, in pixels.
    pub fn source_size(&self) -> (u32, u32) {
        self.source_size
    }

    /// Width and height of the oriented (and transformed) frame, in pixels:
    /// the source dimensions, swapped for an odd number of quarter turns.
    /// This is the frame a [`CropRect`] is normalized to.
    pub fn frame_size(&self) -> (u32, u32) {
        self.frame_size
    }

    /// Width and height of the output at full resolution, in whole pixels:
    /// the crop rectangle's sides in frame pixels, rounded, at least 1.
    pub fn output_size(&self) -> (u32, u32) {
        self.output_size
    }

    /// The source position shown at output position `p`.
    ///
    /// Total: every finite input gives a finite result. Positions the
    /// settings push outside the source map to coordinates outside
    /// `0..width, 0..height`; positions beyond the horizon of an extreme
    /// perspective map far outside it, in no particular direction.
    pub fn out_to_src(&self, p: Point) -> Point {
        let (x, y) = self.map_out_to_src(f64::from(p.x), f64::from(p.y));
        Point::new(x as f32, y as f32)
    }

    /// The output position at which source position `p` is shown.
    ///
    /// The result is **not** limited to the output rectangle: content the
    /// crop cuts away comes back with coordinates outside
    /// `0..output_width, 0..output_height`, so overlays can still be drawn
    /// for it. `None` only when `p` is not finite or lies at or beyond the
    /// horizon of the perspective transform, where it has no image at all.
    pub fn src_to_out(&self, p: Point) -> Option<Point> {
        if !p.x.is_finite() || !p.y.is_finite() {
            return None;
        }
        let (x, y) = self
            .distortion
            .source_to_corrected(f64::from(p.x), f64::from(p.y));
        let [x, y, w] = apply(&self.corrected_to_output, x, y);
        (w > MIN_W).then(|| Point::new((x / w) as f32, (y / w) as f32))
    }

    /// Source pixels per output pixel (the square root of the area ratio) at
    /// the centre of the output. 1 for orientation and crop alone; lens
    /// distortion and the transform's scale change it, and so does
    /// perspective when the crop is not centred on the frame (the transform
    /// keeps the magnification at 1 only at the frame centre). `aspect`
    /// preserves area and does not.
    ///
    /// [`resample`](crate::resample()) multiplies this by the view's own
    /// reduction to pick a pyramid level and a kernel. It is a property of
    /// the whole geometry, never of a region, so every tile of a view
    /// agrees on it; the price is that parts of the picture the geometry
    /// magnifies differently are filtered as the centre is.
    pub fn source_pixels_per_output_pixel(&self) -> f32 {
        self.footprint as f32
    }

    /// The largest crop rectangle that shows no empty area: centred in the
    /// frame, with pixel aspect ratio `crop_aspect` (width / height),
    /// turned by `angle_degrees` (the [`Crop::angle`] it will be used with),
    /// and with its whole boundary inside the source image.
    ///
    /// Uses this geometry's lens, orientation and transform; its own crop is
    /// ignored. `None` when the centre of the frame shows no image (the
    /// transform has pushed the picture away from it), or so little that
    /// the rectangle would have a side under [`CropRect::MIN_SIDE`]. A
    /// non-positive or non-finite `crop_aspect` means the frame's own
    /// aspect ratio.
    ///
    /// The result is the true maximum for perspective alone and for
    /// distortion alone. When both are combined with offsets it is still
    /// free of empty areas but may be slightly smaller than possible.
    pub fn largest_valid_crop(&self, angle_degrees: f32, crop_aspect: f32) -> Option<CropRect> {
        let (frame_w, frame_h) = (f64::from(self.frame_size.0), f64::from(self.frame_size.1));
        let (source_w, source_h) = (f64::from(self.source_size.0), f64::from(self.source_size.1));
        crop::largest_centred(frame_w, frame_h, angle_degrees, crop_aspect, |x, y| {
            let [cx, cy, w] = apply(&self.frame_to_corrected, x, y);
            if w <= MIN_W {
                return false;
            }
            let (sx, sy) = self.distortion.corrected_to_source(cx / w, cy / w);
            (-INSIDE_TOLERANCE..=source_w + INSIDE_TOLERANCE).contains(&sx)
                && (-INSIDE_TOLERANCE..=source_h + INSIDE_TOLERANCE).contains(&sy)
        })
    }

    /// [`Self::out_to_src`] in double precision, for per-pixel work.
    #[inline]
    pub(crate) fn map_out_to_src(&self, x: f64, y: f64) -> (f64, f64) {
        let m = &self.output_to_corrected;
        let (x, y) = if self.affine {
            // No perspective: nothing to divide by, and nothing to round.
            (
                m[0][0] * x + m[0][1] * y + m[0][2],
                m[1][0] * x + m[1][1] * y + m[1][2],
            )
        } else {
            let [x, y, w] = apply(m, x, y);
            // Beyond the horizon there is no image; clamp so the result is
            // at least finite and far outside the source.
            let scale = 1.0 / if w >= MIN_W { w } else { MIN_W };
            (x * scale, y * scale)
        };
        self.distortion.corrected_to_source(x, y)
    }

    /// The footprint as `f64`, for level selection.
    pub(crate) fn footprint(&self) -> f64 {
        self.footprint
    }

    fn footprint_at_output_centre(&self) -> f64 {
        let (cx, cy) = (
            f64::from(self.output_size.0) / 2.0,
            f64::from(self.output_size.1) / 2.0,
        );
        // Central differences over one output pixel.
        let (right, left) = (
            self.map_out_to_src(cx + 0.5, cy),
            self.map_out_to_src(cx - 0.5, cy),
        );
        let (below, above) = (
            self.map_out_to_src(cx, cy + 0.5),
            self.map_out_to_src(cx, cy - 0.5),
        );
        let (ax, ay) = (right.0 - left.0, right.1 - left.1);
        let (bx, by) = (below.0 - above.0, below.1 - above.1);
        let area = (ax * by - ay * bx).abs();
        if area.is_finite() && area > 0.0 {
            area.sqrt()
        } else {
            1.0
        }
    }
}

/// The matrix taking source (and corrected-frame) pixels to oriented-frame
/// pixels, built in pixel units so it is exact: `Orientation::forward` in
/// normalized coordinates, scaled by the frame sizes.
fn orientation_matrix(source_width: f64, source_height: f64, orientation: Orientation) -> Mat3 {
    let (mut width, mut height) = (source_width, source_height);
    let mut matrix = IDENTITY;
    if orientation.flip_horizontal {
        matrix = [[-1.0, 0.0, width], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]];
    }
    for _ in 0..orientation.quarter_turns % 4 {
        // One clockwise quarter turn: (x, y) -> (height - y, x).
        matrix = mul(
            &[[0.0, -1.0, height], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0]],
            &matrix,
        );
        std::mem::swap(&mut width, &mut height);
    }
    matrix
}

/// `m` rescaled so its homogeneous coordinate has magnitude 1 at `(x, y)`.
/// Affine matrices come back bit-for-bit unchanged.
///
/// Only the magnitude is normalized, never the sign: these matrices are
/// inverses of a forward map whose `w` is positive in front of the virtual
/// camera, so their own `w` is positive exactly where there is an image, and
/// that must survive even when `(x, y)` itself lies beyond the horizon.
fn unit_w_at(m: &Mat3, x: f64, y: f64) -> Mat3 {
    let w = apply(m, x, y)[2].abs();
    if w.is_finite() && w != 0.0 && w != 1.0 {
        scaled(m, 1.0 / w)
    } else {
        *m
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_orientation_matrix_matches_the_normalized_definition() {
        let (width, height) = (640.0, 427.0);
        for turns in 0..4 {
            for flip in [false, true] {
                let orientation = Orientation::new(turns, flip);
                let matrix = orientation_matrix(width, height, orientation);
                let (frame_w, frame_h) = orientation.apply_to_size(width, height);
                for (nx, ny) in [(0.0f32, 0.0f32), (1.0, 0.0), (0.25, 0.75)] {
                    let expected = orientation.forward(Point::new(nx, ny));
                    let [x, y, w] = apply(&matrix, f64::from(nx) * width, f64::from(ny) * height);
                    assert_eq!(w, 1.0);
                    assert!(
                        (x - f64::from(expected.x) * frame_w).abs() < 1e-4,
                        "{orientation:?}"
                    );
                    assert!(
                        (y - f64::from(expected.y) * frame_h).abs() < 1e-4,
                        "{orientation:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn pixel_centres_stay_pixel_centres_under_orientation_and_upright_crops() {
        let crop = Crop {
            rect: Some(CropRect {
                cx: 0.37,
                cy: 0.61,
                width: 0.33,
                height: 0.41,
            }),
            ..Default::default()
        };
        for turns in 0..4 {
            for flip in [false, true] {
                let geometry = Geometry::new(
                    4001,
                    2667,
                    &LensCorrections::default(),
                    Orientation::new(turns, flip),
                    &Transform::default(),
                    &crop,
                );
                assert_eq!(geometry.footprint(), 1.0);
                for (x, y) in [(0.5, 0.5), (17.5, 400.5), (1000.5, 3.5)] {
                    let (sx, sy) = geometry.map_out_to_src(x, y);
                    assert_eq!(
                        (sx.fract(), sy.fract()),
                        (0.5, 0.5),
                        "turns {turns} flip {flip}"
                    );
                }
            }
        }
    }

    #[test]
    fn distortion_changes_the_footprint_at_the_centre() {
        let lens = LensCorrections {
            distortion: 100.0,
            ..Default::default()
        };
        let geometry = Geometry::new(
            600,
            400,
            &lens,
            Orientation::IDENTITY,
            &Transform::default(),
            &Crop::default(),
        );
        // Reading from r * (1 + a) near the centre: 1.25 source px per px.
        assert!(
            (geometry.footprint() - 1.25).abs() < 1e-3,
            "{}",
            geometry.footprint()
        );
    }
}
