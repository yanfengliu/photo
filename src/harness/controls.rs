//! The controls of one frame, as `observe` lists them and the commands look
//! them up.
//!
//! They are the nodes of the accessibility tree as
//! `photo_ui::harness::Harness::controls` lists them, each named the way
//! the driver's own lookups name it: a label's text, a widget's own name,
//! or the names of what it is labelled by. Nodes that are only layout are
//! left out.

use egui::Rect;
use egui::accesskit::{Role, Toggled};
use photo_ui::harness::ListedControl;
use serde_json::{Value, json};

/// One node of the accessibility tree as the harness reports it.
pub type Control = ListedControl;

/// Every node worth reporting, in tree order: everything named, valued or
/// interactive; not the unnamed containers egui makes for its layout, nor
/// the text runs inside a text field.
pub fn worth_listing(all: Vec<Control>) -> Vec<Control> {
    all.into_iter()
        .filter(|control| !control.is_layout())
        .collect()
}

/// A role's name, as `observe` writes it and `click` accepts it.
pub fn role_name(role: Role) -> String {
    format!("{role:?}")
}

/// The control as `observe` reports it; fields with nothing to say are left
/// out.
pub fn to_json(control: &Control) -> Value {
    let mut value = json!({ "role": role_name(control.role), "enabled": control.enabled });
    let mut put = |key: &str, field: Value| {
        if !field.is_null() {
            value[key] = field;
        }
    };
    put("name", json!(control.name));
    put("value", json!(control.value));
    put("numeric", json!(control.numeric));
    put("min", json!(control.min));
    put("max", json!(control.max));
    put("step", json!(control.step));
    put("rect", json!(control.rect.map(rect_json)));
    put(
        "toggled",
        match control.toggled {
            Some(Toggled::True) => json!(true),
            Some(Toggled::False) => json!(false),
            Some(Toggled::Mixed) => json!("mixed"),
            None => Value::Null,
        },
    );
    put("selected", json!(control.selected));
    put("expanded", json!(control.expanded));
    put("focused", json!(control.focused.then_some(true)));
    put("description", json!(control.description));
    value
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

/// Whether `wanted` (as a client wrote it: `"Button"`, `"button"`,
/// `"list_box_option"`) is the name of `role`.
pub fn role_matches(role: Role, wanted: &str) -> bool {
    let simple = |text: &str| -> String {
        text.chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect()
    };
    simple(&role_name(role)) == simple(wanted)
}

/// A control with nothing set, for tests to fill in.
#[cfg(test)]
pub fn blank(role: Role) -> Control {
    Control {
        role,
        name: None,
        value: None,
        numeric: None,
        min: None,
        max: None,
        step: None,
        enabled: true,
        rect: None,
        toggled: None,
        selected: None,
        expanded: None,
        focused: false,
        interactive: false,
        description: None,
    }
}

#[cfg(test)]
mod tests {
    use egui::pos2;

    use super::*;

    #[test]
    fn layout_and_the_runs_of_a_text_field_are_left_out() {
        let button = Control {
            name: Some("Import…".to_owned()),
            interactive: true,
            ..blank(Role::Button)
        };
        let text = Control {
            name: Some("6 photos found".to_owned()),
            ..blank(Role::Label)
        };
        let field = Control {
            value: Some("/photos".to_owned()),
            interactive: true,
            ..blank(Role::TextInput)
        };
        let run = Control {
            value: Some("/photos".to_owned()),
            ..blank(Role::TextRun)
        };
        let listed = worth_listing(vec![
            blank(Role::GenericContainer),
            button.clone(),
            text.clone(),
            field.clone(),
            run,
        ]);
        assert_eq!(listed, [button, text, field], "in the order given");
    }

    #[test]
    fn a_control_is_reported_without_the_fields_it_has_nothing_to_say_in() {
        let slider = Control {
            name: Some("Exposure".to_owned()),
            numeric: Some(0.25),
            min: Some(-5.0),
            max: Some(5.0),
            step: Some(0.01),
            rect: Some(Rect::from_min_max(
                pos2(1300.04, 620.0),
                pos2(1560.26, 640.0),
            )),
            focused: true,
            interactive: true,
            ..blank(Role::Slider)
        };
        assert_eq!(
            to_json(&slider),
            json!({
                "role": "Slider",
                "enabled": true,
                "name": "Exposure",
                "numeric": 0.25,
                "min": -5.0,
                "max": 5.0,
                "step": 0.01,
                "rect": [1300.0, 620.0, 1560.3, 640.0],
                "focused": true,
            })
        );
        let switch = |toggled| {
            let control = Control {
                toggled: Some(toggled),
                enabled: false,
                ..blank(Role::CheckBox)
            };
            to_json(&control)
        };
        assert_eq!(
            switch(Toggled::True),
            json!({ "role": "CheckBox", "enabled": false, "toggled": true })
        );
        assert_eq!(switch(Toggled::False)["toggled"], false);
        assert_eq!(switch(Toggled::Mixed)["toggled"], "mixed");
        assert!(to_json(&blank(Role::Label)).get("focused").is_none());
    }

    #[test]
    fn roles_are_matched_however_they_are_spelt() {
        assert!(role_matches(Role::ListBoxOption, "list_box_option"));
        assert!(role_matches(Role::ListBoxOption, "ListBoxOption"));
        assert!(role_matches(Role::Button, "button"));
        assert!(!role_matches(Role::Button, "checkbox"));
    }
}
