//! The values the preview service hands out.

use std::sync::Arc;

use photo_core::image::Rgba8Image;
use photo_imaging::Histogram;

/// The two preview sizes.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum PreviewSize {
    /// For grid cells and the filmstrip: longer edge at most 384 pixels.
    Thumbnail,
    /// For the loupe and the histogram: longer edge at most 2048 pixels.
    Standard,
}

impl PreviewSize {
    /// Both sizes, smaller first.
    pub const ALL: [Self; 2] = [Self::Thumbnail, Self::Standard];

    /// The most pixels along the longer edge.
    pub const fn long_edge(self) -> u32 {
        match self {
            Self::Thumbnail => 384,
            Self::Standard => 2048,
        }
    }

    pub(crate) const fn file_tag(self) -> &'static str {
        match self {
            Self::Thumbnail => "thumb",
            Self::Standard => "standard",
        }
    }

    pub(crate) const fn jpeg_quality(self) -> u8 {
        match self {
            Self::Thumbnail => 85,
            Self::Standard => 90,
        }
    }

    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::Thumbnail => 0,
            Self::Standard => 1,
        }
    }

    pub(crate) const fn from_code(code: u8) -> Option<Self> {
        match code {
            0 => Some(Self::Thumbnail),
            1 => Some(Self::Standard),
            _ => None,
        }
    }
}

/// Where a preview's pixels came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PreviewOrigin {
    /// The decoder's fast preview: a RAW file's embedded camera JPEG, or a
    /// raster file scaled down. Only the photo's orientation is applied.
    EmbeddedPreview,
    /// A render of the photo's develop settings.
    Rendered,
}

impl PreviewOrigin {
    pub(crate) const fn code(self) -> u8 {
        match self {
            Self::EmbeddedPreview => 1,
            Self::Rendered => 2,
        }
    }

    pub(crate) const fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::EmbeddedPreview),
            2 => Some(Self::Rendered),
            _ => None,
        }
    }
}

/// Ready preview pixels. Cloning is cheap: the pixels are shared.
#[derive(Clone, Debug)]
pub struct Preview {
    image: Arc<Rgba8Image>,
    version: u64,
    size: PreviewSize,
    origin: PreviewOrigin,
    histogram: Option<Arc<Histogram>>,
}

impl Preview {
    pub(crate) fn new(
        image: Rgba8Image,
        version: u64,
        size: PreviewSize,
        origin: PreviewOrigin,
        histogram: Option<Histogram>,
    ) -> Self {
        Self {
            image: Arc::new(image),
            version,
            size,
            origin,
            histogram: histogram.map(Arc::new),
        }
    }

    /// The pixels: opaque sRGB RGBA8, upright, longer edge at most the
    /// size's.
    pub fn image(&self) -> &Arc<Rgba8Image> {
        &self.image
    }

    /// The develop version these pixels show (0 for an import candidate).
    pub fn version(&self) -> u64 {
        self.version
    }

    /// Which size this is.
    pub fn size(&self) -> PreviewSize {
        self.size
    }

    /// Where the pixels came from.
    pub fn origin(&self) -> PreviewOrigin {
        self.origin
    }

    /// For a standard preview, the histogram of exactly these pixels.
    pub fn histogram(&self) -> Option<&Arc<Histogram>> {
        self.histogram.as_ref()
    }

    /// What keeping this preview in memory costs, in bytes.
    pub(crate) fn bytes(&self) -> usize {
        self.image.pixels.len() + self.histogram.as_ref().map_or(0, |_| 4 * 256 * 4)
    }
}

/// The answer to a preview request.
#[derive(Clone, Debug)]
pub enum PreviewState {
    /// The pixels for the photo's current develop version.
    Ready(Preview),
    /// A job is queued or running for it (possibly queued by this call);
    /// poll and ask again.
    Pending,
    /// It cannot be made, for this reason. Not retried for the same develop
    /// version.
    Failed(String),
}

impl PreviewState {
    /// The preview, when ready.
    pub fn ready(&self) -> Option<&Preview> {
        match self {
            Self::Ready(preview) => Some(preview),
            Self::Pending | Self::Failed(_) => None,
        }
    }
}

/// How far the pixels for one photo at one size have got, for the photo's
/// current develop version. The UI shows what is there and never has to
/// infer it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoadState {
    /// Nothing has been asked for, or what was there has been evicted.
    Nothing,
    /// A job is queued or running.
    Pending,
    /// Pixels are in memory.
    Ready {
        /// Where they came from: the embedded preview, or a render.
        origin: PreviewOrigin,
    },
    /// Making them failed, for this reason.
    Failed(String),
}
