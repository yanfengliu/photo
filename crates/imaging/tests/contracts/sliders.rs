//! What each geometry slider does, by its effect on the picture: the sign
//! table on `Geometry`, pinned; what happens at and beyond the sliders'
//! limits; and the largest crop that avoids empty areas, which depends on
//! all of them.

use photo_core::develop::{Crop, CropRect, LensCorrections, Orientation, Transform};
use photo_core::geom::Point;
use photo_imaging::Geometry;

use crate::geometry::{H, W, build, cropped, lens, perspective, with_transform};

#[test]
fn hostile_settings_still_give_a_usable_geometry() {
    let crop = Crop {
        rect: Some(CropRect {
            cx: f32::NAN,
            cy: 0.5,
            width: -3.0,
            height: 0.0,
        }),
        angle: f32::INFINITY,
        ..Default::default()
    };
    let transform = Transform {
        vertical: f32::NAN,
        scale: 0.0,
        ..Default::default()
    };
    let geometry = Geometry::new(
        0,
        0,
        &lens(f32::NAN),
        Orientation::IDENTITY,
        &transform,
        &crop,
    );
    let (w, h) = geometry.output_size();
    assert!(w >= 1 && h >= 1);
    let p = geometry.out_to_src(Point::new(0.5, 0.5));
    assert!(p.x.is_finite() && p.y.is_finite());
    // Tilts far beyond the slider's range are held at full scale.
    let extreme = with_transform(Transform {
        vertical: 1.0e9,
        ..Default::default()
    });
    let full = with_transform(Transform {
        vertical: 100.0,
        ..Default::default()
    });
    assert_eq!(extreme, full);
    // A crop centred absurdly far away is brought back to the frame, so the
    // mapping stays finite for every finite position.
    let adrift = cropped(Crop {
        rect: Some(CropRect {
            cx: 1.0e37,
            cy: -1.0e37,
            width: 1.0,
            height: 1.0,
        }),
        ..Default::default()
    });
    let p = adrift.out_to_src(Point::new(0.5, 0.5));
    assert!(p.x.is_finite() && p.y.is_finite(), "{p:?}");
    let offsets = with_transform(Transform {
        x_offset: 3.0e38,
        y_offset: -3.0e38,
        scale: 3.0e38,
        aspect: 3.0e38,
        rotate: 3.0e38,
        ..Default::default()
    });
    let p = offsets.out_to_src(Point::new(300.0, 200.0));
    assert!(p.x.is_finite() && p.y.is_finite(), "{p:?}");
}

fn with_lens(distortion: f32) -> Geometry {
    build(
        lens(distortion),
        Orientation::IDENTITY,
        Transform::default(),
        Crop::default(),
    )
}

/// Horizontal distance in the output between two source points.
fn span_x(geometry: &Geometry, a: (f32, f32), b: (f32, f32)) -> f32 {
    let a = geometry.src_to_out(Point::new(a.0, a.1)).unwrap();
    let b = geometry.src_to_out(Point::new(b.0, b.1)).unwrap();
    (b.x - a.x).abs()
}

fn span_y(geometry: &Geometry, a: (f32, f32), b: (f32, f32)) -> f32 {
    let a = geometry.src_to_out(Point::new(a.0, a.1)).unwrap();
    let b = geometry.src_to_out(Point::new(b.0, b.1)).unwrap();
    (b.y - a.y).abs()
}

#[test]
fn negative_vertical_magnifies_the_top_and_positive_the_bottom() {
    let top = ((200.0, 40.0), (400.0, 40.0));
    let bottom = ((200.0, 360.0), (400.0, 360.0));
    let negative = with_transform(Transform {
        vertical: -50.0,
        ..Default::default()
    });
    assert!(span_x(&negative, top.0, top.1) > 1.15 * span_x(&negative, bottom.0, bottom.1));
    let positive = with_transform(Transform {
        vertical: 50.0,
        ..Default::default()
    });
    assert!(span_x(&positive, bottom.0, bottom.1) > 1.15 * span_x(&positive, top.0, top.1));
}

#[test]
fn negative_horizontal_magnifies_the_left_and_positive_the_right() {
    let left = ((60.0, 100.0), (60.0, 300.0));
    let right = ((540.0, 100.0), (540.0, 300.0));
    let negative = with_transform(Transform {
        horizontal: -50.0,
        ..Default::default()
    });
    assert!(span_y(&negative, left.0, left.1) > 1.15 * span_y(&negative, right.0, right.1));
    let positive = with_transform(Transform {
        horizontal: 50.0,
        ..Default::default()
    });
    assert!(span_y(&positive, right.0, right.1) > 1.15 * span_y(&positive, left.0, left.1));
}

#[test]
fn a_full_tilt_is_forty_five_degrees_of_a_diagonal_focal_length() {
    // With the camera model on `Geometry`, a vertical tilt of angle t maps
    // the row at centred height y to magnification 1 / (cos t + (y / f) sin t)
    // before recentring, so two rows compare as the inverse of that.
    let geometry = with_transform(Transform {
        vertical: -100.0,
        ..Default::default()
    });
    let focal = (W as f32).hypot(H as f32);
    let w = |y: f32| 45f32.to_radians().cos() + (y - 200.0) / focal * 45f32.to_radians().sin();
    let expected = w(350.0) / w(50.0);
    let measured = span_x(&geometry, (250.0, 50.0), (350.0, 50.0))
        / span_x(&geometry, (250.0, 350.0), (350.0, 350.0));
    assert!(
        (measured - expected).abs() < 1e-3,
        "{measured} vs {expected}"
    );
}

#[test]
fn perspective_keeps_the_frame_centre_and_its_area_scale() {
    let geometry = with_transform(Transform {
        vertical: -70.0,
        horizontal: 45.0,
        ..Default::default()
    });
    let centre = geometry.src_to_out(Point::new(300.0, 200.0)).unwrap();
    assert!(
        centre.distance(Point::new(300.0, 200.0)) < 1e-2,
        "{centre:?}"
    );
    let scale = geometry.source_pixels_per_output_pixel();
    assert!((scale - 1.0).abs() < 1e-3, "{scale}");
}

#[test]
fn perspective_keeps_straight_lines_straight() {
    let geometry = with_transform(perspective());
    let at = |t: f32| {
        geometry
            .src_to_out(Point::new(100.0 + 400.0 * t, 80.0 + 250.0 * t))
            .unwrap()
    };
    let (a, b, c) = (at(0.0), at(0.4), at(1.0));
    let cross = (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x);
    assert!(cross.abs() / a.distance(c) < 1e-2, "{cross}");
}

#[test]
fn positive_rotate_turns_the_picture_clockwise() {
    let geometry = with_transform(Transform {
        rotate: 5.0,
        ..Default::default()
    });
    let centre = geometry.src_to_out(Point::new(300.0, 200.0)).unwrap();
    assert!(centre.distance(Point::new(300.0, 200.0)) < 1e-3);
    let right = geometry.src_to_out(Point::new(400.0, 200.0)).unwrap();
    let (sin, cos) = 5f32.to_radians().sin_cos();
    // Clockwise on screen: the point to the right of the centre moves down.
    let expected = Point::new(300.0 + 100.0 * cos, 200.0 + 100.0 * sin);
    assert!(right.distance(expected) < 1e-2, "{right:?} vs {expected:?}");
}

#[test]
fn positive_aspect_makes_the_picture_taller_and_narrower() {
    let geometry = with_transform(Transform {
        aspect: 100.0,
        ..Default::default()
    });
    let across = span_x(&geometry, (250.0, 200.0), (350.0, 200.0));
    let down = span_y(&geometry, (300.0, 150.0), (300.0, 250.0));
    assert!((down / across - 2.0).abs() < 1e-3, "{down} / {across}");
    assert!(
        (down * across - 100.0 * 100.0).abs() < 1.0,
        "area is preserved"
    );
}

#[test]
fn scale_and_offsets_move_the_picture_as_documented() {
    let scaled = with_transform(Transform {
        scale: 150.0,
        ..Default::default()
    });
    assert!((span_x(&scaled, (300.0, 200.0), (400.0, 200.0)) - 150.0).abs() < 1e-2);

    let moved = with_transform(Transform {
        x_offset: 50.0,
        y_offset: -20.0,
        ..Default::default()
    });
    let centre = moved.src_to_out(Point::new(300.0, 200.0)).unwrap();
    // Positive x moves right, positive y moves down; 100 is half the frame.
    let expected = Point::new(300.0 + 0.25 * W as f32, 200.0 - 0.10 * H as f32);
    assert!(centre.distance(expected) < 1e-2, "{centre:?}");
}

#[test]
fn distortion_fixes_the_centre_and_corners_and_is_radially_monotonic() {
    for amount in [-100.0, -40.0, 40.0, 100.0] {
        let geometry = with_lens(amount);
        for fixed in [
            Point::new(300.0, 200.0),
            Point::new(0.0, 0.0),
            Point::new(600.0, 0.0),
            Point::new(600.0, 400.0),
            Point::new(0.0, 400.0),
        ] {
            let mapped = geometry.out_to_src(fixed);
            assert!(
                mapped.distance(fixed) < 1e-3,
                "{amount}: {fixed:?} -> {mapped:?}"
            );
        }
        // Walking outward along any ray, the source radius keeps growing,
        // well past the corners too.
        let centre = Point::new(300.0, 200.0);
        for (dx, dy) in [(1.0, 0.0), (0.0, 1.0), (0.6, 0.8), (-0.8, 0.6)] {
            let mut previous = -1.0f32;
            for step in 0..120 {
                let t = step as f32 * 6.0;
                let radius = geometry
                    .out_to_src(Point::new(300.0 + dx * t, 200.0 + dy * t))
                    .distance(centre);
                assert!(radius > previous, "{amount}: ray ({dx}, {dy}) folds at {t}");
                previous = radius;
            }
        }
    }
}

#[test]
fn positive_distortion_pulls_the_edge_midpoints_inward() {
    // Correcting barrel distortion stretches the corners relative to the
    // centre: with the corners fixed, the middle of each edge moves inward
    // and the centre shrinks.
    let positive = with_lens(60.0);
    let edge = positive.src_to_out(Point::new(600.0, 200.0)).unwrap();
    assert!(edge.x < 590.0, "{edge:?}");
    assert!(span_x(&positive, (300.0, 200.0), (320.0, 200.0)) < 19.0);

    let negative = with_lens(-60.0);
    let edge = negative.src_to_out(Point::new(600.0, 200.0)).unwrap();
    assert!(edge.x > 610.0, "{edge:?}");
    assert!(span_x(&negative, (300.0, 200.0), (320.0, 200.0)) > 21.0);
}

#[test]
fn full_distortion_is_a_quarter_of_the_radius_at_the_centre() {
    // Reading from r * (1 + a * (1 - r^2)) with a = 0.25: near the centre
    // the corrected picture is 1 / 1.25 the size of the source.
    let geometry = with_lens(100.0);
    assert!((span_x(&geometry, (300.0, 200.0), (301.0, 200.0)) - 0.8).abs() < 1e-3);
    assert!((geometry.source_pixels_per_output_pixel() - 1.25).abs() < 1e-3);
}

fn maps_inside_the_source(geometry_with_crop: &Geometry) -> bool {
    let (ow, oh) = geometry_with_crop.output_size();
    (0..=40).all(|i| {
        let t = i as f32 / 40.0;
        [
            Point::new(t * ow as f32, 0.0),
            Point::new(t * ow as f32, oh as f32),
            Point::new(0.0, t * oh as f32),
            Point::new(ow as f32, t * oh as f32),
        ]
        .iter()
        .all(|&p| {
            let s = geometry_with_crop.out_to_src(p);
            // Half a pixel of slack: the output size is rounded to whole pixels.
            s.x >= -0.6 && s.x <= W as f32 + 0.6 && s.y >= -0.6 && s.y <= H as f32 + 0.6
        })
    })
}

#[test]
fn the_largest_valid_crop_of_an_untouched_frame_is_the_whole_frame() {
    let geometry = Geometry::identity(W, H);
    let rect = geometry
        .largest_valid_crop(0.0, W as f32 / H as f32)
        .unwrap();
    assert_eq!(rect, CropRect::FULL);
    // Another shape is fitted into the frame, as large as it goes.
    let square = geometry.largest_valid_crop(0.0, 1.0).unwrap();
    assert!((square.height - 1.0).abs() < 1e-6 && (square.aspect(1.5) - 1.0).abs() < 1e-5);
    // A straightened frame needs the inscribed rectangle and nothing less.
    let tilted = geometry.largest_valid_crop(8.0, 1.5).unwrap();
    let inscribed = CropRect::inscribed(8.0, 1.5, 1.5);
    assert!(
        (tilted.width - inscribed.width).abs() < 1e-4,
        "{tilted:?} vs {inscribed:?}"
    );
}

#[test]
fn the_largest_valid_crop_avoids_empty_areas_and_cannot_grow() {
    let aspect = W as f32 / H as f32;
    let rotated = Transform {
        rotate: 6.0,
        ..Default::default()
    };
    let tilted = Transform {
        vertical: -40.0,
        ..Default::default()
    };
    let cases = [
        (lens(70.0), Transform::default(), 0.0),
        (LensCorrections::default(), rotated, 0.0),
        (LensCorrections::default(), tilted, 0.0),
        (lens(30.0), perspective(), 7.0),
    ];
    for (lens, transform, angle) in cases {
        let open = build(lens, Orientation::IDENTITY, transform, Crop::default());
        let rect = open
            .largest_valid_crop(angle, aspect)
            .unwrap_or_else(|| panic!("{transform:?}: no crop"));
        assert!(
            rect.width < 1.0 && rect.height < 1.0,
            "{transform:?}: {rect:?}"
        );
        assert!((rect.aspect(aspect) - aspect).abs() < 1e-2);

        let cropped = |rect: CropRect| {
            let crop = Crop {
                rect: Some(rect),
                angle,
                ..Default::default()
            };
            build(lens, Orientation::IDENTITY, transform, crop)
        };
        assert!(
            maps_inside_the_source(&cropped(rect)),
            "{transform:?}: {rect:?}"
        );
        let grown = CropRect {
            width: rect.width * 1.03,
            height: rect.height * 1.03,
            ..rect
        };
        assert!(
            !maps_inside_the_source(&cropped(grown)),
            "{transform:?}: {rect:?}"
        );
    }
}

#[test]
fn a_frame_pushed_out_of_the_picture_has_no_valid_crop() {
    let geometry = with_transform(Transform {
        scale: 50.0,
        x_offset: 100.0,
        ..Default::default()
    });
    // The picture now covers only the right-hand edge; the frame centre is empty.
    assert_eq!(geometry.largest_valid_crop(0.0, 1.5), None);

    // With the picture's edge 1.8 px from the frame centre, the only centred
    // rectangle that fits is thinner than a crop is allowed to be. Handing
    // it back would have the crop widened into the empty area.
    let grazing = with_transform(Transform {
        scale: 50.0,
        x_offset: 49.4,
        ..Default::default()
    });
    assert_eq!(grazing.largest_valid_crop(0.0, 1.5), None);
    // A little more overlap and there is a usable, if small, crop.
    let overlapping = with_transform(Transform {
        scale: 50.0,
        x_offset: 40.0,
        ..Default::default()
    });
    let rect = overlapping.largest_valid_crop(0.0, 1.5).unwrap();
    assert!(
        rect.width >= CropRect::MIN_SIDE && rect.width < 0.12,
        "{rect:?}"
    );
}
