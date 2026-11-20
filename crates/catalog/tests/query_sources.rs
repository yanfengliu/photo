//! Sources: where a query looks, and the counts and facets the panels show.

mod common;

use common::{date, import, import_paths, memory, names, raster};
use photo_catalog::{
    Catalog, CatalogError, Entity, Facet, FacetValue, Filter, NewPhoto, PhotoQuery, Source,
};
use photo_core::meta::{CollectionId, FolderId, PhotoId};

fn folder_id(catalog: &Catalog, path: &str) -> FolderId {
    let folders = catalog.folders().unwrap();
    folders
        .into_iter()
        .find(|folder| folder.path.to_str() == Some(path))
        .unwrap()
        .id
}

fn in_folder(catalog: &Catalog, path: &str, include_subfolders: bool) -> Vec<String> {
    let source = Source::Folder {
        id: folder_id(catalog, path),
        include_subfolders,
    };
    let found = names(catalog, &PhotoQuery::from(source));
    assert_eq!(catalog.count(&source).unwrap() as usize, found.len());
    found
}

#[test]
fn all_photos_is_everything_and_counts_agree_with_queries() {
    let mut catalog = memory();
    assert_eq!(catalog.count(&Source::AllPhotos).unwrap(), 0);
    assert!(catalog.query(&PhotoQuery::default()).unwrap().is_empty());

    let ids = import_paths(&mut catalog, &["/a/1.jpg", "/a/2.jpg", "/b/3.jpg"]);
    catalog.create_virtual_copy(ids[0]).unwrap();
    assert_eq!(catalog.count(&Source::AllPhotos).unwrap(), 4);
    assert_eq!(
        names(&catalog, &PhotoQuery::default()),
        ["1.jpg", "1.jpg", "2.jpg", "3.jpg"]
    );
}

#[test]
fn a_folder_shows_its_own_photos_and_optionally_those_of_its_subfolders() {
    let mut catalog = memory();
    import_paths(
        &mut catalog,
        &[
            "/lib/top.jpg",
            "/lib/2024/jan.jpg",
            "/lib/2024/trip/day1.jpg",
            "/lib/2025/feb.jpg",
            "/elsewhere/x.jpg",
        ],
    );
    assert_eq!(in_folder(&catalog, "/lib", false), ["top.jpg"]);
    assert_eq!(
        in_folder(&catalog, "/lib", true),
        ["day1.jpg", "feb.jpg", "jan.jpg", "top.jpg"]
    );
    assert_eq!(in_folder(&catalog, "/lib/2024", false), ["jan.jpg"]);
    assert_eq!(
        in_folder(&catalog, "/lib/2024", true),
        ["day1.jpg", "jan.jpg"]
    );
    assert_eq!(in_folder(&catalog, "/lib/2024/trip", true), ["day1.jpg"]);
}

#[test]
fn a_sibling_folder_sharing_a_prefix_is_not_a_subfolder() {
    let mut catalog = memory();
    import_paths(
        &mut catalog,
        &[
            "/a/b/inside.jpg",
            "/a/b/c/deeper.jpg",
            "/a/bc/sibling.jpg",
            "/a/b c/spaced.jpg",
            "/a/b.d/dotted.jpg",
        ],
    );
    assert_eq!(
        in_folder(&catalog, "/a/b", true),
        ["deeper.jpg", "inside.jpg"]
    );
    assert_eq!(in_folder(&catalog, "/a/bc", true), ["sibling.jpg"]);
}

#[test]
fn wildcard_characters_in_folder_names_are_ordinary_characters() {
    let mut catalog = memory();
    import_paths(
        &mut catalog,
        &[
            "/w/100%/a.jpg",
            "/w/100%/sub/b.jpg",
            "/w/100x/c.jpg",
            "/w/100x/sub/d.jpg",
            "/w/a_b/e.jpg",
            "/w/a_b/sub/f.jpg",
            "/w/axb/g.jpg",
            "/w/axb/sub/h.jpg",
            "/w/[x]/i.jpg",
            "/w/[x]/sub/j.jpg",
        ],
    );
    assert_eq!(in_folder(&catalog, "/w/100%", true), ["a.jpg", "b.jpg"]);
    assert_eq!(in_folder(&catalog, "/w/a_b", true), ["e.jpg", "f.jpg"]);
    assert_eq!(in_folder(&catalog, "/w/[x]", true), ["i.jpg", "j.jpg"]);
    assert_eq!(in_folder(&catalog, "/w/axb", true), ["g.jpg", "h.jpg"]);
}

#[test]
fn folder_names_are_case_sensitive_and_may_hold_any_unicode() {
    let mut catalog = memory();
    import_paths(
        &mut catalog,
        &[
            "/u/Été/a.jpg",
            "/u/Été/日本/b.jpg",
            "/u/été/c.jpg",
            "/u/ÉTÉ/d.jpg",
        ],
    );
    assert_eq!(in_folder(&catalog, "/u/Été", true), ["a.jpg", "b.jpg"]);
    assert_eq!(in_folder(&catalog, "/u/Été/日本", true), ["b.jpg"]);
}

#[test]
fn the_root_folder_contains_every_other_folder() {
    let mut catalog = memory();
    import_paths(&mut catalog, &["/root.jpg", "/a/one.jpg", "/a/b/two.jpg"]);
    assert_eq!(in_folder(&catalog, "/", false), ["root.jpg"]);
    assert_eq!(
        in_folder(&catalog, "/", true),
        ["one.jpg", "root.jpg", "two.jpg"]
    );
}

#[test]
fn windows_paths_work_with_either_separator() {
    let mut catalog = memory();
    import_paths(
        &mut catalog,
        &[
            "C:\\Photos\\top.jpg",
            "C:\\Photos\\2024\\jan.jpg",
            "C:\\Photos\\2024\\trip\\day1.jpg",
            "C:\\Photos/mixed/slash.jpg",
            "C:\\PhotosBackup\\sibling.jpg",
            "C:\\root.jpg",
            "D:\\Photos\\other-drive.jpg",
            "\\\\nas\\share\\Photos\\net.jpg",
        ],
    );
    let record = catalog
        .photo(
            catalog
                .find_by_path("C:\\Photos\\2024\\jan.jpg")
                .unwrap()
                .unwrap(),
        )
        .unwrap();
    assert_eq!(record.file_name, "jan.jpg");

    assert_eq!(in_folder(&catalog, "C:\\Photos", false), ["top.jpg"]);
    assert_eq!(
        in_folder(&catalog, "C:\\Photos", true),
        ["day1.jpg", "jan.jpg", "slash.jpg", "top.jpg"]
    );
    assert_eq!(
        in_folder(&catalog, "C:\\Photos\\2024", true),
        ["day1.jpg", "jan.jpg"]
    );
    assert_eq!(
        in_folder(&catalog, "C:\\PhotosBackup", true),
        ["sibling.jpg"]
    );
    assert_eq!(
        in_folder(&catalog, "C:\\", true),
        [
            "day1.jpg",
            "jan.jpg",
            "root.jpg",
            "sibling.jpg",
            "slash.jpg",
            "top.jpg"
        ]
    );
    assert_eq!(in_folder(&catalog, "C:\\", false), ["root.jpg"]);
    assert_eq!(
        in_folder(&catalog, "\\\\nas\\share\\Photos", true),
        ["net.jpg"]
    );
}

#[test]
fn folder_counts_include_virtual_copies_and_follow_relinking() {
    let mut catalog = memory();
    let ids = import_paths(
        &mut catalog,
        &["/f/one/a.jpg", "/f/one/b.jpg", "/f/two/c.jpg"],
    );
    catalog.create_virtual_copy(ids[0]).unwrap();
    let counts = |catalog: &Catalog| -> Vec<(String, u64)> {
        let folders = catalog.folders().unwrap();
        folders
            .into_iter()
            .map(|f| (f.path.to_string_lossy().into_owned(), f.photo_count))
            .collect()
    };
    assert_eq!(
        counts(&catalog),
        [("/f/one".to_string(), 3), ("/f/two".to_string(), 1)]
    );

    catalog
        .relink_photo(ids[0], std::path::Path::new("/f/three/a.jpg"))
        .unwrap();
    assert_eq!(
        counts(&catalog),
        [
            ("/f/one".to_string(), 1),
            ("/f/three".to_string(), 2),
            ("/f/two".to_string(), 1)
        ]
    );
}

#[test]
fn previous_import_is_the_newest_run_that_added_photos() {
    let mut catalog = memory();
    let previous = |catalog: &Catalog| names(catalog, &PhotoQuery::from(Source::PreviousImport));
    assert_eq!(catalog.count(&Source::PreviousImport).unwrap(), 0);

    let first = import_paths(&mut catalog, &["/i/a.jpg", "/i/b.jpg"]);
    assert_eq!(previous(&catalog), ["a.jpg", "b.jpg"]);
    let second = import_paths(&mut catalog, &["/i/c.jpg"]);
    assert_eq!(previous(&catalog), ["c.jpg"]);

    // A virtual copy belongs to the import of its master.
    catalog.create_virtual_copy(first[0]).unwrap();
    assert_eq!(catalog.count(&Source::PreviousImport).unwrap(), 1);

    // A run that adds nothing (here: its only file was already catalogued)
    // does not hide the last real import.
    let empty = catalog.begin_import().unwrap();
    assert!(catalog.insert_photos(empty, &[raster("/i/c.jpg")]).is_err());
    assert_eq!(previous(&catalog), ["c.jpg"]);
    assert_eq!(catalog.latest_import().unwrap().unwrap().photo_count, 1);

    // Nor does a run whose photos have all been removed again.
    catalog.remove_photos(&second).unwrap();
    assert_eq!(previous(&catalog), ["a.jpg", "a.jpg", "b.jpg"]);
}

#[test]
fn either_separator_names_the_same_windows_file_and_folder() {
    use std::path::{Path, PathBuf};

    let mut catalog = memory();
    let ids = import_paths(
        &mut catalog,
        &["C:/Photos/top.jpg", "C:\\Photos\\2024/jan.jpg"],
    );
    let folders: Vec<PathBuf> = catalog
        .folders()
        .unwrap()
        .into_iter()
        .map(|f| f.path)
        .collect();
    assert_eq!(
        folders,
        [
            PathBuf::from("C:\\Photos"),
            PathBuf::from("C:\\Photos\\2024")
        ]
    );
    assert_eq!(
        catalog.photo(ids[0]).unwrap().path,
        PathBuf::from("C:\\Photos\\top.jpg")
    );
    assert_eq!(
        catalog.find_by_path("C:\\Photos\\top.jpg").unwrap(),
        Some(ids[0])
    );
    assert_eq!(
        catalog.find_by_path("C:/Photos/2024/jan.jpg").unwrap(),
        Some(ids[1])
    );
    assert_eq!(
        in_folder(&catalog, "C:\\Photos", true),
        ["jan.jpg", "top.jpg"]
    );

    // The other spelling is the same file, not a second photo.
    let run = catalog.begin_import().unwrap();
    for spelling in [
        "C:\\Photos\\top.jpg",
        "C:/Photos/top.jpg",
        "C:\\Photos/top.jpg",
    ] {
        assert!(matches!(
            catalog.insert_photo(run, &raster(spelling)),
            Err(CatalogError::DuplicatePath(path)) if path == "C:\\Photos\\top.jpg"
        ));
    }
    assert_eq!(
        catalog.ensure_folder(Path::new("C:/Photos/")).unwrap(),
        folder_id(&catalog, "C:\\Photos")
    );
    catalog
        .relink_photo(ids[1], Path::new("C:/Photos/2025/jan.jpg"))
        .unwrap();
    assert_eq!(
        catalog.photo(ids[1]).unwrap().path,
        PathBuf::from("C:\\Photos\\2025\\jan.jpg")
    );
    assert_eq!(in_folder(&catalog, "C:\\Photos\\2025", false), ["jan.jpg"]);

    // A path that is not Windows-shaped is never rewritten: there a
    // backslash is part of a name.
    let odd = import_paths(&mut catalog, &["/photos/odd\\name/a.jpg"]);
    assert_eq!(
        catalog.photo(odd[0]).unwrap().path,
        PathBuf::from("/photos/odd\\name/a.jpg")
    );
    assert_eq!(in_folder(&catalog, "/photos/odd\\name", true), ["a.jpg"]);
}

#[test]
fn sources_that_do_not_exist_are_typed_errors_not_empty_results() {
    let catalog = memory();
    let missing_folder = Source::Folder {
        id: FolderId(404),
        include_subfolders: true,
    };
    assert!(matches!(
        catalog.query(&PhotoQuery::from(missing_folder)),
        Err(CatalogError::NotFound {
            entity: Entity::Folder,
            id: 404
        })
    ));
    assert!(matches!(
        catalog.count(&Source::Collection(CollectionId(404))),
        Err(CatalogError::NotFound {
            entity: Entity::Collection,
            id: 404
        })
    ));
}

fn facet(catalog: &Catalog, facet: Facet, filter: &Filter) -> Vec<(String, u64)> {
    let values: Vec<FacetValue> = catalog
        .facet_values(&Source::AllPhotos, filter, facet)
        .unwrap();
    values
        .into_iter()
        .map(|value| (value.value, value.count))
        .collect()
}

fn pairs(expected: &[(&str, u64)]) -> Vec<(String, u64)> {
    expected
        .iter()
        .map(|(value, count)| (value.to_string(), *count))
        .collect()
}

#[test]
fn facets_list_distinct_values_with_counts_for_the_photos_in_view() {
    let shot = |path: &str,
                camera: Option<&str>,
                lens: Option<&str>,
                iso: Option<u32>,
                day: Option<u8>| {
        let mut photo: NewPhoto = raster(path);
        photo.metadata.camera_model = camera.map(str::to_string);
        photo.metadata.lens = lens.map(str::to_string);
        photo.metadata.iso = iso;
        photo.metadata.capture_time = day.map(|day| date(2024, 7, day));
        photo
    };
    let mut catalog = memory();
    let ids: Vec<PhotoId> = import(
        &mut catalog,
        &[
            shot("/x/1.jpg", Some("Z8"), Some("50mm"), Some(100), Some(1)),
            shot("/x/2.jpg", Some("Z8"), Some("85mm"), Some(3200), Some(1)),
            shot("/x/3.jpg", Some("X100V"), None, Some(400), Some(2)),
            shot("/y/4.jpg", None, None, None, None),
            shot("/y/5.jpg", Some("Z8"), Some("50mm"), Some(400), Some(3)),
        ],
    );
    let all = Filter::default();
    assert_eq!(
        facet(&catalog, Facet::Camera, &all),
        pairs(&[("", 1), ("X100V", 1), ("Z8", 3)])
    );
    assert_eq!(
        facet(&catalog, Facet::Lens, &all),
        pairs(&[("", 2), ("50mm", 2), ("85mm", 1)])
    );
    assert_eq!(
        facet(&catalog, Facet::Iso, &all),
        pairs(&[("", 1), ("100", 1), ("400", 2), ("3200", 1)])
    );
    assert_eq!(
        facet(&catalog, Facet::CaptureDate, &all),
        pairs(&[
            ("", 1),
            ("2024-07-01", 2),
            ("2024-07-02", 1),
            ("2024-07-03", 1)
        ])
    );

    // Facets describe the photos the source and filter leave in view.
    let z8 = Filter {
        cameras: vec!["Z8".into()],
        ..Filter::default()
    };
    assert_eq!(
        facet(&catalog, Facet::Lens, &z8),
        pairs(&[("50mm", 2), ("85mm", 1)])
    );
    let folder = Source::Folder {
        id: catalog.photo(ids[3]).unwrap().folder_id,
        include_subfolders: false,
    };
    let in_y = catalog.facet_values(&folder, &all, Facet::Camera).unwrap();
    assert_eq!(in_y.iter().map(|value| value.count).sum::<u64>(), 2);
}
