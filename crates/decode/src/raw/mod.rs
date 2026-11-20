//! Camera RAW files through rawler.
//!
//! rawler finds the decoder for a file and unpacks the sensor data and the
//! metadata; everything after that (levels, demosaicing, cropping, turning
//! upright) is driven from [`develop`]. Bayer and four-colour mosaics are
//! interpolated by rawler; level scaling and X-Trans interpolation are done
//! here ([`levels`], [`xtrans`]).

mod color;
mod develop;
mod frame;
mod levels;
mod meta;
mod xtrans;

use std::path::Path;

use image::DynamicImage;
use photo_core::colorimetry::RgbSpace;
use photo_core::image::Rgba8Image;
use photo_core::meta::PhotoMetadata;
use rawler::decoders::{Decoder, FormatHint, RawDecodeParams, WellKnownIFD};
use rawler::rawsource::RawSource;

use self::frame::Frame;
use self::meta::CameraNames;
use crate::Decoded;
use crate::error::DecodeError;
use crate::orient::Orientation;
use crate::preview::to_preview;

/// Maps the file and finds the decoder for it.
fn open(path: &Path) -> Result<RawSource, DecodeError> {
    if std::fs::metadata(path)?.len() == 0 {
        return Err(DecodeError::Corrupt("the file is empty".into()));
    }
    Ok(RawSource::new(path)?)
}

/// The geometry and camera names of a DNG, read from its directories.
///
/// rawler's DNG decoder unpacks every pixel even when asked only for the
/// description of the image, so for this one format the description is
/// taken from the tags directly, exactly as that decoder would.
fn describe_dng(
    decoder: &dyn Decoder,
) -> Result<(Frame, rawler::Orientation, CameraNames), DecodeError> {
    let missing = || DecodeError::Corrupt("the DNG has no raw image directory".into());
    let raw_ifd = decoder.ifd(WellKnownIFD::Raw)?.ok_or_else(missing)?;
    let root = decoder.ifd(WellKnownIFD::Root)?.ok_or_else(missing)?;
    let orientation = root
        .get_entry(rawler::tags::TiffCommonTag::Orientation)
        .and_then(|entry| entry.get_u16(0).ok().flatten())
        .map_or(rawler::Orientation::Unknown, rawler::Orientation::from_u16);
    Ok((
        Frame::of_dng(&raw_ifd)?,
        orientation,
        CameraNames::of_dng(decoder),
    ))
}

pub(crate) fn read_metadata(path: &Path) -> Result<PhotoMetadata, DecodeError> {
    let source = open(path)?;
    let decoder = rawler::get_decoder(&source)?;
    let facts = meta::raw_metadata(decoder.as_ref(), &source);
    let (frame, image_orientation, names) = if decoder.format_hint() == FormatHint::DNG {
        describe_dng(decoder.as_ref())?
    } else {
        // A "dummy" decode: everything about the image except its pixels.
        let raw = decoder.raw_image(&source, &RawDecodeParams::default(), true)?;
        let names = CameraNames {
            make: raw.make.clone(),
            model: raw.model.clone(),
        };
        (Frame::of(&raw)?, raw.orientation, names)
    };
    let exif_orientation = facts.as_ref().and_then(|facts| facts.exif.orientation);
    let orientation = frame::orientation(image_orientation, exif_orientation);
    let (width, height) = orientation.upright_size(frame.crop.width, frame.crop.height);
    Ok(meta::photo_metadata(
        width as u32,
        height as u32,
        &names,
        facts.as_ref(),
        path,
    ))
}

pub(crate) fn decode(path: &Path) -> Result<Decoded, DecodeError> {
    let source = open(path)?;
    let decoder = rawler::get_decoder(&source)?;
    if decoder.format_hint() == FormatHint::DNG {
        // Refuse impossible or enormous frames before rawler allocates them.
        describe_dng(decoder.as_ref())?;
    }
    let facts = meta::raw_metadata(decoder.as_ref(), &source);
    let baseline_exposure = meta::baseline_exposure(decoder.as_ref());
    let raw = decoder.raw_image(&source, &RawDecodeParams::default(), false)?;
    let names = CameraNames {
        make: raw.make.clone(),
        model: raw.model.clone(),
    };
    let exif_orientation = facts.as_ref().and_then(|facts| facts.exif.orientation);
    let orientation = frame::orientation(raw.orientation, exif_orientation);

    let image = develop::develop(raw, orientation, baseline_exposure)?;
    let metadata = meta::photo_metadata(image.width, image.height, &names, facts.as_ref(), path);
    Ok(Decoded { image, metadata })
}

/// The picture a raw file carries for quick display: the preview rawler
/// exposes for the format (for most cameras the largest embedded JPEG),
/// else the thumbnail, else a full-size rendition if the format has one.
///
/// An embedded picture that fails to decode counts as absent: the caller's
/// fallback is to wait for the real decode either way.
pub(crate) fn embedded_preview(
    path: &Path,
    max_edge: u32,
) -> Result<Option<Rgba8Image>, DecodeError> {
    type Extract =
        fn(&dyn Decoder, &RawSource, &RawDecodeParams) -> rawler::Result<Option<DynamicImage>>;
    const SOURCES: [(&str, Extract); 3] = [
        ("preview", |decoder, source, params| {
            decoder.preview_image(source, params)
        }),
        ("thumbnail", |decoder, source, params| {
            decoder.thumbnail_image(source, params)
        }),
        ("full image", |decoder, source, params| {
            decoder.full_image(source, params)
        }),
    ];

    let source = open(path)?;
    let decoder = rawler::get_decoder(&source)?;
    let params = RawDecodeParams::default();
    let embedded = SOURCES.iter().find_map(|(kind, extract)| {
        let extracted = crate::error::guard(|| Ok(extract(decoder.as_ref(), &source, &params)?));
        match extracted {
            Ok(Some(image)) if image.width() > 0 && image.height() > 0 => Some(image),
            Ok(_) => None,
            Err(error) => {
                log::warn!("{}: embedded {kind} unreadable ({error})", path.display());
                None
            }
        }
    });
    let Some(embedded) = embedded else {
        return Ok(None);
    };
    // Embedded pictures are stored the way the sensor data is, so the raw
    // orientation applies to them. Cameras render them in sRGB or Adobe RGB
    // and do not say which; sRGB is the assumption.
    let facts = meta::raw_metadata(decoder.as_ref(), &source);
    let exif_orientation = facts.and_then(|facts| facts.exif.orientation);
    let orientation = Orientation::from_exif(u32::from(exif_orientation.unwrap_or(1)));
    Ok(Some(to_preview(
        embedded,
        orientation,
        RgbSpace::Srgb,
        max_edge,
    )))
}
