//! R4: the same inputs give the same bytes on one thread, on several, and
//! on the default pool.

use photo_core::colorimetry::RgbSpace;
use photo_core::develop::{
    BrushStroke, Crop, LensCorrections, Mask, MaskComponent, MaskShape, Orientation, Spot,
    SpotMode, Transform,
};
use photo_core::geom::Point;
use photo_core::image::Rgba8Image;
use photo_imaging::filter::{
    box_blur, fast_guided_filter, fast_self_guided_filter, gaussian_blur, gradient_magnitude,
    guided_filter, high_pass, min_filter, self_guided_filter, wavelet_decompose, wavelet_denoise,
};
use photo_imaging::{
    Geometry, Plane, RgbImage, SourcePyramid, View, downscale_rgba8, grain_plane, heal_image,
    heal_level, histogram, rasterize_mask, resample, suggest_heal_source,
};

use crate::util::{Rng, noise_plane, noise_source, pools};

/// Run `work` on a one-thread pool, a seven-thread pool and the default
/// pool, and require all three results to be equal.
fn same_on_any_pool<T: PartialEq + Send>(name: &str, work: impl Fn() -> T + Sync) {
    let (single, many) = pools();
    let one = single.install(&work);
    let several = many.install(&work);
    let default = work();
    assert!(
        one == several,
        "{name}: one thread and seven threads disagree"
    );
    assert!(
        one == default,
        "{name}: one thread and the default pool disagree"
    );
}

/// Bit patterns, so that not even the sign of a zero may differ.
fn bits(plane: &Plane) -> Vec<u32> {
    plane.as_slice().iter().map(|v| v.to_bits()).collect()
}

fn image_bits(image: &RgbImage) -> Vec<u32> {
    image.as_slice().iter().map(|v| v.to_bits()).collect()
}

fn warped(width: u32, height: u32) -> Geometry {
    Geometry::new(
        width,
        height,
        &LensCorrections {
            distortion: -35.0,
            ..Default::default()
        },
        Orientation::new(1, false),
        &Transform {
            vertical: 20.0,
            horizontal: -15.0,
            rotate: 2.0,
            ..Default::default()
        },
        &Crop {
            angle: -5.0,
            ..Default::default()
        },
    )
}

#[test]
fn the_pyramid_and_resampling_do_not_depend_on_the_thread_count() {
    let source = noise_source(700, 500, 1);
    same_on_any_pool("pyramid", || {
        let pyramid = SourcePyramid::new(source.clone());
        (1..pyramid.level_count())
            .map(|level| pyramid.level(level).pixels.clone())
            .collect::<Vec<_>>()
    });
    let pyramid = SourcePyramid::new(source);
    let geometry = warped(700, 500);
    for max_edge in [2000, 451, 97] {
        let view = View::fit(&geometry, max_edge);
        same_on_any_pool("resample", || {
            image_bits(&resample(&pyramid, &geometry, &view, 7))
        });
    }
}

#[test]
fn filters_do_not_depend_on_the_thread_count() {
    let plane = noise_plane(613, 407, 2);
    let guide = noise_plane(613, 407, 3);
    same_on_any_pool("box", || bits(&box_blur(&plane, 6.3)));
    same_on_any_pool("gaussian", || bits(&gaussian_blur(&plane, 4.4)));
    same_on_any_pool("guided", || bits(&self_guided_filter(&plane, 5.5, 0.01)));
    same_on_any_pool("cross guided", || {
        bits(&guided_filter(&guide, &plane, 5.5, 0.01))
    });
    same_on_any_pool("fast guided", || {
        bits(&fast_self_guided_filter(&plane, 37.0, 0.01, (-11, 5)))
    });
    same_on_any_pool("fast cross guided", || {
        bits(&fast_guided_filter(&guide, &plane, 37.0, 0.01, (3, -8)))
    });
    same_on_any_pool("min", || bits(&min_filter(&plane, 9)));
    same_on_any_pool("wavelet", || {
        bits(&wavelet_denoise(&plane, &[0.2, 0.1, 0.05, 0.02]))
    });
    same_on_any_pool("decompose", || {
        let decomposition = wavelet_decompose(&plane, 3);
        decomposition
            .details()
            .iter()
            .chain([decomposition.residual()])
            .flat_map(bits)
            .collect::<Vec<_>>()
    });
    same_on_any_pool("high pass", || bits(&high_pass(&plane, 1.3)));
    same_on_any_pool("gradient", || bits(&gradient_magnitude(&plane)));
    same_on_any_pool("map", || {
        bits(&plane.zip_map(&guide, |a, b| a * b).map(|v| v.sqrt()))
    });
}

#[test]
fn masks_do_not_depend_on_the_thread_count() {
    let mut rng = Rng::new(4);
    let scribble = BrushStroke {
        points: (0..80)
            .map(|_| Point::new(rng.unit(), rng.unit()))
            .collect(),
        radius: 0.03,
        feather: 40.0,
        flow: 70.0,
        erase: false,
    };
    let rubber = BrushStroke {
        points: (0..20)
            .map(|_| Point::new(rng.unit(), rng.unit()))
            .collect(),
        radius: 0.05,
        feather: 0.0,
        flow: 100.0,
        erase: true,
    };
    let mask = Mask {
        invert: true,
        components: vec![
            MaskComponent {
                shape: MaskShape::Brush {
                    strokes: vec![scribble, rubber],
                },
                subtract: false,
            },
            MaskComponent {
                shape: MaskShape::Radial {
                    center: Point::new(0.4, 0.6),
                    radius_x: 0.3,
                    radius_y: 0.2,
                    angle: 33.0,
                    feather: 60.0,
                    invert: false,
                },
                subtract: true,
            },
            MaskComponent {
                shape: MaskShape::Linear {
                    start: Point::new(0.1, 0.1),
                    end: Point::new(0.8, 0.5),
                },
                subtract: false,
            },
        ],
        ..Default::default()
    };
    let geometry = warped(900, 600);
    for max_edge in [900, 300] {
        let view = View::fit(&geometry, max_edge);
        same_on_any_pool("mask", || bits(&rasterize_mask(&mask, &geometry, &view, 5)));
    }
}

#[test]
fn healing_does_not_depend_on_the_thread_count() {
    let source = noise_source(640, 480, 5);
    let mut rng = Rng::new(6);
    let spots: Vec<Spot> = (0..12)
        .map(|index| Spot {
            mode: if index % 2 == 0 {
                SpotMode::Heal
            } else {
                SpotMode::Clone
            },
            target: Point::new(rng.unit(), rng.unit()),
            source: Point::new(rng.unit(), rng.unit()),
            radius: 0.01 + 0.08 * rng.unit(),
            feather: 100.0 * rng.unit(),
            opacity: 30.0 + 70.0 * rng.unit(),
        })
        .collect();
    same_on_any_pool("heal", || heal_image(&source, &spots).pixels);
    let pyramid = SourcePyramid::new(source);
    same_on_any_pool("heal level", || heal_level(&pyramid, 2, &spots).pixels);
    same_on_any_pool("suggest", || {
        let suggestion = suggest_heal_source(&pyramid, Point::new(0.3, 0.7), 0.04);
        (suggestion.x.to_bits(), suggestion.y.to_bits())
    });
}

#[test]
fn histogram_grain_and_downscale_do_not_depend_on_the_thread_count() {
    let mut rng = Rng::new(7);
    let image = Rgba8Image {
        width: 811,
        height: 607,
        pixels: (0..811 * 607 * 4)
            .map(|_| (rng.next_u64() >> 56) as u8)
            .collect(),
        space: RgbSpace::DisplayP3,
    };
    same_on_any_pool("histogram", || histogram(&image));
    same_on_any_pool("downscale", || downscale_rgba8(&image, 233, 175).pixels);
    let geometry = Geometry::identity(3000, 2000);
    let view = View::fit(&geometry, 700);
    same_on_any_pool("grain", || bits(&grain_plane(&view, 2.2, 3)));
}
