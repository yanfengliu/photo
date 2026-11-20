//! Mask rasterization: what each shape covers and how components combine.
//! How masks behave under geometry and views is pinned in `mask_views.rs`.

use photo_core::develop::{BrushStroke, Mask, MaskComponent, MaskShape};
use photo_core::geom::Point;
use photo_imaging::{Geometry, Plane, View, rasterize_mask};

pub const W: u32 = 200;
pub const H: u32 = 100;

pub fn add(shape: MaskShape) -> MaskComponent {
    MaskComponent {
        shape,
        subtract: false,
    }
}

pub fn subtract(shape: MaskShape) -> MaskComponent {
    MaskComponent {
        shape,
        subtract: true,
    }
}

pub fn mask(components: Vec<MaskComponent>) -> Mask {
    Mask {
        components,
        ..Default::default()
    }
}

pub fn linear(start: (f32, f32), end: (f32, f32)) -> MaskShape {
    MaskShape::Linear {
        start: Point::new(start.0, start.1),
        end: Point::new(end.0, end.1),
    }
}

/// An ellipse at the centre of the frame, 40 px across and 20 px down on the
/// 200 px long edge.
pub fn radial(feather: f32, invert: bool, angle: f32) -> MaskShape {
    MaskShape::Radial {
        center: Point::new(0.5, 0.5),
        radius_x: 0.2,
        radius_y: 0.1,
        angle,
        feather,
        invert,
    }
}

/// A stroke with a 10 px radius on the 200 px long edge.
pub fn stroke(points: &[(f32, f32)], feather: f32, flow: f32, erase: bool) -> BrushStroke {
    BrushStroke {
        points: points.iter().map(|&(x, y)| Point::new(x, y)).collect(),
        radius: 0.05,
        feather,
        flow,
        erase,
    }
}

pub fn brush(strokes: Vec<BrushStroke>) -> MaskShape {
    MaskShape::Brush { strokes }
}

/// The mask over the whole source at 1:1, with untouched geometry: view
/// pixel `(x, y)` is source pixel `(x, y)`.
fn render(mask: &Mask) -> Plane {
    let geometry = Geometry::identity(W, H);
    rasterize_mask(mask, &geometry, &View::full(&geometry), 0)
}

fn render_shape(shape: MaskShape) -> Plane {
    render(&mask(vec![add(shape)]))
}

#[test]
fn a_linear_gradient_is_full_before_its_start_and_empty_beyond_its_end() {
    let alpha = render_shape(linear((0.25, 0.5), (0.75, 0.5)));
    assert_eq!((alpha.width(), alpha.height()), (200, 100));
    for y in [0, 37, 99] {
        assert_eq!(alpha.get(10, y), 1.0);
        assert_eq!(alpha.get(49, y), 1.0);
        assert_eq!(alpha.get(150, y), 0.0);
        assert_eq!(alpha.get(199, y), 0.0);
        // Halfway along it is half, and it only ever falls.
        assert!((alpha.get(99, y) + alpha.get(100, y) - 1.0).abs() < 1e-5);
        for x in 1..200 {
            assert!(alpha.get(x, y) <= alpha.get(x - 1, y));
        }
        assert!(
            alpha.get(70, y) > 0.8 && alpha.get(130, y) < 0.2,
            "eased, not linear"
        );
    }
}

#[test]
fn a_linear_gradient_is_constant_across_its_direction_in_pixels() {
    // From (40, 20) to (120, 60) px: direction (2, 1), so (-1, 2) is across.
    let alpha = render_shape(linear((0.2, 0.2), (0.6, 0.6)));
    for (x, y) in [(80usize, 40usize), (90, 30), (70, 50)] {
        let moved = alpha.get(x - 10, y + 20);
        assert!((alpha.get(x, y) - moved).abs() < 1e-5, "({x}, {y})");
        assert!(alpha.get(x, y) > 0.01 && alpha.get(x, y) < 0.99);
    }
    // A gradient with no length has no direction and covers nothing.
    let nothing = render_shape(linear((0.5, 0.5), (0.5, 0.5)));
    assert!(nothing.as_slice().iter().all(|&v| v == 0.0));
}

#[test]
fn a_radial_gradient_is_full_at_its_centre_and_empty_far_outside() {
    // Radii are fractions of the long edge: 40 px across, 20 px down.
    let alpha = render_shape(radial(50.0, false, 0.0));
    assert_eq!(alpha.get(100, 50), 1.0);
    assert_eq!(alpha.get(115, 50), 1.0, "inside the unfeathered half");
    assert_eq!(alpha.get(100, 56), 1.0);
    assert_eq!(alpha.get(150, 50), 0.0);
    assert_eq!(alpha.get(100, 80), 0.0);
    assert_eq!(alpha.get(5, 5), 0.0);
    let across = alpha.get(130, 50);
    let down = alpha.get(100, 65);
    assert!(
        (across - 0.5).abs() < 0.05 && (down - 0.5).abs() < 0.1,
        "{across}, {down}"
    );
}

#[test]
fn radial_feather_runs_from_a_hard_edge_to_soft_from_the_centre() {
    let hard = render_shape(radial(0.0, false, 0.0));
    assert_eq!(hard.get(137, 50), 1.0);
    assert_eq!(hard.get(143, 50), 0.0);
    let soft = render_shape(radial(100.0, false, 0.0));
    assert!(soft.get(100, 50) > 0.99);
    assert!(
        (soft.get(120, 50) - 0.5).abs() < 0.05,
        "{}",
        soft.get(120, 50)
    );
    assert!(soft.get(137, 50) < 0.05 && soft.get(137, 50) > 0.0);
    assert_eq!(
        soft.get(143, 50),
        0.0,
        "the ellipse is the outer limit at any feather"
    );
}

#[test]
fn an_inverted_radial_gradient_is_the_complement() {
    let plain = render_shape(radial(50.0, false, 0.0));
    let inverted = render_shape(radial(50.0, true, 0.0));
    assert_eq!(inverted.get(100, 50), 0.0);
    assert_eq!(inverted.get(5, 5), 1.0);
    let sum = plain.zip_map(&inverted, |a, b| a + b);
    assert!(sum.as_slice().iter().all(|v| (v - 1.0).abs() < 1e-6));
}

#[test]
fn a_radial_gradient_turns_with_its_angle() {
    let upright = render_shape(radial(0.0, false, 0.0));
    assert_eq!((upright.get(130, 50), upright.get(100, 75)), (1.0, 0.0));
    let turned = render_shape(radial(0.0, false, 90.0));
    assert_eq!((turned.get(130, 50), turned.get(100, 75)), (0.0, 1.0));
    // 30 degrees clockwise on screen: the long axis now points down-right.
    let tilted = render_shape(radial(0.0, false, 30.0));
    assert_eq!((tilted.get(126, 65), tilted.get(126, 35)), (1.0, 0.0));
}

#[test]
fn a_brush_stroke_covers_its_path_with_round_ends() {
    // 10 px radius on a 200 px long edge, from (40, 50) to (160, 50).
    let alpha = render_shape(brush(vec![stroke(
        &[(0.2, 0.5), (0.8, 0.5)],
        0.0,
        100.0,
        false,
    )]));
    for x in (40..=160).step_by(10) {
        assert_eq!(alpha.get(x, 50), 1.0, "on the path at {x}");
        assert_eq!(alpha.get(x, 43), 1.0);
        assert_eq!(alpha.get(x, 57), 1.0);
        assert_eq!(alpha.get(x, 36), 0.0);
        assert_eq!(alpha.get(x, 64), 0.0);
    }
    assert_eq!(alpha.get(33, 50), 1.0, "inside the round cap");
    assert_eq!(alpha.get(26, 50), 0.0);
    assert_eq!(alpha.get(32, 42), 0.0, "the cap is round, not square");
    assert_eq!(alpha.get(167, 50), 1.0);
}

#[test]
fn a_brush_follows_every_segment_and_handles_dots_and_empty_strokes() {
    let bent = render_shape(brush(vec![stroke(
        &[(0.2, 0.2), (0.5, 0.2), (0.5, 0.8)],
        0.0,
        100.0,
        false,
    )]));
    assert_eq!(bent.get(70, 20), 1.0);
    assert_eq!(bent.get(100, 20), 1.0);
    assert_eq!(bent.get(100, 60), 1.0);
    assert_eq!(bent.get(70, 60), 0.0);

    let dot = render_shape(brush(vec![stroke(&[(0.5, 0.5)], 0.0, 100.0, false)]));
    assert_eq!(dot.get(100, 50), 1.0);
    assert_eq!(dot.get(106, 50), 1.0);
    assert_eq!(dot.get(113, 50), 0.0);

    let nothing = render_shape(brush(vec![stroke(&[], 0.0, 100.0, false)]));
    assert!(nothing.as_slice().iter().all(|&v| v == 0.0));
    assert!(
        render_shape(brush(Vec::new()))
            .as_slice()
            .iter()
            .all(|&v| v == 0.0)
    );
}

#[test]
fn a_stroke_that_doubles_back_covers_its_far_end() {
    // Out 2.4 px to the right and straight back, with a 10 px hard brush.
    // The turning point is the farthest the brush reached, and whatever the
    // stroke's path is reduced to for drawing must still reach it.
    let there_and_back = [(0.5, 0.5), (0.512, 0.5), (0.5, 0.5)];
    let alpha = render_shape(brush(vec![stroke(&there_and_back, 0.0, 100.0, false)]));
    // Pixel 111 is 9.1 px right of the turning point (102.4, 50) and 11.5 px
    // from the start: inside the stroke only because of the turning point.
    assert_eq!(alpha.get(111, 50), 1.0);
    assert_eq!(alpha.get(114, 50), 0.0);
    // While the stroke is being drawn, before it turns, it reaches as far.
    let outward = render_shape(brush(vec![stroke(&there_and_back[..2], 0.0, 100.0, false)]));
    assert_eq!(outward.get(111, 50), alpha.get(111, 50));
}

#[test]
fn brush_feather_softens_from_the_centre_line() {
    let soft = render_shape(brush(vec![stroke(
        &[(0.2, 0.5), (0.8, 0.5)],
        100.0,
        100.0,
        false,
    )]));
    // Pixel centres sit half a pixel off the path, so compare against the
    // falloff at their true distance: smoothstep from 0 to the 10 px radius.
    let expected = |distance: f32| {
        let t = (distance / 10.0).clamp(0.0, 1.0);
        1.0 - t * t * (3.0 - 2.0 * t)
    };
    for (y, distance) in [(50usize, 0.5f32), (52, 2.5), (55, 5.5), (58, 8.5)] {
        assert!(
            (soft.get(100, y) - expected(distance)).abs() < 0.01,
            "{y}: {}",
            soft.get(100, y)
        );
    }
    assert_eq!(soft.get(100, 61), 0.0);
    let half = render_shape(brush(vec![stroke(
        &[(0.2, 0.5), (0.8, 0.5)],
        50.0,
        100.0,
        false,
    )]));
    assert_eq!(half.get(100, 53), 1.0, "the inner half is solid");
    assert!(half.get(100, 57) < 0.6 && half.get(100, 57) > 0.2);
}

#[test]
fn flow_builds_up_across_strokes_and_erasing_removes() {
    let line = [(0.2, 0.5), (0.8, 0.5)];
    let once = render_shape(brush(vec![stroke(&line, 0.0, 50.0, false)]));
    assert!((once.get(100, 50) - 0.5).abs() < 1e-6);
    // One stroke never exceeds its flow, however it overlaps itself.
    let back_and_forth = [(0.2, 0.5), (0.8, 0.5), (0.2, 0.5), (0.8, 0.5)];
    let folded = render_shape(brush(vec![stroke(&back_and_forth, 0.0, 50.0, false)]));
    assert!((folded.get(100, 50) - 0.5).abs() < 1e-6);
    // A second stroke builds on the first.
    let twice = render_shape(brush(vec![
        stroke(&line, 0.0, 50.0, false),
        stroke(&line, 0.0, 50.0, false),
    ]));
    assert!((twice.get(100, 50) - 0.75).abs() < 1e-6);

    let crossing = [(0.5, 0.2), (0.5, 0.8)];
    let erased = render_shape(brush(vec![
        stroke(&line, 0.0, 100.0, false),
        stroke(&crossing, 0.0, 100.0, true),
    ]));
    assert_eq!(erased.get(100, 50), 0.0, "erased where the strokes cross");
    assert_eq!(erased.get(60, 50), 1.0, "untouched elsewhere");
    assert_eq!(erased.get(100, 25), 0.0, "erasing nothing leaves nothing");
    let thinned = render_shape(brush(vec![
        stroke(&line, 0.0, 100.0, false),
        stroke(&crossing, 0.0, 40.0, true),
    ]));
    assert!((thinned.get(100, 50) - 0.6).abs() < 1e-6);
    // Order matters: painting after erasing puts the paint back.
    let repainted = render_shape(brush(vec![
        stroke(&crossing, 0.0, 100.0, true),
        stroke(&line, 0.0, 100.0, false),
    ]));
    assert_eq!(repainted.get(100, 50), 1.0);
}

#[test]
fn components_add_and_subtract_in_order() {
    let gradient = linear((0.25, 0.5), (0.75, 0.5));
    let alone = render_shape(gradient.clone());
    let hole = radial(0.0, false, 0.0);

    let cut = render(&mask(vec![add(gradient.clone()), subtract(hole.clone())]));
    assert_eq!(cut.get(100, 50), 0.0, "subtract removes");
    assert_eq!(cut.get(60, 10), alone.get(60, 10), "and leaves the rest");

    // A subtraction that comes first has nothing to remove.
    let reordered = render(&mask(vec![subtract(hole.clone()), add(gradient.clone())]));
    assert_eq!(reordered, alone);

    // Adding is a union: a + b - a * b, never more than 1.
    let both = render(&mask(vec![add(gradient.clone()), add(hole)]));
    assert_eq!(both.get(100, 50), 1.0);
    assert_eq!(both.get(60, 10), alone.get(60, 10));
    let doubled = render(&mask(vec![add(gradient.clone()), add(gradient)]));
    let a = alone.get(100, 50);
    assert!((doubled.get(100, 50) - (2.0 * a - a * a)).abs() < 1e-6);
}

#[test]
fn inverting_a_mask_flips_it_and_an_empty_mask_covers_nothing() {
    let plain = mask(vec![add(radial(50.0, false, 0.0))]);
    let flipped = Mask {
        invert: true,
        ..plain.clone()
    };
    let sum = render(&plain).zip_map(&render(&flipped), |a, b| a + b);
    assert!(sum.as_slice().iter().all(|v| (v - 1.0).abs() < 1e-6));

    assert!(
        render(&mask(Vec::new()))
            .as_slice()
            .iter()
            .all(|&v| v == 0.0)
    );
    let everything = Mask {
        invert: true,
        ..mask(Vec::new())
    };
    assert!(render(&everything).as_slice().iter().all(|&v| v == 1.0));
}

#[test]
fn only_the_shape_is_rasterized_not_the_strength_or_the_switch() {
    let shape = mask(vec![add(radial(50.0, false, 0.0))]);
    let muted = Mask {
        enabled: false,
        amount: 12.0,
        ..shape.clone()
    };
    assert_eq!(render(&muted), render(&shape));
}

#[test]
fn non_finite_shape_parameters_never_reach_the_alpha_plane() {
    // A NaN that got into a mask must not become a NaN in the pipeline: one
    // bad pixel would spread through every spatial filter after it.
    let line = [(0.2, 0.5), (0.8, 0.5)];
    let nan = f32::NAN;
    let radial_with = |radius: f32, angle: f32, feather: f32, x: f32| MaskShape::Radial {
        center: Point::new(x, 0.5),
        radius_x: radius,
        radius_y: 0.1,
        angle,
        feather,
        invert: false,
    };
    let stroke_with = |radius: f32, feather: f32, flow: f32| BrushStroke {
        radius,
        ..stroke(&line, feather, flow, false)
    };
    let shapes = vec![
        radial_with(0.2, 0.0, nan, 0.5),
        radial_with(0.2, 0.0, f32::INFINITY, 0.5),
        radial_with(0.2, nan, 50.0, 0.5),
        radial_with(nan, 0.0, 50.0, 0.5),
        radial_with(0.2, 0.0, 50.0, nan),
        radial_with(-0.2, 0.0, 50.0, 0.5),
        brush(vec![stroke_with(0.05, nan, 100.0)]),
        brush(vec![stroke_with(nan, 50.0, 100.0)]),
        brush(vec![stroke_with(0.05, 50.0, nan)]),
        brush(vec![stroke(
            &[(nan, 0.5), (0.5, nan), (0.5, 0.5)],
            0.0,
            100.0,
            false,
        )]),
        linear((nan, 0.5), (0.75, 0.5)),
        linear((0.25, 0.5), (f32::INFINITY, 0.5)),
    ];
    for shape in shapes {
        let alpha = render_shape(shape.clone());
        assert!(
            alpha.as_slice().iter().all(|v| (0.0..=1.0).contains(v)),
            "{shape:?}"
        );
    }
    // An unusable feather is no feather.
    assert_eq!(
        render_shape(radial_with(0.2, 0.0, nan, 0.5)),
        render_shape(radial(0.0, false, 0.0))
    );
    assert_eq!(
        render_shape(brush(vec![stroke_with(0.05, nan, 100.0)])),
        render_shape(brush(vec![stroke(&line, 0.0, 100.0, false)]))
    );
}
