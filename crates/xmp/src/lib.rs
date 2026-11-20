//! XMP sidecar reading and writing: ratings, labels, keywords, descriptive
//! metadata and develop settings.
//!
//! # What is read
//!
//! [`parse`] and [`read_sidecar`] accept the forms other applications
//! write: properties as attributes of `rdf:Description` or as child
//! elements, lists as `rdf:Bag`, `rdf:Seq` or `rdf:Alt`, any namespace
//! prefixes (names are resolved by namespace URI), with or without the
//! `<?xpacket?>` wrapper and the `x:xmpmeta` element. A value with
//! qualifiers beside it (XMP specification, part 1) is read from its
//! `rdf:value`, and an update changes only that. Properties that are not
//! understood are ignored; values that do not parse count as absent.
//!
//! # What is written
//!
//! [`to_string`] and [`write_sidecar`] update an existing sidecar in place.
//! Everything they do not own survives: unknown namespaces, unknown
//! properties, nested structures, comments and processing instructions are
//! carried over byte for byte, and a property that already says the right
//! thing is not touched at all. The one thing rewritten around a change is
//! the start tag holding a changed attribute, which keeps every attribute
//! and value but is laid out afresh. The properties owned are listed on
//! [`XmpData`].
//!
//! # Rating and flag
//!
//! Stars are `xmp:Rating`. XMP has no property for a pick, and can say
//! "rejected" only as a rating of -1, which leaves no room for stars. So
//! the flag has a property of its own, the private `photo:Flag`, and what
//! this crate writes to the rating is always the stars. On reading,
//! `photo:Flag` decides the flag whenever it holds one; without it a
//! negative rating means rejected. A negative rating never gives stars.
//! The details are on [`XmpData::rating`] and [`XmpData::flag`].
//!
//! # Develop settings
//!
//! The settings are written twice. The private `photo:Settings` property
//! holds all of them, exactly. Beside it the Camera Raw (`crs:`) properties
//! hold what that vocabulary has a place for, under its process-2012 names,
//! for other applications to read. That mapping exchanges slider positions
//! only: a number means the same slider on both sides, but every
//! application renders it with its own math, so the same numbers do not
//! promise the same picture. Local corrections, retouching and lens
//! profiles are no part of it: they are not read and never written.
//!
//! On reading, `photo:Settings` wins whenever it is present; a sidecar
//! without it has the Camera Raw properties laid over the starting settings
//! for the kind of file. `photo:SettingsFormat` gives the format of the
//! private settings. Private settings this build cannot read, of a newer
//! format or not readable at all, are neither read nor overwritten (see
//! [`XmpData::unreadable_settings_format`]).
//!
//! - The Camera Raw properties come with `crs:ProcessVersion` 11.0 and
//!   `crs:Version` 15.4, the pair beside the 2012 names in the test
//!   fixtures written in Adobe's form (a process version from 6.7 on, which
//!   reads the same names, is kept), and with `crs:HasSettings`. They tell
//!   other applications which names to read, not that the picture renders
//!   alike.
//! - Scalar adjustments are one property each, written in the shortest form
//!   that reads back as the same number.
//! - White balance is `crs:WhiteBalance` with `crs:Temperature` and
//!   `crs:Tint` for a RAW file, and with `crs:IncrementalTemperature` and
//!   `crs:IncrementalTint` for a raster file, whose balance is relative.
//! - Point curves are lists of `x, y` pairs in whole levels from 0 to 255, so
//!   a point between two levels is stored at the nearer one.
//!
//! # Orientation and crop
//!
//! These two are recorded relative to the pixels as stored in the file,
//! where [`DevelopSettings`] has them relative to the upright picture the
//! decoder hands over, so mapping them takes [`ImageInfo`].
//!
//! `tiff:Orientation` (1 to 8, as in TIFF and EXIF) is the file's own
//! orientation followed by whatever the user did on top. It is read into and
//! written from [`DevelopSettings::orientation`] only when
//! [`ImageInfo::orientation`] is known.
//!
//! Adobe's XMP specification names the Camera Raw crop properties without
//! saying what they are measured against. They are read here the way
//! darktable's Lightroom importer reads them (`src/develop/lightroom.c`);
//! that reading has not been checked against Adobe software.
//!
//! - `crs:CropLeft` and `crs:CropTop`, and `crs:CropRight` and
//!   `crs:CropBottom`, are where the crop rectangle's top-left and
//!   bottom-right corners lie once the rectangle is turned by
//!   `crs:CropAngle`, as fractions of the frame's width and height measured
//!   from its top-left corner.
//! - `crs:CropAngle` turns the rectangle clockwise about its centre, in
//!   degrees: the same number as the settings' crop angle, with its sign
//!   flipped when the orientation between the two frames mirrors.
//! - Both are given in the frame of the stored pixels, before
//!   `tiff:Orientation`. The settings' rectangle is the same one carried
//!   into the frame the settings use: through the file's own orientation
//!   and then the user's. When the file's own orientation is not known the
//!   sidecar's stands in for it, and failing that the pixels are taken to be
//!   stored upright.
//! - Turning a rectangle needs the shape of the frame. It is taken from the
//!   sidecar's `tiff:ImageWidth` and `tiff:ImageLength`, else from
//!   [`ImageInfo::size`], else assumed to be 3:2. It plays no part in a crop
//!   that is not rotated.
//! - `crs:HasCrop` is false only when there is neither a rectangle nor an
//!   angle. A straightened photo with no explicit rectangle is `HasCrop`
//!   true with an angle and no corners, and reads back as exactly that.
//!
//! A crop this crate wrote reads back exactly, for any rectangle that lies
//! within the frame, as long as the orientation it was written under is
//! read back too: through `tiff:Orientation`, or through `photo:Settings`.
//!
//! Nothing in this crate decides where files live: every path is an
//! argument.

#![warn(missing_docs)]

mod crs;
mod curve;
mod dom;
mod fields;
mod file;
mod geometry;
mod rdf;
mod settings;
mod values;

use std::path::PathBuf;

use photo_core::develop::{DevelopSettings, Orientation, SETTINGS_FORMAT};
use photo_core::meta::{ColorLabel, FileKind, Flag, Rating};
use photo_core::time::DateTime;

pub use file::{read_sidecar, sidecar_path, write_sidecar};

/// The namespace of this application's own properties, bound to the prefix
/// `photo` in the sidecars it writes. It has two: `photo:Settings` holds
/// the complete [`DevelopSettings`] as JSON, and `photo:Flag` holds the
/// pick or reject flag (see [`XmpData::flag`]).
///
/// The host is under `.invalid`, a top-level domain reserved for names that
/// must never resolve, so the namespace cannot collide with anyone's.
pub const SETTINGS_NAMESPACE: &str = "http://ns.photo.invalid/1.0/";

/// What the mapping needs to know about the image a sidecar belongs to.
///
/// A sidecar records some things relative to the file rather than to the
/// picture, and those cannot be mapped without knowing the file. Only
/// [`Self::kind`] is required; the rest sharpens the mapping of orientation
/// and crop, and must be the same when a sidecar is read as when it was
/// written.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImageInfo {
    /// Whether the image is RAW or raster. It decides the white-balance
    /// properties (kelvin for RAW, relative for raster), the settings a
    /// sidecar's develop properties are laid over, and the ranges they are
    /// sanitised to.
    pub kind: FileKind,
    /// The file's own orientation (its EXIF or TIFF orientation tag): the
    /// turn the decoder has already applied to hand over an upright source.
    ///
    /// `tiff:Orientation` in a sidecar is this turn followed by whatever the
    /// user did on top, so the two can only be told apart when this is
    /// known. With `None`, `tiff:Orientation` is neither read into
    /// [`DevelopSettings::orientation`] nor written: a rotation made here
    /// then travels in `photo:Settings` alone, and one made in another
    /// application is not picked up. To place a crop, the sidecar's
    /// `tiff:Orientation` is taken to be the file's own.
    pub orientation: Option<Orientation>,
    /// The pixel width and height of the upright source, as the decoder
    /// reports them. A rotated crop is written as corner positions, which
    /// depend on the shape of the frame; without a size here, or in the
    /// sidecar's `tiff:ImageWidth` and `tiff:ImageLength`, a 3:2 frame is
    /// assumed.
    pub size: Option<(u32, u32)>,
}

impl ImageInfo {
    /// An image of `kind` of which nothing else is known.
    pub const fn new(kind: FileKind) -> Self {
        Self {
            kind,
            orientation: None,
            size: None,
        }
    }

    /// The same image, with its own orientation known to be `orientation`.
    pub const fn with_orientation(self, orientation: Orientation) -> Self {
        Self {
            orientation: Some(orientation),
            ..self
        }
    }

    /// The same image, with its upright source known to be `width` by
    /// `height` pixels.
    pub const fn with_size(self, width: u32, height: u32) -> Self {
        Self {
            size: Some((width, height)),
            ..self
        }
    }
}

/// What a sidecar says about a photo.
///
/// Reading a sidecar this crate wrote gives back the data it was written
/// from, with the exceptions noted on [`Self::capture_time`] and
/// [`Self::unreadable_settings_format`]. Empty strings and empty lists mean
/// "absent": they are what a missing property reads as, and writing them
/// removes the property (or, for text held in other languages too, empties
/// it; see [`Self::title`]).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct XmpData {
    /// The stars: `xmp:Rating`, 0 to 5, written whatever the flag is.
    ///
    /// A rating outside what the XMP specification allows is read as near
    /// to it as can be: a fraction counts as the nearest whole star and
    /// anything above five as five. A negative rating has no stars and,
    /// unless `photo:Flag` says otherwise, means the photo is rejected (see
    /// [`Self::flag`]). A value that is not a finite number counts as
    /// absent, which is no stars.
    pub rating: Rating,
    /// The pick or reject flag: the private `photo:Flag` property.
    ///
    /// It holds the number [`Flag::to_i8`] gives: 1 for picked, -1 for
    /// rejected. An unflagged photo is written without the property, and
    /// one left from an earlier state is removed; a 0 found there reads as
    /// unflagged all the same.
    ///
    /// On reading, `photo:Flag` decides the flag whenever it holds one of
    /// those three numbers, whatever the rating says; any other value
    /// counts as absent. Without it the flag comes from `xmp:Rating`: -1,
    /// which the XMP specification defines as "rejected", and any other
    /// negative number mean [`Flag::Rejected`], and everything else means
    /// [`Flag::Unflagged`].
    ///
    /// A rejection found as a rating of -1 is left as it is for as long as
    /// the photo stays rejected and without stars. This crate never writes
    /// that rating itself: once the stars or the flag change, the rating
    /// holds the stars and a rejection is in `photo:Flag`, where other
    /// applications do not look for it. A rating of -1 beside a `photo:Flag`
    /// that overrules it (another application rejected a photo picked here)
    /// is replaced by the stars on the next write.
    pub flag: Flag,
    /// `xmp:Label`, by colour name. A label under any other name reads as
    /// [`ColorLabel::None`], and writing `None` back leaves that name alone.
    pub label: ColorLabel,
    /// `dc:title` in the default language (`x-default`), else the first
    /// language. Other languages are kept when the title changes: a new
    /// title goes into the default language, which is added in front of the
    /// others when there is none, and a title cleared while other languages
    /// remain leaves them beside an empty default, so that it reads back as
    /// no title. With no other language, clearing removes the property.
    pub title: String,
    /// `dc:description`, read and written like the title.
    pub caption: String,
    /// `dc:creator`, in order.
    pub creators: Vec<String>,
    /// `dc:rights`, read and written like the title.
    pub copyright: String,
    /// `dc:subject`: the flat keywords.
    pub keywords: Vec<String>,
    /// `lr:hierarchicalSubject`: each entry is the path from a top-level
    /// keyword down to the one assigned, written with `|` between levels. A
    /// level cannot itself contain `|`: it would read back as two levels.
    pub hierarchical_keywords: Vec<Vec<String>>,
    /// When the picture was taken: `exif:DateTimeOriginal`, else
    /// `xmp:CreateDate`, as the clock time written (zone and fractions of a
    /// second are not kept). Writing sets `exif:DateTimeOriginal` only when
    /// the sidecar does not already give this time, and `None` leaves
    /// whatever the sidecar says in place rather than removing it.
    pub capture_time: Option<DateTime>,
    /// The develop settings, sanitised for the kind of file. `None` when
    /// the sidecar holds no develop properties; writing `None` leaves the
    /// develop properties of an existing sidecar as they are.
    ///
    /// Written twice: completely, as JSON in `photo:Settings`, and as the
    /// Camera Raw properties other applications read (everything but the
    /// profile, the crop tool's aspect lock, healing spots and masks). On
    /// reading, `photo:Settings` is authoritative whenever it is present,
    /// even if the Camera Raw properties beside it were changed since; only
    /// a sidecar without it is read through the Camera Raw properties, laid
    /// over the starting settings for the kind of file.
    ///
    /// `photo:SettingsFormat` beside the JSON gives its format,
    /// [`SETTINGS_FORMAT`] when this build wrote it; without it the format
    /// is the first. Private settings this build cannot read are not read
    /// at all: see [`Self::unreadable_settings_format`]. Settings holding a
    /// number that is not finite are refused on writing, since JSON cannot
    /// hold one.
    pub develop: Option<DevelopSettings>,
    /// Set when the sidecar holds private develop settings (`photo:Settings`)
    /// this build cannot read: the format they claim, which is
    /// `photo:SettingsFormat`, or the first format when that does not say.
    /// So it is for settings of a newer format than [`SETTINGS_FORMAT`], and
    /// for settings that are not this build's JSON or not text at all,
    /// whatever format they claim. `None` otherwise.
    ///
    /// When it is set, [`Self::develop`] is `None`: the settings are not
    /// read, because this build would misread them, nor replaced by the
    /// Camera Raw properties beside them, because the next write would then
    /// make those the photo's settings for good. Writing develop settings to
    /// such a sidecar fails with [`XmpError::UnreadableSettings`] and writes
    /// nothing, until the property is removed; writing with `develop` left
    /// `None` updates everything else and leaves the settings as they are.
    /// This field is only ever read: it plays no part in writing.
    pub unreadable_settings_format: Option<u32>,
}

/// Why a sidecar could not be read or written.
#[derive(Debug, thiserror::Error)]
pub enum XmpError {
    /// The sidecar file could not be read, or could not be replaced.
    #[error("could not {action} {}: {source}", .path.display())]
    Io {
        /// What was being attempted: `"read"` or `"write"`.
        action: &'static str,
        /// The sidecar file.
        path: PathBuf,
        /// The underlying failure.
        #[source]
        source: std::io::Error,
    },
    /// The text is not well-formed XML in UTF-8: a tag is unclosed or
    /// mismatched, a reference is undefined, a character XML does not allow
    /// is written out or referred to, the file is cut short, empty, or in
    /// another encoding.
    #[error("not well-formed XML at byte {offset}: {message}")]
    Malformed {
        /// Where in the text the problem was noticed.
        offset: u64,
        /// What the problem is.
        message: String,
    },
    /// The text is well-formed XML but holds no XMP packet: there is no
    /// `rdf:RDF` element at or directly under the root.
    #[error("the XML holds no XMP packet (no rdf:RDF element)")]
    NotXmp,
    /// The sidecar, or the text that would replace it, is larger than
    /// [`MAX_SIDECAR_BYTES`]. Such a sidecar is neither read nor written.
    #[error("the sidecar is larger than the {MAX_SIDECAR_BYTES} bytes accepted")]
    TooLarge,
    /// The sidecar holds private develop settings this build cannot read, of
    /// a newer format or not readable at all, and writing develop settings
    /// would replace them; nothing was written. See
    /// [`XmpData::unreadable_settings_format`].
    #[error(
        "the sidecar holds develop settings this build cannot read (format {format}; this build \
         reads up to {})",
        SETTINGS_FORMAT
    )]
    UnreadableSettings {
        /// The format the sidecar's settings claim.
        format: u32,
    },
    /// The develop settings hold a number that is not finite (NaN or an
    /// infinity). JSON has no way to write one, so the settings could not
    /// be read back; nothing was written.
    #[error("the develop settings hold a number that is not finite")]
    NotFinite,
    /// The sidecar kept changing while it was being updated, so the update
    /// was given up rather than written over what another application had
    /// just saved. The file holds that application's last version.
    #[error("{} kept changing while it was being updated", .path.display())]
    Conflict {
        /// The sidecar file.
        path: PathBuf,
    },
    /// The develop settings could not be encoded as JSON.
    #[error("the develop settings could not be encoded: {0}")]
    Settings(#[from] serde_json::Error),
}

/// The largest sidecar read or written, in bytes: 16 MiB.
///
/// Real sidecars hold kilobytes, a few megabytes at most. The limit bounds
/// the time and memory a hostile file can cost, and a text this crate would
/// write beyond it is refused too, so that every sidecar it writes can be
/// read back.
pub const MAX_SIDECAR_BYTES: usize = 16 << 20;

/// Reads the text of a sidecar for an image described by `image`.
///
/// Never panics, whatever the text: anything that is not a well-formed XMP
/// packet is an [`XmpError`]. Within a well-formed packet nothing is fatal:
/// unknown properties are ignored and values that do not parse count as
/// absent. Develop settings pass through
/// [`DevelopSettings::sanitize_for`] before they are returned.
pub fn parse(xml: &str, image: ImageInfo) -> Result<XmpData, XmpError> {
    let packet = rdf::Packet::parse(xml)?;
    Ok(fields::read(&packet, image))
}

/// The text of a sidecar that says what `data` says, for an image described
/// by `image`.
///
/// With `existing` (the sidecar's current text) the result is that text
/// with only the properties this crate owns changed, and of those only the
/// ones whose value differs; everything else is carried over as it was. An
/// `existing` that is empty or blank counts as none; one that is not a
/// well-formed XMP packet is refused with an error rather than replaced.
///
/// Writing back what was read returns `existing` itself (less a leading
/// byte-order mark and white space inside end tags, which are not kept),
/// with two exceptions:
///
/// - Develop settings are written whole. Where the sidecar lacks some of
///   what this crate writes for them (`photo:Settings` and its format, a
///   Camera Raw property, the Camera Raw header), the first write of what
///   was read adds it; a second write changes nothing. Data read with
///   [`XmpData::develop`] set to `None` touches none of it.
/// - A rating of -1 that `photo:Flag` overrules is replaced by the stars,
///   so that other applications stop taking the photo for rejected.
///
/// Without `existing` the result is a new packet in the standard
/// `<?xpacket?>` wrapper.
///
/// Either way, an `existing` or a result longer than [`MAX_SIDECAR_BYTES`]
/// is refused with [`XmpError::TooLarge`]. Develop settings are refused, and
/// nothing written, when `existing` holds private settings this build
/// cannot read ([`XmpError::UnreadableSettings`]) or when they hold a number
/// that is not finite ([`XmpError::NotFinite`]).
pub fn to_string(
    data: &XmpData,
    image: ImageInfo,
    existing: Option<&str>,
) -> Result<String, XmpError> {
    let too_large = |text: &str| text.len() > MAX_SIDECAR_BYTES;
    if existing.is_some_and(too_large) {
        return Err(XmpError::TooLarge);
    }
    let is_blank = |text: &str| text.trim_start_matches('\u{feff}').trim().is_empty();
    let mut packet = match existing {
        Some(text) if !is_blank(text) => rdf::Packet::parse(text)?,
        _ => rdf::Packet::new(),
    };
    fields::write(&mut packet, data, image)?;
    let text = packet.into_string();
    if too_large(&text) {
        return Err(XmpError::TooLarge);
    }
    Ok(text)
}
