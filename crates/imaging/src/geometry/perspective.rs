//! The manual perspective [`Transform`] as a homography.
//!
//! The oriented frame is treated as the image of a pinhole camera whose
//! principal point is the frame centre and whose focal length equals the
//! frame diagonal (a "normal" lens; the true focal length is not known here,
//! and a wrong guess only changes the aspect ratio of the result, which is
//! what the `aspect` slider is for). Tilting and rolling that camera maps the
//! image through `H = K R K^-1` (Hartley and Zisserman, *Multiple View
//! Geometry*, 2nd ed., section 8.4.2: the homography induced by a pure
//! rotation about the camera centre).
//!
//! `R = Rx(vertical) * Ry(horizontal) * Rz(rotate)`: the picture is first
//! rolled, then tilted. A camera rotation also pans the view and changes its
//! magnification, which a photographer does not expect from a keystone
//! slider, so the result is translated and uniformly scaled until the frame
//! centre maps to itself with an area magnification of exactly 1 there.
//! Aspect, scale and offsets are applied after that, in that order.

use photo_core::develop::Transform;

use super::matrix::{IDENTITY, Mat3, apply, chain, scale, translate};

/// Slider value at which a tilt reaches 45 degrees: the tilt angle is
/// `atan(value / 100)`, so the convergence it adds is linear in the slider.
const TILT_FULL_SCALE: f64 = 100.0;
/// `aspect = +100` stretches the height by `2^(1/2)` and squeezes the width
/// by the same factor.
const ASPECT_FULL_SCALE_OCTAVES: f64 = 0.5;
/// `x_offset = +100` moves the picture by this fraction of the frame width
/// (and likewise for `y_offset` and the height).
const OFFSET_FULL_SCALE: f64 = 0.5;
/// Smallest `scale` percentage honoured; anything lower would be degenerate.
const MIN_SCALE_PERCENT: f64 = 1.0;
/// Limits far outside every slider's range, applied only so that no finite
/// setting can overflow the matrix: `scale` as a percentage, `aspect`,
/// and the offsets.
const MAX_SCALE_PERCENT: f64 = 1.0e5;
const MAX_ASPECT: f64 = 1000.0;
const MAX_OFFSET: f64 = 1.0e5;

fn finite_or(value: f32, fallback: f64) -> f64 {
    if value.is_finite() {
        f64::from(value)
    } else {
        fallback
    }
}

/// Rotation about the x axis (pointing right) by `angle`.
fn rotation_x(angle: f64) -> Mat3 {
    let (sin, cos) = angle.sin_cos();
    [[1.0, 0.0, 0.0], [0.0, cos, -sin], [0.0, sin, cos]]
}

/// Rotation about the y axis (pointing down) by `angle`.
fn rotation_y(angle: f64) -> Mat3 {
    let (sin, cos) = angle.sin_cos();
    [[cos, 0.0, -sin], [0.0, 1.0, 0.0], [sin, 0.0, cos]]
}

/// Rotation about the optical axis by `angle`; positive is clockwise on
/// screen because y points down.
fn rotation_z(angle: f64) -> Mat3 {
    let (sin, cos) = angle.sin_cos();
    [[cos, -sin, 0.0], [sin, cos, 0.0], [0.0, 0.0, 1.0]]
}

/// The similarity that undoes the pan and zoom of `h` at the origin: after
/// it the origin maps to itself and the Jacobian there has determinant 1.
fn recentre(h: &Mat3) -> Mat3 {
    let [x, y, w] = apply(h, 0.0, 0.0);
    let (x, y) = (x / w, y / w);
    // Jacobian of the projective map at the origin.
    let a = (h[0][0] - x * h[2][0]) / w;
    let b = (h[0][1] - x * h[2][1]) / w;
    let c = (h[1][0] - y * h[2][0]) / w;
    let d = (h[1][1] - y * h[2][1]) / w;
    let area = (a * d - b * c).abs();
    let zoom = if area.is_finite() && area > 0.0 {
        1.0 / area.sqrt()
    } else {
        1.0
    };
    chain(&[scale(zoom, zoom), translate(-x, -y)])
}

/// Forward map from the oriented frame to the transformed frame, both in
/// pixels of a `frame_width` x `frame_height` frame. Exactly the identity
/// for a default [`Transform`].
pub(crate) fn transform_matrix(frame_width: f64, frame_height: f64, transform: &Transform) -> Mat3 {
    if transform.is_identity() {
        return IDENTITY;
    }
    // Beyond full scale a tilt would bring the horizon into the frame.
    let vertical = finite_or(transform.vertical, 0.0).clamp(-TILT_FULL_SCALE, TILT_FULL_SCALE);
    let horizontal = finite_or(transform.horizontal, 0.0).clamp(-TILT_FULL_SCALE, TILT_FULL_SCALE);
    // Whole turns change nothing, and removing them keeps the angle exact.
    let rotate = finite_or(transform.rotate, 0.0) % 360.0;
    let aspect = finite_or(transform.aspect, 0.0).clamp(-MAX_ASPECT, MAX_ASPECT);
    let zoom =
        finite_or(transform.scale, 100.0).clamp(MIN_SCALE_PERCENT, MAX_SCALE_PERCENT) / 100.0;
    let x_offset = finite_or(transform.x_offset, 0.0).clamp(-MAX_OFFSET, MAX_OFFSET);
    let y_offset = finite_or(transform.y_offset, 0.0).clamp(-MAX_OFFSET, MAX_OFFSET);

    let (centre_x, centre_y) = (frame_width / 2.0, frame_height / 2.0);
    let focal = frame_width.hypot(frame_height);
    // Negative `vertical` magnifies the top of the picture, negative
    // `horizontal` its left side: see the sign table on `Geometry`.
    let rotation = chain(&[
        rotation_x(-(vertical / TILT_FULL_SCALE).atan()),
        rotation_y(-(horizontal / TILT_FULL_SCALE).atan()),
        rotation_z(rotate.to_radians()),
    ]);
    let camera = chain(&[
        scale(focal, focal),
        rotation,
        scale(1.0 / focal, 1.0 / focal),
    ]);
    let stretch = (aspect / 100.0 * ASPECT_FULL_SCALE_OCTAVES).exp2();
    chain(&[
        translate(centre_x, centre_y),
        translate(
            x_offset / 100.0 * OFFSET_FULL_SCALE * frame_width,
            y_offset / 100.0 * OFFSET_FULL_SCALE * frame_height,
        ),
        scale(zoom, zoom),
        scale(1.0 / stretch, stretch),
        recentre(&camera),
        camera,
        translate(-centre_x, -centre_y),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project(m: &Mat3, x: f64, y: f64) -> (f64, f64) {
        let [px, py, w] = apply(m, x, y);
        (px / w, py / w)
    }

    #[test]
    fn the_default_transform_is_exactly_the_identity() {
        assert_eq!(
            transform_matrix(6000.0, 4000.0, &Transform::default()),
            IDENTITY
        );
    }

    #[test]
    fn a_pure_tilt_keeps_the_centre_and_the_axis_it_tilts_about() {
        let transform = Transform {
            vertical: -80.0,
            ..Default::default()
        };
        let m = transform_matrix(600.0, 400.0, &transform);
        let (cx, cy) = project(&m, 300.0, 200.0);
        assert!((cx - 300.0).abs() < 1e-9 && (cy - 200.0).abs() < 1e-9);
        // Points on the horizontal line through the centre stay on it.
        let (_, y) = project(&m, 520.0, 200.0);
        assert!((y - 200.0).abs() < 1e-9);
        // Vertical lines stay symmetric about the centre column.
        let (left, _) = project(&m, 100.0, 50.0);
        let (right, _) = project(&m, 500.0, 50.0);
        assert!((left + right - 600.0).abs() < 1e-9);
    }

    #[test]
    fn the_horizon_stays_outside_the_frame_at_full_tilt() {
        for (vertical, horizontal) in [(100.0, 0.0), (-100.0, 0.0), (0.0, 100.0), (100.0, -100.0)] {
            let transform = Transform {
                vertical,
                horizontal,
                ..Default::default()
            };
            for (width, height) in [(600.0, 400.0), (400.0, 600.0), (500.0, 500.0)] {
                let m = transform_matrix(width, height, &transform);
                let centre = apply(&m, width / 2.0, height / 2.0)[2];
                for (x, y) in [(0.0, 0.0), (width, 0.0), (width, height), (0.0, height)] {
                    let w = apply(&m, x, y)[2];
                    assert!(
                        w / centre > 0.05,
                        "{vertical}/{horizontal} at ({x}, {y}): {w}"
                    );
                }
            }
        }
    }

    #[test]
    fn non_finite_sliders_fall_back_to_neutral() {
        let broken = Transform {
            vertical: f32::NAN,
            scale: f32::INFINITY,
            rotate: 3.0,
            ..Default::default()
        };
        let clean = Transform {
            rotate: 3.0,
            ..Default::default()
        };
        assert_eq!(
            transform_matrix(300.0, 200.0, &broken),
            transform_matrix(300.0, 200.0, &clean)
        );
    }
}
