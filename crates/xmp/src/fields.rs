//! [`XmpData`] to and from the properties of a packet.
//!
//! Writing compares each field with what the packet already says and touches
//! only the properties that differ, so a value another application wrote
//! keeps its exact spelling until the value itself changes.

use photo_core::develop::DevelopSettings;
use photo_core::meta::{ColorLabel, Flag, Rating};
use photo_core::time::DateTime;

use crate::rdf::{Container, DC, EXIF, Form, Item, LR, PHOTO, Packet, Property, XMP};
use crate::values::parse_number;
use crate::{ImageInfo, XmpData, XmpError, crs};

/// What separates the levels of a hierarchical keyword.
const LEVEL_SEPARATOR: char = '|';

/// Everything a packet says, in this application's terms.
pub(crate) fn read(packet: &Packet, image: ImageInfo) -> XmpData {
    XmpData {
        develop: read_develop(packet, image),
        ..read_metadata(packet)
    }
}

/// The descriptive part of a packet, without develop settings.
fn read_metadata(packet: &Packet) -> XmpData {
    let (rating, flag) = match packet
        .text(XMP.property("Rating"))
        .and_then(|text| parse_number::<f32>(&text))
    {
        Some(value) if value < 0.0 => (Rating::UNRATED, Flag::Rejected),
        // The cast saturates, and `Rating::new` clamps to five stars.
        Some(value) => (Rating::new(value.round() as u8), Flag::Unflagged),
        None => (Rating::UNRATED, Flag::Unflagged),
    };
    let label = packet.text(XMP.property("Label"));
    let hierarchical = packet
        .list(LR.property("hierarchicalSubject"))
        .unwrap_or_default();
    XmpData {
        rating,
        flag,
        label: label.map_or(ColorLabel::None, |name| ColorLabel::from_name(&name)),
        title: packet.alternative(DC.property("title")).unwrap_or_default(),
        caption: packet
            .alternative(DC.property("description"))
            .unwrap_or_default(),
        creators: non_empty(packet.list(DC.property("creator")).unwrap_or_default()),
        copyright: packet
            .alternative(DC.property("rights"))
            .unwrap_or_default(),
        keywords: non_empty(packet.list(DC.property("subject")).unwrap_or_default()),
        hierarchical_keywords: hierarchical
            .iter()
            .map(|path| non_empty(path.split(LEVEL_SEPARATOR).map(str::to_owned)))
            .filter(|path| !path.is_empty())
            .collect(),
        capture_time: [
            EXIF.property("DateTimeOriginal"),
            XMP.property("CreateDate"),
        ]
        .into_iter()
        .find_map(|property| parse_date(&packet.text(property)?)),
        develop: None,
    }
}

/// The private settings when they are present and parse, else the Camera Raw
/// mapping; either way sanitised for the kind of file.
fn read_develop(packet: &Packet, image: ImageInfo) -> Option<DevelopSettings> {
    let own = packet
        .text(PHOTO.property("Settings"))
        .and_then(|json| serde_json::from_str::<DevelopSettings>(&json).ok());
    let mut settings = own.or_else(|| crs::read(packet, image))?;
    settings.sanitize_for(image.kind);
    Some(settings)
}

/// Makes the packet say what `data` says.
pub(crate) fn write(packet: &mut Packet, data: &XmpData, image: ImageInfo) -> Result<(), XmpError> {
    let current = read_metadata(packet);

    let rating = rating_value(data.rating, data.flag);
    if rating != rating_value(current.rating, current.flag) {
        packet.set_text(XMP.property("Rating"), &rating.to_string(), Form::Attribute);
    }
    // A label read as "none" may be a custom name this application has no
    // colour for; it stays until a colour replaces it.
    if data.label != current.label {
        match data.label {
            ColorLabel::None => packet.remove(XMP.property("Label")),
            label => packet.set_text(XMP.property("Label"), label.name(), Form::Attribute),
        }
    }
    write_alternative(packet, DC.property("title"), &current.title, &data.title);
    write_alternative(
        packet,
        DC.property("description"),
        &current.caption,
        &data.caption,
    );
    write_alternative(
        packet,
        DC.property("rights"),
        &current.copyright,
        &data.copyright,
    );
    write_list(
        packet,
        DC.property("creator"),
        Container::Seq,
        &current.creators,
        &data.creators,
    );
    write_list(
        packet,
        DC.property("subject"),
        Container::Bag,
        &current.keywords,
        &data.keywords,
    );
    write_list(
        packet,
        LR.property("hierarchicalSubject"),
        Container::Bag,
        &joined(&current.hierarchical_keywords),
        &joined(&data.hierarchical_keywords),
    );
    if let Some(time) = data.capture_time
        && current.capture_time != Some(time)
    {
        let property = EXIF.property("DateTimeOriginal");
        packet.set_text(property, &time.to_string(), Form::Attribute);
    }

    if let Some(settings) = &data.develop {
        crs::write(packet, settings, image);
        let json = serde_json::to_string(settings)?;
        let property = PHOTO.property("Settings");
        if packet.text(property).as_deref() != Some(json.as_str()) {
            // An element, because quoting JSON inside an attribute would
            // escape every one of its quotation marks.
            packet.set_text(property, &json, Form::Element);
        }
    }
    Ok(())
}

/// `xmp:Rating` for a rating and flag: rejection is -1 and has no stars.
fn rating_value(rating: Rating, flag: Flag) -> i8 {
    match flag {
        Flag::Rejected => -1,
        // At most five, so the conversion cannot fail.
        Flag::Unflagged | Flag::Picked => i8::try_from(rating.stars()).unwrap_or(i8::MAX),
    }
}

fn write_alternative(packet: &mut Packet, property: Property<'_>, current: &str, wanted: &str) {
    if wanted == current {
        return;
    }
    if wanted.is_empty() {
        packet.remove(property);
    } else {
        packet.set_alternative(property, wanted);
    }
}

fn write_list(
    packet: &mut Packet,
    property: Property<'_>,
    container: Container,
    current: &[String],
    wanted: &[String],
) {
    let wanted = non_empty(wanted.iter().cloned());
    if wanted == current {
        return;
    }
    if wanted.is_empty() {
        packet.remove(property);
    } else {
        let items: Vec<Item> = wanted.into_iter().map(Item::plain).collect();
        packet.set_list(property, container, &items);
    }
}

fn non_empty(texts: impl IntoIterator<Item = String>) -> Vec<String> {
    texts.into_iter().filter(|text| !text.is_empty()).collect()
}

/// Hierarchical keywords as the strings a sidecar holds, one per path.
fn joined(paths: &[Vec<String>]) -> Vec<String> {
    let separator = LEVEL_SEPARATOR.to_string();
    paths
        .iter()
        .map(|path| non_empty(path.iter().cloned()).join(&separator))
        .collect()
}

/// An XMP date: the ISO form [`DateTime`] parses, which XMP also allows to
/// stop at the minute. The time is read as written, whatever zone follows.
fn parse_date(text: &str) -> Option<DateTime> {
    let text = text.trim();
    text.parse().ok().or_else(|| {
        let to_the_minute = text.get(..16)?;
        format!("{to_the_minute}:00").parse().ok()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_are_read_to_the_second_or_to_the_minute() {
        let second = DateTime::new(2024, 6, 1, 18, 22, 7);
        assert_eq!(parse_date("2024-06-01T18:22:07"), second);
        assert_eq!(parse_date(" 2024-06-01T18:22:07.31+02:00 "), second);
        assert_eq!(parse_date("2024:06:01 18:22:07"), second);
        let minute = DateTime::new(2023, 12, 24, 7, 15, 0);
        assert_eq!(parse_date("2023-12-24T07:15"), minute);
        assert_eq!(parse_date("2023-12-24T07:15+01:00"), minute);
        assert_eq!(parse_date("2023-12-24T07:15Z"), minute);
        for text in [
            "",
            "2023",
            "2023-12",
            "2023-12-24",
            "2023-12-24T07",
            "2023-13-24T07:15",
            "yesterday at 7:15 am",
            "日本語の日付と時刻です。",
        ] {
            assert_eq!(parse_date(text), None, "{text:?}");
        }
    }

    #[test]
    fn rejection_is_minus_one_whatever_the_stars() {
        assert_eq!(rating_value(Rating::new(4), Flag::Rejected), -1);
        assert_eq!(rating_value(Rating::new(4), Flag::Picked), 4);
        assert_eq!(rating_value(Rating::UNRATED, Flag::Unflagged), 0);
    }

    #[test]
    fn hierarchical_keywords_join_their_levels_and_skip_empty_ones() {
        let paths = vec![
            vec!["A".to_owned(), String::new(), "B".to_owned()],
            vec![String::new()],
            vec!["C".to_owned()],
        ];
        assert_eq!(joined(&paths), ["A|B", "", "C"]);
    }
}
