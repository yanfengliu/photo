//! Geometry settings: orientation, crop, perspective transform and manual
//! lens corrections.
//!
//! Frames of reference, in the order the pipeline applies them:
//!
//! 1. **Source**: the decoded image (already upright per EXIF).
//! 2. **Corrected**: after lens distortion correction. Same dimensions.
//! 3. **Oriented**: after [`Orientation`] (quarter turns and mirroring).
//! 4. **Transformed**: after the perspective [`Transform`]. Same dimensions
//!    as the oriented frame.
//! 5. **Output**: the [`Crop`] rectangle cut out of the transformed frame.
//!
//! Normalized coordinates run from 0 to 1 across a frame's width and height.

use serde::{Deserialize, Serialize};

use crate::geom::Point;

/// A rotation in quarter turns plus an optional mirror: the eight ways to
/// lay a rectangle flat. The mirror is applied first, then the rotation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct Orientation {
    /// Clockwise quarter turns, 0 to 3.
    pub quarter_turns: u8,
    /// Mirror left-to-right before rotating.
    pub flip_horizontal: bool,
}

impl Orientation {
    pub const IDENTITY: Self = Self {
        quarter_turns: 0,
        flip_horizontal: false,
    };

    pub fn new(quarter_turns: u8, flip_horizontal: bool) -> Self {
        Self {
            quarter_turns: quarter_turns % 4,
            flip_horizontal,
        }
    }

    /// This orientation followed by a quarter turn clockwise.
    pub fn rotated_cw(self) -> Self {
        Self::new(self.quarter_turns + 1, self.flip_horizontal)
    }

    /// This orientation followed by a quarter turn counter-clockwise.
    pub fn rotated_ccw(self) -> Self {
        Self::new(self.quarter_turns + 3, self.flip_horizontal)
    }

    /// This orientation followed by a left-to-right mirror of the result.
    pub fn flipped_horizontal(self) -> Self {
        Self::new(4 - self.quarter_turns % 4, !self.flip_horizontal)
    }

    /// This orientation followed by a top-to-bottom mirror of the result.
    pub fn flipped_vertical(self) -> Self {
        Self::new(6 - self.quarter_turns % 4, !self.flip_horizontal)
    }

    /// True when width and height trade places.
    pub fn swaps_axes(self) -> bool {
        self.quarter_turns % 2 == 1
    }

    /// Dimensions of the oriented frame for a source of `width` x `height`.
    pub fn apply_to_size<T>(self, width: T, height: T) -> (T, T) {
        if self.swaps_axes() {
            (height, width)
        } else {
            (width, height)
        }
    }

    /// Map a normalized point in the unoriented frame to the oriented frame.
    pub fn forward(self, p: Point) -> Point {
        let mut p = if self.flip_horizontal {
            Point::new(1.0 - p.x, p.y)
        } else {
            p
        };
        for _ in 0..self.quarter_turns % 4 {
            p = Point::new(1.0 - p.y, p.x);
        }
        p
    }

    /// Inverse of [`Self::forward`].
    pub fn inverse(self, p: Point) -> Point {
        let mut p = p;
        for _ in 0..self.quarter_turns % 4 {
            p = Point::new(p.y, 1.0 - p.x);
        }
        if self.flip_horizontal {
            Point::new(1.0 - p.x, p.y)
        } else {
            p
        }
    }
}

/// The crop rectangle, in normalized coordinates of the transformed frame.
///
/// `(cx, cy)` is its centre. `width` and `height` are its side lengths as
/// fractions of the frame's width and height, measured along the rectangle's
/// own (possibly rotated) axes.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CropRect {
    pub cx: f32,
    pub cy: f32,
    pub width: f32,
    pub height: f32,
}

impl CropRect {
    /// The whole frame.
    pub const FULL: Self = Self {
        cx: 0.5,
        cy: 0.5,
        width: 1.0,
        height: 1.0,
    };

    /// Smallest allowed side, as a fraction of the frame.
    pub const MIN_SIDE: f32 = 0.01;

    /// The four corners in normalized frame coordinates (top-left, top-right,
    /// bottom-right, bottom-left of the rectangle itself) when the rectangle
    /// is rotated by `angle_degrees` about its centre in a frame whose pixel
    /// aspect ratio is `frame_aspect` (width / height).
    pub fn corners(&self, angle_degrees: f32, frame_aspect: f32) -> [Point; 4] {
        let (sin, cos) = angle_degrees.to_radians().sin_cos();
        let half_w = self.width * 0.5 * frame_aspect;
        let half_h = self.height * 0.5;
        [(-half_w, -half_h), (half_w, -half_h), (half_w, half_h), (-half_w, half_h)].map(|(dx, dy)| {
            Point::new(
                self.cx + (dx * cos - dy * sin) / frame_aspect,
                self.cy + dx * sin + dy * cos,
            )
        })
    }

    /// True when every corner lies inside the frame (with a small tolerance
    /// for rounding).
    pub fn fits(&self, angle_degrees: f32, frame_aspect: f32) -> bool {
        const TOLERANCE: f32 = 1e-4;
        self.corners(angle_degrees, frame_aspect).iter().all(|p| {
            p.x >= -TOLERANCE && p.x <= 1.0 + TOLERANCE && p.y >= -TOLERANCE && p.y <= 1.0 + TOLERANCE
        })
    }

    /// The largest rectangle centred in the frame with pixel aspect ratio
    /// `crop_aspect` (width / height) that still fits when rotated by
    /// `angle_degrees`.
    pub fn inscribed(angle_degrees: f32, frame_aspect: f32, crop_aspect: f32) -> Self {
        let (sin, cos) = angle_degrees.to_radians().sin_cos();
        let (sin, cos) = (sin.abs(), cos.abs());
        // Work in units of the frame height: the frame is `frame_aspect` wide
        // and 1 tall, the crop is `crop_aspect * h` wide and `h` tall.
        let by_width = frame_aspect / (crop_aspect * cos + sin);
        let by_height = 1.0 / (crop_aspect * sin + cos);
        let h = by_width.min(by_height);
        Self {
            cx: 0.5,
            cy: 0.5,
            width: (crop_aspect * h / frame_aspect).min(1.0),
            height: h.min(1.0),
        }
    }

    /// Pixel aspect ratio (width / height) of this rectangle in a frame of
    /// pixel aspect ratio `frame_aspect`.
    pub fn aspect(&self, frame_aspect: f32) -> f32 {
        self.width * frame_aspect / self.height
    }
}

/// The aspect ratio the crop tool holds the rectangle to.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CropConstraint {
    /// Keep the frame's own aspect ratio.
    #[default]
    Original,
    /// Any shape.
    Free,
    /// A fixed ratio such as 16 : 9.
    Ratio { width: f32, height: f32 },
}

/// Crop and straighten.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Crop {
    /// `None` means "as much of the frame as fits": the whole frame when
    /// `angle` is zero, otherwise the largest centred rectangle of the
    /// frame's own aspect ratio.
    pub rect: Option<CropRect>,
    /// Straighten angle in degrees, -45 to 45. Positive rotates the image
    /// counter-clockwise on screen.
    pub angle: f32,
    pub constraint: CropConstraint,
}

impl Crop {
    /// The rectangle the pipeline actually cuts out, for a transformed frame
    /// of pixel aspect ratio `frame_aspect` (width / height).
    pub fn resolved(&self, frame_aspect: f32) -> CropRect {
        match self.rect {
            Some(rect) => rect,
            None if self.angle == 0.0 => CropRect::FULL,
            None => CropRect::inscribed(self.angle, frame_aspect, frame_aspect),
        }
    }

    pub fn is_identity(&self) -> bool {
        self.angle == 0.0 && self.rect.is_none_or(|rect| rect == CropRect::FULL)
    }
}

/// Manual perspective transform. `vertical`, `horizontal`, `aspect`,
/// `x_offset` and `y_offset` run from -100 to 100; `rotate` is in degrees
/// (-10 to 10); `scale` is a percentage (50 to 150).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Transform {
    pub vertical: f32,
    pub horizontal: f32,
    pub rotate: f32,
    pub aspect: f32,
    pub scale: f32,
    pub x_offset: f32,
    pub y_offset: f32,
}

impl Default for Transform {
    fn default() -> Self {
        Self {
            vertical: 0.0,
            horizontal: 0.0,
            rotate: 0.0,
            aspect: 0.0,
            scale: 100.0,
            x_offset: 0.0,
            y_offset: 0.0,
        }
    }
}

impl Transform {
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }
}

/// Manual lens corrections. `distortion` and `vignetting` run from -100 to
/// 100 (positive `distortion` corrects barrel distortion, positive
/// `vignetting` brightens the corners); `vignetting_midpoint` 0 to 100.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LensCorrections {
    pub distortion: f32,
    pub vignetting: f32,
    pub vignetting_midpoint: f32,
}

impl Default for LensCorrections {
    fn default() -> Self {
        Self {
            distortion: 0.0,
            vignetting: 0.0,
            vignetting_midpoint: 50.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-5 && (a.y - b.y).abs() < 1e-5
    }

    fn all_orientations() -> impl Iterator<Item = Orientation> {
        (0..4).flat_map(|turns| [false, true].map(|flip| Orientation::new(turns, flip)))
    }

    #[test]
    fn a_clockwise_quarter_turn_moves_the_top_left_corner_to_the_top_right() {
        let turned = Orientation::IDENTITY.rotated_cw();
        assert!(close(turned.forward(Point::new(0.0, 0.0)), Point::new(1.0, 0.0)));
        assert!(close(turned.forward(Point::new(1.0, 0.0)), Point::new(1.0, 1.0)));
        assert_eq!(turned.apply_to_size(300, 200), (200, 300));
    }

    #[test]
    fn inverse_undoes_forward_for_all_eight_orientations() {
        let p = Point::new(0.2, 0.7);
        for orientation in all_orientations() {
            assert!(close(orientation.inverse(orientation.forward(p)), p), "{orientation:?}");
        }
    }

    #[test]
    fn four_quarter_turns_and_two_flips_return_to_the_start() {
        for start in all_orientations() {
            assert_eq!(start.rotated_cw().rotated_cw().rotated_cw().rotated_cw(), start);
            assert_eq!(start.rotated_cw().rotated_ccw(), start);
            assert_eq!(start.flipped_horizontal().flipped_horizontal(), start);
            assert_eq!(start.flipped_vertical().flipped_vertical(), start);
        }
    }

    #[test]
    fn flips_mirror_the_already_oriented_view() {
        let p = Point::new(0.2, 0.7);
        for start in all_orientations() {
            let seen = start.forward(p);
            let mirrored = start.flipped_horizontal().forward(p);
            assert!(close(mirrored, Point::new(1.0 - seen.x, seen.y)), "{start:?}");
            let upended = start.flipped_vertical().forward(p);
            assert!(close(upended, Point::new(seen.x, 1.0 - seen.y)), "{start:?}");
        }
    }

    #[test]
    fn rotations_compose_in_the_oriented_view() {
        let p = Point::new(0.2, 0.7);
        for start in all_orientations() {
            let seen = start.forward(p);
            let turned = start.rotated_cw().forward(p);
            assert!(close(turned, Point::new(1.0 - seen.y, seen.x)), "{start:?}");
        }
    }

    #[test]
    fn the_full_rectangle_fits_only_when_unrotated() {
        assert!(CropRect::FULL.fits(0.0, 1.5));
        assert!(!CropRect::FULL.fits(5.0, 1.5));
    }

    #[test]
    fn the_inscribed_rectangle_fits_and_touches_the_frame() {
        for angle in [-45.0, -12.5, -1.0, 0.0, 0.5, 10.0, 30.0, 45.0] {
            for frame_aspect in [0.5, 1.0, 1.5, 3.0] {
                for crop_aspect in [0.8, 1.0, 1.5, 16.0 / 9.0] {
                    let rect = CropRect::inscribed(angle, frame_aspect, crop_aspect);
                    assert!(rect.fits(angle, frame_aspect), "{angle} {frame_aspect} {crop_aspect}");
                    assert!((rect.aspect(frame_aspect) - crop_aspect).abs() < 1e-3);
                    let grown = CropRect {
                        width: rect.width * 1.01,
                        height: rect.height * 1.01,
                        ..rect
                    };
                    assert!(!grown.fits(angle, frame_aspect), "{angle} {frame_aspect} {crop_aspect}");
                }
            }
        }
    }

    #[test]
    fn an_unset_rectangle_resolves_to_the_full_frame_or_the_inscribed_one() {
        let straight = Crop::default();
        assert_eq!(straight.resolved(1.5), CropRect::FULL);
        assert!(straight.is_identity());

        let tilted = Crop {
            angle: 7.0,
            ..Default::default()
        };
        let rect = tilted.resolved(1.5);
        assert!(rect.width < 1.0 && rect.fits(7.0, 1.5));
        assert!(!tilted.is_identity());
    }
}
