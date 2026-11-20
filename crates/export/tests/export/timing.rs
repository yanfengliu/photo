//! Wall-clock cost of exporting a 24 MP photo. Not a contract: run it by
//! hand with
//! `cargo test -p photo-export --release --test export timing -- --ignored --nocapture`.

use std::time::Instant;

use photo_core::develop::DevelopSettings;
use photo_develop::{CancelToken, Source};
use photo_export::{BitDepth, Format, OutputSharpening, Resize, export};

use crate::common::{Fixture, written};

/// The edit a photographer typically leaves on a photo.
fn typical() -> DevelopSettings {
    let mut settings = DevelopSettings::default();
    settings.tone.exposure = 0.35;
    settings.tone.contrast = 20.0;
    settings.tone.highlights = -40.0;
    settings.tone.shadows = 30.0;
    settings.tone.whites = 10.0;
    settings.tone.blacks = -5.0;
    settings.presence.vibrance = 20.0;
    settings.presence.saturation = 5.0;
    settings.detail.sharpening.amount = 40.0;
    settings
}

#[test]
#[ignore = "prints timings; run on demand with --ignored --nocapture"]
fn timings_of_a_24_mp_export() {
    let (width, height) = (6000, 4000);
    let start = Instant::now();
    let mut fixture = Fixture::new(8, 8);
    let scene = photo_testkit::scene16(width, height);
    let path = fixture.dir.path().join("big.png");
    scene.save(&path).unwrap();
    println!(
        "\nwriting a {width} x {height} 16-bit PNG original: {:.2} s",
        start.elapsed().as_secs_f64()
    );
    let start = Instant::now();
    let decoded = photo_decode::decode(&path).unwrap();
    println!("decoding it: {:.2} s", start.elapsed().as_secs_f64());
    let start = Instant::now();
    fixture.source = Source::new(decoded.image);
    println!(
        "building the source (pyramid, analysis): {:.2} s",
        start.elapsed().as_secs_f64()
    );
    fixture.original = path;
    fixture.settings = typical();

    let cancel = CancelToken::new();
    for (name, format, resize, sharpening) in [
        (
            "JPEG q90, full size",
            Format::Jpeg { quality: 90 },
            Resize::Original,
            OutputSharpening::Off,
        ),
        (
            "TIFF 16-bit, full size",
            Format::Tiff {
                depth: BitDepth::Sixteen,
            },
            Resize::Original,
            OutputSharpening::Off,
        ),
        (
            "PNG 16-bit, full size",
            Format::Png {
                depth: BitDepth::Sixteen,
            },
            Resize::Original,
            OutputSharpening::Off,
        ),
        (
            "JPEG q85, 2048 long edge, sharpened",
            Format::Jpeg { quality: 85 },
            Resize::LongEdge { pixels: 2048 },
            OutputSharpening::Standard,
        ),
    ] {
        let mut settings = fixture.settings_out();
        settings.format = format;
        settings.sizing.resize = resize;
        settings.sharpening = sharpening;
        let start = Instant::now();
        let outcome = export(&fixture.item(), &settings, &cancel).unwrap();
        let elapsed = start.elapsed().as_secs_f64();
        let (path, w, h) = written(&outcome);
        let bytes = std::fs::metadata(&path).unwrap().len();
        println!(
            "{name:<38} {w} x {h}: {elapsed:6.2} s, {:.1} MB",
            bytes as f64 / 1e6
        );
    }
}
