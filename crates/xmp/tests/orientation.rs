//! `tiff:Orientation` is relative to the pixels as stored in the file; the
//! develop settings' orientation is relative to the upright picture.

use photo_core::develop::{DevelopSettings, Orientation};
use photo_core::meta::FileKind;
use photo_xmp::{ImageInfo, XmpData, parse, to_string};

const RASTER: ImageInfo = ImageInfo::new(FileKind::Raster);

fn all_orientations() -> impl Iterator<Item = Orientation> {
    (0..4).flat_map(|turns| [false, true].map(|flip| Orientation::new(turns, flip)))
}

fn turned(orientation: Orientation) -> DevelopSettings {
    DevelopSettings {
        orientation,
        ..DevelopSettings::default_for(FileKind::Raster)
    }
}

fn data(settings: &DevelopSettings) -> XmpData {
    XmpData {
        develop: Some(settings.clone()),
        ..XmpData::default()
    }
}

/// Writes `settings` and returns the XML without the private settings, so
/// that only the standard properties are left to read.
fn mirror(settings: &DevelopSettings, image: ImageInfo) -> String {
    let xml = to_string(&data(settings), image, None).unwrap();
    let start = xml.find("<photo:Settings>").unwrap();
    let end = xml.find("</photo:Settings>").unwrap() + "</photo:Settings>".len();
    format!("{}{}", &xml[..start], &xml[end..])
}

fn read(xml: &str, image: ImageInfo) -> DevelopSettings {
    parse(xml, image).unwrap().develop.unwrap()
}

/// A packet holding `tiff:Orientation` and `more` attributes.
fn packet(code: &str, more: &str) -> String {
    format!(
        r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:tiff="http://ns.adobe.com/tiff/1.0/" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" tiff:Orientation="{code}" {more}/></rdf:RDF>"#
    )
}

/// A packet holding `tiff:Orientation` and one develop property.
fn developed(code: &str) -> String {
    packet(code, r#"crs:Contrast2012="+1""#)
}

#[test]
fn an_unrotated_photo_is_written_with_its_files_own_orientation() {
    let codes = [
        (Orientation::new(0, false), 1),
        (Orientation::new(0, true), 2),
        (Orientation::new(2, false), 3),
        (Orientation::new(2, true), 4),
        (Orientation::new(3, true), 5),
        (Orientation::new(1, false), 6),
        (Orientation::new(1, true), 7),
        (Orientation::new(3, false), 8),
    ];
    for (file, code) in codes {
        let xml = mirror(
            &turned(Orientation::IDENTITY),
            RASTER.with_orientation(file),
        );
        assert!(
            xml.contains(&format!(r#"tiff:Orientation="{code}""#)),
            "{file:?}\n{xml}"
        );
    }
}

#[test]
fn a_sidecar_orientation_is_read_relative_to_the_files_own() {
    let clockwise = Orientation::new(1, false);
    let portrait = RASTER.with_orientation(clockwise);
    let orientation = |code: &str| read(&developed(code), portrait).orientation;
    // The sidecar repeats what the camera recorded: nothing was rotated.
    assert_eq!(orientation("6"), Orientation::IDENTITY);
    // Another application turned the upright picture upside down,
    assert_eq!(orientation("8"), Orientation::new(2, false));
    // or a quarter turn clockwise,
    assert_eq!(orientation("3"), clockwise);
    // or mirrored it left to right, or top to bottom.
    assert_eq!(orientation("5"), Orientation::new(0, true));
    assert_eq!(orientation("7"), Orientation::IDENTITY.flipped_vertical());
    // A value outside 1 to 8 says nothing.
    assert_eq!(orientation("0"), Orientation::IDENTITY);
    assert_eq!(orientation("sideways"), Orientation::IDENTITY);
}

#[test]
fn rotating_composes_with_the_files_own_orientation_in_both_directions() {
    for file in all_orientations() {
        for user in all_orientations() {
            let image = RASTER.with_orientation(file);
            let wanted = turned(user);
            assert_eq!(
                read(&mirror(&wanted, image), image),
                wanted,
                "{file:?} {user:?}"
            );
        }
    }
}

#[test]
fn a_rotation_made_here_replaces_the_orientation_another_application_wrote() {
    let image = RASTER.with_orientation(Orientation::new(1, false));
    let rotated = turned(Orientation::new(1, false));
    let updated = to_string(&data(&rotated), image, Some(&developed("6"))).unwrap();
    assert!(updated.contains(r#"tiff:Orientation="3""#), "{updated}");
    assert_eq!(parse(&updated, image).unwrap().develop, Some(rotated));

    // One that already says the right thing is left as it was written.
    let spaced = developed(" 6 ");
    let unrotated = read(&spaced, image);
    let updated = to_string(&data(&unrotated), image, Some(&spaced)).unwrap();
    assert!(updated.contains(r#"tiff:Orientation=" 6 ""#), "{updated}");
}

#[test]
fn without_the_files_orientation_the_sidecar_orientation_is_left_alone() {
    let rotated = turned(Orientation::new(1, false));
    let xml = mirror(&rotated, RASTER);
    assert!(!xml.contains("tiff:Orientation"), "{xml}");
    assert_eq!(read(&xml, RASTER).orientation, Orientation::IDENTITY);
    assert_eq!(
        read(&developed("6"), RASTER).orientation,
        Orientation::IDENTITY
    );

    // The private settings still carry the rotation.
    let xml = to_string(&data(&rotated), RASTER, None).unwrap();
    assert_eq!(parse(&xml, RASTER).unwrap(), data(&rotated));

    // An orientation another application wrote survives an update untouched.
    let updated = to_string(&data(&rotated), RASTER, Some(&developed("8"))).unwrap();
    assert!(updated.contains(r#"tiff:Orientation="8""#), "{updated}");
}

#[test]
fn a_bare_orientation_that_matches_the_file_is_not_a_develop_setting() {
    let portrait = RASTER.with_orientation(Orientation::new(1, false));
    assert_eq!(parse(&packet("6", ""), portrait).unwrap().develop, None);
    assert_eq!(parse(&packet("6", ""), RASTER).unwrap().develop, None);
    let turned_once = parse(&packet("3", ""), portrait).unwrap().develop;
    assert_eq!(turned_once, Some(turned(Orientation::new(1, false))));
}
