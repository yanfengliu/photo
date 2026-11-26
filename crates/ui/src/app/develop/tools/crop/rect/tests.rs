//! The crop rectangle's arithmetic, on a 1200 x 800 frame.

use egui::{Vec2, vec2};
use photo_core::develop::{Crop, CropRect, DevelopSettings};

use super::*;
use crate::app::develop::canvas::map::frame_geometry;

const SOURCE: (u32, u32) = (1200, 800);
const FRAME: Vec2 = vec2(1200.0, 800.0);
const MIN: Vec2 = vec2(12.0, 8.0);

fn whole() -> Upright {
    Upright {
        center: Vec2::ZERO,
        size: FRAME,
    }
}

fn fits_at(angle: f32) -> impl Fn(Upright) -> bool {
    let frame = frame_geometry(&DevelopSettings::default(), SOURCE);
    move |rect| shows_photo(rect, angle, &frame)
}

#[test]
fn a_rectangle_reads_back_from_the_settings_as_it_was_written() {
    let crop = CropRect {
        cx: 0.4,
        cy: 0.55,
        width: 0.5,
        height: 0.3,
    };
    for angle in [0.0, 7.5, -30.0] {
        let upright = Upright::of(crop, angle, FRAME);
        assert!((upright.size - vec2(600.0, 240.0)).length() < 1.0e-3);
        let back = upright.to_crop(angle, FRAME);
        for (was, is) in [
            (crop.cx, back.cx),
            (crop.cy, back.cy),
            (crop.width, back.width),
            (crop.height, back.height),
        ] {
            assert!((was - is).abs() < 1.0e-5, "{angle}: {crop:?} -> {back:?}");
        }
    }
}

#[test]
fn the_whole_frame_shows_the_photo_and_a_turned_or_larger_one_does_not() {
    assert!(fits_at(0.0)(whole()));
    assert!(!fits_at(3.0)(whole()));
    let larger = Upright {
        size: FRAME + vec2(2.0, 0.0),
        ..whole()
    };
    assert!(!fits_at(0.0)(larger));
    let inscribed = Upright::of(CropRect::inscribed(3.0, 1.5, 1.5), 3.0, FRAME);
    assert!(fits_at(3.0)(inscribed));
}

#[test]
fn a_corner_drag_keeps_the_opposite_corner_and_stops_at_the_photos_edge() {
    let start = Upright {
        center: vec2(0.0, 0.0),
        size: vec2(600.0, 400.0),
    };
    let corner = Handle { x: 1, y: 1 };
    let free = dragged(start, corner, vec2(60.0, -40.0), None, MIN, &fits_at(0.0));
    assert_eq!(free.rect().left_top(), start.rect().left_top());
    assert_eq!(free.size, vec2(660.0, 360.0));

    let far = dragged(
        start,
        corner,
        vec2(5000.0, 5000.0),
        None,
        MIN,
        &fits_at(0.0),
    );
    assert!((far.rect().right() - 600.0).abs() < 0.5, "{far:?}");
    assert!((far.rect().bottom() - 400.0).abs() < 0.5, "{far:?}");

    let tiny = dragged(
        start,
        corner,
        vec2(-5000.0, -5000.0),
        None,
        MIN,
        &fits_at(0.0),
    );
    assert_eq!(tiny.size, MIN, "no side under the least");
}

#[test]
fn a_locked_drag_keeps_the_ratio_from_a_corner_and_from_an_edge() {
    let start = Upright {
        center: Vec2::ZERO,
        size: vec2(640.0, 360.0),
    };
    let ratio = 16.0 / 9.0;
    let fits = fits_at(0.0);
    let corner = dragged(
        start,
        Handle { x: -1, y: -1 },
        vec2(-80.0, -10.0),
        Some(ratio),
        MIN,
        &fits,
    );
    assert!((corner.aspect() - ratio).abs() < 1.0e-4);
    assert_eq!(corner.size.x, 720.0, "the larger pull is followed");
    assert_eq!(corner.rect().right_bottom(), start.rect().right_bottom());

    let edge = dragged(
        start,
        Handle { x: 1, y: 0 },
        vec2(160.0, 0.0),
        Some(ratio),
        MIN,
        &fits,
    );
    assert!((edge.aspect() - ratio).abs() < 1.0e-4);
    assert_eq!(edge.rect().left(), start.rect().left());
    assert_eq!(edge.center.y, 0.0, "it grows evenly up and down");

    // Pulled far past the photo's edge: it stops there, its top where it
    // was and its shape kept.
    let held = dragged(
        start,
        Handle { x: 0, y: 1 },
        vec2(0.0, 5000.0),
        Some(ratio),
        MIN,
        &fits,
    );
    assert!((held.aspect() - ratio).abs() < 1.0e-3);
    assert!(fits(held), "{held:?}");
    assert!((held.rect().bottom() - 400.0).abs() < 0.5, "{held:?}");
    assert!((held.rect().top() - start.rect().top()).abs() < 1.0e-3);
}

#[test]
fn a_move_slides_along_the_edge_that_holds_it() {
    let start = Upright {
        center: Vec2::ZERO,
        size: vec2(400.0, 300.0),
    };
    let moved = dragged(
        start,
        Handle::MOVE,
        vec2(5000.0, 60.0),
        None,
        MIN,
        &fits_at(0.0),
    );
    assert_eq!(moved.size, start.size);
    assert!((moved.rect().right() - 600.0).abs() < 0.5, "{moved:?}");
    assert_eq!(moved.center.y, 60.0);
}

#[test]
fn straightening_shrinks_about_the_centre_and_grows_back() {
    let fits = fits_at(10.0);
    let turned = straightened(Vec2::ZERO, FRAME, 10.0, MIN, &fits);
    assert!(fits(turned));
    assert!((turned.aspect() - 1.5).abs() < 1.0e-3, "the shape is kept");
    assert!(
        turned.size.x < 1100.0 && turned.size.x > 900.0,
        "{turned:?}"
    );
    let grown = Upright {
        size: turned.size * 1.01,
        ..turned
    };
    assert!(!fits(grown), "it is the largest that fits");
    let back = straightened(Vec2::ZERO, FRAME, 0.0, MIN, &fits_at(0.0));
    assert_eq!(back, whole());

    // A rectangle in a corner: kept where it is while it fits.
    let corner = vec2(-380.0, -240.0);
    let small = straightened(corner, vec2(200.0, 120.0), 2.0, MIN, &fits_at(2.0));
    assert_eq!(small.size, vec2(200.0, 120.0));
}

#[test]
fn a_line_drawn_along_a_slope_gives_the_angle_that_levels_it() {
    let level = |dx: f32, dy: f32| leveling_angle(vec2(100.0, 100.0), vec2(100.0 + dx, 100.0 + dy));
    assert!((level(400.0, 400.0 * 5.0f32.to_radians().tan()).unwrap() - 5.0).abs() < 1.0e-3);
    assert!((level(-400.0, -400.0 * 5.0f32.to_radians().tan()).unwrap() - 5.0).abs() < 1.0e-3);
    assert!((level(400.0, -70.0).unwrap() + 9.926).abs() < 0.01);
    // Nearly upright: levelled as a vertical.
    assert!((level(20.0, 400.0).unwrap() + 2.862).abs() < 0.01);
    assert_eq!(level(1.0, 1.0), None);
}

#[test]
fn the_whole_frame_is_stored_as_no_rectangle() {
    let crop = Crop::default();
    assert_eq!(with_rect(&crop, whole(), 0.0, FRAME).rect, None);
    assert!(with_rect(&crop, whole(), 4.0, FRAME).rect.is_some());
    let part = Upright {
        center: vec2(10.0, 0.0),
        size: vec2(600.0, 400.0),
    };
    let stored = with_rect(&crop, part, 0.0, FRAME);
    assert_eq!(upright_of(&stored, FRAME), part);
}
