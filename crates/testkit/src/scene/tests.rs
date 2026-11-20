use super::*;

const W: u32 = 320;
const H: u32 = 240;

#[test]
fn the_scene_is_deterministic() {
    assert_eq!(scene(96, 64), scene(96, 64));
    assert_eq!(scene_linear(96, 64), scene_linear(96, 64));
}

#[test]
fn the_sun_is_far_above_white_and_clips_when_rendered() {
    let linear = scene_linear(W, H);
    let (x, y) = (
        (SUN_CENTRE.0 * W as f32) as u32,
        (SUN_CENTRE.1 * H as f32) as u32,
    );
    assert_eq!(linear.pixel(x, y), SUN_RADIANCE);
    assert_eq!(scene(W, H).get_pixel(x, y).0, [255, 255, 255]);
    assert_eq!(scene16(W, H).get_pixel(x, y).0, [65535, 65535, 65535]);
}

#[test]
fn the_glow_clips_one_channel_at_a_time() {
    // Walking away from the disk there is a band where some channels are
    // still past white and others have already dropped below it.
    let linear = scene_linear(W, H);
    let centre_x = (SUN_CENTRE.0 * W as f32) as u32;
    let centre_y = (SUN_CENTRE.1 * H as f32) as u32;
    let partly_clipped = (0..centre_x)
        .map(|x| linear.pixel(x, centre_y))
        .filter(|p| p.iter().any(|&c| c > 1.0) && p.iter().any(|&c| c < 1.0))
        .count();
    assert!(
        partly_clipped >= 3,
        "{partly_clipped} partly clipped pixels"
    );
}

#[test]
fn the_sky_brightens_toward_the_horizon_and_the_ground_is_dark() {
    let linear = scene_linear(W, H);
    let high = linear.mean(8, 4, 40, 8);
    let low = linear.mean(8, (HORIZON * H as f32) as u32 - 12, 40, 8);
    assert!(
        low[0] > high[0] && low[1] > high[1],
        "{high:?} then {low:?}"
    );
    assert!(high[2] > high[0], "the sky is blue");

    let ground = linear.mean(0, H - 12, W, 12);
    assert!(ground.iter().all(|&c| c < 0.06), "{ground:?}");
}

#[test]
fn the_ground_has_fine_texture_and_the_sky_only_mild_noise() {
    let linear = scene_linear(W, H);
    let contrast = |y: u32| {
        let row: Vec<f32> = (0..W).map(|x| linear.pixel(x, y)[1]).collect();
        let mean = row.iter().sum::<f32>() / row.len() as f32;
        let adjacent =
            row.windows(2).map(|p| (p[1] - p[0]).abs()).sum::<f32>() / (row.len() - 1) as f32;
        adjacent / mean
    };
    assert!(contrast(H - 4) > 0.2, "ground {}", contrast(H - 4));
    assert!(contrast(4) < 0.05, "sky {}", contrast(4));
    assert!(contrast(4) > 0.005, "sky {}", contrast(4));
}

#[test]
fn patches_are_flat_and_report_their_own_colour() {
    let linear = scene_linear(W, H);
    let found = patches(W, H);
    assert_eq!(found.len(), 8);
    for patch in &found {
        assert!(patch.width >= 8 && patch.height >= 8, "{patch:?}");
        let (right, bottom) = (patch.width - 1, patch.height - 1);
        for (dx, dy) in [(0, 0), (right, 0), (0, bottom), (right, bottom)] {
            let colour = linear.pixel(patch.x + dx, patch.y + dy);
            assert_eq!(colour, patch.linear_srgb, "{}", patch.name);
        }
        // The inset really is inside: a little further out is still the patch.
        let outside_inset = linear.pixel(patch.x - 2, patch.y - 2);
        assert_eq!(outside_inset, patch.linear_srgb, "{}", patch.name);
    }
    let names: Vec<_> = found.iter().map(|p| p.name).collect();
    let expected = [
        "red",
        "green",
        "blue",
        "yellow",
        "cyan",
        "magenta",
        "light skin",
        "dark skin",
    ];
    assert_eq!(names, expected);
}

#[test]
fn saturated_patches_are_saturated_and_skin_is_warm() {
    for patch in patches(W, H) {
        let [r, g, b] = patch.linear_srgb;
        let (max, min) = (r.max(g).max(b), r.min(g).min(b));
        if patch.name.ends_with("skin") {
            assert!(r > g && g > b, "{patch:?}");
        } else {
            assert!(max / min > 8.0, "{patch:?}");
        }
    }
}

#[test]
fn the_grey_ramp_is_neutral_and_climbs_from_black_to_white() {
    let (x, y, width, height) = grey_ramp(W, H);
    assert!(width > 200 && height >= 8);
    let rendered = scene16(W, H);
    let mut previous = 0u16;
    for column in x..x + width {
        let [r, g, b] = rendered.get_pixel(column, y).0;
        assert_eq!((r, g), (g, b), "neutral at {column}");
        assert!(r > previous || column == x, "rising at {column}");
        let bottom = rendered.get_pixel(column, y + height - 1).0;
        assert_eq!(bottom, [r, g, b], "flat down the column");
        previous = r;
    }
    assert!(rendered.get_pixel(x, y).0[0] < 400);
    assert!(rendered.get_pixel(x + width - 1, y).0[0] > 65000);
}

#[test]
fn sixteen_bit_rendering_has_more_levels_than_eight_bits_can_hold() {
    let rendered = scene16(W, H);
    let mut levels: Vec<u16> = rendered.pixels().map(|p| p.0[1]).collect();
    levels.sort_unstable();
    levels.dedup();
    assert!(levels.len() > 2000, "{} distinct levels", levels.len());
}

#[test]
fn eight_and_sixteen_bit_renderings_agree() {
    let (low, high) = (scene(96, 64), scene16(96, 64));
    for (a, b) in low.pixels().zip(high.pixels()) {
        for c in 0..3 {
            let widened = f32::from(a.0[c]) * 257.0;
            assert!((widened - f32::from(b.0[c])).abs() <= 129.0);
        }
    }
}

#[test]
fn radiance_continues_past_the_frame_edges() {
    let outside = radiance(-3, -2, W, H);
    let inside = radiance(0, 0, W, H);
    for c in 0..3 {
        assert!((outside[c] - inside[c]).abs() < 0.05 * inside[c].max(0.01));
    }
    let beyond = radiance(i64::from(W) + 5, i64::from(H) + 5, W, H);
    assert!(beyond.iter().all(|c| c.is_finite()));
}

#[test]
fn the_marker_sits_in_the_top_left_corner_only() {
    let marked = scene_with_marker(W, H);
    let plain = scene(W, H);
    let side = marker_size(W, H);
    assert_eq!(side, 40);
    assert_eq!(marked.get_pixel(0, 0).0, MARKER_RGB);
    assert_eq!(marked.get_pixel(side - 1, side - 1).0, MARKER_RGB);
    assert_eq!(marked.get_pixel(side, side), plain.get_pixel(side, side));
    assert_ne!(marked.get_pixel(W - 1, 0).0, MARKER_RGB);
    assert_eq!(marker_size(4, 3), 3, "never larger than the frame");
}

#[test]
fn tiny_frames_report_empty_interiors_instead_of_panicking() {
    for patch in patches(16, 12) {
        assert!(patch.width == 0 || patch.height == 0, "{patch:?}");
    }
    let (_, _, width, height) = grey_ramp(2, 2);
    assert!(width <= 2 && height <= 2);
}
