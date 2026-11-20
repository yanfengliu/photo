//! The histogram view.

use egui::epaint::{Mesh, Shape};
use egui::{
    Color32, CornerRadius, Id, Rect, Response, Role, Sense, Stroke, StrokeKind, Ui, Vec2,
    WidgetInfo, lerp, pos2, vec2,
};

use crate::theme::{self, palette, radius, space};

/// Number of bins in each channel of a histogram.
pub const BINS: usize = 256;

/// Share of a channel's pixels that must sit in an end bin before that end
/// counts as clipped.
const CLIP_SHARE: f64 = 0.0005;
/// Edge of a clipping indicator.
const INDICATOR: f32 = 14.0;

/// The counts a histogram is drawn from: how many pixels fall in each of 256
/// levels, for red, green, blue and luminance.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistogramData {
    /// Red channel counts, darkest level first.
    pub red: [u32; BINS],
    /// Green channel counts.
    pub green: [u32; BINS],
    /// Blue channel counts.
    pub blue: [u32; BINS],
    /// Luminance counts.
    pub luma: [u32; BINS],
}

impl Default for HistogramData {
    fn default() -> Self {
        Self {
            red: [0; BINS],
            green: [0; BINS],
            blue: [0; BINS],
            luma: [0; BINS],
        }
    }
}

impl HistogramData {
    fn channels(&self) -> [&[u32; BINS]; 4] {
        [&self.luma, &self.red, &self.green, &self.blue]
    }

    /// The count the tallest column is scaled to. The two end bins are left
    /// out: clipped pixels pile up there and would flatten everything else.
    fn peak(&self) -> u32 {
        let inner = |channel: &[u32; BINS]| channel[1..BINS - 1].iter().copied().max();
        let peak = self.channels().into_iter().filter_map(inner).max();
        match peak {
            Some(peak) if peak > 0 => peak,
            _ => self
                .channels()
                .into_iter()
                .flat_map(|channel| channel.iter().copied())
                .max()
                .unwrap_or(0),
        }
    }

    /// Whether a noticeable share of pixels sits at the dark end.
    pub fn shadows_clipped(&self) -> bool {
        self.channels()
            .into_iter()
            .any(|channel| end_is_full(channel, 0))
    }

    /// Whether a noticeable share of pixels sits at the bright end.
    pub fn highlights_clipped(&self) -> bool {
        self.channels()
            .into_iter()
            .any(|channel| end_is_full(channel, BINS - 1))
    }
}

fn end_is_full(channel: &[u32; BINS], bin: usize) -> bool {
    let total: u64 = channel.iter().map(|count| u64::from(*count)).sum();
    total > 0 && f64::from(channel[bin]) > total as f64 * CLIP_SHARE
}

/// Which clipping overlays the user has switched on with the histogram's
/// corner indicators.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ClippingToggles {
    /// Show where shadows are clipped.
    pub shadows: bool,
    /// Show where highlights are clipped.
    pub highlights: bool,
}

/// A histogram of red, green, blue and luminance, drawn with additive
/// blending so overlaps read as yellow, cyan, magenta and grey.
///
/// The two corner indicators light up when that end of the range is clipped,
/// and each is a toggle button (named "Show shadow clipping" and "Show
/// highlight clipping") bound to `toggles`. The returned response is marked
/// changed when a toggle was flipped.
pub fn histogram(
    ui: &mut Ui,
    data: &HistogramData,
    toggles: &mut ClippingToggles,
    height: f32,
) -> Response {
    show(ui, data, Some(toggles), height)
}

/// The same histogram for a view that has no clipping overlay (the
/// Library): the corner indicators still light up when that end is clipped,
/// but they are marks, not buttons.
pub fn histogram_readout(ui: &mut Ui, data: &HistogramData, height: f32) -> Response {
    show(ui, data, None, height)
}

fn show(
    ui: &mut Ui,
    data: &HistogramData,
    mut toggles: Option<&mut ClippingToggles>,
    height: f32,
) -> Response {
    let (rect, mut response) =
        ui.allocate_exact_size(vec2(ui.available_width(), height), Sense::hover());
    response.widget_info(|| WidgetInfo::labeled(Role::Image, true, "Histogram"));

    if ui.is_rect_visible(rect) {
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, CornerRadius::same(radius::MD), palette::WELL);
        // No guide lines here: the channels brighten whatever is under them,
        // so a line would show through the fill as a stripe.
        let plot = rect.shrink2(vec2(0.0, space::SM));
        let peak = data.peak();
        for (channel, color) in data.channels().into_iter().zip([
            palette::CHANNEL_LUMA,
            palette::CHANNEL_RED,
            palette::CHANNEL_GREEN,
            palette::CHANNEL_BLUE,
        ]) {
            painter.add(Shape::mesh(area_mesh(plot, channel, peak, color)));
        }
    }

    let shadows = indicator(
        ui,
        response.id.with("shadows"),
        Rect::from_min_size(rect.min + Vec2::splat(space::XS), Vec2::splat(INDICATOR)),
        Corner::Left,
        ("Show shadow clipping", palette::CLIP_SHADOWS),
        (
            data.shadows_clipped(),
            toggles.as_deref_mut().map(|toggles| &mut toggles.shadows),
        ),
    );
    let highlights = indicator(
        ui,
        response.id.with("highlights"),
        Rect::from_min_size(
            rect.right_top() + vec2(-space::XS - INDICATOR, space::XS),
            Vec2::splat(INDICATOR),
        ),
        Corner::Right,
        ("Show highlight clipping", palette::CLIP_HIGHLIGHTS),
        (
            data.highlights_clipped(),
            toggles.map(|toggles| &mut toggles.highlights),
        ),
    );
    for toggle in [shadows, highlights].into_iter().flatten() {
        if toggle.clicked() {
            response.mark_changed();
        }
        response = response.union(toggle);
    }
    response
}

/// A filled area under the columns of `counts`, scaled so `peak` reaches the
/// top of `rect`. Used for histograms and as the backdrop of the curve editor.
pub(crate) fn area_mesh(rect: Rect, counts: &[u32; BINS], peak: u32, color: Color32) -> Mesh {
    let mut mesh = Mesh::default();
    if peak == 0 {
        return mesh;
    }
    mesh.reserve_vertices(2 * BINS);
    mesh.reserve_triangles(2 * (BINS - 1));
    for (bin, count) in counts.iter().enumerate() {
        let x = lerp(rect.x_range(), bin as f32 / (BINS - 1) as f32);
        let level = (*count as f32 / peak as f32).min(1.0);
        mesh.colored_vertex(pos2(x, rect.bottom()), color);
        mesh.colored_vertex(pos2(x, lerp(rect.bottom()..=rect.top(), level)), color);
        if bin > 0 {
            let base = 2 * bin as u32;
            mesh.add_triangle(base - 2, base - 1, base);
            mesh.add_triangle(base - 1, base, base + 1);
        }
    }
    mesh
}

/// The colour of the faint guide lines inside a plot well.
pub(crate) fn grid_color() -> Color32 {
    theme::mix(palette::WELL, palette::OUTLINE, 0.3)
}

/// Which top corner of the histogram an indicator sits in.
#[derive(Clone, Copy)]
enum Corner {
    Left,
    Right,
}

/// One clipping indicator: a corner triangle that is lit in `color` while
/// that end is clipped, and boxed while its overlay is switched on. With no
/// toggle to flip it is only a mark, and returns no response.
fn indicator(
    ui: &Ui,
    id: Id,
    rect: Rect,
    corner: Corner,
    (name, color): (&str, Color32),
    (clipped, toggle): (bool, Option<&mut bool>),
) -> Option<Response> {
    let response = toggle.map(|on| {
        let response = ui.interact(rect, id, Sense::click());
        if response.clicked() {
            *on = !*on;
        }
        let (enabled, state) = (ui.is_enabled(), *on);
        response.widget_info(|| WidgetInfo::selected(Role::Button, enabled, state, name));
        (response, state)
    });
    let state = response.as_ref().is_some_and(|(_, state)| *state);
    let hovered = response
        .as_ref()
        .is_some_and(|(response, _)| response.hovered());
    let focused = response
        .as_ref()
        .is_some_and(|(response, _)| response.has_focus());

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let rounding = CornerRadius::same(radius::SM);
        if state || hovered {
            painter.rect_filled(rect, rounding, palette::HEADER);
        }
        if state {
            let outline = Stroke::new(1.0, palette::TEXT);
            painter.rect_stroke(rect, rounding, outline, StrokeKind::Inside);
        }
        let inner = rect.shrink(3.5);
        let triangle = match corner {
            Corner::Left => vec![inner.left_top(), inner.right_top(), inner.left_bottom()],
            Corner::Right => vec![inner.left_top(), inner.right_top(), inner.right_bottom()],
        };
        let fill = if clipped { color } else { palette::TEXT_OFF };
        painter.add(Shape::convex_polygon(triangle, fill, Stroke::NONE));
        if focused {
            let ring = Stroke::new(1.0, palette::ACCENT);
            painter.rect_stroke(rect.expand(1.0), rounding, ring, StrokeKind::Outside);
        }
    }
    response.map(|(response, _)| response)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(count: u32) -> HistogramData {
        HistogramData {
            red: [count; BINS],
            green: [count; BINS],
            blue: [count; BINS],
            luma: [count; BINS],
        }
    }

    #[test]
    fn the_scale_ignores_spikes_in_the_end_bins() {
        let mut data = flat(10);
        data.red[0] = 9000;
        data.blue[BINS - 1] = 7000;
        data.green[100] = 40;
        assert_eq!(data.peak(), 40);
    }

    #[test]
    fn a_histogram_with_everything_in_the_end_bins_still_has_a_scale() {
        let mut data = HistogramData::default();
        data.luma[0] = 500;
        assert_eq!(data.peak(), 500);
        assert_eq!(HistogramData::default().peak(), 0);
    }

    #[test]
    fn clipping_needs_a_noticeable_share_of_the_pixels() {
        let mut data = flat(1000);
        assert!(data.shadows_clipped() && data.highlights_clipped());
        data.red[0] = 0;
        data.green[0] = 0;
        data.blue[0] = 0;
        data.luma[0] = 0;
        assert!(!data.shadows_clipped());
        // One stray pixel out of a quarter of a million is not clipping.
        data.blue[0] = 1;
        assert!(!data.shadows_clipped());
        assert!(!HistogramData::default().highlights_clipped());
    }

    #[test]
    fn the_area_mesh_spans_the_rect_and_clamps_tall_columns() {
        let rect = Rect::from_min_max(pos2(10.0, 20.0), pos2(110.0, 70.0));
        let mut counts = [5_u32; BINS];
        counts[0] = 1000;
        let mesh = area_mesh(rect, &counts, 10, Color32::WHITE);
        assert_eq!(mesh.vertices.len(), 2 * BINS);
        assert_eq!(mesh.indices.len(), 6 * (BINS - 1));
        assert!(mesh.is_valid());
        let bounds = mesh.calc_bounds();
        assert_eq!((bounds.left(), bounds.right()), (10.0, 110.0));
        assert_eq!(
            bounds.top(),
            20.0,
            "a column above the peak stops at the top"
        );
        assert_eq!(
            mesh.vertices[3].pos.y, 45.0,
            "half the peak is half the height"
        );
        assert!(area_mesh(rect, &counts, 0, Color32::WHITE).is_empty());
    }
}
