//! Smart collection rule sets: combining, validating, storing, and the
//! guarantee that a rule's value can never become part of the SQL.

mod common;

use common::names;
use common::smart::{library, matched, one, rule};
use photo_catalog::{
    Catalog, CatalogError, PhotoQuery, SmartField, SmartOp, SmartRules, SmartValue, SmartValueKind,
    Source, TextField,
};
use photo_core::meta::Rating;

#[test]
fn rules_combine_as_all_or_any() {
    let (mut catalog, _) = library();
    let rules = || {
        vec![
            rule(SmartField::Rating, SmartOp::AtLeast, 3),
            rule(SmartField::Label, SmartOp::Is, "red"),
            rule(SmartField::Keyword, SmartOp::Contains, "portrait"),
        ]
    };
    assert_eq!(matched(&mut catalog, true, rules()), "a");
    assert_eq!(matched(&mut catalog, false, rules()), "abcd");
    // With no rules there is nothing to fail, whichever way they combine.
    assert_eq!(matched(&mut catalog, true, Vec::new()), "abcde");
    assert_eq!(matched(&mut catalog, false, Vec::new()), "abcde");
    assert!(SmartRules::default().match_all && SmartRules::default().rules.is_empty());
}

#[test]
fn a_rule_set_may_hold_up_to_the_stated_number_of_rules() {
    let (mut catalog, _) = library();
    let many = |count: usize| -> Vec<_> {
        (0..count)
            .map(|index| rule(SmartField::Iso, SmartOp::IsNot, index as i32))
            .collect()
    };
    // The most that is allowed really runs...
    assert_eq!(
        matched(&mut catalog, true, many(SmartRules::MAX_RULES)),
        "abcde"
    );
    assert_eq!(
        matched(&mut catalog, false, many(SmartRules::MAX_RULES)),
        "abcde"
    );
    // ...and one more is refused before it is stored.
    let too_many = SmartRules {
        match_all: true,
        rules: many(SmartRules::MAX_RULES + 1),
    };
    assert!(matches!(
        too_many.validate(),
        Err(CatalogError::InvalidRule(_))
    ));
    assert!(matches!(
        catalog.create_smart_collection("Too many", None, &too_many),
        Err(CatalogError::InvalidRule(_))
    ));
    assert!(catalog.collections().unwrap().is_empty());
}

#[test]
fn text_that_is_only_blanks_counts_as_empty() {
    let (mut catalog, ids) = library();
    // b has a caption; give the others a tab, a line break, spaces, nothing.
    let blanks = [(ids[0], "\t"), (ids[2], " \r\n "), (ids[3], "   ")];
    catalog.set_text(TextField::Caption, &blanks).unwrap();
    assert_eq!(
        one(
            &mut catalog,
            SmartField::Caption,
            SmartOp::IsEmpty,
            SmartValue::None
        ),
        "acde"
    );
    assert_eq!(
        one(
            &mut catalog,
            SmartField::Caption,
            SmartOp::IsNotEmpty,
            SmartValue::None
        ),
        "b"
    );
}

#[test]
fn any_is_not_confused_by_the_filter_it_is_combined_with() {
    // "rating >= 5 OR label is green" must stay one bracketed condition when
    // the query adds its own conditions next to it.
    let (mut catalog, _) = library();
    let rules = SmartRules {
        match_all: false,
        rules: vec![
            rule(SmartField::Rating, SmartOp::AtLeast, 5),
            rule(SmartField::Label, SmartOp::Is, "green"),
        ],
    };
    let id = catalog
        .create_smart_collection("Either", None, &rules)
        .unwrap();
    let query = PhotoQuery {
        source: Source::Collection(id),
        filter: photo_catalog::Filter {
            kind: Some(photo_core::meta::FileKind::Raster),
            ..Default::default()
        },
        ..Default::default()
    };
    assert_eq!(names(&catalog, &query), ["c.jpg"]);
}

#[test]
fn a_smart_collection_follows_the_catalog_as_it_changes() {
    let (mut catalog, ids) = library();
    let rules = SmartRules {
        match_all: true,
        rules: vec![rule(SmartField::Rating, SmartOp::AtLeast, 4)],
    };
    let id = catalog
        .create_smart_collection("Best", None, &rules)
        .unwrap();
    let held = |catalog: &Catalog| names(catalog, &PhotoQuery::from(Source::Collection(id)));
    assert_eq!(held(&catalog), ["a.arw"]);

    catalog
        .set_ratings(&[(ids[4], Rating::new(5)), (ids[0], Rating::new(1))])
        .unwrap();
    assert_eq!(held(&catalog), ["e.png"]);

    let looser = SmartRules {
        match_all: true,
        rules: vec![rule(SmartField::Rating, SmartOp::AtLeast, 3)],
    };
    catalog.set_smart_rules(id, &looser).unwrap();
    assert_eq!(held(&catalog), ["e.png", "b.arw", "c.jpg"]);
    assert_eq!(catalog.smart_rules(id).unwrap(), looser);
}

#[test]
fn hostile_values_are_compared_as_text_and_cannot_change_the_statement() {
    let (mut catalog, ids) = library();
    let before = catalog.stats().unwrap();
    let hostile = [
        "' OR 1=1 --",
        "'; DROP TABLE photos; --",
        "\" OR \"\"=\"",
        "') OR ('1'='1",
        "%",
        "_",
        "\\",
        "?1",
        "x' UNION SELECT id FROM photos --",
        "\u{0}",
    ];
    let fields = [
        SmartField::Title,
        SmartField::Keyword,
        SmartField::FolderPath,
        SmartField::Camera,
    ];
    let ops = [
        SmartOp::Contains,
        SmartOp::Is,
        SmartOp::StartsWith,
        SmartOp::EndsWith,
    ];
    for value in hostile {
        for field in fields {
            for op in ops {
                // One title really contains an underscore; nothing else in
                // the library contains any of these strings.
                let expected = match (field, op, value) {
                    (SmartField::Title, SmartOp::Contains, "_") => "c",
                    _ => "",
                };
                let found = one(&mut catalog, field, op, value);
                assert_eq!(found, expected, "{field:?} {op:?} {value:?}");
            }
        }
        // The negations match everything instead of nothing.
        let found = one(
            &mut catalog,
            SmartField::Camera,
            SmartOp::DoesNotContain,
            value,
        );
        assert_eq!(found, "abcde", "{value:?}");
    }
    // Wildcard characters are literal: "50%" is found, "5_%" is not.
    assert_eq!(
        one(&mut catalog, SmartField::Caption, SmartOp::Contains, "50%"),
        "b"
    );
    assert_eq!(
        one(&mut catalog, SmartField::Caption, SmartOp::Contains, "5_%"),
        ""
    );

    // A title that *is* an injection attempt is found by exactly that text.
    let attack = "'; DROP TABLE photos; --";
    catalog
        .set_text(TextField::Title, &[(ids[4], attack)])
        .unwrap();
    assert_eq!(
        one(&mut catalog, SmartField::Title, SmartOp::Is, attack),
        "e"
    );
    assert_eq!(
        one(
            &mut catalog,
            SmartField::Title,
            SmartOp::Contains,
            "DROP TABLE"
        ),
        "e"
    );
    catalog.set_text(TextField::Title, &[(ids[4], "")]).unwrap();

    assert_eq!(
        catalog.stats().unwrap(),
        before,
        "nothing was created, altered or dropped"
    );
    assert_eq!(catalog.check_integrity().unwrap(), Vec::<String>::new());
}

#[test]
fn hostile_values_in_number_and_choice_rules_are_refused_not_executed() {
    let (mut catalog, _) = library();
    let before = catalog.stats().unwrap();
    for (field, value) in [
        (SmartField::Rating, "3 OR 1=1"),
        (SmartField::Flag, "picked' OR '1'='1"),
        (SmartField::Label, "red; DROP TABLE photos"),
        (SmartField::FileKind, "raw) OR (1=1"),
        (SmartField::CaptureDate, "2024-03-12' OR '1'='1"),
    ] {
        let rules = SmartRules {
            match_all: true,
            rules: vec![rule(field, SmartOp::Is, value)],
        };
        let refused = catalog.create_smart_collection("attack", None, &rules);
        assert!(
            matches!(refused, Err(CatalogError::InvalidRule(_))),
            "{field:?} {value:?}"
        );
    }
    assert_eq!(catalog.stats().unwrap(), before);
}

#[test]
fn a_rule_whose_parts_do_not_belong_together_is_refused_before_it_is_stored() {
    let (mut catalog, _) = library();
    let bad = [
        rule(SmartField::Rating, SmartOp::Contains, 3),
        rule(SmartField::Rating, SmartOp::Is, "three"),
        rule(SmartField::Rating, SmartOp::Is, f64::NAN),
        rule(SmartField::Iso, SmartOp::AtLeast, f64::INFINITY),
        rule(SmartField::Rating, SmartOp::Is, SmartValue::None),
        rule(SmartField::Title, SmartOp::AtLeast, "a"),
        rule(SmartField::Title, SmartOp::Contains, 5),
        rule(SmartField::Title, SmartOp::Contains, SmartValue::None),
        rule(SmartField::Flag, SmartOp::Is, "maybe"),
        rule(SmartField::Flag, SmartOp::Contains, "picked"),
        rule(SmartField::Label, SmartOp::Is, "magenta"),
        rule(SmartField::FileKind, SmartOp::Is, "video"),
        rule(SmartField::Edited, SmartOp::Is, "yes"),
        rule(SmartField::Edited, SmartOp::AtLeast, true),
        rule(SmartField::CaptureDate, SmartOp::Is, "last tuesday"),
        rule(SmartField::CaptureDate, SmartOp::Is, "2024-02-30"),
        rule(SmartField::CaptureDate, SmartOp::Contains, "2024"),
        rule(SmartField::Keyword, SmartOp::AtMost, "a"),
    ];
    for rule in bad {
        let rules = SmartRules {
            match_all: true,
            rules: vec![rule.clone()],
        };
        assert!(
            matches!(rules.validate(), Err(CatalogError::InvalidRule(_))),
            "{rule:?}"
        );
        let created = catalog.create_smart_collection("bad", None, &rules);
        assert!(
            matches!(created, Err(CatalogError::InvalidRule(_))),
            "{rule:?}"
        );
    }
    assert!(catalog.collections().unwrap().is_empty());

    let id = catalog
        .create_smart_collection("good", None, &SmartRules::default())
        .unwrap();
    let invalid = SmartRules {
        match_all: false,
        rules: vec![rule(SmartField::Iso, SmartOp::StartsWith, "1")],
    };
    assert!(matches!(
        catalog.set_smart_rules(id, &invalid),
        Err(CatalogError::InvalidRule(_))
    ));
    assert_eq!(catalog.smart_rules(id).unwrap(), SmartRules::default());
}

#[test]
fn every_field_lists_the_operators_and_value_kind_that_validate() {
    let fields = [
        SmartField::Rating,
        SmartField::Flag,
        SmartField::Label,
        SmartField::Keyword,
        SmartField::Title,
        SmartField::Caption,
        SmartField::Creator,
        SmartField::Copyright,
        SmartField::Camera,
        SmartField::Lens,
        SmartField::CaptureDate,
        SmartField::Iso,
        SmartField::Aperture,
        SmartField::FocalLength,
        SmartField::FileKind,
        SmartField::Edited,
        SmartField::FileName,
        SmartField::FolderPath,
    ];
    let every_op = [
        SmartOp::Is,
        SmartOp::IsNot,
        SmartOp::LessThan,
        SmartOp::AtMost,
        SmartOp::GreaterThan,
        SmartOp::AtLeast,
        SmartOp::Contains,
        SmartOp::DoesNotContain,
        SmartOp::StartsWith,
        SmartOp::EndsWith,
        SmartOp::IsEmpty,
        SmartOp::IsNotEmpty,
    ];
    let (mut catalog, _) = library();
    for field in fields {
        let sample: SmartValue = match field.value_kind() {
            SmartValueKind::Number => 2.into(),
            SmartValueKind::Text => "text".into(),
            SmartValueKind::Date => "2024-01-31".into(),
            SmartValueKind::Flag => "picked".into(),
            SmartValueKind::Label => "blue".into(),
            SmartValueKind::FileKind => "raster".into(),
            SmartValueKind::Boolean => true.into(),
        };
        assert!(!field.operators().is_empty());
        // Exactly the listed operators validate, and each of them runs.
        for op in every_op {
            let rules = SmartRules {
                match_all: true,
                rules: vec![rule(field, op, sample.clone())],
            };
            let listed = field.operators().contains(&op);
            assert_eq!(rules.validate().is_ok(), listed, "{field:?} {op:?}");
            if listed {
                matched(&mut catalog, true, rules.rules);
            }
        }
    }
}

#[test]
fn rules_are_stored_as_plain_json() {
    let rules = SmartRules {
        match_all: false,
        rules: vec![
            rule(SmartField::Rating, SmartOp::AtLeast, 4),
            rule(SmartField::FolderPath, SmartOp::DoesNotContain, "scans"),
            rule(SmartField::Edited, SmartOp::Is, true),
            rule(SmartField::Title, SmartOp::IsEmpty, SmartValue::None),
        ],
    };
    let json = serde_json::to_string(&rules).unwrap();
    assert_eq!(
        json,
        concat!(
            r#"{"match_all":false,"rules":["#,
            r#"{"field":"rating","op":"at_least","value":4.0},"#,
            r#"{"field":"folder_path","op":"does_not_contain","value":"scans"},"#,
            r#"{"field":"edited","op":"is","value":true},"#,
            r#"{"field":"title","op":"is_empty","value":null}]}"#
        )
    );
    assert_eq!(serde_json::from_str::<SmartRules>(&json).unwrap(), rules);
    // A value may be left out where the operator needs none.
    let terse = r#"{"match_all":true,"rules":[{"field":"keyword","op":"is_empty"}]}"#;
    let terse: SmartRules = serde_json::from_str(terse).unwrap();
    assert_eq!(terse.rules[0].value, SmartValue::None);
}
