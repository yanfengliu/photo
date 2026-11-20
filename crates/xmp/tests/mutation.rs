//! Sidecars altered in arbitrary ways are either refused or handled like any
//! other sidecar: never a panic, and never an update that does not hold.

use photo_core::develop::{
    Crop, CropConstraint, CropRect, CurvePoint, DevelopSettings, Orientation, PointCurve, Profile,
    Treatment,
};
use photo_core::meta::FileKind;
use photo_xmp::{ImageInfo, XmpData, parse, to_string};

const FIXTURES: [&str; 3] = [
    include_str!("fixtures/attribute_form.xmp"),
    include_str!("fixtures/element_form.xmp"),
    include_str!("fixtures/foreign.xmp"),
];

/// Characters XML gives meaning to, spliced in anywhere.
const NOISE: [&str; 18] = [
    "<", ">", "\"", "'", "&", "/", "=", " ", ":", ";", "\n", "|", "&amp;", "&#10;", "]]>", "<!--",
    "-->", "\u{feff}",
];

/// Attributes this crate reads, spliced in after a quotation mark.
const ATTRIBUTES: [&str; 14] = [
    " xmp:Rating=\"7\"",
    " xmp:Label=\"Blue\"",
    " dc:title=\"loose\"",
    " crs:HasCrop=\"True\"",
    " crs:CropAngle=\"12\" crs:CropLeft=\"0.1\" crs:CropTop=\"0.2\" crs:CropRight=\"0.7\" crs:CropBottom=\"0.9\"",
    " crs:CropRight=\"-1\"",
    " crs:WhiteBalance=\"Auto\"",
    " crs:Temperature=\"4000\" crs:IncrementalTemperature=\"-30\"",
    " crs:Exposure2012=\"1e9\"",
    " tiff:Orientation=\"7\"",
    " tiff:ImageWidth=\"300\" tiff:ImageLength=\"200\"",
    " xmlns:crs=\"urn:other\"",
    " xmlns:tiff=\"http://ns.adobe.com/tiff/1.0/\" xmlns:xmp=\"http://ns.adobe.com/xap/1.0/\"",
    " xml:lang=\"x-default\"",
];

/// Elements this crate reads, spliced in before a tag.
const ELEMENTS: [&str; 10] = [
    "<xmp:Rating>2</xmp:Rating>",
    "<dc:title>plain</dc:title>",
    "<dc:subject><rdf:Bag><rdf:li>extra</rdf:li><rdf:li/></rdf:Bag></dc:subject>",
    "<crs:ToneCurvePV2012><rdf:Seq><rdf:li>3, 4</rdf:li><rdf:li>200, 9</rdf:li></rdf:Seq></crs:ToneCurvePV2012>",
    "<crs:Exposure2012>-2</crs:Exposure2012>",
    "<photo:Settings xmlns:photo=\"http://ns.photo.invalid/1.0/\">{\"tone\":{\"exposure\":2}}</photo:Settings>",
    "<photo:Settings xmlns:photo=\"http://ns.photo.invalid/1.0/\">not json</photo:Settings>",
    "<rdf:Description rdf:about=\"\" xmlns:dc=\"http://purl.org/dc/elements/1.1/\" dc:rights=\"mine\"/>",
    "<rdf:li>stray</rdf:li>",
    "</rdf:Description><rdf:Description>",
];

/// A small deterministic generator, so that a failure can be reproduced.
struct Random(u64);

impl Random {
    fn below(&mut self, bound: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) as usize) % bound.max(1)
    }

    fn pick<'a>(&mut self, choices: &[&'a str]) -> &'a str {
        choices[self.below(choices.len())]
    }

    /// A position in `text` that does not split a character.
    fn position(&mut self, text: &str) -> usize {
        let mut position = self.below(text.len() + 1);
        while !text.is_char_boundary(position) {
            position -= 1;
        }
        position
    }

    /// The position of one of the places `mark` occurs in `text` (`skip`
    /// bytes further on), or any position if it does not occur.
    fn occurrence(&mut self, text: &str, mark: char, skip: usize) -> usize {
        let places: Vec<usize> = text
            .match_indices(mark)
            .map(|(position, _)| position)
            .collect();
        match places.is_empty() {
            true => self.position(text),
            false => places[self.below(places.len())] + skip,
        }
    }
}

fn spliced(text: &str, at: usize, piece: &str) -> String {
    format!("{}{piece}{}", &text[..at], &text[at..])
}

/// `text` with one random edit: a piece cut out, repeated or moved, or
/// something spliced in.
fn mutated(text: &str, random: &mut Random) -> String {
    let (mut start, mut end) = (random.position(text), random.position(text));
    if start > end {
        (start, end) = (end, start);
    }
    while end - start > 60 {
        end = random.position(&text[..end]).max(start);
    }
    let piece = &text[start..end];
    let without = format!("{}{}", &text[..start], &text[end..]);
    match random.below(6) {
        0 => without,
        1 => spliced(text, start, piece),
        2 => spliced(&without, random.position(&without), piece),
        3 => spliced(text, start, random.pick(&NOISE)),
        4 => {
            let quote = if text.contains('"') { '"' } else { '\'' };
            spliced(
                text,
                random.occurrence(text, quote, 1),
                random.pick(&ATTRIBUTES),
            )
        }
        _ => spliced(
            text,
            random.occurrence(text, '<', 0),
            random.pick(&ELEMENTS),
        ),
    }
}

/// A curve point given on the 0 to 255 scale sidecars use.
fn level(x: u8, y: u8) -> CurvePoint {
    CurvePoint::new(f32::from(x) / 255.0, f32::from(y) / 255.0)
}

/// `xml` without the element holding the private settings, or `None` when
/// they are not held in an element of the usual name.
fn without_private_settings(xml: &str) -> Option<String> {
    let close = "</photo:Settings>";
    let start = xml.find("<photo:Settings")?;
    let end = start + xml[start..].find(close)? + close.len();
    Some(format!("{}{}", &xml[..start], &xml[end..]))
}

#[test]
fn altered_sidecars_are_refused_or_updated_soundly() {
    let images = [
        ImageInfo::new(FileKind::Raw),
        ImageInfo::new(FileKind::Raster)
            .with_orientation(Orientation::new(1, false))
            .with_size(4000, 6000),
        ImageInfo::new(FileKind::Raw).with_size(6000, 4000),
        ImageInfo::new(FileKind::Raw).with_orientation(Orientation::new(2, true)),
    ];
    let mut random = Random(0x5eed);
    let (mut accepted, mut refused) = (0, 0);
    for round in 0..1_200 {
        let mut text = FIXTURES[round % FIXTURES.len()].to_owned();
        for _ in 0..=random.below(3) {
            text = mutated(&text, &mut random);
        }
        let image = images[round % images.len()];
        let Ok(read) = parse(&text, image) else {
            refused += 1;
            assert!(
                to_string(&XmpData::default(), image, Some(&text)).is_err(),
                "{text}"
            );
            continue;
        };
        accepted += 1;

        // Writing back what was read, without develop settings, is not a change.
        let metadata = XmpData {
            develop: None,
            ..read.clone()
        };
        let untouched = to_string(&metadata, image, Some(&text)).unwrap();
        assert_eq!(parse(&untouched, image).unwrap(), read, "{text}");
        assert_eq!(
            to_string(&metadata, image, Some(&untouched)).unwrap(),
            untouched,
            "{text}"
        );

        // Writing everything back holds, and holds still.
        let written = to_string(&read, image, Some(&text)).unwrap();
        assert_eq!(parse(&written, image).unwrap(), read, "{text}");
        assert_eq!(
            to_string(&read, image, Some(&written)).unwrap(),
            written,
            "{text}"
        );

        // So does a change to what this crate owns, the develop settings
        // included: through the private settings, and through the Camera Raw
        // properties alone once those are taken away.
        let mut develop = read
            .develop
            .clone()
            .unwrap_or_else(|| DevelopSettings::default_for(image.kind));
        develop.tone.exposure = 1.5;
        develop.treatment = Treatment::BlackAndWhite;
        develop.orientation = develop.orientation.rotated_cw();
        develop.crop = Crop {
            rect: Some(CropRect {
                cx: 0.4,
                cy: 0.6,
                width: 0.5,
                height: 0.3,
            }),
            angle: -8.5,
            ..Crop::default()
        };
        develop.tone_curve.red = PointCurve::new([level(0, 30), level(200, 255)]);
        let changed = XmpData {
            rating: read.rating.increased().decreased().decreased(),
            title: format!("{} <&>", read.title),
            keywords: vec!["one".to_owned(), "two & three".to_owned()],
            develop: Some(develop.clone()),
            ..read
        };
        let rewritten = to_string(&changed, image, Some(&written)).unwrap();
        assert_eq!(parse(&rewritten, image).unwrap(), changed, "{text}");
        if image.orientation.is_some()
            && let Some(mirror) = without_private_settings(&rewritten)
        {
            let mapped = DevelopSettings {
                profile: Profile::default(),
                crop: Crop {
                    constraint: CropConstraint::default(),
                    ..develop.crop
                },
                spots: Vec::new(),
                masks: Vec::new(),
                ..develop
            };
            assert_eq!(
                parse(&mirror, image).unwrap().develop,
                Some(mapped),
                "{text}"
            );
        }
    }
    // The edits are small enough to leave a good share of the files usable,
    // and rough enough to break the rest: both paths are exercised.
    assert!(
        accepted > 100 && refused > 100,
        "{accepted} accepted, {refused} refused"
    );
}
