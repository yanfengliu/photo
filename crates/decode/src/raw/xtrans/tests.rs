//! The interpolation against scenes whose every pixel is known.

use std::f32::consts::TAU;

use super::*;

const X_TRANS: &str = "GGRGGBGGBGGRBRGRBGGGBGGRGGRGGBRBGBRG";

type Scene = fn(f32, f32) -> [f32; 3];

/// The samples of a frame whose site at `(x, y)` records `scene(x, y)`
/// through its colour filter.
fn mosaic(
    cfa: &CFA,
    width: usize,
    height: usize,
    scene: impl Fn(usize, usize) -> [f32; 3],
) -> Vec<f32> {
    (0..height)
        .flat_map(|y| (0..width).map(move |x| (x, y)))
        .map(|(x, y)| scene(x, y)[cfa.color_at(y, x)])
        .collect()
}

fn whole(width: usize, height: usize) -> Area {
    Area {
        x: 0,
        y: 0,
        width,
        height,
    }
}

/// How far an interpolated picture is from the scene it was sampled from.
#[derive(Debug)]
struct Errors {
    /// Mean absolute error over all channels.
    mean: f32,
    /// The largest error of any channel of any pixel.
    worst: f32,
    /// The share of channel values that are off by more than 0.02 of full
    /// scale: enough to see.
    visible: f32,
}

/// Interpolates a 120 x 90 X-Trans mosaic of `scene` and measures the
/// result, four pixels in from the border.
fn errors(scene: Scene) -> Errors {
    let (width, height) = (120usize, 90usize);
    let cfa = CFA::new(X_TRANS);
    let samples = mosaic(&cfa, width, height, |x, y| scene(x as f32, y as f32));
    let out = interpolate(&samples, width, whole(width, height), &cfa);
    let (mut total, mut worst, mut visible, mut count) = (0.0f64, 0.0f32, 0u32, 0u32);
    for y in 4..height - 4 {
        for x in 4..width - 4 {
            let wanted = scene(x as f32, y as f32);
            for (got, want) in out[y * width + x].iter().zip(wanted) {
                let error = (got - want).abs();
                total += f64::from(error);
                worst = worst.max(error);
                visible += u32::from(error > 0.02);
                count += 1;
            }
        }
    }
    Errors {
        mean: (total / f64::from(count)) as f32,
        worst,
        visible: visible as f32 / count as f32,
    }
}

/// A fine diagonal wave, nine pixels from crest to crest.
fn wave(x: f32, y: f32) -> f32 {
    0.35 + 0.25 * ((x * 0.866 + y * 0.5) * TAU / 9.0).sin()
}

/// 0 on one side of a tilted line, 1 on the other, with a ramp `width`
/// pixels wide between them (a hard step for a width of zero).
fn edge(x: f32, y: f32, width: f32) -> f32 {
    let distance = x * 0.94 + y * 0.34 - 70.3;
    if width == 0.0 {
        return if distance > 0.0 { 1.0 } else { 0.0 };
    }
    let t = (distance / width + 0.5).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[test]
fn a_flat_field_comes_back_flat_wherever_the_active_area_starts() {
    let colour = [0.2f32, 0.5, 0.3];
    let cfa = CFA::new(X_TRANS);
    let (frame_width, frame_height) = (30usize, 26usize);
    for area_y in 0..6 {
        for area_x in 0..6 {
            let area = Area {
                x: area_x,
                y: area_y,
                width: 20,
                height: 17,
            };
            // Everything outside the area is garbage that must never show.
            let samples = mosaic(&cfa, frame_width, frame_height, |x, y| {
                let inside =
                    (area_x..area_x + 20).contains(&x) && (area_y..area_y + 17).contains(&y);
                if inside { colour } else { [9.0; 3] }
            });
            let out = interpolate(&samples, frame_width, area, &cfa);
            assert_eq!(out.len(), 20 * 17);
            for (index, pixel) in out.iter().enumerate() {
                for (got, want) in pixel.iter().zip(colour) {
                    assert!(
                        (got - want).abs() < 1e-6,
                        "area at {area_x},{area_y}, pixel {index}: {pixel:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn measured_samples_pass_through_untouched() {
    let cfa = CFA::new(X_TRANS);
    let (width, height) = (43usize, 31usize);
    let busy = |x: usize, y: usize| [((x * 31 + y * 17) % 97) as f32 / 96.0; 3];
    let samples = mosaic(&cfa, width, height, busy);
    let out = interpolate(&samples, width, whole(width, height), &cfa);
    for y in 0..height {
        for x in 0..width {
            let measured = out[y * width + x][cfa.color_at(y, x)];
            assert_eq!(measured, samples[y * width + x], "site {x},{y}");
        }
    }
}

#[test]
fn interpolated_values_stay_within_a_third_of_the_range_of_the_measured_ones() {
    // The harshest mosaic there is: every sample is either 0 or 1. A cubic
    // through such samples reaches a third of their range beyond them, and
    // nothing may go further than that.
    let cfa = CFA::new(X_TRANS);
    let (width, height) = (50usize, 38usize);
    let harsh = |x: usize, y: usize| [f32::from(u8::from((x * x * 7 + y * 13 + x * y) % 5 < 2)); 3];
    let samples = mosaic(&cfa, width, height, harsh);
    let out = interpolate(&samples, width, whole(width, height), &cfa);
    assert_eq!(out.len(), width * height);
    let allowed = -1.0 / 3.0 - 1e-5..=4.0 / 3.0 + 1e-5;
    for (index, pixel) in out.iter().enumerate() {
        assert!(
            pixel.iter().all(|value| allowed.contains(value)),
            "pixel {index}: {pixel:?}"
        );
    }
}

#[test]
fn fine_smooth_detail_is_rebuilt_closely() {
    let neutral: Scene = |x, y| [wave(x, y); 3];
    let coloured: Scene = |x, y| {
        let level = wave(x, y);
        [
            level * (1.0 + 0.3 * (x / 23.0).sin()),
            level,
            level * (1.0 + 0.3 * (y / 19.0).cos()),
        ]
    };
    // (rawler's own X-Trans interpolation, an average over 5 x 5 pixels,
    // measured a mean error of 0.065 on this wave.)
    let found = errors(neutral);
    assert!(
        found.mean < 0.002 && found.worst < 0.05 && found.visible < 0.005,
        "neutral wave: {found:?}"
    );
    let found = errors(coloured);
    assert!(
        found.mean < 0.006 && found.worst < 0.1,
        "coloured wave: {found:?}"
    );
}

#[test]
fn a_neutral_edge_stays_neutral_and_sharp() {
    // The red and blue sites next to the edge cannot tell from their greens
    // which side they are on; their own samples settle it.
    let edges: [(&str, Scene); 3] = [
        ("hard", |x, y| [0.1 + 0.6 * edge(x, y, 0.0); 3]),
        ("1.5 pixels wide", |x, y| [0.1 + 0.6 * edge(x, y, 1.5); 3]),
        ("3 pixels wide", |x, y| [0.1 + 0.6 * edge(x, y, 3.0); 3]),
    ];
    for (name, scene) in edges {
        let found = errors(scene);
        assert!(
            found.mean < 2e-4 && found.worst < 0.08 && found.visible < 0.001,
            "{name}: {found:?}"
        );
    }
}

#[test]
fn a_coloured_edge_keeps_its_colours_apart() {
    // Red and green move in opposite directions across the edge: the worst
    // case for anything that interpolates colour differences. A few pixels
    // on the edge itself may be off; the two sides must not bleed.
    fn colours(t: f32) -> [f32; 3] {
        [0.1 + 0.5 * t, 0.4 - 0.2 * t, 0.15 + 0.3 * t]
    }
    let edges: [(&str, Scene); 2] = [
        ("hard", |x, y| colours(edge(x, y, 0.0))),
        ("3 pixels wide", |x, y| colours(edge(x, y, 3.0))),
    ];
    for (name, scene) in edges {
        let found = errors(scene);
        assert!(
            found.mean < 1.2e-3 && found.visible < 0.02,
            "{name}: {found:?}"
        );
    }
}

#[test]
fn areas_too_small_for_the_pattern_are_still_interpolated() {
    let cfa = CFA::new(X_TRANS);
    let colour = [0.2f32, 0.5, 0.3];
    for (width, height) in [
        (1usize, 1usize),
        (1, 7),
        (7, 1),
        (2, 3),
        (3, 3),
        (5, 5),
        (7, 40),
    ] {
        let samples = mosaic(&cfa, width, height, |_, _| colour);
        let out = interpolate(&samples, width, whole(width, height), &cfa);
        assert_eq!(out.len(), width * height, "{width} x {height}");
        for pixel in &out {
            // Where a colour was never measured nearby the pixel is left
            // neutral; nothing is ever invented outside the samples.
            assert!(
                pixel.iter().all(|value| (0.2..=0.5).contains(value)),
                "{width} x {height}: {pixel:?}"
            );
        }
    }
    assert!(interpolate(&[], 0, whole(0, 0), &cfa).is_empty());
}

#[test]
fn other_red_green_blue_patterns_are_accepted() {
    let colour = [0.2f32, 0.5, 0.3];
    for pattern in ["RGGB", "BGGR", "GRBG", "GBRG"] {
        let cfa = CFA::new(pattern);
        let samples = mosaic(&cfa, 16, 12, |_, _| colour);
        for pixel in interpolate(&samples, 16, whole(16, 12), &cfa) {
            for (got, want) in pixel.iter().zip(colour) {
                assert!((got - want).abs() < 1e-6, "{pattern}: {pixel:?}");
            }
        }
    }
}
