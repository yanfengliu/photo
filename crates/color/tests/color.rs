//! Contracts of the colour stage (DESIGN §5.6): Oklab, saturation, vibrance,
//! the colour mixer and black and white. Grading and calibration have their
//! own file.

use photo_color::color::{ColorStage, band_centre_degrees, band_weights};
use photo_color::oklab::{hue_degrees, oklab_to_working, working_to_oklab};
use photo_core::colorimetry::{RgbSpace, luminance, mat3_apply, rgb_to_rgb};
use photo_core::develop::{Calibration, ColorBand, DevelopSettings, GradeWheel, Treatment};

/// An sRGB-encoded colour as linear working RGB.
fn srgb(encoded: [f32; 3]) -> [f32; 3] {
    let linear = encoded.map(|c| RgbSpace::Srgb.decode(c));
    mat3_apply(&rgb_to_rgb(RgbSpace::Srgb, RgbSpace::Rec2020), linear)
}

/// The reference colour of each band, at half strength so there is room to
/// brighten it.
fn band_colour(band: ColorBand) -> [f32; 3] {
    let encoded = match band {
        ColorBand::Red => [1.0, 0.0, 0.0],
        ColorBand::Orange => [1.0, 0.5, 0.0],
        ColorBand::Yellow => [1.0, 1.0, 0.0],
        ColorBand::Green => [0.0, 1.0, 0.0],
        ColorBand::Aqua => [0.0, 1.0, 1.0],
        ColorBand::Blue => [0.0, 0.0, 1.0],
        ColorBand::Purple => [0.5, 0.0, 1.0],
        ColorBand::Magenta => [1.0, 0.0, 1.0],
    };
    srgb(encoded).map(|c| c * 0.4)
}

fn chroma(rgb: [f32; 3]) -> f32 {
    let lab = working_to_oklab(rgb);
    lab[1].hypot(lab[2])
}

fn hue(rgb: [f32; 3]) -> f32 {
    let lab = working_to_oklab(rgb);
    hue_degrees(lab[1], lab[2])
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

const SAMPLES: [[f32; 3]; 6] = [
    [0.25, 0.5, 0.75],
    [0.8, 0.1, 0.05],
    [0.02, 0.03, 0.01],
    [0.6, 0.55, 0.2],
    [1.5, 0.2, 3.0],
    [0.4, 0.4, 0.4],
];

#[test]
fn oklab_round_trips_and_matches_published_values() {
    for rgb in SAMPLES {
        let back = oklab_to_working(working_to_oklab(rgb));
        assert_close(back, rgb, 3e-6 * rgb.iter().cloned().fold(1.0, f32::max));
    }
    // Ottosson's reference values for the sRGB primaries.
    for (encoded, expected) in [
        ([1.0, 1.0, 1.0], [1.0, 0.0, 0.0]),
        ([1.0, 0.0, 0.0], [0.627_955, 0.224_863, 0.125_846]),
        ([0.0, 1.0, 0.0], [0.866_440, -0.233_888, 0.179_498]),
        ([0.0, 0.0, 1.0], [0.452_014, -0.032_457, -0.311_528]),
    ] {
        assert_close(working_to_oklab(srgb(encoded)), expected, 6e-4);
    }
    for level in [1.0e-4, 0.18, 1.0, 7.0] {
        let lab = working_to_oklab([level; 3]);
        assert!(
            lab[1].abs() < 2e-6 && lab[2].abs() < 2e-6,
            "grey {level} has chroma: {lab:?}"
        );
    }
}

#[test]
fn t1_neutral_settings_are_exactly_the_identity() {
    let neutral = stage(|_| {});
    assert!(neutral.is_identity());
    for rgb in SAMPLES {
        assert_eq!(neutral.apply(rgb), rgb);
        assert_eq!(neutral.apply_with_saturation(rgb, 0.0), rgb);
    }
    assert!(!stage(|s| s.presence.saturation = 1.0).is_identity());
    assert!(!stage(|s| s.treatment = Treatment::BlackAndWhite).is_identity());
}

#[test]
fn greys_are_untouched_by_the_hue_selective_controls() {
    let busy = stage(|s| {
        s.presence.vibrance = 80.0;
        s.presence.saturation = 60.0;
        s.color_mixer.hue = [100.0, -100.0, 100.0, -100.0, 100.0, -100.0, 100.0, -100.0];
        s.color_mixer.saturation = [100.0, -100.0, 50.0, -50.0, 100.0, -100.0, 50.0, -50.0];
        s.color_mixer.luminance = [100.0, -100.0, 100.0, -100.0, 100.0, -100.0, 100.0, -100.0];
        s.calibration.red_hue = 60.0;
        s.calibration.green_saturation = -40.0;
        s.calibration.blue_hue = -80.0;
    });
    for level in [0.001, 0.18, 0.5, 1.0, 3.0] {
        let out = busy.apply([level; 3]);
        assert_close(out, [level; 3], 2e-5 * level.max(0.05));
    }
}

#[test]
fn saturation_scales_chroma_and_minus_100_is_exactly_grey() {
    let grey = stage(|s| s.presence.saturation = -100.0);
    for rgb in SAMPLES {
        let out = grey.apply(rgb);
        assert!(out[0] == out[1] && out[1] == out[2], "{rgb:?} -> {out:?}");
        assert!(out[0] > 0.0);
    }
    for rgb in &SAMPLES[..4] {
        let more = stage(|s| s.presence.saturation = 50.0).apply(*rgb);
        let less = stage(|s| s.presence.saturation = -50.0).apply(*rgb);
        assert!(chroma(more) > 1.2 * chroma(*rgb), "{rgb:?}");
        assert!(chroma(less) < 0.8 * chroma(*rgb), "{rgb:?}");
        assert!(
            (hue(less) - hue(*rgb)).abs() < 0.1,
            "saturation must not shift hue: {rgb:?}"
        );
        // A boost that stays inside the working gamut keeps the hue too; one
        // that leaves it is brought back toward its luminance instead.
        if more.iter().all(|channel| *channel > 1e-4) {
            assert!(
                (hue(more) - hue(*rgb)).abs() < 0.1,
                "saturation must not shift hue: {rgb:?}"
            );
        }
    }
    let boosted = stage(|s| s.presence.saturation = 50.0).apply(SAMPLES[0]);
    assert!(
        boosted.iter().all(|channel| *channel > 1e-4),
        "the in-gamut case must be exercised"
    );
}

#[test]
fn a_local_saturation_offset_equals_the_same_global_saturation() {
    let local = stage(|s| s.presence.saturation = 20.0);
    let global = stage(|s| s.presence.saturation = 55.0);
    for rgb in SAMPLES {
        assert_close(
            local.apply_with_saturation(rgb, 35.0),
            global.apply(rgb),
            1e-6,
        );
    }
    // Stacked masks cannot push saturation below grey.
    let out = local.apply_with_saturation(SAMPLES[0], -500.0);
    assert!(out[0] == out[1] && out[1] == out[2]);
    let neutral = stage(|_| {});
    assert!(chroma(neutral.apply_with_saturation(SAMPLES[0], 40.0)) > chroma(SAMPLES[0]));
}

#[test]
fn vibrance_favours_muted_colours_and_protects_skin() {
    let vivid = srgb([0.9, 0.1, 0.1]);
    let muted = srgb([0.55, 0.45, 0.45]);
    let boost = stage(|s| s.presence.vibrance = 80.0);
    let gain = |rgb: [f32; 3]| chroma(boost.apply(rgb)) / chroma(rgb);
    assert!(gain(muted) > 1.2, "{}", gain(muted));
    assert!(gain(vivid) > 1.0 && gain(vivid) < gain(muted));

    // Two colours of similar chroma: a skin tone and a sky blue.
    let skin = srgb([0.82, 0.62, 0.50]);
    let sky = srgb([0.50, 0.62, 0.82]);
    assert!(
        gain(skin) > 1.0 && gain(skin) < gain(sky),
        "{} vs {}",
        gain(skin),
        gain(sky)
    );

    let cut = stage(|s| s.presence.vibrance = -80.0);
    assert!(chroma(cut.apply(muted)) < 0.6 * chroma(muted));
    // Negative vibrance never fully desaturates a saturated colour.
    assert!(chroma(stage(|s| s.presence.vibrance = -100.0).apply(vivid)) > 0.3 * chroma(vivid));
}

#[test]
fn band_weights_are_a_partition_of_unity_centred_on_the_reference_colours() {
    let mut previous = -1.0;
    for band in ColorBand::ALL {
        let centre = band_centre_degrees(band);
        assert!(
            centre > previous,
            "band centres must ascend in hue: {band:?}"
        );
        previous = centre;
        assert!((hue(band_colour(band)) - centre).abs() < 0.05, "{band:?}");
        let weights = band_weights(centre);
        assert!(
            (weights[band.index()] - 1.0).abs() < 1e-5,
            "{band:?}: {weights:?}"
        );
    }
    for step in 0..720 {
        let weights = band_weights(step as f32 * 0.5);
        assert!((weights.iter().sum::<f32>() - 1.0).abs() < 1e-5);
        assert!(weights.iter().all(|w| (0.0..=1.0).contains(w)));
        assert!(weights.iter().filter(|w| **w > 0.0).count() <= 2);
    }
}

#[test]
fn each_mixer_band_acts_on_its_own_colour() {
    // A moderately saturated patch exactly on a band's centre hue, well
    // inside the working gamut so that boosts are not limited by it.
    let patch = |band: ColorBand| {
        let angle = band_centre_degrees(band).to_radians();
        oklab_to_working([0.6, 0.08 * angle.cos(), 0.08 * angle.sin()])
    };
    for band in ColorBand::ALL {
        let colour = patch(band);
        let far = patch(ColorBand::ALL[(band.index() + 4) % 8]);
        let i = band.index();

        let drained = stage(|s| s.color_mixer.saturation[i] = -100.0);
        assert!(
            chroma(drained.apply(colour)) < 0.03 * chroma(colour),
            "{band:?}"
        );
        assert_close(drained.apply(far), far, 2e-5);
        assert!(
            chroma(stage(|s| s.color_mixer.saturation[i] = 60.0).apply(colour))
                > 1.3 * chroma(colour)
        );

        let lighter = stage(|s| s.color_mixer.luminance[i] = 80.0).apply(colour);
        let darker = stage(|s| s.color_mixer.luminance[i] = -80.0).apply(colour);
        assert!(luminance(lighter) > 1.2 * luminance(colour), "{band:?}");
        assert!(luminance(darker) < 0.8 * luminance(colour), "{band:?}");

        // Hue moves toward the next band for positive values and toward the
        // previous one for negative values, never past it.
        let next = band_centre_degrees(ColorBand::ALL[(i + 1) % 8]);
        let prev = band_centre_degrees(ColorBand::ALL[(i + 7) % 8]);
        let centre = band_centre_degrees(band);
        let turn = |from: f32, to: f32| (to - from + 540.0).rem_euclid(360.0) - 180.0;
        let up = turn(
            centre,
            hue(stage(|s| s.color_mixer.hue[i] = 100.0).apply(colour)),
        );
        let down = turn(
            centre,
            hue(stage(|s| s.color_mixer.hue[i] = -100.0).apply(colour)),
        );
        assert!(
            up > 3.0 && up < turn(centre, next),
            "{band:?}: +100 turned {up}"
        );
        assert!(
            down < -3.0 && down > turn(centre, prev),
            "{band:?}: -100 turned {down}"
        );
    }
}

#[test]
fn mixer_hue_shifts_never_fold_the_hue_circle() {
    // The worst case: neighbouring bands pulled toward each other.
    let folded =
        stage(|s| s.color_mixer.hue = [100.0, -100.0, 100.0, -100.0, 100.0, -100.0, 100.0, -100.0]);
    let mut previous: Option<f32> = None;
    let mut total = 0.0;
    for step in 0..=1440 {
        let angle = (step as f32 * 0.25).to_radians();
        let out = folded.apply(oklab_to_working([
            0.7,
            0.06 * angle.cos(),
            0.06 * angle.sin(),
        ]));
        let now = hue(out);
        if let Some(before) = previous {
            let advance = (now - before + 540.0).rem_euclid(360.0) - 180.0;
            assert!(
                advance > 0.0,
                "hue order reversed near {} degrees",
                step as f32 * 0.25
            );
            total += advance;
        }
        previous = Some(now);
    }
    assert!(
        (total - 360.0).abs() < 0.5,
        "one turn in, one turn out: {total}"
    );
}

#[test]
fn black_and_white_is_neutral_and_its_mix_follows_the_bands() {
    let plain = stage(|s| s.treatment = Treatment::BlackAndWhite);
    for rgb in SAMPLES {
        let out = plain.apply(rgb);
        assert!(out[0] == out[1] && out[1] == out[2], "{rgb:?} -> {out:?}");
    }
    for band in ColorBand::ALL {
        let colour = band_colour(band);
        let mixed = |amount: f32| {
            stage(|s| {
                s.treatment = Treatment::BlackAndWhite;
                s.color_mixer.bw_mix[band.index()] = amount;
                // The colour-only controls must not leak into black and white.
                s.presence.saturation = 100.0;
                s.color_mixer.saturation[band.index()] = 100.0;
            })
            .apply(colour)
        };
        let (dark, mid, light) = (mixed(-100.0), mixed(0.0), mixed(100.0));
        assert!(
            dark[0] < 0.8 * mid[0] && light[0] > 1.2 * mid[0],
            "{band:?}: {dark:?} {mid:?} {light:?}"
        );
        assert!(light[0] == light[1] && light[1] == light[2]);
        assert_close(mid, plain.apply(colour), 1e-6);
    }
    let lifted = stage(|s| {
        s.treatment = Treatment::BlackAndWhite;
        s.color_mixer.bw_mix = [100.0; 8];
    });
    assert_close(lifted.apply([0.3; 3]), [0.3; 3], 1e-5);
}

#[test]
fn t5_no_input_produces_nan_or_a_negative_value() {
    let inputs = [
        [0.0, 0.0, 0.0],
        [1.0e-6, 1.0e-6, 1.0e-6],
        [16.0, 0.0, 1.0e-6],
        [-0.01, 0.2, 0.5],
        [-1.0e-4, -1.0e-4, -1.0e-4],
        [0.3, f32::NAN, 0.1],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
    ];
    let wild = |sign: f32, treatment: Treatment| {
        stage(move |s| {
            s.treatment = treatment;
            s.presence.vibrance = 100.0 * sign;
            s.presence.saturation = 100.0 * sign;
            s.color_mixer.hue = [100.0 * sign; 8];
            s.color_mixer.saturation = [100.0 * sign; 8];
            s.color_mixer.luminance = [-100.0 * sign; 8];
            s.color_mixer.bw_mix = [100.0 * sign; 8];
            for wheel in [
                &mut s.color_grading.shadows,
                &mut s.color_grading.midtones,
                &mut s.color_grading.highlights,
                &mut s.color_grading.global,
            ] {
                *wheel = GradeWheel {
                    hue: 250.0,
                    saturation: 100.0,
                    luminance: -100.0 * sign,
                };
            }
            s.color_grading.blending = 0.0;
            s.calibration = Calibration {
                shadows_tint: 100.0 * sign,
                red_hue: 100.0 * sign,
                red_saturation: 100.0,
                green_hue: -100.0 * sign,
                green_saturation: 100.0,
                blue_hue: 100.0 * sign,
                blue_saturation: 100.0,
            };
        })
    };
    for sign in [-1.0, 1.0] {
        for treatment in [Treatment::Color, Treatment::BlackAndWhite] {
            let stage = wild(sign, treatment);
            for rgb in inputs {
                for delta in [0.0, 300.0, -300.0, f32::NAN] {
                    let out = stage.apply_with_saturation(rgb, delta);
                    assert!(
                        out.iter().all(|v| v.is_finite() && *v >= 0.0),
                        "{rgb:?}: {out:?}"
                    );
                }
            }
        }
    }
}
