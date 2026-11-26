//! The mask shapes' handles, on a 1200 x 800 photo fitted at half size.

use photo_core::develop::DevelopSettings;
use photo_engine::Geometry;

use super::*;
use crate::app::develop::canvas::map::Display;
use crate::app::develop::canvas::mapping::{CanvasZoom, Placement};

/// A 1200 x 800 photo fitted at half size, one pixel per point.
fn map() -> CanvasMap {
    let geometry = Geometry::from_settings(1200, 800, &DevelopSettings::default());
    let canvas = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(632.0, 432.0));
    let placement = Placement::new(CanvasZoom::Fit, vec2(0.5, 0.5), vec2(1200.0, 800.0), canvas);
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
