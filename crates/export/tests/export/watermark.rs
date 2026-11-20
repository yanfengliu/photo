//! The watermark: it changes pixels only around its anchor, scales with the
//! picture, and honours its colour and opacity.

use image::ImageBuffer;
use photo_core::colorimetry::RgbSpace;
use photo_develop::CancelToken;
use photo_export::{Anchor, BitDepth, ExportSettings, Format, Resize, Watermark, export};

use crate::common::{Fixture, open, written};

type Image16 = ImageBuffer<image::Rgb<u16>, Vec<u16>>;

fn exported(fixture: &Fixture, settings: &ExportSettings, name: &str) -> Image16 {
    let mut settings = settings.clone();
    settings.format = Format::Png {
        depth: BitDepth::Sixteen,
    };
    settings.naming.template = name.into();
    open(&written(&export(&fixture.item(), &settings, &CancelToken::new()).unwrap()).0).into_rgb16()
}

/// The smallest rectangle `(x0, y0, x1, y1)` holding every pixel that
/// differs, or `None` when none does.
fn changed(a: &Image16, b: &Image16) -> Option<(u32, u32, u32, u32)> {
    let mut bounds: Option<(u32, u32, u32, u32)> = None;
    for (x, y, pixel) in a.enumerate_pixels() {
        if pixel != b.get_pixel(x, y) {
            bounds = Some(match bounds {
                None => (x, y, x + 1, y + 1),
                Some((x0, y0, x1, y1)) => (x0.min(x), y0.min(y), x1.max(x + 1), y1.max(y + 1)),
            });
        }
    }
    bounds
}

fn mark(anchor: Anchor) -> Watermark {
    Watermark {
        text: "© Photo 2026".into(),
        anchor,
        size: 6.0,
        opacity: 80.0,
        color: [255, 255, 255],
    }
}

#[test]
fn the_watermark_changes_pixels_only_around_its_anchor() {
    let fixture = Fixture::new(600, 400);
    let mut settings = fixture.settings_out();
    let plain = exported(&fixture, &settings, "plain");
    for anchor in Anchor::ALL {
        settings.watermark = Some(mark(anchor));
        let marked = exported(&fixture, &settings, &format!("{anchor:?}"));
        let (x0, y0, x1, y1) =
            changed(&plain, &marked).unwrap_or_else(|| panic!("{anchor:?} drew nothing"));
        let (fx, fy) = anchor.fractions();
        // The changed area sits in the anchor's third of the picture, clear
        // of the edges it is anchored to.
        let centre = (
            (x0 + x1) as f32 / 2.0 / 600.0,
            (y0 + y1) as f32 / 2.0 / 400.0,
        );
        assert!(
            (centre.0 - (0.15 + 0.7 * fx)).abs() < 0.15,
            "{anchor:?}: x {x0}..{x1}"
        );
        assert!(
            (centre.1 - (0.05 + 0.9 * fy)).abs() < 0.06,
            "{anchor:?}: y {y0}..{y1}"
        );
        assert!(
            x0 >= 4 && y0 >= 4 && x1 <= 596 && y1 <= 396,
            "{anchor:?}: inset from the edges"
        );
        assert!(
            y1 - y0 < 45 && x1 - x0 < 300,
            "{anchor:?}: {x0},{y0} {x1},{y1}"
        );
    }
}

#[test]
fn the_watermark_scales_with_the_picture() {
    let mut fixture = Fixture::new(800, 600);
    fixture.settings = Default::default();
    let mut settings = fixture.settings_out();
    let mut boxes = Vec::new();
    for long_edge in [400u32, 800] {
        settings.sizing.resize = Resize::LongEdge { pixels: long_edge };
        settings.watermark = None;
        let plain = exported(&fixture, &settings, &format!("plain{long_edge}"));
        settings.watermark = Some(mark(Anchor::BottomRight));
        let marked = exported(&fixture, &settings, &format!("marked{long_edge}"));
        boxes.push(changed(&plain, &marked).unwrap());
    }
    let (small, large) = (boxes[0], boxes[1]);
    for (a, b) in [
        (small.0, large.0),
        (small.1, large.1),
        (small.2, large.2),
        (small.3, large.3),
    ] {
        assert!(
            (f64::from(b) - 2.0 * f64::from(a)).abs() <= 4.0,
            "{small:?} vs {large:?}"
        );
    }
}

#[test]
fn colour_and_opacity_are_blended_in_linear_light() {
    // A flat mid-grey picture shows exactly what the text does.
    let grey = 0.2f32;
    let fixture = Fixture::from_fn(400, 200, |_, _| [grey; 3]);
    let mut settings = fixture.settings_out();
    settings.color_space = RgbSpace::Srgb;
    let mut text = Watermark {
        text: "I".into(),
        anchor: Anchor::Center,
        size: 40.0,
        opacity: 100.0,
        color: [255, 0, 0],
    };
    let linear = |image: &Image16, x: u32, y: u32| {
        image
            .get_pixel(x, y)
            .0
            .map(|v| RgbSpace::Srgb.decode(f32::from(v) / 65535.0))
    };
    // The middle of the I's stem is fully covered.
    settings.watermark = Some(text.clone());
    let opaque = exported(&fixture, &settings, "opaque");
    let centre = linear(&opaque, 200, 100);
    assert!(
        (centre[0] - 1.0).abs() < 1e-3 && centre[1] < 1e-3 && centre[2] < 1e-3,
        "{centre:?}"
    );
    text.opacity = 25.0;
    settings.watermark = Some(text);
    let faint = exported(&fixture, &settings, "faint");
    let centre = linear(&faint, 200, 100);
    let expected = [grey + 0.25 * (1.0 - grey), grey * 0.75, grey * 0.75];
    for (got, want) in centre.iter().zip(expected) {
        assert!((got - want).abs() < 2e-3, "{centre:?} vs {expected:?}");
    }
    // Away from the text the picture is untouched.
    assert_eq!(faint.get_pixel(20, 20), opaque.get_pixel(20, 20));
    assert!((linear(&faint, 20, 20)[0] - grey).abs() < 1e-3);
}
