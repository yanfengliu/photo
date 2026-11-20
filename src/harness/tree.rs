//! The accessibility tree of one frame, as `observe` lists it and the
//! commands look controls up in it.
//!
//! The nodes are egui's AccessKit nodes, copied at the end of a frame (see
//! `observed`). Names are worked out the way AccessKit, and so the driver's
//! own queries, work them out: a label's text, a widget's own name, or the
//! names of the nodes it is labelled by.

use std::collections::{HashMap, HashSet};

use egui::accesskit::{self, Action, NodeId, Role, Toggled};
use egui::{Rect, pos2};
use serde_json::{Value, json};

/// The accessibility tree of one frame.
pub struct Tree {
    nodes: HashMap<NodeId, accesskit::Node>,
    root: NodeId,
    focused: Option<NodeId>,
}

/// One node of the tree as the harness reports it.
#[derive(Clone, Debug, PartialEq)]
pub struct Control {
    /// What kind of control it is.
    pub role: Role,
    /// Its accessible name, which `click` and the other commands find it
    /// by.
    pub name: Option<String>,
    /// Its text value (a text field's content).
    pub value: Option<String>,
    /// Its number (a slider's value).
    pub numeric: Option<f64>,
    /// The smallest number it takes.
    pub min: Option<f64>,
    /// The largest number it takes.
    pub max: Option<f64>,
    /// The step it moves by.
    pub step: Option<f64>,
    /// Whether it can be used.
    pub enabled: bool,
    /// Where it is, in logical points.
    pub rect: Option<Rect>,
    /// On or off, for switches and checkboxes.
    pub toggled: Option<Toggled>,
    /// Whether it is selected, for items of a list or grid.
    pub selected: Option<bool>,
    /// Whether it is expanded, for disclosure headers.
    pub expanded: Option<bool>,
    /// Whether it has keyboard focus.
    pub focused: bool,
    /// Whether it takes clicks, focus or a value.
    pub interactive: bool,
    /// Its description (a tooltip).
    pub description: Option<String>,
}

impl Control {
    /// The role's name, as `observe` writes it and `click` accepts it.
    pub fn role_name(&self) -> String {
        format!("{:?}", self.role)
    }

    /// The control as `observe` reports it; fields with nothing to say are
    /// left out.
    pub fn to_json(&self) -> Value {
        let mut value = json!({ "role": self.role_name(), "enabled": self.enabled });
        let mut put = |key: &str, field: Value| {
            if !field.is_null() {
                value[key] = field;
            }
        };
        put("name", json!(self.name));
        put("value", json!(self.value));
        put("numeric", json!(self.numeric));
        put("min", json!(self.min));
        put("max", json!(self.max));
        put("step", json!(self.step));
        put("rect", json!(self.rect.map(rect_json)));
        put(
            "toggled",
            match self.toggled {
                Some(Toggled::True) => json!(true),
                Some(Toggled::False) => json!(false),
                Some(Toggled::Mixed) => json!("mixed"),
                None => Value::Null,
            },
        );
        put("selected", json!(self.selected));
        put("expanded", json!(self.expanded));
        put("focused", json!(self.focused.then_some(true)));
        put("description", json!(self.description));
        value
    }
}

/// A rectangle as `[left, top, right, bottom]`, to a tenth of a point.
pub fn rect_json(rect: Rect) -> Value {
    let tenth = |v: f32| (f64::from(v) * 10.0).round() / 10.0;
    json!([
        tenth(rect.min.x),
        tenth(rect.min.y),
        tenth(rect.max.x),
        tenth(rect.max.y)
    ])
}

impl Tree {
    /// Copy the nodes egui has built so far this frame, which at the end of
    /// the app's frame is all of them. `None` when accessibility is off.
    pub fn capture(ctx: &egui::Context) -> Option<Self> {
        let nodes = ctx.viewport(|viewport| {
            viewport.this_pass.accesskit_state.as_ref().map(|state| {
                state
                    .nodes
                    .iter()
                    .map(|(id, node)| (id.accesskit_id(), node.clone()))
                    .collect::<HashMap<_, _>>()
            })
        })?;
        let focused = ctx
            .memory(|memory| memory.focused())
            .map(|id| id.accesskit_id());
        Some(Self {
            nodes,
            root: egui::accesskit_root_id().accesskit_id(),
            focused,
        })
    }

    /// Every node worth reporting, in tree order: everything named, valued
    /// or interactive; not the window itself nor the unnamed containers
    /// egui makes for its layout.
    pub fn controls(&self) -> Vec<Control> {
        let mut order = Vec::with_capacity(self.nodes.len());
        let mut seen = HashSet::new();
        let mut stack = vec![self.root];
        while let Some(id) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            if let Some(node) = self.nodes.get(&id) {
                order.push(id);
                stack.extend(node.children().iter().rev().copied());
            }
        }
        let mut stray: Vec<NodeId> = self
            .nodes
            .keys()
            .filter(|id| !seen.contains(id))
            .copied()
            .collect();
        stray.sort_unstable();
        order.extend(stray);
        order
            .into_iter()
            .filter(|id| *id != self.root)
            .filter_map(|id| self.control(id))
            .collect()
    }

    fn control(&self, id: NodeId) -> Option<Control> {
        let node = self.nodes.get(&id)?;
        let name = self.name_of(node).filter(|name| !name.is_empty());
        let value = (node.role() != Role::Label)
            .then(|| node.value().map(str::to_owned))
            .flatten();
        let interactive = [Action::Click, Action::Focus, Action::SetValue]
            .into_iter()
            .any(|action| node.supports_action(action));
        // An unnamed node with nothing to say is layout, not a control; so
        // are the text runs inside a text field, whose text the field
        // reports itself.
        let says_something =
            node.role() != Role::TextRun && value.as_deref().is_some_and(|value| !value.is_empty());
        if name.is_none() && !says_something && !interactive {
            return None;
        }
        Some(Control {
            role: node.role(),
            name,
            value,
            numeric: node.numeric_value(),
            min: node.min_numeric_value(),
            max: node.max_numeric_value(),
            step: node.numeric_value_step(),
            enabled: !node.is_disabled(),
            rect: node.bounds().map(|bounds| {
                Rect::from_min_max(
                    pos2(bounds.x0 as f32, bounds.y0 as f32),
                    pos2(bounds.x1 as f32, bounds.y1 as f32),
                )
            }),
            toggled: node.toggled(),
            selected: node.is_selected(),
            expanded: node.is_expanded(),
            focused: self.focused == Some(id),
            interactive,
            description: node.description().map(str::to_owned),
        })
    }

    /// A node's accessible name: a label's text; a widget's own name; else
    /// the names of what it is labelled by, followed through nodes that are
    /// themselves only links; else, for buttons and the like, the labels
    /// inside it.
    fn name_of(&self, node: &accesskit::Node) -> Option<String> {
        if node.role() == Role::Label {
            return node.value().map(str::to_owned);
        }
        if let Some(label) = node.label() {
            return Some(label.to_owned());
        }
        let mut parts = Vec::new();
        if node.labelled_by().is_empty() {
            if names_itself_from_content(node.role()) {
                self.inner_labels(node, &mut parts);
            }
        } else {
            let mut visited = HashSet::new();
            let mut stack: Vec<NodeId> = node.labelled_by().iter().rev().copied().collect();
            while let Some(id) = stack.pop() {
                if !visited.insert(id) {
                    continue;
                }
                let Some(target) = self.nodes.get(&id) else {
                    continue;
                };
                if target.label().is_none() && !target.labelled_by().is_empty() {
                    stack.extend(target.labelled_by().iter().rev().copied());
                } else if let Some(part) = direct_name(target) {
                    parts.push(part.to_owned());
                }
            }
        }
        (!parts.is_empty()).then(|| parts.join(" "))
    }

    /// The labels and images inside `node`, looking through unnamed
    /// containers only.
    fn inner_labels(&self, node: &accesskit::Node, parts: &mut Vec<String>) {
        for child in node.children() {
            let Some(child) = self.nodes.get(child) else {
                continue;
            };
            match child.role() {
                Role::Label | Role::Image => {
                    if let Some(part) = direct_name(child) {
                        parts.push(part.to_owned());
                    }
                }
                Role::GenericContainer => self.inner_labels(child, parts),
                _ => {}
            }
        }
    }
}

/// What a node says of itself: a label's text, anything else's own name.
fn direct_name(node: &accesskit::Node) -> Option<&str> {
    if node.role() == Role::Label {
        node.value()
    } else {
        node.label()
    }
}

/// The roles AccessKit names from the labels inside them when nothing
/// else names them.
fn names_itself_from_content(role: Role) -> bool {
    matches!(
        role,
        Role::Button
            | Role::CheckBox
            | Role::DefaultButton
            | Role::Link
            | Role::MenuItem
            | Role::MenuItemCheckBox
            | Role::MenuItemRadio
            | Role::RadioButton
    )
}

/// Whether `wanted` (as a client wrote it: `"Button"`, `"button"`,
/// `"list_box_option"`) is the name of `role`.
pub fn role_matches(role: Role, wanted: &str) -> bool {
    let simple = |text: &str| -> String {
        text.chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    simple(&format!("{role:?}")) == simple(wanted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn node(role: Role) -> accesskit::Node {
        accesskit::Node::new(role)
    }

    fn tree(nodes: Vec<(u64, accesskit::Node)>) -> Tree {
        Tree {
            nodes: nodes
                .into_iter()
                .map(|(id, node)| (NodeId(id), node))
                .collect(),
            root: NodeId(1),
            focused: Some(NodeId(3)),
        }
    }

    #[test]
    fn controls_are_listed_in_tree_order_with_their_names() {
        let mut root = node(Role::Window);
        root.set_children(vec![NodeId(2), NodeId(4), NodeId(7)]);
        let mut panel = node(Role::GenericContainer);
        panel.set_children(vec![NodeId(3), NodeId(5)]);
        let mut button = node(Role::Button);
        button.set_label("Import…");
        button.add_action(Action::Click);
        let mut text = node(Role::Label);
        text.set_value("6 photos found");
        let mut field = node(Role::TextInput);
        field.set_value("/photos");
        field.add_action(Action::Focus);
        field.push_labelled_by(NodeId(7));
        let mut caption = node(Role::Label);
        caption.set_value("Source folder");
        let mut text_run = node(Role::TextRun);
        text_run.set_value("/photos");
        let tree = tree(vec![
            (1, root),
            (2, panel),
            (3, button),
            (4, field),
            (5, text),
            (7, caption),
            (8, text_run),
        ]);
        let controls = tree.controls();
        let names: Vec<Option<&str>> = controls.iter().map(|c| c.name.as_deref()).collect();
        assert_eq!(
            names,
            [
                Some("Import…"),
                Some("6 photos found"),
                Some("Source folder"),
                Some("Source folder")
            ],
            "the unnamed panel and the field's text run are left out"
        );
        assert!(controls[0].focused && controls[0].interactive);
        assert_eq!(controls[2].role, Role::TextInput);
        assert_eq!(controls[2].value.as_deref(), Some("/photos"));
        assert_eq!(controls[1].value, None, "a label's text is its name");
        assert_eq!(controls[0].to_json()["focused"], true);
        assert!(controls[1].to_json().get("focused").is_none());
    }

    #[test]
    fn a_name_is_followed_through_nodes_that_only_point_elsewhere() {
        let mut root = node(Role::Window);
        root.set_children(vec![NodeId(2)]);
        let mut slider = node(Role::Slider);
        slider.add_action(Action::SetValue);
        slider.push_labelled_by(NodeId(3));
        let mut link = node(Role::GenericContainer);
        link.push_labelled_by(NodeId(4));
        let mut caption = node(Role::Label);
        caption.set_value("Exposure");
        let tree = tree(vec![(1, root), (2, slider), (3, link), (4, caption)]);
        assert_eq!(tree.controls()[0].name.as_deref(), Some("Exposure"));
    }

    #[test]
    fn a_button_without_a_name_takes_the_labels_inside_it() {
        let mut root = node(Role::Window);
        root.set_children(vec![NodeId(2)]);
        let mut button = node(Role::Button);
        button.set_children(vec![NodeId(3)]);
        button.add_action(Action::Click);
        let mut inner = node(Role::Label);
        inner.set_value("Check All");
        let tree = tree(vec![(1, root), (2, button), (3, inner)]);
        assert_eq!(tree.controls()[0].name.as_deref(), Some("Check All"));
    }

    #[test]
    fn roles_are_matched_however_they_are_spelt() {
        assert!(role_matches(Role::ListBoxOption, "list_box_option"));
        assert!(role_matches(Role::ListBoxOption, "ListBoxOption"));
        assert!(role_matches(Role::Button, "button"));
        assert!(!role_matches(Role::Button, "checkbox"));
    }
}
