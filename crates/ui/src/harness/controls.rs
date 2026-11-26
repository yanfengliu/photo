//! Every node of the accessibility tree at once, for a driver that lists
//! what is on screen (the agent harness's `observe`) instead of asking for
//! one control by role and name.

use egui::accesskit::{Action, Role, Toggled};
use egui::{Pos2, Rect};
use egui_kittest::kittest::{NodeT as _, Queryable as _};

use super::Harness;

/// One node of the accessibility tree, as [`Harness::controls`] lists it.
#[derive(Clone, Debug, PartialEq)]
pub struct ListedControl {
    /// What kind of control it is.
    pub role: Role,
    /// Its accessible name, by which [`Harness::control`] and the other
    /// lookups find it: a label's text, a widget's own name, or the names
    /// of what it is labelled by.
    pub name: Option<String>,
    /// Its text value (a text field's content, a row's count). A label's
    /// text is its name, not a value.
    pub value: Option<String>,
    /// Its number (a slider's value).
    pub numeric: Option<f64>,
    /// The least number it takes.
    pub min: Option<f64>,
    /// The greatest number it takes.
    pub max: Option<f64>,
    /// The step it moves by.
    pub step: Option<f64>,
    /// Whether it can be used.
    pub enabled: bool,
    /// Where it is, in logical points; `None` for a container without a
    /// rectangle of its own.
    pub rect: Option<Rect>,
    /// On, off or mixed, for switches, checkboxes and toggle buttons.
    pub toggled: Option<Toggled>,
    /// Whether it is selected, for items of a list or grid.
    pub selected: Option<bool>,
    /// Whether it is expanded, for disclosure headers.
    pub expanded: Option<bool>,
    /// Whether it has keyboard focus.
    pub focused: bool,
    /// Whether it takes a click, keyboard focus or a value.
    pub interactive: bool,
    /// Its description (a tooltip).
    pub description: Option<String>,
}

impl ListedControl {
    /// Whether the node is only layout: no name, nothing to say and nothing
    /// to do. egui makes such containers for its panels and rows.
    pub fn is_layout(&self) -> bool {
        let says_something = self.value.as_deref().is_some_and(|value| !value.is_empty());
        self.name.is_none() && !says_something && !self.interactive
    }
}

impl<A: eframe::App + 'static> Harness<A> {
    /// Every node of the accessibility tree as the last frame left it, in
    /// tree order, the window itself left out. Run a frame first
    /// ([`Self::run`], [`Self::step`]) to list what an action changed.
    pub fn controls(&self) -> Vec<ListedControl> {
        let scale = f64::from(self.inner.ctx.pixels_per_point());
        self.inner
            .query_all_by(|_| true)
            .map(|node| {
                let access = node.accesskit_node();
                let data = access.data();
                let role = access.role();
                // A label keeps its text as its value; everything else is
                // named by its own label or by what labels it.
                let name = if role == Role::Label {
                    access.value().or_else(|| access.label())
                } else {
                    access.label()
                }
                .filter(|name| !name.is_empty());
                // The tree is in physical pixels.
                let rect = access.bounding_box().map(|bounds| {
                    let point = |x: f64, y: f64| Pos2::new((x / scale) as f32, (y / scale) as f32);
                    Rect::from_min_max(point(bounds.x0, bounds.y0), point(bounds.x1, bounds.y1))
                });
                let interactive = [Action::Click, Action::Focus, Action::SetValue]
                    .into_iter()
                    .any(|action| data.supports_action(action));
                ListedControl {
                    role,
                    name,
                    value: (role != Role::Label).then(|| access.value()).flatten(),
                    numeric: access.numeric_value(),
                    min: access.min_numeric_value(),
                    max: access.max_numeric_value(),
                    step: access.numeric_value_step(),
                    enabled: !access.is_disabled(),
                    rect,
                    toggled: access.toggled(),
                    selected: access.is_selected(),
                    expanded: data.is_expanded(),
                    focused: access.is_focused(),
                    interactive,
                    description: access.description().filter(|text| !text.is_empty()),
                }
            })
            .collect()
    }
}
