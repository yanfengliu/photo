//! Controls, through the accessibility tree: one found by its role and
//! accessible name, described and operated; or every node at once, for a
//! driver that lists what is on screen (the agent harness's `observe`).

use egui::accesskit::{self, Action, Role, Toggled};
use egui::{Pos2, Rect};
use egui_kittest::kittest::{NodeT as _, Queryable as _};

use super::{Harness, HarnessError};

/// What the accessibility tree says about one control.
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    /// Where it is, in logical points.
    pub rect: Rect,
    /// Its number, for sliders and the like.
    pub value: Option<f64>,
    /// The least number it takes, for sliders and the like.
    pub min: Option<f64>,
    /// The greatest number it takes, for sliders and the like.
    pub max: Option<f64>,
    /// Its text: the content of a text field, or a widget's own description
    /// of its state.
    pub text: Option<String>,
    /// Whether it is on, for switches and toggle buttons.
    pub toggled: Option<bool>,
    /// Whether it is selected, for items of a list or grid.
    pub selected: Option<bool>,
    /// Whether it is expanded, for disclosure headers.
    pub expanded: Option<bool>,
    /// Whether it has keyboard focus.
    pub focused: bool,
    /// Whether it is disabled.
    pub disabled: bool,
}

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
    /// to do. egui makes such containers for its panels and rows; so are
    /// the text runs inside a text field, whose text the field reports
    /// itself.
    pub fn is_layout(&self) -> bool {
        let says_something = self.role != Role::TextRun
            && self.value.as_deref().is_some_and(|value| !value.is_empty());
        self.name.is_none() && !says_something && !self.interactive
    }
}

impl<A: eframe::App + 'static> Harness<A> {
    /// Describe the one control with this role and name.
    pub fn control(&self, role: Role, name: &str) -> Result<Control, HarnessError> {
        let node = self.node(role, name)?;
        let access = node.accesskit_node();
        Ok(Control {
            rect: self.rect_of(&node, role, name)?,
            value: access.numeric_value(),
            min: access.min_numeric_value(),
            max: access.max_numeric_value(),
            text: access.value(),
            toggled: access
                .toggled()
                .map(|toggled| toggled == accesskit::Toggled::True),
            selected: access.is_selected(),
            expanded: access.data().is_expanded(),
            focused: access.is_focused(),
            disabled: access.is_disabled(),
        })
    }

    /// How many controls have this role and name (0 when it is not shown).
    pub fn count(&self, role: Role, name: &str) -> usize {
        self.inner.query_all_by_role_and_label(role, name).count()
    }

    /// Click the centre of a control with the pointer.
    pub fn click(&mut self, role: Role, name: &str) -> Result<(), HarnessError> {
        let center = self.control(role, name)?.rect.center();
        self.click_at(center)
    }

    /// Double-click the centre of a control with the pointer.
    pub fn double_click(&mut self, role: Role, name: &str) -> Result<(), HarnessError> {
        let center = self.control(role, name)?.rect.center();
        self.double_click_at(center)
    }

    /// Activate a control through the accessibility tree, the way assistive
    /// technology does. Unlike [`Self::click`] this works when the control is
    /// scrolled out of view or covered.
    pub fn invoke(&mut self, role: Role, name: &str) -> Result<(), HarnessError> {
        self.node(role, name)?.click_accesskit();
        self.run().map(drop)
    }

    /// Give a control keyboard focus.
    pub fn focus(&mut self, role: Role, name: &str) -> Result<(), HarnessError> {
        self.node(role, name)?.focus();
        self.run().map(drop)
    }

    /// Set the number of a slider-like control.
    pub fn set_value(&mut self, role: Role, name: &str, value: f64) -> Result<(), HarnessError> {
        self.node(role, name)?.set_value_accesskit(value);
        self.run().map(drop)
    }

    /// Replace the content of a text field.
    pub fn set_text(&mut self, role: Role, name: &str, text: &str) -> Result<(), HarnessError> {
        self.node(role, name)?.set_text_value_accesskit(text);
        self.run().map(drop)
    }

    fn node<'a>(
        &'a self,
        role: Role,
        name: &'a str,
    ) -> Result<egui_kittest::Node<'a>, HarnessError> {
        let mut matches = self.inner.query_all_by_role_and_label(role, name);
        let first = matches.next().ok_or_else(|| HarnessError::NotFound {
            role,
            name: name.to_owned(),
        })?;
        match matches.count() {
            0 => Ok(first),
            more => Err(HarnessError::Ambiguous {
                role,
                name: name.to_owned(),
                count: more + 1,
            }),
        }
    }

    /// Where `node` is, in logical points. Containers have no rectangle.
    fn rect_of(
        &self,
        node: &egui_kittest::Node<'_>,
        role: Role,
        name: &str,
    ) -> Result<Rect, HarnessError> {
        let bounds =
            node.accesskit_node()
                .bounding_box()
                .ok_or_else(|| HarnessError::NoBounds {
                    role,
                    name: name.to_owned(),
                })?;
        Ok(self.in_points(bounds))
    }

    /// A rectangle of the accessibility tree, which is in physical pixels,
    /// in logical points.
    fn in_points(&self, bounds: accesskit::Rect) -> Rect {
        let scale = self.inner.ctx.pixels_per_point();
        let point = |x: f64, y: f64| Pos2::new(x as f32 / scale, y as f32 / scale);
        Rect::from_min_max(point(bounds.x0, bounds.y0), point(bounds.x1, bounds.y1))
    }

    /// Every node of the accessibility tree as the last frame left it, in
    /// tree order, the window itself left out. Run a frame first
    /// ([`Self::run`], [`Self::step`]) to list what an action changed.
    pub fn controls(&self) -> Vec<ListedControl> {
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
                let rect = access.bounding_box().map(|bounds| self.in_points(bounds));
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
