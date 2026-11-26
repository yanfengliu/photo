//! The handles of the gradients. A linear gradient is three lines (where
//! the effect is full, its middle, and where it ends) with a handle at each
//! end, a pin in the middle that moves it and a handle that turns it. A
//! radial gradient is its ellipse with a handle at the end of each axis
//! (its size and roundness), one that sizes both together and one that
//! turns it, around the pin at its centre.
//!
//! Shapes are stored in normalized source coordinates with lengths as
//! fractions of the longer side; the handles are worked out in source
//! pixels, where a circle is a circle, and drawn through the photo's
//! geometry.

use egui::epaint::Shape;
use egui::{Color32, Painter, Pos2, Stroke, Vec2, pos2, vec2};
use photo_core::develop::{MaskShape, Spot};
use photo_core::geom::Point;

use crate::app::develop::canvas::map::{CanvasMap, turned};
use crate::theme::palette;

/// How near a handle the pointer takes it, in points.
const REACH: f32 = 10.0;
/// How far from the middle the handle that turns a gradient sits, in
/// points.
const TURN_ARM: f32 = 64.0;

/// A handle of a gradient.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Handle {
    /// The pin: moves the whole shape.
    Move,
    /// Where a linear gradient's effect is full.
    LinearStart,
    /// Where a linear gradient's effect ends.
    LinearEnd,
    /// Turns a linear gradient about its middle.
    LinearTurn,
    /// An end of the ellipse's own x axis: its width.
    RadialX(bool),
    /// An end of the ellipse's own y axis: its height.
    RadialY(bool),
    /// Sizes both axes together.
    RadialSize,
    /// Turns the ellipse about its centre.
    RadialTurn,
}

fn v(point: Point) -> Vec2 {
    vec2(point.x, point.y)
}

fn p(vector: Vec2) -> Point {
    Point::new(vector.x, vector.y)
}

/// The middle of a linear gradient, the vector from start to end, and the
/// unit vector along its lines, all in source pixels.
fn linear_parts(map: &CanvasMap, start: Point, end: Point) -> (Vec2, Vec2, Vec2) {
    let (from, to) = (v(map.pixels(start)), v(map.pixels(end)));
    let along = to - from;
    let across = if along.length() > 1.0e-3 {
        vec2(-along.y, along.x).normalized()
    } else {
        vec2(1.0, 0.0)
    };
    ((from + to) / 2.0, along, across)
}

/// How long `points` of the screen are in source pixels at a source
/// position.
fn reach(map: &CanvasMap, at: Vec2, points: f32) -> f32 {
    map.source_to_screen(p(at))
        .map_or(points, |screen| map.source_per_point(screen) * points)
}

/// Where the handles of `shape` are, in source pixels. A brush has none.
pub(crate) fn handles(shape: &MaskShape, map: &CanvasMap) -> Vec<(Handle, Point)> {
    match shape {
        MaskShape::Linear { start, end } => {
            let (middle, _, across) = linear_parts(map, *start, *end);
            let arm = reach(map, middle, TURN_ARM);
            vec![
                (Handle::Move, p(middle)),
                (Handle::LinearStart, map.pixels(*start)),
                (Handle::LinearEnd, map.pixels(*end)),
                (Handle::LinearTurn, p(middle + across * arm)),
            ]
        }
        MaskShape::Radial {
            center,
            radius_x,
            radius_y,
            angle,
            ..
        } => {
            let long = map.long_edge();
            let middle = v(map.pixels(*center));
            let (x, y) = (
                turned(vec2(radius_x * long, 0.0), *angle),
                turned(vec2(0.0, radius_y * long), *angle),
            );
            let arm = reach(map, middle + x, TURN_ARM / 2.0);
            let diagonal = std::f32::consts::FRAC_1_SQRT_2;
            vec![
                (Handle::Move, p(middle)),
                (Handle::RadialX(true), p(middle + x)),
                (Handle::RadialX(false), p(middle - x)),
                (Handle::RadialY(true), p(middle + y)),
                (Handle::RadialY(false), p(middle - y)),
                (Handle::RadialSize, p(middle + (x + y) * diagonal)),
                (Handle::RadialTurn, p(middle + x + x.normalized() * arm)),
            ]
        }
        MaskShape::Brush { .. } => Vec::new(),
    }
}

/// The handle of `shape` under a screen position (points), with where it
/// is in source pixels.
pub(crate) fn hit(shape: &MaskShape, map: &CanvasMap, at: Pos2) -> Option<(Handle, Point)> {
    handles(shape, map)
        .into_iter()
        .filter_map(|(handle, source)| {
            let distance = map.source_to_screen(source)?.distance(at);
            (distance <= REACH).then_some((distance, handle, source))
        })
        .min_by(|a, b| a.0.total_cmp(&b.0))
        .map(|(_, handle, source)| (handle, source))
}

/// Where a shape's pin is (normalized source): the middle of a gradient,
/// the first point painted of a brush.
pub(crate) fn pin(shape: &MaskShape) -> Option<Point> {
    match shape {
        MaskShape::Linear { start, end } => Some(start.lerp(*end, 0.5)),
        MaskShape::Radial { center, .. } => Some(*center),
        MaskShape::Brush { strokes } => strokes
            .iter()
            .find_map(|stroke| stroke.points.first().copied()),
    }
}

/// `start` with `handle` taken to `to` (source pixels).
pub(crate) fn moved(start: &MaskShape, handle: Handle, to: Point, map: &CanvasMap) -> MaskShape {
    let long = map.long_edge();
    let to = v(to);
    match start {
        MaskShape::Linear { start: from, end } => {
            let (middle, along, _) = linear_parts(map, *from, *end);
            let (from, end) = match handle {
                Handle::LinearStart => (to, v(map.pixels(*end))),
                Handle::LinearEnd => (v(map.pixels(*from)), to),
                Handle::LinearTurn if (to - middle).length() > 1.0e-3 => {
                    // The handle sits a quarter turn on from the gradient's
                    // direction.
                    let across = (to - middle).normalized();
                    let half = vec2(across.y, -across.x) * along.length() / 2.0;
                    (middle - half, middle + half)
                }
                Handle::Move => (to - along / 2.0, to + along / 2.0),
                _ => return start.clone(),
            };
            MaskShape::Linear {
                start: map.normalized(p(from)),
                end: map.normalized(p(end)),
            }
        }
        MaskShape::Radial {
            center,
            radius_x,
            radius_y,
            angle,
            feather,
            invert,
        } => {
            let middle = v(map.pixels(*center));
            let from_middle = to - middle;
            let own = turned(from_middle, -*angle);
            let hold = |radius: f32| radius.clamp(Spot::MIN_RADIUS, MaskShape::MAX_RADIAL_RADIUS);
            let (mut center, mut x, mut y, mut angle) = (*center, *radius_x, *radius_y, *angle);
            match handle {
                Handle::Move => center = map.normalized(p(to)),
                Handle::RadialX(_) => x = hold(own.x.abs() / long),
                Handle::RadialY(_) => y = hold(own.y.abs() / long),
                Handle::RadialSize => {
                    let was = vec2(x * long, y * long).length() * std::f32::consts::FRAC_1_SQRT_2;
                    let scale = from_middle.length() / was.max(1.0e-3);
                    (x, y) = (hold(x * scale), hold(y * scale));
                }
                Handle::RadialTurn if from_middle.length() > 1.0e-3 => {
                    angle = from_middle.y.atan2(from_middle.x).to_degrees();
                }
                _ => {}
            }
            MaskShape::Radial {
                center,
                radius_x: x,
                radius_y: y,
                angle,
                feather: *feather,
                invert: *invert,
            }
        }
        MaskShape::Brush { .. } => start.clone(),
    }
}

/// The radial gradient pulled out from where the pointer was `pressed` to
/// where it is now (both in points): its centre under the press, its axes
/// along the screen's, reaching as far as the pointer went each way.
pub(crate) fn pulled_radial(
    map: &CanvasMap,
    pressed: Pos2,
    pointer: Pos2,
    feather: f32,
) -> MaskShape {
    let long = map.long_edge();
    let center = v(map.screen_to_source(pressed));
    let across = v(map.screen_to_source(pos2(pointer.x, pressed.y))) - center;
    let down = v(map.screen_to_source(pos2(pressed.x, pointer.y))) - center;
    let unit = v(map.screen_to_source(pressed + vec2(1.0, 0.0))) - center;
    let hold = |length: f32| (length / long).clamp(Spot::MIN_RADIUS, MaskShape::MAX_RADIAL_RADIUS);
    MaskShape::Radial {
        center: map.normalized(p(center)),
        radius_x: hold(across.length()),
        radius_y: hold(down.length()),
        angle: unit.y.atan2(unit.x).to_degrees(),
        feather,
        invert: false,
    }
}

/// A line that reads on any photo.
fn line(painter: &Painter, ends: [Pos2; 2], color: Color32, width: f32) {
    painter.line_segment(
        ends,
        Stroke::new(width + 2.0, Color32::from_black_alpha(120)),
    );
    painter.line_segment(ends, Stroke::new(width, color));
}

/// A square handle.
fn grip(painter: &Painter, at: Pos2, color: Color32) {
    let square = egui::Rect::from_center_size(at, Vec2::splat(8.0));
    painter.rect_filled(square.expand(1.0), 1.0, Color32::from_black_alpha(160));
    painter.rect_filled(square, 1.0, color);
}

/// A round handle.
fn knob(painter: &Painter, at: Pos2, color: Color32) {
    painter.circle_filled(at, 5.5, Color32::from_black_alpha(160));
    painter.circle_filled(at, 4.0, color);
}

/// A pin: where a mask is, and what a click selects.
pub(crate) fn paint_pin(painter: &Painter, at: Pos2, selected: bool) {
    painter.circle_filled(at, 7.5, Color32::from_black_alpha(160));
    let fill = if selected {
        palette::ACCENT_STRONG
    } else {
        palette::TEXT_DIM
    };
    painter.circle_filled(at, 6.0, palette::TEXT_STRONG);
    painter.circle_filled(at, 4.0, fill);
}

/// Paint a shape's outline, and its handles when it is the one selected.
pub(crate) fn paint(painter: &Painter, shape: &MaskShape, map: &CanvasMap, selected: bool) {
    let color = if selected {
        palette::TEXT_STRONG
    } else {
        palette::TEXT_DIM
    };
    match shape {
        MaskShape::Linear { start, end } => {
            let (middle, along, across) = linear_parts(map, *start, *end);
            let far = map.long_edge() * 3.0;
            for (offset, width) in [(-0.5, 1.5), (0.0, 1.0), (0.5, 1.5)] {
                let on = middle + along * offset;
                let ends = [
                    map.source_to_screen(p(on - across * far)),
                    map.source_to_screen(p(on + across * far)),
                ];
                if let [Some(from), Some(to)] = ends {
                    let tone = if offset == 0.0 {
                        palette::TEXT_DIM
                    } else {
                        color
                    };
                    line(painter, [from, to], tone, width);
                }
            }
        }
        MaskShape::Radial {
            center,
            radius_x,
            radius_y,
            angle,
            feather,
            ..
        } => {
            let long = map.long_edge();
            let radii = vec2(radius_x * long, radius_y * long);
            let middle = map.pixels(*center);
            let outer = map.ellipse(middle, radii, *angle);
            let shadow = Stroke::new(3.5, Color32::from_black_alpha(120));
            painter.add(Shape::closed_line(outer.clone(), shadow));
            painter.add(Shape::closed_line(outer, Stroke::new(1.5, color)));
            let inner = map.ellipse(middle, radii * (1.0 - feather / 100.0), *angle);
            if selected && *feather > 0.0 && *feather < 100.0 {
                painter.add(Shape::closed_line(
                    inner,
                    Stroke::new(1.0, Color32::from_white_alpha(90)),
                ));
            }
        }
        MaskShape::Brush { .. } => {}
    }
    if !selected {
        return;
    }
    for (handle, source) in handles(shape, map) {
        let Some(at) = map.source_to_screen(source) else {
            continue;
        };
        match handle {
            Handle::Move => {}
            Handle::LinearTurn | Handle::RadialTurn => knob(painter, at, palette::ACCENT_STRONG),
            Handle::RadialSize => knob(painter, at, palette::TEXT_STRONG),
            _ => grip(painter, at, palette::TEXT_STRONG),
        }
    }
}

#[cfg(test)]
mod tests {
    use photo_core::develop::DevelopSettings;
    use photo_engine::Geometry;

    use super::*;
    use crate::app::develop::canvas::map::Display;
    use crate::app::develop::canvas::mapping::{CanvasZoom, Placement};

    /// A 1200 x 800 photo fitted at half size, one pixel per point.
    fn map() -> CanvasMap {
        let geometry = Geometry::from_settings(1200, 800, &DevelopSettings::default());
        let canvas = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(632.0, 432.0));
        let placement =
            Placement::new(CanvasZoom::Fit, vec2(0.5, 0.5), vec2(1200.0, 800.0), canvas);
        CanvasMap {
            ppp: 1.0,
            geometry,
            display: Display::Output(placement),
        }
    }

    fn radial() -> MaskShape {
        MaskShape::Radial {
            center: Point::new(0.5, 0.5),
            radius_x: 0.2,
            radius_y: 0.1,
            angle: 0.0,
            feather: 50.0,
            invert: false,
        }
    }

    fn at(shape: &MaskShape, handle: Handle, map: &CanvasMap) -> Point {
        handles(shape, map)
            .into_iter()
            .find(|(found, _)| *found == handle)
            .unwrap()
            .1
    }

    #[test]
    fn a_linear_gradient_is_moved_by_its_pin_and_stretched_by_its_ends() {
        let map = map();
        let shape = MaskShape::Linear {
            start: Point::new(0.5, 0.25),
            end: Point::new(0.5, 0.75),
        };
        assert_eq!(pin(&shape), Some(Point::new(0.5, 0.5)));
        let moved_shape = moved(&shape, Handle::Move, Point::new(300.0, 400.0), &map);
        assert_eq!(
            moved_shape,
            MaskShape::Linear {
                start: Point::new(0.25, 0.25),
                end: Point::new(0.25, 0.75),
            }
        );
        let stretched = moved(&shape, Handle::LinearEnd, Point::new(600.0, 720.0), &map);
        let MaskShape::Linear { start, end } = stretched else {
            panic!("linear");
        };
        assert_eq!((start, end.y), (Point::new(0.5, 0.25), 0.9));
    }

    #[test]
    fn turning_a_linear_gradient_keeps_its_middle_and_length() {
        let map = map();
        let shape = MaskShape::Linear {
            start: Point::new(0.5, 0.25),
            end: Point::new(0.5, 0.75),
        };
        let turn = at(&shape, Handle::LinearTurn, &map);
        // The handle is beside the middle, on the middle line.
        assert!((turn.y - 400.0).abs() < 1.0e-3 && (turn.x - 600.0).abs() > 50.0);
        // Take it straight below the middle: the gradient lies down.
        let turned_shape = moved(&shape, Handle::LinearTurn, Point::new(600.0, 600.0), &map);
        let MaskShape::Linear { start, end } = turned_shape else {
            panic!("linear");
        };
        let (from, to) = (map.pixels(start), map.pixels(end));
        assert!((from.y - 400.0).abs() < 1.0e-2 && (to.y - 400.0).abs() < 1.0e-2);
        assert!(
            (from.distance(to) - 400.0).abs() < 1.0e-2,
            "the length is kept"
        );
        // And the handle is where the pointer took it.
        let back = at(&turned_shape, Handle::LinearTurn, &map);
        assert!((back.x - 600.0).abs() < 1.0e-2 && back.y > 400.0);
    }

    #[test]
    fn a_radial_gradients_handles_size_round_and_turn_it() {
        let map = map();
        let shape = radial();
        let right = at(&shape, Handle::RadialX(true), &map);
        assert_eq!((right.x, right.y), (840.0, 400.0));
        let wider = moved(
            &shape,
            Handle::RadialX(true),
            Point::new(900.0, 380.0),
            &map,
        );
        let MaskShape::Radial {
            radius_x, radius_y, ..
        } = wider
        else {
            panic!("radial");
        };
        assert_eq!((radius_x, radius_y), (0.25, 0.1), "only its own axis");

        let corner = at(&shape, Handle::RadialSize, &map);
        let twice = Point::new(
            600.0 + 2.0 * (corner.x - 600.0),
            400.0 + 2.0 * (corner.y - 400.0),
        );
        let MaskShape::Radial {
            radius_x, radius_y, ..
        } = moved(&shape, Handle::RadialSize, twice, &map)
        else {
            panic!("radial");
        };
        assert!((radius_x - 0.4).abs() < 1.0e-4 && (radius_y - 0.2).abs() < 1.0e-4);

        let MaskShape::Radial { angle, .. } =
            moved(&shape, Handle::RadialTurn, Point::new(600.0, 700.0), &map)
        else {
            panic!("radial");
        };
        assert!((angle - 90.0).abs() < 1.0e-3);
    }

    #[test]
    fn the_nearest_handle_within_reach_is_taken() {
        let map = map();
        let shape = radial();
        let centre = map.source_to_screen(Point::new(600.0, 400.0)).unwrap();
        assert_eq!(
            hit(&shape, &map, centre).map(|found| found.0),
            Some(Handle::Move)
        );
        let right = map.source_to_screen(Point::new(840.0, 400.0)).unwrap();
        assert_eq!(
            hit(&shape, &map, right + vec2(3.0, -2.0)).map(|found| found.0),
            Some(Handle::RadialX(true))
        );
        assert_eq!(hit(&shape, &map, centre + vec2(40.0, 30.0)), None);
        assert!(
            handles(
                &MaskShape::Brush {
                    strokes: Vec::new()
                },
                &map
            )
            .is_empty()
        );
    }

    #[test]
    fn a_radial_gradient_is_pulled_out_from_its_centre_along_the_screen() {
        let map = map();
        let pressed = map.source_to_screen(Point::new(600.0, 400.0)).unwrap();
        let shape = pulled_radial(&map, pressed, pressed + vec2(120.0, -60.0), 50.0);
        let MaskShape::Radial {
            center,
            radius_x,
            radius_y,
            angle,
            ..
        } = shape
        else {
            panic!("radial");
        };
        assert_eq!(center, Point::new(0.5, 0.5));
        // Half size on screen: 120 points are 240 pixels of 1200.
        assert!((radius_x - 0.2).abs() < 1.0e-4 && (radius_y - 0.1).abs() < 1.0e-4);
        assert!(angle.abs() < 1.0e-3);
    }
}
