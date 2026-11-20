//! Contracts of colour grading and calibration (DESIGN §5.6).

use photo_color::color::{ColorStage, calibration_matrix, grading_weights};
use photo_color::oklab::working_to_oklab;
use photo_core::colorimetry::{IDENTITY, RgbSpace, luminance, mat3_apply, rgb_to_rgb};
use photo_core::develop::{Calibration, DevelopSettings, GradeWheel};

fn chroma(rgb: [f32; 3]) -> f32 {
    let lab = working_to_oklab(rgb);
    lab[1].hypot(lab[2])
}

fn stage(edit: impl FnOnce(&mut DevelopSettings)) -> ColorStage {
    let mut settings = DevelopSettings::default();
    edit(&mut settings);
    ColorStage::new(&settings)
}

fn assert_close(a: [f32; 3], b: [f32; 3], tolerance: f32) {
    for (x, y) in a.iter().zip(b) {
        assert!((x - y).abs() <= tolerance, "{a:?} vs {b:?}");
    }
}

#[test]
fn grading_masks_are_a_partition_of_unity() {
    for blending in [0.0, 50.0, 100.0] {
        for balance in [-100.0, 0.0, 100.0] {
            for step in 0..=100 {
                let lightness = step as f32 / 100.0;
                let weights = grading_weights(lightness, blending, balance);
                assert!((weights.iter().sum::<f32>() - 1.0).abs() < 1e-6);
                assert!(
                    weights.iter().all(|w| (-1e-6..=1.0 + 1e-6).contains(w)),
                    "{weights:?}"
                );
            }
            let dark = grading_weights(0.02, blending, balance);
            let bright = grading_weights(0.98, blending, balance);
            assert!(
                dark[0] > 0.9 && bright[2] > 0.9,
                "{blending}/{balance}: {dark:?} {bright:?}"
            );
        }
    }
    // More blending, more overlap; positive balance hands tones to the highlights.
    let narrow = grading_weights(0.5, 0.0, 0.0);
    let wide = grading_weights(0.5, 100.0, 0.0);
    assert!(
        narrow[1] > 0.99 && wide[0] > 0.1 && wide[2] > 0.1,
        "{narrow:?} {wide:?}"
    );
    assert!(grading_weights(0.5, 50.0, 80.0)[2] > grading_weights(0.5, 50.0, -80.0)[2]);
    assert!(grading_weights(0.5, 50.0, -80.0)[0] > grading_weights(0.5, 50.0, 80.0)[0]);
}

#[test]
fn grading_tints_the_range_its_wheel_names() {
    let wheel = |hue: f32| GradeWheel {
        hue,
        saturation: 80.0,
        luminance: 0.0,
    };
    let (dark, mid, bright) = ([0.01f32; 3], [0.18f32; 3], [0.85f32; 3]);
    let tint =
        |stage: &ColorStage, rgb: [f32; 3]| chroma(stage.apply(rgb)) / working_to_oklab(rgb)[0];

    let shadows = stage(|s| s.color_grading.shadows = wheel(220.0));
    assert!(tint(&shadows, dark) > 4.0 * tint(&shadows, bright));
    let highlights = stage(|s| s.color_grading.highlights = wheel(40.0));
    assert!(tint(&highlights, bright) > 4.0 * tint(&highlights, dark));
    let midtones = stage(|s| s.color_grading.midtones = wheel(120.0));
    assert!(tint(&midtones, mid) > 2.0 * tint(&midtones, dark).max(tint(&midtones, bright)));
    let global = stage(|s| s.color_grading.global = wheel(300.0));
    assert!(
        tint(&global, dark) > 0.01
            && (tint(&global, dark) / tint(&global, bright) - 1.0).abs() < 0.2
    );

    // The tint has the hue of the wheel: 0 is red, 120 green, 240 blue.
    for (degrees, strongest) in [(0.0, 0), (120.0, 1), (240.0, 2)] {
        let out = stage(|s| s.color_grading.global = wheel(degrees)).apply(mid);
        let srgb_out = mat3_apply(&rgb_to_rgb(RgbSpace::Rec2020, RgbSpace::Srgb), out);
        for channel in 0..3 {
            if channel != strongest {
                assert!(
                    srgb_out[strongest] > srgb_out[channel],
                    "wheel at {degrees}: {srgb_out:?}"
                );
            }
        }
    }
    // A wheel with hue but no saturation or luminance does nothing.
    assert!(stage(|s| s.color_grading.shadows.hue = 200.0).is_identity());
}

#[test]
fn grading_a_neutral_keeps_it_neutral_only_where_no_tint_applies() {
    // Split toning on a black-and-white picture: the shadows take the tint,
    // the highlights stay exactly neutral.
    let toned = stage(|s| {
        s.treatment = photo_core::develop::Treatment::BlackAndWhite;
        s.color_grading.shadows = GradeWheel {
            hue: 220.0,
            saturation: 60.0,
            luminance: 0.0,
        };
    });
    let dark = toned.apply([0.02, 0.01, 0.015]);
    assert!(dark[2] > dark[0], "blue-toned shadows: {dark:?}");
    let bright = toned.apply([0.9, 0.7, 0.8]);
    assert!(
        bright[0] == bright[1] && bright[1] == bright[2],
        "{bright:?}"
    );
}

#[test]
fn grading_luminance_brightens_and_darkens_its_range() {
    let lift = |amount: f32| {
        stage(move |s| {
            s.color_grading.shadows.luminance = amount;
        })
    };
    let dark = [0.01f32, 0.012, 0.009];
    assert!(luminance(lift(80.0).apply(dark)) > 1.2 * luminance(dark));
    assert!(luminance(lift(-80.0).apply(dark)) < 0.8 * luminance(dark));
    let bright = [0.85f32, 0.8, 0.9];
    assert_close(lift(80.0).apply(bright), bright, 5e-3);
}

#[test]
fn calibration_keeps_neutrals_and_moves_its_primary() {
    for edit in [
        (|c: &mut Calibration| c.red_hue = 100.0) as fn(&mut Calibration),
        |c| c.green_saturation = -100.0,
        |c| c.blue_hue = -100.0,
        |c| {
            c.red_saturation = 100.0;
            c.green_hue = 100.0;
            c.blue_saturation = 100.0;
        },
    ] {
        let mut calibration = Calibration::default();
        edit(&mut calibration);
        let white = mat3_apply(&calibration_matrix(&calibration), [1.0; 3]);
        assert_close(white, [1.0; 3], 1e-6);
    }
    assert_eq!(calibration_matrix(&Calibration::default()), IDENTITY);

    let red = [0.5, 0.05, 0.05];
    let orange = stage(|s| s.calibration.red_hue = 100.0).apply(red);
    let magenta = stage(|s| s.calibration.red_hue = -100.0).apply(red);
    assert!(
        orange[1] / orange[2] > 1.5 && magenta[2] / magenta[1] > 1.5,
        "{orange:?} {magenta:?}"
    );
    assert!(
        chroma(stage(|s| s.calibration.red_saturation = 100.0).apply(red)) > 1.05 * chroma(red)
    );
    assert!(
        chroma(stage(|s| s.calibration.red_saturation = -100.0).apply(red)) < 0.9 * chroma(red)
    );
    let green = [0.05, 0.5, 0.05];
    let aqua = stage(|s| s.calibration.green_hue = 100.0).apply(green);
    assert!(aqua[2] > aqua[0], "{aqua:?}");
    let blue = [0.05, 0.05, 0.5];
    let purple = stage(|s| s.calibration.blue_hue = 100.0).apply(blue);
    assert!(purple[0] > purple[1], "{purple:?}");
}

#[test]
fn the_shadow_tint_colours_only_the_shadows() {
    let magenta = stage(|s| s.calibration.shadows_tint = 100.0);
    let green = stage(|s| s.calibration.shadows_tint = -100.0);
    let dark = [0.01f32; 3];
    let (m, g) = (magenta.apply(dark), green.apply(dark));
    assert!(m[1] < m[0] && m[1] < m[2], "{m:?}");
    assert!(g[1] > g[0] && g[1] > g[2], "{g:?}");
    assert!(
        (luminance(m) / luminance(dark) - 1.0).abs() < 0.01,
        "the tint keeps luminance"
    );
    assert_eq!(magenta.apply([0.7; 3]), [0.7; 3]);
}
