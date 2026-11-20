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

/// How a packet marks a photo: the two properties its stars and its flag
/// are read from.
struct Marks {
    /// `xmp:Rating`, when it holds a finite number.
    rating: Option<f32>,
    /// `photo:Flag`, when it holds a flag.
    flag: Option<Flag>,
}

impl Marks {
    fn of(packet: &Packet) -> Self {
        let rating = packet.text(XMP.property("Rating"));
        let flag = packet.text(PHOTO.property("Flag"));
        Self {
            rating: rating.and_then(|text| parse_number(&text)),
            flag: flag.and_then(|text| parse_flag(&text)),
        }
    }

    /// Whether the rating says "rejected". The XMP specification writes
    /// that as -1 and allows no other negative value, so every negative
    /// number is taken to mean it.
    fn rejects(&self) -> bool {
        self.rating.is_some_and(|rating| rating < 0.0)
    }

    /// The stars: the rating to the nearest whole number, five at most. A
    /// rating that rejects has none.
    fn stars(&self) -> Rating {
        match self.rating {
            // The cast saturates, and `Rating::new` clamps to five stars.
            Some(rating) if !self.rejects() => Rating::new(rating.round() as u8),
            _ => Rating::UNRATED,
        }
    }

    /// The flag: the private property when it is there, else whatever the
    /// rating says.
    fn flag(&self) -> Flag {
        let by_rating = if self.rejects() {
            Flag::Rejected
        } else {
            Flag::Unflagged
        };
        self.flag.unwrap_or(by_rating)
    }
}

/// The flag a `photo:Flag` value stands for: one of the whole numbers
/// [`Flag::to_i8`] gives. Any other value stands for none.
fn parse_flag(text: &str) -> Option<Flag> {
    let value: i8 = text.trim().parse().ok()?;
    [Flag::Rejected, Flag::Unflagged, Flag::Picked]
        .into_iter()
        .find(|flag| flag.to_i8() == value)
}

/// The descriptive part of a packet, without develop settings.
fn read_metadata(packet: &Packet) -> XmpData {
    let marks = Marks::of(packet);
    let label = packet.text(XMP.property("Label"));
    let hierarchical = packet
        .list(LR.property("hierarchicalSubject"))
        .unwrap_or_default();
    XmpData {
        rating: marks.stars(),
        flag: marks.flag(),
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

    write_marks(packet, data.rating, data.flag);
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

/// Makes the packet give these stars and this flag.
///
/// The stars are `xmp:Rating` whatever the flag is. A pick or a rejection
/// is `photo:Flag`, and an unflagged photo has no such property. A packet
/// that already reads this way is left alone, so a rejection another
/// application wrote as a rating of -1 stays as it is until the stars or
/// the flag change; this crate itself never writes that rating.
fn write_marks(packet: &mut Packet, rating: Rating, flag: Flag) {
    let current = Marks::of(packet);
    if (current.stars(), current.flag()) == (rating, flag) {
        return;
    }
    if current.rejects() || current.stars() != rating {
        let stars = rating.stars().to_string();
        packet.set_text(XMP.property("Rating"), &stars, Form::Attribute);
    }
    let property = PHOTO.property("Flag");
    match flag {
        Flag::Unflagged => packet.remove(property),
        Flag::Picked | Flag::Rejected if current.flag == Some(flag) => {}
        Flag::Picked | Flag::Rejected => {
            packet.set_text(property, &flag.to_i8().to_string(), Form::Attribute);
        }
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
    fn a_flag_is_one_of_three_whole_numbers() {
        for flag in [Flag::Rejected, Flag::Unflagged, Flag::Picked] {
            assert_eq!(parse_flag(&flag.to_i8().to_string()), Some(flag));
        }
        assert_eq!(parse_flag(" +1\n"), Some(Flag::Picked));
        for text in ["", "2", "-2", "1.0", "one", "picked", "1 1"] {
            assert_eq!(parse_flag(text), None, "{text:?}");
        }
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
