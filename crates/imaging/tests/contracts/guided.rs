//! The guided filters: a brute-force reference, what they are for, and how
//! the fast variant relates to the exact one.

use photo_imaging::Plane;
use photo_imaging::filter::{
    box_blur, fast_guided_filter, fast_guided_subsample, fast_guided_support,
    fast_self_guided_filter, gaussian_blur, guided_filter, guided_support, self_guided_filter,
};

use crate::util::{edge_height, flat_noise, max_abs_diff, mean_abs_diff, noise_plane, noisy_step};

fn clamped(plane: &Plane, x: i64, y: i64) -> f64 {
    let cx = x.clamp(0, plane.width() as i64 - 1) as usize;
    let cy = y.clamp(0, plane.height() as i64 - 1) as usize;
    f64::from(plane.get(cx, cy))
}

/// The guided filter by its definition, in double precision: fit
/// `a * guide + b` to the input in every window, then give each pixel the
/// mean of the fits of the windows covering it.
fn guided_reference(guide: &Plane, input: &Plane, radius: i64, eps: f64) -> Plane {
    let (width, height) = (guide.width(), guide.height());
    let window = |f: &dyn Fn(i64, i64) -> f64, x: usize, y: usize| {
        let mut sum = 0.0;
        for dy in -radius..=radius {
            for dx in -radius..=radius {
                sum += f(x as i64 + dx, y as i64 + dy);
            }
        }
        sum / ((2 * radius + 1) * (2 * radius + 1)) as f64
    };
    let mut fits = vec![(0.0f64, 0.0f64); width * height];
    for y in 0..height {
        for x in 0..width {
            let mean_g = window(&|x, y| clamped(guide, x, y), x, y);
            let mean_p = window(&|x, y| clamped(input, x, y), x, y);
            let mean_gg = window(&|x, y| clamped(guide, x, y).powi(2), x, y);
            let mean_gp = window(&|x, y| clamped(guide, x, y) * clamped(input, x, y), x, y);
            let a = (mean_gp - mean_g * mean_p) / (mean_gg - mean_g * mean_g + eps);
            fits[y * width + x] = (a, mean_p - a * mean_g);
        }
    }
    let fit = |x: i64, y: i64| {
        let cx = x.clamp(0, width as i64 - 1) as usize;
        let cy = y.clamp(0, height as i64 - 1) as usize;
        fits[cy * width + cx]
    };
    Plane::from_fn(width, height, |x, y| {
        let a = window(&|x, y| fit(x, y).0, x, y);
        let b = window(&|x, y| fit(x, y).1, x, y);
        (a * f64::from(guide.get(x, y)) + b) as f32
    })
}

#[test]
fn the_guided_filters_equal_the_brute_force_reference() {
    let guide = noise_plane(31, 23, 15);
    let input = noise_plane(31, 23, 16);
    for radius in [1i64, 3, 6] {
        for eps in [1e-3f32, 0.02, 0.5] {
            let own = self_guided_filter(&guide, radius as f32, eps);
            let wanted = guided_reference(&guide, &guide, radius, f64::from(eps));
            let diff = max_abs_diff(own.as_slice(), wanted.as_slice());
            assert!(diff < 1e-4, "self, radius {radius}, eps {eps}: {diff}");
            let cross = guided_filter(&guide, &input, radius as f32, eps);
            let wanted = guided_reference(&guide, &input, radius, f64::from(eps));
            let diff = max_abs_diff(cross.as_slice(), wanted.as_slice());
            assert!(diff < 1e-4, "cross, radius {radius}, eps {eps}: {diff}");
        }
    }
}

#[test]
fn the_guided_filter_smooths_the_flats_and_keeps_the_step() {
    let input = noisy_step(128, 96, 0.02, 3);
    let noise_in = flat_noise(&input, 20);
    assert!((noise_in - 0.02).abs() < 0.003);

    let out = self_guided_filter(&input, 6.0, 0.004);
    let noise_out = flat_noise(&out, 20);
    assert!(noise_out < noise_in / 4.0, "noise {noise_in} -> {noise_out}");
    // Adjacent columns either side of the 0.6 step still differ by most of it.
    let edge = edge_height(&out);
    assert!(edge > 0.85 * 0.6, "the 0.6 step is now {edge}");

    // A Gaussian with the same noise reduction cannot do that.
    let blurred = gaussian_blur(&input, 3.0);
    assert!(flat_noise(&blurred, 20) < noise_in / 4.0);
    assert!(edge_height(&blurred) < 0.2);
}

#[test]
fn a_separate_guide_lends_its_edges_to_the_input() {
    let guide = noisy_step(128, 96, 0.0, 4);
    let input = noisy_step(128, 96, 0.05, 5);
    let out = guided_filter(&guide, &input, 8.0, 1e-4);
    assert!(flat_noise(&out, 20) < 0.05 / 5.0);
    assert!(edge_height(&out) > 0.95 * 0.6);
    // Guiding a plane by itself is the self-guided filter.
    let twice = guided_filter(&input, &input, 5.0, 0.01);
    let once = self_guided_filter(&input, 5.0, 0.01);
    assert!(max_abs_diff(twice.as_slice(), once.as_slice()) < 1e-5);
}

#[test]
fn eps_below_the_resolution_of_a_variance_is_not_trusted() {
    // A constant guide has no variance; with eps = 0 only rounding noise
    // would be left to divide by. The result must still be the input's
    // local mean (two box blurs deep), not that noise amplified into gains
    // of thousands that then fail to cancel.
    let guide = Plane::filled(48, 32, 0.3);
    let input = noise_plane(48, 32, 17);
    let wanted = box_blur(&box_blur(&input, 4.0), 4.0);
    for eps in [0.0f32, -1.0, 1e-20, f32::NAN] {
        let out = guided_filter(&guide, &input, 4.0, eps);
        let diff = max_abs_diff(out.as_slice(), wanted.as_slice());
        assert!(diff < 1e-4, "eps {eps}: {diff}");
    }
}

#[test]
fn the_fast_guided_filter_does_the_same_job_at_large_radii() {
    let input = noisy_step(256, 160, 0.02, 6);
    let radius = 24.0;
    assert!(
        fast_guided_subsample(radius) >= 4,
        "this radius must actually subsample"
    );
    let noise_in = flat_noise(&input, 60);
    let fast = fast_self_guided_filter(&input, radius, 0.004, (0, 0));
    assert!(flat_noise(&fast, 60) < noise_in / 4.0);
    assert!(edge_height(&fast) > 0.8 * 0.6, "{}", edge_height(&fast));
    // And it stays close to the exact filter it approximates: the same
    // amount of smoothing on the flats, the same step.
    let exact = self_guided_filter(&input, radius, 0.004);
    let close = mean_abs_diff(fast.as_slice(), exact.as_slice());
    assert!(close < 0.004, "mean difference {close}");
    let (fast_noise, exact_noise) = (flat_noise(&fast, 60), flat_noise(&exact, 60));
    assert!(
        (fast_noise / exact_noise - 1.0).abs() < 0.25,
        "{fast_noise} vs {exact_noise}"
    );
    // Small radii are not subsampled at all: the result is the exact filter.
    assert_eq!(fast_guided_subsample(3.0), 1);
    assert_eq!(
        fast_self_guided_filter(&input, 3.0, 0.004, (5, 9)),
        self_guided_filter(&input, 3.0, 0.004)
    );
    let guide = noisy_step(256, 160, 0.0, 7);
    let cross = fast_guided_filter(&guide, &input, radius, 1e-4, (0, 0));
    assert!(edge_height(&cross) > 0.8 * 0.6, "{}", edge_height(&cross));
    assert!(flat_noise(&cross, 60) < noise_in / 10.0);
}

#[test]
fn the_fast_guided_filter_sees_detail_finer_than_its_cells() {
    // A one-pixel checker of +-0.1 has a variance of 0.01. With eps equal to
    // that, half of it must survive, whether or not the radius is large
    // enough for the filter to work on reduced planes: the variance it
    // compares with eps is that of the pixels, not of cell averages (in
    // which a checker would vanish altogether).
    //
    // Only the interior is compared. At the border the two filters replicate
    // the edge at different grains (the exact one pixels, which turns a
    // checker into solid blocks; the fast one whole cells, which keep its
    // statistics), and both are right by their own definition.
    let checker = Plane::from_fn(320, 256, |x, y| if (x + y) % 2 == 0 { 0.6 } else { 0.4 });
    for radius in [7.0f32, 8.0, 12.5, 33.0] {
        let fast = fast_self_guided_filter(&checker, radius, 0.01, (0, 0));
        let exact = self_guided_filter(&checker, radius, 0.01);
        assert!((exact.get(160, 128) - 0.55).abs() < 2e-3, "{}", exact.get(160, 128));
        let diff = interior_diff(&fast, &exact, radius);
        assert!(diff < 1e-3, "radius {radius}: fast and exact differ by {diff}");
    }
    // The same with a separate guide: the input here is the guide's negative.
    let negative = checker.map(|v| 1.0 - v);
    let fast = fast_guided_filter(&checker, &negative, 12.5, 0.01, (0, 0));
    let exact = guided_filter(&checker, &negative, 12.5, 0.01);
    assert!((exact.get(160, 128) - 0.45).abs() < 2e-3);
    let diff = interior_diff(&fast, &exact, 12.5);
    assert!(diff < 1e-3, "cross: {diff}");
}

/// The largest difference between two filtered planes beyond the reach of
/// either filter's border handling.
fn interior_diff(a: &Plane, b: &Plane, radius: f32) -> f32 {
    let margin = fast_guided_support(radius).max(guided_support(radius));
    let (width, height) = (a.width() - 2 * margin, a.height() - 2 * margin);
    let (a, b) = (
        a.crop(margin, margin, width, height),
        b.crop(margin, margin, width, height),
    );
    max_abs_diff(a.as_slice(), b.as_slice())
}

#[test]
fn the_result_changes_gently_where_the_filter_starts_to_subsample() {
    // Radius 7.99 runs the exact filter, 8.0 the reduced one. On an
    // ordinary noisy edge the two must be the same picture, give or take.
    let input = noisy_step(128, 96, 0.02, 18);
    let below = fast_self_guided_filter(&input, 7.99, 0.004, (0, 0));
    let above = fast_self_guided_filter(&input, 8.0, 0.004, (0, 0));
    assert_eq!(fast_guided_subsample(7.99), 1);
    assert_eq!(fast_guided_subsample(8.0), 2);
    let diff = mean_abs_diff(below.as_slice(), above.as_slice());
    assert!(diff < 1.5e-3, "{diff}");
}
