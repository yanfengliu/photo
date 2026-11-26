//! Fixtures and readers shared by the contract tests. Everything is written
//! into temporary directories and checked by decoding what was written.

use std::fs;
use std::io::BufReader;
use std::path::{Path, PathBuf};

use image::{DynamicImage, ImageDecoder, ImageReader};
use photo_core::colorimetry::RgbSpace;
use photo_core::develop::DevelopSettings;
use photo_core::geom::Rect;
use photo_core::image::{Rgb16Image, SourceColor, SourceImage};
use photo_core::meta::PhotoMetadata;
use photo_develop::{CancelToken, Quality, RenderRequest, Renderer, Source, output_size};
use photo_export::{ExportAttributes, ExportItem, ExportOutcome, ExportSettings, Folder};
use photo_imaging::View;
use tempfile::TempDir;

/// An original on disk, decoded, with an edit and metadata to export.
pub struct Fixture {
    pub dir: TempDir,
    pub original: PathBuf,
    pub source: Source,
    pub settings: DevelopSettings,
    pub metadata: PhotoMetadata,
    pub attributes: ExportAttributes,
    pub sequence: u32,
    pub protected: Vec<PathBuf>,
}

impl Fixture {
    /// The testkit scene as a 16-bit PNG named `original.png`.
    pub fn new(width: u32, height: u32) -> Self {
        Self::named("original.png", width, height)
    }

    /// The testkit scene written under `name` (PNG, TIFF or JPEG by
    /// extension).
    pub fn named(name: &str, width: u32, height: u32) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let original = match Path::new(name).extension().and_then(|e| e.to_str()) {
            Some("tif" | "tiff" | "TIF") => {
                photo_testkit::write_tiff16(dir.path(), name, width, height)
            }
            Some("jpg" | "jpeg" | "JPG") => {
                photo_testkit::write_jpeg(dir.path(), name, width, height)
            }
            _ => photo_testkit::write_png16(dir.path(), name, width, height),
        };
        let decoded = photo_decode::decode(&original).unwrap();
        Self {
            dir,
            original,
            source: Source::new(decoded.image),
            settings: edited(),
            metadata: decoded.metadata,
            attributes: ExportAttributes::default(),
            sequence: 1,
            protected: Vec::new(),
        }
    }

    /// A fixture whose original is a raster of `pixels(x, y)` in linear
    /// sRGB, written as a 16-bit PNG.
    pub fn from_fn(width: u32, height: u32, pixels: impl Fn(u32, u32) -> [f32; 3]) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let encoded = image::ImageBuffer::from_fn(width, height, |x, y| {
            image::Rgb(
                pixels(x, y)
                    .map(|v| (RgbSpace::Srgb.encode(v.clamp(0.0, 1.0)) * 65535.0).round() as u16),
            )
        });
        let original = dir.path().join("original.png");
        encoded.save(&original).unwrap();
        let decoded = photo_decode::decode(&original).unwrap();
        Self {
            dir,
            original,
            source: Source::new(decoded.image),
            settings: DevelopSettings::default(),
            metadata: decoded.metadata,
            attributes: ExportAttributes::default(),
            sequence: 1,
            protected: Vec::new(),
        }
    }

    /// The item to export, reusing the decoded source.
    pub fn item(&self) -> ExportItem<'_> {
        ExportItem {
            source_path: &self.original,
            source: Some(&self.source),
            settings: &self.settings,
            metadata: &self.metadata,
            attributes: self.attributes.clone(),
            sequence: self.sequence,
            protected: &self.protected,
        }
    }

    /// A folder for exports inside the fixture's directory.
    pub fn out_dir(&self) -> PathBuf {
        self.dir.path().join("out")
    }

    /// Export settings writing into [`Self::out_dir`].
    pub fn settings_out(&self) -> ExportSettings {
        let mut settings = ExportSettings::default();
        settings.location.folder = Folder::Specific(self.out_dir());
        settings
    }

    /// The direct single render the export of this fixture should equal:
    /// the whole output at `size`, in `space`.
    pub fn render(&self, size: (u32, u32), space: RgbSpace) -> Rgb16Image {
        render(&self.source, &self.settings, size, space)
    }
}

/// A plausible edit touching every kind of stage.
pub fn edited() -> DevelopSettings {
    let mut settings = DevelopSettings::default();
    settings.tone.exposure = 0.2;
    settings.tone.contrast = 20.0;
    settings.tone.shadows = 30.0;
    settings.presence.clarity = 20.0;
    settings.presence.vibrance = 15.0;
    settings.detail.sharpening.amount = 40.0;
    settings.effects.vignette.amount = -20.0;
    settings
}

/// The whole output of `source` rendered at `size` in `space`, in one piece.
pub fn render(
    source: &Source,
    settings: &DevelopSettings,
    size: (u32, u32),
    space: RgbSpace,
) -> Rgb16Image {
    let (width, height) = output_size(source, settings);
    let request = RenderRequest {
        source,
        settings,
        view: View::new(Rect::from_size(width as f32, height as f32), size.0, size.1),
        quality: Quality::Full,
        space,
    };
    Renderer::new()
        .render_rgb16(&request, &CancelToken::new())
        .unwrap()
}

/// The path and size of a written export; panics on a skip.
pub fn written(outcome: &ExportOutcome) -> (PathBuf, u32, u32) {
    match outcome {
        ExportOutcome::Written {
            path,
            width,
            height,
        } => (path.clone(), *width, *height),
        other => panic!("expected a written file, got {other:?}"),
    }
}

/// Decodes an exported file with the `image` crate.
pub fn open(path: &Path) -> DynamicImage {
    ImageReader::open(path)
        .unwrap()
        .with_guessed_format()
        .unwrap()
        .decode()
        .unwrap()
}

/// The embedded ICC profile of an exported file.
pub fn icc_profile(path: &Path) -> Option<Vec<u8>> {
    let extension = path.extension().unwrap().to_ascii_lowercase();
    if extension == "tif" {
        // Opened directly: behind `ImageReader` the TIFF decoder caps tag
        // sizes at the pixel data size, which small images undercut.
        let file = BufReader::new(fs::File::open(path).unwrap());
        let mut decoder = image::codecs::tiff::TiffDecoder::new(file).unwrap();
        return decoder.icc_profile().unwrap();
    }
    let mut decoder = ImageReader::open(path)
        .unwrap()
        .with_guessed_format()
        .unwrap()
        .into_decoder()
        .unwrap();
    decoder.icc_profile().unwrap()
}

/// Every name in `dir`, sorted. The `._name` companions that macOS keeps
/// beside files on a volume without extended attributes (FAT, exFAT) are
/// its own business and are left out, so that these tests can be run with
/// `TMPDIR` on such a volume.
pub fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| !name.starts_with("._"))
        .collect();
    names.sort();
    names
}

/// Gives `file` a second name, `link`: a real hard link wherever the
/// volume has them, and a symbolic link on one that has none (FAT, exFAT).
pub fn second_name(file: &Path, link: &Path) {
    if fs::hard_link(file, link).is_ok() {
        return;
    }
    #[cfg(unix)]
    std::os::unix::fs::symlink(file, link).unwrap();
    #[cfg(not(unix))]
    panic!("{} has no hard links", link.display());
}

/// A raster source made directly from linear sRGB values.
pub fn linear_source(width: u32, height: u32, pixels: impl Fn(u32, u32) -> [f32; 3]) -> Source {
    let mut samples = Vec::with_capacity(width as usize * height as usize * 3);
    for y in 0..height {
        for x in 0..width {
            samples.extend(pixels(x, y).map(|v| (v.clamp(0.0, 1.0) * 65535.0).round() as u16));
        }
    }
    let image = SourceImage::new(width, height, samples, SourceColor::Rgb(RgbSpace::Srgb));
    Source::new(image.unwrap())
}
