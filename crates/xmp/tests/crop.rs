//! The crop: a sidecar gives the corners of the rotated rectangle in the
//! frame of the stored pixels, the develop settings its centre, sides and
//! angle in the upright frame.

use photo_core::develop::{Crop, CropRect, DevelopSettings, Orientation};
use photo_core::geom::Point;
use photo_core::meta::FileKind;
use photo_xmp::{ImageInfo, XmpData, parse, to_string};

const RASTER: ImageInfo = ImageInfo::new(FileKind::Raster);

const RECT: CropRect = CropRect {
    cx: 0.45,
    cy: 0.55,
    width: 0.5,
    height: 0.4,
};

fn all_orientations() -> impl Iterator<Item = Orientation> {
    (0..4).flat_map(|turns| [false, true].map(|flip| Orientation::new(turns, flip)))
}

fn cropped(crop: Crop) -> DevelopSettings {
    DevelopSettings {
        crop,
        ..DevelopSettings::default_for(FileKind::Raster)
    }
}

fn rotated(rect: CropRect, angle: f32) -> Crop {
    Crop {
        rect: Some(rect),
        angle,
        ..Crop::default()
    }
}

/// Writes `settings` over `existing` and returns the XML without the
/// private settings, so that only the Camera Raw mapping is left to read.
fn mirror_onto(settings: &DevelopSettings, image: ImageInfo, existing: Option<&str>) -> String {
    let data = XmpData {
        develop: Some(settings.clone()),
        ..XmpData::default()
    };
    let xml = to_string(&data, image, existing).unwrap();
    let start = xml.find("<photo:Settings>").unwrap();
    let end = xml.find("</photo:Settings>").unwrap() + "</photo:Settings>".len();
    format!("{}{}", &xml[..start], &xml[end..])
}

fn mirror(settings: &DevelopSettings, image: ImageInfo) -> String {
    mirror_onto(settings, image, None)
}

fn read(xml: &str, image: ImageInfo) -> Crop {
    parse(xml, image).unwrap().develop.unwrap().crop
}

/// A packet with `attributes` on its one description.
fn packet(attributes: &str) -> String {
    format!(
        r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:tiff="http://ns.adobe.com/tiff/1.0/" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" {attributes}/></rdf:RDF>"#
    )
}

/// The value of a numeric attribute of `xml`.
fn number(xml: &str, name: &str) -> f32 {
    let opening = format!("{name}=\"");
    let start = xml
        .find(&opening)
        .unwrap_or_else(|| panic!("{name} is missing from\n{xml}"))
        + opening.len();
    let end = start + xml[start..].find('"').unwrap();
    xml[start..end].parse().unwrap()
}

/// The two corners a sidecar records: top-left and bottom-right.
fn written_corners(xml: &str) -> [Point; 2] {
    [
        Point::new(number(xml, "crs:CropLeft"), number(xml, "crs:CropTop")),
        Point::new(number(xml, "crs:CropRight"), number(xml, "crs:CropBottom")),
    ]
}

fn close(a: Point, b: Point) -> bool {
    a.distance(b) < 1e-5
}

fn assert_corners(xml: &str, rect: CropRect, angle: f32, aspect: f32) {
    let [top_left, _, bottom_right, _] = rect.corners(angle, aspect);
    let [first, second] = written_corners(xml);
    assert!(
        close(first, top_left),
        "top-left {first:?}, expected {top_left:?}"
    );
    assert!(
        close(second, bottom_right),
        "bottom-right {second:?}, expected {bottom_right:?}"
    );
}

#[test]
fn an_unrotated_crop_is_written_as_its_edges() {
    let xml = mirror(&cropped(rotated(RECT, 0.0)), RASTER);
    for expected in [r#"crs:HasCrop="True""#, r#"crs:CropAngle="0""#] {
        assert!(xml.contains(expected), "{expected} is missing from\n{xml}");
    }
    assert_corners(&xml, RECT, 0.0, 1.0);
    assert!((number(&xml, "crs:CropLeft") - 0.2).abs() < 1e-6);
    assert!((number(&xml, "crs:CropBottom") - 0.75).abs() < 1e-6);
}

#[test]
fn a_rotated_crop_is_written_as_the_positions_of_its_corners() {
    let image = RASTER.with_size(3000, 2000);
    for angle in [-30.0, -2.5, 7.5, 45.0] {
        let xml = mirror(&cropped(rotated(RECT, angle)), image);
        assert_corners(&xml, RECT, angle, 1.5);
        assert_eq!(number(&xml, "crs:CropAngle"), angle);
    }
}

#[test]
fn the_shape_of_the_frame_comes_from_the_sidecar_then_the_caller_then_an_assumption() {
    let crop = cropped(rotated(RECT, 20.0));
    assert_corners(&mirror(&crop, RASTER), RECT, 20.0, 1.5);
    assert_corners(
        &mirror(&crop, RASTER.with_size(2000, 2000)),
        RECT,
        20.0,
        1.0,
    );
    assert_corners(&mirror(&crop, RASTER.with_size(0, 2000)), RECT, 20.0, 1.5);

    let sized = packet(r#"tiff:ImageWidth="4000" tiff:ImageLength="1000""#);
    let onto = |image| mirror_onto(&crop, image, Some(&sized));
    assert_corners(&onto(RASTER.with_size(2000, 2000)), RECT, 20.0, 4.0);
    assert_corners(&onto(RASTER), RECT, 20.0, 4.0);
    assert_eq!(read(&onto(RASTER), RASTER.with_size(2000, 2000)), crop.crop);
}

#[test]
fn a_rotated_crop_from_another_application_is_read_through_its_image_size() {
    let expected = CropRect {
        cx: 0.52,
        cy: 0.47,
        width: 0.8,
        height: 0.7,
    };
    let [top_left, _, bottom_right, _] = expected.corners(-6.25, 1.5);
    let xml = packet(&format!(
        r#"tiff:ImageWidth="6000" tiff:ImageLength="4000" crs:HasCrop="True" crs:CropAngle="-6.25" crs:CropLeft="{}" crs:CropTop="{}" crs:CropRight="{}" crs:CropBottom="{}""#,
        top_left.x, top_left.y, bottom_right.x, bottom_right.y
    ));
    let crop = read(&xml, RASTER);
    assert_eq!(crop.angle, -6.25);
    let rect = crop.rect.unwrap();
    for (read, wanted) in [
        (rect.cx, expected.cx),
        (rect.cy, expected.cy),
        (rect.width, expected.width),
        (rect.height, expected.height),
    ] {
        assert!((read - wanted).abs() < 1e-5, "{rect:?}");
    }
}

#[test]
fn crop_corners_are_stored_in_the_frame_of_the_files_pixels() {
    // The left half of an upright portrait whose pixels are stored lying on
    // their side (to be turned a quarter clockwise) is the bottom half of
    // the stored frame.
    let left_half = rotated(
        CropRect {
            cx: 0.25,
            cy: 0.5,
            width: 0.5,
            height: 1.0,
        },
        0.0,
    );
    let image = RASTER.with_orientation(Orientation::new(1, false));
    let xml = mirror(&cropped(left_half), image);
    for expected in [
        r#"tiff:Orientation="6""#,
        r#"crs:HasCrop="True""#,
        r#"crs:CropLeft="0""#,
        r#"crs:CropTop="0.5""#,
        r#"crs:CropRight="1""#,
        r#"crs:CropBottom="1""#,
        r#"crs:CropAngle="0""#,
    ] {
        assert!(xml.contains(expected), "{expected} is missing from\n{xml}");
    }
    assert_eq!(read(&xml, image), left_half);

    // A sidecar that names the orientation is enough to place the crop,
    // even when the file's own orientation is not known.
    assert_eq!(read(&xml, RASTER), left_half);
}

#[test]
fn a_photo_turned_here_has_its_crop_written_for_the_picture_before_the_turn() {
    // Once the user has turned the picture a quarter clockwise, the left
    // half of what is on screen is the bottom half of the picture as stored.
    let left_half = rotated(
        CropRect {
            cx: 0.25,
            cy: 0.5,
            width: 0.5,
            height: 1.0,
        },
        0.0,
    );
    let turned = DevelopSettings {
        orientation: Orientation::new(1, false),
        ..cropped(left_half)
    };
    let stored_bottom_half = [
        r#"crs:CropLeft="0""#,
        r#"crs:CropTop="0.5""#,
        r#"crs:CropRight="1""#,
        r#"crs:CropBottom="1""#,
    ];

    // With the file's own orientation known, the sidecar carries the turn.
    let known = RASTER.with_orientation(Orientation::IDENTITY);
    let xml = mirror(&turned, known);
    assert!(xml.contains(r#"tiff:Orientation="6""#), "{xml}");
    assert!(
        stored_bottom_half.iter().all(|corner| xml.contains(corner)),
        "{xml}"
    );
    assert_eq!(parse(&xml, known).unwrap().develop, Some(turned.clone()));

    // Without it the turn cannot be written, but the crop still names the
    // same part of the picture for whoever shows it unturned.
    let xml = mirror(&turned, RASTER);
    assert!(!xml.contains("tiff:Orientation"), "{xml}");
    assert!(
        stored_bottom_half.iter().all(|corner| xml.contains(corner)),
        "{xml}"
    );
    let bottom_half = rotated(
        CropRect {
            cx: 0.5,
            cy: 0.75,
            width: 1.0,
            height: 0.5,
        },
        0.0,
    );
    assert_eq!(read(&xml, RASTER), bottom_half);
}

#[test]
fn a_rotated_crop_keeps_its_place_on_the_picture_in_every_frame() {
    // An upright source 2000 wide and 3000 high, or 3000 by 2000 when the
    // frame is not turned on its side.
    for file in all_orientations() {
        let (width, height) = if file.swaps_axes() {
            (2000, 3000)
        } else {
            (3000, 2000)
        };
        let upright_aspect = width as f32 / height as f32;
        let image = RASTER.with_orientation(file).with_size(width, height);
        let crop = rotated(RECT, 15.0);
        let xml = mirror(&cropped(crop), image);

        // The two corners written are opposite corners of the rectangle the
        // settings describe, seen in the stored frame.
        let stored = RECT
            .corners(15.0, upright_aspect)
            .map(|corner| file.inverse(corner));
        let [first, second] = written_corners(&xml);
        let opposite = (0..4)
            .any(|index| close(first, stored[index]) && close(second, stored[(index + 2) % 4]));
        assert!(
            opposite,
            "{file:?}: {first:?} and {second:?} are not opposite corners of {stored:?}"
        );
        // In the stored frame the first of them is the upper left one.
        assert!(
            stored
                .iter()
                .all(|corner| first.x + first.y <= corner.x + corner.y + 1e-5),
            "{file:?}"
        );
        assert_eq!(read(&xml, image), crop, "{file:?}");
    }
}

#[test]
fn a_straighten_angle_changes_sign_in_a_mirrored_frame() {
    let tilted = Crop {
        angle: 7.5,
        ..Crop::default()
    };
    let upright = mirror(&cropped(tilted), RASTER);
    assert!(upright.contains(r#"crs:CropAngle="+7.5""#), "{upright}");
    let turned = RASTER.with_orientation(Orientation::new(1, false));
    let xml = mirror(&cropped(tilted), turned);
    assert!(xml.contains(r#"crs:CropAngle="+7.5""#), "{xml}");
    let mirrored = RASTER.with_orientation(Orientation::new(0, true));
    let xml = mirror(&cropped(tilted), mirrored);
    assert!(xml.contains(r#"crs:CropAngle="-7.5""#), "{xml}");
    assert_eq!(read(&xml, mirrored), tilted);
}

#[test]
fn the_three_states_of_a_crop_are_told_apart() {
    let none = Crop::default();
    let xml = mirror(&cropped(none), RASTER);
    assert!(xml.contains(r#"crs:HasCrop="False""#), "{xml}");
    assert!(!xml.contains("crs:CropLeft"), "{xml}");
    assert_eq!(read(&xml, RASTER), none);

    let straightened = Crop {
        angle: -3.25,
        ..Crop::default()
    };
    let xml = mirror(&cropped(straightened), RASTER);
    assert!(xml.contains(r#"crs:HasCrop="True""#), "{xml}");
    assert!(xml.contains(r#"crs:CropAngle="-3.25""#), "{xml}");
    assert!(!xml.contains("crs:CropLeft"), "{xml}");
    assert_eq!(read(&xml, RASTER), straightened);

    let whole_frame = rotated(CropRect::FULL, 0.0);
    let xml = mirror(&cropped(whole_frame), RASTER);
    assert!(xml.contains(r#"crs:HasCrop="True""#), "{xml}");
    assert_eq!(read(&xml, RASTER), whole_frame);
}

#[test]
fn removing_a_crop_removes_its_corners_from_the_sidecar() {
    let with_crop = mirror(&cropped(rotated(RECT, 5.0)), RASTER);
    let without = mirror_onto(&cropped(Crop::default()), RASTER, Some(&with_crop));
    assert!(without.contains(r#"crs:HasCrop="False""#), "{without}");
    assert!(without.contains(r#"crs:CropAngle="0""#), "{without}");
    for corner in ["CropLeft", "CropTop", "CropRight", "CropBottom"] {
        assert!(!without.contains(corner), "{corner} is still in\n{without}");
    }
}

#[test]
fn a_crop_another_application_switched_off_is_no_crop_whatever_numbers_remain() {
    let xml = packet(
        r#"crs:HasCrop="False" crs:CropLeft="0.1" crs:CropTop="0.1" crs:CropRight="0.9" crs:CropBottom="0.9" crs:CropAngle="4""#,
    );
    assert_eq!(read(&xml, RASTER), Crop::default());

    // Corners without the switch still mean a crop; corners that enclose
    // nothing do not.
    let implied = xml.replace(r#"crs:HasCrop="False" "#, "");
    let crop = read(&implied, RASTER);
    assert_eq!(crop.angle, 4.0);
    assert!(crop.rect.is_some());
    let inverted = implied.replace(r#"crs:CropRight="0.9""#, r#"crs:CropRight="0.05""#);
    assert_eq!(read(&inverted, RASTER).rect, None);
    let incomplete = implied.replace(r#"crs:CropTop="0.1""#, "");
    assert_eq!(read(&incomplete, RASTER).rect, None);
}

#[test]
fn crops_round_trip_exactly_in_every_frame() {
    let mut state = 0x2545_f491_u32;
    let mut unit = move || {
        state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
        (state >> 8) as f32 / 16_777_216.0
    };
    for file in all_orientations() {
        for user in all_orientations() {
            let oriented = RASTER.with_orientation(file);
            for image in [
                oriented,
                oriented.with_size(6000, 4000),
                RASTER.with_size(3024, 4032),
            ] {
                for _ in 0..4 {
                    let width = CropRect::MIN_SIDE + unit() * (1.0 - CropRect::MIN_SIDE);
                    let height = CropRect::MIN_SIDE + unit() * (1.0 - CropRect::MIN_SIDE);
                    let rect = CropRect {
                        cx: width / 2.0 + unit() * (1.0 - width),
                        cy: height / 2.0 + unit() * (1.0 - height),
                        width,
                        height,
                    };
                    // The user's turn is carried only beside the file's own.
                    let turn = image.orientation.map_or(Orientation::IDENTITY, |_| user);
                    let wanted = DevelopSettings {
                        orientation: turn,
                        ..cropped(rotated(rect, (unit() - 0.5) * 90.0))
                    };
                    let read = parse(&mirror(&wanted, image), image).unwrap().develop;
                    assert_eq!(read, Some(wanted), "{image:?}");
                }
            }
        }
    }
}
