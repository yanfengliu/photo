//! The crop rectangle, between the form the settings keep it in and the
//! form a sidecar does. The conventions are set out in the crate
//! documentation, under "Orientation and crop".

use photo_core::develop::{Crop, CropConstraint, CropRect, Orientation};

use super::StoredFrame;
use super::orientation::inverse;
use crate::rdf::{CRS, Form, Packet, Values};
use crate::values::{format_bool, format_number, parse_bool, parse_number};

/// The properties holding the top-left and the bottom-right corner, in the
/// order of [`Edges`].
const CORNERS: [&str; 4] = ["CropLeft", "CropTop", "CropRight", "CropBottom"];
const ANGLE: &str = "CropAngle";
const SWITCH: &str = "HasCrop";

/// Two opposite corners as `[left, top, right, bottom]`, in fractions of a
/// frame: the edges of a rectangle that is not rotated, or the top-left and
/// bottom-right corners of one that is.
///
/// Everything here is computed in `f64`. The values come from `f32` and go
/// back to `f32`, and the rounding of the arithmetic in between is some
/// hundred million times finer than the spacing of `f32` fractions, so a
/// rectangle taken to a sidecar and back is the very same rectangle.
type Edges = [f64; 4];

/// The edges of `rect` before it is rotated.
fn edges_of(rect: &CropRect) -> Edges {
    let [cx, cy, width, height] = [rect.cx, rect.cy, rect.width, rect.height].map(f64::from);
    let (half_width, half_height) = (width / 2.0, height / 2.0);
    [
        cx - half_width,
        cy - half_height,
        cx + half_width,
        cy + half_height,
    ]
}

fn rect_of([left, top, right, bottom]: Edges) -> CropRect {
    CropRect {
        cx: ((left + right) / 2.0) as f32,
        cy: ((top + bottom) / 2.0) as f32,
        width: (right - left) as f32,
        height: (bottom - top) as f32,
    }
}

/// Carries a rectangle into the frame `orientation` produces: mirrored left
/// to right first, then turned clockwise, exactly as
/// [`Orientation::forward`] moves a point.
fn oriented(edges: Edges, orientation: Orientation) -> Edges {
    let [mut left, mut top, mut right, mut bottom] = edges;
    if orientation.flip_horizontal {
        (left, right) = (1.0 - right, 1.0 - left);
    }
    for _ in 0..orientation.quarter_turns % 4 {
        (left, top, right, bottom) = (1.0 - bottom, left, 1.0 - top, right);
    }
    [left, top, right, bottom]
}

/// Turns the diagonal of a rectangle about its centre by `degrees`,
/// clockwise on screen, in a frame `aspect` times as wide as it is high.
///
/// Applied to the edges of a rectangle before rotation it gives the corners
/// that rectangle has once rotated; applied to those corners with the angle
/// negated it gives the edges back.
fn turned([left, top, right, bottom]: Edges, degrees: f64, aspect: f64) -> Edges {
    if degrees == 0.0 {
        return [left, top, right, bottom];
    }
    let (sin, cos) = degrees.to_radians().sin_cos();
    let (cx, cy) = ((left + right) / 2.0, (top + bottom) / 2.0);
    // Half the diagonal, with x in units of the frame's height so that both
    // axes share a scale and a turn is a turn.
    let (x, y) = ((right - left) / 2.0 * aspect, (bottom - top) / 2.0);
    let (x, y) = ((x * cos - y * sin) / aspect, x * sin + y * cos);
    [cx - x, cy - y, cx + x, cy + y]
}

/// A straighten angle carried between the upright and the stored frame.
/// Turning the frame leaves it alone; mirroring reverses it, in either
/// direction.
fn angle_across(angle: f32, to_upright: Orientation) -> f32 {
    if to_upright.flip_horizontal {
        -angle
    } else {
        angle
    }
}

/// The crop the Camera Raw properties describe, or `None` when the sidecar
/// says nothing about cropping.
pub(super) fn read(crs: &Values, frame: StoredFrame) -> Option<Crop> {
    let corners = CORNERS.map(|name| crs.get(name).and_then(|text| parse_number::<f64>(text)));
    let angle = crs.get(ANGLE).and_then(|text| parse_number::<f32>(text));
    let switch = crs.get(SWITCH).and_then(|text| parse_bool(text));
    if switch.is_none() && angle.is_none() && corners.iter().all(Option::is_none) {
        return None;
    }
    if switch == Some(false) {
        // Whatever numbers are left over from an earlier crop do not count.
        return Some(Crop::default());
    }
    let angle = angle.unwrap_or(0.0);
    let rect = match corners {
        [Some(left), Some(top), Some(right), Some(bottom)] => {
            let edges = turned([left, top, right, bottom], -f64::from(angle), frame.aspect);
            let encloses_something = edges[2] > edges[0] && edges[3] > edges[1];
            encloses_something.then(|| rect_of(oriented(edges, frame.to_upright)))
        }
        _ => None,
    };
    Some(Crop {
        rect,
        angle: angle_across(angle, frame.to_upright),
        constraint: CropConstraint::default(),
    })
}

/// Writes the crop, unless the properties already describe it.
pub(super) fn write(packet: &mut Packet, crs: &Values, crop: &Crop, frame: StoredFrame) {
    // Numbers that are not finite cannot be written; they count as unset.
    let angle = if crop.angle.is_finite() {
        crop.angle
    } else {
        0.0
    };
    let stored_angle = angle_across(angle, frame.to_upright);
    let corners = crop
        .rect
        .map(|rect| oriented(edges_of(&rect), inverse(frame.to_upright)))
        .map(|edges| turned(edges, f64::from(stored_angle), frame.aspect))
        .filter(|corners| corners.iter().all(|corner| corner.is_finite()));
    let wanted = Crop {
        rect: crop.rect.filter(|_| corners.is_some()),
        angle,
        constraint: CropConstraint::default(),
    };
    if crs.contains_key(SWITCH) && read(crs, frame) == Some(wanted) {
        return;
    }
    let cropped = corners.is_some() || angle != 0.0;
    packet.set_text(CRS.property(SWITCH), format_bool(cropped), Form::Attribute);
    let angle = format_number(stored_angle, true).unwrap_or_default();
    packet.set_text(CRS.property(ANGLE), &angle, Form::Attribute);
    for (index, name) in CORNERS.into_iter().enumerate() {
        match corners.and_then(|corners| format_number(corners[index], false)) {
            Some(corner) => packet.set_text(CRS.property(name), &corner, Form::Attribute),
            None => packet.remove(CRS.property(name)),
        }
    }
}

#[cfg(test)]
mod tests {
    use photo_core::geom::Point;

    use super::*;

    const RECT: CropRect = CropRect {
        cx: 0.45,
        cy: 0.5,
        width: 0.4,
        height: 0.3,
    };

    fn all_orientations() -> impl Iterator<Item = Orientation> {
        (0..4).flat_map(|turns| [false, true].map(|flip| Orientation::new(turns, flip)))
    }

    fn close(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-5 && (a.y - b.y).abs() < 1e-5
    }

    fn point(x: f64, y: f64) -> Point {
        Point::new(x as f32, y as f32)
    }

    #[test]
    fn an_unrotated_rectangle_is_carried_like_its_corners() {
        let [left, top, right, bottom] = edges_of(&RECT);
        for orientation in all_orientations() {
            let [l, t, r, b] = oriented([left, top, right, bottom], orientation);
            assert!(l < r && t < b, "{orientation:?}");
            let moved = [(l, t), (r, t), (r, b), (l, b)].map(|(x, y)| point(x, y));
            for (x, y) in [(left, top), (right, top), (right, bottom), (left, bottom)] {
                let corner = orientation.forward(point(x, y));
                assert!(
                    moved.iter().any(|candidate| close(*candidate, corner)),
                    "{orientation:?}"
                );
            }
        }
    }

    #[test]
    fn a_rotated_rectangle_is_carried_like_its_corners_too() {
        // The rectangle the sidecar describes and the one the settings
        // describe must be the same shape on the picture, corner for corner,
        // whatever the orientation between their frames.
        let (stored_angle, stored_aspect) = (12.0_f32, 1.5_f32);
        for to_upright in all_orientations() {
            let upright = rect_of(oriented(edges_of(&RECT), to_upright));
            let upright_angle = angle_across(stored_angle, to_upright);
            let upright_aspect = if to_upright.swaps_axes() {
                1.0 / stored_aspect
            } else {
                stored_aspect
            };
            let expected = RECT.corners(stored_angle, stored_aspect);
            let actual = upright.corners(upright_angle, upright_aspect);
            for corner in expected.map(|corner| to_upright.forward(corner)) {
                assert!(
                    actual.iter().any(|candidate| close(*candidate, corner)),
                    "{to_upright:?}: {corner:?} is not a corner of {actual:?}"
                );
            }
        }
    }

    #[test]
    fn turning_the_edges_gives_the_corners_the_settings_describe() {
        for angle in [-45.0_f32, -7.5, 0.0, 3.25, 30.0, 45.0] {
            for aspect in [0.5_f32, 1.0, 1.5, 3.0] {
                let [left, top, right, bottom] =
                    turned(edges_of(&RECT), f64::from(angle), f64::from(aspect));
                let [top_left, _, bottom_right, _] = RECT.corners(angle, aspect);
                assert!(close(point(left, top), top_left), "{angle} {aspect}");
                assert!(
                    close(point(right, bottom), bottom_right),
                    "{angle} {aspect}"
                );
            }
        }
    }

    #[test]
    fn turning_back_restores_the_rectangle_exactly() {
        let mut state = 0x1234_5678_u32;
        let mut unit = move || {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (state >> 8) as f32 / 16_777_216.0
        };
        for _ in 0..2_000 {
            let width = CropRect::MIN_SIDE + unit() * 0.9;
            let height = CropRect::MIN_SIDE + unit() * 0.9;
            let rect = CropRect {
                cx: width / 2.0 + unit() * (1.0 - width),
                cy: height / 2.0 + unit() * (1.0 - height),
                width,
                height,
            };
            let degrees = f64::from((unit() - 0.5) * 90.0);
            let aspect = f64::from(0.4 + unit() * 3.0);
            let corners = turned(edges_of(&rect), degrees, aspect);
            assert_eq!(
                rect_of(turned(corners, -degrees, aspect)),
                rect,
                "{degrees} {aspect}"
            );
        }
    }
}
