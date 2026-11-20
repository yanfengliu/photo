//! Raster fixtures: the scene written as JPEG, PNG and TIFF.
//!
//! Every writer takes the directory to write into and a bare file name, and
//! returns the path it wrote. They panic rather than return errors: a fixture
//! that cannot be written is a broken test, not a condition to handle.

use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::tiff::TiffEncoder;
use image::{ExtendedColorType, ImageEncoder, ImageFormat, RgbImage};
use photo_core::colorimetry::RgbSpace;

use crate::exif::{ExifSpec, add_exif, exif_block};
use crate::icc::icc_profile;
use crate::orient::to_stored;
use crate::scene::{MARKER_RGB, marker_size, scene, scene_with_marker, scene16};
use crate::tiff::{Ifd, TiffWriter, Value};

/// JPEG quality used for every JPEG fixture: high enough that a decoded
/// fixture stays within a few code values of the scene.
const JPEG_QUALITY: u8 = 92;

/// Resolves `name` inside `dir` after checking that this really is fixture
/// territory.
///
/// Panics unless `dir` exists and lies inside the system temporary directory,
/// and `name` is a bare file name. This is what backs the crate's promise
/// never to touch a path outside a temp directory, whatever a test passes in.
pub(crate) fn fixture_path(dir: &Path, name: &str) -> PathBuf {
    let bare = Path::new(name).file_name().is_some_and(|file| file == name);
    assert!(bare, "fixture name must be a bare file name, got {name:?}");
    let temp_root = tempfile::env::temp_dir()
        .canonicalize()
        .expect("the system temporary directory must exist");
    let real_dir = dir.canonicalize().unwrap_or_else(|error| {
        panic!("fixture directory {} is not usable: {error}", dir.display())
    });
    assert!(
        real_dir.starts_with(&temp_root),
        "photo-testkit only writes inside the temporary directory {}, not {}",
        temp_root.display(),
        real_dir.display()
    );
    dir.join(name)
}

pub(crate) fn write_bytes(path: &Path, bytes: &[u8]) {
    fs::write(path, bytes)
        .unwrap_or_else(|error| panic!("cannot write fixture {}: {error}", path.display()));
}

/// Writes [`scene`] as a baseline 8-bit JPEG with no metadata.
pub fn write_jpeg(dir: &Path, name: &str, width: u32, height: u32) -> PathBuf {
    let path = fixture_path(dir, name);
    write_bytes(&path, &jpeg_bytes(&scene(width, height)));
    path
}

/// Writes [`scene`] as a lossless 8-bit RGB PNG.
pub fn write_png8(dir: &Path, name: &str, width: u32, height: u32) -> PathBuf {
    let path = fixture_path(dir, name);
    scene(width, height)
        .save_with_format(&path, ImageFormat::Png)
        .expect("encode PNG fixture");
    path
}

/// Writes [`scene16`] as a lossless 16-bit RGB PNG.
pub fn write_png16(dir: &Path, name: &str, width: u32, height: u32) -> PathBuf {
    let path = fixture_path(dir, name);
    scene16(width, height)
        .save_with_format(&path, ImageFormat::Png)
        .expect("encode PNG fixture");
    path
}

/// Writes [`scene16`] as a lossless 16-bit RGB TIFF.
pub fn write_tiff16(dir: &Path, name: &str, width: u32, height: u32) -> PathBuf {
    let path = fixture_path(dir, name);
    scene16(width, height)
        .save_with_format(&path, ImageFormat::Tiff)
        .expect("encode TIFF fixture");
    path
}

/// Writes [`scene_with_marker`] as a JPEG carrying the EXIF block `spec`
/// describes.
///
/// `width` x `height` is the size of the upright picture. The stored pixel
/// grid is turned and mirrored to suit `spec.orientation` (so it is `height`
/// x `width` for orientations 5 to 8), which means a correct reader shows the
/// scene upright, marker in the top-left corner, whatever the orientation.
pub fn write_jpeg_with_exif(
    dir: &Path,
    name: &str,
    width: u32,
    height: u32,
    spec: &ExifSpec,
) -> PathBuf {
    let path = fixture_path(dir, name);
    let stored = stored_rgb(&scene_with_marker(width, height), spec.orientation);
    write_bytes(
        &path,
        &with_exif_segment(jpeg_bytes(&stored), &exif_block(spec)),
    );
    path
}

/// Writes the 16-bit scene, with the corner marker of [`scene_with_marker`],
/// as an uncompressed RGB TIFF whose directories carry the EXIF `spec`
/// describes. Sizes and orientation work as in [`write_jpeg_with_exif`].
pub fn write_tiff16_with_exif(
    dir: &Path,
    name: &str,
    width: u32,
    height: u32,
    spec: &ExifSpec,
) -> PathBuf {
    let path = fixture_path(dir, name);
    let mut upright: Vec<[u16; 3]> = scene16(width, height)
        .pixels()
        .map(|pixel| pixel.0)
        .collect();
    let side = marker_size(width, height) as usize;
    for row in upright.chunks_exact_mut(width as usize).take(side) {
        row[..side].fill(MARKER_RGB.map(|c| u16::from(c) * 257));
    }
    let (stored, stored_width, stored_height) =
        to_stored(&upright, width as usize, height as usize, spec.orientation);
    let strip: Vec<u8> = stored
        .iter()
        .flatten()
        .flat_map(|sample| sample.to_le_bytes())
        .collect();

    let mut writer = TiffWriter::new();
    let strip_offset = writer.data(&strip);
    let mut ifd = Ifd::new();
    ifd.long(0x0100, stored_width as u32)
        .long(0x0101, stored_height as u32)
        .set(0x0102, Value::Short(vec![16, 16, 16]))
        .short(0x0103, 1)
        .short(0x0106, 2)
        .long(0x0111, strip_offset)
        .short(0x0115, 3)
        .long(0x0116, stored_height as u32)
        .long(0x0117, strip.len() as u32)
        .set(0x011a, Value::Rational(vec![(300, 1)]))
        .set(0x011b, Value::Rational(vec![(300, 1)]))
        .short(0x011c, 1)
        .short(0x0128, 2);
    add_exif(&mut writer, &mut ifd, spec);
    let first = writer.ifd(&ifd);
    write_bytes(&path, &writer.finish(first));
    path
}

/// Writes the scene tagged with an embedded ICC profile for `space`. The
/// extension of `name` picks the container: `png` (16-bit), `tif`/`tiff`
/// (16-bit) or `jpg`/`jpeg` (8-bit).
///
/// The code values are those of [`scene16`] (or [`scene`] for JPEG), simply
/// labelled as belonging to `space`: a reader that honours the profile must
/// linearise them with that space's transfer function and report its
/// primaries.
pub fn write_tagged(dir: &Path, name: &str, width: u32, height: u32, space: RgbSpace) -> PathBuf {
    let path = fixture_path(dir, name);
    let extension = path
        .extension()
        .and_then(|extension| extension.to_str())
        .map(str::to_ascii_lowercase);
    let deep = scene16(width, height);
    let deep_bytes: Vec<u8> = deep
        .as_raw()
        .iter()
        .flat_map(|sample| sample.to_ne_bytes())
        .collect();
    let mut bytes = Cursor::new(Vec::new());
    match extension.as_deref() {
        Some("png") => {
            let mut encoder = PngEncoder::new(&mut bytes);
            encoder
                .set_icc_profile(icc_profile(space))
                .expect("PNG carries ICC profiles");
            encoder
                .write_image(&deep_bytes, width, height, ExtendedColorType::Rgb16)
                .expect("encode PNG fixture");
        }
        Some("tif" | "tiff") => {
            let mut encoder = TiffEncoder::new(&mut bytes);
            encoder
                .set_icc_profile(icc_profile(space))
                .expect("TIFF carries ICC profiles");
            encoder
                .write_image(&deep_bytes, width, height, ExtendedColorType::Rgb16)
                .expect("encode TIFF fixture");
        }
        Some("jpg" | "jpeg") => {
            let mut encoder = JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY);
            encoder
                .set_icc_profile(icc_profile(space))
                .expect("JPEG carries ICC profiles");
            encoder
                .write_image(
                    scene(width, height).as_raw(),
                    width,
                    height,
                    ExtendedColorType::Rgb8,
                )
                .expect("encode JPEG fixture");
        }
        other => panic!("write_tagged supports png, tif and jpg, not {other:?}"),
    }
    write_bytes(&path, bytes.get_ref());
    path
}

/// Encodes `image` as a baseline JPEG in memory.
pub(crate) fn jpeg_bytes(image: &RgbImage) -> Vec<u8> {
    let mut bytes = Vec::new();
    JpegEncoder::new_with_quality(&mut bytes, JPEG_QUALITY)
        .encode_image(image)
        .expect("encode JPEG fixture");
    bytes
}

/// Rearranges an upright picture into the grid stored for `orientation`.
pub(crate) fn stored_rgb(upright: &RgbImage, orientation: u8) -> RgbImage {
    let pixels: Vec<[u8; 3]> = upright.pixels().map(|pixel| pixel.0).collect();
    let (stored, width, height) = to_stored(
        &pixels,
        upright.width() as usize,
        upright.height() as usize,
        orientation,
    );
    RgbImage::from_raw(
        width as u32,
        height as u32,
        stored.into_iter().flatten().collect(),
    )
    .expect("stored grid has the right length")
}

/// Splices an `APP1` EXIF segment into a JPEG stream, after the JFIF header
/// if there is one.
fn with_exif_segment(jpeg: Vec<u8>, exif: &[u8]) -> Vec<u8> {
    assert_eq!(&jpeg[..2], [0xff, 0xd8], "JPEG starts with SOI");
    let mut insert_at = 2;
    if jpeg[2..4] == [0xff, 0xe0] {
        insert_at += 2 + usize::from(u16::from_be_bytes([jpeg[4], jpeg[5]]));
    }
    let payload_len = 2 + 6 + exif.len();
    let length = u16::try_from(payload_len).expect("EXIF block fits one JPEG segment");

    let mut out = Vec::with_capacity(jpeg.len() + payload_len + 2);
    out.extend_from_slice(&jpeg[..insert_at]);
    out.extend_from_slice(&[0xff, 0xe1]);
    out.extend_from_slice(&length.to_be_bytes());
    out.extend_from_slice(b"Exif\0\0");
    out.extend_from_slice(exif);
    out.extend_from_slice(&jpeg[insert_at..]);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::metadata::Orientation;
    use image::{DynamicImage, ImageDecoder, ImageReader};

    fn open(path: &Path) -> DynamicImage {
        ImageReader::open(path)
            .unwrap()
            .with_guessed_format()
            .unwrap()
            .decode()
            .unwrap()
    }

    #[test]
    fn plain_writers_produce_files_of_the_requested_size_and_depth() {
        let dir = tempfile::tempdir().unwrap();
        let jpeg = open(&write_jpeg(dir.path(), "a.jpg", 96, 64));
        let png8 = open(&write_png8(dir.path(), "a.png", 96, 64));
        let png16 = open(&write_png16(dir.path(), "b.png", 96, 64));
        let tiff16 = open(&write_tiff16(dir.path(), "a.tif", 96, 64));
        for image in [&jpeg, &png8, &png16, &tiff16] {
            assert_eq!((image.width(), image.height()), (96, 64));
        }
        assert!(matches!(jpeg, DynamicImage::ImageRgb8(_)));
        assert_eq!(png8.as_rgb8().unwrap(), &scene(96, 64));
        assert_eq!(png16.as_rgb16().unwrap(), &scene16(96, 64));
        assert_eq!(tiff16.as_rgb16().unwrap(), &scene16(96, 64));
    }

    #[test]
    fn the_jpeg_stays_close_to_the_scene() {
        let dir = tempfile::tempdir().unwrap();
        let decoded = open(&write_jpeg(dir.path(), "a.jpg", 160, 120)).into_rgb8();
        let expected = scene(160, 120);
        let total: u64 = decoded
            .as_raw()
            .iter()
            .zip(expected.as_raw())
            .map(|(a, b)| u64::from(a.abs_diff(*b)))
            .sum();
        let mean = total as f64 / decoded.as_raw().len() as f64;
        assert!(mean < 4.0, "mean error {mean}");
    }

    #[test]
    fn the_exif_jpeg_stores_a_turned_grid_that_the_tag_turns_back() {
        let dir = tempfile::tempdir().unwrap();
        let upright = scene_with_marker(120, 80);
        for orientation in 1..=8u8 {
            let spec = ExifSpec {
                orientation,
                ..ExifSpec::sample()
            };
            let path =
                write_jpeg_with_exif(dir.path(), &format!("o{orientation}.jpg"), 120, 80, &spec);

            let mut decoder = ImageReader::open(&path).unwrap().into_decoder().unwrap();
            let expected_grid = if orientation >= 5 {
                (80, 120)
            } else {
                (120, 80)
            };
            assert_eq!(
                decoder.dimensions(),
                expected_grid,
                "orientation {orientation}"
            );
            let tagged = decoder.orientation().unwrap();
            assert_eq!(tagged, Orientation::from_exif(orientation).unwrap());

            let mut image = DynamicImage::from_decoder(decoder).unwrap();
            image.apply_orientation(tagged);
            let image = image.into_rgb8();
            assert_eq!(image.dimensions(), (120, 80));
            let probe = marker_size(120, 80) / 2;
            let [r, g, b] = image.get_pixel(probe, probe).0;
            assert!(
                r > 200 && g < 60 && b > 200,
                "orientation {orientation}: marker is {:?}",
                [r, g, b]
            );
            let far = image.get_pixel(119 - probe, 79 - probe).0;
            let reference = upright.get_pixel(119 - probe, 79 - probe).0;
            assert!(
                far.iter().zip(reference).all(|(a, b)| a.abs_diff(b) < 40),
                "orientation {orientation}"
            );
        }
    }

    #[test]
    fn the_exif_tiff_is_a_readable_sixteen_bit_tiff_with_an_orientation() {
        let dir = tempfile::tempdir().unwrap();
        let spec = ExifSpec {
            orientation: 8,
            ..ExifSpec::sample()
        };
        let path = write_tiff16_with_exif(dir.path(), "o8.tif", 60, 40, &spec);
        let mut decoder = ImageReader::open(&path).unwrap().into_decoder().unwrap();
        assert_eq!(decoder.dimensions(), (40, 60));
        assert_eq!(decoder.orientation().unwrap(), Orientation::Rotate270);
        let mut image = DynamicImage::from_decoder(decoder).unwrap();
        assert!(matches!(image, DynamicImage::ImageRgb16(_)));
        image.apply_orientation(Orientation::Rotate270);
        let image = image.into_rgb16();
        assert_eq!(
            image.get_pixel(0, 0).0,
            MARKER_RGB.map(|c| u16::from(c) * 257)
        );
        assert_eq!(image.get_pixel(59, 39), scene16(60, 40).get_pixel(59, 39));
    }

    #[test]
    fn tagged_files_carry_the_profile_in_every_container() {
        let dir = tempfile::tempdir().unwrap();
        let profile = Some(icc_profile(RgbSpace::DisplayP3));
        for name in ["p3.png", "p3.jpg"] {
            let path = write_tagged(dir.path(), name, 48, 32, RgbSpace::DisplayP3);
            let mut decoder = ImageReader::open(&path).unwrap().into_decoder().unwrap();
            assert_eq!(decoder.dimensions(), (48, 32));
            assert_eq!(decoder.icc_profile().unwrap(), profile, "{name}");
        }
        // The TIFF decoder is opened directly: behind `ImageReader` its tag
        // size limit shrinks to the pixel data size, which for an image this
        // small is less than the profile.
        let path = write_tagged(dir.path(), "p3.tif", 48, 32, RgbSpace::DisplayP3);
        let file = std::io::BufReader::new(fs::File::open(&path).unwrap());
        let mut decoder = image::codecs::tiff::TiffDecoder::new(file).unwrap();
        assert_eq!(decoder.dimensions(), (48, 32));
        assert_eq!(decoder.icc_profile().unwrap(), profile);

        for name in ["p3.png", "p3.tif"] {
            let decoded = open(&dir.path().join(name));
            assert_eq!(decoded.as_rgb16().unwrap(), &scene16(48, 32), "{name}");
        }
    }

    #[test]
    #[should_panic(expected = "only writes inside the temporary directory")]
    fn writing_outside_the_temporary_directory_is_refused() {
        // The crate's own manifest directory exists and is certainly not a
        // temp directory; nothing is written because the check comes first.
        write_png8(Path::new(env!("CARGO_MANIFEST_DIR")), "never.png", 4, 4);
    }

    #[test]
    #[should_panic(expected = "bare file name")]
    fn names_with_directories_in_them_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        write_png8(dir.path(), "../escape.png", 4, 4);
    }

    #[test]
    #[should_panic(expected = "not usable")]
    fn a_missing_directory_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        write_png8(&dir.path().join("absent"), "a.png", 4, 4);
    }
}
