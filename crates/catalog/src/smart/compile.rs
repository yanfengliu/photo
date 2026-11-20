//! Smart collection rules into SQL.
//!
//! The field and operator of a rule select statement text that is written
//! out in this file; the rule's value is only ever bound as a parameter. So
//! whatever a value contains, it can be compared but never executed.

use photo_core::meta::{ColorLabel, FileKind, Flag};
use photo_core::time::DateTime;

use super::{SmartField, SmartOp, SmartRule, SmartRules, SmartValue, SmartValueKind};
use crate::codec::{flag_to_sql, kind_to_sql, label_to_sql};
use crate::error::{CatalogError, Result};
use crate::sql::Sql;
use crate::text::fold;

/// Append ` AND (...)` restricting `photos p` to the photos the rules match.
/// Fails, having appended nothing usable, if a rule is not valid.
pub(crate) fn push_rules(sql: &mut Sql, rules: &SmartRules) -> Result<()> {
    if rules.rules.is_empty() {
        return Ok(());
    }
    if rules.rules.len() > SmartRules::MAX_RULES {
        return Err(CatalogError::InvalidRule(format!(
            "{} rules, but at most {} can be combined",
            rules.rules.len(),
            SmartRules::MAX_RULES
        )));
    }
    sql.push(" AND (");
    for (index, rule) in rules.rules.iter().enumerate() {
        if index > 0 {
            sql.push(if rules.match_all { " AND " } else { " OR " });
        }
        sql.push("(");
        push_rule(sql, rule)?;
        sql.push(")");
    }
    sql.push(")");
    Ok(())
}

/// The SQL expression holding a field's value. Keyword and folder rules run
/// inside a subquery over `keywords k` or `folders f`; the rest read a column
/// of `photos p`.
fn expression(field: SmartField) -> &'static str {
    match field {
        SmartField::Rating => "p.rating",
        SmartField::Flag => "p.flag",
        SmartField::Label => "p.label",
        SmartField::Title => "p.title",
        SmartField::Caption => "p.caption",
        SmartField::Creator => "p.creator",
        SmartField::Copyright => "p.copyright",
        SmartField::Camera => "COALESCE(p.camera, '')",
        SmartField::Lens => "COALESCE(p.lens, '')",
        SmartField::CaptureDate => "substr(p.capture_time, 1, 10)",
        SmartField::Iso => "p.iso",
        SmartField::Aperture => "p.aperture",
        SmartField::FocalLength => "p.focal_length",
        SmartField::FileKind => "p.kind",
        SmartField::Edited => "p.has_edits",
        SmartField::FileName => "p.file_name",
        SmartField::Keyword => "k.name",
        SmartField::FolderPath => "f.path",
    }
}

fn invalid(rule: &SmartRule, problem: &str) -> CatalogError {
    CatalogError::InvalidRule(format!(
        "{:?} {:?} {:?}: {problem}",
        rule.field, rule.op, rule.value
    ))
}

fn push_rule(sql: &mut Sql, rule: &SmartRule) -> Result<()> {
    let field = rule.field;
    if !field.operators().contains(&rule.op) {
        return Err(invalid(rule, "the operator does not apply to this field"));
    }
    let column = expression(field);
    match (field.value_kind(), &rule.value) {
        (SmartValueKind::Text, _) if matches!(rule.op, SmartOp::IsEmpty | SmartOp::IsNotEmpty) => {
            push_text_rule(sql, field, rule.op, "");
        }
        (SmartValueKind::Text, SmartValue::Text(text)) => push_text_rule(sql, field, rule.op, text),
        (SmartValueKind::Number, SmartValue::Number(number)) if number.is_finite() => {
            // Aperture and focal length were stored from 32-bit floats, so
            // the value is rounded the same way before it is compared.
            let number = match field {
                SmartField::Aperture | SmartField::FocalLength => f64::from(*number as f32),
                _ => *number,
            };
            sql.push(column).push(comparison(rule.op)).bind(number);
        }
        (SmartValueKind::Date, SmartValue::Text(day)) => {
            let day = format!("{}T00:00:00", day.trim())
                .parse::<DateTime>()
                .map_err(|_| invalid(rule, "expected a date as YYYY-MM-DD"))?;
            sql.push(column)
                .push(comparison(rule.op))
                .bind(day.date_string());
        }
        (SmartValueKind::Flag, SmartValue::Text(name)) => {
            let flag = named::<Flag>(name)
                .ok_or_else(|| invalid(rule, "expected picked, unflagged or rejected"))?;
            sql.push(column)
                .push(comparison(rule.op))
                .bind(flag_to_sql(flag));
        }
        (SmartValueKind::Label, SmartValue::Text(name)) => {
            let label = named::<ColorLabel>(name)
                .ok_or_else(|| invalid(rule, "expected a label colour or none"))?;
            sql.push(column)
                .push(comparison(rule.op))
                .bind(label_to_sql(label));
        }
        (SmartValueKind::FileKind, SmartValue::Text(name)) => {
            let kind =
                named::<FileKind>(name).ok_or_else(|| invalid(rule, "expected raw or raster"))?;
            sql.push(column)
                .push(comparison(rule.op))
                .bind(kind_to_sql(kind));
        }
        (SmartValueKind::Boolean, SmartValue::Bool(value)) => {
            sql.push(column).push(comparison(rule.op)).bind(*value);
        }
        _ => {
            return Err(invalid(
                rule,
                "the value is of the wrong kind for this field",
            ));
        }
    }
    Ok(())
}

/// The SQL operator for a comparison. `IS NOT` rather than `<>` so that a
/// photo where the value is not recorded counts as "is not" any value.
fn comparison(op: SmartOp) -> &'static str {
    match op {
        SmartOp::IsNot => " IS NOT ",
        SmartOp::LessThan => " < ",
        SmartOp::AtMost => " <= ",
        SmartOp::GreaterThan => " > ",
        SmartOp::AtLeast => " >= ",
        _ => " = ",
    }
}

/// One of photo-core's enums from the name it is serialized under, ignoring
/// case and surrounding white space.
fn named<T: serde::de::DeserializeOwned>(name: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(name.trim().to_lowercase())).ok()
}

fn push_text_rule(sql: &mut Sql, field: SmartField, op: SmartOp, text: &str) {
    match field {
        SmartField::Keyword => {
            // A photo has many keywords, so the question is whether any of
            // them matches; the negative operators negate that.
            let (negated, positive) = match op {
                SmartOp::DoesNotContain => (true, SmartOp::Contains),
                SmartOp::IsNot => (true, SmartOp::Is),
                SmartOp::IsEmpty => (true, SmartOp::IsNotEmpty),
                other => (false, other),
            };
            sql.push(if negated {
                "p.id NOT IN ("
            } else {
                "p.id IN ("
            });
            sql.push("SELECT tagged.photo_id FROM photo_keywords tagged");
            if positive != SmartOp::IsNotEmpty {
                sql.push(" WHERE tagged.keyword_id IN (SELECT k.id FROM keywords k WHERE ");
                push_text_match(sql, expression(field), positive, text);
                sql.push(")");
            }
            sql.push(")");
        }
        SmartField::FolderPath => {
            sql.push("p.folder_id IN (SELECT f.id FROM folders f WHERE ");
            push_text_match(sql, expression(field), op, text);
            sql.push(")");
        }
        _ => push_text_match(sql, expression(field), op, text),
    }
}

/// Second argument for SQL `trim`: the characters that count as blank (tab,
/// the line breaks and space). Without it `trim` removes spaces only.
const BLANKS: &str = ", char(9, 10, 11, 12, 13, 32)";

/// Compare the text in `column` with `text`, both case-folded. Every
/// character of `text` is literal; there are no wildcards.
fn push_text_match(sql: &mut Sql, column: &'static str, op: SmartOp, text: &str) {
    let needle = fold(text);
    match op {
        SmartOp::Contains | SmartOp::StartsWith | SmartOp::EndsWith if needle.is_empty() => {
            sql.push("1");
        }
        SmartOp::DoesNotContain if needle.is_empty() => {
            sql.push("0");
        }
        SmartOp::IsEmpty => {
            sql.push("trim(").push(column).push(BLANKS).push(") = ''");
        }
        SmartOp::IsNotEmpty => {
            sql.push("trim(").push(column).push(BLANKS).push(") <> ''");
        }
        SmartOp::EndsWith => {
            sql.push("substr(photo_fold(")
                .push(column)
                .push("), -length(")
                .bind(needle.clone());
            sql.push(")) = ").bind(needle);
        }
        SmartOp::Is => {
            sql.push("photo_fold(")
                .push(column)
                .push(") = ")
                .bind(needle);
        }
        SmartOp::IsNot => {
            sql.push("photo_fold(")
                .push(column)
                .push(") <> ")
                .bind(needle);
        }
        _ => {
            sql.push("instr(photo_fold(")
                .push(column)
                .push("), ")
                .bind(needle);
            sql.push(match op {
                SmartOp::DoesNotContain => ") = 0",
                SmartOp::StartsWith => ") = 1",
                _ => ") > 0",
            });
        }
    }
}
