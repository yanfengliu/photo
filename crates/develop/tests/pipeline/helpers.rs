//! The helpers the engine and the UI call besides rendering.

use photo_core::develop::{DevelopSettings, Mask, MaskShape, WbPreset};
use photo_core::geom::Point;
use photo_develop::{
    as_shot_white_balance, auto_tone, auto_white_balance, histogram, mask_alpha,
    pick_white_balance, tone_curve_sampler,
};
use photo_testkit::{grey_ramp, radiance};

use crate::fixtures::{camera_scene, chroma, edit, full, mean, raster, raster_linear, render};

#[test]
fn the_as_shot_white_balance_is_the_cameras_and_raster_files_have_none() {
    let camera = camera_scene(240, 160);
    let (kelvin, tint) = as_shot_white_balance(&camera).unwrap();
    // The testkit camera's as-shot neutral is its response to D50.
    assert!(
        (kelvin - 5000.0).abs() < 150.0 && tint.abs() < 10.0,
        "{kelvin} {tint}"
    );
    let (_, raster) = raster(240, 160);
    assert_eq!(as_shot_white_balance(&raster), None);
}

/// The testkit scene in linear sRGB under a warm cast.
fn tinted(width: u32, height: u32) -> photo_develop::Source {
    raster_linear(width, height, |x, y| {
        let [r, g, b] = radiance(i64::from(x), i64::from(y), width, height);
        [r * 1.25, g, b * 0.7]
    })
}

#[test]
fn the_picker_neutralises_what_it_is_pointed_at() {
    let (width, height) = (480, 320);
    let source = tinted(width, height);
    let (x0, y0, ramp_width, ramp_height) = grey_ramp(width, height);
    let point = Point::new(
        (x0 + ramp_width * 3 / 5) as f32,
        (y0 + ramp_height / 2) as f32,
    );
    let picked = pick_white_balance(&source, &DevelopSettings::default(), point).unwrap();
    assert_eq!(picked.preset, WbPreset::Custom);
    let settings = edit(|s| s.white_balance = picked);
    let image = render(&source, &settings, full(&source, &settings));
    let grey = mean(&image, point.x as u32 - 2, point.y as u32 - 2, 5, 5);
    assert!(chroma(grey) <= 2.0, "{grey:?}");
    assert!(pick_white_balance(&source, &settings, Point::new(-5.0, 10.0)).is_none());
    // A camera source picks in kelvin.
    let camera = camera_scene(width, height);
    let picked = pick_white_balance(&camera, &DevelopSettings::default(), point).unwrap();
    let (kelvin, _) = as_shot_white_balance(&camera).unwrap();
    assert!(
        (picked.temp - kelvin).abs() < 150.0,
        "{picked:?} vs {kelvin}"
    );
}

/// Mostly neutral surfaces (a ramp of greys) with a band of strong colours,
/// all under a warm cast: what grey world assumes a scene to be.
fn cast_greys(width: u32, height: u32) -> photo_develop::Source {
    const COLOURS: [[f32; 3]; 4] = [
        [0.6, 0.05, 0.04],
        [0.05, 0.4, 0.06],
        [0.05, 0.08, 0.5],
        [0.6, 0.5, 0.05],
    ];
    raster_linear(width, height, |x, y| {
        let colour = if y > height * 4 / 5 {
            COLOURS[(x * 4 / width) as usize]
        } else {
            [0.03 + 0.6 * (x as f32 / width as f32).powi(2); 3]
        };
        [colour[0] * 1.25, colour[1], colour[2] * 0.7]
    })
}

#[test]
fn auto_white_balance_removes_a_cast_from_neutral_surfaces() {
    let (width, height) = (480, 320);
    let source = cast_greys(width, height);
    let auto = auto_white_balance(&source);
    assert_eq!(auto.preset, WbPreset::Auto);
    let cast = |settings: &DevelopSettings| {
        let image = render(&source, settings, full(&source, settings));
        chroma(mean(&image, width / 2, height / 3, 20, 20))
    };
    let (before, after) = (
        cast(&DevelopSettings::default()),
        cast(&edit(|s| s.white_balance = auto)),
    );
    assert!(after < before * 0.2, "cast {before} -> {after}");
    // A scene without a cast is left (nearly) alone.
    let neutral = raster_linear(width, height, |x, _| {
        [0.03 + 0.6 * (x as f32 / width as f32); 3]
    });
    let auto = auto_white_balance(&neutral);
    assert!(auto.temp.abs() < 1.0 && auto.tint.abs() < 1.0, "{auto:?}");
}

#[test]
fn auto_tone_brings_an_underexposed_picture_back() {
    let (width, height) = (480, 320);
    let dark = raster_linear(width, height, |x, y| {
        radiance(i64::from(x), i64::from(y), width, height).map(|c| c / 4.0)
    });
    let tone = auto_tone(&dark, &DevelopSettings::default());
    assert!(tone.exposure > 1.0 && tone.exposure < 3.0, "{tone:?}");
    let (_, normal) = raster(width, height);
    let tone = auto_tone(&normal, &DevelopSettings::default());
    assert!(tone.exposure.abs() < 1.2, "{tone:?}");
    for value in [tone.highlights, tone.shadows, tone.whites, tone.blacks] {
        assert!((-100.0..=100.0).contains(&value) && value.fract() == 0.0);
    }
    // The median lands near middle grey.
    let settings = edit(|s| s.tone = auto_tone(&dark, s));
    let image = render(&dark, &settings, full(&dark, &settings));
    let mut lumas: Vec<u8> = image.pixels.chunks(4).map(|p| p[1]).collect();
    lumas.sort_unstable();
    let median = lumas[lumas.len() / 2];
    assert!((90..=150).contains(&median), "median code {median}");
}

#[test]
fn the_curve_sampler_plots_contrast_and_the_curves() {
    let neutral = tone_curve_sampler(&DevelopSettings::default());
    for step in 0..=10 {
        let v = step as f32 / 10.0;
        assert!((neutral(v) - v).abs() < 1e-6);
    }
    let contrast = tone_curve_sampler(&edit(|s| s.tone.contrast = 50.0));
    assert!(contrast(0.25) < 0.25 && contrast(0.75) > 0.75);
    assert!((contrast(0.5) - 0.5).abs() < 1e-6);
}

#[test]
fn the_histogram_and_the_mask_overlay_describe_the_render() {
    let (_, source) = raster(240, 160);
    let settings = DevelopSettings::default();
    let view = full(&source, &settings);
    let counts = histogram(&render(&source, &settings, view));
    assert_eq!(counts.pixel_count(), 240 * 160);
    let mask = Mask::from_shape(
        MaskShape::Radial {
            center: Point::new(0.5, 0.5),
            radius_x: 0.2,
            radius_y: 0.2,
            angle: 0.0,
            feather: 0.0,
            invert: false,
        },
        1,
    );
    let settings = edit(|s| s.masks.push(mask));
    let alpha = mask_alpha(&source, &settings, 0, view).unwrap();
    assert_eq!((alpha.width(), alpha.height()), (240, 160));
    assert_eq!((alpha.get(120, 80), alpha.get(5, 5)), (1.0, 0.0));
    assert!(mask_alpha(&source, &settings, 1, view).is_none());
}
