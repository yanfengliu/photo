//! A decoded photo, ready to render.

use std::sync::Arc;

use photo_core::image::{SourceColor, SourceImage};
use photo_core::meta::FileKind;
use photo_imaging::SourcePyramid;

use crate::analysis::Analysis;

/// A decoded photo ready to render: its mip pyramid and its whole-image
/// [`Analysis`] (contract R3), both computed once in [`Source::new`].
///
/// Cloning is cheap (one reference count): every clone is the same source,
/// which is also how the renderer's caches recognise it.
#[derive(Clone)]
pub struct Source {
    inner: Arc<Inner>,
}

struct Inner {
    pyramid: SourcePyramid,
    analysis: Analysis,
}

impl Source {
    /// Build the pyramid and the analysis. The image itself becomes level 0
    /// of the pyramid without being copied.
    ///
    /// # Panics
    ///
    /// When `image.pixels` does not hold `width * height * 3` samples or a
    /// dimension is zero, which [`SourceImage::new`] never produces.
    pub fn new(image: SourceImage) -> Self {
        let pyramid = SourcePyramid::new(image);
        let analysis = Analysis::new(&pyramid);
        Self {
            inner: Arc::new(Inner { pyramid, analysis }),
        }
    }

    /// Width of the decoded image in pixels.
    pub fn width(&self) -> u32 {
        self.inner.pyramid.source_size().0
    }

    /// Height of the decoded image in pixels.
    pub fn height(&self) -> u32 {
        self.inner.pyramid.source_size().1
    }

    /// Width and height of the decoded image in pixels.
    pub fn size(&self) -> (u32, u32) {
        self.inner.pyramid.source_size()
    }

    /// What the source's numbers mean: a colour space or a camera.
    pub fn color(&self) -> SourceColor {
        self.inner.pyramid.level(0).color
    }

    /// [`FileKind::Raw`] for camera-native data, [`FileKind::Raster`] for
    /// everything else.
    pub fn kind(&self) -> FileKind {
        if self.color().is_camera() {
            FileKind::Raw
        } else {
            FileKind::Raster
        }
    }

    /// The source's mip pyramid; level 0 is the decoded image.
    pub fn pyramid(&self) -> &SourcePyramid {
        &self.inner.pyramid
    }

    /// The whole-image statistics every render and every tile shares.
    pub fn analysis(&self) -> &Analysis {
        &self.inner.analysis
    }

    /// True when both handles are the same source (not merely equal
    /// pixels).
    pub(crate) fn same(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

/// Prints the size and colour description, not the pixels.
impl std::fmt::Debug for Source {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Source")
            .field("size", &self.size())
            .field("color", &self.color())
            .finish_non_exhaustive()
    }
}

/// The source pixel `index` (row-major) of `image` as linear `0..=1` values.
#[inline]
pub(crate) fn unit_pixel(image: &SourceImage, index: usize) -> [f32; 3] {
    const WHITE: f32 = SourceImage::WHITE as f32;
    let i = index * SourceImage::CHANNELS;
    [
        f32::from(image.pixels[i]) / WHITE,
        f32::from(image.pixels[i + 1]) / WHITE,
        f32::from(image.pixels[i + 2]) / WHITE,
    ]
}
