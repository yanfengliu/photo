//! The crop: the last step from the transformed frame to the output.

use photo_core::develop::{Crop, CropRect};

use super::matrix::{Mat3, translate};

/// A [`CropRect`] with every field finite, its centre inside the frame, and
/// both sides at least [`CropRect::MIN_SIDE`] and at most the frame diagonal
/// (a rotated rectangle may be longer than the frame is wide, but never
/// longer than that). A rectangle with a non-finite field falls back to the
/// whole frame.
fn usable(rect: CropRect, frame_width: f64, frame_height: f64) -> CropRect {
    let finite = [rect.cx, rect.cy, rect.width, rect.height]
        .iter()
        .all(|v| v.is_finite());
    if !finite {
        return CropRect::FULL;
    }
    let diagonal = frame_width.hypot(frame_height);
    CropRect {
        cx: rect.cx.clamp(0.0, 1.0),
        cy: rect.cy.clamp(0.0, 1.0),
        width: rect
            .width
            .clamp(CropRect::MIN_SIDE, (diagonal / frame_width) as f32),
        height: rect
            .height
            .clamp(CropRect::MIN_SIDE, (diagonal / frame_height) as f32),
    }
}

/// The straighten angle the geometry actually uses: the setting, or 0 when
/// it is not finite.
pub(crate) fn usable_angle(angle: f32) -> f32 {
    if angle.is_finite() { angle } else { 0.0 }
}

/// The map from output pixels to transformed-frame pixels, and the output
/// size in whole pixels.
///
/// The output has one pixel per frame pixel. An upright crop is snapped to
/// the frame's pixel grid, so cropping alone never interpolates; a rotated
/// one turns about the rectangle's centre exactly as
/// [`CropRect::corners`] describes.
pub(crate) fn crop_matrix(frame_width: u32, frame_height: u32, crop: &Crop) -> (Mat3, (u32, u32)) {
    let (fw, fh) = (f64::from(frame_width), f64::from(frame_height));
    let angle = usable_angle(crop.angle);
    let resolved = Crop { angle, ..*crop }.resolved((fw / fh) as f32);
    let rect = usable(resolved, fw, fh);
    let width = (f64::from(rect.width) * fw).round().max(1.0);
    let height = (f64::from(rect.height) * fh).round().max(1.0);
    let (centre_x, centre_y) = (f64::from(rect.cx) * fw, f64::from(rect.cy) * fh);
    let matrix = if angle == 0.0 {
        translate(
            (centre_x - width / 2.0).round(),
            (centre_y - height / 2.0).round(),
        )
    } else {
        let (sin, cos) = f64::from(angle).to_radians().sin_cos();
        let (half_w, half_h) = (width / 2.0, height / 2.0);
        [
            [cos, -sin, centre_x - cos * half_w + sin * half_h],
            [sin, cos, centre_y - sin * half_w - cos * half_h],
            [0.0, 0.0, 1.0],
        ]
    };
    (matrix, (width as u32, height as u32))
}

/// Number of points checked along each side of a candidate rectangle.
const EDGE_SAMPLES: usize = 64;
/// Bisection steps: 2^-20 of the frame is far below a pixel.
const SEARCH_STEPS: usize = 20;

/// The largest rectangle centred in the frame, with pixel aspect ratio
/// `crop_aspect` and rotated by `angle_degrees`, whose whole boundary
/// satisfies `inside` (given positions in frame pixels). `None` when the
/// largest one that does has a side under [`CropRect::MIN_SIDE`]: a crop
/// cannot be that small, so using it would mean enlarging it into the area
/// it was meant to avoid.
///
/// Shrinks [`CropRect::inscribed`] about the frame centre by bisection. That
/// finds the true maximum whenever the acceptable region is convex or
/// star-shaped about the frame centre, which covers perspective alone and
/// distortion alone. For other combinations the result is still guaranteed
/// acceptable, but a slightly larger rectangle may exist.
pub(crate) fn largest_centred(
    frame_width: f64,
    frame_height: f64,
    angle_degrees: f32,
    crop_aspect: f32,
    inside: impl Fn(f64, f64) -> bool,
) -> Option<CropRect> {
    let frame_aspect = (frame_width / frame_height) as f32;
    let crop_aspect = if crop_aspect.is_finite() && crop_aspect > 0.0 {
        crop_aspect
    } else {
        frame_aspect
    };
    let angle = usable_angle(angle_degrees);
    let outer = CropRect::inscribed(angle, frame_aspect, crop_aspect);
    let shrunk = |factor: f32| CropRect {
        width: outer.width * factor,
        height: outer.height * factor,
        ..outer
    };
    let fits = |rect: &CropRect| {
        let corners = rect.corners(angle, frame_aspect);
        (0..4).all(|side| {
            let (from, to) = (corners[side], corners[(side + 1) % 4]);
            (0..EDGE_SAMPLES).all(|step| {
                let p = from.lerp(to, step as f32 / EDGE_SAMPLES as f32);
                inside(f64::from(p.x) * frame_width, f64::from(p.y) * frame_height)
            })
        })
    };
    let (mut low, mut high) = (0.0f32, 1.0f32);
    if fits(&outer) {
        low = 1.0;
    } else {
        for _ in 0..SEARCH_STEPS {
            let middle = (low + high) / 2.0;
            if fits(&shrunk(middle)) {
                low = middle;
            } else {
                high = middle;
            }
        }
    }
    let rect = shrunk(low);
    (rect.width >= CropRect::MIN_SIDE && rect.height >= CropRect::MIN_SIDE).then_some(rect)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::geometry::matrix::apply;

    #[test]
    fn the_default_crop_is_the_exact_identity() {
        let (matrix, size) = crop_matrix(6000, 4000, &Crop::default());
        assert_eq!(matrix, translate(0.0, 0.0));
        assert_eq!(size, (6000, 4000));
    }

    #[test]
    fn an_upright_crop_starts_on_a_whole_pixel() {
        let crop = Crop {
            rect: Some(CropRect {
                cx: 0.3333,
                cy: 0.6,
                width: 0.21,
                height: 0.37,
            }),
            ..Default::default()
        };
        let (matrix, (width, height)) = crop_matrix(1001, 667, &crop);
        assert_eq!((width, height), (210, 247));
        let [x, y, _] = apply(&matrix, 0.0, 0.0);
        assert_eq!((x.fract(), y.fract()), (0.0, 0.0));
        assert!((x + 105.0 - 0.3333 * 1001.0).abs() <= 0.5);
    }

    #[test]
    fn a_rotated_crop_turns_about_its_own_centre() {
        let rect = CropRect {
            cx: 0.4,
            cy: 0.5,
            width: 0.5,
            height: 0.5,
        };
        let crop = Crop {
            rect: Some(rect),
            angle: 30.0,
            ..Default::default()
        };
        let (matrix, (width, height)) = crop_matrix(800, 600, &crop);
        let [x, y, _] = apply(&matrix, f64::from(width) / 2.0, f64::from(height) / 2.0);
        assert!(
            (x - 320.0).abs() < 1e-4 && (y - 300.0).abs() < 1e-4,
            "({x}, {y})"
        );
    }

    #[test]
    fn unusable_rectangles_fall_back_or_are_clamped() {
        let nan = Crop {
            rect: Some(CropRect {
                cx: f32::NAN,
                ..CropRect::FULL
            }),
            ..Default::default()
        };
        assert_eq!(crop_matrix(300, 200, &nan).1, (300, 200));
        let tiny = Crop {
            rect: Some(CropRect {
                width: 0.0,
                height: -1.0,
                ..CropRect::FULL
            }),
            ..Default::default()
        };
        assert_eq!(crop_matrix(300, 200, &tiny).1, (3, 2));
        assert_eq!(crop_matrix(1, 1, &tiny).1, (1, 1));
    }

    #[test]
    fn the_search_finds_the_edge_of_a_circular_region() {
        // Acceptable region: a disc of radius 100 px at the centre of a
        // 400 x 400 frame. The largest centred square has a 100 px half-diagonal.
        let rect = largest_centred(400.0, 400.0, 0.0, 1.0, |x, y| {
            (x - 200.0).hypot(y - 200.0) <= 100.0
        })
        .unwrap();
        let side = rect.width * 400.0;
        assert!((side - 100.0 * 2f32.sqrt()).abs() < 0.05, "{side}");
        assert!(largest_centred(400.0, 400.0, 0.0, 1.0, |_, _| false).is_none());
    }
}
