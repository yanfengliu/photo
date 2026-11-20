//! Contracts of the tone stage (DESIGN §5.5, T1-T5).

use photo_color::tone::{
    BLACKS_PEAK, EndpointCurve, HIGHLIGHTS_PEAK, SHADOWS_PEAK, ToneCurves, ToneDelta, ToneStage,
    WHITES_PEAK, ZoneCurve, contrast_curve, decode_perceptual, encode_perceptual, tone_base_value,
};
use photo_core::colorimetry::luminance;
use photo_core::develop::{CurvePoint, PointCurve, Tone, ToneCurve};

fn stage(tone: Tone) -> ToneStage {
    ToneStage::new(&tone, &ToneCurve::default())
}

/// Perceptual response of a neutral pixel whose base is its own value.
fn respond(stage: &ToneStage, v: f32) -> f32 {
    let rgb = [decode_perceptual(v); 3];
    encode_perceptual(luminance(stage.apply(rgb, tone_base_value(rgb))))
}

fn with(edit: impl FnOnce(&mut Tone)) -> Tone {
    let mut tone = Tone::default();
    edit(&mut tone);
    tone
}

#[test]
fn t1_neutral_settings_are_the_identity() {
    let neutral = stage(Tone::default());
    assert!(neutral.is_identity());
    assert!(!neutral.needs_base());
    for rgb in [
        [0.0, 0.5, 1.0],
        [0.123_456_7, 0.2, 0.999_999],
        [1.0e-6, 3.0, 16.0],
    ] {
        assert_eq!(neutral.apply(rgb, 0.3), rgb);
        assert_eq!(
            neutral.apply_with_delta(rgb, 0.3, &ToneDelta::default()),
            rgb
        );
    }
    let curves = ToneCurves::new(&Tone::default(), &ToneCurve::default());
    for step in 0..=100 {
        let v = step as f32 / 100.0;
        assert_eq!(curves.eval(v), v);
    }
}

#[test]
fn the_perceptual_encoding_is_gamma_2_2_and_round_trips() {
    assert!((encode_perceptual(0.5) - 0.5f32.powf(1.0 / 2.2)).abs() < 2e-6);
    assert!((decode_perceptual(0.5) - 0.5f32.powf(2.2)).abs() < 1e-7);
    for step in 0..=2000 {
        let linear = (step as f32 / 500.0).powi(2);
        let exact = linear.powf(1.0 / 2.2);
        assert!((encode_perceptual(linear) - exact).abs() < 3e-6, "{linear}");
    }
    assert_eq!(encode_perceptual(0.0), 0.0);
    assert_eq!(encode_perceptual(-1.0), 0.0);
    assert!((encode_perceptual(100.0) - 100.0f32.powf(1.0 / 2.2)).abs() < 1e-4);
    assert!((tone_base_value([0.2, 0.2, 0.2]) - 0.2f32.powf(1.0 / 2.2)).abs() < 3e-6);
}

#[test]
fn contrast_pivots_on_perceptual_one_half() {
    for amount in [-100.0, -40.0, 30.0, 100.0] {
        assert!((contrast_curve(0.0, amount)).abs() < 1e-7);
        assert!((contrast_curve(0.5, amount) - 0.5).abs() < 1e-7);
        assert!((contrast_curve(1.0, amount) - 1.0).abs() < 1e-6);
        let slope = (contrast_curve(0.501, amount) - contrast_curve(0.499, amount)) / 0.002;
        assert!(
            (slope - 2.0f32.powf(amount / 100.0)).abs() < 0.01,
            "{amount}: slope {slope}"
        );
    }
    assert!(contrast_curve(0.75, 50.0) > 0.75 && contrast_curve(0.25, 50.0) < 0.25);
    assert!(contrast_curve(0.75, -50.0) < 0.75 && contrast_curve(0.25, -50.0) > 0.25);
    assert_eq!(contrast_curve(0.3, 0.0), 0.3);
}

#[test]
fn every_tone_slider_moves_pixels_in_its_documented_direction() {
    let neutral = stage(Tone::default());
    let brighter = |tone: Tone, v: f32| respond(&stage(tone), v) > respond(&neutral, v) + 0.01;
    let darker = |tone: Tone, v: f32| respond(&stage(tone), v) < respond(&neutral, v) - 0.01;

    assert!(brighter(with(|t| t.highlights = 60.0), HIGHLIGHTS_PEAK));
    assert!(darker(with(|t| t.highlights = -60.0), HIGHLIGHTS_PEAK));
    assert!(brighter(with(|t| t.shadows = 60.0), SHADOWS_PEAK));
    assert!(darker(with(|t| t.shadows = -60.0), SHADOWS_PEAK));
    assert!(brighter(with(|t| t.whites = 60.0), WHITES_PEAK));
    assert!(darker(with(|t| t.whites = -60.0), WHITES_PEAK));
    assert!(brighter(with(|t| t.blacks = 60.0), BLACKS_PEAK));
    assert!(darker(with(|t| t.blacks = -60.0), BLACKS_PEAK));
    assert!(brighter(with(|t| t.contrast = 60.0), 0.75));
    assert!(darker(with(|t| t.contrast = 60.0), 0.25));
    assert!(darker(with(|t| t.contrast = -60.0), 0.75));
    assert!(brighter(with(|t| t.contrast = -60.0), 0.25));

    // Whites above zero pushes the top end past display white (it clips at
    // output); below zero dulls it.
    assert!(respond(&stage(with(|t| t.whites = 100.0)), 1.0) > 1.1);
    assert!(respond(&stage(with(|t| t.whites = -100.0)), 1.0) < 0.9);
    // Blacks never moves black itself.
    assert_eq!(respond(&stage(with(|t| t.blacks = 100.0)), 0.0), 0.0);
    assert_eq!(respond(&stage(with(|t| t.blacks = -100.0)), 0.0), 0.0);
}

#[test]
fn t3_t4_each_slider_moves_its_own_zone_most() {
    // T4: every sample sits on the peak of a zone, never on a boundary.
    let peaks = [BLACKS_PEAK, SHADOWS_PEAK, HIGHLIGHTS_PEAK, WHITES_PEAK];
    assert!(
        peaks.iter().all(|peak| *peak > 0.03 && *peak < 0.97),
        "{peaks:?}"
    );
    let neutral = stage(Tone::default());
    type Edit = fn(&mut Tone, f32);
    let sliders: [(&str, Edit); 4] = [
        ("blacks", |t, x| t.blacks = x),
        ("shadows", |t, x| t.shadows = x),
        ("highlights", |t, x| t.highlights = x),
        ("whites", |t, x| t.whites = x),
    ];
    for amount in [-100.0, -50.0, 50.0, 100.0] {
        for (own, (name, edit)) in sliders.iter().enumerate() {
            let edited = stage(with(|t| edit(t, amount)));
            let shift = |v: f32| (respond(&edited, v) - respond(&neutral, v)).abs();
            let own_shift = shift(peaks[own]);
            assert!(
                own_shift > 0.01,
                "{name} {amount} barely moves its own zone"
            );
            for (other, peak) in peaks.iter().enumerate() {
                if other != own {
                    assert!(
                        own_shift > 1.5 * shift(*peak),
                        "{name} {amount} moves zone {other} by {} but its own by {own_shift}",
                        shift(*peak)
                    );
                }
            }
        }
    }
}

#[test]
fn t2_the_tone_response_is_strictly_increasing_at_every_extreme() {
    for mask in 0..32u32 {
        let pick = |bit: u32| {
            if mask & (1 << bit) != 0 {
                100.0
            } else {
                -100.0
            }
        };
        let tone = Tone {
            exposure: 0.0,
            contrast: pick(0),
            highlights: pick(1),
            shadows: pick(2),
            whites: pick(3),
            blacks: pick(4),
        };
        let stage = stage(tone);
        let mut previous = -1.0f32;
        for step in 0..=1024 {
            let rgb = [step as f32 / 1024.0; 3];
            let out = luminance(stage.apply(rgb, tone_base_value(rgb)));
            assert!(
                out > previous,
                "{tone:?}: not increasing at step {step} ({previous} -> {out})"
            );
            previous = out;
        }
    }
}

#[test]
fn the_zone_and_endpoint_curves_keep_a_healthy_slope() {
    for highlights in [-100.0, 0.0, 100.0] {
        for shadows in [-100.0, 0.0, 100.0] {
            let zone = ZoneCurve::new(highlights, shadows);
            assert_eq!(zone.eval(0.0), 0.0);
            assert!((zone.eval(1.0) - 1.0).abs() < 1e-6);
            for step in 0..1000 {
                let v = step as f32 / 1000.0;
                let slope = (zone.eval(v + 0.001) - zone.eval(v)) / 0.001;
                assert!(
                    slope > 0.05 && slope < 6.0,
                    "zone {highlights}/{shadows}: slope {slope} at {v}"
                );
            }
        }
    }
    for whites in [-100.0, 0.0, 100.0] {
        for blacks in [-100.0, 0.0, 100.0] {
            let ends = EndpointCurve::new(whites, blacks);
            assert_eq!(ends.eval(0.0), 0.0);
            for step in 0..1500 {
                let v = step as f32 / 1000.0;
                let slope = (ends.eval(v + 0.001) - ends.eval(v)) / 0.001;
                assert!(
                    slope > 0.1 && slope < 4.0,
                    "ends {whites}/{blacks}: slope {slope} at {v}"
                );
            }
        }
    }
}

#[test]
fn highlights_and_shadows_are_a_gain_looked_up_from_the_base() {
    // Two pixels of different brightness that share a base (a textured area
    // under the edge-aware filter) keep their ratio: local contrast survives.
    let stage = stage(with(|t| {
        t.highlights = -100.0;
        t.shadows = 100.0;
    }));
    assert!(stage.needs_base());
    for base in [0.2, SHADOWS_PEAK, 0.5, HIGHLIGHTS_PEAK, 0.9] {
        let (dark, bright) = ([0.10, 0.12, 0.08], [0.20, 0.24, 0.16]);
        let (a, b) = (stage.apply(dark, base), stage.apply(bright, base));
        for channel in 0..3 {
            assert!((b[channel] / a[channel] - 2.0).abs() < 1e-4, "base {base}");
        }
    }
    // The base, not the pixel, selects the gain.
    let pixel = [0.3, 0.3, 0.3];
    let in_shadow = stage.apply(pixel, SHADOWS_PEAK)[0];
    let in_highlight = stage.apply(pixel, HIGHLIGHTS_PEAK)[0];
    assert!(in_shadow > 0.3 && in_highlight < 0.3);
}

#[test]
fn the_tone_stage_preserves_hue_order() {
    let curve = ToneCurve {
        rgb: PointCurve::new([
            CurvePoint::new(0.0, 0.0),
            CurvePoint::new(0.3, 0.2),
            CurvePoint::new(0.7, 0.85),
            CurvePoint::new(1.0, 1.0),
        ]),
        ..Default::default()
    };
    let tone = Tone {
        exposure: 0.0,
        contrast: 70.0,
        highlights: -50.0,
        shadows: 40.0,
        whites: 30.0,
        blacks: -20.0,
    };
    let stage = ToneStage::new(&tone, &curve);
    for rgb in [
        [0.9, 0.5, 0.1],
        [0.1, 0.5, 0.9],
        [0.5, 0.9, 0.1],
        [0.3, 0.3, 0.8],
        [0.02, 0.6, 0.6],
    ] {
        let out = stage.apply(rgb, tone_base_value(rgb));
        for i in 0..3 {
            for j in 0..3 {
                if rgb[i] > rgb[j] {
                    assert!(out[i] > out[j], "{rgb:?} -> {out:?}");
                }
                if rgb[i] == rgb[j] {
                    assert_eq!(out[i], out[j], "{rgb:?} -> {out:?}");
                }
            }
        }
    }
}

#[test]
fn the_composite_curve_matches_what_the_stage_does_to_a_grey_pixel() {
    let curve = ToneCurve {
        rgb: PointCurve::new([
            CurvePoint::new(0.0, 0.05),
            CurvePoint::new(0.5, 0.6),
            CurvePoint::new(1.0, 0.95),
        ]),
        ..Default::default()
    };
    let mut curve = curve;
    curve.parametric.lights = 40.0;
    curve.parametric.shadows = -30.0;
    let tone = Tone {
        exposure: 0.0,
        contrast: 35.0,
        highlights: -60.0,
        shadows: 45.0,
        whites: -20.0,
        blacks: 25.0,
    };
    let stage = ToneStage::new(&tone, &curve);
    let curves = ToneCurves::new(&tone, &curve);
    for step in 1..=50 {
        let v = step as f32 / 50.0;
        assert!((respond(&stage, v) - curves.eval(v)).abs() < 2e-4, "at {v}");
    }
}

#[test]
fn per_channel_curves_touch_only_their_channel() {
    let lift = PointCurve::new([
        CurvePoint::new(0.0, 0.0),
        CurvePoint::new(0.5, 0.7),
        CurvePoint::new(1.0, 1.0),
    ]);
    let curve = ToneCurve {
        red: lift.clone(),
        ..Default::default()
    };
    let stage = ToneStage::new(&Tone::default(), &curve);
    assert!(!stage.is_identity());
    let out = stage.apply([0.2, 0.2, 0.2], 0.5);
    assert!(out[0] > 0.25);
    assert_eq!([out[1], out[2]], [0.2, 0.2]);
    let curves = ToneCurves::new(&Tone::default(), &curve);
    assert!((curves.channel(0, 0.5) - 0.7).abs() < 1e-6);
    assert_eq!(curves.channel(1, 0.5), 0.5);
    assert_eq!(
        curves.eval(0.5),
        0.5,
        "the composite curve excludes the channel curves"
    );
}

#[test]
fn a_local_delta_equals_the_same_change_made_globally() {
    let base = Tone {
        exposure: 0.0,
        contrast: 10.0,
        highlights: -20.0,
        shadows: 15.0,
        whites: 5.0,
        blacks: -10.0,
    };
    let delta = ToneDelta {
        contrast: 30.0,
        highlights: -40.0,
        shadows: 35.0,
        whites: -25.0,
        blacks: 40.0,
    };
    let summed = Tone {
        exposure: 0.0,
        contrast: 40.0,
        highlights: -60.0,
        shadows: 50.0,
        whites: -20.0,
        blacks: 30.0,
    };
    let (local, global) = (stage(base), stage(summed));
    for rgb in [
        [0.02, 0.03, 0.01],
        [0.2, 0.25, 0.1],
        [0.5, 0.5, 0.5],
        [0.9, 0.7, 0.8],
        [0.0, 0.0, 0.0],
    ] {
        let v = tone_base_value(rgb);
        let (a, b) = (local.apply_with_delta(rgb, v, &delta), global.apply(rgb, v));
        for channel in 0..3 {
            assert!(
                (a[channel] - b[channel]).abs() < 2e-4,
                "{rgb:?}: {a:?} vs {b:?}"
            );
        }
        // A zero delta is the fast path.
        assert_eq!(
            local.apply_with_delta(rgb, v, &ToneDelta::default()),
            local.apply(rgb, v)
        );
    }
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
        [1.0, 1.0, 1.0],
        [4.0, 9.0, 16.0],
    ];
    let curve = ToneCurve {
        rgb: PointCurve::new([
            CurvePoint::new(0.1, 0.0),
            CurvePoint::new(0.5, 0.8),
            CurvePoint::new(0.9, 0.4),
        ]),
        blue: PointCurve::new([CurvePoint::new(0.0, 1.0), CurvePoint::new(1.0, 0.0)]),
        ..Default::default()
    };
    for mask in 0..32u32 {
        let pick = |bit: u32| {
            if mask & (1 << bit) != 0 {
                100.0
            } else {
                -100.0
            }
        };
        let tone = Tone {
            exposure: 0.0,
            contrast: pick(0),
            highlights: pick(1),
            shadows: pick(2),
            whites: pick(3),
            blacks: pick(4),
        };
        let delta = ToneDelta {
            contrast: pick(4),
            highlights: pick(3),
            shadows: pick(2),
            whites: pick(1),
            blacks: pick(0),
        };
        for stage in [
            ToneStage::new(&tone, &ToneCurve::default()),
            ToneStage::new(&tone, &curve),
        ] {
            for rgb in inputs {
                for base in [0.0, 0.4, 1.0, 5.0, -1.0, f32::NAN] {
                    for out in [
                        stage.apply(rgb, base),
                        stage.apply_with_delta(rgb, base, &delta),
                    ] {
                        assert!(
                            out.iter().all(|v| v.is_finite() && *v >= 0.0),
                            "{tone:?} {rgb:?}: {out:?}"
                        );
                    }
                }
            }
        }
    }
}
