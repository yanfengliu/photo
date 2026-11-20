//! The histogram, the tone-curve editor and the colour wheel.

use egui::{Key, Modifiers, Role, vec2};
use photo_core::develop::{Adj, CurvePoint, GradeWheel, PointCurve};
use photo_ui::harness::{Harness, HarnessBuilder, Host};
use photo_ui::widgets::color_wheel::handle_pos;
use photo_ui::widgets::curve_editor::point_pos;
use photo_ui::widgets::{
    AdjSlider, ClippingToggles, ColorWheel, CurveEditor, HistogramData, histogram,
};

use crate::TestResult;

// ---- Histogram ----

#[test]
fn the_clipping_indicators_toggle() -> TestResult {
    let data = HistogramData {
        luma: [100; 256],
        ..Default::default()
    };
    let mut harness = HarnessBuilder::new()
        .size([300.0, 160.0])
        .strict_glyphs(true)
        .build_ui(ClippingToggles::default(), move |ui, toggles| {
            histogram(ui, &data, toggles, 96.0);
        });
    harness.run()?;
    assert_eq!(harness.count(Role::Image, "Histogram"), 1);
    assert_eq!(
        harness
            .control(Role::Button, "Show shadow clipping")?
            .toggled,
        Some(false)
    );

    harness.click(Role::Button, "Show shadow clipping")?;
    assert_eq!(
        harness.app().state,
        ClippingToggles {
            shadows: true,
            highlights: false
        }
    );
    assert_eq!(
        harness
            .control(Role::Button, "Show shadow clipping")?
            .toggled,
        Some(true)
    );

    harness.click(Role::Button, "Show highlight clipping")?;
    harness.click(Role::Button, "Show shadow clipping")?;
    assert_eq!(
        harness.app().state,
        ClippingToggles {
            shadows: false,
            highlights: true
        }
    );
    Ok(())
}

// ---- Tone-curve editor ----

struct CurveProbe {
    curve: PointCurve,
    changes: u32,
    finishes: u32,
}

/// The curve drawn straight from point to point: enough to stand in for the
/// pipeline's interpolation in these tests.
fn straight(points: &[CurvePoint], x: f32) -> f32 {
    let after = points
        .partition_point(|point| point.x < x)
        .clamp(1, points.len() - 1);
    let (a, b) = (points[after - 1], points[after]);
    a.y + (b.y - a.y) * ((x - a.x) / (b.x - a.x)).clamp(0.0, 1.0)
}

fn curve_editor(points: &[(f32, f32)]) -> Harness<Host<CurveProbe>> {
    let probe = CurveProbe {
        curve: PointCurve::new(points.iter().map(|(x, y)| CurvePoint::new(*x, *y))),
        changes: 0,
        finishes: 0,
    };
    HarnessBuilder::new()
        .size([320.0, 320.0])
        .strict_glyphs(true)
        .build_ui(probe, |ui, probe| {
            let drawn = probe.curve.clone();
            let sample = |x: f32| straight(drawn.points(), x);
            let response = CurveEditor::new("Tone curve", &mut probe.curve, &sample)
                .size([200.0, 200.0])
                .show(ui);
            probe.changes += u32::from(response.changed);
            probe.finishes += u32::from(response.finished);
        })
}

fn points(harness: &Harness<Host<CurveProbe>>) -> Vec<(f32, f32)> {
    let round = |value: f32| (value * 100.0).round() / 100.0;
    harness
        .app()
        .state
        .curve
        .points()
        .iter()
        .map(|point| (round(point.x), round(point.y)))
        .collect()
}

const IDENTITY: [(f32, f32); 2] = [(0.0, 0.0), (1.0, 1.0)];
const THREE: [(f32, f32); 3] = [(0.0, 0.0), (0.5, 0.5), (1.0, 1.0)];

#[test]
fn clicking_the_curve_adds_a_point_there() -> TestResult {
    let mut harness = curve_editor(&IDENTITY);
    harness.run()?;
    let plot = harness.control(Role::Canvas, "Tone curve")?;
    assert_eq!(plot.text.as_deref(), Some("0.00,0.00 1.00,1.00"));

    harness.click_at(point_pos(plot.rect, CurvePoint::new(0.25, 0.25)))?;
    assert_eq!(points(&harness), [(0.0, 0.0), (0.25, 0.25), (1.0, 1.0)]);
    assert_eq!(harness.app().state.finishes, 1);
    let plot = harness.control(Role::Canvas, "Tone curve")?;
    assert_eq!(plot.text.as_deref(), Some("0.00,0.00 0.25,0.25 1.00,1.00"));
    Ok(())
}

#[test]
fn clicking_away_from_the_curve_adds_nothing() -> TestResult {
    let mut harness = curve_editor(&IDENTITY);
    harness.run()?;
    let plot = harness.control(Role::Canvas, "Tone curve")?.rect;

    harness.click_at(point_pos(plot, CurvePoint::new(0.2, 0.9)))?;
    assert_eq!(points(&harness), IDENTITY);
    assert_eq!(harness.app().state.changes, 0);
    Ok(())
}

#[test]
fn a_new_point_can_be_placed_in_the_same_drag() -> TestResult {
    let mut harness = curve_editor(&IDENTITY);
    harness.run()?;
    let plot = harness.control(Role::Canvas, "Tone curve")?.rect;

    harness.pointer_down(point_pos(plot, CurvePoint::new(0.5, 0.5)))?;
    harness.pointer_move(point_pos(plot, CurvePoint::new(0.55, 0.65)))?;
    harness.pointer_move(point_pos(plot, CurvePoint::new(0.6, 0.8)))?;
    assert_eq!(
        harness.app().state.finishes,
        0,
        "not finished while the button is down"
    );
    harness.pointer_up(point_pos(plot, CurvePoint::new(0.6, 0.8)))?;

    assert_eq!(points(&harness), [(0.0, 0.0), (0.6, 0.8), (1.0, 1.0)]);
    assert!(harness.app().state.changes >= 3);
    assert_eq!(harness.app().state.finishes, 1);
    Ok(())
}

#[test]
fn dragging_a_point_moves_it() -> TestResult {
    let mut harness = curve_editor(&THREE);
    harness.run()?;
    let plot = harness.control(Role::Canvas, "Tone curve")?.rect;

    harness.drag(
        point_pos(plot, CurvePoint::new(0.5, 0.5)),
        point_pos(plot, CurvePoint::new(0.4, 0.7)),
    )?;
    assert_eq!(points(&harness), [(0.0, 0.0), (0.4, 0.7), (1.0, 1.0)]);
    assert_eq!(harness.app().state.finishes, 1);

    // An end point moves too: lifting the black point.
    harness.drag(
        point_pos(plot, CurvePoint::new(0.0, 0.0)),
        point_pos(plot, CurvePoint::new(0.0, 0.2)),
    )?;
    assert_eq!(points(&harness), [(0.0, 0.2), (0.4, 0.7), (1.0, 1.0)]);
    Ok(())
}

#[test]
fn a_dragged_point_cannot_pass_its_neighbour() -> TestResult {
    let mut harness = curve_editor(&[(0.0, 0.0), (0.3, 0.3), (0.6, 0.6), (1.0, 1.0)]);
    harness.run()?;
    let plot = harness.control(Role::Canvas, "Tone curve")?.rect;

    harness.drag(
        point_pos(plot, CurvePoint::new(0.3, 0.3)),
        point_pos(plot, CurvePoint::new(0.9, 0.5)),
    )?;
    let after = points(&harness);
    assert_eq!(after.len(), 4, "no point was swallowed: {after:?}");
    assert!(after[1].0 < after[2].0, "{after:?}");
    assert_eq!(after[1].1, 0.5);
    Ok(())
}

#[test]
fn double_clicking_a_point_removes_it() -> TestResult {
    let mut harness = curve_editor(&THREE);
    harness.run()?;
    let plot = harness.control(Role::Canvas, "Tone curve")?.rect;

    harness.double_click_at(point_pos(plot, CurvePoint::new(0.5, 0.5)))?;
    assert_eq!(points(&harness), IDENTITY);
    assert_eq!(harness.app().state.finishes, 1);

    // A curve needs two points, so the last two stay.
    harness.double_click_at(point_pos(plot, CurvePoint::new(0.0, 0.0)))?;
    assert_eq!(points(&harness), IDENTITY);
    Ok(())
}

#[test]
fn dragging_a_point_off_the_plot_removes_it() -> TestResult {
    let mut harness = curve_editor(&THREE);
    harness.run()?;
    let plot = harness.control(Role::Canvas, "Tone curve")?.rect;
    let middle = point_pos(plot, CurvePoint::new(0.5, 0.5));
    let far_outside = plot.right_center() + vec2(90.0, 0.0);

    harness.pointer_down(middle)?;
    harness.pointer_move(far_outside)?;
    assert_eq!(points(&harness), IDENTITY, "gone while it is held outside");
    harness.pointer_move(middle + vec2(10.0, 0.0))?;
    assert_eq!(points(&harness).len(), 3, "back when it is brought back in");
    harness.pointer_move(far_outside)?;
    harness.pointer_up(far_outside)?;

    assert_eq!(points(&harness), IDENTITY);
    assert_eq!(harness.app().state.finishes, 1);
    Ok(())
}

#[test]
fn one_click_too_many_does_not_put_a_removed_point_back() -> TestResult {
    let mut harness = curve_editor(&THREE);
    harness.run()?;
    let plot = harness.control(Role::Canvas, "Tone curve")?.rect;
    let middle = point_pos(plot, CurvePoint::new(0.5, 0.5));

    harness.double_click_at(middle)?;
    assert_eq!(points(&harness), IDENTITY);
    // A third click right away lands on the curve where the point was.
    harness.pointer_down(middle)?;
    harness.pointer_up(middle)?;
    assert_eq!(points(&harness), IDENTITY);
    assert_eq!(harness.app().state.finishes, 1);
    Ok(())
}

#[test]
fn a_secondary_click_does_not_edit_the_curve() -> TestResult {
    let mut harness = curve_editor(&IDENTITY);
    harness.run()?;
    let plot = harness.control(Role::Canvas, "Tone curve")?.rect;

    harness.secondary_click_at(point_pos(plot, CurvePoint::new(0.25, 0.25)))?;
    assert_eq!(points(&harness), IDENTITY);
    assert_eq!(harness.app().state.changes, 0);
    Ok(())
}

#[test]
fn typing_a_number_above_the_curve_does_not_break_a_drag_on_it() -> TestResult {
    let curve = PointCurve::new(IDENTITY.iter().map(|(x, y)| CurvePoint::new(*x, *y)));
    let mut harness = HarnessBuilder::new().size([320.0, 320.0]).build_ui(
        (0.0_f32, curve),
        |ui, (contrast, curve)| {
            AdjSlider::new(Adj::Contrast.spec(), contrast, 0.0).show(ui);
            let drawn = curve.clone();
            let sample = |x: f32| straight(drawn.points(), x);
            CurveEditor::new("Tone curve", curve, &sample)
                .size([200.0, 200.0])
                .show(ui);
        },
    );
    harness.run()?;
    harness.click(Role::TextInput, "Contrast")?;
    harness.type_text("30")?;

    // The press that ends the typing (the number goes back to plain text
    // above the plot) adds a point to the curve and goes on to drag it.
    let plot = harness.control(Role::Canvas, "Tone curve")?.rect;
    harness.drag(
        point_pos(plot, CurvePoint::new(0.5, 0.5)),
        point_pos(plot, CurvePoint::new(0.5, 0.7)),
    )?;
    assert_eq!(harness.app().state.0, 30.0);
    let points = harness.app().state.1.points();
    assert_eq!(points.len(), 3, "{points:?}");
    let dragged = points[1];
    assert!(
        (dragged.x - 0.5).abs() < 0.01 && (dragged.y - 0.7).abs() < 0.01,
        "{points:?}"
    );
    Ok(())
}

// ---- Colour wheel ----

struct WheelProbe {
    wheel: GradeWheel,
    changes: u32,
    finishes: u32,
}

fn color_wheel(wheel: GradeWheel) -> Harness<Host<WheelProbe>> {
    let probe = WheelProbe {
        wheel,
        changes: 0,
        finishes: 0,
    };
    HarnessBuilder::new()
        .size([200.0, 240.0])
        .strict_glyphs(true)
        .build_ui(probe, |ui, probe| {
            let luminance = Adj::GradeShadowsLuminance.spec();
            let response = ColorWheel::new("Shadows", &mut probe.wheel, luminance)
                .diameter(100.0)
                .show(ui);
            probe.changes += u32::from(response.changed);
            probe.finishes += u32::from(response.finished);
        })
}

#[test]
fn dragging_the_handle_changes_hue_and_saturation() -> TestResult {
    let mut harness = color_wheel(GradeWheel::default());
    harness.run()?;
    let disc = harness.control(Role::ColorWell, "Shadows wheel")?;
    assert_eq!(disc.text.as_deref(), Some("hue 0, saturation 0"));

    harness.pointer_down(disc.rect.center())?;
    harness.pointer_move(handle_pos(disc.rect, 90.0, 30.0))?;
    let midway = harness.app().state.wheel;
    assert_eq!((midway.hue, midway.saturation), (90.0, 30.0));
    harness.pointer_move(handle_pos(disc.rect, 210.0, 60.0))?;
    assert_eq!(harness.app().state.finishes, 0);
    harness.pointer_up(handle_pos(disc.rect, 210.0, 60.0))?;

    let wheel = harness.app().state.wheel;
    assert_eq!((wheel.hue, wheel.saturation), (210.0, 60.0));
    assert_eq!(wheel.luminance, 0.0, "the disc does not touch luminance");
    assert_eq!(harness.app().state.finishes, 1);
    let disc = harness.control(Role::ColorWell, "Shadows wheel")?;
    assert_eq!(disc.text.as_deref(), Some("hue 210, saturation 60"));
    Ok(())
}

#[test]
fn clicking_the_disc_moves_the_handle_there_and_double_clicking_clears_the_tint() -> TestResult {
    let mut harness = color_wheel(GradeWheel::default());
    harness.run()?;
    let disc = harness.control(Role::ColorWell, "Shadows wheel")?.rect;

    harness.click_at(handle_pos(disc, 45.0, 80.0))?;
    let wheel = harness.app().state.wheel;
    assert_eq!((wheel.hue, wheel.saturation), (45.0, 80.0));

    harness.double_click_at(handle_pos(disc, 45.0, 80.0))?;
    let wheel = harness.app().state.wheel;
    assert_eq!((wheel.hue, wheel.saturation), (0.0, 0.0));
    assert_eq!(harness.app().state.finishes, 2);
    Ok(())
}

#[test]
fn dragging_to_the_centre_removes_the_tint_but_keeps_the_hue() -> TestResult {
    let mut harness = color_wheel(GradeWheel {
        hue: 120.0,
        saturation: 50.0,
        luminance: 0.0,
    });
    harness.run()?;
    let disc = harness.control(Role::ColorWell, "Shadows wheel")?.rect;

    harness.drag(handle_pos(disc, 120.0, 50.0), disc.center())?;
    let wheel = harness.app().state.wheel;
    assert_eq!((wheel.hue, wheel.saturation), (120.0, 0.0));
    Ok(())
}

#[test]
fn the_wheel_has_a_luminance_slider_of_its_own() -> TestResult {
    let mut harness = color_wheel(GradeWheel::default());
    harness.run()?;

    harness.set_value(Role::Slider, "Shadows Luminance", -20.0)?;
    let wheel = harness.app().state.wheel;
    assert_eq!(
        (wheel.hue, wheel.saturation, wheel.luminance),
        (0.0, 0.0, -20.0)
    );
    assert_eq!(harness.app().state.finishes, 1);
    Ok(())
}

#[test]
fn arrow_keys_turn_the_hue_and_change_the_saturation() -> TestResult {
    let mut harness = color_wheel(GradeWheel {
        hue: 359.0,
        saturation: 20.0,
        luminance: 0.0,
    });
    harness.run()?;
    harness.focus(Role::ColorWell, "Shadows wheel")?;

    harness.press_key(Key::ArrowRight, Modifiers::NONE)?;
    assert_eq!(harness.app().state.wheel.hue, 0.0, "the hue wraps around");
    harness.press_key(Key::ArrowLeft, Modifiers::SHIFT)?;
    assert_eq!(harness.app().state.wheel.hue, 350.0);
    harness.press_key(Key::ArrowUp, Modifiers::NONE)?;
    harness.press_key(Key::ArrowDown, Modifiers::SHIFT)?;
    assert_eq!(harness.app().state.wheel.saturation, 11.0);
    assert_eq!(harness.app().state.finishes, 4);
    Ok(())
}

#[test]
fn pressing_the_handle_does_not_round_the_tint() -> TestResult {
    // A preset can leave hue and saturation between whole units.
    let mut harness = color_wheel(GradeWheel {
        hue: 222.4,
        saturation: 34.6,
        luminance: 0.0,
    });
    harness.run()?;
    let disc = harness.control(Role::ColorWell, "Shadows wheel")?.rect;
    let handle = handle_pos(disc, 222.4, 34.6);

    harness.pointer_down(handle)?;
    harness.pointer_up(handle)?;
    let wheel = harness.app().state.wheel;
    assert_eq!((wheel.hue, wheel.saturation), (222.4, 34.6));
    assert_eq!(harness.app().state.changes, 0);
    Ok(())
}

#[test]
fn a_secondary_click_does_not_tint() -> TestResult {
    let mut harness = color_wheel(GradeWheel::default());
    harness.run()?;
    let disc = harness.control(Role::ColorWell, "Shadows wheel")?.rect;

    harness.secondary_click_at(handle_pos(disc, 45.0, 80.0))?;
    let wheel = harness.app().state.wheel;
    assert_eq!((wheel.hue, wheel.saturation), (0.0, 0.0));
    Ok(())
}

#[test]
fn one_click_too_many_does_not_tint_a_cleared_wheel_again() -> TestResult {
    let mut harness = color_wheel(GradeWheel {
        hue: 45.0,
        saturation: 80.0,
        luminance: 0.0,
    });
    harness.run()?;
    let disc = harness.control(Role::ColorWell, "Shadows wheel")?.rect;
    let handle = handle_pos(disc, 45.0, 80.0);

    harness.double_click_at(handle)?;
    // A third click right away, where the handle was.
    harness.pointer_down(handle)?;
    harness.pointer_up(handle)?;
    let wheel = harness.app().state.wheel;
    assert_eq!((wheel.hue, wheel.saturation), (0.0, 0.0));
    assert_eq!(harness.app().state.finishes, 1);
    Ok(())
}
