//! Orientation and crop: the two settings a sidecar records relative to the
//! pixels as stored in the file, where [`DevelopSettings`] records them
//! relative to the upright picture the decoder hands over.
//!
//! The conventions followed, and where they come from, are set out in the
//! crate documentation under "Orientation and crop". This module works out
//! the stored frame (how it is turned, and its shape) and reads and writes
//! the two settings through it.

mod crop;
mod orientation;

use photo_core::develop::{DevelopSettings, Orientation};

use crate::ImageInfo;
use crate::rdf::{Form, Packet, TIFF, Values};
use crate::values::parse_number;

/// The frame shape assumed when neither the sidecar nor the caller gives
/// the image's size: the commonest shape of a camera sensor.
const ASSUMED_ASPECT: f64 = 3.0 / 2.0;

/// The frame of the stored pixels, which a sidecar measures a crop in.
#[derive(Clone, Copy, Debug, PartialEq)]
struct StoredFrame {
    /// The orientation that takes it to the frame the settings measure a
    /// crop in: the file's own, then the user's.
    to_upright: Orientation,
    /// Its width over its height.
    aspect: f64,
}

/// How the sidecar says the stored pixels are to be turned.
fn stored_orientation(packet: &Packet) -> Option<Orientation> {
    orientation::from_tiff(&packet.text(TIFF.property("Orientation"))?)
}

/// The stored frame of a picture the user has turned by `user`.
fn stored_frame(packet: &Packet, image: ImageInfo, user: Orientation) -> StoredFrame {
    // When the file's own orientation is not known, the sidecar's stands in
    // for it: that is what it is unless another application turned the
    // picture. With neither, the pixels are taken to be stored upright.
    let file = image.orientation.or_else(|| stored_orientation(packet));
    StoredFrame {
        to_upright: orientation::then(file.unwrap_or(Orientation::IDENTITY), user),
        aspect: stored_aspect(packet, image, file),
    }
}

/// The stored frame's width over its height, for a file whose own
/// orientation is `file`.
fn stored_aspect(packet: &Packet, image: ImageInfo, file: Option<Orientation>) -> f64 {
    let pixels = |name: &str| {
        let text = packet.text(TIFF.property(name))?;
        parse_number::<f64>(&text).filter(|pixels| *pixels > 0.0)
    };
    if let (Some(width), Some(height)) = (pixels("ImageWidth"), pixels("ImageLength")) {
        return width / height;
    }
    let Some((width, height)) = image
        .size
        .filter(|(width, height)| *width > 0 && *height > 0)
    else {
        return ASSUMED_ASPECT;
    };
    // The caller's size is that of the upright source: the stored frame has
    // its sides the other way round when the file's own orientation swaps
    // them.
    let (width, height) = (f64::from(width), f64::from(height));
    if file.is_some_and(Orientation::swaps_axes) {
        height / width
    } else {
        width / height
    }
}

/// Reads orientation and crop into `settings`. Returns whether the sidecar
/// had anything to say about either.
pub(crate) fn read(
    packet: &Packet,
    crs: &Values,
    image: ImageInfo,
    settings: &mut DevelopSettings,
) -> bool {
    let mut found = false;
    if let (Some(stored), Some(file)) = (stored_orientation(packet), image.orientation) {
        settings.orientation = orientation::then(orientation::inverse(file), stored);
        // A sidecar that only repeats the file's own orientation says nothing.
        found = settings.orientation != Orientation::IDENTITY;
    }
    let frame = stored_frame(packet, image, settings.orientation);
    if let Some(crop) = crop::read(crs, frame) {
        settings.crop = crop;
        found = true;
    }
    found
}

/// Writes orientation and crop. `crs` is the packet's Camera Raw properties
/// as they were before the write.
pub(crate) fn write(
    packet: &mut Packet,
    crs: &Values,
    image: ImageInfo,
    settings: &DevelopSettings,
) {
    if let Some(file) = image.orientation {
        let stored = orientation::then(file, settings.orientation);
        if stored_orientation(packet) != Some(stored) {
            let code = orientation::to_tiff(stored).to_string();
            packet.set_text(TIFF.property("Orientation"), &code, Form::Attribute);
        }
    }
    let frame = stored_frame(packet, image, settings.orientation);
    crop::write(packet, crs, &settings.crop, frame);
}
