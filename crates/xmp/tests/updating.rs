//! Updating a sidecar another application wrote changes what this crate owns
//! and keeps everything else.

use std::collections::BTreeMap;

use photo_core::meta::{ColorLabel, FileKind, Flag, Rating};
use photo_core::time::DateTime;
use photo_xmp::{ImageInfo, XmpData, XmpError, parse, to_string};
use quick_xml::Reader;
use quick_xml::events::Event;

const FOREIGN: &str = include_str!("fixtures/foreign.xmp");
const ELEMENT_FORM: &str = include_str!("fixtures/element_form.xmp");

const RAW: ImageInfo = ImageInfo::new(FileKind::Raw);

/// The descriptive metadata of `xml`, without develop settings.
fn metadata(xml: &str) -> XmpData {
    XmpData {
        develop: None,
        ..parse(xml, RAW).unwrap()
    }
}

/// Everything a document says, counted: elements by name, attributes by name
/// and value, and every piece of text. Layout does not count; content does.
fn inventory(xml: &str) -> BTreeMap<String, usize> {
    let mut reader = Reader::from_str(xml);
    let mut counts = BTreeMap::new();
    let mut count = |entry: String| *counts.entry(entry).or_insert(0) += 1;
    loop {
        match reader.read_event().unwrap() {
            Event::Start(tag) | Event::Empty(tag) => {
                count(format!(
                    "element {}",
                    String::from_utf8_lossy(tag.name().as_ref())
                ));
                for attribute in tag.attributes() {
                    let attribute = attribute.unwrap();
                    let value = attribute
                        .normalized_value(quick_xml::XmlVersion::Implicit1_0)
                        .unwrap();
                    count(format!(
                        "attribute {}={value}",
                        String::from_utf8_lossy(attribute.key.as_ref())
                    ));
                }
            }
            Event::Text(text) => {
                let text = text.decode().unwrap();
                if !text.trim().is_empty() {
                    count(format!("text {}", text.trim()));
                }
            }
            Event::GeneralRef(reference) => {
                count(format!("reference {}", reference.decode().unwrap()))
            }
            Event::CData(data) => count(format!("cdata {}", data.decode().unwrap())),
            Event::Comment(comment) => count(format!("comment {}", comment.decode().unwrap())),
            Event::PI(instruction) => count(format!(
                "instruction {}",
                String::from_utf8_lossy(&instruction)
            )),
            Event::Decl(declaration) => count(format!(
                "declaration {}",
                String::from_utf8_lossy(&declaration)
            )),
            Event::DocType(doctype) => count(format!("doctype {}", doctype.decode().unwrap())),
            Event::End(_) => {}
            Event::Eof => return counts,
        }
    }
}

/// The text of `xml` from `open` through `close`.
fn block<'a>(xml: &'a str, open: &str, close: &str) -> &'a str {
    let start = xml.find(open).unwrap();
    let end = start + xml[start..].find(close).unwrap() + close.len();
    &xml[start..end]
}

#[test]
fn writing_back_what_was_read_changes_nothing() {
    assert_eq!(
        to_string(&metadata(FOREIGN), RAW, Some(FOREIGN)).unwrap(),
        FOREIGN
    );
    assert_eq!(
        to_string(&metadata(ELEMENT_FORM), RAW, Some(ELEMENT_FORM)).unwrap(),
        ELEMENT_FORM
    );
}

#[test]
fn changing_one_value_changes_only_that_value() {
    let data = XmpData {
        rating: Rating::new(4),
        ..metadata(FOREIGN)
    };
    let updated = to_string(&data, RAW, Some(FOREIGN)).unwrap();

    let mut expected = inventory(FOREIGN);
    expected.remove("attribute xmp:Rating=2").unwrap();
    expected.insert("attribute xmp:Rating=4".to_owned(), 1);
    assert_eq!(inventory(&updated), expected);

    // Outside the one start tag that holds the rating, not a byte moved.
    let tag = r#"<rdf:Description rdf:about="""#;
    let rest = "<!-- a comment between properties -->";
    assert_eq!(
        &updated[..updated.find(tag).unwrap()],
        &FOREIGN[..FOREIGN.find(tag).unwrap()]
    );
    assert_eq!(
        &updated[updated.find(rest).unwrap()..],
        &FOREIGN[FOREIGN.find(rest).unwrap()..]
    );
}

#[test]
fn picking_and_unflagging_touch_only_the_private_flag() {
    let picked = XmpData {
        flag: Flag::Picked,
        ..metadata(FOREIGN)
    };
    let updated = to_string(&picked, RAW, Some(FOREIGN)).unwrap();
    assert_eq!(metadata(&updated), picked);

    // The flag needs its namespace declared; nothing else is new or gone.
    let mut without_flag = inventory(FOREIGN);
    let declaration = "attribute xmlns:photo=http://ns.photo.invalid/1.0/";
    without_flag.insert(declaration.to_owned(), 1);
    let mut with_flag = without_flag.clone();
    with_flag.insert("attribute photo:Flag=1".to_owned(), 1);
    assert_eq!(inventory(&updated), with_flag);

    // Unflagging removes the property again and leaves the declaration.
    let unflagged = to_string(&metadata(FOREIGN), RAW, Some(&updated)).unwrap();
    assert_eq!(inventory(&unflagged), without_flag);
    assert_eq!(metadata(&unflagged), metadata(FOREIGN));
}

#[test]
fn adding_develop_settings_keeps_everything_the_other_application_wrote() {
    let read = parse(FOREIGN, RAW).unwrap();
    let mut settings = read.develop.clone().unwrap();
    assert_eq!(
        (settings.tone.exposure, settings.tone.contrast),
        (0.5, 10.0)
    );
    settings.tone.exposure = 1.25;
    let data = XmpData {
        rating: Rating::new(5),
        keywords: vec!["harbour".to_owned(), "night".to_owned()],
        develop: Some(settings),
        ..read
    };
    let updated = to_string(&data, RAW, Some(FOREIGN)).unwrap();
    assert_eq!(parse(&updated, RAW).unwrap(), data);

    // Nothing the other application wrote is gone, except the two values that changed.
    let after = inventory(&updated);
    for (entry, count) in inventory(FOREIGN) {
        let replaced = ["attribute xmp:Rating=2", "attribute crs:Exposure2012=+0.50"]
            .contains(&entry.as_str());
        let kept = after.get(&entry).copied().unwrap_or(0);
        assert!(
            replaced || kept >= count,
            "lost {entry:?}: {count} before, {kept} after"
        );
    }
    assert_eq!(after.get("attribute xmp:Rating=5"), Some(&1));
    assert_eq!(after.get("attribute crs:Exposure2012=+1.25"), Some(&1));
    assert_eq!(after.get("attribute xmp:Rating=2"), None);
    assert_eq!(after.get("attribute crs:Exposure2012=+0.50"), None);
    // A value that did not change keeps the spelling it had.
    assert_eq!(after.get("attribute crs:Contrast2012=+10"), Some(&1));
    assert_eq!(after.get("text night"), Some(&1));

    // Structures this crate does not interpret are carried over byte for byte,
    // including the look-alike properties nested inside them.
    for (open, close) in [
        ("<xmpMM:History>", "</xmpMM:History>"),
        ("<crs:Look>", "</crs:Look>"),
        (
            "<crs:MaskGroupBasedCorrections>",
            "</crs:MaskGroupBasedCorrections>",
        ),
        ("<acme:Recipe", "</acme:Recipe>"),
        ("<dc:title>", "</dc:title>"),
        (
            r#"<rdf:Description rdf:about="" xmlns:other"#,
            "</rdf:Description>",
        ),
        ("<?xml", "this comment must survive -->"),
        ("</rdf:RDF>", "<?acme-trailer done?>\n"),
    ] {
        let kept = block(FOREIGN, open, close);
        assert!(updated.contains(kept), "changed:\n{kept}");
    }
    assert!(updated.contains(r#"xmp:Label="Review later""#));
    assert!(updated.contains(r#"acme:Note="He said &quot;keep&quot; &amp; left""#));
    assert!(updated.contains("<photo:Settings>{"));
}

#[test]
fn an_update_is_stable_once_written() {
    let read = parse(FOREIGN, RAW).unwrap();
    let once = to_string(&read, RAW, Some(FOREIGN)).unwrap();
    assert_ne!(
        once, FOREIGN,
        "the first write adds the settings the file did not have"
    );
    assert_eq!(to_string(&read, RAW, Some(&once)).unwrap(), once);
    assert_eq!(parse(&once, RAW).unwrap(), read);
}

#[test]
fn an_element_form_sidecar_keeps_its_forms_and_prefixes() {
    let read = parse(ELEMENT_FORM, RAW).unwrap();
    let mut settings = read.develop.clone().unwrap();
    settings.tone.exposure = 0.5;
    settings.tone_curve.blue = Default::default();
    let data = XmpData {
        rating: Rating::new(3),
        flag: Default::default(),
        label: ColorLabel::Red,
        title: "Evening".to_owned(),
        keywords: vec!["Zürich".to_owned(), "dusk".to_owned()],
        develop: Some(settings),
        ..read
    };
    let updated = to_string(&data, RAW, Some(ELEMENT_FORM)).unwrap();
    assert_eq!(parse(&updated, RAW).unwrap(), data);

    for expected in [
        "<xap:Rating>3</xap:Rating>",
        "<xap:Label>Red</xap:Label>",
        "<cr:Exposure2012>+0.5</cr:Exposure2012>",
        "<cr:Contrast2012>+30</cr:Contrast2012>",
        "<r:li xml:lang='x-default'>Evening</r:li>",
        "<r:li>dusk</r:li>",
        "<r:li>255, 255</r:li>",
        "cr:Shadows2012=\"0\"",
        "<r:li xml:lang='de'>Alle Rechte vorbehalten</r:li>",
        "<r:li>centerX = 0.25, centerY = 0.75, radius = 0.01, sourceState = sourceSetExplicitly</r:li>",
        "<?xpacket end='w'?>",
    ] {
        assert!(
            updated.contains(expected),
            "{expected} is missing from\n{updated}"
        );
    }
    // Every namespace was already bound, under the file's own prefixes.
    for unexpected in [
        "<rdf:",
        "xmlns:rdf",
        "xmlns:crs",
        "xmlns:xmp=",
        "xmlns:dc",
        "crs:",
    ] {
        assert!(
            !updated.contains(unexpected),
            "{unexpected} appears in\n{updated}"
        );
    }
}

#[test]
fn a_label_this_crate_does_not_know_is_kept_until_a_colour_replaces_it() {
    let kept = to_string(&metadata(FOREIGN), RAW, Some(FOREIGN)).unwrap();
    assert!(kept.contains(r#"xmp:Label="Review later""#));

    let coloured = XmpData {
        label: ColorLabel::Yellow,
        ..metadata(FOREIGN)
    };
    let updated = to_string(&coloured, RAW, Some(FOREIGN)).unwrap();
    assert!(updated.contains(r#"xmp:Label="Yellow""#) && !updated.contains("Review later"));

    let cleared = to_string(&metadata(FOREIGN), RAW, Some(&updated)).unwrap();
    assert!(!cleared.contains("xmp:Label"), "{cleared}");
}

#[test]
fn a_title_in_several_languages_keeps_the_others() {
    let renamed = XmpData {
        title: "Harbour at night".to_owned(),
        ..metadata(FOREIGN)
    };
    let updated = to_string(&renamed, RAW, Some(FOREIGN)).unwrap();
    assert!(updated.contains(r#"<rdf:li xml:lang="x-default">Harbour at night</rdf:li>"#));
    assert!(updated.contains(r#"<rdf:li xml:lang="de">Hafen</rdf:li>"#));

    let untitled = XmpData {
        title: String::new(),
        keywords: Vec::new(),
        ..metadata(FOREIGN)
    };
    let updated = to_string(&untitled, RAW, Some(FOREIGN)).unwrap();
    assert!(
        !updated.contains("dc:title") && !updated.contains("dc:subject"),
        "{updated}"
    );
    assert_eq!(parse(&updated, RAW).unwrap().title, "");
}

#[test]
fn a_capture_time_is_added_when_new_and_otherwise_left_as_written() {
    let attribute_form = include_str!("fixtures/attribute_form.xmp");
    let read = metadata(attribute_form);
    assert_eq!(read.capture_time, DateTime::new(2024, 6, 1, 18, 22, 7));
    let silent = XmpData {
        capture_time: None,
        ..read.clone()
    };
    assert_eq!(
        to_string(&silent, RAW, Some(attribute_form)).unwrap(),
        attribute_form
    );

    let corrected = XmpData {
        capture_time: DateTime::new(2024, 6, 2, 9, 0, 0),
        ..read
    };
    let updated = to_string(&corrected, RAW, Some(attribute_form)).unwrap();
    assert!(updated.contains(r#"exif:DateTimeOriginal="2024-06-02T09:00:00""#));
    assert!(updated.contains(r#"xmp:CreateDate="2024-06-01T18:22:07.31+02:00""#));
    assert_eq!(
        parse(&updated, RAW).unwrap().capture_time,
        corrected.capture_time
    );
}

#[test]
fn a_property_present_twice_ends_up_present_once() {
    let xml = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/">
<rdf:Description rdf:about="" xmp:Rating="1" dc:subject="loose"><xmp:Rating>2</xmp:Rating></rdf:Description>
<rdf:Description rdf:about=""><xmp:Rating>3</xmp:Rating><dc:subject><rdf:Bag><rdf:li>bagged</rdf:li></rdf:Bag></dc:subject></rdf:Description>
</rdf:RDF>"#;
    let read = metadata(xml);
    assert_eq!(
        (read.rating, read.keywords.as_slice()),
        (Rating::new(1), ["loose".to_owned()].as_slice())
    );
    let data = XmpData {
        rating: Rating::new(4),
        keywords: vec!["new".to_owned()],
        ..read
    };
    let updated = to_string(&data, RAW, Some(xml)).unwrap();
    assert_eq!(updated.matches("Rating").count(), 1, "{updated}");
    assert!(updated.contains(r#"xmp:Rating="4""#), "{updated}");
    assert_eq!(updated.matches("dc:subject").count(), 2, "{updated}");
    assert!(
        updated.contains("<rdf:li>new</rdf:li>")
            && !updated.contains("bagged")
            && !updated.contains("loose")
    );
    assert_eq!(metadata(&updated), data);
}

#[test]
fn a_prefix_that_already_means_something_else_is_not_reused() {
    let xml = r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:xmp="http://ns.example.com/not-xmp/"><rdf:Description rdf:about="" xmp:Rating="9"/></rdf:RDF></x:xmpmeta>"#;
    assert_eq!(metadata(xml).rating, Rating::UNRATED);
    let data = XmpData {
        rating: Rating::new(2),
        ..XmpData::default()
    };
    let updated = to_string(&data, RAW, Some(xml)).unwrap();
    assert!(updated.contains(r#"xmp:Rating="9""#), "{updated}");
    assert!(
        updated.contains(r#"xmlns:xmp1="http://ns.adobe.com/xap/1.0/""#),
        "{updated}"
    );
    assert!(updated.contains(r#"xmp1:Rating="2""#), "{updated}");
    assert_eq!(metadata(&updated), data);
}

#[test]
fn a_new_property_joins_the_description_that_holds_its_namespace() {
    // One description per namespace, as some applications write them.
    let xml = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
 <rdf:Description rdf:about="" xmlns:purl="http://purl.org/dc/elements/1.1/">
  <purl:format>image/x-raw</purl:format>
 </rdf:Description>
 <rdf:Description rdf:about="" xmlns:xap="http://ns.adobe.com/xap/1.0/">
  <xap:CreatorTool>Acme Raw 9.1</xap:CreatorTool>
 </rdf:Description>
</rdf:RDF>"#;
    let data = XmpData {
        rating: Rating::new(3),
        keywords: vec!["quay".to_owned()],
        capture_time: DateTime::new(2025, 3, 9, 6, 30, 0),
        ..XmpData::default()
    };
    let updated = to_string(&data, RAW, Some(xml)).unwrap();
    let expected = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
 <rdf:Description rdf:about=""
   xmlns:purl="http://purl.org/dc/elements/1.1/"
   xmlns:exif="http://ns.adobe.com/exif/1.0/"
   exif:DateTimeOriginal="2025-03-09T06:30:00">
  <purl:format>image/x-raw</purl:format>
  <purl:subject>
   <rdf:Bag>
    <rdf:li>quay</rdf:li>
   </rdf:Bag>
  </purl:subject>
 </rdf:Description>
 <rdf:Description rdf:about=""
   xmlns:xap="http://ns.adobe.com/xap/1.0/"
   xap:Rating="3">
  <xap:CreatorTool>Acme Raw 9.1</xap:CreatorTool>
 </rdf:Description>
</rdf:RDF>"#;
    assert_eq!(updated, expected);
    assert_eq!(metadata(&updated), data);
}

#[test]
fn a_packet_without_a_description_or_without_rdf_gains_what_it_needs() {
    let data = XmpData {
        rating: Rating::new(1),
        keywords: vec!["only".to_owned()],
        ..XmpData::default()
    };
    for existing in [
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"/></x:xmpmeta>"#,
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"/>"#,
        r#"<RDF xmlns="http://www.w3.org/1999/02/22-rdf-syntax-ns#"></RDF>"#,
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/" xmlns:r="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><r:RDF><r:Description/></r:RDF></x:xmpmeta>"#,
    ] {
        let updated = to_string(&data, RAW, Some(existing)).unwrap();
        assert_eq!(metadata(&updated), data, "{updated}");
        assert_eq!(to_string(&data, RAW, Some(&updated)).unwrap(), updated);
    }
}

#[test]
fn a_blank_existing_sidecar_counts_as_none() {
    let data = XmpData {
        rating: Rating::new(3),
        ..XmpData::default()
    };
    let fresh = to_string(&data, RAW, None).unwrap();
    for blank in ["", " \n", "\u{feff}\n"] {
        assert_eq!(to_string(&data, RAW, Some(blank)).unwrap(), fresh);
    }
}

#[test]
fn an_existing_sidecar_that_cannot_be_understood_is_refused() {
    let data = XmpData::default();
    assert!(matches!(
        to_string(&data, RAW, Some("<x:xmpmeta")),
        Err(XmpError::Malformed { .. })
    ));
    assert!(matches!(
        to_string(&data, RAW, Some("<settings><exposure/></settings>")),
        Err(XmpError::NotXmp)
    ));
}
