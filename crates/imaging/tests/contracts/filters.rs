//! Spatial filters: references, what each one is for, and region
//! independence (R1) for every one of them.

use photo_imaging::Plane;
use photo_imaging::filter::{
    WaveletDecomposition, box_blur, box_support, fast_guided_filter, fast_guided_support,
    fast_self_guided_filter, gaussian_blur, gaussian_support, gradient_magnitude, guided_filter,
    guided_support, high_pass, min_filter, self_guided_filter, soft_threshold, wavelet_decompose,
    wavelet_denoise, wavelet_support,
};

use crate::util::{edge_height, flat_noise, max_abs_diff, mean, noise_plane, noisy_step, std_dev};

fn clamped(plane: &Plane, x: i64, y: i64) -> f64 {
    let cx = x.clamp(0, plane.width() as i64 - 1) as usize;
    let cy = y.clamp(0, plane.height() as i64 - 1) as usize;
    f64::from(plane.get(cx, cy))
}

/// The box filter by its definition: weight 1 within the whole part of the
/// radius, the fractional part on the next sample out, edges replicated.
fn box_reference(plane: &Plane, radius: f32) -> Plane {
    let whole = radius.floor() as i64;
    let fraction = f64::from(radius - radius.floor());
    let weight = |d: i64| if d.abs() <= whole { 1.0 } else { fraction };
    let norm = 2.0 * f64::from(radius) + 1.0;
    Plane::from_fn(plane.width(), plane.height(), |x, y| {
        let mut sum = 0.0;
        for dy in -(whole + 1)..=whole + 1 {
            for dx in -(whole + 1)..=whole + 1 {
                sum += weight(dx) * weight(dy) * clamped(plane, x as i64 + dx, y as i64 + dy);
            }
        }
        (sum / (norm * norm)) as f32
    })
}

#[test]
fn box_blur_equals_the_brute_force_reference() {
    let plane = noise_plane(37, 29, 1);
    for radius in [0.4f32, 1.0, 2.5, 7.0, 13.75, 40.0] {
        let diff = max_abs_diff(
            box_blur(&plane, radius).as_slice(),
            box_reference(&plane, radius).as_slice(),
        );
        assert!(diff < 1e-5, "radius {radius}: {diff}");
    }
}

#[test]
fn a_zero_radius_changes_nothing() {
    let plane = noise_plane(20, 15, 2);
    assert_eq!(box_blur(&plane, 0.0), plane);
    assert_eq!(box_blur(&plane, -3.0), plane);
    assert_eq!(gaussian_blur(&plane, 0.0), plane);
    assert_eq!(self_guided_filter(&plane, 0.0, 0.01), plane);
    assert_eq!(min_filter(&plane, 0), plane);
    assert_eq!(box_support(0.0), 0);
    assert_eq!(gaussian_support(0.0), 0);
}

#[test]
fn every_filter_leaves_a_flat_plane_flat() {
    let flat = Plane::filled(40, 30, 0.37);
    let same = |name: &str, out: Plane| {
        let diff = max_abs_diff(out.as_slice(), flat.as_slice());
        assert!(diff < 1e-6, "{name}: {diff}");
    };
    same("box", box_blur(&flat, 3.3));
    same("gaussian", gaussian_blur(&flat, 4.2));
    same("guided", self_guided_filter(&flat, 5.0, 1e-3));
    same(
        "fast guided",
        fast_self_guided_filter(&flat, 24.0, 1e-3, (0, 0)),
    );
    same("min", min_filter(&flat, 4));
    same("wavelet", wavelet_denoise(&flat, &[0.1, 0.1, 0.1]));
    assert!(
        high_pass(&flat, 2.0)
            .as_slice()
            .iter()
            .all(|v| v.abs() < 1e-6)
    );
    assert!(
        gradient_magnitude(&flat)
            .as_slice()
            .iter()
            .all(|v| v.abs() < 1e-6)
    );
}

#[test]
fn the_gaussian_has_unit_gain_and_the_requested_standard_deviation() {
    let size = 151;
    let mut impulse = Plane::new(size, size);
    impulse.set(75, 75, 1.0);
    for sigma in [0.5f32, 0.8, 1.5, 3.3, 7.0, 12.4] {
        let out = gaussian_blur(&impulse, sigma);
        let total: f64 = out.as_slice().iter().map(|&v| f64::from(v)).sum();
        assert!((total - 1.0).abs() < 1e-4, "sigma {sigma}: gain {total}");
        let mut variance = 0.0;
        for y in 0..size {
            for x in 0..size {
                variance += f64::from(out.get(x, y)) * (x as f64 - 75.0).powi(2);
            }
        }
        let expected = f64::from(sigma).powi(2);
        assert!(
            (variance - expected).abs() < 0.01 * expected,
            "sigma {sigma}: variance {variance}"
        );
        // Isotropic and symmetric.
        assert!((out.get(80, 75) - out.get(75, 80)).abs() < 1e-7);
        assert!((out.get(80, 75) - out.get(70, 75)).abs() < 1e-7);
        // Nothing lands outside the documented support.
        let support = gaussian_support(sigma);
        assert_eq!(out.get(75 + support + 1, 75), 0.0, "sigma {sigma}");
        assert!(
            support as f32 >= 2.12 * sigma,
            "sigma {sigma}: support {support}"
        );
    }
}

#[test]
fn three_box_passes_follow_the_true_gaussian() {
    // Three boxes make a quadratic B-spline, whose peak is 6% below the
    // Gaussian of the same variance; everywhere else it is closer.
    let size = 101;
    let mut impulse = Plane::new(size, size);
    impulse.set(50, 50, 1.0);
    for sigma in [2.0f32, 4.5, 9.0] {
        let out = gaussian_blur(&impulse, sigma);
        // Summing over rows leaves the one-dimensional kernel.
        let kernel = |x: usize| (0..size).map(|y| out.get(x, y)).sum::<f32>();
        let ideal_peak = 1.0 / (sigma * (2.0 * std::f32::consts::PI).sqrt());
        for d in 0..(3.0 * sigma) as usize {
            let ideal = ideal_peak * (-((d * d) as f32) / (2.0 * sigma * sigma)).exp();
            let error = (kernel(50 + d) - ideal).abs() / ideal_peak;
            assert!(error < 0.07, "sigma {sigma} at {d}: {error}");
        }
    }
}

#[test]
fn the_min_filter_equals_the_brute_force_reference() {
    let plane = noise_plane(37, 23, 8);
    for radius in [1usize, 2, 5, 11, 40] {
        let reference = Plane::from_fn(37, 23, |x, y| {
            let r = radius as i64;
            let mut lowest = f64::INFINITY;
            for dy in -r..=r {
                for dx in -r..=r {
                    lowest = lowest.min(clamped(&plane, x as i64 + dx, y as i64 + dy));
                }
            }
            lowest as f32
        });
        assert_eq!(min_filter(&plane, radius), reference, "radius {radius}");
    }
}

#[test]
fn wavelet_decomposition_reconstructs_the_input() {
    let plane = noise_plane(70, 50, 9);
    for levels in [0usize, 1, 3, 5] {
        let mut decomposition: WaveletDecomposition = wavelet_decompose(&plane, levels);
        assert_eq!(decomposition.details().len(), levels);
        let diff = max_abs_diff(decomposition.reconstruct().as_slice(), plane.as_slice());
        assert!(diff < 1e-6, "{levels} levels: {diff}");
        // Zero thresholds are a no-op, both in place and fused.
        decomposition.soft_threshold(&vec![0.0; levels]);
        let diff = max_abs_diff(decomposition.reconstruct().as_slice(), plane.as_slice());
        assert!(diff < 1e-6, "{levels} levels after zero thresholds: {diff}");
        let diff = max_abs_diff(
            wavelet_denoise(&plane, &vec![0.0; levels]).as_slice(),
            plane.as_slice(),
        );
        assert!(diff < 1e-6, "{levels} levels fused: {diff}");
    }
}

#[test]
fn wavelet_details_are_band_passes_with_zero_mean() {
    let plane = noise_plane(96, 96, 10);
    let decomposition = wavelet_decompose(&plane, 4);
    for (level, detail) in decomposition.details().iter().enumerate() {
        assert!(mean(detail.as_slice()).abs() < 0.01, "level {level}");
    }
    // White noise has most of its energy in the finest band.
    let energy: Vec<f32> = decomposition
        .details()
        .iter()
        .map(|d| std_dev(d.as_slice()))
        .collect();
    assert!(
        energy[0] > energy[1] && energy[1] > energy[2] && energy[2] > energy[3],
        "{energy:?}"
    );
    // The residual is the smooth remainder, close to the plane's mean.
    assert!(std_dev(decomposition.residual().as_slice()) < 0.03);
}

#[test]
fn soft_thresholding_removes_noise_and_keeps_a_strong_edge() {
    let input = noisy_step(128, 96, 0.02, 11);
    // About three standard deviations of what white noise of 0.02 leaves in
    // each band (0.889, 0.200 and 0.086 of it for this wavelet in 2D).
    let thresholds = [0.053, 0.012, 0.005];
    let fused = wavelet_denoise(&input, &thresholds);
    assert!(
        flat_noise(&fused, 20) < 0.02 / 4.0,
        "{}",
        flat_noise(&fused, 20)
    );
    // Shrinking costs a step at most twice the sum of the thresholds.
    assert!(edge_height(&fused) > 0.7 * 0.6, "{}", edge_height(&fused));
    // The fused form is the decomposition, thresholded and rebuilt.
    let mut decomposition = wavelet_decompose(&input, 3);
    decomposition.soft_threshold(&thresholds);
    let diff = max_abs_diff(decomposition.reconstruct().as_slice(), fused.as_slice());
    assert!(diff < 1e-6, "{diff}");
}

#[test]
fn soft_threshold_shrinks_toward_zero_without_crossing_it() {
    assert_eq!(soft_threshold(0.5, 0.2), 0.3);
    assert_eq!(soft_threshold(-0.5, 0.2), -0.3);
    assert_eq!(soft_threshold(0.1, 0.2), 0.0);
    assert_eq!(soft_threshold(-0.1, 0.2), 0.0);
    assert_eq!(soft_threshold(0.7, 0.0), 0.7);
    assert_eq!(
        soft_threshold(0.7, -1.0),
        0.7,
        "a negative threshold is no threshold"
    );
}

#[test]
fn the_high_pass_is_what_a_gaussian_removes() {
    let plane = noise_plane(60, 40, 12);
    let detail = high_pass(&plane, 2.5);
    let blurred = gaussian_blur(&plane, 2.5);
    let rebuilt = detail.zip_map(&blurred, |d, b| d + b);
    assert!(max_abs_diff(rebuilt.as_slice(), plane.as_slice()) < 1e-6);
    assert!(mean(detail.as_slice()).abs() < 0.01);
}

#[test]
fn gradient_magnitude_measures_slope_in_any_direction() {
    let slope = |dx: f32, dy: f32| {
        Plane::from_fn(30, 30, move |x, y| 0.01 * (dx * x as f32 + dy * y as f32))
    };
    for (dx, dy) in [(1.0, 0.0), (0.0, 1.0), (0.6, 0.8), (-0.8, 0.6)] {
        let magnitude = gradient_magnitude(&slope(dx, dy));
        // Away from the border, a ramp of slope 0.01 per pixel reads 0.01.
        assert!(
            (magnitude.get(15, 15) - 0.01).abs() < 1e-5,
            "({dx}, {dy}): {}",
            magnitude.get(15, 15)
        );
    }
    let step = Plane::from_fn(30, 30, |x, _| if x < 15 { 0.0 } else { 1.0 });
    let magnitude = gradient_magnitude(&step);
    assert!(magnitude.get(15, 10) > 0.4 && magnitude.get(5, 10) == 0.0);
}

/// R1 for a filter: filter the whole plane and crop, or crop a region padded
/// by the filter's support and filter that; the region must come out the
/// same. `filter` is given the plane and the position of its top-left sample
/// in the grid the whole plane belongs to.
///
/// The whole plane is placed in that grid twice: at the origin, and five
/// samples before it, as the padded buffer of a whole view would be. Tiles
/// start on cell boundaries and partway into cells alike.
fn assert_tile_invariant(name: &str, support: usize, filter: impl Fn(&Plane, (i64, i64)) -> Plane) {
    let (width, height) = (190usize, 150usize);
    let whole = noise_plane(width, height, 13);
    for shift in [0i64, -5] {
        let expected = filter(&whole, (shift, shift));
        for (x, y, w, h) in [
            (60usize, 50usize, 40usize, 30usize),
            (0, 0, 50, 40),
            (130, 100, 60, 50),
            (17, 90, 33, 21),
            (61, 41, 30, 30),
            (97, 97, 40, 20),
        ] {
            let x0 = x.saturating_sub(support);
            let y0 = y.saturating_sub(support);
            let x1 = (x + w + support).min(width);
            let y1 = (y + h + support).min(height);
            let origin = (x0 as i64 + shift, y0 as i64 + shift);
            let tile = filter(&whole.crop(x0, y0, x1 - x0, y1 - y0), origin);
            let got = tile.crop(x - x0, y - y0, w, h);
            let diff = max_abs_diff(got.as_slice(), expected.crop(x, y, w, h).as_slice());
            assert!(
                diff <= 1e-4,
                "{name}: tile at ({x}, {y}), grid shifted by {shift}, differs by {diff} with support {support}"
            );
        }
    }
}

#[test]
fn r1_every_filter_gives_the_same_pixels_on_a_padded_tile() {
    for radius in [0.7f32, 3.0, 9.5] {
        assert_tile_invariant("box", box_support(radius), |p, _| box_blur(p, radius));
        assert_tile_invariant("gaussian", gaussian_support(radius), |p, _| {
            gaussian_blur(p, radius)
        });
        assert_tile_invariant("guided", guided_support(radius), |p, _| {
            self_guided_filter(p, radius, 0.01)
        });
    }
    assert_tile_invariant("cross guided", guided_support(4.0), |p, _| {
        guided_filter(&p.map(|v| v * v), p, 4.0, 0.01)
    });
    for radius in [9.0f32, 17.0, 33.0] {
        assert_tile_invariant("fast guided", fast_guided_support(radius), |p, origin| {
            fast_self_guided_filter(p, radius, 0.01, origin)
        });
        assert_tile_invariant(
            "fast cross guided",
            fast_guided_support(radius),
            |p, origin| fast_guided_filter(&p.map(|v| v * v), p, radius, 0.01, origin),
        );
    }
    for radius in [1usize, 4, 12] {
        assert_tile_invariant("min", radius, |p, _| min_filter(p, radius));
    }
    for levels in [1usize, 2, 4] {
        assert_tile_invariant("wavelet", wavelet_support(levels), |p, _| {
            wavelet_denoise(p, &vec![0.05; levels])
        });
    }
    assert_tile_invariant("high pass", gaussian_support(2.0), |p, _| high_pass(p, 2.0));
    assert_tile_invariant("gradient", 1, |p, _| gradient_magnitude(p));
}

#[test]
fn the_documented_support_is_tight_enough_to_matter() {
    // One pixel less padding than documented must be able to change the
    // result; otherwise the support functions are over-promising and
    // callers pad more than they need to.
    let whole = noise_plane(120, 100, 14);
    let differs = |support: usize, filter: &dyn Fn(&Plane) -> Plane| {
        let expected = filter(&whole).crop(50, 40, 20, 20);
        let pad = support - 1;
        let tile = filter(&whole.crop(50 - pad, 40 - pad, 20 + 2 * pad, 20 + 2 * pad));
        max_abs_diff(tile.crop(pad, pad, 20, 20).as_slice(), expected.as_slice()) > 1e-7
    };
    assert!(differs(box_support(4.0), &|p| box_blur(p, 4.0)));
    assert!(differs(gaussian_support(2.0), &|p| gaussian_blur(p, 2.0)));
    assert!(differs(guided_support(3.0), &|p| self_guided_filter(
        p, 3.0, 0.01
    )));
    assert!(differs(5, &|p| min_filter(p, 5)));
    assert!(differs(wavelet_support(3), &|p| wavelet_denoise(
        p, &[0.05; 3]
    )));
}

#[test]
fn filters_accept_empty_and_tiny_planes() {
    for (width, height) in [(0usize, 0usize), (0, 4), (1, 1), (2, 1), (1, 7)] {
        let plane = Plane::filled(width, height, 0.5);
        let same_size = |out: Plane| (out.width(), out.height()) == (width, height);
        assert!(same_size(box_blur(&plane, 3.5)));
        assert!(same_size(gaussian_blur(&plane, 3.5)));
        assert!(same_size(self_guided_filter(&plane, 3.5, 0.01)));
        assert!(same_size(fast_self_guided_filter(
            &plane,
            40.0,
            0.01,
            (-3, 7)
        )));
        assert!(same_size(min_filter(&plane, 3)));
        assert!(same_size(wavelet_denoise(&plane, &[0.1, 0.1])));
        assert!(same_size(high_pass(&plane, 1.0)));
        assert!(same_size(gradient_magnitude(&plane)));
        let out = fast_self_guided_filter(&plane, 40.0, 0.01, (-3, 7));
        assert!(out.as_slice().iter().all(|v| (v - 0.5).abs() < 1e-6));
    }
}
