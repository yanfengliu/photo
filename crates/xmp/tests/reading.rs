//! Sidecars in the forms other applications write parse into the expected
//! values, and broken ones are refused with a typed error.

use photo_core::develop::{
    Calibration, ColorBand, ColorGrading, Crop, CropRect, CurvePoint, DevelopSettings, GradeWheel,
    Grain, LensCorrections, NoiseReduction, ParametricCurve, PointCurve, Presence, Sharpening,
    Tone, Transform, Treatment, Vignette, WbPreset, WhiteBalance,
};
use photo_core::meta::{ColorLabel, FileKind, Flag, Rating};
use photo_core::time::DateTime;
use photo_xmp::{ImageInfo, XmpError, parse};

const ATTRIBUTE_FORM: &str = include_str!("fixtures/attribute_form.xmp");
const ELEMENT_FORM: &str = include_str!("fixtures/element_form.xmp");

const RAW: ImageInfo = ImageInfo::new(FileKind::Raw);
const RASTER: ImageInfo = ImageInfo::new(FileKind::Raster);

/// A curve point given on the 0 to 255 scale sidecars use.
fn level(x: u8, y: u8) -> CurvePoint {
    CurvePoint::new(f32::from(x) / 255.0, f32::from(y) / 255.0)
}

/// A minimal packet with `attributes` on its one description.
fn packet(attributes: &str) -> String {
    format!(
        r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"><rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#"><rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmlns:dc="http://purl.org/dc/elements/1.1/" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" {attributes}/></rdf:RDF></x:xmpmeta>"#
    )
}

fn is_malformed(result: Result<photo_xmp::XmpData, XmpError>) -> bool {
    matches!(result, Err(XmpError::Malformed { .. }))
}

#[test]
fn an_attribute_form_sidecar_yields_its_metadata() {
    let data = parse(ATTRIBUTE_FORM, RAW).unwrap();
    assert_eq!(data.rating, Rating::new(4));
    assert_eq!(data.flag, Flag::Unflagged);
    assert_eq!(data.label, ColorLabel::Green);
    assert_eq!(data.title, "");
    assert_eq!(data.keywords, ["harbour", "blue hour"]);
    assert_eq!(
        data.hierarchical_keywords,
        [vec!["Places", "Harbour"], vec!["Light", "blue hour"]]
    );
    assert_eq!(data.capture_time, DateTime::new(2024, 6, 1, 18, 22, 7));
}

#[test]
fn an_attribute_form_sidecar_yields_its_develop_settings_and_nothing_else() {
    let develop = parse(ATTRIBUTE_FORM, RAW).unwrap().develop.unwrap();

    let mut expected = DevelopSettings::default_for(FileKind::Raw);
    expected.white_balance = WhiteBalance {
        preset: WbPreset::Custom,
        temp: 5350.0,
        tint: 12.0,
    };
    expected.tone = Tone {
        exposure: 0.35,
        contrast: -12.0,
        highlights: -80.0,
        shadows: 45.0,
        whites: 10.0,
        blacks: -5.0,
    };
    expected.presence = Presence {
        texture: 8.0,
        clarity: 15.0,
        dehaze: 6.0,
        vibrance: 20.0,
        saturation: -3.0,
    };
    expected.tone_curve.parametric = ParametricCurve {
        highlights: -4.0,
        lights: 7.0,
        darks: 5.0,
        shadows: -10.0,
        split_shadows: 20.0,
        split_midtones: 55.0,
        split_highlights: 80.0,
    };
    expected.tone_curve.rgb =
        PointCurve::new([level(0, 0), level(64, 51), level(192, 204), level(255, 255)]);
    expected.tone_curve.green = PointCurve::new([level(0, 0), level(128, 140), level(255, 255)]);
    expected.color_mixer.hue[ColorBand::Blue.index()] = -20.0;
    expected.color_mixer.saturation[ColorBand::Orange.index()] = 12.0;
    expected.color_mixer.luminance[ColorBand::Green.index()] = 9.0;
    expected.color_mixer.bw_mix[ColorBand::Aqua.index()] = -7.0;
    expected.color_grading = ColorGrading {
        shadows: GradeWheel {
            hue: 220.0,
            saturation: 18.0,
            luminance: -6.0,
        },
        midtones: GradeWheel {
            hue: 30.0,
            saturation: 5.0,
            luminance: 2.0,
        },
        highlights: GradeWheel {
            hue: 45.0,
            saturation: 9.0,
            luminance: 4.0,
        },
        global: GradeWheel {
            hue: 140.0,
            saturation: 3.0,
            luminance: -1.0,
        },
        blending: 60.0,
        balance: -15.0,
    };
    expected.detail.sharpening = Sharpening {
        amount: 60.0,
        radius: 1.2,
        detail: 30.0,
        masking: 15.0,
    };
    expected.detail.noise_reduction = NoiseReduction {
        luminance: 22.0,
        luminance_detail: 55.0,
        luminance_contrast: 5.0,
        color: 30.0,
        color_detail: 45.0,
        color_smoothness: 60.0,
    };
    expected.lens = LensCorrections {
        distortion: 3.0,
        vignetting: 10.0,
        vignetting_midpoint: 40.0,
    };
    expected.transform = Transform {
        vertical: -8.0,
        horizontal: 2.0,
        rotate: -0.5,
        aspect: 4.0,
        scale: 102.0,
        x_offset: -1.5,
        y_offset: 0.5,
    };
    expected.effects.vignette = Vignette {
        amount: -25.0,
        midpoint: 45.0,
        roundness: 10.0,
        feather: 70.0,
        highlights: 20.0,
    };
    expected.effects.grain = Grain {
        amount: 12.0,
        size: 30.0,
        roughness: 65.0,
    };
    expected.calibration = Calibration {
        shadows_tint: -2.0,
        red_hue: 4.0,
        red_saturation: -3.0,
        green_hue: 1.0,
        green_saturation: 6.0,
        blue_hue: -5.0,
        blue_saturation: -6.0,
    };
    expected.crop = Crop {
        rect: Some(CropRect {
            cx: 0.5,
            cy: 0.5,
            width: 0.6,
            height: 0.8,
        }),
        ..Crop::default()
    };
    assert_eq!(develop, expected);
}

#[test]
fn an_element_form_sidecar_with_other_prefixes_reads_the_same_way() {
    let data = parse(ELEMENT_FORM, RAW).unwrap();
    assert_eq!(data.rating, Rating::UNRATED);
    assert_eq!(data.flag, Flag::Rejected);
    assert_eq!(data.label, ColorLabel::Purple);
    assert_eq!(data.title, "Morning");
    assert_eq!(data.caption, "Fog & frost <morning>");
    assert_eq!(data.creators, ["Ada Example", "Ben Example"]);
    assert_eq!(data.copyright, "© 2023 Ada Example");
    assert_eq!(data.keywords, ["Zürich", "雪"]);
    assert_eq!(
        data.hierarchical_keywords,
        [vec!["Places", "Europe", "Zürich"], vec!["Weather", "Snow"]]
    );
    assert_eq!(data.capture_time, DateTime::new(2023, 12, 24, 7, 15, 0));

    let mut expected = DevelopSettings::default_for(FileKind::Raw);
    expected.white_balance = WhiteBalance {
        preset: WbPreset::Daylight,
        temp: 5500.0,
        tint: 10.0,
    };
    expected.tone.exposure = -1.25;
    expected.tone.contrast = 30.0;
    expected.treatment = Treatment::BlackAndWhite;
    expected.color_mixer.bw_mix[ColorBand::Red.index()] = -20.0;
    expected.tone_curve.blue = PointCurve::new([level(0, 10), level(255, 245)]);
    assert_eq!(data.develop, Some(expected));
}

#[test]
fn a_bare_rdf_document_without_the_packet_wrapper_is_accepted() {
    let xml = r#"<?xml version="1.0"?>
<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#">
 <rdf:Description rdf:about="" xmlns:xmp="http://ns.adobe.com/xap/1.0/" xmp:Rating="5"/>
</rdf:RDF>"#;
    let data = parse(xml, RASTER).unwrap();
    assert_eq!(data.rating, Rating::new(5));
    assert_eq!(data.develop, None);
}

#[test]
fn a_packet_without_properties_reads_as_nothing() {
    let data = parse(r#"<x:xmpmeta xmlns:x="adobe:ns:meta/"/>"#, RASTER).unwrap();
    assert_eq!(data, photo_xmp::XmpData::default());
}

#[test]
fn raster_white_balance_uses_the_relative_properties() {
    let xml = packet(
        r#"crs:WhiteBalance="Custom" crs:IncrementalTemperature="+18" crs:IncrementalTint="-4" crs:Temperature="5600" crs:Tint="+30""#,
    );
    let raster = parse(&xml, RASTER).unwrap().develop.unwrap();
    assert_eq!(
        raster.white_balance,
        WhiteBalance {
            preset: WbPreset::Custom,
            temp: 18.0,
            tint: -4.0
        }
    );
    let raw = parse(&xml, RAW).unwrap().develop.unwrap();
    assert_eq!(
        raw.white_balance,
        WhiteBalance {
            preset: WbPreset::Custom,
            temp: 5600.0,
            tint: 30.0
        }
    );
}

#[test]
fn as_shot_white_balance_ignores_the_numbers_beside_it() {
    let xml = packet(r#"crs:WhiteBalance="As Shot" crs:Temperature="5200" crs:Tint="+8""#);
    let develop = parse(&xml, RAW).unwrap().develop.unwrap();
    assert_eq!(develop.white_balance, WhiteBalance::default());
    assert!(develop.is_default_for(FileKind::Raw));
}

#[test]
fn everything_read_is_sanitised_for_the_kind_of_file() {
    let xml = packet(
        r#"crs:Exposure2012="+40" crs:Clarity2012="-900" crs:SharpenRadius="9" crs:Sharpness="inf" crs:WhiteBalance="Custom" crs:IncrementalTemperature="5600""#,
    );
    let develop = parse(&xml, RASTER).unwrap().develop.unwrap();
    assert_eq!(develop.tone.exposure, 5.0);
    assert_eq!(develop.presence.clarity, -100.0);
    assert_eq!(develop.detail.sharpening.radius, 3.0);
    assert_eq!(develop.detail.sharpening.amount, 0.0);
    assert_eq!(develop.white_balance, WhiteBalance::default());
}

#[test]
fn values_that_do_not_parse_are_ignored_rather_than_fatal() {
    let xml = packet(
        r#"xmp:Rating="lots" xmp:Label="Review later" crs:Contrast2012="high" crs:Shadows2012="+12""#,
    );
    let data = parse(&xml, RAW).unwrap();
    assert_eq!(data.rating, Rating::UNRATED);
    assert_eq!(data.label, ColorLabel::None);
    let develop = data.develop.unwrap();
    assert_eq!(develop.tone.contrast, 0.0);
    assert_eq!(develop.tone.shadows, 12.0);
}

#[test]
fn list_and_text_properties_written_as_plain_values_are_still_read() {
    let data = parse(
        &packet(r#"dc:subject="lighthouse" dc:title="Plain title""#),
        RAW,
    )
    .unwrap();
    assert_eq!(data.keywords, ["lighthouse"]);
    assert_eq!(data.title, "Plain title");
}

#[test]
fn a_language_alternative_without_a_default_falls_back_to_its_first_item() {
    let xml = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:dc="http://purl.org/dc/elements/1.1/">
<rdf:Description rdf:about=""><dc:title><rdf:Alt><rdf:li xml:lang="fr">Port</rdf:li><rdf:li xml:lang="de">Hafen</rdf:li></rdf:Alt></dc:title></rdf:Description>
</rdf:RDF>"#;
    assert_eq!(parse(xml, RAW).unwrap().title, "Port");
}

#[test]
fn character_data_sections_and_references_are_decoded() {
    let xml = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:dc="http://purl.org/dc/elements/1.1/">
<rdf:Description rdf:about="" dc:rights="a&#10;b &#x263A; &quot;q&quot;"><dc:title><rdf:Alt><rdf:li xml:lang="x-default">A <![CDATA[<b> & c]]> &#169; &apos;d&apos;</rdf:li></rdf:Alt></dc:title></rdf:Description>
</rdf:RDF>"#;
    let data = parse(xml, RAW).unwrap();
    assert_eq!(data.title, "A <b> & c © 'd'");
    assert_eq!(data.copyright, "a\nb ☺ \"q\"");
}

#[test]
fn properties_nested_inside_other_structures_are_not_mistaken_for_top_level_ones() {
    let xml = r#"<rdf:RDF xmlns:rdf="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xmlns:crs="http://ns.adobe.com/camera-raw-settings/1.0/" xmlns:xmp="http://ns.adobe.com/xap/1.0/">
<rdf:Description rdf:about=""><crs:Look><rdf:Description xmp:Rating="5" crs:Exposure2012="+3"/></crs:Look></rdf:Description>
</rdf:RDF>"#;
    let data = parse(xml, RAW).unwrap();
    assert_eq!(data.rating, Rating::UNRATED);
    assert_eq!(data.develop, None);
}

#[test]
fn every_truncation_of_a_sidecar_is_an_error_and_never_a_panic() {
    for fixture in [ATTRIBUTE_FORM, ELEMENT_FORM] {
        let complete = fixture.rfind("xmpmeta>").unwrap() + "xmpmeta>".len();
        for cut in (0..complete).filter(|cut| fixture.is_char_boundary(*cut)) {
            let result = parse(&fixture[..cut], RAW);
            assert!(is_malformed(result), "cut at byte {cut} was accepted");
        }
        assert!(parse(&fixture[..complete], RAW).is_ok());
    }
}

#[test]
fn an_empty_or_blank_file_is_an_error() {
    assert!(is_malformed(parse("", RAW)));
    assert!(is_malformed(parse(" \n\t ", RAW)));
    assert!(is_malformed(parse("\u{feff}", RAW)));
}

#[test]
fn malformed_xml_is_an_error() {
    let cases = [
        "this is not xml",
        "<a><b></a></b>",
        "<a></a><b></b>",
        "<a></a> trailing text",
        "<a b=c></a>",
        r#"<a b="1" b="2"/>"#,
        "<a>&nosuchentity;</a>",
        r#"<a b="&nosuchentity;"/>"#,
        "<a>&#0;</a>",
        "<a>unterminated &amp</a>",
        "<a><!-- never closed </a>",
        "<a><![CDATA[never closed</a>",
        "</a>",
        "<></>",
        r#"<?xml version="1.0" encoding="ISO-8859-1"?><a/>"#,
    ];
    for case in cases {
        assert!(is_malformed(parse(case, RAW)), "{case:?} was accepted");
    }
}

#[test]
fn a_name_whose_prefix_is_never_declared_is_malformed() {
    let cases = [
        "<rdf:RDF/>",
        r#"<a xmp:Rating="3"/>"#,
        r#"<a><b xmlns:p="urn:p"/><p:c/></a>"#,
        r#"<a xmlns:p="urn:p"><b xmlns:p=""><p:c/></b></a>"#,
    ];
    for case in cases {
        assert!(is_malformed(parse(case, RAW)), "{case:?} was accepted");
    }
    // `xml:` needs no declaration, and a declaration covers its own element.
    let declared = r#"<p:RDF xmlns:p="http://www.w3.org/1999/02/22-rdf-syntax-ns#" xml:lang="en" p:about=""/>"#;
    assert!(parse(declared, RAW).is_ok());
}

#[test]
fn hostile_nesting_is_refused_instead_of_exhausting_the_stack() {
    let depth = 100_000;
    let xml = format!("{}{}", "<a>".repeat(depth), "</a>".repeat(depth));
    assert!(is_malformed(parse(&xml, RAW)));
}

#[test]
fn well_formed_xml_that_is_not_xmp_is_its_own_error() {
    assert!(matches!(
        parse("<html><body/></html>", RAW),
        Err(XmpError::NotXmp)
    ));
    assert!(matches!(
        parse(r#"<svg xmlns="http://www.w3.org/2000/svg"/>"#, RAW),
        Err(XmpError::NotXmp)
    ));
}

#[test]
fn errors_say_where_the_problem_is() {
    let error = parse("<a>\n<b></c></a>", RAW).unwrap_err();
    let XmpError::Malformed { offset, message } = &error else {
        panic!("unexpected error {error:?}");
    };
    assert_eq!(*offset, 7);
    assert!(message.contains("</c>"), "{message}");
    assert!(error.to_string().contains("byte 7"), "{error}");
}
