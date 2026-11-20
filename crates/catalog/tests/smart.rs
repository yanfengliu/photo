//! Smart collections: every field with every operator it supports.
//!
//! The library (see `common::smart`) holds five photos named a to e; each
//! expectation is the letters of the photos a rule matches.

mod common;

use common::smart::{check, library, one};
use photo_catalog::{SmartField, SmartOp, SmartValue};

#[test]
fn number_fields_support_every_comparison() {
    let (mut catalog, _) = library();
    let c = &mut catalog;
    check(
        c,
        SmartField::Rating,
        &[
            (SmartOp::Is, 3, "bc"),
            (SmartOp::IsNot, 3, "ade"),
            (SmartOp::AtLeast, 3, "abc"),
            (SmartOp::GreaterThan, 3, "a"),
            (SmartOp::AtMost, 3, "bcde"),
            (SmartOp::LessThan, 3, "de"),
        ],
    );
    check(
        c,
        SmartField::Iso,
        &[
            (SmartOp::AtLeast, 1600, "cd"),
            (SmartOp::GreaterThan, 1600, "d"),
            (SmartOp::AtMost, 400, "ab"),
            (SmartOp::Is, 400, "b"),
            // A photo with no ISO recorded is not 400, but it is not above
            // or below anything either.
            (SmartOp::IsNot, 400, "acde"),
            (SmartOp::LessThan, 100_000, "abcd"),
        ],
    );
    check(
        c,
        SmartField::Aperture,
        &[
            // f/2.8 was stored from a 32-bit float; it must still be "2.8".
            (SmartOp::Is, 2.8, "b"),
            (SmartOp::IsNot, 2.8, "acde"),
            (SmartOp::AtMost, 2.8, "ab"),
            (SmartOp::LessThan, 2.8, "a"),
            (SmartOp::GreaterThan, 2.8, "cd"),
            (SmartOp::AtLeast, 2.8, "bcd"),
        ],
    );
    check(
        c,
        SmartField::FocalLength,
        &[
            (SmartOp::AtLeast, 85.0, "ad"),
            (SmartOp::LessThan, 35.0, "c"),
            (SmartOp::Is, 16.0, "c"),
            (SmartOp::IsNot, 16.0, "abde"),
            (SmartOp::AtMost, 35.0, "bc"),
            (SmartOp::GreaterThan, 85.0, "d"),
        ],
    );
}

#[test]
fn flag_label_kind_and_edited_are_or_are_not_a_value() {
    let (mut catalog, _) = library();
    let c = &mut catalog;
    check(
        c,
        SmartField::Flag,
        &[
            (SmartOp::Is, "picked", "ab"),
            (SmartOp::Is, "Rejected", "d"),
            (SmartOp::Is, "unflagged", "ce"),
            (SmartOp::IsNot, "unflagged", "abd"),
        ],
    );
    check(
        c,
        SmartField::Label,
        &[
            (SmartOp::Is, "red", "ad"),
            (SmartOp::Is, "none", "be"),
            (SmartOp::Is, " GREEN ", "c"),
            (SmartOp::IsNot, "red", "bce"),
        ],
    );
    check(
        c,
        SmartField::FileKind,
        &[
            (SmartOp::Is, "raw", "ab"),
            (SmartOp::Is, "raster", "cde"),
            (SmartOp::IsNot, "raw", "cde"),
        ],
    );
    check(
        c,
        SmartField::Edited,
        &[
            (SmartOp::Is, true, "c"),
            (SmartOp::Is, false, "abde"),
            (SmartOp::IsNot, true, "abde"),
        ],
    );
}

#[test]
fn text_fields_support_every_text_operator_ignoring_case() {
    let (mut catalog, _) = library();
    let c = &mut catalog;
    check(
        c,
        SmartField::Title,
        &[
            (SmartOp::Contains, "GARDEN", "ac"),
            (SmartOp::DoesNotContain, "garden", "bde"),
            (SmartOp::StartsWith, "anna", "a"),
            (SmartOp::StartsWith, "garden", "c"),
            (SmartOp::EndsWith, "Garden", "a"),
            (SmartOp::EndsWith, "a title longer than any stored one", ""),
            (SmartOp::Is, "GARDEN_PATH", "c"),
            (SmartOp::IsNot, "garden_path", "abde"),
            // The value is ignored by these two.
            (SmartOp::IsEmpty, "ignored", "bde"),
            (SmartOp::IsNotEmpty, "ignored", "ac"),
        ],
    );
    // No value at all is the natural way to write them.
    assert_eq!(
        one(c, SmartField::Title, SmartOp::IsEmpty, SmartValue::None),
        "bde"
    );
    assert_eq!(
        one(c, SmartField::Lens, SmartOp::IsNotEmpty, SmartValue::None),
        "ab"
    );
    assert_eq!(
        one(c, SmartField::Camera, SmartOp::IsEmpty, SmartValue::None),
        "e"
    );

    check(
        c,
        SmartField::Caption,
        &[(SmartOp::Contains, "street", "b")],
    );
    // Folding is not limited to ASCII.
    check(
        c,
        SmartField::Creator,
        &[(SmartOp::StartsWith, "élodie", "a")],
    );
    check(c, SmartField::Copyright, &[(SmartOp::Is, "cc by", "b")]);
    check(
        c,
        SmartField::Camera,
        &[(SmartOp::Is, "z8", "ab"), (SmartOp::IsNot, "Z8", "cde")],
    );
    check(c, SmartField::Lens, &[(SmartOp::Contains, "85mm", "a")]);
    check(
        c,
        SmartField::FileName,
        &[
            (SmartOp::EndsWith, ".JPG", "cd"),
            (SmartOp::StartsWith, "e.", "e"),
        ],
    );
    check(
        c,
        SmartField::FolderPath,
        &[
            (SmartOp::Contains, "landscapes", "cd"),
            (SmartOp::EndsWith, "/scans", "e"),
            (SmartOp::StartsWith, "/smart/p", "ab"),
            (SmartOp::Is, "/SMART/SCANS", "e"),
            (SmartOp::IsNot, "/smart/scans", "abcd"),
            (SmartOp::DoesNotContain, "portraits", "cde"),
            (SmartOp::IsNotEmpty, "", "abcde"),
            (SmartOp::IsEmpty, "", ""),
        ],
    );
}

#[test]
fn an_empty_text_value_is_contained_in_everything() {
    let (mut catalog, _) = library();
    check(
        &mut catalog,
        SmartField::Title,
        &[
            (SmartOp::Contains, "", "abcde"),
            (SmartOp::StartsWith, "", "abcde"),
            (SmartOp::EndsWith, "", "abcde"),
            (SmartOp::DoesNotContain, "", ""),
            (SmartOp::Is, "", "bde"),
            (SmartOp::IsNot, "", "ac"),
        ],
    );
    check(
        &mut catalog,
        SmartField::Keyword,
        &[
            (SmartOp::Contains, "", "abc"),
            (SmartOp::DoesNotContain, "", "de"),
        ],
    );
}

#[test]
fn keyword_rules_look_at_the_names_of_a_photos_keywords() {
    let (mut catalog, _) = library();
    check(
        &mut catalog,
        SmartField::Keyword,
        &[
            (SmartOp::Contains, "PORT", "ab"),
            (SmartOp::DoesNotContain, "port", "cde"),
            (SmartOp::Is, "street", "b"),
            (SmartOp::IsNot, "street", "acde"),
            (SmartOp::StartsWith, "land", "c"),
            (SmartOp::EndsWith, "den", "a"),
            (SmartOp::IsEmpty, "", "de"),
            (SmartOp::IsNotEmpty, "", "abc"),
        ],
    );
}

#[test]
fn capture_date_rules_compare_calendar_days() {
    let (mut catalog, _) = library();
    check(
        &mut catalog,
        SmartField::CaptureDate,
        &[
            (SmartOp::Is, "2024-03-12", "cd"),
            // A photo with no capture time was not taken on that day.
            (SmartOp::IsNot, "2024-03-12", "abe"),
            (SmartOp::LessThan, "2024-03-11", "a"),
            (SmartOp::AtMost, "2024-03-11", "ab"),
            (SmartOp::GreaterThan, "2024-03-11", "cd"),
            (SmartOp::AtLeast, "2024-03-11", "bcd"),
            // The EXIF spelling of a date is understood too.
            (SmartOp::Is, "2024:03:10", "a"),
        ],
    );
}
