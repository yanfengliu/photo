//! Filters in combination: a photo passes a combined filter exactly when it
//! passes every part of it, for every pair of filter fields.

mod common;

use std::collections::BTreeSet;

use common::{date, import, memory};
use photo_catalog::{
    Catalog, Comparison, CopyFilter, DateRange, Filter, IsoRange, NewPhoto, PhotoQuery,
    RatingFilter, Source,
};
use photo_core::develop::DevelopSettings;
use photo_core::meta::{ColorLabel, FileKind, Flag, PhotoId, Rating};

/// Sixty photos whose attributes cycle with different periods, so that the
/// fields vary independently of one another; every fifth is a virtual copy.
fn library() -> Catalog {
    let photo = |index: usize| {
        let kind = if index.is_multiple_of(2) {
            FileKind::Raw
        } else {
            FileKind::Raster
        };
        let mut photo = NewPhoto::new(format!("/lib/{}/shot_{index}.x", index % 4), kind);
        photo.metadata.capture_time =
            (!index.is_multiple_of(7)).then(|| date(2024, 1, 1 + (index % 20) as u8));
        photo.metadata.camera_model =
            (!index.is_multiple_of(5)).then(|| ["Alpha", "Beta", "Gamma"][index % 3].to_string());
        photo.metadata.lens =
            (!index.is_multiple_of(4)).then(|| ["Wide", "Tele"][index % 2].to_string());
        photo.metadata.iso = (!index.is_multiple_of(6)).then(|| 100 << (index % 5));
        photo.rating = Rating::new((index % 6) as u8);
        photo.flag = [Flag::Unflagged, Flag::Picked, Flag::Rejected][index % 3];
        photo.label = ColorLabel::ALL[index % 4];
        photo.title = ["dawn", "noon", "dusk", "night", "storm"][index % 5].to_string();
        if index.is_multiple_of(3) {
            let mut settings = DevelopSettings::default_for(kind);
            settings.tone.contrast = 10.0;
            photo.develop = Some(settings);
        }
        photo
    };
    let mut catalog = memory();
    let ids = import(&mut catalog, &(0..48).map(photo).collect::<Vec<_>>());
    for id in ids.iter().step_by(4) {
        catalog.create_virtual_copy(*id).unwrap();
    }
    catalog
}

/// One filter per field, each letting some photos through and not others.
fn single_field_filters() -> Vec<(&'static str, Filter)> {
    let base = Filter::default;
    vec![
        (
            "text",
            Filter {
                text: "d".into(),
                ..base()
            },
        ),
        (
            "flags",
            Filter {
                flags: vec![Flag::Picked, Flag::Rejected],
                ..base()
            },
        ),
        (
            "rating",
            Filter {
                rating: Some(RatingFilter {
                    comparison: Comparison::AtLeast,
                    rating: Rating::new(3),
                }),
                ..base()
            },
        ),
        (
            "labels",
            Filter {
                labels: vec![ColorLabel::None, ColorLabel::Yellow],
                ..base()
            },
        ),
        (
            "copies",
            Filter {
                copies: Some(CopyFilter::Masters),
                ..base()
            },
        ),
        (
            "kind",
            Filter {
                kind: Some(FileKind::Raster),
                ..base()
            },
        ),
        (
            "edited",
            Filter {
                edited: Some(true),
                ..base()
            },
        ),
        (
            "cameras",
            Filter {
                cameras: vec!["Alpha".into(), String::new()],
                ..base()
            },
        ),
        (
            "lenses",
            Filter {
                lenses: vec!["Tele".into()],
                ..base()
            },
        ),
        (
            "capture",
            Filter {
                capture: Some(DateRange {
                    from: Some(date(2024, 1, 5)),
                    to: Some(date(2024, 1, 14)),
                }),
                ..base()
            },
        ),
        (
            "iso",
            Filter {
                iso: Some(IsoRange {
                    min: Some(200),
                    max: Some(800),
                }),
                ..base()
            },
        ),
    ]
}

/// The fields `extra` sets, laid over `filter`.
fn with(mut filter: Filter, extra: &Filter) -> Filter {
    let unset = Filter::default();
    macro_rules! take {
        ($($field:ident),*) => {$(
            if extra.$field != unset.$field {
                filter.$field = extra.$field.clone();
            }
        )*};
    }
    take!(
        text, flags, rating, labels, copies, kind, edited, cameras, lenses, capture, iso
    );
    filter
}

fn matching(catalog: &Catalog, filter: &Filter) -> BTreeSet<PhotoId> {
    let query = PhotoQuery {
        filter: filter.clone(),
        ..PhotoQuery::default()
    };
    let ids: BTreeSet<PhotoId> = catalog.query_ids(&query).unwrap().into_iter().collect();
    assert_eq!(
        catalog.count_matching(&Source::AllPhotos, filter).unwrap() as usize,
        ids.len()
    );
    ids
}

#[test]
fn every_single_filter_lets_some_photos_through_and_keeps_some_out() {
    let catalog = library();
    let everything = matching(&catalog, &Filter::default());
    assert_eq!(everything.len(), 60);
    for (name, filter) in single_field_filters() {
        let passed = matching(&catalog, &filter).len();
        assert!(passed > 0 && passed < everything.len(), "{name}: {passed}");
    }
}

#[test]
fn any_two_filters_together_keep_exactly_what_both_keep() {
    let catalog = library();
    let filters = single_field_filters();
    let alone: Vec<BTreeSet<PhotoId>> = filters
        .iter()
        .map(|(_, filter)| matching(&catalog, filter))
        .collect();
    for first in 0..filters.len() {
        for second in first + 1..filters.len() {
            let both = with(filters[first].1.clone(), &filters[second].1);
            let expected: BTreeSet<PhotoId> =
                alone[first].intersection(&alone[second]).copied().collect();
            assert_eq!(
                matching(&catalog, &both),
                expected,
                "{} and {}",
                filters[first].0,
                filters[second].0
            );
        }
    }
}

/// `list` over and over, far past the number of values one statement takes.
fn repeated<T: Clone>(list: &[T]) -> Vec<T> {
    list.iter().cloned().cycle().take(40_000).collect()
}

#[test]
fn a_value_repeated_in_a_filter_list_counts_once_however_long_the_list_gets() {
    // A caller that builds a list by appending (one entry per selected
    // photo, say) may name the same value any number of times.
    let catalog = library();
    for (name, filter) in single_field_filters() {
        let mut long = filter.clone();
        long.flags = repeated(&filter.flags);
        long.labels = repeated(&filter.labels);
        long.cameras = repeated(&filter.cameras);
        long.lenses = repeated(&filter.lenses);
        assert_eq!(
            matching(&catalog, &long),
            matching(&catalog, &filter),
            "{name}"
        );
    }
}

#[test]
fn a_filter_may_name_more_cameras_and_lenses_than_one_statement_takes_values() {
    let mut catalog = library();
    // A name with quotes, a backslash and a non-ASCII letter is matched
    // exactly, like any other.
    let odd_name = "Odd \"quoted\" \\ name \u{2601}";
    let mut odd = NewPhoto::new("/lib/odd.x", FileKind::Raster);
    odd.metadata.camera_model = Some(odd_name.into());
    odd.metadata.lens = Some("Tele".into());
    let odd = import(&mut catalog, &[odd])[0];
    let short = Filter {
        cameras: vec!["Alpha".into(), String::new(), odd_name.into()],
        lenses: vec!["Tele".into()],
        ..Filter::default()
    };
    assert!(matching(&catalog, &short).contains(&odd));
    // Every name different, so there is nothing to fold together.
    let unknown =
        |what: &'static str| (0..40_000).map(move |index| format!("No such {what} {index}"));
    let long = Filter {
        cameras: short
            .cameras
            .iter()
            .cloned()
            .chain(unknown("camera"))
            .collect(),
        lenses: short
            .lenses
            .iter()
            .cloned()
            .chain(unknown("lens"))
            .collect(),
        ..Filter::default()
    };
    let expected = matching(&catalog, &short);
    assert!(!expected.is_empty());
    assert_eq!(matching(&catalog, &long), expected);
}

#[test]
fn all_filters_together_keep_exactly_what_every_one_of_them_keeps() {
    let catalog = library();
    let filters = single_field_filters();
    let mut everything = Filter::default();
    let mut expected = matching(&catalog, &Filter::default());
    for (_, filter) in &filters {
        everything = with(everything, filter);
        expected = expected
            .intersection(&matching(&catalog, filter))
            .copied()
            .collect();
        // Adding one field at a time, the result narrows step by step.
        assert_eq!(matching(&catalog, &everything), expected);
    }
    assert!(!everything.is_empty());
}
