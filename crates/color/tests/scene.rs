//! Contracts of exposure and base rendering (DESIGN §5.4). Highlights and
//! Shadows, which also act in this stage, have their own file (`zones.rs`).

use photo_color::base::{
    RAW_EXPOSURE_BIAS_EV, SceneDelta, SceneToDisplay, base_curve, exposure_gain, raster_exposure,
    rgb_tone, soft_clip,
};
use photo_core::colorimetry::RgbSpace;
use photo_core::develop::{Profile, Tone};
use photo_core::image::{CameraColor, SourceColor};

const RASTER: SourceColor = SourceColor::Rgb(RgbSpace::Srgb);

/// Stage 5 with only the Exposure slider set.
fn exposed(profile: Profile, exposure: f32, source: &SourceColor) -> SceneToDisplay {
    let tone = Tone {
        exposure,
        ..Default::default()
    };
    SceneToDisplay::new(profile, &tone, source)
}

/// A local exposure offset in stops.
fn local(exposure: f32) -> SceneDelta {
    SceneDelta {
        exposure,
        ..Default::default()
    }
}

/// With Highlights and Shadows at zero the base is not read; any value does.
const NO_BASE: f32 = 0.0;

fn camera(baseline_exposure: f32) -> SourceColor {
    SourceColor::Camera(CameraColor {
        xyz_to_camera: [
            [0.6640, -0.1847, -0.0503],
            [-0.5238, 1.3010, 0.2474],
            [-0.0993, 0.1673, 0.6527],
        ],
        as_shot_gains: [2.31, 1.0, 1.58],
        baseline_exposure,
    })
}

/// Where the sensor clips, in the scene units the base curve sees.
fn clip_point() -> f32 {
    2.0f32.powf(RAW_EXPOSURE_BIAS_EV)
}

const CURVED: [Profile; 3] = [Profile::Standard, Profile::Neutral, Profile::Vivid];

#[test]
fn raster_exposure_at_zero_is_exactly_the_identity() {
    let stage = exposed(Profile::Standard, 0.0, &RASTER);
    assert!(stage.is_identity() && !stage.needs_base());
    for rgb in [
        [0.0, 0.5, 1.0],
        [0.123_456_7, 0.2, 0.999_999],
        [1.0e-6, 3.0, 16.0],
    ] {
        for base in [NO_BASE, 0.7, f32::NAN] {
            assert_eq!(stage.apply(rgb, base), rgb);
            assert_eq!(stage.apply_with_delta(rgb, base, &local(0.0)), rgb);
        }
    }
    assert!(!exposed(Profile::Standard, 0.01, &RASTER).is_identity());
    assert!(!exposed(Profile::Standard, 0.0, &camera(0.0)).is_identity());
}

#[test]
fn negative_raster_exposure_is_a_pure_gain() {
    let gain = exposure_gain(-1.5);
    assert!((gain - 0.353_553_4).abs() < 1e-6);
    let out = raster_exposure([1.0, 0.5, 0.25], gain);
    assert_eq!(out, [gain, 0.5 * gain, 0.25 * gain]);
}

#[test]
fn the_raster_shoulder_is_a_pure_gain_below_its_knee_and_lands_on_white() {
    for ev in [0.25, 1.0, 2.5, 5.0] {
        let gain = exposure_gain(ev);
        // The knee sits at 1/gain after the gain, i.e. 1/gain^2 before it.
        let below = 0.9 / (gain * gain);
        assert_eq!(raster_exposure([below; 3], gain), [below * gain; 3]);
        let white = raster_exposure([1.0; 3], gain);
        assert!(
            (white[0] - 1.0).abs() < 2e-6,
            "{ev} EV: white landed on {}",
            white[0]
        );
    }
}

#[test]
fn the_raster_response_is_strictly_increasing_in_pixel_value() {
    for ev in [0.01, 0.25, 1.0, 2.5, 5.0] {
        let gain = exposure_gain(ev);
        let mut previous = -1.0f32;
        for step in 0..=1024 {
            let value = raster_exposure([step as f32 / 1024.0; 3], gain)[0];
            assert!(value > previous, "{ev} EV: not increasing at step {step}");
            assert!(
                value - previous.max(0.0) < 0.05,
                "{ev} EV: jump at step {step}"
            );
            previous = value;
        }
    }
}

#[test]
fn the_raster_response_is_continuous_and_monotonic_in_exposure() {
    for pixel in [0.02, 0.18, 0.5, 0.8, 0.97, 1.0] {
        let mut previous = 0.0f32;
        for step in 0..=2000 {
            let ev = -5.0 + step as f32 * 0.005;
            let value = raster_exposure([pixel; 3], exposure_gain(ev))[0];
            assert!(
                value >= previous - 1e-6,
                "pixel {pixel}: response fell at {ev} EV"
            );
            assert!(
                value - previous < 0.02 || step == 0,
                "pixel {pixel}: jump at {ev} EV"
            );
            previous = value;
        }
    }
    // Arbitrarily close to zero exposure is arbitrarily close to identity.
    let nearly = raster_exposure([0.3, 0.6, 0.95], exposure_gain(1.0e-4));
    for (out, original) in nearly.iter().zip([0.3, 0.6, 0.95]) {
        assert!((out - original).abs() < 1e-4);
    }
}

#[test]
fn base_curves_hit_their_anchors() {
    for profile in CURVED {
        assert_eq!(base_curve(profile, 0.0), 0.0);
        let grey = base_curve(profile, 0.18);
        assert!(
            (0.18..=0.20).contains(&grey),
            "{profile:?}: middle grey renders at {grey}"
        );
        let clip = base_curve(profile, clip_point());
        assert!(
            (clip - 0.99).abs() < 0.003,
            "{profile:?}: the clip point renders at {clip}"
        );
    }
}

#[test]
fn base_curves_are_increasing_and_never_exceed_one() {
    for profile in Profile::ALL {
        let mut previous = -1.0f32;
        for step in 0..=4000 {
            let x = step as f32 / 1000.0;
            let y = base_curve(profile, x);
            assert!(y < 1.0, "{profile:?} reached {y} at {x}");
            assert!(y > previous, "{profile:?} is not increasing at {x}");
            previous = y;
        }
        for x in [8.0, 64.0, 1.0e4, 1.0e9, f32::MAX] {
            let y = base_curve(profile, x);
            assert!(y <= 1.0 && y >= previous, "{profile:?} at {x}: {y}");
        }
    }
}

#[test]
fn base_curves_have_a_toe_and_a_shoulder() {
    for profile in CURVED {
        let slope = |a: f32, b: f32| (base_curve(profile, b) - base_curve(profile, a)) / (b - a);
        let toe = slope(0.0, 0.01);
        let mid = slope(0.15, 0.21);
        let shoulder = slope(1.5, clip_point());
        assert!(toe < mid, "{profile:?}: toe {toe} vs mid {mid}");
        assert!(
            shoulder < 0.25 * mid,
            "{profile:?}: shoulder {shoulder} vs mid {mid}"
        );
    }
}

#[test]
fn profiles_are_ordered_by_contrast_and_saturation() {
    // Contrast: the log-log slope through middle grey.
    let contrast = |profile| {
        (base_curve(profile, 0.22) / base_curve(profile, 0.145)).ln() / (0.22f32 / 0.145).ln()
    };
    let (neutral, standard, vivid) = (
        contrast(Profile::Neutral),
        contrast(Profile::Standard),
        contrast(Profile::Vivid),
    );
    assert!(
        neutral < standard && standard < vivid,
        "{neutral} {standard} {vivid}"
    );
    assert!(neutral > 1.0 && vivid < 1.6);

    let source = camera(0.0);
    let chroma = |profile| {
        let out = exposed(profile, 0.0, &source).apply([0.12, 0.08, 0.05], NO_BASE);
        (out[0] - out[2]) / out[1]
    };
    assert!(chroma(Profile::Neutral) < chroma(Profile::Standard));
    assert!(chroma(Profile::Standard) < chroma(Profile::Vivid));
}

#[test]
fn the_camera_stage_applies_the_bias_then_the_curve() {
    for profile in CURVED {
        for baseline in [0.0, 0.5] {
            let stage = exposed(profile, 0.3, &camera(baseline));
            let gain = exposure_gain(0.3 + RAW_EXPOSURE_BIAS_EV + baseline);
            for raw in [0.0, 0.004, 0.05, 0.1, 0.4, 1.0, 2.3] {
                let out = stage.apply([raw; 3], NO_BASE);
                // The base handed to the guided filter is the perceptual
                // value of exactly this, read from luminance alone.
                let base = stage.base_value([raw; 3], 0.0);
                assert!(
                    (base - out[1].powf(1.0 / 2.2)).abs() < 2e-5,
                    "{profile:?} at {raw}"
                );
                let expected = base_curve(profile, raw * gain);
                assert!(
                    (out[0] - expected).abs() < 2e-5,
                    "{profile:?} at {raw}: {} vs {expected}",
                    out[0]
                );
                assert!(
                    out[0] == out[1] && out[1] == out[2],
                    "neutral in, neutral out"
                );
            }
        }
    }
    // A raw value of 0.1 is middle grey after the +0.85 EV bias.
    let grey =
        exposed(Profile::Standard, 0.0, &camera(0.0)).apply([0.18 / clip_point(); 3], NO_BASE);
    assert!((0.18..=0.20).contains(&grey[0]), "{grey:?}");
}

#[test]
fn the_linear_profile_is_a_gain_with_a_soft_clip() {
    let stage = exposed(Profile::Linear, 0.0, &camera(0.7));
    assert_eq!(
        stage.apply([0.1, 0.4, 0.75], NO_BASE),
        [0.1, 0.4, 0.75],
        "no bias, no curve below the knee"
    );
    let doubled = exposed(Profile::Linear, 1.0, &camera(0.0)).apply([0.1, 0.2, 0.3], NO_BASE);
    assert_eq!(doubled, [0.2, 0.4, 0.6]);
    assert_eq!(soft_clip(0.5), 0.5);
    let mut previous = 0.0;
    for step in 0..=2000 {
        let y = soft_clip(step as f32 / 100.0);
        assert!(y < 1.0 && y >= previous);
        assert!(y - previous < 0.011, "soft clip jumps at step {step}");
        previous = y;
    }
    assert!(soft_clip(20.0) > 0.98);
}

#[test]
fn more_exposure_is_brighter_for_both_source_kinds() {
    for source in [RASTER, camera(0.0)] {
        let mut previous = [0.0f32; 3];
        for ev in [-5.0, -2.0, -0.5, 0.0, 0.5, 2.0, 5.0] {
            let out = exposed(Profile::Standard, ev, &source).apply([0.02, 0.05, 0.03], NO_BASE);
            assert!(
                out.iter().zip(previous).all(|(now, before)| *now > before),
                "{ev} EV: {out:?}"
            );
            previous = out;
        }
    }
}

#[test]
fn a_local_exposure_offset_equals_the_same_global_exposure() {
    for source in [RASTER, camera(0.2)] {
        for profile in Profile::ALL {
            for rgb in [[0.02, 0.05, 0.03], [0.6, 0.3, 0.9], [0.4, 0.4, 0.4]] {
                let global = exposed(profile, 1.25, &source).apply(rgb, NO_BASE);
                let stage = exposed(profile, 0.5, &source);
                let masked = stage.apply_with_delta(rgb, NO_BASE, &local(0.75));
                for (a, b) in global.iter().zip(masked) {
                    assert!(
                        (a - b).abs() < 1e-5,
                        "{profile:?}: {global:?} vs {masked:?}"
                    );
                }
                // The base of a masked pixel includes its local exposure.
                let global_base = exposed(profile, 1.25, &source).base_value(rgb, 0.0);
                assert!((stage.base_value(rgb, 0.75) - global_base).abs() < 1e-5);
            }
        }
    }
}

#[test]
fn the_rgb_tone_method_preserves_channel_order_and_neutrals() {
    let curve = |x: f32| x / (x + 0.5);
    for rgb in [
        [0.9, 0.5, 0.1],
        [0.1, 0.5, 0.9],
        [0.5, 0.9, 0.1],
        [0.3, 0.3, 0.8],
        [2.0, 0.0, 0.7],
    ] {
        let out = rgb_tone(rgb, curve);
        for i in 0..3 {
            for j in 0..3 {
                if rgb[i] > rgb[j] {
                    assert!(out[i] > out[j], "{rgb:?} -> {out:?}");
                }
                if rgb[i] == rgb[j] {
                    assert_eq!(out[i], out[j]);
                }
            }
        }
        let (hi, lo) = (
            rgb.iter().cloned().fold(0.0, f32::max),
            rgb.iter().cloned().fold(9.0, f32::min),
        );
        let (out_hi, out_lo) = (
            out.iter().cloned().fold(0.0, f32::max),
            out.iter().cloned().fold(9.0, f32::min),
        );
        assert!((out_hi - curve(hi)).abs() < 1e-6 && (out_lo - curve(lo)).abs() < 1e-6);
        // The middle channel keeps its relative position.
        let mid = rgb.iter().sum::<f32>() - hi - lo;
        let out_mid = out.iter().sum::<f32>() - out_hi - out_lo;
        assert!(((out_mid - out_lo) / (out_hi - out_lo) - (mid - lo) / (hi - lo)).abs() < 1e-5);
    }
    assert_eq!(rgb_tone([0.4; 3], curve), [curve(0.4); 3]);
}

#[test]
fn every_input_gives_a_finite_non_negative_result() {
    let inputs = [
        [0.0, 0.0, 0.0],
        [1.0e-6, 1.0e-6, 1.0e-6],
        [16.0, 0.0, 1.0e-6],
        [-0.01, 0.2, 0.5],
        [-1.0e-4, -1.0e-4, -1.0e-4],
        [0.3, f32::NAN, 0.1],
        [1.0, 1.0, 1.0],
    ];
    for source in [RASTER, camera(0.0), camera(-1.0)] {
        for profile in Profile::ALL {
            for ev in [-5.0, 0.0, 0.01, 5.0] {
                for zone in [0.0, -100.0, 100.0] {
                    let tone = Tone {
                        exposure: ev,
                        highlights: zone,
                        shadows: -zone,
                        ..Default::default()
                    };
                    let stage = SceneToDisplay::new(profile, &tone, &source);
                    for rgb in inputs {
                        for delta in [0.0, -4.0, 4.0, f32::NAN] {
                            let base = stage.base_value(rgb, delta);
                            assert!(
                                base.is_finite() && base >= 0.0,
                                "{profile:?} {rgb:?}: {base}"
                            );
                            let masked = SceneDelta {
                                exposure: delta,
                                highlights: delta * 25.0,
                                shadows: -delta * 25.0,
                            };
                            for base in [base, 0.0, 1.0, 7.0, -1.0, f32::NAN] {
                                for out in [
                                    stage.apply(rgb, base),
                                    stage.apply_with_delta(rgb, base, &masked),
                                ] {
                                    assert!(
                                        out.iter().all(|v| v.is_finite() && *v >= 0.0),
                                        "{profile:?} {tone:?}, {rgb:?} + {delta} at {base}: {out:?}"
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}
