//! Raster files: dimensions, bit depth, linearisation, grey, alpha and
//! embedded colour profiles.

mod common;

use std::path::Path;

use common::{
    linear_from_8, linear_from_16, linearised_8, linearised_16, max_abs_diff, mean_abs_diff,
};
use image::{GrayAlphaImage, GrayImage, ImageBuffer, Luma, LumaA, Rgb, Rgba, RgbaImage};
use photo_core::colorimetry::RgbSpace;
use photo_core::image::SourceColor;
use photo_decode::{decode, read_metadata};
use photo_testkit as kit;

const W: u32 = 160;
const H: u32 = 120;
const SRGB: SourceColor = SourceColor::Rgb(RgbSpace::Srgb);

#[test]
fn every_raster_writer_decodes_with_the_right_dimensions() {
    let dir = tempfile::tempdir().unwrap();
    let files = [
        kit::write_jpeg(dir.path(), "a.jpg", W, H),
        kit::write_png8(dir.path(), "a.png", W, H),
        kit::write_png16(dir.path(), "b.png", W, H),
        kit::write_tiff16(dir.path(), "a.tif", W, H),
    ];
    for path in files {
        let decoded = decode(&path).unwrap();
        assert_eq!(
            (decoded.image.width, decoded.image.height),
            (W, H),
            "{}",
            path.display()
        );
        assert_eq!(decoded.image.pixels.len(), (W * H * 3) as usize);
        assert_eq!(decoded.image.color, SRGB);
        assert_eq!((decoded.metadata.width, decoded.metadata.height), (W, H));
        assert_eq!(read_metadata(&path).unwrap(), decoded.metadata);
    }
}

#[test]
fn webp_bmp_and_gif_decode_too() {
    let dir = tempfile::tempdir().unwrap();
    let scene = kit::scene(W, H);
    for name in ["a.webp", "a.bmp", "a.gif"] {
        let path = dir.path().join(name);
        scene.save(&path).unwrap();
        let decoded = decode(&path).unwrap();
        assert_eq!(
            (decoded.image.width, decoded.image.height),
            (W, H),
            "{name}"
        );
        assert_eq!(decoded.image.color, SRGB);
        if name != "a.gif" {
            // WebP (lossless here) and BMP keep every code value; GIF is
            // limited to a palette.
            assert_eq!(
                decoded.image.pixels,
                linearised_8(&scene, RgbSpace::Srgb),
                "{name}"
            );
        }
    }
}

#[test]
fn eight_bit_pixels_are_linearised_exactly_as_core_decodes_srgb() {
    let dir = tempfile::tempdir().unwrap();
    let decoded = decode(kit::write_png8(dir.path(), "a.png", W, H)).unwrap();
    assert_eq!(
        decoded.image.pixels,
        linearised_8(&kit::scene(W, H), RgbSpace::Srgb)
    );
    // Spot values: black, white and the familiar middle grey (sRGB 118 is
    // 18 % reflectance).
    assert_eq!(linear_from_8(RgbSpace::Srgb, 0), 0);
    assert_eq!(linear_from_8(RgbSpace::Srgb, 255), 65535);
    let middle = f32::from(linear_from_8(RgbSpace::Srgb, 118)) / 65535.0;
    assert!((middle - 0.1812).abs() < 5e-4, "{middle}");
}

#[test]
fn sixteen_bit_sources_keep_more_than_eight_bits_of_precision() {
    let dir = tempfile::tempdir().unwrap();
    let scene = kit::scene16(W, H);
    let expected = linearised_16(&scene, RgbSpace::Srgb);
    for path in [
        kit::write_png16(dir.path(), "a.png", W, H),
        kit::write_tiff16(dir.path(), "a.tif", W, H),
    ] {
        let decoded = decode(&path).unwrap();
        // Exact: an 8-bit detour would be off by up to 1/255 of full scale.
        assert_eq!(decoded.image.pixels, expected, "{}", path.display());

        let mut levels: Vec<u16> = decoded.image.pixels.clone();
        levels.sort_unstable();
        levels.dedup();
        assert!(levels.len() > 2000, "only {} distinct levels", levels.len());
    }
    // And the 8-bit rendering of the same scene really is coarser.
    let coarse = linearised_8(&kit::scene(W, H), RgbSpace::Srgb);
    assert!(max_abs_diff(&coarse, &expected) > 100);
}

#[test]
fn jpeg_pixels_stay_close_to_the_scene() {
    let dir = tempfile::tempdir().unwrap();
    let decoded = decode(kit::write_jpeg(dir.path(), "a.jpg", W, H)).unwrap();
    let expected = linearised_8(&kit::scene(W, H), RgbSpace::Srgb);
    let error = mean_abs_diff(&decoded.image.pixels, &expected) / 65535.0;
    assert!(error < 0.01, "mean linear error {error}");
}

#[test]
fn grey_images_expand_to_rgb() {
    let dir = tempfile::tempdir().unwrap();
    let grey8 = GrayImage::from_fn(16, 8, |x, y| Luma([(x * 16 + y) as u8]));
    let grey16: ImageBuffer<Luma<u16>, Vec<u16>> =
        ImageBuffer::from_fn(16, 8, |x, y| Luma([(x * 4000 + y * 37) as u16]));
    let (path8, path16) = (dir.path().join("g8.png"), dir.path().join("g16.png"));
    grey8.save(&path8).unwrap();
    grey16.save(&path16).unwrap();

    let decoded8 = decode(&path8).unwrap().image;
    let decoded16 = decode(&path16).unwrap().image;
    for y in 0..8 {
        for x in 0..16 {
            let want8 = linear_from_8(RgbSpace::Srgb, grey8.get_pixel(x, y).0[0]);
            assert_eq!(decoded8.pixel(x, y), [want8; 3]);
            let want16 = linear_from_16(RgbSpace::Srgb, grey16.get_pixel(x, y).0[0]);
            assert_eq!(decoded16.pixel(x, y), [want16; 3]);
        }
    }
}

#[test]
fn alpha_is_composited_over_white_in_linear_light() {
    let dir = tempfile::tempdir().unwrap();
    // Columns: opaque, half transparent, fully transparent. Colour: mid red.
    let alphas = [255u8, 128, 0];
    let rgba = RgbaImage::from_fn(3, 2, |x, _| Rgba([200, 40, 10, alphas[x as usize]]));
    let grey_alpha = GrayAlphaImage::from_fn(3, 2, |x, _| LumaA([90, alphas[x as usize]]));
    let (rgba_path, grey_path) = (dir.path().join("rgba.png"), dir.path().join("la.png"));
    rgba.save(&rgba_path).unwrap();
    grey_alpha.save(&grey_path).unwrap();

    let blend = |code: u8, alpha: u8| {
        let (linear, coverage) = (
            RgbSpace::Srgb.decode(f32::from(code) / 255.0),
            f32::from(alpha) / 255.0,
        );
        ((linear * coverage + (1.0 - coverage)) * 65535.0).round() as u16
    };
    let colour = decode(&rgba_path).unwrap().image;
    let grey = decode(&grey_path).unwrap().image;
    for (x, &alpha) in alphas.iter().enumerate() {
        let x = x as u32;
        let want = [blend(200, alpha), blend(40, alpha), blend(10, alpha)];
        for (got, want) in colour.pixel(x, 1).iter().zip(want) {
            assert!(
                got.abs_diff(want) <= 1,
                "alpha {alpha}: {:?} vs {want}",
                colour.pixel(x, 1)
            );
        }
        assert!(grey.pixel(x, 0)[1].abs_diff(blend(90, alpha)) <= 1);
        assert_eq!(grey.pixel(x, 0)[0], grey.pixel(x, 0)[2]);
    }
    assert_eq!(colour.pixel(2, 0), [65535; 3], "fully transparent is white");
    assert_eq!(
        colour.pixel(0, 0),
        [
            linear_from_8(RgbSpace::Srgb, 200),
            linear_from_8(RgbSpace::Srgb, 40),
            linear_from_8(RgbSpace::Srgb, 10)
        ]
    );
}

#[test]
fn sixteen_bit_alpha_is_composited_too() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("rgba16.png");
    let image: ImageBuffer<Rgba<u16>, Vec<u16>> = ImageBuffer::from_fn(2, 1, |x, _| {
        Rgba([30000, 30000, 30000, if x == 0 { 65535 } else { 0 }])
    });
    image.save(&path).unwrap();
    let decoded = decode(&path).unwrap().image;
    assert_eq!(
        decoded.pixel(0, 0),
        [linear_from_16(RgbSpace::Srgb, 30000); 3]
    );
    assert_eq!(decoded.pixel(1, 0), [65535; 3]);
}

#[test]
fn an_embedded_profile_selects_the_colour_space_and_its_transfer_function() {
    let dir = tempfile::tempdir().unwrap();
    let wide = [
        RgbSpace::DisplayP3,
        RgbSpace::AdobeRgb,
        RgbSpace::ProPhoto,
        RgbSpace::Rec2020,
    ];
    for space in wide {
        for extension in ["png", "tif", "jpg"] {
            let name = format!("{space:?}.{extension}");
            let decoded = decode(kit::write_tagged(dir.path(), &name, W, H, space)).unwrap();
            assert_eq!(decoded.image.color, SourceColor::Rgb(space), "{name}");
            if extension == "jpg" {
                let expected = linearised_8(&kit::scene(W, H), space);
                let error = mean_abs_diff(&decoded.image.pixels, &expected) / 65535.0;
                assert!(error < 0.01, "{name}: mean linear error {error}");
            } else {
                assert_eq!(
                    decoded.image.pixels,
                    linearised_16(&kit::scene16(W, H), space),
                    "{name}"
                );
            }
        }
    }
}

#[test]
fn an_srgb_profile_and_an_unknown_profile_both_mean_srgb() {
    let dir = tempfile::tempdir().unwrap();
    let tagged = decode(kit::write_tagged(
        dir.path(),
        "srgb.png",
        32,
        24,
        RgbSpace::Srgb,
    ))
    .unwrap();
    assert_eq!(tagged.image.color, SRGB);

    // A profile whose primaries match nothing we know: swap two colorants.
    let mut odd = kit::icc_profile(RgbSpace::AdobeRgb);
    let tag_offset = |signature: &[u8; 4]| {
        let count = u32::from_be_bytes(odd[128..132].try_into().unwrap()) as usize;
        (0..count)
            .map(|index| 132 + index * 12)
            .find(|&at| &odd[at..at + 4] == signature)
            .map(|at| u32::from_be_bytes(odd[at + 4..at + 8].try_into().unwrap()) as usize)
            .unwrap()
    };
    let (red, blue) = (tag_offset(b"rXYZ"), tag_offset(b"bXYZ"));
    for byte in 8..20 {
        odd.swap(red + byte, blue + byte);
    }
    let path = dir.path().join("odd.png");
    write_png_with_profile(&path, odd);
    assert_eq!(decode(&path).unwrap().image.color, SRGB);

    // Garbage in place of a profile is ignored rather than fatal.
    let path = dir.path().join("junk.png");
    write_png_with_profile(&path, vec![0xab; 300]);
    assert_eq!(decode(&path).unwrap().image.color, SRGB);
}

fn write_png_with_profile(path: &Path, profile: Vec<u8>) {
    use image::ImageEncoder;
    let scene = kit::scene(32, 24);
    let mut bytes = Vec::new();
    let mut encoder = image::codecs::png::PngEncoder::new(&mut bytes);
    encoder.set_icc_profile(profile).unwrap();
    encoder
        .write_image(scene.as_raw(), 32, 24, image::ExtendedColorType::Rgb8)
        .unwrap();
    std::fs::write(path, bytes).unwrap();
}

#[test]
fn float_tiff_samples_are_taken_as_linear_light() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("float.tif");
    let values = [-0.25f32, 0.0, 0.18, 0.5, 1.0, 4.0];
    let image: ImageBuffer<Rgb<f32>, Vec<f32>> =
        ImageBuffer::from_fn(values.len() as u32, 1, |x, _| Rgb([values[x as usize]; 3]));
    image.save(&path).unwrap();
    let decoded = decode(&path).unwrap().image;
    for (x, value) in values.iter().enumerate() {
        let want = (value.clamp(0.0, 1.0) * 65535.0).round() as u16;
        assert_eq!(decoded.pixel(x as u32, 0), [want; 3], "sample {value}");
    }
    assert_eq!(decoded.color, SRGB);
}

#[test]
fn the_content_decides_the_format_not_the_extension() {
    let dir = tempfile::tempdir().unwrap();
    let png = kit::write_png8(dir.path(), "real.png", 40, 30);
    let disguised = dir.path().join("disguised.jpg");
    std::fs::copy(&png, &disguised).unwrap();
    let decoded = decode(&disguised).unwrap();
    assert_eq!(
        decoded.image.pixels,
        linearised_8(&kit::scene(40, 30), RgbSpace::Srgb)
    );
    assert_eq!(read_metadata(&disguised).unwrap().width, 40);
}

#[test]
fn a_single_pixel_image_decodes() {
    let dir = tempfile::tempdir().unwrap();
    let decoded = decode(kit::write_png8(dir.path(), "dot.png", 1, 1)).unwrap();
    assert_eq!((decoded.image.width, decoded.image.height), (1, 1));
    assert_eq!(decoded.image.pixels.len(), 3);
}
