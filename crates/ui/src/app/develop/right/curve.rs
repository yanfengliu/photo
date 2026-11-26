//! The Tone Curve panel: the parametric curve (its four regions and three
//! split points) and the point curves of RGB, red, green and blue, each
//! drawn over the histogram of the frame on screen.
//!
//! What is plotted is the pipeline's own shape of each curve, which the
//! engine hands out ([`plotted`]): the editor never works a curve out for
//! itself, so it shows exactly what is rendered.

use egui::epaint::Shape;
use egui::{Button, Color32, CornerRadius, Pos2, Rect, RichText, Stroke, Ui, lerp, pos2, vec2};
use photo_core::develop::{Adj, PointCurve, ToneCurve};
use photo_engine::{MonotoneCubic, ParametricShape};

use super::{Editing, WIDE_LABEL, edited, heading, sliders_in};
use crate::app::Cx;
use crate::app::develop::drafts::Pending;
use crate::app::develop::state::CurveChannel;
use crate::theme::{self, palette, radius, space};
use crate::widgets::histogram::{BINS, area_mesh, grid_color};
use crate::widgets::{CurveEditor, HistogramData};

/// The region sliders, as the panel lists them.
const REGIONS: [Adj; 4] = [
    Adj::CurveHighlights,
    Adj::CurveLights,
    Adj::CurveDarks,
    Adj::CurveShadows,
];

/// The split sliders.
const SPLITS: [Adj; 3] = [
    Adj::CurveSplitShadows,
    Adj::CurveSplitMidtones,
    Adj::CurveSplitHighlights,
];

/// The channel buttons: what each shows, its accessible name and the curve.
const CHANNELS: [(CurveChannel, &str, &str); 5] = [
    (CurveChannel::Parametric, "Parametric", "Parametric curve"),
    (CurveChannel::Rgb, "RGB", "RGB curve"),
    (CurveChannel::Red, "Red", "Red curve"),
    (CurveChannel::Green, "Green", "Green curve"),
    (CurveChannel::Blue, "Blue", "Blue curve"),
];

/// Show the panel.
pub(super) fn show(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = space::SM;
        for (channel, text, name) in CHANNELS {
            let selected = cx.ui.develop.curve == channel;
            if ui
                .add(Button::selectable(selected, text))
                .accessible_name(name)
                .clicked()
            {
                cx.ui.develop.curve = channel;
            }
        }
    });
    ui.add_space(space::SM);
    let backdrop = super::frame_histogram(cx);
    match cx.ui.develop.curve {
        CurveChannel::Parametric => parametric(ui, cx, editing, backdrop.as_ref()),
        channel => point(ui, cx, editing, channel, backdrop.as_ref()),
    }
}

/// The curve the panel plots for `channel` of `curves`: the pipeline's
/// compiled shape of it, as a function of a perceptual value.
pub(crate) fn plotted(curves: &ToneCurve, channel: CurveChannel) -> Box<dyn Fn(f32) -> f32> {
    let point = |curve: &PointCurve| -> Box<dyn Fn(f32) -> f32> {
        let cubic = MonotoneCubic::new(curve);
        Box::new(move |x| cubic.eval(x))
    };
    match channel {
        CurveChannel::Parametric => {
            let shape = ParametricShape::new(&curves.parametric);
            Box::new(move |v| shape.eval(v))
        }
        CurveChannel::Rgb => point(&curves.rgb),
        CurveChannel::Red => point(&curves.red),
        CurveChannel::Green => point(&curves.green),
        CurveChannel::Blue => point(&curves.blue),
    }
}

/// The three splits as the pipeline's parametric curve uses them (kept in
/// order and apart, whatever the sliders say), read from where its regions
/// peak: the shadows halfway to the first split, the darks halfway between
/// the first two, the highlights halfway from the last to white.
pub(crate) fn splits(shape: &ParametricShape) -> [f32; 3] {
    let [shadows, darks, _, highlights] = shape.region_centres();
    let first = 2.0 * shadows;
    [first, 2.0 * darks - first, 2.0 * highlights - 1.0]
}

/// The height of a plot `width` wide.
fn plot_height(width: f32) -> f32 {
    (width * 0.8).round()
}

fn parametric(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing, backdrop: Option<&HistogramData>) {
    let curves = &editing.settings.tone_curve;
    let shape = ParametricShape::new(&curves.parametric);
    let curve = plotted(curves, CurveChannel::Parametric);
    let width = ui.available_width();
    let (_, plot) = ui.allocate_space(vec2(width, plot_height(width)));
    if ui.is_rect_visible(plot) {
        let painter = ui.painter_at(plot);
        well(&painter, plot, backdrop.map(|data| &data.luma));
        // The splits, and the shading of the regions between them.
        for (index, split) in splits(&shape).iter().enumerate() {
            let x = lerp(plot.x_range(), *split);
            let stroke = Stroke::new(1.0, theme::mix(palette::WELL, palette::OUTLINE, 0.7));
            painter.vline(x, plot.y_range(), stroke);
            let mark = [
                pos2(x, plot.bottom() - 6.0),
                pos2(x - 4.0, plot.bottom()),
                pos2(x + 4.0, plot.bottom()),
            ];
            let color = if index == 1 {
                palette::TEXT
            } else {
                palette::TEXT_DIM
            };
            painter.add(Shape::convex_polygon(mark.to_vec(), color, Stroke::NONE));
        }
        curve_line(&painter, plot, curve, palette::TEXT_STRONG);
    }
    ui.add_space(space::SM);
    heading(ui, "Regions");
    sliders_in(ui, cx, editing, &REGIONS, Some("Curve"), WIDE_LABEL);
    heading(ui, "Splits");
    sliders_in(ui, cx, editing, &SPLITS, None, WIDE_LABEL);
}

fn point(
    ui: &mut Ui,
    cx: &mut Cx<'_>,
    editing: &Editing,
    channel: CurveChannel,
    backdrop: Option<&HistogramData>,
) {
    let mut settings = editing.settings.clone();
    // The curve as it is before this frame's edit, as the pipeline shapes it.
    let sample = plotted(&settings.tone_curve, channel);
    let curves = &mut settings.tone_curve;
    let (curve, name, color, counts): (&mut PointCurve, &str, Color32, Option<&[u32; BINS]>) =
        match channel {
            CurveChannel::Red => (
                &mut curves.red,
                "Tone curve Red",
                palette::LABEL_RED,
                backdrop.map(|data| &data.red),
            ),
            CurveChannel::Green => (
                &mut curves.green,
                "Tone curve Green",
                palette::LABEL_GREEN,
                backdrop.map(|data| &data.green),
            ),
            CurveChannel::Blue => (
                &mut curves.blue,
                "Tone curve Blue",
                palette::LABEL_BLUE,
                backdrop.map(|data| &data.blue),
            ),
            CurveChannel::Rgb | CurveChannel::Parametric => (
                &mut curves.rgb,
                "Tone curve RGB",
                palette::TEXT_STRONG,
                backdrop.map(|data| &data.luma),
            ),
        };
    let width = ui.available_width();
    let mut editor = CurveEditor::new(name, curve, &sample)
        .color(color)
        .size([width, plot_height(width)]);
    if let Some(counts) = counts {
        editor = editor.backdrop(counts);
    }
    let response = editor.show(ui);
    let draft = response
        .changed
        .then(|| Pending::Settings(Box::new(settings)));
    edited(cx, draft, response.finished);
    ui.add_space(space::XS);
    ui.label(
        RichText::new(
            "Press the curve to add a point and drag it; double-click a point to remove it.",
        )
        .color(palette::TEXT_DIM)
        .small(),
    );
}

/// The plot's well: background, quarter lines, the histogram behind and the
/// diagonal of the identity.
fn well(painter: &egui::Painter, plot: Rect, counts: Option<&[u32; BINS]>) {
    painter.rect_filled(plot, CornerRadius::same(radius::MD), palette::WELL);
    let guide = Stroke::new(1.0, grid_color());
    for quarter in 1..4 {
        let t = quarter as f32 / 4.0;
        painter.hline(plot.x_range(), lerp(plot.y_range(), t), guide);
    }
    if let Some(counts) = counts {
        let peak = counts[1..BINS - 1].iter().copied().max().unwrap_or(0);
        let fill = theme::mix(palette::WELL, palette::OUTLINE, 0.45);
        painter.add(Shape::mesh(area_mesh(plot, counts, peak, fill)));
    }
    painter.line_segment([plot.left_bottom(), plot.right_top()], guide);
}

/// The curve `value` across the plot.
fn curve_line(painter: &egui::Painter, plot: Rect, value: impl Fn(f32) -> f32, color: Color32) {
    let steps = (plot.width() / 2.0).ceil().max(2.0) as usize;
    let line: Vec<Pos2> = (0..=steps)
        .map(|step| {
            let x = step as f32 / steps as f32;
            let y = value(x).clamp(0.0, 1.0);
            pos2(lerp(plot.x_range(), x), lerp(plot.bottom()..=plot.top(), y))
        })
        .collect();
    painter.add(Shape::line(line, Stroke::new(1.5, color)));
}
