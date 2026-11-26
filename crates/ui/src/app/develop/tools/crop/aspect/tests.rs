//! Turns, mirrors and the aspect menu, on a 1200 x 800 frame.

use egui::{Vec2, vec2};
use photo_core::develop::{Crop, CropConstraint, CropRect, DevelopSettings, Orientation};
use photo_core::geom::Point;
use photo_engine::Geometry;

use super::*;

const SOURCE: (u32, u32) = (1200, 800);
const FRAME: Vec2 = vec2(1200.0, 800.0);

/// The source positions of the output's four corners.
fn shown(settings: &DevelopSettings) -> Vec<Point> {
    let geometry = Geometry::from_settings(SOURCE.0, SOURCE.1, settings);
    let (width, height) = geometry.output_size();
    let (width, height) = (width as f32, height as f32);
    [(0.0, 0.0), (width, 0.0), (width, height), (0.0, height)]
        .iter()
        .map(|(x, y)| geometry.out_to_src(Point::new(*x, *y)))
        .collect()
}

#[test]
fn a_turned_or_mirrored_crop_stays_on_its_content() {
    let settings = DevelopSettings {
        orientation: Orientation::new(0, true),
        crop: Crop {
            rect: Some(CropRect {
                cx: 0.3,
                cy: 0.6,
                width: 0.4,
                height: 0.25,
            }),
            angle: 8.0,
            constraint: CropConstraint::Ratio {
                width: 16.0,
                height: 9.0,
            },
        },
        ..DevelopSettings::default()
    };
    let before = shown(&settings);
    for way in [
        Turn::Right,
        Turn::Left,
        Turn::FlipHorizontal,
        Turn::FlipVertical,
    ] {
        let mut turned = settings.clone();
        turn(&mut turned, way);
        let after = shown(&turned);
        for corner in &before {
            let nearest = after
                .iter()
                .map(|other| other.distance(*corner))
                .fold(f32::MAX, f32::min);
            assert!(
                nearest < 1.5,
                "{way:?}: {corner:?} is {nearest} px from any corner"
            );
        }
    }
    let mut quarter = settings.clone();
    turn(&mut quarter, Turn::Right);
    assert_eq!(
        quarter.crop.constraint,
        CropConstraint::Ratio {
            width: 9.0,
            height: 16.0
        }
    );
    assert_eq!(quarter.crop.angle, 8.0);
    let mut mirrored = settings.clone();
    turn(&mut mirrored, Turn::FlipHorizontal);
    assert_eq!(mirrored.crop.angle, -8.0);
}

#[test]
fn the_menu_reads_a_stored_ratio_either_way_round() {
    assert_eq!(
        Aspect::of(CropConstraint::Original, FRAME),
        Some(Aspect::Original)
    );
    assert_eq!(Aspect::of(CropConstraint::Free, FRAME), None);
    let ratio = |width, height| CropConstraint::Ratio { width, height };
    assert_eq!(
        Aspect::of(ratio(16.0, 9.0), FRAME),
        Some(Aspect::Ratio(9.0, 16.0))
    );
    assert_eq!(
        Aspect::of(ratio(4.0, 5.0), FRAME),
        Some(Aspect::Ratio(4.0, 5.0))
    );
    assert_eq!(
        Aspect::of(ratio(3.0, 2.0), FRAME),
        Some(Aspect::Ratio(2.0, 3.0))
    );
    assert_eq!(
        Aspect::of(ratio(1200.0, 700.0), vec2(1200.0, 700.0)),
        Some(Aspect::AsShot)
    );
    assert_eq!(Aspect::of(ratio(2.2, 1.0), FRAME), Some(Aspect::Custom));

    let wide = Aspect::Ratio(9.0, 16.0).constraint(FRAME, true, (1.0, 1.0));
    assert_eq!(wide, ratio(16.0, 9.0));
    let tall = Aspect::Ratio(9.0, 16.0).constraint(FRAME, false, (1.0, 1.0));
    assert_eq!(tall, ratio(9.0, 16.0));
    assert_eq!(
        Aspect::Custom.constraint(FRAME, true, (7.0, 3.0)),
        ratio(7.0, 3.0)
    );
    assert_eq!(locked_ratio(CropConstraint::Original, FRAME), Some(1.5));
    assert_eq!(locked_ratio(CropConstraint::Free, FRAME), None);
    assert_eq!(Aspect::Ratio(2.0, 3.0).label(), "2 : 3");
}
