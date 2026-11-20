//! Contracts of Highlights and Shadows (DESIGN §5.5): a scene-linear gain
//! looked up from an edge-aware base, applied in front of the base curve.

use photo_color::tone::{
    BLACKS_PEAK, HIGHLIGHTS_PEAK, SHADOWS_PEAK, ToneCurves, WHITES_PEAK, ZoneCurve,
    encode_perceptual,
};
use photo_color::{LocalDelta, PixelPipeline};
use photo_core::colorimetry::{RgbSpace, luminance};
use photo_core::develop::{CurvePoint, DevelopSettings, PointCurve, Profile, Tone, ToneCurve};
use photo_core::image::{CameraColor, SourceColor};
use photo_core::meta::FileKind;

const WORKING: SourceColor = SourceColor::Rgb(RgbSpace::Rec2020);
const KINDS: [FileKind; 2] = [FileKind::Raster, FileKind::Raw];

fn camera() -> SourceColor {
    SourceColor::Camera(CameraColor {
        xyz_to_camera: [
            [0.6640, -0.1847, -0.0503],
            [-0.5238, 1.3010, 0.2474],
            [-0.0993, 0.1673, 0.6527],
        ],
        as_shot_gains: [2.31, 1.0, 1.58],
        baseline_exposure: 0.0,
    })
}

fn with_tone(tone: Tone) -> DevelopSettings {
    DevelopSettings {
        tone,
        ..Default::default()
    }
}

/// The owner's usual raw recipe.
const RECIPE: Tone = Tone {
    exposure: 0.30,
    contrast: 6.0,
    highlights: -100.0,
    shadows: 100.0,
    whites: 30.0,
    blacks: -30.0,
};

const HIGHLIGHTS_PULL: Tone = Tone {
    exposure: 0.0,
    contrast: 0.0,
    highlights: -100.0,
    shadows: 0.0,
    whites: 0.0,
    blacks: 0.0,
};

/// Stages 5 and 6 for a flat neutral area of scene luminance `y` whose base
/// is its own value: the base and the perceptual result.
fn flat(pipeline: &PixelPipeline, y: f32) -> (f32, f32) {
    let rgb = [y; 3];
    let base = pipeline.tone_base(rgb, None);
    let out = pipeline.tone(pipeline.scene_to_display(rgb, base, None), None);
    (base, encode_perceptual(luminance(out)))
}

#[test]
fn a_flat_neutral_area_lands_where_the_plotted_curve_says() {
    let mut curve = ToneCurve {
        rgb: PointCurve::new([
            CurvePoint::new(0.0, 0.0),
            CurvePoint::new(0.4, 0.36),
            CurvePoint::new(1.0, 1.0),
        ]),
        ..Default::default()
    };
    curve.parametric.lights = 25.0;
    let tones = [
        RECIPE,
        HIGHLIGHTS_PULL,
        Tone {
            exposure: -1.0,
            contrast: -40.0,
            highlights: 100.0,
            shadows: -100.0,
            whites: -60.0,
            blacks: 50.0,
        },
    ];
    let sources = [
        (WORKING, Profile::Standard, FileKind::Raster),
        (camera(), Profile::Standard, FileKind::Raw),
        (camera(), Profile::Neutral, FileKind::Raw),
        (camera(), Profile::Vivid, FileKind::Raw),
        (camera(), Profile::Linear, FileKind::Raw),
    ];
    for (source, profile, kind) in sources {
        for tone in tones {
            let settings = DevelopSettings {
                profile,
                tone,
                tone_curve: curve.clone(),
                ..Default::default()
            };
            let pipeline = PixelPipeline::new(&settings, &source);
            let curves = ToneCurves::new(&tone, &curve, kind);
            let mut checked = 0;
            for step in 0..=600 {
                // From twelve stops below to two stops above nominal white.
                let y = (-12.0 + 14.0 * step as f32 / 600.0).exp2();
                let (base, out) = flat(&pipeline, y);
                // The documented limit: closer to white, the gain is held.
                if base > 1.0 - 1.0e-5 {
                    continue;
                }
                let plotted = curves.eval(base);
                assert!(
                    (out - plotted).abs() < 1.0e-3,
                    "{profile:?} {kind:?} {tone:?} at y = {y}: rendered {out}, plotted {plotted}"
                );
                checked += 1;
            }
            assert!(
                checked > 450,
                "{profile:?} {kind:?}: only {checked} levels checked"
            );
        }
    }
}

/// How much a ±4% texture riding on luminance `y` differs after a full
/// render, in encoded sRGB output. The base is the untextured value, as a
/// perfect edge-aware filter would give.
const TEXTURE: f32 = 0.04;

fn rendered_texture(pipeline: &PixelPipeline, y: f32) -> f32 {
    let base = pipeline.tone_base([y; 3], None);
    let shade = |scale: f32| {
        let display = pipeline.scene_to_display([y * scale; 3], base, None);
        pipeline.output_f32(pipeline.color(pipeline.tone(display, None), None))[1]
    };
    shade(1.0 + TEXTURE) - shade(1.0 - TEXTURE)
}

/// The same with Highlights and Shadows applied the way the design first
/// had it: after the base curve, as the display-linear gain `(Z(b) / b)^2.2`
/// looked up from the smoothed perceptual luminance of the rendered picture.
/// `pipeline` is the same settings with Highlights and Shadows at zero.
fn post_curve_texture(pipeline: &PixelPipeline, zone: &ZoneCurve, y: f32) -> f32 {
    let base = encode_perceptual(luminance(pipeline.scene_to_display([y; 3], 0.0, None)));
    let gain = (zone.eval(base) / base).powf(2.2);
    let shade = |scale: f32| {
        let display = pipeline.scene_to_display([y * scale; 3], 0.0, None);
        let darkened = display.map(|channel| channel * gain);
        pipeline.output_f32(pipeline.color(pipeline.tone(darkened, None), None))[1]
    };
    shade(1.0 + TEXTURE) - shade(1.0 - TEXTURE)
}

/// Texture contrast summed over a ramp from nominal white (white-balanced
/// 1.0, the sensor's clip point) to two stops above it, `from` and `to` in
/// stops: everything the default rendering squeezes above display 0.99.
fn headroom_texture(texture: impl Fn(f32) -> f32, from: f32, to: f32) -> f32 {
    (0..=64)
        .map(|step| texture((from + (to - from) * step as f32 / 64.0).exp2()))
        .sum()
}

/// (new method, post-curve method) texture over the headroom as a multiple
/// of the default rendering's, for `tone` over the stops `from..to`.
fn recovery_factors(tone: Tone, from: f32, to: f32) -> (f32, f32) {
    let source = camera();
    let default = PixelPipeline::new(&DevelopSettings::default(), &source);
    let edited = PixelPipeline::new(&with_tone(tone), &source);
    let flat_zones = PixelPipeline::new(
        &with_tone(Tone {
            highlights: 0.0,
            shadows: 0.0,
            ..tone
        }),
        &source,
    );
    let zone = ZoneCurve::new(tone.highlights, tone.shadows, FileKind::Raw);
    let reference = headroom_texture(|y| rendered_texture(&default, y), from, to);
    let new = headroom_texture(|y| rendered_texture(&edited, y), from, to);
    let old = headroom_texture(|y| post_curve_texture(&flat_zones, &zone, y), from, to);
    (new / reference, old / reference)
}

#[test]
fn pulling_highlights_down_recovers_texture_from_the_headroom() {
    for (name, tone) in [("highlights -100", HIGHLIGHTS_PULL), ("recipe", RECIPE)] {
        let (new, old) = recovery_factors(tone, 0.0, 2.0);
        let (first_new, first_old) = recovery_factors(tone, 0.0, 1.0);
        let (second_new, second_old) = recovery_factors(tone, 1.0, 2.0);
        eprintln!(
            "{name}: texture over the 2 EV of headroom x{new:.1} (post-curve x{old:.2}); \
             first stop x{first_new:.1} (x{first_old:.2}), second stop x{second_new:.1} \
             (x{second_old:.2})"
        );
        assert!(new > 4.0, "{name}: only x{new} the default's texture");
        assert!(
            new > 4.0 * old,
            "{name}: x{new} is not clearly above post-curve x{old}"
        );
    }
    // Pulling highlights alone recovers both stops of headroom.
    let (first, _) = recovery_factors(HIGHLIGHTS_PULL, 0.0, 1.0);
    let (second, _) = recovery_factors(HIGHLIGHTS_PULL, 1.0, 2.0);
    assert!(first > 4.0 && second > 4.0, "x{first} and x{second}");
}

#[test]
fn recovered_headroom_stays_below_white_and_in_order() {
    let pipeline = PixelPipeline::new(&with_tone(HIGHLIGHTS_PULL), &camera());
    let default = PixelPipeline::new(&DevelopSettings::default(), &camera());
    let mut previous = 0.0;
    for step in 0..=64 {
        let y = (2.0 * step as f32 / 64.0).exp2();
        let (_, out) = flat(&pipeline, y);
        let (_, untouched) = flat(&default, y);
        assert!(out > previous, "headroom out of order at {y}");
        assert!(
            out < untouched,
            "{y}: {out} is not below the default {untouched}"
        );
        previous = out;
    }
    // The clip point lands in the visible range, two stops above it short
    // of white (display-linear 0.66 and 0.93 on this profile).
    let display = |y: f32| flat(&pipeline, y).1.powf(2.2);
    let (at_clip, one_up, two_up) = (display(1.0), display(2.0), display(4.0));
    eprintln!("headroom lands at display {at_clip:.3}, {one_up:.3}, {two_up:.3}");
    assert!((at_clip - 0.66).abs() < 0.01 && (two_up - 0.93).abs() < 0.01);
    assert!(one_up > at_clip + 0.1 && two_up > one_up + 0.05);
}

/// The display EV by which a flat neutral area at perceptual `v` moves.
fn shift_ev(curves: &ToneCurves, v: f32) -> f32 {
    2.2 * (curves.eval(v) / v).log2()
}

#[test]
fn a_single_zone_slider_stays_targeted() {
    // The owner's rule: bands about as tight as a Gaussian of σ = 1 EV, not
    // √2, so a full-strength slider reads as targeted recovery rather than
    // as an exposure shift. Each response away from the zone's own peak is
    // held to that Gaussian's weight at the same distance in display EV.
    let sigma_one = |from: f32, to: f32| {
        let distance = 2.2 * (to / from).log2();
        (-0.5 * distance * distance).exp()
    };
    let slider = |highlights: f32, shadows: f32| Tone {
        highlights,
        shadows,
        ..Default::default()
    };
    for kind in KINDS {
        for (name, tone, own, other) in [
            (
                "Highlights +100",
                slider(100.0, 0.0),
                HIGHLIGHTS_PEAK,
                SHADOWS_PEAK,
            ),
            (
                "Highlights -100",
                slider(-100.0, 0.0),
                HIGHLIGHTS_PEAK,
                SHADOWS_PEAK,
            ),
            (
                "Shadows +100",
                slider(0.0, 100.0),
                SHADOWS_PEAK,
                HIGHLIGHTS_PEAK,
            ),
            (
                "Shadows -100",
                slider(0.0, -100.0),
                SHADOWS_PEAK,
                HIGHLIGHTS_PEAK,
            ),
        ] {
            let curves = ToneCurves::new(&tone, &ToneCurve::default(), kind);
            let at_own = shift_ev(&curves, own);
            assert!(
                at_own.abs() > 0.25,
                "{kind:?} {name}: {at_own} EV at its own peak"
            );
            let mut line = format!("{kind:?} {name}: {at_own:+.2} EV at its peak");
            for probe in [other, 0.5] {
                let fraction = shift_ev(&curves, probe) / at_own;
                line += &format!(", {:.0}% at {probe}", 100.0 * fraction);
                assert!(
                    (0.0..=sigma_one(own, probe) + 0.03).contains(&fraction),
                    "{kind:?} {name}: {fraction} of its peak at {probe}"
                );
            }
            eprintln!("{line}");
        }
    }
    // Full-strength Shadows has about 1.5 EV of reach.
    let shadows = ToneCurves::new(&slider(0.0, 100.0), &ToneCurve::default(), FileKind::Raw);
    assert!((shift_ev(&shadows, SHADOWS_PEAK) - 1.5).abs() < 0.05);
}

#[test]
fn t3_t4_each_slider_moves_its_own_zone_most() {
    // T4: every sample sits on the peak of a zone, inside (0, 1).
    let peaks = [BLACKS_PEAK, SHADOWS_PEAK, HIGHLIGHTS_PEAK, WHITES_PEAK];
    assert!(
        peaks.iter().all(|peak| *peak > 0.0 && *peak < 1.0),
        "{peaks:?}"
    );
    type Edit = fn(&mut Tone, f32);
    let sliders: [(&str, Edit); 4] = [
        ("blacks", |t, x| t.blacks = x),
        ("shadows", |t, x| t.shadows = x),
        ("highlights", |t, x| t.highlights = x),
        ("whites", |t, x| t.whites = x),
    ];
    for kind in KINDS {
        let neutral = ToneCurves::new(&Tone::default(), &ToneCurve::default(), kind);
        for amount in [-100.0, -50.0, 50.0, 100.0] {
            for (own, (name, edit)) in sliders.iter().enumerate() {
                let mut tone = Tone::default();
                edit(&mut tone, amount);
                let edited = ToneCurves::new(&tone, &ToneCurve::default(), kind);
                let shift = |v: f32| (edited.eval(v) - neutral.eval(v)).abs();
                let own_shift = shift(peaks[own]);
                assert!(
                    own_shift > 0.01,
                    "{kind:?} {name} {amount} barely moves its zone"
                );
                for (other, peak) in peaks.iter().enumerate() {
                    if other != own {
                        assert!(
                            own_shift > 1.5 * shift(*peak),
                            "{kind:?} {name} {amount} moves zone {other} by {} but its own by \
                             {own_shift}",
                            shift(*peak)
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn a_mask_pulls_highlights_through_the_same_path_as_the_slider() {
    let source = camera();
    let half = PixelPipeline::new(
        &with_tone(Tone {
            highlights: -40.0,
            shadows: 30.0,
            ..Default::default()
        }),
        &source,
    );
    let full = PixelPipeline::new(&with_tone(HIGHLIGHTS_PULL), &source);
    let delta = LocalDelta {
        highlights: -60.0,
        shadows: -30.0,
        ..Default::default()
    };
    for step in 0..=40 {
        let y = (-6.0 + 8.0 * step as f32 / 40.0).exp2();
        for scale in [1.0 - TEXTURE, 1.0, 1.0 + TEXTURE] {
            let rgb = [y * scale; 3];
            let base = full.tone_base([y; 3], None);
            let masked =
                half.scene_to_display(rgb, half.tone_base([y; 3], Some(&delta)), Some(&delta));
            let global = full.scene_to_display(rgb, base, None);
            for channel in 0..3 {
                assert!(
                    (masked[channel] - global[channel]).abs() <= 1.0e-3 * global[channel].max(0.05),
                    "y = {y}: masked {masked:?} vs global {global:?}"
                );
            }
        }
    }
}

#[test]
fn highlights_and_shadows_move_pixels_in_their_documented_direction() {
    let slider = |highlights: f32, shadows: f32| {
        with_tone(Tone {
            highlights,
            shadows,
            ..Default::default()
        })
    };
    for source in [WORKING, camera()] {
        let neutral = PixelPipeline::new(&DevelopSettings::default(), &source);
        // The scene luminance whose own base is `v` with no zone active.
        let at = |v: f32| {
            let mut y = 1.0f32;
            for _ in 0..60 {
                let base = neutral.tone_base([y; 3], None);
                y *= (v / base).powf(2.2).clamp(0.5, 2.0);
            }
            y
        };
        let respond = |settings: &DevelopSettings, v: f32| {
            flat(&PixelPipeline::new(settings, &source), at(v)).1
        };
        for (settings, v, up) in [
            (slider(60.0, 0.0), HIGHLIGHTS_PEAK, true),
            (slider(-60.0, 0.0), HIGHLIGHTS_PEAK, false),
            (slider(0.0, 60.0), SHADOWS_PEAK, true),
            (slider(0.0, -60.0), SHADOWS_PEAK, false),
        ] {
            let (moved, before) = (
                respond(&settings, v),
                respond(&DevelopSettings::default(), v),
            );
            assert!(
                (before - v).abs() < 1e-3,
                "{source:?}: probe at {before}, not {v}"
            );
            assert_eq!(
                moved > before + 0.01,
                up,
                "{source:?} {:?}: {before} -> {moved}",
                settings.tone
            );
            assert_eq!(
                moved < before - 0.01,
                !up,
                "{source:?} {:?}: {before} -> {moved}",
                settings.tone
            );
        }
    }
}

#[test]
fn the_base_not_the_pixel_selects_the_gain() {
    // A raster source at zero exposure has no base curve, so the scene gain
    // is visible directly: pixels sharing a base keep their ratio (local
    // contrast survives), and the same pixel under different bases moves
    // differently.
    let pipeline = PixelPipeline::new(
        &with_tone(Tone {
            highlights: -100.0,
            shadows: 100.0,
            ..Default::default()
        }),
        &WORKING,
    );
    assert!(pipeline.tone_needs_base());
    for base in [0.2, SHADOWS_PEAK, 0.5, HIGHLIGHTS_PEAK, 0.9] {
        let (dark, bright) = ([0.10, 0.12, 0.08], [0.20, 0.24, 0.16]);
        let a = pipeline.scene_to_display(dark, base, None);
        let b = pipeline.scene_to_display(bright, base, None);
        for channel in 0..3 {
            assert!((b[channel] / a[channel] - 2.0).abs() < 1e-5, "base {base}");
        }
    }
    let pixel = [0.3, 0.3, 0.3];
    let in_shadow = pipeline.scene_to_display(pixel, SHADOWS_PEAK, None)[0];
    let in_highlight = pipeline.scene_to_display(pixel, HIGHLIGHTS_PEAK, None)[0];
    assert!(in_shadow > 0.3 && in_highlight < 0.3);
}

#[test]
fn a_raster_source_at_default_settings_is_untouched() {
    for space in [RgbSpace::Rec2020, RgbSpace::Srgb] {
        let pipeline = PixelPipeline::new(&DevelopSettings::default(), &SourceColor::Rgb(space));
        assert!(pipeline.scene_to_display_is_identity() && pipeline.tone_is_identity());
        assert!(!pipeline.tone_needs_base());
        for rgb in [
            [0.0, 0.5, 1.0],
            [0.123_456_7, 0.2, 0.999_999],
            [1.0e-6, 3.0, 16.0],
        ] {
            for base in [0.0, 0.5, f32::NAN] {
                let display = pipeline.scene_to_display(rgb, base, None);
                assert_eq!(pipeline.tone(display, None), rgb);
            }
        }
    }
}
