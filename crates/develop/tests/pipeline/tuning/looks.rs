//! Sheets for tone curves, colour, presence, detail and effects.

use std::path::Path;

use photo_core::develop::{ColorBand, CurvePoint, DevelopSettings, PointCurve, Treatment};
use photo_develop::Renderer;

use super::{Scenes, Sheet, actual, edit, thumb};

fn curve(points: &[(f32, f32)]) -> PointCurve {
    PointCurve::new(points.iter().map(|&(x, y)| CurvePoint::new(x, y)))
}

pub fn sheets(scenes: &Scenes, dir: &Path) {
    curves(scenes, dir);
    colour(scenes, dir);
    presence(scenes, dir);
    detail(scenes, dir);
    effects(scenes, dir);
}

fn curves(scenes: &Scenes, dir: &Path) {
    let mut renderer = Renderer::new();
    let mut sheet = Sheet::new("TONE CURVES", 4);
    let cases = [
        (
            "STRONG S CURVE",
            edit(|s| {
                s.tone_curve.rgb = curve(&[
                    (0.0, 0.0),
                    (0.25, 0.12),
                    (0.5, 0.5),
                    (0.75, 0.9),
                    (1.0, 1.0),
                ])
            }),
        ),
        (
            "PARAMETRIC",
            edit(|s| {
                let p = &mut s.tone_curve.parametric;
                (p.highlights, p.lights, p.darks, p.shadows) = (-80.0, 50.0, -50.0, 80.0);
            }),
        ),
        (
            "RED UP BLUE DOWN",
            edit(|s| {
                s.tone_curve.red = curve(&[(0.0, 0.0), (0.5, 0.62), (1.0, 1.0)]);
                s.tone_curve.blue = curve(&[(0.0, 0.0), (0.5, 0.4), (1.0, 1.0)]);
            }),
        ),
        ("CONTRAST +100", edit(|s| s.tone.contrast = 100.0)),
    ];
    for source in [&scenes.raster, &scenes.camera] {
        for (label, settings) in &cases {
            sheet.add(*label, thumb(&mut renderer, source, settings));
        }
    }
    sheet.save(dir, "07_curves");
}

fn colour(scenes: &Scenes, dir: &Path) {
    let mut renderer = Renderer::new();
    let mut sheet = Sheet::new("HSL, GRADING, BLACK AND WHITE (RASTER)", 6);
    for band in [ColorBand::Red, ColorBand::Blue] {
        let i = band.index();
        let name = band.label();
        for (what, value) in [
            ("HUE", -100.0),
            ("HUE", 100.0),
            ("SAT", -100.0),
            ("SAT", 100.0),
            ("LUM", -100.0),
            ("LUM", 100.0),
        ] {
            let settings = edit(|s| match what {
                "HUE" => s.color_mixer.hue[i] = value,
                "SAT" => s.color_mixer.saturation[i] = value,
                _ => s.color_mixer.luminance[i] = value,
            });
            sheet.add(
                format!("{name} {what} {value:+}"),
                thumb(&mut renderer, &scenes.raster, &settings),
            );
        }
    }
    let grades = [
        (
            "SHADOWS BLUE",
            edit(|s| {
                s.color_grading.shadows.hue = 220.0;
                s.color_grading.shadows.saturation = 60.0;
            }),
        ),
        (
            "HIGHLIGHTS ORANGE",
            edit(|s| {
                s.color_grading.highlights.hue = 40.0;
                s.color_grading.highlights.saturation = 60.0;
            }),
        ),
        (
            "MIDTONES TEAL",
            edit(|s| {
                s.color_grading.midtones.hue = 180.0;
                s.color_grading.midtones.saturation = 50.0;
            }),
        ),
        (
            "SPLIT, BALANCE +100",
            edit(|s| {
                s.color_grading.shadows.hue = 220.0;
                s.color_grading.shadows.saturation = 50.0;
                s.color_grading.highlights.hue = 40.0;
                s.color_grading.highlights.saturation = 50.0;
                s.color_grading.balance = 100.0;
            }),
        ),
        (
            "GLOBAL WARM, LUM -30",
            edit(|s| {
                s.color_grading.global.hue = 50.0;
                s.color_grading.global.saturation = 30.0;
                s.color_grading.global.luminance = -30.0;
            }),
        ),
        (
            "CALIBRATION",
            edit(|s| {
                s.calibration.red_hue = 60.0;
                s.calibration.blue_saturation = 60.0;
                s.calibration.shadows_tint = 40.0;
            }),
        ),
    ];
    for (label, settings) in &grades {
        sheet.add(*label, thumb(&mut renderer, &scenes.raster, settings));
    }
    let mono = [
        ("B&W", edit(|s| s.treatment = Treatment::BlackAndWhite)),
        (
            "B&W RED+ BLUE-",
            edit(|s| {
                s.treatment = Treatment::BlackAndWhite;
                s.color_mixer.bw_mix[ColorBand::Red.index()] = 100.0;
                s.color_mixer.bw_mix[ColorBand::Blue.index()] = -100.0;
            }),
        ),
        (
            "B&W + CONTRAST",
            edit(|s| {
                s.treatment = Treatment::BlackAndWhite;
                s.tone.contrast = 50.0;
            }),
        ),
        (
            "B&W + SPLIT TONE",
            edit(|s| {
                s.treatment = Treatment::BlackAndWhite;
                s.color_grading.shadows.hue = 220.0;
                s.color_grading.shadows.saturation = 40.0;
                s.color_grading.highlights.hue = 40.0;
                s.color_grading.highlights.saturation = 40.0;
            }),
        ),
        ("SATURATION -100", edit(|s| s.presence.saturation = -100.0)),
        (
            "CAMERA B&W",
            edit(|s| s.treatment = Treatment::BlackAndWhite),
        ),
    ];
    for (index, (label, settings)) in mono.iter().enumerate() {
        let source = if index == 5 {
            &scenes.camera
        } else {
            &scenes.raster
        };
        sheet.add(*label, thumb(&mut renderer, source, settings));
    }
    sheet.save(dir, "08_colour");
}

fn presence(scenes: &Scenes, dir: &Path) {
    let mut renderer = Renderer::new();
    let mut sheet = Sheet::new("PRESENCE AT -100 / 0 / +100 (FIT, THEN 1:1)", 6);
    for slider in ["texture", "clarity", "dehaze", "vibrance", "saturation"] {
        for value in [-100.0, 0.0, 100.0] {
            let settings = edit(|s| match slider {
                "texture" => s.presence.texture = value,
                "clarity" => s.presence.clarity = value,
                "dehaze" => s.presence.dehaze = value,
                "vibrance" => s.presence.vibrance = value,
                _ => s.presence.saturation = value,
            });
            sheet.add(
                format!("{slider} {value:+}"),
                thumb(&mut renderer, &scenes.raster, &settings),
            );
            sheet.add(
                format!("CAM {slider} {value:+}"),
                thumb(&mut renderer, &scenes.camera, &settings),
            );
        }
    }
    for slider in ["texture", "clarity", "dehaze"] {
        for value in [-100.0, 0.0, 100.0] {
            let settings = edit(|s| match slider {
                "texture" => s.presence.texture = value,
                "clarity" => s.presence.clarity = value,
                _ => s.presence.dehaze = value,
            });
            sheet.add(
                format!("1:1 {slider} {value:+}"),
                actual(&mut renderer, &scenes.raster, &settings, 900.0, 520.0),
            );
            sheet.add(
                format!("1:1 SKY {slider} {value:+}"),
                actual(&mut renderer, &scenes.raster, &settings, 1000.0, 120.0),
            );
        }
    }
    sheet.save(dir, "09_presence");
}

fn detail(scenes: &Scenes, dir: &Path) {
    let mut renderer = Renderer::new();
    let mut sheet = Sheet::new("DETAIL AT 1:1 (NOISY CAMERA SCENE)", 4);
    let noise = [
        ("NO NOISE REDUCTION", DevelopSettings::default()),
        (
            "LUMINANCE 50",
            edit(|s| s.detail.noise_reduction.luminance = 50.0),
        ),
        (
            "LUMINANCE 100",
            edit(|s| s.detail.noise_reduction.luminance = 100.0),
        ),
        (
            "LUM 100 DETAIL 100",
            edit(|s| {
                s.detail.noise_reduction.luminance = 100.0;
                s.detail.noise_reduction.luminance_detail = 100.0;
            }),
        ),
        (
            "COLOR 25 (RAW DEFAULT)",
            edit(|s| s.detail.noise_reduction.color = 25.0),
        ),
        (
            "COLOR 100",
            edit(|s| s.detail.noise_reduction.color = 100.0),
        ),
        (
            "COLOR 100 DETAIL 0",
            edit(|s| {
                s.detail.noise_reduction.color = 100.0;
                s.detail.noise_reduction.color_detail = 0.0;
            }),
        ),
        (
            "RAW DEFAULTS",
            DevelopSettings::default_for(photo_core::meta::FileKind::Raw),
        ),
    ];
    for (label, settings) in &noise {
        sheet.add(
            *label,
            actual(&mut renderer, &scenes.noisy, settings, 120.0, 700.0),
        );
    }
    let sharpen = [
        ("SHARPEN 0", DevelopSettings::default()),
        ("AMOUNT 70 R1", edit(|s| s.detail.sharpening.amount = 70.0)),
        (
            "AMOUNT 150 R1",
            edit(|s| s.detail.sharpening.amount = 150.0),
        ),
        (
            "AMOUNT 100 R3",
            edit(|s| {
                s.detail.sharpening.amount = 100.0;
                s.detail.sharpening.radius = 3.0;
            }),
        ),
        (
            "150 DETAIL 0",
            edit(|s| {
                s.detail.sharpening.amount = 150.0;
                s.detail.sharpening.detail = 0.0;
            }),
        ),
        (
            "150 DETAIL 100",
            edit(|s| {
                s.detail.sharpening.amount = 150.0;
                s.detail.sharpening.detail = 100.0;
            }),
        ),
        (
            "150 MASKING 80",
            edit(|s| {
                s.detail.sharpening.amount = 150.0;
                s.detail.sharpening.masking = 80.0;
            }),
        ),
        (
            "NEGATIVE (MASK -100)",
            edit(|s| {
                let mut mask = photo_core::develop::Mask::from_shape(
                    photo_core::develop::MaskShape::Linear {
                        start: photo_core::geom::Point::new(0.0, 0.5),
                        end: photo_core::geom::Point::new(0.01, 0.5),
                    },
                    1,
                );
                mask.adjust.sharpness = -100.0;
                mask.invert = true;
                s.masks.push(mask);
            }),
        ),
    ];
    for (label, settings) in &sharpen {
        sheet.add(
            *label,
            actual(&mut renderer, &scenes.raster, settings, 1150.0, 520.0),
        );
    }
    sheet.save(dir, "10_detail");
}

fn effects(scenes: &Scenes, dir: &Path) {
    let mut renderer = Renderer::new();
    let mut sheet = Sheet::new("EFFECTS: VIGNETTE AND GRAIN", 4);
    let vignettes = [
        (
            "VIGNETTE -100",
            edit(|s| s.effects.vignette.amount = -100.0),
        ),
        ("VIGNETTE -50", edit(|s| s.effects.vignette.amount = -50.0)),
        ("VIGNETTE +50", edit(|s| s.effects.vignette.amount = 50.0)),
        ("VIGNETTE +100", edit(|s| s.effects.vignette.amount = 100.0)),
        (
            "-80 ROUNDNESS -100",
            edit(|s| {
                s.effects.vignette.amount = -80.0;
                s.effects.vignette.roundness = -100.0;
            }),
        ),
        (
            "-80 ROUNDNESS +100",
            edit(|s| {
                s.effects.vignette.amount = -80.0;
                s.effects.vignette.roundness = 100.0;
            }),
        ),
        (
            "-80 FEATHER 0",
            edit(|s| {
                s.effects.vignette.amount = -80.0;
                s.effects.vignette.feather = 0.0;
            }),
        ),
        (
            "-80 HIGHLIGHTS 100",
            edit(|s| {
                s.effects.vignette.amount = -80.0;
                s.effects.vignette.highlights = 100.0;
            }),
        ),
    ];
    for (label, settings) in &vignettes {
        sheet.add(*label, thumb(&mut renderer, &scenes.raster, settings));
    }
    let grains = [
        ("GRAIN 50", edit(|s| s.effects.grain.amount = 50.0)),
        (
            "GRAIN 100 SIZE 80",
            edit(|s| {
                s.effects.grain.amount = 100.0;
                s.effects.grain.size = 80.0;
            }),
        ),
        (
            "GRAIN 100 ROUGH 0",
            edit(|s| {
                s.effects.grain.amount = 100.0;
                s.effects.grain.roughness = 0.0;
            }),
        ),
        (
            "GRAIN 100 ROUGH 100",
            edit(|s| {
                s.effects.grain.amount = 100.0;
                s.effects.grain.roughness = 100.0;
            }),
        ),
    ];
    for (label, settings) in &grains {
        sheet.add(*label, thumb(&mut renderer, &scenes.raster, settings));
    }
    for (label, settings) in &grains {
        sheet.add(
            format!("1:1 {label}"),
            actual(&mut renderer, &scenes.raster, settings, 300.0, 200.0),
        );
    }
    sheet.save(dir, "11_effects");
}
