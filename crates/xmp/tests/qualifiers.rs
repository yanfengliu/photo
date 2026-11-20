//! Properties in the rarer forms RDF allows: a value with qualifiers beside
//! it (XMP specification, part 1, "Qualifiers"), and a list inside a
//! property element that binds the RDF prefix to something else. Reading
//! finds the value; an update changes the value and nothing around it.

use photo_core::develop::{CurvePoint, DevelopSettings, PointCurve};
use photo_core::meta::{ColorLabel, FileKind, Rating};
use photo_xmp::{ImageInfo, XmpData, parse, to_string};

const RAW: ImageInfo = ImageInfo::new(FileKind::Raw);
const RDF: &str = "http://www.w3.org/1999/02/22-rdf-syntax-ns#";

/// A packet whose one description holds `content`.
fn packet(content: &str) -> String {
    format!(
        r#"<rdf:RDF xmlns:rdf="{RDF}">
 <rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" xmlns:q="urn:example:qualifiers">
  {content}
 </rdf:Description>
</rdf:RDF>"#
    )
}

fn metadata(xml: &str) -> XmpData {
    XmpData {
        develop: None,
        ..parse(xml, RAW).unwrap()
    }
}

/// `xml` without the element holding the private settings, so that only
/// the Camera Raw mapping is left to read.
fn without_private_settings(xml: &str) -> String {
    let start = xml.find("<photo:Settings>").unwrap();
    let close = "</photo:Settings>";
    let end = xml.find(close).unwrap() + close.len();
    format!("{}{}", &xml[..start], &xml[end..])
}

/// The forms the XMP specification gives a qualified value: `property`
/// holding `value`, with one qualifier beside it.
fn qualified(property: &str, value: &str) -> [String; 4] {
    [
        format!(
            r#"<{property}><rdf:Description><rdf:value>{value}</rdf:value><q:source>camera</q:source></rdf:Description></{property}>"#
        ),
        format!(
            r#"<{property} rdf:parseType="Resource"><rdf:value>{value}</rdf:value><q:source>camera</q:source></{property}>"#
        ),
        format!(r#"<{property} rdf:value="{value}" q:source="camera"/>"#),
        format!(
            r#"<{property}><rdf:Description rdf:value="{value}" q:source="camera"/></{property}>"#
        ),
    ]
}

#[test]
fn a_qualified_value_is_read_from_its_rdf_value() {
    for form in qualified("xmp:Rating", "4") {
        assert_eq!(metadata(&packet(&form)).rating, Rating::new(4), "{form}");
    }
    for form in qualified("xmp:Label", "Blue") {
        assert_eq!(metadata(&packet(&form)).label, ColorLabel::Blue, "{form}");
    }
    for form in qualified("crs:Exposure2012", "+1.5") {
        let develop = parse(&packet(&form), RAW).unwrap().develop.unwrap();
        assert_eq!(develop.tone.exposure, 1.5, "{form}");
    }
}

#[test]
fn a_structure_without_rdf_value_is_still_no_value() {
    for form in [
        "<xmp:Rating><rdf:Description><q:stars>4</q:stars></rdf:Description></xmp:Rating>",
        r#"<xmp:Rating rdf:parseType="Resource"><q:stars>4</q:stars></xmp:Rating>"#,
    ] {
        assert_eq!(metadata(&packet(form)).rating, Rating::UNRATED, "{form}");
    }
}

#[test]
fn updating_a_qualified_value_changes_the_value_and_keeps_its_qualifiers() {
    let forms = qualified("xmp:Rating", "4")
        .into_iter()
        .zip(qualified("xmp:Rating", "2"));
    for (before, after) in forms {
        let xml = packet(&before);
        let data = XmpData {
            rating: Rating::new(2),
            ..metadata(&xml)
        };
        let updated = to_string(&data, RAW, Some(&xml)).unwrap();
        assert_eq!(updated, packet(&after));
        assert_eq!(metadata(&updated), data);
    }
}

#[test]
fn qualified_items_and_qualified_lists_are_read_from_rdf_value() {
    let xml = packet(
        r#"<dc:subject><rdf:Bag><rdf:li rdf:parseType="Resource"><rdf:value>harbour</rdf:value><q:source>camera</q:source></rdf:li><rdf:li>dusk</rdf:li><rdf:li rdf:value="quay" q:source="me"/></rdf:Bag></dc:subject>
  <dc:title><rdf:Alt><rdf:li xml:lang="x-default"><rdf:Description><rdf:value>Port</rdf:value><q:source>me</q:source></rdf:Description></rdf:li></rdf:Alt></dc:title>
  <dc:creator rdf:parseType="Resource"><rdf:value><rdf:Seq><rdf:li>Ada</rdf:li></rdf:Seq></rdf:value><q:source>me</q:source></dc:creator>"#,
    );
    let data = metadata(&xml);
    assert_eq!(data.keywords, ["harbour", "dusk", "quay"]);
    assert_eq!(data.title, "Port");
    assert_eq!(data.creators, ["Ada"]);
}

#[test]
fn a_qualified_title_or_list_keeps_its_qualifiers_when_it_changes() {
    let title = r#"<dc:title><rdf:Alt><rdf:li xml:lang="x-default" rdf:parseType="Resource"><rdf:value>Port</rdf:value><q:source>me</q:source></rdf:li><rdf:li xml:lang="de">Hafen</rdf:li></rdf:Alt></dc:title>"#;
    let creators = r#"<dc:creator rdf:parseType="Resource"><rdf:value><rdf:Seq><rdf:li>Ada</rdf:li></rdf:Seq></rdf:value><q:source>me</q:source></dc:creator>"#;
    let xml = packet(&format!("{title}\n  {creators}"));
    let data = XmpData {
        title: "Harbour".to_owned(),
        creators: vec!["Ada".to_owned(), "Ben".to_owned()],
        ..metadata(&xml)
    };
    let updated = to_string(&data, RAW, Some(&xml)).unwrap();
    assert_eq!(metadata(&updated), data, "{updated}");
    let title = title.replace(">Port<", ">Harbour<");
    assert!(updated.contains(&title), "{updated}");
    let qualifier = "</rdf:value><q:source>me</q:source></dc:creator>";
    assert!(updated.contains(qualifier), "{updated}");
    assert!(updated.contains("<rdf:li>Ben</rdf:li>"), "{updated}");
}

#[test]
fn a_list_is_written_in_the_rdf_namespace_wherever_its_prefix_is_rebound() {
    let xml = format!(
        r#"<r:RDF xmlns:r="{RDF}">
 <r:Description r:about="" xmlns:dc="http://purl.org/dc/elements/1.1/">
  <dc:subject xmlns:r="urn:foreign"><s:Bag xmlns:s="{RDF}"><s:li>old</s:li></s:Bag></dc:subject>
 </r:Description>
</r:RDF>"#
    );
    assert_eq!(metadata(&xml).keywords, ["old"]);
    let data = XmpData {
        keywords: vec!["new".to_owned(), "newer".to_owned()],
        ..metadata(&xml)
    };
    let updated = to_string(&data, RAW, Some(&xml)).unwrap();
    assert_eq!(metadata(&updated), data, "{updated}");
    assert!(
        updated.contains(r#"<dc:subject xmlns:r="urn:foreign">"#),
        "{updated}"
    );
    assert_eq!(to_string(&data, RAW, Some(&updated)).unwrap(), updated);
}

#[test]
fn a_tone_curve_is_written_in_the_rdf_namespace_wherever_its_prefix_is_rebound() {
    let xml = packet(&format!(
        r#"<crs:ToneCurvePV2012 xmlns:rdf="urn:foreign"><x:Seq xmlns:x="{RDF}"><x:li>0, 0</x:li><x:li>255, 255</x:li></x:Seq></crs:ToneCurvePV2012>"#
    ));
    let mut settings = parse(&xml, RAW)
        .unwrap()
        .develop
        .unwrap_or_else(|| DevelopSettings::default_for(FileKind::Raw));
    let level = |x: u8, y: u8| CurvePoint::new(f32::from(x) / 255.0, f32::from(y) / 255.0);
    settings.tone_curve.rgb = PointCurve::new([level(0, 10), level(128, 140), level(255, 250)]);
    let data = XmpData {
        develop: Some(settings.clone()),
        ..XmpData::default()
    };
    let updated = to_string(&data, RAW, Some(&xml)).unwrap();
    let mirror = without_private_settings(&updated);
    let read = parse(&mirror, RAW).unwrap().develop.unwrap();
    assert_eq!(read.tone_curve.rgb, settings.tone_curve.rgb, "{updated}");
}
