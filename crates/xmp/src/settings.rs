//! The private settings: `photo:Settings`, the complete develop settings as
//! JSON, and `photo:SettingsFormat`, the format that JSON is in
//! ([`SETTINGS_FORMAT`] when this build wrote it).
//!
//! Settings in a newer format than this build's are left alone: they are not
//! read, because this build would misread them, nor replaced by what the
//! Camera Raw properties beside them say, because a later write would then
//! make that the photo's settings for good. Writing develop settings over
//! them is refused.

use photo_core::develop::{DevelopSettings, SETTINGS_FORMAT};

use crate::rdf::{Form, PHOTO, Packet};
use crate::{ImageInfo, XmpError, crs};

/// The format of settings whose format is not given: the first, in which the
/// builds wrote that did not yet give it.
const FIRST_FORMAT: u32 = 1;

/// The develop settings a packet holds, sanitised for the kind of file: the
/// private settings when they are present and parse, else the Camera Raw
/// mapping. When the private settings are in a newer format, none, and that
/// format.
pub(crate) fn read(packet: &Packet, image: ImageInfo) -> (Option<DevelopSettings>, Option<u32>) {
    if let Some(json) = packet.text(PHOTO.property("Settings")) {
        let format = format_of(packet);
        if format > SETTINGS_FORMAT {
            return (None, Some(format));
        }
        if let Ok(mut settings) = serde_json::from_str::<DevelopSettings>(&json) {
            settings.sanitize_for(image.kind);
            return (Some(settings), None);
        }
    }
    let settings = crs::read(packet, image).map(|mut settings| {
        settings.sanitize_for(image.kind);
        settings
    });
    (settings, None)
}

/// The JSON for `settings`, once it is known that the packet may take them:
/// that it holds no settings of a newer format, and that the settings read
/// back from the JSON as they are. A number that is not finite does not:
/// JSON cannot write one, and the encoder writes `null` in its place.
pub(crate) fn encoded(packet: &Packet, settings: &DevelopSettings) -> Result<String, XmpError> {
    if packet.text(PHOTO.property("Settings")).is_some() {
        let format = format_of(packet);
        if format > SETTINGS_FORMAT {
            return Err(XmpError::NewerSettings { format });
        }
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
