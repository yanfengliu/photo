//! The private settings: `photo:Settings`, the complete develop settings as
//! JSON, and `photo:SettingsFormat`, the format that JSON is in
//! ([`SETTINGS_FORMAT`] when this build wrote it).
//!
//! Private settings this build cannot read are left alone, whatever format
//! they claim and whatever form they take: of a newer format, not JSON of
//! this build's shape, or not text at all. They are not read, because this
//! build would misread them, nor replaced by what the Camera Raw properties
//! beside them say, because a later write would then make that the photo's
//! settings for good. Writing develop settings over them is refused.

use photo_core::develop::{DevelopSettings, SETTINGS_FORMAT};

use crate::rdf::{Form, PHOTO, Packet};
use crate::{ImageInfo, XmpError, crs};

/// The format of settings whose format is not given: the first, in which the
/// builds wrote that did not yet give it.
const FIRST_FORMAT: u32 = 1;

/// What a packet's private settings are to this build.
enum Private {
    /// There are none.
    Absent,
    /// They read as these settings.
    Read(Box<DevelopSettings>),
    /// They are there, but this build cannot read them; they claim this
    /// format.
    Unreadable(u32),
}

fn private(packet: &Packet) -> Private {
    let property = PHOTO.property("Settings");
    if !packet.has(property) {
        return Private::Absent;
    }
    let format = format_of(packet);
    if format > SETTINGS_FORMAT {
        return Private::Unreadable(format);
    }
    let json = packet.text(property);
    match json.and_then(|json| serde_json::from_str::<DevelopSettings>(&json).ok()) {
        Some(settings) => Private::Read(Box::new(settings)),
        None => Private::Unreadable(format),
    }
}

/// The develop settings a packet holds, sanitised for the kind of file: the
/// private settings when it has them, else the Camera Raw mapping. When the
/// private settings are there but this build cannot read them, none, and
/// the format they claim.
pub(crate) fn read(packet: &Packet, image: ImageInfo) -> (Option<DevelopSettings>, Option<u32>) {
    let settings = match private(packet) {
        Private::Read(settings) => Some(*settings),
        Private::Unreadable(format) => return (None, Some(format)),
        Private::Absent => crs::read(packet, image),
    };
    let settings = settings.map(|mut settings| {
        settings.sanitize_for(image.kind);
        settings
    });
    (settings, None)
}

/// The JSON for `settings`, once it is known that the packet may take them:
/// that it holds no private settings this build cannot read, and that the
/// settings read back from the JSON as they are. A number that is not
/// finite does not: JSON cannot write one, and the encoder writes `null` in
/// its place.
pub(crate) fn encoded(packet: &Packet, settings: &DevelopSettings) -> Result<String, XmpError> {
    if let Private::Unreadable(format) = private(packet) {
        return Err(XmpError::UnreadableSettings { format });
    }
    let json = serde_json::to_string(settings)?;
    // Only a `null` where a number belongs keeps this build's own JSON from
    // reading back.
    if serde_json::from_str::<DevelopSettings>(&json).is_err() {
        return Err(XmpError::NotFinite);
    }
    Ok(json)
}

/// Writes `json`, from [`encoded`], and the format it is in, unless the
/// packet already says both.
pub(crate) fn write(packet: &mut Packet, json: &str) {
    let property = PHOTO.property("Settings");
    if packet.text(property).as_deref() != Some(json) {
        // An element, because quoting JSON inside an attribute would escape
        // every one of its quotation marks.
        packet.set_text(property, json, Form::Element);
    }
    let format = PHOTO.property("SettingsFormat");
    let written = SETTINGS_FORMAT.to_string();
    if packet.text(format).as_deref() != Some(written.as_str()) {
        packet.set_text(format, &written, Form::Attribute);
    }
}

/// The format `photo:SettingsFormat` gives. Absent, or not a whole number,
/// it counts as the first format; a whole number too large to hold counts
/// as the newest there could be.
fn format_of(packet: &Packet) -> u32 {
    let Some(text) = packet.text(PHOTO.property("SettingsFormat")) else {
        return FIRST_FORMAT;
    };
    let text = text.trim();
    let digits = text.strip_prefix('+').unwrap_or(text);
    if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
        return FIRST_FORMAT;
    }
    digits.parse().unwrap_or(u32::MAX)
}
