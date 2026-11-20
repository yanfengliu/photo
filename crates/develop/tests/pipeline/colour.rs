//! Direction tests for white balance and the colour panels.

use photo_core::develop::{ColorBand, DevelopSettings, Treatment, WbPreset, WhiteBalance};
use photo_core::image::Rgba8Image;
use photo_develop::Source;
use photo_testkit::{Patch, patches};

use crate::fixtures::{camera_scene, chroma, edit, full, mean, raster, render};
use crate::tone::wedge;

const WIDTH: u32 = 480;
const HEIGHT: u32 = 320;

fn patch_mean(image: &Rgba8Image, patch: &Patch) -> [f32; 3] {
    mean(image, patch.x, patch.y, patch.width, patch.height)
}

/// The scene's chart patches and its hazy sky, as rendered.
struct Look {
    patches: Vec<[f32; 3]>,
    sky: [f32; 3],
    soil: [f32; 3],
}

fn look(source: &Source, settings: &DevelopSettings) -> Look {
    let image = render(source, settings, full(source, settings));
    Look {
        patches: patches(WIDTH, HEIGHT)
            .iter()
            .map(|p| patch_mean(&image, p))
            .collect(),
        sky: mean(&image, WIDTH / 10, HEIGHT * 2 / 5, WIDTH / 5, HEIGHT / 10),
        soil: mean(
            &image,
            WIDTH / 10,
            HEIGHT * 59 / 100,
            WIDTH / 5,
            HEIGHT / 30,
        ),
    }
}

const RED: usize = 0;
const BLUE: usize = 2;

fn custom(temp: f32, tint: f32) -> WhiteBalance {
    WhiteBalance {
        preset: WbPreset::Custom,
        temp,
        tint,
    }
}

#[test]
fn temperature_warms_and_tint_turns_magenta() {
    let source = wedge();
    let grey = |settings: &DevelopSettings| {
        let image = render(&source, settings, full(&source, settings));
        mean(&image, 180, 40, 40, 40)
    };
    let (warm, cool) = (
        grey(&edit(|s| s.white_balance = custom(50.0, 0.0))),
        grey(&edit(|s| s.white_balance = custom(-50.0, 0.0))),
    );
    assert!(
        warm[0] > warm[2] + 5.0 && cool[2] > cool[0] + 5.0,
        "{warm:?} {cool:?}"
    );
    let magenta = grey(&edit(|s| s.white_balance = custom(0.0, 50.0)));
    assert!(magenta[1] + 3.0 < magenta[0].min(magenta[2]), "{magenta:?}");

    // A camera source: a higher kelvin setting renders warmer.
    let camera = camera_scene(WIDTH, HEIGHT);
    let at = |kelvin: f32| look(&camera, &edit(|s| s.white_balance = custom(kelvin, 0.0))).sky;
    let (hot, cold) = (at(8000.0), at(3500.0));
    assert!(
        hot[0] - hot[2] > cold[0] - cold[2] + 20.0,
        "{hot:?} {cold:?}"
    );
}

#[test]
fn saturation_and_vibrance_scale_colour() {
    let (_, source) = raster(WIDTH, HEIGHT);
    let before = look(&source, &DevelopSettings::default());
    let more = look(&source, &edit(|s| s.presence.saturation = 50.0));
    assert!(chroma(more.patches[RED]) > chroma(before.patches[RED]) + 3.0);
    let grey = look(&source, &edit(|s| s.presence.saturation = -100.0));
    for colour in grey.patches.iter().chain([&grey.sky]) {
        assert!(chroma(*colour) <= 1.0, "{colour:?}");
    }
    // Vibrance raises a muted colour (the sky) more than a saturated one.
    let vibrant = look(&source, &edit(|s| s.presence.vibrance = 100.0));
    let gain = |after: [f32; 3], before: [f32; 3]| chroma(after) / chroma(before);
    let (sky, blue) = (
        gain(vibrant.sky, before.sky),
        gain(vibrant.patches[BLUE], before.patches[BLUE]),
    );
    assert!(
        sky > 1.1 && sky > blue + 0.05,
        "sky x{sky}, blue patch x{blue}"
    );
}

#[test]
fn the_mixer_moves_only_its_own_band() {
    let (_, source) = raster(WIDTH, HEIGHT);
    let before = look(&source, &DevelopSettings::default());
    let blue = ColorBand::Blue.index();
    let saturated = look(&source, &edit(|s| s.color_mixer.saturation[blue] = 100.0));
    assert!(chroma(saturated.patches[BLUE]) > chroma(before.patches[BLUE]) + 3.0);
    let red_change = (chroma(saturated.patches[RED]) - chroma(before.patches[RED])).abs();
    assert!(red_change < 1.5, "the red patch changed by {red_change}");
    let darker = look(&source, &edit(|s| s.color_mixer.luminance[blue] = -100.0));
    let lum = |c: [f32; 3]| 0.2126 * c[0] + 0.7152 * c[1] + 0.0722 * c[2];
    assert!(lum(darker.patches[BLUE]) < lum(before.patches[BLUE]) - 2.0);
    assert!((lum(darker.patches[RED]) - lum(before.patches[RED])).abs() < 1.0);
    // Red's hue +100 turns it toward orange.
    let red = ColorBand::Red.index();
    let turned = look(&source, &edit(|s| s.color_mixer.hue[red] = 100.0));
    let (g0, g1) = (before.patches[RED][1], turned.patches[RED][1]);
    assert!(g1 > g0 + 2.0, "green in the red patch {g0} -> {g1}");
}

#[test]
fn grading_tints_its_tonal_range_and_black_and_white_is_neutral() {
    let (_, source) = raster(WIDTH, HEIGHT);
    let before = look(&source, &DevelopSettings::default());
    let graded = look(
        &source,
        &edit(|s| {
            s.color_grading.shadows.hue = 220.0;
            s.color_grading.shadows.saturation = 80.0;
        }),
    );
    let blueness = |c: [f32; 3]| c[2] - c[0];
    assert!(
        blueness(graded.soil) > blueness(before.soil) + 2.0,
        "{:?} {:?}",
        before.soil,
        graded.soil
    );
    let mono = look(&source, &edit(|s| s.treatment = Treatment::BlackAndWhite));
    for colour in mono.patches.iter().chain([&mono.sky, &mono.soil]) {
        assert!(chroma(*colour) <= 1.0, "{colour:?}");
    }
    let mixed = edit(|s| {
        s.treatment = Treatment::BlackAndWhite;
        s.color_mixer.bw_mix[ColorBand::Blue.index()] = -80.0;
    });
    let darker_blue = look(&source, &mixed);
    assert!(darker_blue.patches[BLUE][1] < mono.patches[BLUE][1] - 2.0);
}

#[test]
fn calibration_tints_the_shadows_and_desaturates_a_primary() {
    let (_, source) = raster(WIDTH, HEIGHT);
    let before = look(&source, &DevelopSettings::default());
    let tinted = look(&source, &edit(|s| s.calibration.shadows_tint = 80.0));
    let magenta = |c: [f32; 3]| (c[0] + c[2]) / 2.0 - c[1];
    assert!(
        magenta(tinted.soil) > magenta(before.soil) + 1.0,
        "{:?} {:?}",
        before.soil,
        tinted.soil
    );
    let muted = look(&source, &edit(|s| s.calibration.red_saturation = -100.0));
    assert!(chroma(muted.patches[RED]) < chroma(before.patches[RED]) - 3.0);
}
