use photo_core::geom::Point;

use super::*;

fn line(radius: f32, feather: f32) -> BrushStroke {
    BrushStroke {
        points: vec![Point::new(0.25, 0.5), Point::new(0.75, 0.5)],
        radius,
        feather,
        flow: 100.0,
        erase: false,
    }
}

/// Stamp for a view that reads the whole source.
fn whole(strokes: &[BrushStroke], size: (u32, u32), footprint: f64) -> Option<BrushBitmap> {
    let view = [0.0, 0.0, f64::from(size.0), f64::from(size.1)];
    BrushBitmap::stamp(strokes, size, footprint, view)
}

#[test]
fn the_cell_size_follows_the_view_and_is_a_power_of_two() {
    let strokes = [line(0.05, 0.0)];
    let cell = |footprint: f64| whole(&strokes, (1000, 1000), footprint).unwrap().cell;
    assert_eq!(cell(0.25), 1.0, "never finer than a source pixel");
    assert_eq!(cell(1.0), 1.0);
    assert_eq!(cell(1.99), 1.0);
    assert_eq!(cell(2.0), 2.0);
    assert_eq!(cell(7.3), 4.0);
    assert_eq!(cell(f64::NAN), 1.0);
}

#[test]
fn the_bitmap_holds_only_what_its_view_reads() {
    // A big stroke on a big source, read through a small window at 1:1:
    // full resolution, and only the window's cells (plus margins).
    let strokes = [line(0.4, 0.0)];
    let view = [9_900.0, 9_950.0, 10_100.0, 10_050.0];
    let bitmap = BrushBitmap::stamp(&strokes, (20_000, 20_000), 1.0, view).unwrap();
    assert_eq!(bitmap.cell, 1.0);
    assert_eq!(
        (bitmap.coverage.width(), bitmap.coverage.height()),
        (204, 104)
    );
    assert_eq!(bitmap.sample(10_000.0, 10_000.0), 1.0);
    // A window the strokes do not reach needs no bitmap at all.
    assert!(
        BrushBitmap::stamp(&strokes, (20_000, 20_000), 1.0, [10.0, 10.0, 50.0, 50.0]).is_none()
    );
}

#[test]
fn a_window_holds_the_same_cells_as_the_whole() {
    let mut stroke = line(0.03, 40.0);
    stroke.points = (0..40)
        .map(|i| {
            let t = i as f32 / 39.0;
            Point::new(0.1 + 0.8 * t, 0.5 + 0.3 * (t * 9.0).sin())
        })
        .collect();
    let strokes = [stroke];
    for footprint in [1.0, 2.5, 6.0] {
        let all = whole(&strokes, (1000, 800), footprint).unwrap();
        let part = BrushBitmap::stamp(
            &strokes,
            (1000, 800),
            footprint,
            [310.0, 205.0, 520.0, 460.0],
        )
        .unwrap();
        for step in 0..500 {
            let (x, y) = (310.0 + step as f64 * 0.42, 205.0 + step as f64 * 0.51);
            assert_eq!(
                all.sample(x, y).to_bits(),
                part.sample(x, y).to_bits(),
                "{footprint} at ({x}, {y})"
            );
        }
    }
}

#[test]
fn strokes_outside_the_source_paint_nothing() {
    let mut outside = line(0.01, 0.0);
    outside.points = vec![Point::new(2.0, 2.0), Point::new(3.0, 2.5)];
    assert!(whole(&[outside], (500, 400), 1.0).is_none());
    let mut flowless = line(0.05, 0.0);
    flowless.flow = 0.0;
    assert!(whole(&[flowless], (500, 400), 1.0).is_none());
    let mut broken = line(0.05, 0.0);
    broken.points = vec![Point::new(f32::NAN, 0.5)];
    assert!(whole(&[broken], (500, 400), 1.0).is_none());
    let mut shapeless = line(f32::NAN, 0.0);
    assert!(whole(&[shapeless.clone()], (500, 400), 1.0).is_none());
    shapeless.radius = f32::INFINITY;
    let everywhere = whole(&[shapeless], (500, 400), 1.0).unwrap();
    assert_eq!(everywhere.sample(1.0, 1.0), 1.0);
}

#[test]
fn a_brush_thinner_than_a_cell_is_faint_rather_than_missing() {
    // 2 px radius seen through 16 px cells.
    let bitmap = whole(&[line(0.002, 0.0)], (1000, 1000), 16.0).unwrap();
    let on_path = bitmap.sample(500.0, 500.0);
    assert!(on_path > 0.05 && on_path < 0.9, "{on_path}");
}

#[test]
fn densely_sampled_straight_paths_are_simplified_without_moving_their_ends() {
    let mut dense = line(0.05, 0.0);
    dense.points = (0..=2000)
        .map(|i| Point::new(0.25 + 0.5 * i as f32 / 2000.0, 0.5))
        .collect();
    let stamp = Outline::new(&dense, 1000.0, 1000.0).unwrap().stamp(1.0);
    // 2000 collinear segments, at most 64 per drawn segment.
    assert!(stamp.segments.len() <= 32, "{}", stamp.segments.len());
    assert_eq!(stamp.segments[0].start, (250.0, 500.0));
    assert_eq!(stamp.segments.last().unwrap().end(), (750.0, 500.0));
}

#[test]
fn a_soft_brush_draws_a_jittery_path_with_few_segments() {
    // A pointer's path wobbles by its quantization: here 0.75 px either
    // side of a line, a point every 3 px. Under a soft edge 120 px wide
    // the wobble is invisible and need not be drawn segment by segment;
    // under a hard edge every wobble is drawn.
    let wobbly = |feather: f32| BrushStroke {
        points: (0..600)
            .map(|i| {
                let off = if i % 2 == 0 { 0.75 } else { -0.75 };
                Point::new((1000.0 + 3.0 * i as f32) / 6000.0, (2000.0 + off) / 4000.0)
            })
            .collect(),
        radius: 0.04,
        feather,
        flow: 100.0,
        erase: false,
    };
    let soft = Outline::new(&wobbly(50.0), 6000.0, 4000.0)
        .unwrap()
        .stamp(1.0);
    assert!(soft.segments.len() < 60, "{}", soft.segments.len());
    let hard = Outline::new(&wobbly(0.0), 6000.0, 4000.0)
        .unwrap()
        .stamp(1.0);
    assert!(hard.segments.len() > 500, "{}", hard.segments.len());
}

#[test]
fn a_dot_is_one_segment_of_no_length() {
    let mut dot = line(0.05, 0.0);
    dot.points.truncate(1);
    let stamp = Outline::new(&dot, 1000.0, 1000.0).unwrap().stamp(1.0);
    assert_eq!(stamp.segments.len(), 1);
    assert_eq!(stamp.segments[0].inverse_length_squared, 0.0);
    let mut row = [0.0f32; 1000];
    // Centred at x = 250 with a 50-cell radius: the rim is the cell at 300.
    assert_eq!(stamp.cover_row(500, 0, &mut row), Some((199, 300)));
    assert_eq!((row[250], row[298], row[300]), (1.0, 1.0, 0.0));
    // The same cells through a window that starts elsewhere.
    let mut window = [0.0f32; 200];
    assert_eq!(stamp.cover_row(500, 150, &mut window), Some((49, 150)));
    assert_eq!(window[100].to_bits(), row[250].to_bits());
    assert_eq!(window[149].to_bits(), row[299].to_bits());
}
