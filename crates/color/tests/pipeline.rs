//! Contracts of the compiled per-pixel pipeline: the stage boundaries that
//! `photo-develop` calls, end to end.

use photo_color::{LocalDelta, PixelPipeline, displayed_white_balance, pick_white_balance};
use photo_core::colorimetry::{RgbSpace, luminance, mat3_apply, rgb_to_rgb};
use photo_core::develop::{DevelopSettings, Tone, Treatment, WbPreset, WhiteBalance};
use photo_core::image::{CameraColor, SourceColor};

const WORKING: SourceColor = SourceColor::Rgb(RgbSpace::Rec2020);
const SRGB: SourceColor = SourceColor::Rgb(RgbSpace::Srgb);

fn camera_color() -> CameraColor {
    CameraColor {
        xyz_to_camera: [
            [0.6640, -0.1847, -0.0503],
            [-0.5238, 1.3010, 0.2474],
            [-0.0993, 0.1673, 0.6527],
        ],
        as_shot_gains: [2.31, 1.0, 1.58],
        baseline_exposure: 0.0,
    }
}

fn camera() -> SourceColor {
    SourceColor::Camera(camera_color())
}

/// What the sensor records for a grey of the given level under the as-shot
/// light.
fn raw_grey(level: f32) -> [f32; 3] {
    let gains = camera_color().as_shot_gains;
    [level / gains[0], level / gains[1], level / gains[2]]
}

fn settings(edit: impl FnOnce(&mut DevelopSettings)) -> DevelopSettings {
    let mut settings = DevelopSettings::default();
    edit(&mut settings);
    settings
}

/// Stages 3, 5, 6 and 7 with the tone base equal to the pixel's own value.
fn develop(pipeline: &PixelPipeline, source: [f32; 3], delta: Option<&LocalDelta>) -> [f32; 3] {
    let balanced = pipeline.input(source, delta);
    let base = pipeline.tone_base(balanced, delta);
    let display = pipeline.scene_to_display(balanced, base, delta);
    pipeline.color(pipeline.tone(display, delta), delta)
}

fn assert_close(a: [f32; 3], b: [f32; 3], tolerance: f32) {
    for (x, y) in a.iter().zip(b) {
        assert!((x - y).abs() <= tolerance, "{a:?} vs {b:?}");
    }
}

const PIXELS: [[f32; 3]; 6] = [
    [0.0, 0.5, 1.0],
    [0.123_456_7, 0.2, 0.999_999],
    [0.02, 0.03, 0.01],
    [0.6, 0.55, 0.2],
    [0.4, 0.4, 0.4],
    [1.0e-6, 0.25, 0.9],
];

#[test]
fn t1_neutral_settings_are_exactly_the_identity_for_a_raster_source() {
    let pipeline = PixelPipeline::new(&DevelopSettings::default(), &WORKING);
    assert!(pipeline.input_is_identity());
    assert!(pipeline.scene_to_display_is_identity());
    assert!(pipeline.tone_is_identity());
    assert!(!pipeline.tone_needs_base());
    assert!(pipeline.color_is_identity());
    let zero = LocalDelta::default();
    for rgb in PIXELS {
        assert_eq!(develop(&pipeline, rgb, None), rgb);
        assert_eq!(develop(&pipeline, rgb, Some(&zero)), rgb);
    }

    // A file in another colour space is converted to working space by the
    // input stage, exactly, and nothing else touches it.
    let from_srgb = PixelPipeline::new(&DevelopSettings::default(), &SRGB);
    assert!(!from_srgb.input_is_identity());
    let matrix = rgb_to_rgb(RgbSpace::Srgb, RgbSpace::Rec2020);
    for rgb in PIXELS {
        assert_eq!(develop(&from_srgb, rgb, None), mat3_apply(&matrix, rgb));
    }
}

#[test]
fn default_settings_reproduce_an_srgb_file_code_for_code() {
    let pipeline = PixelPipeline::new(&DevelopSettings::default(), &SRGB);
    let decode = |code: u8| RgbSpace::Srgb.decode(f32::from(code) / 255.0);
    for code in 0..=255u8 {
        let out = pipeline.output_u8(develop(&pipeline, [decode(code); 3], None));
        assert_eq!(out, [code; 3]);
    }
    for r in (0..=255u8).step_by(51) {
        for g in (0..=255u8).step_by(51) {
            for b in (0..=255u8).step_by(51) {
                let out =
                    pipeline.output_u8(develop(&pipeline, [decode(r), decode(g), decode(b)], None));
                assert_eq!(out, [r, g, b]);
            }
        }
    }
}

fn extreme_tone(mask: u32, exposure: [f32; 2]) -> Tone {
    let pick = |bit: u32| {
        if mask & (1 << bit) != 0 {
            100.0
        } else {
            -100.0
        }
    };
    Tone {
        exposure: exposure[(mask >> 5 & 1) as usize],
        contrast: pick(0),
        highlights: pick(1),
        shadows: pick(2),
        whites: pick(3),
        blacks: pick(4),
    }
}

#[test]
fn t2_the_composite_response_is_strictly_increasing_at_all_64_extremes() {
    // Raster source: all six tone sliders, exposure included, at ±extreme.
    for mask in 0..64u32 {
        let tone = extreme_tone(mask, [-5.0, 5.0]);
        let pipeline = PixelPipeline::new(&settings(|s| s.tone = tone), &WORKING);
        let mut previous = -1.0f32;
        for step in 0..=256 {
            let out = luminance(develop(&pipeline, [step as f32 / 256.0; 3], None));
            assert!(
                out > previous,
                "{tone:?}: not increasing at level {step} ({previous} -> {out})"
            );
            previous = out;
        }
    }
}

#[test]
fn t2_holds_for_a_camera_source_up_to_where_the_base_curve_saturates() {
    // With exposure at -5 and 0 the response is strictly increasing over the
    // whole raw range. At +5 nearly every level is blown and the base curve
    // is within a few `f32` steps of its asymptote, where consecutive levels
    // differ by less than rounding does; there the response can only be
    // required not to fall by more than that rounding.
    for mask in 0..64u32 {
        let tone = extreme_tone(mask, [-5.0, 0.0]);
        let pipeline = PixelPipeline::new(&settings(|s| s.tone = tone), &camera());
        let mut previous = -1.0f32;
        for step in 0..=256 {
            let out = luminance(develop(&pipeline, raw_grey(step as f32 / 256.0), None));
            assert!(
                out > previous,
                "{tone:?}: not increasing at level {step} ({previous} -> {out})"
            );
            previous = out;
        }
    }
    for mask in 0..32u32 {
        let tone = extreme_tone(mask, [5.0, 5.0]);
        let pipeline = PixelPipeline::new(&settings(|s| s.tone = tone), &camera());
        let mut previous = -1.0f32;
        for step in 0..=256 {
            let out = luminance(develop(&pipeline, raw_grey(step as f32 / 256.0), None));
            assert!(
                out >= previous - 4e-6,
                "{tone:?}: fell at level {step} ({previous} -> {out})"
            );
            previous = out;
        }
    }
}

#[test]
fn t5_no_input_produces_nan_or_a_negative_value_anywhere_in_the_chain() {
    let inputs = [
        [0.0, 0.0, 0.0],
        [1.0e-6, 1.0e-6, 1.0e-6],
        [16.0, 0.0, 1.0e-6],
        [8.0, 12.0, 16.0],
        [-0.01, 0.2, 0.5],
        [-1.0e-4, -1.0e-4, -1.0e-4],
        [1.0, 1.0, 1.0],
        [0.9, 1.0, 0.95],
    ];
    let wild = |sign: f32| {
        settings(|s| {
            s.white_balance = WhiteBalance {
                preset: WbPreset::Custom,
                temp: 100.0 * sign,
                tint: -100.0 * sign,
            };
            s.tone = Tone {
                exposure: 5.0 * sign,
                contrast: 100.0,
                highlights: -100.0 * sign,
                shadows: 100.0 * sign,
                whites: 100.0,
                blacks: -100.0 * sign,
            };
            s.presence.vibrance = 100.0;
            s.presence.saturation = 100.0;
            s.color_mixer.saturation = [100.0; 8];
            s.color_mixer.luminance = [-100.0 * sign; 8];
            s.tone_curve.parametric.shadows = 100.0;
            s.tone_curve.parametric.highlights = -100.0;
            s.color_grading.global.saturation = 100.0;
            s.calibration.blue_saturation = 100.0;
        })
    };
    let delta = LocalDelta {
        exposure: 4.0,
        contrast: 100.0,
        highlights: 100.0,
        shadows: -100.0,
        whites: -100.0,
        blacks: 100.0,
        temp: -100.0,
        tint: 100.0,
        saturation: 100.0,
    };
    for source in [
        WORKING,
        SRGB,
        SourceColor::Rgb(RgbSpace::ProPhoto),
        camera(),
    ] {
        for develop_settings in [DevelopSettings::default(), wild(1.0), wild(-1.0)] {
            let pipeline = PixelPipeline::new(&develop_settings, &source);
            for rgb in inputs {
                for local in [None, Some(&delta)] {
                    let balanced = pipeline.input(rgb, local);
                    let base = pipeline.tone_base(balanced, local);
                    let display = pipeline.scene_to_display(balanced, base, local);
                    let toned = pipeline.tone(display, local);
                    let coloured = pipeline.color(toned, local);
                    for (stage, value) in [
                        ("input", balanced),
                        ("scene", display),
                        ("tone", toned),
                        ("color", coloured),
                    ] {
                        assert!(
                            value.iter().all(|v| v.is_finite() && *v >= 0.0),
                            "{stage} of {rgb:?} from {source:?}: {value:?}"
                        );
                    }
                    assert!(base.is_finite() && base >= 0.0);
                    let encoded = pipeline.output_f32(coloured);
                    assert!(
                        encoded.iter().all(|v| (0.0..=1.0).contains(v)),
                        "{encoded:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn saturation_minus_100_and_black_and_white_reach_the_screen_neutral() {
    for source in [SRGB, camera()] {
        for develop_settings in [
            settings(|s| s.presence.saturation = -100.0),
            settings(|s| s.treatment = Treatment::BlackAndWhite),
            settings(|s| {
                s.treatment = Treatment::BlackAndWhite;
                s.color_mixer.bw_mix = [40.0, -30.0, 20.0, -10.0, 0.0, 10.0, -20.0, 30.0];
                s.tone.contrast = 30.0;
            }),
        ] {
            let pipeline = PixelPipeline::new(&develop_settings, &source);
            for rgb in PIXELS {
                let bytes = pipeline.output_u8(develop(&pipeline, rgb, None));
                assert!(
                    bytes[0] == bytes[1] && bytes[1] == bytes[2],
                    "{rgb:?} -> {bytes:?}"
                );
                let words = pipeline.output_u16(develop(&pipeline, rgb, None));
                assert!(
                    words[0] == words[1] && words[1] == words[2],
                    "{rgb:?} -> {words:?}"
                );
            }
        }
    }
}

#[test]
fn white_balance_moves_pixels_the_way_the_sliders_say() {
    // Raster: positive temperature is warmer, positive tint more magenta.
    let grey = [0.4f32; 3];
    let raster = |temp: f32, tint: f32| {
        let develop_settings = settings(|s| {
            s.white_balance = WhiteBalance {
                preset: WbPreset::Custom,
                temp,
                tint,
            };
        });
        PixelPipeline::new(&develop_settings, &WORKING).input(grey, None)
    };
    let (warm, cool) = (raster(50.0, 0.0), raster(-50.0, 0.0));
    assert!(warm[0] > warm[2] && cool[2] > cool[0], "{warm:?} {cool:?}");
    let magenta = raster(0.0, 50.0);
    assert!(
        magenta[1] < magenta[0] && magenta[1] < magenta[2],
        "{magenta:?}"
    );
    // As Shot ignores stray numbers.
    let stray = settings(|s| s.white_balance.temp = 80.0);
    assert!(PixelPipeline::new(&stray, &WORKING).input_is_identity());

    // Camera: as shot renders the as-shot grey neutral; a higher kelvin
    // setting renders it warmer.
    let as_shot =
        PixelPipeline::new(&DevelopSettings::default(), &camera()).input(raw_grey(0.3), None);
    assert_close(as_shot, [0.3; 3], 1e-5);
    let kelvin = |temp: f32| {
        let develop_settings = settings(|s| {
            s.white_balance = WhiteBalance {
                preset: WbPreset::Custom,
                temp,
                tint: 0.0,
            };
        });
        PixelPipeline::new(&develop_settings, &camera()).input(raw_grey(0.3), None)
    };
    let (hot, cold) = (kelvin(9000.0), kelvin(3200.0));
    assert!(hot[0] / hot[2] > cold[0] / cold[2]);
}

#[test]
fn the_white_balance_readout_and_picker_agree_with_the_input_stage() {
    assert_eq!(
        displayed_white_balance(&WhiteBalance::default(), &SRGB),
        (0.0, 0.0)
    );
    let custom = WhiteBalance {
        preset: WbPreset::Custom,
        temp: 12.0,
        tint: -7.0,
    };
    assert_eq!(displayed_white_balance(&custom, &SRGB), (12.0, -7.0));
    let (kelvin, tint) = displayed_white_balance(&WhiteBalance::default(), &camera());
    assert!((2000.0..=50_000.0).contains(&kelvin) && tint.abs() <= 150.0);

    // Switching from As Shot to the displayed numbers changes nothing.
    let explicit = settings(|s| {
        s.white_balance = WhiteBalance {
            preset: WbPreset::Custom,
            temp: kelvin,
            tint,
        };
    });
    let (a, b) = (
        PixelPipeline::new(&DevelopSettings::default(), &camera()),
        PixelPipeline::new(&explicit, &camera()),
    );
    for raw in [raw_grey(0.3), [0.1, 0.3, 0.2], [0.02, 0.01, 0.03]] {
        let (x, y) = (a.input(raw, None), b.input(raw, None));
        assert_close(x, y, 3e-3 * x.iter().cloned().fold(0.01, f32::max));
    }

    // The picker neutralises its sample, for both kinds of source. The
    // camera sample is a grey card under 4200 K light with a little green.
    let light = photo_color::temperature::temp_tint_to_xy(4200.0, 15.0)
        .to_xyz()
        .map(|v| v as f32 * 0.3);
    let card = mat3_apply(&camera_color().xyz_to_camera, light);
    for (source, sample) in [(camera(), card), (SRGB, [0.42, 0.40, 0.33])] {
        let picked = pick_white_balance(&source, sample).expect("a usable sample");
        assert_eq!(picked.preset, WbPreset::Custom);
        let pipeline = PixelPipeline::new(&settings(|s| s.white_balance = picked), &source);
        let out = pipeline.input(sample, None);
        let mean = (out[0] + out[1] + out[2]) / 3.0;
        assert_close(out, [mean; 3], 3e-3 * mean);
    }
    assert_eq!(pick_white_balance(&SRGB, [0.0; 3]), None);
}

#[test]
fn the_output_space_is_selectable() {
    let develop_settings = DevelopSettings::default();
    let srgb = PixelPipeline::new(&develop_settings, &WORKING);
    let p3 = PixelPipeline::new(&develop_settings, &WORKING).with_output_space(RgbSpace::DisplayP3);
    assert_eq!(srgb.output_space(), RgbSpace::Srgb);
    assert_eq!(p3.output_space(), RgbSpace::DisplayP3);
    assert_eq!(srgb.output_u8([0.3; 3]), p3.output_u8([0.3; 3]));
    assert_ne!(
        srgb.output_u8([0.5, 0.1, 0.1]),
        p3.output_u8([0.5, 0.1, 0.1])
    );
}
