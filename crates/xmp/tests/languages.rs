//! Text held in several languages (`rdf:Alt`): the title, the caption and
//! the copyright read the default language, and changing or clearing one
//! leaves the other languages as they were.

use photo_core::meta::FileKind;
use photo_xmp::{ImageInfo, XmpData, parse, to_string};

const RAW: ImageInfo = ImageInfo::new(FileKind::Raw);

/// A packet whose one description holds `content`.
fn packet(content: &str) -> String {
    format!(
        r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
 <rdf:Description rdf:about="" xmlns:dc="http://purl.org/dc/elements/1.1/">
  {content}
 </rdf:Description>
</rdf:RDF>"#
    )
}

/// `property` as a language alternative of `items`, each a language and a
/// text.
fn alternative(property: &str, items: &[(&str, &str)]) -> String {
    let items: String = items
        .iter()
        .map(|(language, text)| format!(r#"<rdf:li xml:lang="{language}">{text}</rdf:li>"#))
        .collect();
    format!("<{property}><rdf:Alt>{items}</rdf:Alt></{property}>")
}

fn metadata(xml: &str) -> XmpData {
    parse(xml, RAW).unwrap()
}

fn title(data: &mut XmpData) -> &mut String {
    &mut data.title
}

fn caption(data: &mut XmpData) -> &mut String {
    &mut data.caption
}

fn copyright(data: &mut XmpData) -> &mut String {
    &mut data.copyright
}

/// The properties held as language alternatives, with the field each fills.
type Field = fn(&mut XmpData) -> &mut String;
const FIELDS: [(&str, Field); 3] = [
    ("dc:title", title),
    ("dc:description", caption),
    ("dc:rights", copyright),
];

#[test]
fn clearing_text_held_in_other_languages_too_keeps_them() {
    for (property, field) in FIELDS {
        let languages = [("x-default", "Harbour"), ("de", "Hafen"), ("fr", "Port")];
        let xml = packet(&alternative(property, &languages));
        let mut data = metadata(&xml);
        assert_eq!(field(&mut data), "Harbour");
        field(&mut data).clear();
        let updated = to_string(&data, RAW, Some(&xml)).unwrap();
        assert_eq!(metadata(&updated), data, "{updated}");
        for kept in [
            r#"<rdf:li xml:lang="de">Hafen</rdf:li>"#,
            r#"<rdf:li xml:lang="fr">Port</rdf:li>"#,
        ] {
            assert!(updated.contains(kept), "{kept} is missing from\n{updated}");
        }
        assert_eq!(to_string(&data, RAW, Some(&updated)).unwrap(), updated);
    }
}

#[test]
fn clearing_text_held_in_the_default_language_alone_removes_it() {
    for (property, field) in FIELDS {
        let xml = packet(&alternative(property, &[("x-default", "Harbour")]));
        let mut data = metadata(&xml);
        field(&mut data).clear();
        let updated = to_string(&data, RAW, Some(&xml)).unwrap();
        assert!(!updated.contains(property), "{updated}");
        assert_eq!(metadata(&updated), data);
    }
}

#[test]
fn new_text_without_a_default_language_becomes_it_beside_the_others() {
    for (property, field) in FIELDS {
        let xml = packet(&alternative(property, &[("fr", "Port"), ("de", "Hafen")]));
        let mut data = metadata(&xml);
        assert_eq!(field(&mut data), "Port", "the first language stands in");
        *field(&mut data) = "Harbour".to_owned();
        let updated = to_string(&data, RAW, Some(&xml)).unwrap();
        assert_eq!(metadata(&updated), data, "{updated}");
        for expected in [
            r#"<rdf:li xml:lang="x-default">Harbour</rdf:li>"#,
            r#"<rdf:li xml:lang="fr">Port</rdf:li>"#,
            r#"<rdf:li xml:lang="de">Hafen</rdf:li>"#,
        ] {
            assert!(
                updated.contains(expected),
                "{expected} is missing from\n{updated}"
            );
        }

        // Cleared again, it reads as no text and the languages stay.
        field(&mut data).clear();
        let cleared = to_string(&data, RAW, Some(&updated)).unwrap();
        assert_eq!(metadata(&cleared), data, "{cleared}");
        assert!(cleared.contains(r#"<rdf:li xml:lang="fr">Port</rdf:li>"#));
    }
}

#[test]
fn text_in_the_default_language_changes_where_it_is() {
    let xml = packet(&alternative(
        "dc:title",
        &[("de", "Hafen"), ("x-default", "Harbour")],
    ));
    let data = XmpData {
        title: "Quay".to_owned(),
        ..metadata(&xml)
    };
    let updated = to_string(&data, RAW, Some(&xml)).unwrap();
    let expected = xml.replace(">Harbour<", ">Quay<");
    assert_eq!(updated, expected);
}
