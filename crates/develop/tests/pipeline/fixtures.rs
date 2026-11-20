//! Sources, renders and measurements shared by the contract tests.

use photo_core::colorimetry::{RgbSpace, mat3_apply, mat3_mul};
use photo_core::develop::DevelopSettings;
use photo_core::image::{CameraColor, Rgb16Image, Rgba8Image, SourceColor, SourceImage};
use photo_develop::{CancelToken, Quality, RenderRequest, Renderer, Source, geometry};
use photo_imaging::View;
use photo_testkit::{dng_camera, radiance, scene};

/// The testkit scene as an 8-bit sRGB file, and the source a decoder makes
/// of it: every code linearised into `u16`.
pub fn raster(width: u32, height: u32) -> (image::RgbImage, Source) {
    let file = scene(width, height);
    let source = raster_from(&file);
    (file, source)
}

/// The source a decoder makes of an 8-bit sRGB picture.
pub fn raster_from(file: &image::RgbImage) -> Source {
    let pixels = file
        .as_raw()
        .iter()
        .map(|&code| (RgbSpace::Srgb.decode(f32::from(code) / 255.0) * 65535.0).round() as u16)
        .collect();
    let image = SourceImage::new(
        file.width(),
        file.height(),
        pixels,
        SourceColor::Rgb(RgbSpace::Srgb),
    )
    .unwrap();
    Source::new(image)
}

/// A raster source from linear sRGB values, `0..1`.
pub fn raster_linear(width: u32, height: u32, f: impl Fn(u32, u32) -> [f32; 3]) -> Source {
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 3);
    for y in 0..height {
        for x in 0..width {
            for value in f(x, y) {
                pixels.push((value.clamp(0.0, 1.0) * 65535.0).round() as u16);
            }
        }
    }
    let color = SourceColor::Rgb(RgbSpace::Srgb);
    Source::new(SourceImage::new(width, height, pixels, color).unwrap())
}

/// The colour description of the testkit camera.
pub fn camera_color(baseline_exposure: f32) -> CameraColor {
    let camera = dng_camera();
    CameraColor {
        xyz_to_camera: camera.xyz_to_camera,
        as_shot_gains: camera.as_shot_gains(),
        baseline_exposure,
    }
}

/// What the testkit camera records of a scene given in linear sRGB (1.0 is
/// diffuse white), demosaiced as a decoder delivers it: camera-native,
/// scaled to the sensor's white level, not white balanced, clipped at 1.
pub fn camera_source(
    width: u32,
    height: u32,
    baseline_exposure: f32,
    f: impl Fn(u32, u32) -> [f32; 3],
) -> Source {
    let camera = dng_camera();
    let mut to_raw = mat3_mul(&camera.xyz_to_balanced_camera(), &RgbSpace::Srgb.to_xyz());
    for (row, neutral) in to_raw.iter_mut().zip(camera.as_shot_neutral) {
        for value in row.iter_mut() {
            *value *= neutral;
        }
    }
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 3);
    for y in 0..height {
        for x in 0..width {
            for value in mat3_apply(&to_raw, f(x, y)) {
                pixels.push((value.clamp(0.0, 1.0) * 65535.0).round() as u16);
            }
        }
    }
    let color = SourceColor::Camera(camera_color(baseline_exposure));
    Source::new(SourceImage::new(width, height, pixels, color).unwrap())
}

/// The testkit scene photographed by the testkit camera, with the DNG
/// fixture's baseline exposure.
pub fn camera_scene(width: u32, height: u32) -> Source {
    let baseline = dng_camera().baseline_exposure;
    camera_source(width, height, baseline, |x, y| {
        radiance(i64::from(x), i64::from(y), width, height)
    })
}

/// Settings edited from the defaults.
pub fn edit(change: impl FnOnce(&mut DevelopSettings)) -> DevelopSettings {
    let mut settings = DevelopSettings::default();
    change(&mut settings);
    settings
}

/// The whole output at full resolution.
pub fn full(source: &Source, settings: &DevelopSettings) -> View {
    View::full(&geometry(source, settings))
}

pub fn request<'a>(
    source: &'a Source,
    settings: &'a DevelopSettings,
    view: View,
) -> RenderRequest<'a> {
    RenderRequest {
        source,
        settings,
        view,
        quality: Quality::Full,
        space: RgbSpace::Srgb,
    }
}

/// An 8-bit sRGB render with a fresh renderer.
pub fn render(source: &Source, settings: &DevelopSettings, view: View) -> Rgba8Image {
    Renderer::new()
        .render_rgba8(&request(source, settings, view), &CancelToken::new())
        .expect("not cancelled")
}

/// A 16-bit sRGB render with a fresh renderer.
pub fn render16(source: &Source, settings: &DevelopSettings, view: View) -> Rgb16Image {
    Renderer::new()
        .render_rgb16(&request(source, settings, view), &CancelToken::new())
        .expect("not cancelled")
}

pub fn rgb(image: &Rgba8Image, x: u32, y: u32) -> [u8; 3] {
    let [r, g, b, _] = image.pixel(x, y);
    [r, g, b]
}

/// Mean code value per channel over a rectangle.
pub fn mean(image: &Rgba8Image, x: u32, y: u32, width: u32, height: u32) -> [f32; 3] {
    let mut sum = [0.0f64; 3];
    for row in y..y + height {
        for column in x..x + width {
            for (total, value) in sum.iter_mut().zip(rgb(image, column, row)) {
                *total += f64::from(value);
            }
        }
    }
    sum.map(|total| (total / f64::from(width * height)) as f32)
}

/// Mean luma (Rec. 709 weights on the codes) over a rectangle.
pub fn luma(image: &Rgba8Image, x: u32, y: u32, width: u32, height: u32) -> f32 {
    let [r, g, b] = mean(image, x, y, width, height);
    0.2126 * r + 0.7152 * g + 0.0722 * b
}

/// Standard deviation of luma codes over a rectangle: local contrast.
pub fn spread(image: &Rgba8Image, x: u32, y: u32, width: u32, height: u32) -> f32 {
    let values: Vec<f32> = (y..y + height)
        .flat_map(|row| (x..x + width).map(move |column| (column, row)))
        .map(|(column, row)| {
            let [r, g, b] = rgb(image, column, row).map(f32::from);
            0.2126 * r + 0.7152 * g + 0.0722 * b
        })
        .collect();
    let average = values.iter().sum::<f32>() / values.len() as f32;
    let variance = values.iter().map(|v| (v - average).powi(2)).sum::<f32>() / values.len() as f32;
    variance.sqrt()
}

/// Largest per-channel difference between two images of the same size.
pub fn max_difference(a: &Rgba8Image, b: &Rgba8Image) -> u8 {
    assert_eq!((a.width, a.height), (b.width, b.height), "sizes differ");
    a.pixels
        .iter()
        .zip(&b.pixels)
        .map(|(x, y)| x.abs_diff(*y))
        .max()
        .unwrap_or(0)
}

/// The `width` x `height` rectangle of `image` whose top-left is `(x, y)`.
pub fn crop(image: &Rgba8Image, x: u32, y: u32, width: u32, height: u32) -> Rgba8Image {
    let mut pixels = Vec::with_capacity(width as usize * height as usize * 4);
    for row in y..y + height {
        let start = (row as usize * image.width as usize + x as usize) * 4;
        pixels.extend_from_slice(&image.pixels[start..start + width as usize * 4]);
    }
    Rgba8Image {
        width,
        height,
        pixels,
        space: image.space,
    }
}

/// Colourfulness of a mean colour: the spread of its channels.
pub fn chroma(colour: [f32; 3]) -> f32 {
    colour.iter().cloned().fold(f32::MIN, f32::max)
        - colour.iter().cloned().fold(f32::MAX, f32::min)
}

/// A one-thread pool and a many-thread pool, for R4.
pub fn pools() -> (rayon::ThreadPool, rayon::ThreadPool) {
    let pool = |threads| {
        rayon::ThreadPoolBuilder::new()
            .num_threads(threads)
            .build()
            .unwrap()
    };
    (pool(1), pool(7))
}
