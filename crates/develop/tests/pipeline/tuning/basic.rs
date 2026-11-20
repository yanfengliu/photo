//! Sheets for the Basic panel: tone, the recovery recipe, the step wedge,
//! highlight headroom, white balance and the profiles.

use std::path::Path;

use photo_core::colorimetry::RgbSpace;
use photo_core::develop::{DevelopSettings, Profile, Tone, WbPreset, WhiteBalance};
use photo_develop::{Renderer, Source, as_shot_white_balance};

use super::{Scenes, Sheet, actual, edit, noise, shot, thumb};
use crate::fixtures::{camera_source, full, raster_linear, render16};

const SLIDERS: [&str; 5] = ["contrast", "highlights", "shadows", "whites", "blacks"];

fn with_slider(slider: &str, value: f32) -> DevelopSettings {
    edit(|s| match slider {
        "exposure" => s.tone.exposure = value,
        "contrast" => s.tone.contrast = value,
        "highlights" => s.tone.highlights = value,
        "shadows" => s.tone.shadows = value,
        "whites" => s.tone.whites = value,
        _ => s.tone.blacks = value,
    })
}

/// The six-slider "endpoint recovery" recipe.
pub fn recipe() -> DevelopSettings {
    edit(|s| {
        s.tone = Tone {
            exposure: 0.30,
            contrast: 6.0,
            highlights: -100.0,
            shadows: 100.0,
            whites: 30.0,
            blacks: -30.0,
        };
        s.presence.vibrance = 15.0;
        s.presence.saturation = -3.0;
    })
}

pub fn sheets(scenes: &Scenes, dir: &Path) {
    for (name, source) in [("raster", &scenes.raster), ("camera", &scenes.camera)] {
        tone(name, source, dir);
    }
    let mut renderer = Renderer::new();
    let mut sheet = Sheet::new("RECOVERY RECIPE: DEFAULT / RECIPE", 2);
    for source in [&scenes.raster, &scenes.camera] {
        sheet.add(
            "DEFAULT",
            thumb(&mut renderer, source, &DevelopSettings::default()),
        );
        sheet.add("RECIPE", thumb(&mut renderer, source, &recipe()));
    }
    sheet.save(dir, "02_recipe");
    wedge(dir);
    headroom(dir);
    white_balance(scenes, dir);
    let mut sheet = Sheet::new("PROFILES (CAMERA)", 4);
    for profile in Profile::ALL {
        let settings = edit(|s| s.profile = profile);
        sheet.add(
            profile.label(),
            thumb(&mut renderer, &scenes.camera, &settings),
        );
    }
    sheet.save(dir, "05_profiles");
}

fn tone(name: &str, source: &Source, dir: &Path) {
    let mut renderer = Renderer::new();
    let mut sheet = Sheet::new(&format!("TONE SLIDERS ({name})"), 5);
    for ev in [-2.0, -1.0, 0.0, 1.0, 2.0] {
        let settings = with_slider("exposure", ev);
        sheet.add(
            format!("EXPOSURE {ev:+}"),
            thumb(&mut renderer, source, &settings),
        );
    }
    for slider in SLIDERS {
        for value in [-100.0, -50.0, 0.0, 50.0, 100.0] {
            let settings = with_slider(slider, value);
            sheet.add(
                format!("{slider} {value:+}"),
                thumb(&mut renderer, source, &settings),
            );
        }
    }
    sheet.save(dir, &format!("01_tone_{name}"));
}

/// Perceptual values of the wedge's steps.
const STEPS: usize = 16;
const STEP_WIDTH: u32 = 40;

/// A neutral step wedge: raster steps evenly spaced in perceptual value,
/// camera steps half a stop apart from eight stops under the clip point.
fn wedges() -> [(&'static str, Source); 2] {
    let (width, height) = (STEP_WIDTH * STEPS as u32, 90);
    let step = |x: u32| (x / STEP_WIDTH) as usize;
    let raster = raster_linear(width, height, |x, _| {
        [(step(x) as f32 / (STEPS - 1) as f32).powf(2.2); 3]
    });
    let camera = camera_source(width, height, 0.25, |x, _| {
        [2.0f32.powf(-7.5 + 0.5 * step(x) as f32); 3]
    });
    [("raster", raster), ("camera", camera)]
}

/// Each step's perceptual value, from the middle of the step.
fn steps(source: &Source, settings: &DevelopSettings) -> Vec<f32> {
    let image = render16(source, settings, full(source, settings));
    (0..STEPS as u32)
        .map(|step| {
            let [_, g, _] = image.pixel(step * STEP_WIDTH + STEP_WIDTH / 2, 45);
            RgbSpace::Srgb
                .decode(f32::from(g) / 65535.0)
                .powf(1.0 / 2.2)
        })
        .collect()
}

/// One tone slider at a time at ±100 over a neutral step wedge, to see
/// that each acts on its own zone. Also printed as shifts per step.
fn wedge(dir: &Path) {
    let mut renderer = Renderer::new();
    let mut sheet = Sheet::new("STEP WEDGE: EACH TONE SLIDER ALONE", 2);
    for (name, source) in wedges() {
        let base = steps(&source, &DevelopSettings::default());
        println!("\n{name} wedge, default: {}", row(&base));
        for slider in SLIDERS {
            for value in [-100.0, 100.0] {
                let settings = with_slider(slider, value);
                let view = full(&source, &settings);
                sheet.add(
                    format!("{name} {slider} {value:+}"),
                    shot(&mut renderer, &source, &settings, view),
                );
                let moved: Vec<f32> = steps(&source, &settings)
                    .iter()
                    .zip(&base)
                    .map(|(after, before)| after - before)
                    .collect();
                println!("{name} {slider:>10} {value:+4}: {}", row(&moved));
            }
        }
        let view = full(&source, &DevelopSettings::default());
        sheet.add(
            format!("{name} default"),
            shot(&mut renderer, &source, &DevelopSettings::default(), view),
        );
        sheet.end_row();
    }
    sheet.save(dir, "03_wedge");
}

fn row(values: &[f32]) -> String {
    values
        .iter()
        .map(|v| format!("{v:+.3}"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A camera source whose top half is textured highlights running from
/// nominal white to two stops above it (the file asks for a baseline
/// exposure that puts nominal white two stops under the sensor's clip),
/// and whose bottom half is a textured ramp of mid and dark tones.
fn headroom_scene() -> Source {
    let (width, height) = super::SCENE;
    camera_source(width, height, 1.6, |x, y| {
        let u = x as f32 / width as f32;
        let ripple = (x as f32 * 1.3).sin() * (y as f32 * 0.9).sin();
        let texture = 1.0 + 0.05 * ripple + 0.06 * noise(x, y, 0);
        let level = if y < height / 2 {
            0.24 * 4.0f32.powf(u)
        } else {
            0.002 * 100.0f32.powf(u)
        };
        [(level * texture).min(0.985); 3]
    })
}

fn headroom(dir: &Path) {
    let source = headroom_scene();
    let mut renderer = Renderer::new();
    let mut sheet = Sheet::new("HEADROOM: 2 STOPS OVER NOMINAL WHITE (CAMERA)", 4);
    let cases = [
        ("DEFAULT", DevelopSettings::default()),
        ("HIGHLIGHTS -100", edit(|s| s.tone.highlights = -100.0)),
        ("WHITES -100", edit(|s| s.tone.whites = -100.0)),
        ("RECIPE", recipe()),
    ];
    for (label, settings) in &cases {
        sheet.add(*label, thumb(&mut renderer, &source, settings));
        // The texture's contrast in the top two stops: the standard
        // deviation of perceptual values over a 1:1 patch, per band.
        let image = render16(&source, settings, full(&source, settings));
        let spread = |x0: u32| {
            let values: Vec<f32> = (100..300)
                .flat_map(|y| (x0..x0 + 100).map(move |x| (x, y)))
                .map(|(x, y)| {
                    RgbSpace::Srgb
                        .decode(f32::from(image.pixel(x, y)[1]) / 65535.0)
                        .powf(1.0 / 2.2)
                })
                .collect();
            let mean = values.iter().sum::<f32>() / values.len() as f32;
            let variance =
                values.iter().map(|v| (v - mean).powi(2)).sum::<f32>() / values.len() as f32;
            (mean, variance.sqrt())
        };
        let (low, high) = (spread(400), spread(1300));
        println!(
            "headroom {label:>16}: 1 stop over: mean {:.3} texture {:.4}; 2 stops over: mean {:.3} texture {:.4}",
            low.0, low.1, high.0, high.1
        );
    }
    for (label, settings) in &cases {
        sheet.add(
            format!("1:1 {label}"),
            actual(&mut renderer, &source, settings, 1100.0, 200.0),
        );
    }
    for (label, settings) in &cases {
        sheet.add(
            format!("1:1 MID {label}"),
            actual(&mut renderer, &source, settings, 500.0, 200.0),
        );
    }
    sheet.save(dir, "04_headroom");
}

fn custom(temp: f32, tint: f32) -> WhiteBalance {
    WhiteBalance {
        preset: WbPreset::Custom,
        temp,
        tint,
    }
}

fn white_balance(scenes: &Scenes, dir: &Path) {
    let mut renderer = Renderer::new();
    let mut sheet = Sheet::new("WHITE BALANCE SWEEPS", 5);
    for value in [-100.0, -50.0, 0.0, 50.0, 100.0] {
        let settings = edit(|s| s.white_balance = custom(value, 0.0));
        sheet.add(
            format!("RASTER TEMP {value:+}"),
            thumb(&mut renderer, &scenes.raster, &settings),
        );
    }
    for value in [-100.0, -50.0, 0.0, 50.0, 100.0] {
        let settings = edit(|s| s.white_balance = custom(0.0, value));
        sheet.add(
            format!("RASTER TINT {value:+}"),
            thumb(&mut renderer, &scenes.raster, &settings),
        );
    }
    let (kelvin, tint) = as_shot_white_balance(&scenes.camera).unwrap();
    for temp in [2500.0, 3800.0, kelvin, 7000.0, 12000.0] {
        let settings = edit(|s| s.white_balance = custom(temp, tint));
        sheet.add(
            format!("CAMERA {temp:.0}K"),
            thumb(&mut renderer, &scenes.camera, &settings),
        );
    }
    for offset in [-60.0, -30.0, 0.0, 30.0, 60.0] {
        let settings = edit(|s| s.white_balance = custom(kelvin, tint + offset));
        sheet.add(
            format!("CAMERA TINT {:+.0}", tint + offset),
            thumb(&mut renderer, &scenes.camera, &settings),
        );
    }
    sheet.save(dir, "06_white_balance");
}
