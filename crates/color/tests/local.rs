//! Contracts of local (masked) adjustments: the per-pixel [`LocalDelta`]
//! that carries `Σ alphaᵢ · amountᵢ · localᵢ` into each stage (DESIGN §5.2).

use photo_color::tone::tone_base_value;
use photo_color::{LocalDelta, PixelPipeline};
use photo_core::colorimetry::RgbSpace;
use photo_core::develop::{
    DevelopSettings, LocalAdjustments, Mask, MaskShape, Tone, WbPreset, WhiteBalance,
};
use photo_core::geom::Point;
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
    let display = pipeline.scene_to_display(balanced, delta);
    let toned = pipeline.tone(display, tone_base_value(display), delta);
    pipeline.color(toned, delta)
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
fn a_local_delta_matches_the_same_change_made_globally() {
    let base = settings(|s| {
        s.white_balance = WhiteBalance {
            preset: WbPreset::Custom,
            temp: 10.0,
            tint: -5.0,
        };
        s.tone = Tone {
            exposure: 0.25,
            contrast: 10.0,
            highlights: -20.0,
            shadows: 15.0,
            whites: 5.0,
            blacks: -10.0,
        };
        s.presence.saturation = 10.0;
    });
    let delta = LocalDelta {
        exposure: 0.75,
        contrast: 25.0,
        highlights: -30.0,
        shadows: 25.0,
        whites: -15.0,
        blacks: 20.0,
        temp: 20.0,
        tint: 15.0,
        saturation: 30.0,
    };
    let summed = settings(|s| {
        s.white_balance = WhiteBalance {
            preset: WbPreset::Custom,
            temp: 30.0,
            tint: 10.0,
        };
        s.tone = Tone {
            exposure: 1.0,
            contrast: 35.0,
            highlights: -50.0,
            shadows: 40.0,
            whites: -10.0,
            blacks: 10.0,
        };
        s.presence.saturation = 40.0;
    });
    let (local, global) = (
        PixelPipeline::new(&base, &SRGB),
        PixelPipeline::new(&summed, &SRGB),
    );
    for rgb in PIXELS {
        assert_close(
            develop(&local, rgb, Some(&delta)),
            develop(&global, rgb, None),
            3e-4,
        );
    }
}

#[test]
fn a_fading_mask_edge_meets_the_unmasked_pixel_continuously() {
    // As a mask's alpha goes to zero its delta goes to zero, and the masked
    // path (curves evaluated directly) must land on the unmasked one
    // (tables) with no visible seam: well inside one 8-bit step.
    let edited = settings(|s| {
        s.white_balance = WhiteBalance {
            preset: WbPreset::Custom,
            temp: 15.0,
            tint: 5.0,
        };
        s.tone = Tone {
            exposure: 0.5,
            contrast: 30.0,
            highlights: -40.0,
            shadows: 35.0,
            whites: 15.0,
            blacks: -10.0,
        };
        s.tone_curve.parametric.lights = 30.0;
        s.presence.saturation = 20.0;
    });
    let full = LocalAdjustments {
        exposure: 1.0,
        contrast: 40.0,
        highlights: -50.0,
        shadows: 50.0,
        whites: 20.0,
        blacks: 20.0,
        temp: 30.0,
        tint: -20.0,
        saturation: 40.0,
        ..Default::default()
    };
    for source in [SRGB, camera()] {
        let pipeline = PixelPipeline::new(&edited, &source);
        for rgb in PIXELS {
            let outside = develop(&pipeline, rgb, None);
            let mut previous = outside;
            for alpha in [1.0e-5, 1.0e-3, 0.01, 0.02, 0.03] {
                let mut delta = LocalDelta::default();
                delta.add(&full, alpha);
                let inside = develop(&pipeline, rgb, Some(&delta));
                let scale = outside.iter().cloned().fold(0.05, f32::max);
                if alpha < 1.0e-4 {
                    assert_close(inside, outside, 4e-4 * scale);
                }
                // No jump between neighbouring alphas either.
                assert_close(inside, previous, 0.06 * scale);
                previous = inside;
            }
        }
    }
}

#[test]
fn mask_adjustments_accumulate_into_a_delta() {
    let adjust = LocalAdjustments {
        exposure: 1.0,
        saturation: -40.0,
        temp: 10.0,
        clarity: 50.0,
        ..Default::default()
    };
    let mut delta = LocalDelta::default();
    assert!(delta.is_zero());
    // Two overlapping +1 EV masks add up to +2 EV.
    delta.add(&adjust, 1.0);
    delta.add(&adjust, 1.0);
    assert_eq!(
        (delta.exposure, delta.saturation, delta.temp),
        (2.0, -80.0, 20.0)
    );
    assert!(
        delta.affects_scene()
            && delta.affects_color()
            && delta.affects_input()
            && !delta.affects_tone()
    );
    // Half coverage of a mask at half amount contributes a quarter.
    let mut partial = LocalDelta::default();
    partial.add(&adjust, 0.25);
    assert_eq!(partial.exposure, 0.25);

    let pipeline = PixelPipeline::new(&DevelopSettings::default(), &WORKING);
    let lit = pipeline.scene_to_display([0.05; 3], Some(&delta));
    assert_close(lit, [0.2; 3], 1e-6);
    // A camera source takes local temperature as a relative warming too.
    let raw = PixelPipeline::new(&DevelopSettings::default(), &camera());
    let warmed = raw.input(raw_grey(0.3), Some(&delta));
    assert!(warmed[0] > warmed[1] && warmed[1] > warmed[2], "{warmed:?}");
}

#[test]
fn each_delta_field_moves_pixels_in_its_sliders_direction() {
    let pipeline = PixelPipeline::new(&DevelopSettings::default(), &WORKING);
    let grey = [0.18f32; 3];
    let with = |edit: fn(&mut LocalDelta)| {
        let mut delta = LocalDelta::default();
        edit(&mut delta);
        develop(&pipeline, grey, Some(&delta))
    };
    assert!(with(|d| d.exposure = 1.0)[1] > 0.3);
    assert!(with(|d| d.exposure = -1.0)[1] < 0.1);
    // Middle grey sits just below the contrast pivot (perceptual 0.5).
    assert!(with(|d| d.contrast = 60.0)[1] < grey[1]);
    assert!(with(|d| d.shadows = 60.0)[1] > grey[1]);
    assert!(with(|d| d.shadows = -60.0)[1] < grey[1]);
    assert!(with(|d| d.blacks = 60.0)[1] > grey[1]);
    let bright = |edit: fn(&mut LocalDelta)| {
        let mut delta = LocalDelta::default();
        edit(&mut delta);
        develop(&pipeline, [0.6; 3], Some(&delta))[1]
    };
    assert!(bright(|d| d.highlights = -60.0) < 0.6 && bright(|d| d.highlights = 60.0) > 0.6);
    assert!(bright(|d| d.whites = -60.0) < 0.6 && bright(|d| d.whites = 60.0) > 0.6);
    let warm = with(|d| d.temp = 40.0);
    assert!(warm[0] > warm[1] && warm[1] > warm[2], "{warm:?}");
    let magenta = with(|d| d.tint = 40.0);
    assert!(
        magenta[1] < magenta[0] && magenta[1] < magenta[2],
        "{magenta:?}"
    );
    let coloured = [0.3, 0.2, 0.1];
    let mut delta = LocalDelta {
        saturation: -100.0,
        ..Default::default()
    };
    let drained = develop(&pipeline, coloured, Some(&delta));
    assert!(drained[0] == drained[1] && drained[1] == drained[2]);
    delta.saturation = 50.0;
    let boosted = develop(&pipeline, coloured, Some(&delta));
    assert!(boosted[0] - boosted[2] > coloured[0] - coloured[2]);
}

#[test]
fn the_tone_base_is_needed_exactly_when_highlights_or_shadows_are_in_play() {
    assert!(PixelPipeline::new(&settings(|s| s.tone.shadows = 20.0), &WORKING).tone_needs_base());
    assert!(!PixelPipeline::new(&settings(|s| s.tone.contrast = 20.0), &WORKING).tone_needs_base());
    let masked = |edit: fn(&mut Mask)| {
        let mut mask = Mask::from_shape(
            MaskShape::Linear {
                start: Point::new(0.5, 0.0),
                end: Point::new(0.5, 1.0),
            },
            1,
        );
        edit(&mut mask);
        PixelPipeline::new(&settings(|s| s.masks.push(mask)), &WORKING)
    };
    assert!(masked(|m| m.adjust.highlights = -30.0).tone_needs_base());
    assert!(!masked(|m| m.adjust.exposure = 1.0).tone_needs_base());
    assert!(
        !masked(|m| {
            m.adjust.highlights = -30.0;
            m.enabled = false;
        })
        .tone_needs_base()
    );
    // Masks never change the global identity answers.
    assert!(masked(|m| m.adjust.highlights = -30.0).tone_is_identity());
}

#[test]
fn a_compiled_pipeline_can_be_shared_between_render_threads() {
    fn shareable<T: Send + Sync>() {}
    shareable::<PixelPipeline>();
    shareable::<LocalDelta>();
    // Including the path that builds a table on first use.
    let pipeline = PixelPipeline::new(&settings(|s| s.tone.contrast = 20.0), &SRGB);
    let delta = LocalDelta {
        contrast: 30.0,
        ..Default::default()
    };
    let expected = pipeline.tone([0.2, 0.3, 0.4], 0.5, Some(&delta));
    std::thread::scope(|scope| {
        for _ in 0..4 {
            scope.spawn(|| assert_eq!(pipeline.tone([0.2, 0.3, 0.4], 0.5, Some(&delta)), expected));
        }
    });
}
