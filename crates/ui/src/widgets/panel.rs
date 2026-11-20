//! Collapsible panels: the sections a side column is made of.
//!
//! ```text
//! ▾ TONE CURVE                    (●)   header: triangle, title, switch
//!   ...body...
//! ▸ COLOR GRADING                       collapsed: header only
//! ```
//!
//! Which panels are open is the caller's presentation state, kept in a
//! [`PanelGroup`] per column. In solo mode opening a panel closes the others;
//! Alt-clicking a header turns solo mode on or off.

use egui::collapsing_header::CollapsingState;
use egui::epaint::Shape;
use egui::{
    Align2, Color32, Frame, Margin, Pos2, Rect, Response, Role, ScrollArea, Sense, Stroke, Ui,
    Vec2, WidgetInfo, pos2, vec2,
};

use crate::theme::{self, palette, size, space};

/// Size of the on/off switch at the right of a header.
const SWITCH: Vec2 = vec2(22.0, 12.0);

/// Which panels of one column are open, and whether the column is in solo
/// mode (at most one panel open at a time).
///
/// Panels are known by their titles, which must be unique within a group.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PanelGroup {
    open: Vec<String>,
    solo: bool,
}

impl PanelGroup {
    /// A group with every panel closed.
    pub fn new() -> Self {
        Self::default()
    }

    /// A group with the panels titled `titles` open.
    pub fn with_open<'a>(titles: impl IntoIterator<Item = &'a str>) -> Self {
        let mut group = Self::new();
        for title in titles {
            group.set_open(title, true);
        }
        group
    }

    /// Whether the panel titled `title` is open.
    pub fn is_open(&self, title: &str) -> bool {
        self.open.iter().any(|open| open == title)
    }

    /// The titles of the open panels, oldest first.
    pub fn open_panels(&self) -> impl Iterator<Item = &str> {
        self.open.iter().map(String::as_str)
    }

    /// Open or close a panel. In solo mode, opening one closes the others.
    pub fn set_open(&mut self, title: &str, open: bool) {
        self.open.retain(|other| other != title);
        if open {
            if self.solo {
                self.open.clear();
            }
            self.open.push(title.to_owned());
        }
    }

    /// Open the panel if it is closed, close it if it is open.
    pub fn toggle(&mut self, title: &str) {
        self.set_open(title, !self.is_open(title));
    }

    /// Whether the group is in solo mode.
    pub fn solo(&self) -> bool {
        self.solo
    }

    /// Turn solo mode on or off. Turning it on keeps only the most recently
    /// opened panel open.
    pub fn set_solo(&mut self, solo: bool) {
        self.solo = solo;
        if solo && self.open.len() > 1 {
            self.open.drain(..self.open.len() - 1);
        }
    }
}

/// What a panel reports for one frame.
pub struct PanelResponse<R> {
    /// The header's response.
    pub header: Response,
    /// The header was clicked and the panel opened or closed.
    pub toggled: bool,
    /// The enable switch was flipped.
    pub switch_changed: bool,
    /// What the body closure returned; `None` while the panel is collapsed.
    pub body: Option<R>,
}

/// A collapsible section with a header. See the [module documentation](self).
#[must_use = "call `.show(ui, group, body)`"]
pub struct Panel<'a> {
    title: &'a str,
    switch: Option<&'a mut bool>,
}

impl<'a> Panel<'a> {
    /// A panel titled `title`. The title is shown in the header, names the
    /// header for accessibility, and identifies the panel in its group.
    pub fn new(title: &'a str) -> Self {
        Self {
            title,
            switch: None,
        }
    }

    /// Put an on/off switch at the right of the header, bound to `enabled`.
    /// It is named `"Enable {title}"`. A panel that is switched off shows its
    /// body dimmed; the controls stay usable.
    pub fn enable_switch(mut self, enabled: &'a mut bool) -> Self {
        self.switch = Some(enabled);
        self
    }

    /// Show the header and, if the panel is open in `group`, the body.
    pub fn show<R>(
        self,
        ui: &mut Ui,
        group: &mut PanelGroup,
        body: impl FnOnce(&mut Ui) -> R,
    ) -> PanelResponse<R> {
        let Self { title, switch } = self;
        ui.push_id(("panel", title), |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            let id = ui.make_persistent_id("header");
            let (_, rect) = ui.allocate_space(vec2(ui.available_width(), size::HEADER));
            let header = ui.interact(rect, id, Sense::click());

            let was_open = group.is_open(title);
            if header.clicked() {
                if ui.input(|input| input.modifiers.alt) {
                    group.set_solo(!group.solo());
                    group.set_open(title, true);
                } else {
                    group.toggle(title);
                }
            }
            let open = group.is_open(title);
            let enabled = ui.is_enabled();
            header.widget_info(|| WidgetInfo::labeled(Role::DisclosureTriangle, enabled, title));
            ui.ctx()
                .accesskit_node_builder(id, |node| node.set_expanded(open));

            let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, open);
            state.set_open(open);
            let switched_on = switch.as_deref().copied().unwrap_or(true);
            if ui.is_rect_visible(rect) {
                paint_header(
                    ui,
                    rect,
                    title,
                    &header,
                    group.solo(),
                    state.openness(ui.ctx()),
                );
            }
            let switch_changed = match switch {
                Some(on) => switch_ui(ui, rect, id.with("switch"), title, on),
                None => false,
            };

            let inner = state.show_body_unindented(ui, |ui| {
                let margin = Margin {
                    left: space::LG as i8,
                    right: space::LG as i8,
                    top: space::MD as i8,
                    bottom: space::LG as i8,
                };
                Frame::new()
                    .inner_margin(margin)
                    .show(ui, |ui| {
                        ui.spacing_mut().item_spacing = vec2(space::MD, space::XS);
                        if !switched_on {
                            ui.multiply_opacity(0.5);
                        }
                        body(ui)
                    })
                    .inner
            });
            // The body can close its own panel with `ui.close()`.
            if open && !state.is_open() {
                group.set_open(title, false);
            }

            PanelResponse {
                header,
                toggled: was_open != group.is_open(title),
                switch_changed,
                body: inner.map(|inner| inner.inner),
            }
        })
        .inner
    }
}

/// A scrolling column of [`Panel`]s: full width, no gaps between the panels.
pub fn panel_column<R>(
    ui: &mut Ui,
    id_salt: impl egui::AsIdSalt,
    add_panels: impl FnOnce(&mut Ui) -> R,
) -> R {
    ScrollArea::vertical()
        .id_salt(id_salt)
        .auto_shrink([false, false])
        .show(ui, |ui| {
            ui.spacing_mut().item_spacing.y = 0.0;
            add_panels(ui)
        })
        .inner
}

fn paint_header(ui: &Ui, rect: Rect, title: &str, header: &Response, solo: bool, openness: f32) {
    let painter = ui.painter();
    let fill = if header.is_pointer_button_down_on() {
        palette::PRESSED
    } else if header.hovered() {
        theme::mix(palette::HEADER, palette::HOVER, 0.35)
    } else {
        palette::HEADER
    };
    painter.rect_filled(rect, 0.0, fill);
    painter.hline(
        rect.x_range(),
        rect.top() + 0.5,
        Stroke::new(1.0, palette::SEPARATOR),
    );

    let color = if header.hovered() || openness > 0.0 {
        palette::TEXT
    } else {
        palette::TEXT_DIM
    };
    let center = pos2(rect.left() + space::LG + 3.0, rect.center().y);
    let triangle = disclosure_triangle(center, openness).to_vec();
    if solo {
        // A hollow triangle marks a column in solo mode.
        painter.add(Shape::closed_line(triangle, Stroke::new(1.0, color)));
    } else {
        painter.add(Shape::convex_polygon(triangle, color, Stroke::NONE));
    }

    let galley = painter.layout_job(theme::header_text(title, color));
    let text = Align2::LEFT_CENTER.anchor_size(
        pos2(rect.left() + space::LG + 14.0, rect.center().y),
        galley.size(),
    );
    painter.galley(text.min, galley, color);
    if header.has_focus() {
        painter.rect_stroke(
            rect.shrink(1.0),
            0.0,
            Stroke::new(1.0, palette::ACCENT),
            egui::StrokeKind::Inside,
        );
    }
}

/// The corners of the disclosure triangle around `center`: pointing right
/// when `openness` is 0 and down when it is 1.
fn disclosure_triangle(center: Pos2, openness: f32) -> [Pos2; 3] {
    let angle = (1.0 - openness.clamp(0.0, 1.0)) * -std::f32::consts::FRAC_PI_2;
    let (sin, cos) = angle.sin_cos();
    // Pointing down, before rotation.
    [vec2(-4.0, -2.5), vec2(4.0, -2.5), vec2(0.0, 3.5)].map(|corner| {
        center
            + vec2(
                corner.x * cos - corner.y * sin,
                corner.x * sin + corner.y * cos,
            )
    })
}

/// The on/off switch at the right of the header. Returns whether it flipped.
fn switch_ui(ui: &Ui, header: Rect, id: egui::Id, title: &str, on: &mut bool) -> bool {
    let rect = Rect::from_center_size(
        pos2(
            header.right() - space::LG - SWITCH.x / 2.0,
            header.center().y,
        ),
        SWITCH,
    );
    // A taller target than the drawn switch: the whole height of the header.
    let target = rect.expand2(vec2(space::SM, (header.height() - SWITCH.y) / 2.0));
    let response = ui.interact(target, id, Sense::click());
    let flipped = response.clicked();
    if flipped {
        *on = !*on;
    }
    let (enabled, state) = (ui.is_enabled(), *on);
    response.widget_info(|| {
        WidgetInfo::selected(Role::Switch, enabled, state, format!("Enable {title}"))
    });

    if ui.is_rect_visible(rect) {
        let travel = ui.ctx().animate_bool_responsive(id, *on);
        let track: Color32 = theme::mix(palette::RAISED, palette::ACCENT, travel);
        let painter = ui.painter();
        painter.rect_filled(rect, SWITCH.y / 2.0, track);
        let knob_radius = SWITCH.y / 2.0 - 2.0;
        let knob_x = egui::lerp(
            rect.left() + SWITCH.y / 2.0..=rect.right() - SWITCH.y / 2.0,
            travel,
        );
        let knob = if response.hovered() {
            palette::TEXT_STRONG
        } else {
            palette::TEXT
        };
        painter.circle_filled(pos2(knob_x, rect.center().y), knob_radius, knob);
        if response.has_focus() {
            painter.rect_stroke(
                rect.expand(2.0),
                SWITCH.y / 2.0 + 2.0,
                Stroke::new(1.0, palette::ACCENT_STRONG),
                egui::StrokeKind::Inside,
            );
        }
    }
    flipped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panels_open_and_close_independently() {
        let mut group = PanelGroup::with_open(["Basic"]);
        assert!(group.is_open("Basic") && !group.is_open("Detail"));
        group.toggle("Detail");
        group.toggle("Basic");
        assert_eq!(group.open_panels().collect::<Vec<_>>(), ["Detail"]);
        group.set_open("Detail", true);
        assert_eq!(
            group.open_panels().count(),
            1,
            "opening twice is opening once"
        );
    }

    #[test]
    fn in_solo_mode_opening_one_panel_closes_the_others() {
        let mut group = PanelGroup::with_open(["Basic", "Detail", "Effects"]);
        group.set_solo(true);
        assert_eq!(group.open_panels().collect::<Vec<_>>(), ["Effects"]);
        group.set_open("Basic", true);
        assert_eq!(group.open_panels().collect::<Vec<_>>(), ["Basic"]);
        group.toggle("Basic");
        assert_eq!(
            group.open_panels().count(),
            0,
            "the open panel can still be closed"
        );

        group.set_solo(false);
        group.set_open("Basic", true);
        group.set_open("Detail", true);
        assert_eq!(group.open_panels().count(), 2);
    }

    #[test]
    fn the_triangle_points_right_when_closed_and_down_when_open() {
        let center = pos2(10.0, 10.0);
        let tip = |openness: f32| disclosure_triangle(center, openness)[2];
        let closed = tip(0.0);
        assert!(closed.x > center.x + 3.0 && (closed.y - center.y).abs() < 1.0e-3);
        let open = tip(1.0);
        assert!(open.y > center.y + 3.0 && (open.x - center.x).abs() < 1.0e-3);
    }
}
