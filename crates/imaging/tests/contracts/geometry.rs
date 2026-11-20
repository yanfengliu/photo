//! Geometry: frame mapping round trips, output sizes, and agreement with the
//! frame definitions in `photo_core::develop::geometry`. What each slider
//! does is pinned in `sliders.rs`.

use photo_core::develop::{Crop, CropRect, LensCorrections, Orientation, Transform};
use photo_core::geom::Point;
use photo_imaging::Geometry;

pub const W: u32 = 600;
pub const H: u32 = 400;

pub fn all_orientations() -> Vec<Orientation> {
    (0..4)
        .flat_map(|turns| [false, true].map(|flip| Orientation::new(turns, flip)))
        .collect()
}

pub fn lens(distortion: f32) -> LensCorrections {
    LensCorrections {
        distortion,
        ..Default::default()
    }
}

pub fn build(
    lens: LensCorrections,
    orientation: Orientation,
    transform: Transform,
    crop: Crop,
) -> Geometry {
    Geometry::new(W, H, &lens, orientation, &transform, &crop)
}

/// A geometry that differs from the identity only in its orientation.
fn oriented(orientation: Orientation) -> Geometry {
    build(
        LensCorrections::default(),
        orientation,
        Transform::default(),
        Crop::default(),
    )
}

/// A geometry that differs from the identity only in its crop.
fn cropped(crop: Crop) -> Geometry {
    build(
        LensCorrections::default(),
        Orientation::IDENTITY,
        Transform::default(),
        crop,
    )
}

/// A geometry that differs from the identity only in its transform.
pub fn with_transform(transform: Transform) -> Geometry {
    build(
        LensCorrections::default(),
        Orientation::IDENTITY,
        transform,
        Crop::default(),
    )
}

fn angled_crop() -> Crop {
    Crop {
        rect: Some(CropRect {
            cx: 0.45,
            cy: 0.55,
            width: 0.5,
            height: 0.4,
        }),
        angle: 12.0,
        ..Default::default()
    }
}

pub fn perspective() -> Transform {
    Transform {
        vertical: -55.0,
        horizontal: 30.0,
        rotate: 4.0,
        aspect: 25.0,
        scale: 110.0,
        x_offset: 8.0,
        y_offset: -6.0,
    }
}

fn varied() -> Vec<(String, Geometry)> {
    let mut cases = Vec::new();
    for orientation in all_orientations() {
        cases.push((format!("{orientation:?}"), oriented(orientation)));
        cases.push((
            format!("{orientation:?} + angled crop"),
            build(
                LensCorrections::default(),
                orientation,
                Transform::default(),
                angled_crop(),
            ),
        ));
        cases.push((
            format!("{orientation:?} + everything"),
            build(lens(40.0), orientation, perspective(), angled_crop()),
        ));
    }
    for distortion in [-100.0, -35.0, 35.0, 100.0] {
        cases.push((
            format!("distortion {distortion}"),
            build(
                lens(distortion),
                Orientation::IDENTITY,
                Transform::default(),
                Crop::default(),
            ),
        ));
    }
    for (vertical, horizontal) in [(-100.0, 0.0), (100.0, 0.0), (0.0, -100.0), (60.0, 60.0)] {
        let tilt = Transform {
            vertical,
            horizontal,
            ..Default::default()
        };
        cases.push((
            format!("perspective {vertical}/{horizontal}"),
            with_transform(tilt),
        ));
    }
    cases.push(("perspective mix".into(), with_transform(perspective())));
    cases
}

#[test]
fn mapping_out_to_source_and_back_is_identity() {
    for (name, geometry) in varied() {
        let (w, h) = geometry.output_size();
        for gy in 0..=10 {
            for gx in 0..=10 {
                let p = Point::new(w as f32 * gx as f32 / 10.0, h as f32 * gy as f32 / 10.0);
                let source = geometry.out_to_src(p);
                let back = geometry
                    .src_to_out(source)
                    .unwrap_or_else(|| panic!("{name}: {p:?} has no way back from {source:?}"));
                assert!(
                    back.distance(p) < 1e-2,
                    "{name}: {p:?} -> {source:?} -> {back:?}"
                );
            }
        }
    }
}

#[test]
fn mapping_source_to_out_and_back_is_identity() {
    for (name, geometry) in varied() {
        for gy in 1..10 {
            for gx in 1..10 {
                let p = Point::new(W as f32 * gx as f32 / 10.0, H as f32 * gy as f32 / 10.0);
                let out = geometry
                    .src_to_out(p)
                    .unwrap_or_else(|| panic!("{name}: {p:?}"));
                let back = geometry.out_to_src(out);
                assert!(
                    back.distance(p) < 1e-2,
                    "{name}: {p:?} -> {out:?} -> {back:?}"
                );
            }
        }
    }
}

#[test]
fn large_pictures_and_small_scales_are_mapped_like_any_other() {
    // A mirrored panorama: nothing about its width makes the mirror special.
    let panorama = Geometry::new(
        120_000,
        5_000,
        &LensCorrections::default(),
        Orientation::new(2, true),
        &Transform::default(),
        &Crop::default(),
    );
    assert_eq!(panorama.output_size(), (120_000, 5_000));
    // Mirror, then half a turn: a top-to-bottom flip.
    assert_eq!(
        panorama.src_to_out(Point::new(10.5, 20.5)),
        Some(Point::new(10.5, 4_979.5))
    );
    assert_eq!(
        panorama.out_to_src(Point::new(10.5, 4_979.5)),
        Point::new(10.5, 20.5)
    );
    // The smallest scale the transform honours, on an ordinary frame.
    let shrunk = Geometry::new(
        6_000,
        4_000,
        &LensCorrections::default(),
        Orientation::IDENTITY,
        &Transform {
            scale: 1.0,
            ..Default::default()
        },
        &Crop::default(),
    );
    assert!((shrunk.source_pixels_per_output_pixel() - 100.0).abs() < 1e-2);
    let corner = shrunk.src_to_out(Point::new(0.0, 0.0)).unwrap();
    assert!(corner.distance(Point::new(2_970.0, 1_980.0)) < 1e-2, "{corner:?}");
}

#[test]
fn identity_settings_map_every_point_to_itself_exactly() {
    let geometry = Geometry::identity(W, H);
    assert_eq!(geometry.output_size(), (W, H));
    assert_eq!(geometry.frame_size(), (W, H));
    assert_eq!(geometry.source_size(), (W, H));
    assert_eq!(geometry.source_pixels_per_output_pixel(), 1.0);
    for p in [
        Point::new(0.5, 0.5),
        Point::new(123.5, 77.5),
        Point::new(599.5, 399.5),
    ] {
        assert_eq!(geometry.out_to_src(p), p);
        assert_eq!(geometry.src_to_out(p), Some(p));
    }
}

#[test]
fn quarter_turns_swap_the_output_dimensions() {
    for orientation in all_orientations() {
        let geometry = oriented(orientation);
        let expected = if orientation.swaps_axes() {
            (H, W)
        } else {
            (W, H)
        };
        assert_eq!(geometry.output_size(), expected, "{orientation:?}");
        assert_eq!(geometry.frame_size(), expected, "{orientation:?}");
    }
}

#[test]
fn orientation_agrees_with_the_core_definition() {
    for orientation in all_orientations() {
        let geometry = oriented(orientation);
        let (ow, oh) = geometry.output_size();
        for (nx, ny) in [(0.0, 0.0), (1.0, 0.0), (0.2, 0.7), (0.9, 0.35)] {
            let expected = orientation.forward(Point::new(nx, ny));
            let out = geometry
                .src_to_out(Point::new(nx * W as f32, ny * H as f32))
                .unwrap();
            let want = Point::new(expected.x * ow as f32, expected.y * oh as f32);
            assert!(
                out.distance(want) < 1e-3,
                "{orientation:?}: {out:?} vs {want:?}"
            );
        }
    }
}

#[test]
fn a_crop_sets_the_output_size_in_frame_pixels() {
    let rect = CropRect {
        cx: 0.5,
        cy: 0.5,
        width: 0.5,
        height: 0.25,
    };
    let crop = Crop {
        rect: Some(rect),
        ..Default::default()
    };
    assert_eq!(cropped(crop).output_size(), (300, 100));
    // The crop is defined in the transformed frame, so after a quarter turn
    // its fractions apply to the swapped dimensions.
    let turned = build(
        LensCorrections::default(),
        Orientation::new(1, false),
        Transform::default(),
        crop,
    );
    assert_eq!(turned.output_size(), (200, 150));
}

#[test]
fn an_unset_rectangle_with_an_angle_uses_the_inscribed_crop() {
    let crop = Crop {
        angle: 10.0,
        ..Default::default()
    };
    let aspect = W as f32 / H as f32;
    let rect = CropRect::inscribed(10.0, aspect, aspect);
    let expected = (
        (rect.width * W as f32).round() as u32,
        (rect.height * H as f32).round() as u32,
    );
    assert_eq!(cropped(crop).output_size(), expected);
}

#[test]
fn output_corners_land_on_the_crop_rectangle_corners() {
    let crop = angled_crop();
    let geometry = cropped(crop);
    let (ow, oh) = geometry.output_size();
    let corners = crop.rect.unwrap().corners(crop.angle, W as f32 / H as f32);
    let output = [
        (0.0, 0.0),
        (ow as f32, 0.0),
        (ow as f32, oh as f32),
        (0.0, oh as f32),
    ];
    for (corner, (x, y)) in corners.iter().zip(output) {
        let got = geometry.out_to_src(Point::new(x, y));
        let want = Point::new(corner.x * W as f32, corner.y * H as f32);
        // The output size is rounded to whole pixels, so allow half a pixel.
        assert!(got.distance(want) < 0.75, "{got:?} vs {want:?}");
    }
}

#[test]
fn a_positive_crop_angle_turns_the_picture_counter_clockwise() {
    let geometry = cropped(Crop {
        angle: 10.0,
        ..Default::default()
    });
    let (ow, oh) = geometry.output_size();
    let centre = geometry.src_to_out(Point::new(300.0, 200.0)).unwrap();
    assert!(centre.distance(Point::new(ow as f32 / 2.0, oh as f32 / 2.0)) < 1e-2);
    // Content to the right of the centre moves up the screen.
    let right = geometry.src_to_out(Point::new(400.0, 200.0)).unwrap();
    assert!(right.x > centre.x && right.y < centre.y - 10.0, "{right:?}");
}

#[test]
fn points_behind_the_virtual_camera_have_no_output_position() {
    let geometry = with_transform(Transform {
        vertical: -100.0,
        ..Default::default()
    });
    // Far above the frame the tilted picture plane passes behind the camera.
    assert_eq!(geometry.src_to_out(Point::new(300.0, -5000.0)), None);
    assert!(geometry.src_to_out(Point::new(300.0, 0.0)).is_some());
    assert_eq!(geometry.src_to_out(Point::new(f32::NAN, 0.0)), None);
    // The other direction never fails: it always returns something finite.
    let far = geometry.out_to_src(Point::new(300.0, -1.0e7));
    assert!(far.x.is_finite() && far.y.is_finite());
}

#[test]
fn content_cut_away_by_the_crop_still_has_an_output_position() {
    let crop = Crop {
        rect: Some(CropRect {
            cx: 0.5,
            cy: 0.5,
            width: 0.5,
            height: 0.5,
        }),
        ..Default::default()
    };
    let geometry = cropped(crop);
    assert_eq!(geometry.output_size(), (300, 200));
    // The source's top-left corner is outside the 300 x 200 output.
    assert_eq!(
        geometry.src_to_out(Point::new(0.0, 0.0)),
        Some(Point::new(-150.0, -100.0))
    );
    assert_eq!(
        geometry.out_to_src(Point::new(-150.0, -100.0)),
        Point::new(0.0, 0.0)
    );
}

#[test]
fn the_picture_keeps_its_place_when_a_crop_looks_past_the_horizon() {
    // An extreme keystone on a shrunken, shifted picture brings the horizon
    // of the tilted plane into the frame. A crop placed beyond it shows no
    // image at all, but the picture itself is still in front of the camera:
    // it must keep a (far away) output position, and map back from it.
    let transform = Transform {
        vertical: -100.0,
        aspect: -100.0,
        scale: 50.0,
        y_offset: -100.0,
        ..Default::default()
    };
    let centre = Point::new(300.0, 200.0);
    for cy in [0.03, 0.5, 0.97] {
        let crop = Crop {
            rect: Some(CropRect {
                cx: 0.5,
                cy,
                width: 0.05,
                height: 0.05,
            }),
            ..Default::default()
        };
        let geometry = build(
            LensCorrections::default(),
            Orientation::IDENTITY,
            transform,
            crop,
        );
        let out = geometry
            .src_to_out(centre)
            .unwrap_or_else(|| panic!("crop at {cy}: the picture's centre has no position"));
        let back = geometry.out_to_src(out);
        assert!(
            back.distance(centre) < 1e-2,
            "crop at {cy}: {out:?} -> {back:?}"
        );
        // Whatever the crop shows, positions inside it map somewhere finite.
        let (ow, oh) = geometry.output_size();
        let inside = geometry.out_to_src(Point::new(ow as f32 / 2.0, oh as f32 / 2.0));
        assert!(inside.x.is_finite() && inside.y.is_finite());
    }
}

#[test]
fn settings_build_the_same_geometry_as_their_parts() {
    let mut settings = photo_core::develop::DevelopSettings {
        lens: lens(20.0),
        orientation: Orientation::new(3, true),
        transform: perspective(),
        crop: angled_crop(),
        ..Default::default()
    };
    settings.tone.exposure = 1.0;
    let from_settings = Geometry::from_settings(W, H, &settings);
    let from_parts = build(
        settings.lens,
        settings.orientation,
        settings.transform,
        settings.crop,
    );
    assert_eq!(from_settings, from_parts);
    assert_ne!(from_settings, Geometry::identity(W, H));
}

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
