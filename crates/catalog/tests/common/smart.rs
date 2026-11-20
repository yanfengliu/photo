//! The library and helpers the smart-collection tests share.

use photo_catalog::{
    Catalog, NewPhoto, PhotoQuery, SmartField, SmartOp, SmartRule, SmartRules, SmartValue, Source,
};
use photo_core::develop::DevelopSettings;
use photo_core::meta::{ColorLabel, Flag, PhotoId, Rating};

use super::{date, import, memory, names, raster, raw};

/// Five photos, named a to e, that differ in every field a rule can test.
pub fn library() -> (Catalog, Vec<PhotoId>) {
    let shot = |mut photo: NewPhoto, day: Option<u8>, iso, aperture, focal| {
        photo.metadata.capture_time = day.map(|day| date(2024, 3, day));
        photo.metadata.iso = iso;
        photo.metadata.aperture = aperture;
        photo.metadata.focal_length_mm = focal;
        photo
    };
    let portraits = |name: &str| raw(&format!("/smart/portraits/{name}"));
    let landscapes = |name: &str| raster(&format!("/smart/Landscapes/{name}"));
    let mut photos = vec![
        shot(
            portraits("a.arw"),
            Some(10),
            Some(100),
            Some(1.8),
            Some(85.0),
        ),
        shot(
            portraits("b.arw"),
            Some(11),
            Some(400),
            Some(2.8),
            Some(35.0),
        ),
        shot(
            landscapes("c.jpg"),
            Some(12),
            Some(1600),
            Some(8.0),
            Some(16.0),
        ),
        shot(
            landscapes("d.jpg"),
            Some(12),
            Some(6400),
            Some(11.0),
            Some(200.0),
        ),
        shot(raster("/smart/scans/e.png"), None, None, None, None),
    ];
    photos[0].rating = Rating::new(5);
    photos[0].flag = Flag::Picked;
    photos[0].label = ColorLabel::Red;
    photos[0].title = "Anna in the Garden".into();
    photos[0].creator = "Élodie Martin".into();
    photos[0].keywords = vec!["portrait".into(), "garden".into()];
    photos[0].metadata.camera_model = Some("Z8".into());
    photos[0].metadata.lens = Some("NIKKOR Z 85mm f/1.8 S".into());
    photos[1].rating = Rating::new(3);
    photos[1].flag = Flag::Picked;
    photos[1].caption = "Street portrait, 50% crop".into();
    photos[1].copyright = "CC BY".into();
    photos[1].keywords = vec!["portrait".into(), "Street".into()];
    photos[1].metadata.camera_model = Some("Z8".into());
    photos[1].metadata.lens = Some("NIKKOR Z 35mm f/1.8 S".into());
    photos[2].rating = Rating::new(3);
    photos[2].label = ColorLabel::Green;
    photos[2].title = "garden_path".into();
    photos[2].keywords = vec!["landscape".into()];
    photos[2].metadata.camera_model = Some("X-T5".into());
    photos[2].develop = Some(edited());
    photos[3].flag = Flag::Rejected;
    photos[3].label = ColorLabel::Red;
    photos[3].metadata.camera_model = Some("X-T5".into());

    let mut catalog = memory();
    let ids = import(&mut catalog, &photos);
    (catalog, ids)
}

fn edited() -> DevelopSettings {
    let mut settings = DevelopSettings::default();
    settings.tone.exposure = 0.7;
    settings
}

pub fn rule(field: SmartField, op: SmartOp, value: impl Into<SmartValue>) -> SmartRule {
    SmartRule {
        field,
        op,
        value: value.into(),
    }
}

/// The photos a smart collection with these rules holds, as the letters of
/// their file names in alphabetical order. Also checks that the stored
/// rules, the count and the listed count all agree with the query.
pub fn matched(catalog: &mut Catalog, match_all: bool, rules: Vec<SmartRule>) -> String {
    let rules = SmartRules { match_all, rules };
    let id = catalog
        .create_smart_collection("probe", None, &rules)
        .unwrap();
    assert_eq!(catalog.smart_rules(id).unwrap(), rules);
    let source = Source::Collection(id);
    let found = names(catalog, &PhotoQuery::from(source));
    assert_eq!(catalog.count(&source).unwrap() as usize, found.len());
    let listed = catalog.collections().unwrap();
    let listed = listed
        .iter()
        .find(|collection| collection.id == id)
        .unwrap();
    assert_eq!(listed.photo_count as usize, found.len());
    catalog.delete_collection(id).unwrap();
    let mut letters: Vec<&str> = found.iter().map(|name| &name[..1]).collect();
    letters.sort_unstable();
    letters.concat()
}

/// [`matched`] for a single rule.
pub fn one(
    catalog: &mut Catalog,
    field: SmartField,
    op: SmartOp,
    value: impl Into<SmartValue>,
) -> String {
    matched(catalog, true, vec![rule(field, op, value)])
}

/// Check one field against a table of `(operator, value, expected photos)`.
pub fn check<V: Into<SmartValue> + Clone>(
    catalog: &mut Catalog,
    field: SmartField,
    cases: &[(SmartOp, V, &str)],
) {
    for (op, value, expected) in cases {
        let found = one(catalog, field, *op, value.clone());
        assert_eq!(found, *expected, "{field:?} {op:?}");
    }
}
