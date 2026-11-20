//! Wall-clock cost of the primitives on a preview-sized buffer.
//!
//! Not a pass/fail test: timings depend on the machine. Run it with
//! `cargo test -p photo-imaging -- --ignored --nocapture timings` and read
//! the table.

use std::time::Instant;

use photo_core::colorimetry::RgbSpace;
use photo_core::develop::{
    BrushStroke, Crop, LensCorrections, Mask, MaskComponent, MaskShape, Orientation, Spot,
    SpotMode, Transform,
};
use photo_core::geom::{Point, Rect};
use photo_core::image::Rgba8Image;
use photo_imaging::filter::{
    box_blur, fast_self_guided_filter, gaussian_blur, high_pass, min_filter, self_guided_filter,
    wavelet_denoise,
};
use photo_imaging::{
    Geometry, SourcePyramid, View, downscale_rgba8, grain_plane, heal_level, histogram,
    rasterize_mask, resample, resample_level,
};

use crate::util::{noise_plane, noise_source};

const WIDTH: usize = 2048;
const HEIGHT: usize = 1365;

/// Best of several runs, in milliseconds, after one warm-up run.
fn milliseconds<T>(work: impl Fn() -> T) -> f64 {
    std::hint::black_box(work());
    (0..7)
        .map(|_| {
            let start = Instant::now();
            std::hint::black_box(work());
            start.elapsed().as_secs_f64() * 1000.0
        })
        .fold(f64::INFINITY, f64::min)
}

fn report(name: &str, time: f64) {
    println!("{name:<44} {time:>8.2} ms");
}

#[test]
#[ignore = "prints timings; run on demand with --ignored --nocapture"]
fn timings_on_a_2048_by_1365_buffer() {
    println!(
        "\n{WIDTH} x {HEIGHT}, {} threads",
        rayon::current_num_threads()
    );
    let plane = noise_plane(WIDTH, HEIGHT, 1);
    report(
        "box blur, radius 10",
        milliseconds(|| box_blur(&plane, 10.0)),
    );
    report(
        "box blur, radius 100.5",
        milliseconds(|| box_blur(&plane, 100.5)),
    );
    report(
        "gaussian blur, sigma 5",
        milliseconds(|| gaussian_blur(&plane, 5.0)),
    );
    report(
        "gaussian blur, sigma 50",
        milliseconds(|| gaussian_blur(&plane, 50.0)),
    );
    report(
        "guided filter, radius 8",
        milliseconds(|| self_guided_filter(&plane, 8.0, 0.01)),
    );
    report(
        "guided filter, radius 60",
        milliseconds(|| self_guided_filter(&plane, 60.0, 0.01)),
    );
    report(
        "fast guided filter, radius 20",
        milliseconds(|| fast_self_guided_filter(&plane, 20.0, 0.01, (0, 0))),
    );
    report(
        "fast guided filter, radius 60",
        milliseconds(|| fast_self_guided_filter(&plane, 60.0, 0.01, (0, 0))),
    );
    report(
        "min filter, radius 7",
        milliseconds(|| min_filter(&plane, 7)),
    );
    report(
        "wavelet denoise, 4 levels",
        milliseconds(|| wavelet_denoise(&plane, &[0.1, 0.05, 0.02, 0.01])),
    );
    report(
        "high pass, sigma 1",
        milliseconds(|| high_pass(&plane, 1.0)),
    );

    // A 24 MP source seen fit-to-window, and a 1:1 window onto it.
    let source = noise_source(6000, 4000, 2);
    let pyramid = SourcePyramid::new(source);
    let upright = Geometry::identity(6000, 4000);
    let fit = View::fit(&upright, WIDTH as u32);
    assert_eq!(
        (fit.width() as usize, fit.height() as usize),
        (WIDTH, HEIGHT)
    );
    let window = View::new(
        Rect::new(1000.0, 1000.0, 3048.0, 2365.0),
        WIDTH as u32,
        HEIGHT as u32,
    );
    let warped = Geometry::new(
        6000,
        4000,
        &LensCorrections {
            distortion: 20.0,
            ..Default::default()
        },
        Orientation::IDENTITY,
        &Transform {
            vertical: -20.0,
            rotate: 1.5,
            ..Default::default()
        },
        &Crop {
            angle: 3.0,
            ..Default::default()
        },
    );
    let warped_fit = View::fit(&warped, WIDTH as u32);
    report(
        "pyramid of a 6000 x 4000 source",
        milliseconds(|| SourcePyramid::new(pyramid.level(0).clone())),
    );
    report(
        "resample, fit view (level 1, mixed kernel)",
        milliseconds(|| resample(&pyramid, &upright, &fit, 0)),
    );
    report(
        "resample, 1:1 view (level 0, cubic)",
        milliseconds(|| resample(&pyramid, &upright, &window, 0)),
    );
    report(
        "resample, fit view, perspective + lens",
        milliseconds(|| resample(&pyramid, &warped, &warped_fit, 0)),
    );

    let stroke = BrushStroke {
        points: (0..200)
            .map(|i| {
                let t = i as f32 / 199.0;
                Point::new(0.1 + 0.8 * t, 0.5 + 0.3 * (t * 12.0).sin())
            })
            .collect(),
        radius: 0.04,
        feather: 50.0,
        flow: 80.0,
        erase: false,
    };
    let linear = Mask {
        components: vec![MaskComponent {
            shape: MaskShape::Linear {
                start: Point::new(0.5, 0.2),
                end: Point::new(0.5, 0.7),
            },
            subtract: false,
        }],
        ..Default::default()
    };
    let brushed = Mask {
        components: vec![MaskComponent {
            shape: MaskShape::Brush {
                strokes: vec![stroke],
            },
            subtract: false,
        }],
        ..Default::default()
    };
    report(
        "mask, linear gradient, fit view",
        milliseconds(|| rasterize_mask(&linear, &warped, &warped_fit, 0)),
    );
    report(
        "mask, long brush stroke, fit view",
        milliseconds(|| rasterize_mask(&brushed, &warped, &warped_fit, 0)),
    );
    report(
        "mask, long brush stroke, 1:1 view",
        milliseconds(|| rasterize_mask(&brushed, &upright, &window, 0)),
    );

    let spots: Vec<Spot> = (0..10)
        .map(|i| Spot {
            mode: SpotMode::Heal,
            target: Point::new(0.1 + 0.08 * i as f32, 0.3 + 0.04 * i as f32),
            source: Point::new(0.15 + 0.08 * i as f32, 0.35 + 0.04 * i as f32),
            radius: 0.01,
            feather: 50.0,
            opacity: 100.0,
        })
        .collect();
    let level = resample_level(&pyramid, &upright, &fit);
    report(
        "heal, 10 spots of 1%, fit view's level",
        milliseconds(|| heal_level(&pyramid, level, &spots)),
    );
    report(
        "heal, 10 spots of 1%, full resolution",
        milliseconds(|| heal_level(&pyramid, 0, &spots)),
    );

    report(
        "grain, fit view",
        milliseconds(|| grain_plane(&fit, 2.0, 0)),
    );
    let rendered = Rgba8Image {
        width: WIDTH as u32,
        height: HEIGHT as u32,
        pixels: (0..WIDTH * HEIGHT * 4)
            .map(|i| (i * 31 % 251) as u8)
            .collect(),
        space: RgbSpace::Srgb,
    };
    report("histogram", milliseconds(|| histogram(&rendered)));
    report(
        "downscale to 384 x 256",
        milliseconds(|| downscale_rgba8(&rendered, 384, 256)),
    );
}
