//! The free-text filter: letter case beyond ASCII, and long texts.

mod common;

use common::{import, memory, names, raster};
use photo_catalog::{
    Catalog, Filter, PhotoQuery, SmartField, SmartOp, SmartRule, SmartRules, Source,
};

fn titled(titles: &[&str]) -> Catalog {
    let photos: Vec<_> = titles
        .iter()
        .enumerate()
        .map(|(index, title)| {
            let mut photo = raster(&format!("/t/{index}.jpg"));
            photo.title = title.to_string();
            photo
        })
        .collect();
    let mut catalog = memory();
    import(&mut catalog, &photos);
    catalog
}

fn found(catalog: &Catalog, text: &str) -> Vec<String> {
    let query = PhotoQuery {
        filter: Filter {
            text: text.into(),
            ..Filter::default()
        },
        ..PhotoQuery::default()
    };
    names(catalog, &query)
}

fn smart(catalog: &mut Catalog, op: SmartOp, value: &str) -> Vec<String> {
    let rules = SmartRules {
        match_all: true,
        rules: vec![SmartRule {
            field: SmartField::Title,
            op,
            value: value.into(),
        }],
    };
    let id = catalog
        .create_smart_collection("probe", None, &rules)
        .unwrap();
    let held = names(catalog, &PhotoQuery::from(Source::Collection(id)));
    catalog.delete_collection(id).unwrap();
    held
}

#[test]
fn a_part_of_a_word_is_found_in_any_letter_case() {
    // Greek writes sigma differently at the end of a word, so lower-casing
    // "ΚΟΣ" on its own and inside "ΚΟΣΜΟΣ" gives different letters.
    let mut catalog = titled(&["ΚΟΣΜΟΣ", "κόσμος", "other"]);
    for needle in ["ΚΟΣ", "κοσ", "Κοσ", "ΟΣΜΟΣ", "κοσμος", "ΚΟΣΜΟΣ", "σ", "ς"]
    {
        assert!(
            found(&catalog, needle).contains(&"0.jpg".to_string()),
            "{needle}"
        );
    }
    // Final and medial sigma are one letter; an accent is a different one.
    assert_eq!(found(&catalog, "ΜΟΣ"), ["0.jpg", "1.jpg"]);
    assert_eq!(found(&catalog, "κοσμος"), ["0.jpg"]);
    assert_eq!(found(&catalog, "ΚΌΣΜΟΣ"), ["1.jpg"]);

    assert_eq!(smart(&mut catalog, SmartOp::StartsWith, "ΚΟΣ"), ["0.jpg"]);
    assert_eq!(
        smart(&mut catalog, SmartOp::EndsWith, "μοσ"),
        ["0.jpg", "1.jpg"]
    );
    assert_eq!(smart(&mut catalog, SmartOp::Is, "κοσμοσ"), ["0.jpg"]);
    assert_eq!(smart(&mut catalog, SmartOp::Contains, "ΟΣΜ"), ["0.jpg"]);
}

#[test]
fn sharp_s_matches_double_s_and_dotted_capital_i_matches_i() {
    let mut catalog = titled(&["Hauptstraße", "HAUPTSTRASSE", "İstanbul", "plain"]);
    for needle in ["straße", "STRASSE", "strasse", "Straße", "STRAẞE"] {
        assert_eq!(found(&catalog, needle), ["0.jpg", "1.jpg"], "{needle}");
    }
    for needle in ["istanbul", "ISTANBUL", "İSTANBUL"] {
        assert_eq!(found(&catalog, needle), ["2.jpg"], "{needle}");
    }
    assert_eq!(
        smart(&mut catalog, SmartOp::EndsWith, "SSE"),
        ["0.jpg", "1.jpg"]
    );
    assert_eq!(
        smart(&mut catalog, SmartOp::Is, "hauptstrasse"),
        ["0.jpg", "1.jpg"]
    );
}

#[test]
fn ligatures_the_micro_sign_and_long_s_match_the_letters_they_stand_for() {
    let mut catalog = titled(&["ﬁeld trip", "10 µm grain", "Meſſe", "plain"]);
    assert_eq!(found(&catalog, "FIELD"), ["0.jpg"]);
    assert_eq!(found(&catalog, "fi"), ["0.jpg"]);
    assert_eq!(found(&catalog, "μm"), ["1.jpg"]);
    assert_eq!(found(&catalog, "ΜM"), ["1.jpg"]);
    assert_eq!(found(&catalog, "messe"), ["2.jpg"]);
    assert_eq!(smart(&mut catalog, SmartOp::StartsWith, "FIE"), ["0.jpg"]);
    assert_eq!(smart(&mut catalog, SmartOp::Is, "MESSE"), ["2.jpg"]);
}

#[test]
fn repeated_words_count_once_and_only_the_first_distinct_words_are_used() {
    let words: Vec<String> = (0..Filter::MAX_WORDS)
        .map(|index| format!("w{index}x"))
        .collect();
    let caption = words.join(" ");
    let mut photo = raster("/t/long.jpg");
    photo.caption = caption.clone();
    let mut catalog = memory();
    import(&mut catalog, &[photo, raster("/t/other.jpg")]);

    assert_eq!(found(&catalog, &caption), ["long.jpg"]);
    // One word more than is used: it is ignored, so the photo is still found.
    assert_eq!(found(&catalog, &format!("{caption} absent")), ["long.jpg"]);
    // But a word that does not match among the ones that are used excludes it.
    assert!(found(&catalog, &format!("absent {caption}")).is_empty());
    // Repeats do not use up the allowance.
    let repeated = format!("{} {caption}", "w0x ".repeat(500));
    assert_eq!(found(&catalog, &repeated), ["long.jpg"]);
}

#[test]
fn a_very_long_text_is_a_query_like_any_other() {
    let catalog = titled(&["anything"]);
    let essay: String = (0..5000).map(|index| format!("word{index} ")).collect();
    assert!(found(&catalog, &essay).is_empty());
    let count = catalog
        .count_matching(
            &Source::AllPhotos,
            &Filter {
                text: essay,
                ..Filter::default()
            },
        )
        .unwrap();
    assert_eq!(count, 0);
}
