//! One row of a source list: the Catalog, Folders, Collections and Keyword
//! List panels.
//!
//! ```text
//!   ▾ ▣ Trips                        128     disclosure, icon, label, count
//!       ▣ Iceland                     96     a row one level down
//! ```
//!
//! A row is a tree item named after what it lists. It is selected while it
//! is the current source. A row that accepts drops lights up while a drag
//! (anything carried with `egui::DragAndDrop`) is held over it; what to do
//! with the drop is the caller's, through the response's
//! `dnd_release_payload`. A row with children has a disclosure triangle of
//! its own, which opens and closes it without selecting it.

use egui::epaint::Shape;
use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align2, CornerRadius, Rect, Response, Role, Sense, Stroke, StrokeKind, Ui, Vec2, WidgetInfo,
    pos2, vec2,
};

use super::icons::Icon;
use crate::theme::{self, palette, radius, size, space};

/// How far each level of the tree is indented.
const INDENT: f32 = 14.0;
/// Width of the click target of the disclosure triangle.
const DISCLOSURE: f32 = 14.0;
/// Edge of the icon.
const ICON: f32 = 14.0;

/// What a row reports for one frame.
#[derive(Clone, Debug)]
pub struct ListRowResponse {
    /// The row's response (its rect, hover, drag-and-drop helpers).
    pub response: Response,
    /// The row was clicked with the primary button: make it the source.
    pub clicked: bool,
    /// The row was clicked with the secondary button: open its menu.
    pub secondary_clicked: bool,
    /// The disclosure triangle was clicked: open or close the row.
    pub toggled: bool,
}

/// One row of a source list. See the [module documentation](self).
#[must_use = "call `.show(ui)`"]
pub struct ListRow<'a> {
    name: &'a str,
    label: &'a str,
    count: Option<u64>,
    depth: usize,
    icon: Option<Icon>,
    selected: bool,
    accepts_drop: bool,
    expanded: Option<bool>,
}

impl<'a> ListRow<'a> {
    /// A row showing `label`, found by tests and assistive technology as
    /// the tree item named `name`.
    pub fn new(name: &'a str, label: &'a str) -> Self {
        Self {
            name,
            label,
            count: None,
            depth: 0,
            icon: None,
            selected: false,
            accepts_drop: false,
            expanded: None,
        }
    }

    /// The number at the right: how many photos the row stands for. It is
    /// also the row's value in the accessibility tree.
    pub fn count(mut self, count: u64) -> Self {
        self.count = Some(count);
        self
    }

    /// How many levels down the tree the row sits.
    pub fn depth(mut self, depth: usize) -> Self {
        self.depth = depth;
        self
    }

    /// The icon before the label.
    pub fn icon(mut self, icon: Icon) -> Self {
        self.icon = Some(icon);
        self
    }

    /// Whether the row is the current choice (the source being shown).
    pub fn selected(mut self, selected: bool) -> Self {
        self.selected = selected;
        self
    }

    /// Whether the row takes drops: it then lights up while a drag is held
    /// over it.
    pub fn accepts_drop(mut self, accepts_drop: bool) -> Self {
        self.accepts_drop = accepts_drop;
        self
    }

    /// Give the row a disclosure triangle, open or closed. Its button is
    /// named `"Expand {name}"` or `"Collapse {name}"`.
    pub fn expanded(mut self, expanded: bool) -> Self {
        self.expanded = Some(expanded);
        self
    }

    /// Show the row across the available width.
    pub fn show(self, ui: &mut Ui) -> ListRowResponse {
        let (rect, response) = ui.allocate_exact_size(
            vec2(ui.available_width(), size::ROW),
            Sense::click_and_drag(),
        );
        let enabled = ui.is_enabled();
        let value = self.count.map(|count| count.to_string());
        response.widget_info(|| WidgetInfo {
            label: Some(self.name.to_owned()),
            current_text_value: value.clone(),
            enabled,
            ..WidgetInfo::new(Role::TreeItem)
        });
        let (selected, expanded) = (self.selected, self.expanded);
        ui.ctx().accesskit_node_builder(response.id, |node| {
            node.set_selected(selected);
            if let Some(expanded) = expanded {
                node.set_expanded(expanded);
            }
        });

        let indent = INDENT * self.depth as f32;
        let mut toggled = false;
        if let Some(expanded) = self.expanded {
            let target = Rect::from_min_size(
                pos2(rect.left() + indent, rect.top()),
                vec2(DISCLOSURE, rect.height()),
            );
            let disclosure = ui.interact(target, response.id.with("disclosure"), Sense::click());
            let verb = if expanded { "Collapse" } else { "Expand" };
            disclosure.widget_info(|| {
                WidgetInfo::labeled(
                    Role::DisclosureTriangle,
                    enabled,
                    format!("{verb} {}", self.name),
                )
            });
            toggled = disclosure.clicked();
        }

        if ui.is_rect_visible(rect) {
            let drop_here = self.accepts_drop
                && response.contains_pointer()
                && egui::DragAndDrop::has_any_payload(ui.ctx());
            self.paint(ui, rect, &response, indent, drop_here);
        }
        ListRowResponse {
            clicked: response.clicked() && !toggled,
            secondary_clicked: response.secondary_clicked(),
            toggled,
            response,
        }
    }

    fn paint(&self, ui: &Ui, rect: Rect, response: &Response, indent: f32, drop_here: bool) {
        let painter = ui.painter();
        let rounding = CornerRadius::same(radius::MD);
        let band = rect.expand2(vec2(space::SM, 0.0));
        if drop_here {
            painter.rect_filled(band, rounding, palette::ACCENT_DIM);
            let edge = Stroke::new(1.0, palette::ACCENT);
            painter.rect_stroke(band, rounding, edge, StrokeKind::Inside);
        } else if self.selected {
            painter.rect_filled(band, rounding, palette::RAISED);
        } else if response.hovered() {
            painter.rect_filled(band, rounding, theme::mix(palette::PANEL, palette::HOVER, 0.4));
        }
        let ink = if self.selected || drop_here {
            palette::TEXT_STRONG
        } else {
            palette::TEXT
        };

        let mut x = rect.left() + indent;
        if let Some(expanded) = self.expanded {
            let center = pos2(x + DISCLOSURE / 2.0, rect.center().y);
            let corners = if expanded {
                vec![
                    center + vec2(-3.5, -2.0),
                    center + vec2(3.5, -2.0),
                    center + vec2(0.0, 2.5),
                ]
            } else {
                vec![
                    center + vec2(-2.0, -3.5),
                    center + vec2(2.5, 0.0),
                    center + vec2(-2.0, 3.5),
                ]
            };
            painter.add(Shape::convex_polygon(corners, palette::TEXT_DIM, Stroke::NONE));
        }
        x += DISCLOSURE;
        if let Some(icon) = self.icon {
            let glyph = Rect::from_min_size(
                pos2(x, rect.center().y - ICON / 2.0),
                Vec2::splat(ICON),
            );
            icon.paint(painter, glyph, palette::TEXT_DIM);
            x += ICON + space::MD;
        }

        let count_width = match self.count {
            Some(count) => {
                let galley = painter.layout_no_wrap(
                    count.to_string(),
                    theme::small_font(),
                    palette::TEXT_DIM,
                );
                let width = galley.size().x;
                let at = Align2::RIGHT_CENTER.anchor_size(rect.right_center(), galley.size());
                painter.galley(at.min, galley, palette::TEXT_DIM);
                width + space::LG
            }
            None => 0.0,
        };
        let room = (rect.right() - count_width - x).max(0.0);
        let mut job = LayoutJob::simple_singleline(self.label.to_owned(), theme::body_font(), ink);
        job.wrap = TextWrapping::truncate_at_width(room);
        let galley = painter.layout_job(job);
        let at = Align2::LEFT_CENTER.anchor_size(pos2(x, rect.center().y), galley.size());
        painter.galley(at.min, galley, ink);
    }
}
