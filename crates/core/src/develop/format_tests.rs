//! Pins the JSON shape of [`DevelopSettings`] to [`SETTINGS_FORMAT`].
//!
//! Claim: if a field of the settings is added, removed or renamed, or an enum
//! value's name changes, this test fails until `SETTINGS_FORMAT` is bumped and
//! the pin below is updated. Stored settings carry the format number, and a
//! build refuses to overwrite settings of a newer format; that protection is
//! only as good as the discipline of bumping the number, which is what this
//! test enforces.

use std::collections::BTreeSet;

use serde_json::Value;

use super::*;
use crate::geom::Point;

/// The format and the FNV-1a hash of the shape it names. When the test below
/// fails, bump [`SETTINGS_FORMAT`] and replace both numbers here with the
/// ones its message prints.
const PINNED: (u32, u64) = (1, 0x4303_a0ef_ec0b_cc6b);

/// Settings that use every field, every field-carrying enum variant and every
/// optional value, so their JSON holds every key the format can contain.
fn every_field() -> DevelopSettings {
    let point = Point::new;
    let stroke = BrushStroke {
        points: vec![point(0.1, 0.2), point(0.3, 0.4)],
        radius: 0.05,
        feather: 50.0,
        flow: 80.0,
        erase: false,
    };
    let shapes = [
        MaskShape::Linear {
            start: point(0.5, 0.0),
            end: point(0.5, 0.5),
        },
        MaskShape::Radial {
            center: point(0.5, 0.5),
            radius_x: 0.2,
            radius_y: 0.1,
            angle: 10.0,
            feather: 50.0,
            invert: false,
        },
        MaskShape::Brush {
            strokes: vec![stroke],
        },
    ];
    let curve = PointCurve::new([CurvePoint::new(0.0, 0.0), CurvePoint::new(1.0, 1.0)]);
    DevelopSettings {
        tone_curve: ToneCurve {
            rgb: curve.clone(),
            red: curve.clone(),
            green: curve.clone(),
            blue: curve,
            ..Default::default()
        },
        crop: Crop {
            rect: Some(CropRect::FULL),
            angle: 1.0,
            constraint: CropConstraint::Ratio {
                width: 3.0,
                height: 2.0,
            },
        },
        spots: vec![Spot {
            mode: SpotMode::Heal,
            target: point(0.5, 0.5),
            source: point(0.6, 0.5),
            radius: 0.02,
            feather: 50.0,
            opacity: 100.0,
        }],
        masks: vec![Mask {
            components: shapes
                .into_iter()
                .map(|shape| MaskComponent {
                    shape,
                    subtract: false,
                })
                .collect(),
            ..Mask::default()
        }],
        ..Default::default()
    }
}

/// The names the format gives to the values of every field-less enum.
fn enum_value_names() -> Vec<Value> {
    let mut names = Vec::new();
    let mut push = |value: Value| names.push(value);
    for treatment in [Treatment::Color, Treatment::BlackAndWhite] {
        push(serde_json::to_value(treatment).unwrap());
    }
    for profile in [
        Profile::Standard,
        Profile::Neutral,
        Profile::Vivid,
        Profile::Linear,
    ] {
        push(serde_json::to_value(profile).unwrap());
    }
    for preset in WbPreset::ALL {
        push(serde_json::to_value(preset).unwrap());
    }
    for mode in [SpotMode::Heal, SpotMode::Clone] {
        push(serde_json::to_value(mode).unwrap());
    }
    for constraint in [CropConstraint::Original, CropConstraint::Free] {
        push(serde_json::to_value(constraint).unwrap());
    }
    names
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
fn the_settings_json_shape_is_pinned_to_its_format() {
    let mut paths = BTreeSet::new();
    key_paths(
        &serde_json::to_value(every_field()).unwrap(),
        "",
        &mut paths,
    );
    for name in enum_value_names() {
        key_paths(&name, "", &mut paths);
    }
    let shape: Vec<_> = paths.into_iter().collect();
    let hash = fnv1a(&shape.join("\n"));
    assert_eq!(
        (SETTINGS_FORMAT, hash),
        PINNED,
        "the JSON shape of DevelopSettings changed: bump SETTINGS_FORMAT, then set PINNED in \
         {} to ({}, {hash:#x}). The shape is now:\n{}",
        file!(),
        SETTINGS_FORMAT + 1,
        shape.join("\n"),
    );
}

#[test]
fn every_field_really_uses_every_optional_part() {
    let settings = every_field();
    assert!(settings.crop.rect.is_some());
    assert_eq!(settings.masks[0].components.len(), 3);
    assert!(!settings.spots.is_empty());
    assert!(!settings.tone_curve.rgb.is_identity() || !settings.tone_curve.rgb.points().is_empty());
}
