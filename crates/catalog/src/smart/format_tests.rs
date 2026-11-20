//! Pins the JSON shape of [`SmartRules`] to [`SmartRules::FORMAT`].
//!
//! Claim: if a key of the stored rules is added, removed or renamed, or a
//! field or operator is added, removed or renamed, this test fails until
//! `SmartRules::FORMAT` is bumped and the pin below is updated. Stored rules
//! carry the format number, and a build never replaces rules of a newer
//! format; that protection is only as good as the discipline of bumping the
//! number, which is what this test enforces.

use std::collections::BTreeSet;

use serde_json::Value;

use super::*;

/// The format and the FNV-1a hash of the shape it names. When the test below
/// fails, bump [`SmartRules::FORMAT`] and replace both numbers here with the
/// ones its message prints.
const PINNED: (u32, u64) = (1, 0xcbfb_651a_3ad9_9a39);

/// Every field. The match has no catch-all arm, so a new field does not
/// compile until it is looked at here.
fn every_field() -> Vec<SmartField> {
    use SmartField::*;
    let fields = vec![
        Rating,
        Flag,
        Label,
        Keyword,
        Title,
        Caption,
        Creator,
        Copyright,
        Camera,
        Lens,
        CaptureDate,
        Iso,
        Aperture,
        FocalLength,
        FileKind,
        Edited,
        FileName,
        FolderPath,
    ];
    for field in &fields {
        match field {
            Rating | Flag | Label | Keyword | Title | Caption | Creator | Copyright | Camera
            | Lens | CaptureDate | Iso | Aperture | FocalLength | FileKind | Edited | FileName
            | FolderPath => {}
        }
    }
    fields
}

/// Every operator, guarded the same way.
fn every_op() -> Vec<SmartOp> {
    use SmartOp::*;
    let ops = vec![
        Is,
        IsNot,
        LessThan,
        AtMost,
        GreaterThan,
        AtLeast,
        Contains,
        DoesNotContain,
        StartsWith,
        EndsWith,
        IsEmpty,
        IsNotEmpty,
    ];
    for op in &ops {
        match op {
            Is | IsNot | LessThan | AtMost | GreaterThan | AtLeast | Contains | DoesNotContain
            | StartsWith | EndsWith | IsEmpty | IsNotEmpty => {}
        }
    }
    ops
}

/// Rules holding one value of every kind a rule's value can take.
fn every_value() -> SmartRules {
    let rule = |value| SmartRule {
        field: SmartField::Title,
        op: SmartOp::Is,
        value,
    };
    SmartRules {
        match_all: false,
        rules: vec![
            rule(SmartValue::None),
            rule(SmartValue::Bool(true)),
            rule(SmartValue::Number(1.5)),
            rule(SmartValue::Text("x".into())),
        ],
    }
}

fn key_paths(value: &Value, prefix: &str, out: &mut BTreeSet<String>) {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                let path = format!("{prefix}.{key}");
                out.insert(path.clone());
                key_paths(child, &path, out);
            }
        }
        Value::Array(items) => {
            for item in items {
                key_paths(item, &format!("{prefix}[]"), out);
            }
        }
        Value::String(text) if prefix.is_empty() => {
            out.insert(format!("value:{text}"));
        }
        _ => {}
    }
}

fn fnv1a(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

#[test]
fn the_rules_json_shape_is_pinned_to_its_format() {
    let mut paths = BTreeSet::new();
    key_paths(
        &serde_json::to_value(every_value()).unwrap(),
        "",
        &mut paths,
    );
    for field in every_field() {
        key_paths(&serde_json::to_value(field).unwrap(), "", &mut paths);
    }
    for op in every_op() {
        key_paths(&serde_json::to_value(op).unwrap(), "", &mut paths);
    }
    let shape: Vec<_> = paths.into_iter().collect();
    let hash = fnv1a(&shape.join("\n"));
    assert_eq!(
        (SmartRules::FORMAT, hash),
        PINNED,
        "the JSON shape of SmartRules changed: bump SmartRules::FORMAT, then set PINNED in \
         {} to ({}, {hash:#x}). The shape is now:\n{}",
        file!(),
        SmartRules::FORMAT + 1,
        shape.join("\n"),
    );
}
