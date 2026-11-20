//! Direction tests for the Basic panel's tone controls, the Tone Curve
//! panel and the camera profiles: each slider moves the image the way its
//! name says, and each tone zone slider moves its own zone the most.

use photo_color::tone::{BLACKS_PEAK, HIGHLIGHTS_PEAK, SHADOWS_PEAK, WHITES_PEAK};
use photo_core::colorimetry::RgbSpace;
use photo_core::develop::{CurvePoint, DevelopSettings, PointCurve, Profile};
use photo_develop::Source;
use photo_testkit::patches;

use crate::fixtures::{camera_scene, edit, full, mean, raster_linear, render, render16};

/// Perceptual values of the wedge's flat bands, left to right: the four
/// zone peaks and middle grey in between.
pub const BANDS: [f32; 5] = [BLACKS_PEAK, SHADOWS_PEAK, 0.5, HIGHLIGHTS_PEAK, WHITES_PEAK];
const BAND_WIDTH: u32 = 80;
const HEIGHT: u32 = 120;

/// A raster source of flat neutral bands at [`BANDS`] (perceptual values,
/// `v = Y^(1/2.2)`).
pub fn wedge() -> Source {
    raster_linear(BAND_WIDTH * BANDS.len() as u32, HEIGHT, |x, _| {
        let v = BANDS[(x / BAND_WIDTH) as usize];
        [v.powf(2.2); 3]
    })
}

/// Each band's interior rendered, as perceptual values from 16-bit sRGB.
pub fn bands(source: &Source, settings: &DevelopSettings) -> [f32; 5] {
    let image = render16(source, settings, full(source, settings));
    std::array::from_fn(|band| {
        let x = band as u32 * BAND_WIDTH + BAND_WIDTH / 2;
        let mut sum = 0.0;
        for y in HEIGHT / 4..HEIGHT * 3 / 4 {
            let [_, g, _] = image.pixel(x, y);
            sum += RgbSpace::Srgb
                .decode(f32::from(g) / 65535.0)
                .powf(1.0 / 2.2);
        }
        sum / (HEIGHT / 2) as f32
    })
}

/// How far each band moved from the default rendering.
fn shifts(source: &Source, settings: &DevelopSettings) -> [f32; 5] {
    let (before, after) = (
        bands(source, &DevelopSettings::default()),
        bands(source, settings),
    );
    std::array::from_fn(|band| after[band] - before[band])
}

#[test]
fn exposure_brightens_and_darkens_every_tone() {
    let source = wedge();
    let up = shifts(&source, &edit(|s| s.tone.exposure = 1.0));
    let down = shifts(&source, &edit(|s| s.tone.exposure = -1.0));
    for band in 0..4 {
        assert!(
            up[band] > 0.02 && down[band] < -0.02,
            "{band}: {up:?} {down:?}"
        );
    }
}

#[test]
fn contrast_spreads_the_tones_around_middle_grey() {
    let source = wedge();
    let more = shifts(&source, &edit(|s| s.tone.contrast = 60.0));
    let less = shifts(&source, &edit(|s| s.tone.contrast = -60.0));
    assert!(more[1] < -0.01 && more[3] > 0.01, "{more:?}");
    assert!(less[1] > 0.01 && less[3] < -0.01, "{less:?}");
    assert!(
        more[2].abs() < 0.005 && less[2].abs() < 0.005,
        "the pivot moved"
    );
}

#[test]
fn each_zone_slider_moves_its_own_zone_the_most_in_its_own_direction() {
    let source = wedge();
    // (slider, band of its peak, sign of its +100 shift)
    let cases: [(&str, usize, f32); 4] = [
        ("blacks", 0, 1.0),
        ("shadows", 1, 1.0),
        ("highlights", 3, 1.0),
        ("whites", 4, 1.0),
    ];
    for (slider, own, sign) in cases {
        for amount in [-100.0, 100.0] {
            let settings = edit(|s| match slider {
                "blacks" => s.tone.blacks = amount,
                "shadows" => s.tone.shadows = amount,
                "highlights" => s.tone.highlights = amount,
                _ => s.tone.whites = amount,
            });
            let moved = shifts(&source, &settings);
            if slider == "whites" && amount > 0.0 {
                // Its peak sits a hair below white: pushed up, it can only
                // reach the clipping point. (The curve's v^4 reach also
                // lifts the highlights peak by about 0.11.)
                assert!(bands(&source, &settings)[own] >= 0.9999, "{moved:?}");
                assert!(moved[own] > 0.0 && moved[3] > 0.05, "{moved:?}");
                continue;
            }
            let expected = sign * amount.signum();
            assert!(
                moved[own] * expected > 0.004,
                "{slider} {amount}: its own zone moved {:?}",
                moved
            );
            for (band, shift) in moved.iter().enumerate() {
                if band != own && !(slider == "whites" && band == 3) {
                    assert!(
                        shift.abs() < moved[own].abs(),
                        "{slider} {amount}: band {band} moved more than its own: {moved:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn the_tone_curve_bends_the_tones_it_is_drawn_through() {
    let source = wedge();
    let darks = shifts(&source, &edit(|s| s.tone_curve.parametric.darks = 80.0));
    assert!(darks[1] > 0.01 && darks[4].abs() < 0.002, "{darks:?}");
    let lifted = shifts(
        &source,
        &edit(|s| {
            s.tone_curve.rgb = PointCurve::new([
                CurvePoint::new(0.0, 0.0),
                CurvePoint::new(0.5, 0.6),
                CurvePoint::new(1.0, 1.0),
            ]);
        }),
    );
    assert!(lifted[2] > 0.05, "{lifted:?}");
    // The red curve alone reddens a grey (in working space, so the sRGB
    // green and blue move a little too).
    let red = edit(|s| {
        s.tone_curve.red = PointCurve::new([
            CurvePoint::new(0.0, 0.0),
            CurvePoint::new(0.5, 0.62),
            CurvePoint::new(1.0, 1.0),
        ]);
    });
    let image = render(&source, &red, full(&source, &red));
    let [r, g, b] = mean(&image, 2 * BAND_WIDTH + 20, 40, 40, 40);
    assert!(r > g.max(b) + 30.0, "{:?}", [r, g, b]);
}

/// Perceptual value of the green channel at `(x, y)` of a 16-bit render.
fn value_at(image: &photo_core::image::Rgb16Image, x: u32, y: u32) -> f32 {
    RgbSpace::Srgb
        .decode(f32::from(image.pixel(x, y)[1]) / 65535.0)
        .powf(1.0 / 2.2)
}

#[test]
fn a_flat_patch_smaller_than_the_base_radius_stays_flat() {
    // A bright square on a dark ground, smaller than the edge-aware base's
    // window: Highlights must pull the square down evenly, not darken its
    // middle more than its rim (nor lift a dark square's rim with Shadows).
    let square = |x: u32, y: u32, inside: f32, outside: f32| {
        let within = (280..320).contains(&x) && (180..220).contains(&y);
        [if within { inside } else { outside }.powf(2.2); 3]
    };
    let bright = raster_linear(600, 400, |x, y| square(x, y, 0.85, 0.25));
    let dark = raster_linear(600, 400, |x, y| square(x, y, 0.3, 0.8));
    for (source, settings) in [
        (&bright, edit(|s| s.tone.highlights = -100.0)),
        (&dark, edit(|s| s.tone.shadows = 100.0)),
    ] {
        let image = render16(source, &settings, full(source, &settings));
        let centre = value_at(&image, 300, 200);
        let before = render16(source, &DevelopSettings::default(), full(source, &settings));
        assert!(
            (centre - value_at(&before, 300, 200)).abs() > 0.05,
            "the square did not move"
        );
        for (x, y) in [(283, 200), (300, 183), (316, 216)] {
            let rim = value_at(&image, x, y);
            assert!(
                (rim - centre).abs() < 0.01,
                "rim ({x}, {y}) {rim} vs centre {centre}"
            );
        }
    }
}

#[test]
fn recovered_headroom_has_no_halo_along_an_edge() {
    // A camera source: the top half two stops above nominal white, the
    // bottom half mid grey. Highlights -100 must pull the top down evenly
    // up to the edge, not leave a bright band along it.
    let source =
        crate::fixtures::camera_source(600, 400, 1.6, |_, y| [if y < 200 { 0.8 } else { 0.06 }; 3]);
    let settings = edit(|s| s.tone.highlights = -100.0);
    let image = render16(&source, &settings, full(&source, &settings));
    let far = value_at(&image, 300, 100);
    for y in [150, 180, 192, 196] {
        let near = value_at(&image, 300, y);
        assert!(
            (near - far).abs() < 0.01,
            "row {y}: {near} vs {far} far from the edge"
        );
    }
}

#[test]
fn highlights_and_shadows_recover_a_camera_sources_tones() {
    let (width, height) = (480, 320);
    let source = camera_scene(width, height);
    let sky = (width / 10, height / 4, width / 5, height / 8);
    let soil = (width / 10, height * 59 / 100, width / 5, height / 30);
    let measure = |settings: &DevelopSettings| {
        let image = render(&source, settings, full(&source, settings));
        let at = |(x, y, w, h): (u32, u32, u32, u32)| mean(&image, x, y, w, h)[1];
        (at(sky), at(soil))
    };
    let (sky0, soil0) = measure(&DevelopSettings::default());
    let (sky1, _) = measure(&edit(|s| s.tone.highlights = -100.0));
    let (_, soil1) = measure(&edit(|s| s.tone.shadows = 100.0));
    assert!(sky1 < sky0 - 2.0, "highlights -100: sky {sky0} -> {sky1}");
    assert!(soil1 > soil0 + 2.0, "shadows +100: soil {soil0} -> {soil1}");
}

#[test]
fn vivid_has_more_contrast_and_colour_than_neutral() {
    let (width, height) = (480, 320);
    let source = camera_scene(width, height);
    let red = &patches(width, height)[0];
    let look = |profile: Profile| {
        let settings = edit(|s| s.profile = profile);
        let image = render(&source, &settings, full(&source, &settings));
        let [r, g, b] = mean(&image, red.x, red.y, red.width, red.height);
        let soil = mean(
            &image,
            width / 10,
            height * 59 / 100,
            width / 5,
            height / 30,
        )[1];
        let sky = mean(&image, width / 10, height / 4, width / 5, height / 8)[1];
        (r - g.max(b), sky - soil)
    };
    let (neutral_colour, neutral_range) = look(Profile::Neutral);
    let (vivid_colour, vivid_range) = look(Profile::Vivid);
    assert!(
        vivid_colour > neutral_colour + 3.0,
        "{vivid_colour} vs {neutral_colour}"
    );
    assert!(
        vivid_range > neutral_range + 2.0,
        "{vivid_range} vs {neutral_range}"
    );
}
