//! The smart collection dialog: a name, where it goes, and a rule editor of
//! field, operator and value rows, offering for each field exactly the
//! operators and the kind of value the engine's smart rules allow. The same
//! editor changes the rules of an existing smart collection.

use egui::{ComboBox, RichText, TextEdit, Ui};
use photo_core::meta::{CollectionId, Rating};
use photo_engine::{SmartField, SmartOp, SmartRule, SmartRules, SmartValue, SmartValueKind};

use super::parent_menu;
use crate::app::Cx;
use crate::app::actions::Action;
use crate::theme::{palette, space};
use crate::widgets::{Dialog, DialogOutcome, Icon, IconButton, rating_stars};

/// The fields, as the field menu lists them.
const FIELDS: [(SmartField, &str); 18] = [
    (SmartField::Rating, "Rating"),
    (SmartField::Flag, "Pick Flag"),
    (SmartField::Label, "Color Label"),
    (SmartField::Keyword, "Keyword"),
    (SmartField::Title, "Title"),
    (SmartField::Caption, "Caption"),
    (SmartField::Creator, "Creator"),
    (SmartField::Copyright, "Copyright"),
    (SmartField::Camera, "Camera"),
    (SmartField::Lens, "Lens"),
    (SmartField::CaptureDate, "Capture Date"),
    (SmartField::Iso, "ISO"),
    (SmartField::Aperture, "Aperture"),
    (SmartField::FocalLength, "Focal Length"),
    (SmartField::FileKind, "File Type"),
    (SmartField::Edited, "Edited"),
    (SmartField::FileName, "File Name"),
    (SmartField::FolderPath, "Folder"),
];

/// What an operator is called in a rule row.
fn op_name(op: SmartOp) -> &'static str {
    match op {
        SmartOp::Is => "is",
        SmartOp::IsNot => "is not",
        SmartOp::LessThan => "is less than",
        SmartOp::AtMost => "is at most",
        SmartOp::GreaterThan => "is greater than",
        SmartOp::AtLeast => "is at least",
        SmartOp::Contains => "contains",
        SmartOp::DoesNotContain => "does not contain",
        SmartOp::StartsWith => "starts with",
        SmartOp::EndsWith => "ends with",
        SmartOp::IsEmpty => "is empty",
        SmartOp::IsNotEmpty => "is not empty",
    }
}

/// The choices of a field whose value is one of a few words, with how
/// each is shown.
fn choices(kind: SmartValueKind) -> &'static [(&'static str, &'static str)] {
    match kind {
        SmartValueKind::Flag => &[
            ("picked", "Picked"),
            ("unflagged", "Unflagged"),
            ("rejected", "Rejected"),
        ],
        SmartValueKind::Label => &[
            ("none", "None"),
            ("red", "Red"),
            ("yellow", "Yellow"),
            ("green", "Green"),
            ("blue", "Blue"),
            ("purple", "Purple"),
        ],
        SmartValueKind::FileKind => &[("raw", "RAW"), ("raster", "Raster")],
        SmartValueKind::Boolean => &[("yes", "Yes"), ("no", "No")],
        SmartValueKind::Number | SmartValueKind::Text | SmartValueKind::Date => &[],
    }
}

/// One rule as edited: the value is kept as text whatever its kind.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct RuleDraft {
    field: SmartField,
    op: SmartOp,
    value: String,
}

impl RuleDraft {
    /// A fresh rule on `field`: its first operator and a neutral value.
    fn on(field: SmartField) -> Self {
        let kind = field.value_kind();
        let value = match kind {
            SmartValueKind::Number if field == SmartField::Rating => "3".to_owned(),
            _ => choices(kind)
                .first()
                .map_or_else(String::new, |(word, _)| (*word).to_owned()),
        };
        Self {
            field,
            op: field.operators()[0],
            value,
        }
    }

    fn from_rule(rule: &SmartRule) -> Self {
        let value = match &rule.value {
            SmartValue::None => String::new(),
            SmartValue::Bool(true) => "yes".to_owned(),
            SmartValue::Bool(false) => "no".to_owned(),
            SmartValue::Number(number) => number.to_string(),
            SmartValue::Text(text) => text.clone(),
        };
        Self {
            field: rule.field,
            op: rule.op,
            value,
        }
    }

    /// The rule, or `None` while the value is not one the field takes.
    pub(crate) fn rule(&self) -> Option<SmartRule> {
        let value = if matches!(self.op, SmartOp::IsEmpty | SmartOp::IsNotEmpty) {
            SmartValue::None
        } else {
            match self.field.value_kind() {
                SmartValueKind::Number => {
                    let number: f64 = self.value.trim().parse().ok()?;
                    number.is_finite().then_some(SmartValue::Number(number))?
                }
                SmartValueKind::Boolean => SmartValue::Bool(self.value == "yes"),
                SmartValueKind::Text | SmartValueKind::Date => SmartValue::Text(self.value.clone()),
                kind => {
                    let known = choices(kind).iter().any(|(word, _)| *word == self.value);
                    known.then(|| SmartValue::Text(self.value.clone()))?
                }
            }
        };
        Some(SmartRule {
            field: self.field,
            op: self.op,
            value,
        })
    }
}

/// The smart collection dialog.
pub(crate) struct SmartDialog {
    editing: Option<CollectionId>,
    name: String,
    parent: Option<CollectionId>,
    match_all: bool,
    rules: Vec<RuleDraft>,
}

impl SmartDialog {
    /// A new smart collection, inside the set `parent`, starting from a
    /// rating rule.
    pub(crate) fn new(parent: Option<CollectionId>) -> Self {
        Self {
            editing: None,
            name: String::new(),
            parent,
            match_all: true,
            rules: vec![RuleDraft::on(SmartField::Rating)],
        }
    }

    /// The rules of the existing smart collection `id`, called `name`.
    pub(crate) fn edit(id: CollectionId, name: String, rules: &SmartRules) -> Self {
        Self {
            editing: Some(id),
            name,
            parent: None,
            match_all: rules.match_all,
            rules: rules.rules.iter().map(RuleDraft::from_rule).collect(),
        }
    }

    /// The rules as edited, or `None` while one cannot be built.
    fn smart_rules(&self) -> Option<SmartRules> {
        let rules = self
            .rules
            .iter()
            .map(RuleDraft::rule)
            .collect::<Option<Vec<_>>>()?;
        Some(SmartRules {
            match_all: self.match_all,
            rules,
        })
    }

    /// Show the dialog; returns whether it stays open.
    pub(crate) fn show(&mut self, ctx: &egui::Context, cx: &mut Cx<'_>) -> bool {
        let built = self.smart_rules();
        let new = self.editing.is_none();
        let (title, confirm) = if new {
            ("New Smart Collection", "Create")
        } else {
            ("Edit Smart Collection", "Save")
        };
        let can_confirm = built.is_some() && (!new || !self.name.trim().is_empty());
        let response = Dialog::new(title)
            .confirm_label(confirm)
            .can_confirm(can_confirm)
            .width(560.0)
            .show(ctx, |ui| self.body(ui, cx));
        match response.outcome {
            DialogOutcome::Open => true,
            DialogOutcome::Cancelled => false,
            DialogOutcome::Confirmed => {
                if let Some(rules) = built {
                    cx.act(match self.editing {
                        None => Action::CreateSmartCollection {
                            name: self.name.trim().to_owned(),
                            parent: self.parent,
                            rules,
                        },
                        Some(id) => Action::SetSmartRules(id, rules),
                    });
                }
                false
            }
        }
    }

    fn body(&mut self, ui: &mut Ui, cx: &Cx<'_>) {
        if self.editing.is_none() {
            ui.label(RichText::new("Name").color(palette::TEXT_DIM));
            ui.add(
                TextEdit::singleline(&mut self.name)
                    .id_salt("smart-name")
                    .desired_width(f32::INFINITY),
            )
            .accessible_name("Smart collection name");
            ui.add_space(space::MD);
            parent_menu(ui, cx, &mut self.parent);
            ui.add_space(space::MD);
        } else {
            ui.label(RichText::new(&self.name).color(palette::TEXT_STRONG));
            ui.add_space(space::MD);
        }
        ui.horizontal(|ui| {
            ui.label("Match");
            let shown = if self.match_all { "all" } else { "any" };
            ComboBox::from_id_salt("smart-match")
                .selected_text(shown)
                .width(64.0)
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.match_all, true, "all");
                    ui.selectable_value(&mut self.match_all, false, "any");
                })
                .response
                .accessible_name("Match");
            ui.label("of the following rules:");
        });
        ui.add_space(space::SM);
        let mut removed = None;
        for (index, rule) in self.rules.iter_mut().enumerate() {
            if rule_row(ui, index + 1, rule) {
                removed = Some(index);
            }
        }
        if let Some(index) = removed {
            self.rules.remove(index);
        }
        ui.add_space(space::SM);
        if ui.button("Add Rule").clicked() {
            self.rules.push(RuleDraft::on(SmartField::Keyword));
        }
        if self.rules.is_empty() {
            ui.label(RichText::new("With no rules, every photo matches.").color(palette::TEXT_DIM));
        }
    }
}

/// One rule row, numbered from 1. Returns whether its remove button was
/// clicked.
fn rule_row(ui: &mut Ui, number: usize, rule: &mut RuleDraft) -> bool {
    ui.horizontal(|ui| {
        let field_name = FIELDS
            .iter()
            .find(|(field, _)| *field == rule.field)
            .map_or("Field", |(_, name)| *name);
        let mut field = rule.field;
        ComboBox::from_id_salt(("rule-field", number))
            .selected_text(field_name)
            .width(120.0)
            .show_ui(ui, |ui| {
                for (option, name) in FIELDS {
                    ui.selectable_value(&mut field, option, name);
                }
            })
            .response
            .accessible_name(format!("Rule {number} field"));
        if field != rule.field {
            *rule = RuleDraft::on(field);
        }
        ComboBox::from_id_salt(("rule-op", number))
            .selected_text(op_name(rule.op))
            .width(120.0)
            .show_ui(ui, |ui| {
                for op in rule.field.operators() {
                    ui.selectable_value(&mut rule.op, *op, op_name(*op));
                }
            })
            .response
            .accessible_name(format!("Rule {number} operator"));
        if !matches!(rule.op, SmartOp::IsEmpty | SmartOp::IsNotEmpty) {
            value_editor(ui, number, rule);
        }
        ui.add(IconButton::new(
            Icon::Minus,
            &format!("Remove rule {number}"),
        ))
        .clicked()
    })
    .inner
}

/// The value of a rule, edited the way its kind needs.
fn value_editor(ui: &mut Ui, number: usize, rule: &mut RuleDraft) {
    let name = format!("Rule {number} value");
    let kind = rule.field.value_kind();
    if rule.field == SmartField::Rating {
        let stars = rule
            .value
            .trim()
            .parse::<f64>()
            .unwrap_or(0.0)
            .clamp(0.0, 5.0) as u8;
        let mut rating = Rating::new(stars);
        if rating_stars(ui, &format!("Rule {number} rating"), &mut rating).changed() {
            rule.value = rating.stars().to_string();
        }
        return;
    }
    let options = choices(kind);
    if options.is_empty() {
        let hint = match kind {
            SmartValueKind::Date => "YYYY-MM-DD",
            SmartValueKind::Number => "number",
            _ => "text",
        };
        ui.add(
            TextEdit::singleline(&mut rule.value)
                .id_salt(("rule-value", number))
                .hint_text(hint)
                .desired_width(160.0),
        )
        .accessible_name(name);
        return;
    }
    let shown = options
        .iter()
        .find(|(word, _)| *word == rule.value)
        .map_or("", |(_, text)| *text);
    ComboBox::from_id_salt(("rule-value", number))
        .selected_text(shown)
        .width(120.0)
        .show_ui(ui, |ui| {
            for (word, text) in options {
                ui.selectable_value(&mut rule.value, (*word).to_owned(), *text);
            }
        })
        .response
        .accessible_name(name);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_field_is_offered_and_starts_with_a_rule_the_engine_accepts() {
        for (field, _) in FIELDS {
            let draft = RuleDraft::on(field);
            let rule = match field.value_kind() {
                // A number or text field needs the user's value first.
                SmartValueKind::Number if field != SmartField::Rating => continue,
                SmartValueKind::Text | SmartValueKind::Date => continue,
                _ => draft.rule(),
            };
            let rules = SmartRules {
                match_all: true,
                rules: rule.into_iter().collect(),
            };
            assert!(rules.validate().is_ok(), "{field:?}");
        }
    }

    #[test]
    fn a_number_field_needs_a_number() {
        let mut draft = RuleDraft::on(SmartField::Iso);
        assert_eq!(draft.rule(), None);
        draft.value = "400".to_owned();
        assert_eq!(
            draft.rule().map(|rule| rule.value),
            Some(SmartValue::Number(400.0))
        );
        draft.op = SmartOp::IsEmpty;
        assert!(draft.rule().is_some(), "an operator without a value");
    }

    #[test]
    fn rules_read_back_as_they_were_written() {
        let rule = SmartRule {
            field: SmartField::Label,
            op: SmartOp::IsNot,
            value: SmartValue::Text("red".to_owned()),
        };
        assert_eq!(RuleDraft::from_rule(&rule).rule(), Some(rule));
    }
}
