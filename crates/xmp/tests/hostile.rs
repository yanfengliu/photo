//! Sidecars no well-behaved application writes: characters XML forbids,
//! sizes no sidecar needs, and namespace declarations in numbers that make
//! careless lookups quadratic. Each is refused, or handled in time that grows
//! with its size and no faster.

use std::fs;
use std::time::{Duration, Instant};

use photo_core::develop::DevelopSettings;
use photo_core::meta::{FileKind, Flag, Rating};
use photo_xmp::{
    ImageInfo, MAX_SIDECAR_BYTES, XmpData, XmpError, parse, read_sidecar, to_string, write_sidecar,
};

const RAW: ImageInfo = ImageInfo::new(FileKind::Raw);
const FOREIGN: &str = include_str!("fixtures/foreign.xmp");

/// How many declarations and attributes the floods of prefixes hold.
const FLOOD: usize = 50_000;

/// How many copies of a property the floods of copies hold: more, because
/// moving the rest along for each copy is cheap work, only done too often.
const COPIES: usize = 100_000;

/// Far beyond what work in proportion to a flood takes, and far below what
/// work in proportion to its square does.
const FLOOD_TIME: Duration = Duration::from_secs(20);

fn is_malformed<T: std::fmt::Debug>(result: Result<T, XmpError>) -> bool {
    matches!(result, Err(XmpError::Malformed { .. }))
}

/// A packet whose one description carries `attributes` and holds `content`.
fn packet(attributes: &str, content: &str) -> String {
    format!(
        r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/"{attributes}>{content}</rdf:Description></rdf:RDF>"#
    )
}

/// A packet giving three stars, padded with white space to `size` bytes.
fn padded(size: usize) -> String {
    let mut text = packet(r#" xmp:Rating="3""#, "");
    let padding = size - text.len();
    text.push_str(&" ".repeat(padding));
    text
}

#[test]
fn a_character_xml_forbids_is_malformed_whether_written_or_referenced() {
    let referenced = ["&#1;", "&#x1F;", "&#11;", "&#xFFFE;", "&#xFFFF;"];
    let written = ["\u{1}", "\u{1b}", "\u{7f}\u{0}", "\u{fffe}", "\u{ffff}"];
    for character in referenced.into_iter().chain(written) {
        for xml in [
            format!("<a>{character}</a>"),
            format!(r#"<a b="{character}"/>"#),
        ] {
            assert!(is_malformed(parse(&xml, RAW)), "{xml:?} was accepted");
        }
    }
    // A comment resolves no references, but it may not hold such a character
    // itself either.
    for character in written {
        let xml = format!("<a><!--{character}--></a>");
        assert!(is_malformed(parse(&xml, RAW)), "{xml:?} was accepted");
    }
    // Every kind of character XML allows still reads, by reference or not.
    let allowed = "&#9;&#10;&#13;&#x20;&#x7F;&#xD7FF;&#xE000;&#xFFFD;&#x10000;&#x10FFFF;";
    let xml = packet(
        &format!(r#" dc:rights="{allowed}""#),
        &format!(
            r#"<dc:title><rdf:Alt><rdf:li xml:lang="x-default">{allowed} ☺</rdf:li></rdf:Alt></dc:title>"#
        ),
    );
    let data = parse(&xml, RAW).unwrap();
    let expected = "\t\n\r \u{7f}\u{d7ff}\u{e000}\u{fffd}\u{10000}\u{10ffff}";
    assert_eq!(data.copyright, expected);
    assert_eq!(data.title, format!("{expected} ☺"));
}

#[test]
fn a_sidecar_holding_a_character_xml_forbids_is_refused_rather_than_rewritten() {
    let forbidden = FOREIGN.replace(r#"acme:Score="7.5""#, r#"acme:Score="7&#1;5""#);
    assert_ne!(forbidden, FOREIGN);
    let read = XmpData {
        develop: None,
        ..parse(FOREIGN, RAW).unwrap()
    };
    let data = XmpData {
        rating: Rating::new(4),
        ..read
    };
    assert!(is_malformed(to_string(&data, RAW, Some(&forbidden))));

    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("IMG_0001.xmp");
    fs::write(&path, &forbidden).unwrap();
    assert!(is_malformed(write_sidecar(&path, &data, RAW)));
    assert_eq!(fs::read_to_string(&path).unwrap(), forbidden);
}

#[test]
fn a_packet_without_rdf_that_gains_no_property_comes_back_as_it_was() {
    let data = XmpData::default();
    for xml in [
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"/>"#,
        "<x:xmpmeta xmlns:x=\"adobe:ns:meta/\">\n <x:other/>\n</x:xmpmeta>",
        // An `rdf:RDF` whose namespace is mistyped is not RDF.
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns"/></x:xmpmeta>"#,
    ] {
        assert_eq!(parse(xml, RAW).unwrap(), data);
        assert_eq!(to_string(&data, RAW, Some(xml)).unwrap(), xml);
        // Given something to hold, it gains the element it lacked.
        let rated = XmpData {
            rating: Rating::new(2),
            ..XmpData::default()
        };
        let updated = to_string(&rated, RAW, Some(xml)).unwrap();
        assert_eq!(parse(&updated, RAW).unwrap(), rated, "{updated}");
    }
}

#[test]
fn a_prefix_declared_empty_is_bound_again_rather_than_declared_twice() {
    let xml = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:photo="" xmlns:xmp="" xmlns:dc=""/></rdf:RDF>"#;
    let data = XmpData {
        rating: Rating::new(3),
        flag: Flag::Picked,
        keywords: vec!["quay".to_owned()],
        ..XmpData::default()
    };
    let updated = to_string(&data, RAW, Some(xml)).unwrap();
    for declaration in ["xmlns:photo=", "xmlns:xmp=", "xmlns:dc="] {
        assert_eq!(updated.matches(declaration).count(), 1, "{updated}");
    }
    assert_eq!(parse(&updated, RAW).unwrap(), data);
}

#[test]
fn a_sidecar_larger_than_the_limit_is_refused_unread() {
    let largest = padded(MAX_SIDECAR_BYTES);
    assert_eq!(largest.len(), MAX_SIDECAR_BYTES);
    assert_eq!(parse(&largest, RAW).unwrap().rating, Rating::new(3));

    let too_large = padded(MAX_SIDECAR_BYTES + 1);
    assert!(matches!(parse(&too_large, RAW), Err(XmpError::TooLarge)));
    let data = XmpData::default();
    assert!(matches!(
        to_string(&data, RAW, Some(&too_large)),
        Err(XmpError::TooLarge)
    ));

    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("IMG_0001.xmp");
    fs::write(&path, &too_large).unwrap();
    assert!(matches!(read_sidecar(&path, RAW), Err(XmpError::TooLarge)));
    assert!(matches!(
        write_sidecar(&path, &data, RAW),
        Err(XmpError::TooLarge)
    ));
    assert_eq!(fs::read(&path).unwrap().len(), MAX_SIDECAR_BYTES + 1);
    let error = XmpError::TooLarge.to_string();
    assert!(error.contains(&MAX_SIDECAR_BYTES.to_string()), "{error}");
}

#[test]
fn a_sidecar_too_large_to_read_back_is_not_written() {
    let data = XmpData {
        title: "x".repeat(MAX_SIDECAR_BYTES),
        ..XmpData::default()
    };
    assert!(matches!(
        to_string(&data, RAW, None),
        Err(XmpError::TooLarge)
    ));
    let folder = tempfile::tempdir().unwrap();
    let path = folder.path().join("IMG_0001.xmp");
    assert!(matches!(
        write_sidecar(&path, &data, RAW),
        Err(XmpError::TooLarge)
    ));
    assert!(!path.exists());
}

/// Reads `xml`, changes the rating, the keywords and the exposure, and
/// checks that the change holds; all of it within [`FLOOD_TIME`].
fn update_in_time(xml: &str) {
    assert!(xml.len() < MAX_SIDECAR_BYTES);
    let start = Instant::now();
    let read = parse(xml, RAW).unwrap();
    let mut develop = read
        .develop
        .clone()
        .unwrap_or_else(|| DevelopSettings::default_for(FileKind::Raw));
    develop.tone.exposure = 1.5;
    let data = XmpData {
        rating: Rating::new(5),
        keywords: vec!["one".to_owned()],
        develop: Some(develop),
        ..read
    };
    let updated = to_string(&data, RAW, Some(xml)).unwrap();
    assert_eq!(parse(&updated, RAW).unwrap(), data);
    let elapsed = start.elapsed();
    assert!(elapsed < FLOOD_TIME, "took {elapsed:?}");
}

#[test]
fn many_prefixes_on_one_element_are_looked_up_without_scanning_them_all() {
    // Each prefix bound to a namespace of its own, and used once.
    let mut distinct = String::new();
    for index in 0..FLOOD {
        distinct.push_str(&format!(r#" xmlns:p{index}="urn:p{index}" p{index}:a="1""#));
    }
    update_in_time(&packet(&distinct, ""));

    // Every prefix bound to the Camera Raw namespace, each naming the same
    // property again.
    let mut same = String::new();
    for index in 0..FLOOD {
        same.push_str(&format!(
            r#" xmlns:c{index}="http://ns.adobe.com/camera-raw-settings/1.0/" c{index}:Exposure2012="+0.5""#
        ));
    }
    update_in_time(&packet(&same, ""));
}

#[test]
fn many_copies_of_a_property_are_removed_without_shifting_the_rest_each_time() {
    // Each copy is followed by a property that stays, which removing the
    // copies one at a time would move along again for every copy before it.
    let ratings = "<xmp:Rating>1</xmp:Rating><xmp:Nickname>n</xmp:Nickname>\n".repeat(COPIES);
    update_in_time(&packet("", &ratings));
    let subject = "<dc:subject><rdf:Bag><rdf:li>a</rdf:li></rdf:Bag></dc:subject>";
    let subjects = format!("{subject}<dc:format>f</dc:format>").repeat(COPIES);
    update_in_time(&packet("", &subjects));
}
