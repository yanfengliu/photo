//! Every filter field, alone and combined.

mod common;

use common::{date, import, memory, names, raster, raw};
use photo_catalog::{
    Catalog, Comparison, CopyFilter, DateRange, Filter, IsoRange, NewPhoto, PhotoQuery,
    RatingFilter, Sort, SortDirection, SortKey, Source,
};
use photo_core::develop::DevelopSettings;
use photo_core::meta::{ColorLabel, FileKind, Flag, PhotoId, Rating};

/// Six photos that differ in every filterable way.
fn library() -> (Catalog, Vec<PhotoId>) {
    let shot = |mut photo: NewPhoto,
                day: Option<u8>,
                camera: Option<(&str, &str)>,
                lens: Option<&str>,
                iso| {
        photo.metadata.capture_time = day.map(|day| date(2024, 6, day));
        photo.metadata.camera_make = camera.map(|(make, _)| make.to_string());
        photo.metadata.camera_model = camera.map(|(_, model)| model.to_string());
        photo.metadata.lens = lens.map(str::to_string);
        photo.metadata.iso = iso;
        photo
    };
    let sony = Some(("SONY", "ILCE-7M4"));
    let fuji = Some(("FUJIFILM", "X-T5"));
    let mut photos = vec![
        shot(
            raw("/lib/beach/DSC_0001.ARW"),
            Some(1),
            sony,
            Some("FE 35mm F1.8"),
            Some(100),
        ),
        shot(
            raw("/lib/beach/DSC_0002.ARW"),
            Some(2),
            sony,
            Some("FE 85mm F1.8"),
            Some(400),
        ),
        shot(
            raster("/lib/beach/sunset.jpg"),
            Some(3),
            fuji,
            Some("XF 23mm F2"),
            Some(1600),
        ),
        shot(
            raster("/lib/city/skyline.jpg"),
            Some(4),
            fuji,
            None,
            Some(6400),
        ),
        shot(raster("/lib/city/scan_01.png"), None, None, None, None),
        shot(
            raw("/lib/city/DSC_0003.ARW"),
            Some(6),
            sony,
            Some("FE 35mm F1.8"),
            Some(12_800),
        ),
    ];
    photos[0].rating = Rating::new(5);
    photos[0].flag = Flag::Picked;
    photos[0].label = ColorLabel::Red;
    photos[0].title = "Morning Tide".into();
    photos[0].keywords = vec!["ocean".into(), "Été".into()];
    photos[1].rating = Rating::new(3);
    photos[1].flag = Flag::Picked;
    photos[1].label = ColorLabel::Green;
    photos[1].caption = "Portrait of a surfer at the tide line".into();
    photos[2].rating = Rating::new(3);
    photos[2].label = ColorLabel::Red;
    photos[2].keywords = vec!["ocean".into(), "sunset".into()];
    photos[3].rating = Rating::new(1);
    photos[3].flag = Flag::Rejected;
    photos[3].develop = Some(edited());
    photos[4].title = "100% wool_blend".into();
    photos[5].develop = Some(DevelopSettings::default_for(FileKind::Raw));

    let mut catalog = memory();
    let ids = import(&mut catalog, &photos);
    (catalog, ids)
}

fn edited() -> DevelopSettings {
    let mut settings = DevelopSettings::default();
    settings.tone.contrast = 20.0;
    settings
}

/// File names matching `filter` across the whole catalog, in file-name order.
fn matching(catalog: &Catalog, filter: Filter) -> Vec<String> {
    let query = PhotoQuery {
        source: Source::AllPhotos,
        filter,
        sort: Sort {
            key: SortKey::FileName,
            direction: SortDirection::Ascending,
        },
    };
    let found = names(catalog, &query);
    assert_eq!(
        catalog
            .count_matching(&query.source, &query.filter)
            .unwrap() as usize,
        found.len(),
        "the count agrees with the query"
    );
    assert_eq!(catalog.query_ids(&query).unwrap().len(), found.len());
    found
}

const ALL: [&str; 6] = [
    "DSC_0001.ARW",
    "DSC_0002.ARW",
    "DSC_0003.ARW",
    "scan_01.png",
    "skyline.jpg",
    "sunset.jpg",
];

#[test]
fn an_empty_filter_matches_everything() {
    let (catalog, _) = library();
    assert_eq!(matching(&catalog, Filter::default()), ALL);
    assert!(Filter::default().is_empty());
}

#[test]
fn text_matches_file_name_title_caption_keywords_camera_and_lens() {
    let (catalog, _) = library();
    let text = |text: &str| {
        matching(
            &catalog,
            Filter {
                text: text.into(),
                ..Filter::default()
            },
        )
    };
    assert_eq!(text("skyline"), ["skyline.jpg"], "file name");
    assert_eq!(text("morning"), ["DSC_0001.ARW"], "title, ignoring case");
    assert_eq!(text("surfer"), ["DSC_0002.ARW"], "caption");
    assert_eq!(text("ocean"), ["DSC_0001.ARW", "sunset.jpg"], "keyword");
    assert_eq!(text("fujifilm"), ["skyline.jpg", "sunset.jpg"], "camera");
    assert_eq!(text("85mm"), ["DSC_0002.ARW"], "lens");
    assert_eq!(
        text("ÉTÉ"),
        ["DSC_0001.ARW"],
        "case folding is not limited to ASCII"
    );
    assert!(text("nothing-like-this").is_empty());
}

#[test]
fn every_word_of_the_text_must_match_but_each_may_match_a_different_field() {
    let (catalog, _) = library();
    let text = |text: &str| {
        matching(
            &catalog,
            Filter {
                text: text.into(),
                ..Filter::default()
            },
        )
    };
    assert_eq!(text("tide"), ["DSC_0001.ARW", "DSC_0002.ARW"]);
    assert_eq!(text("  tide   ocean "), ["DSC_0001.ARW"]);
    assert_eq!(text("sony 35mm"), ["DSC_0001.ARW", "DSC_0003.ARW"]);
    assert!(text("sony fujifilm").is_empty());
    assert_eq!(text("   "), ALL, "white space alone is no filter");
}

#[test]
fn text_is_matched_literally_never_as_a_pattern() {
    let (catalog, _) = library();
    let text = |text: &str| {
        matching(
            &catalog,
            Filter {
                text: text.into(),
                ..Filter::default()
            },
        )
    };
    assert_eq!(text("100%"), ["scan_01.png"]);
    assert_eq!(text("wool_blend"), ["scan_01.png"]);
    assert!(text("%").len() == 1, "a percent sign is not a wildcard");
    assert_eq!(
        text("_"),
        [
            "DSC_0001.ARW",
            "DSC_0002.ARW",
            "DSC_0003.ARW",
            "scan_01.png"
        ]
    );
    assert!(
        text("DSC_000_").is_empty(),
        "an underscore is not a wildcard"
    );
    assert!(text("') OR 1=1 --").is_empty());
}

#[test]
fn flags_match_any_of_the_listed_states() {
    let (catalog, _) = library();
    let flags = |flags: &[Flag]| {
        matching(
            &catalog,
            Filter {
                flags: flags.to_vec(),
                ..Filter::default()
            },
        )
    };
    assert_eq!(flags(&[Flag::Picked]), ["DSC_0001.ARW", "DSC_0002.ARW"]);
    assert_eq!(flags(&[Flag::Rejected]), ["skyline.jpg"]);
    assert_eq!(
        flags(&[Flag::Unflagged]),
        ["DSC_0003.ARW", "scan_01.png", "sunset.jpg"]
    );
    assert_eq!(
        flags(&[Flag::Picked, Flag::Rejected]),
        ["DSC_0001.ARW", "DSC_0002.ARW", "skyline.jpg"]
    );
}

#[test]
fn ratings_compare_at_least_at_most_and_exactly() {
    let (catalog, _) = library();
    let rated = |comparison, stars| {
        matching(
            &catalog,
            Filter {
                rating: Some(RatingFilter {
                    comparison,
                    rating: Rating::new(stars),
                }),
                ..Filter::default()
            },
        )
    };
    assert_eq!(
        rated(Comparison::AtLeast, 3),
        ["DSC_0001.ARW", "DSC_0002.ARW", "sunset.jpg"]
    );
    assert_eq!(
        rated(Comparison::AtMost, 1),
        ["DSC_0003.ARW", "scan_01.png", "skyline.jpg"]
    );
    assert_eq!(
        rated(Comparison::Exactly, 3),
        ["DSC_0002.ARW", "sunset.jpg"]
    );
    assert_eq!(
        rated(Comparison::Exactly, 0),
        ["DSC_0003.ARW", "scan_01.png"]
    );
    assert_eq!(rated(Comparison::AtLeast, 0), ALL);
}

#[test]
fn labels_match_any_of_the_listed_colours_including_none() {
    let (catalog, _) = library();
    let labelled = |labels: &[ColorLabel]| {
        matching(
            &catalog,
            Filter {
                labels: labels.to_vec(),
                ..Filter::default()
            },
        )
    };
    assert_eq!(labelled(&[ColorLabel::Red]), ["DSC_0001.ARW", "sunset.jpg"]);
    assert_eq!(
        labelled(&[ColorLabel::Red, ColorLabel::Green]),
        ["DSC_0001.ARW", "DSC_0002.ARW", "sunset.jpg"]
    );
    assert_eq!(
        labelled(&[ColorLabel::None]),
        ["DSC_0003.ARW", "scan_01.png", "skyline.jpg"]
    );
    assert!(labelled(&[ColorLabel::Purple]).is_empty());
}

#[test]
fn masters_and_virtual_copies_can_be_told_apart() {
    let (mut catalog, ids) = library();
    let copy = catalog.create_virtual_copy(ids[2]).unwrap();
    let copies = |copies| {
        catalog.query(&PhotoQuery {
            filter: Filter {
                copies: Some(copies),
                ..Filter::default()
            },
            ..PhotoQuery::default()
        })
    };
    let virtual_copies = copies(CopyFilter::VirtualCopies).unwrap();
    assert_eq!(common::ids(&virtual_copies), [copy]);
    assert_eq!(virtual_copies[0].file_name, "sunset.jpg");
    assert_eq!(copies(CopyFilter::Masters).unwrap().len(), 6);
    assert_eq!(catalog.count(&Source::AllPhotos).unwrap(), 7);
}

#[test]
fn file_kind_separates_raw_from_raster() {
    let (catalog, _) = library();
    let kind = |kind| {
        matching(
            &catalog,
            Filter {
                kind: Some(kind),
                ..Filter::default()
            },
        )
    };
    assert_eq!(
        kind(FileKind::Raw),
        ["DSC_0001.ARW", "DSC_0002.ARW", "DSC_0003.ARW"]
    );
    assert_eq!(
        kind(FileKind::Raster),
        ["scan_01.png", "skyline.jpg", "sunset.jpg"]
    );
}

#[test]
fn edited_means_settings_that_differ_from_the_default() {
    let (mut catalog, ids) = library();
    let edited = |catalog: &Catalog, edited| {
        matching(
            catalog,
            Filter {
                edited: Some(edited),
                ..Filter::default()
            },
        )
    };
    // DSC_0003 carries stored settings, but they are the RAW defaults.
    assert_eq!(edited(&catalog, true), ["skyline.jpg"]);
    assert_eq!(edited(&catalog, false).len(), 5);

    catalog
        .set_develop_settings(ids[0], &self::edited(), Some("Contrast"))
        .unwrap();
    assert_eq!(edited(&catalog, true), ["DSC_0001.ARW", "skyline.jpg"]);
}

#[test]
fn cameras_and_lenses_match_exact_names_and_the_empty_name_means_not_recorded() {
    let (catalog, _) = library();
    let cameras = |cameras: &[&str]| {
        matching(
            &catalog,
            Filter {
                cameras: cameras.iter().map(|camera| camera.to_string()).collect(),
                ..Filter::default()
            },
        )
    };
    let lenses = |lenses: &[&str]| {
        matching(
            &catalog,
            Filter {
                lenses: lenses.iter().map(|lens| lens.to_string()).collect(),
                ..Filter::default()
            },
        )
    };
    assert_eq!(
        cameras(&["SONY ILCE-7M4"]),
        ["DSC_0001.ARW", "DSC_0002.ARW", "DSC_0003.ARW"]
    );
    assert_eq!(
        cameras(&["FUJIFILM X-T5", ""]),
        ["scan_01.png", "skyline.jpg", "sunset.jpg"]
    );
    assert!(cameras(&["SONY"]).is_empty(), "the whole name must match");
    assert!(cameras(&["sony ilce-7m4"]).is_empty(), "and its case");
    assert_eq!(lenses(&["FE 35mm F1.8"]), ["DSC_0001.ARW", "DSC_0003.ARW"]);
    assert_eq!(lenses(&[""]), ["scan_01.png", "skyline.jpg"]);
}

#[test]
fn capture_date_ranges_are_inclusive_and_skip_undated_photos() {
    let (catalog, _) = library();
    let between = |from: Option<u8>, to: Option<u8>| {
        matching(
            &catalog,
            Filter {
                capture: Some(DateRange {
                    from: from.map(|day| date(2024, 6, day)),
                    to: to.map(|day| date(2024, 6, day)),
                }),
                ..Filter::default()
            },
        )
    };
    assert_eq!(
        between(Some(2), Some(4)),
        ["DSC_0002.ARW", "skyline.jpg", "sunset.jpg"]
    );
    assert_eq!(between(Some(4), None), ["DSC_0003.ARW", "skyline.jpg"]);
    assert_eq!(between(None, Some(1)), ["DSC_0001.ARW"]);
    assert_eq!(
        between(None, None).len(),
        5,
        "an open range still needs a capture time"
    );
    assert!(between(Some(5), Some(5)).is_empty());
}

#[test]
fn iso_ranges_are_inclusive_and_skip_photos_without_an_iso() {
    let (catalog, _) = library();
    let iso = |min, max| {
        matching(
            &catalog,
            Filter {
                iso: Some(IsoRange { min, max }),
                ..Filter::default()
            },
        )
    };
    assert_eq!(iso(Some(400), Some(1600)), ["DSC_0002.ARW", "sunset.jpg"]);
    assert_eq!(iso(Some(6400), None), ["DSC_0003.ARW", "skyline.jpg"]);
    assert_eq!(iso(None, Some(100)), ["DSC_0001.ARW"]);
    assert_eq!(iso(None, None).len(), 5);
}

#[test]
fn filters_combine_with_and() {
    let (catalog, ids) = library();
    let combined = Filter {
        text: "sony".into(),
        flags: vec![Flag::Picked, Flag::Unflagged],
        rating: Some(RatingFilter {
            comparison: Comparison::AtMost,
            rating: Rating::new(4),
        }),
        labels: vec![ColorLabel::None, ColorLabel::Green],
        copies: Some(CopyFilter::Masters),
        kind: Some(FileKind::Raw),
        edited: Some(false),
        cameras: vec!["SONY ILCE-7M4".into()],
        lenses: vec!["FE 85mm F1.8".into(), "FE 35mm F1.8".into()],
        capture: Some(DateRange {
            from: Some(date(2024, 6, 2)),
            to: None,
        }),
        iso: Some(IsoRange {
            min: Some(200),
            max: None,
        }),
    };
    assert!(!combined.is_empty());
    assert_eq!(
        matching(&catalog, combined.clone()),
        ["DSC_0002.ARW", "DSC_0003.ARW"]
    );

    // Narrowed further by the source.
    let in_city = PhotoQuery {
        source: Source::Folder {
            id: catalog.photo(ids[5]).unwrap().folder_id,
            include_subfolders: false,
        },
        filter: combined,
        sort: Sort::default(),
    };
    assert_eq!(names(&catalog, &in_city), ["DSC_0003.ARW"]);
}
