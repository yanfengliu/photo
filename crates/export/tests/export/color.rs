//! Colour spaces: every file carries the profile of its space, the profile
//! describes photo-core's primaries, white and transfer function, and the
//! pixel values are those of that space.

use photo_core::colorimetry::{RgbSpace, mat3_apply};
use photo_core::image::SourceColor;
use photo_develop::CancelToken;
use photo_export::{BitDepth, Format, MetadataMode, export};

use crate::common::{Fixture, icc_profile, open, written};
use crate::icc_reader::Profile;

#[test]
fn every_format_embeds_a_profile_that_describes_its_space() {
    let fixture = Fixture::new(64, 48);
    let cancel = CancelToken::new();
    for space in RgbSpace::ALL {
        for format in [
            Format::Jpeg { quality: 90 },
            Format::Png {
                depth: BitDepth::Sixteen,
            },
            Format::Tiff {
                depth: BitDepth::Eight,
            },
        ] {
            for include in [MetadataMode::All, MetadataMode::None] {
                let mut settings = fixture.settings_out();
                settings.format = format;
                settings.color_space = space;
                settings.metadata.include = include;
                let (path, _, _) = written(&export(&fixture.item(), &settings, &cancel).unwrap());
                let bytes = icc_profile(&path)
                    .unwrap_or_else(|| panic!("{format:?} {space:?} {include:?}: no profile"));
                check_profile(&Profile::parse(&bytes), space);
                // The reader of this app recognises the space again.
                let decoded = photo_decode::decode(&path).unwrap();
                assert_eq!(decoded.image.color, SourceColor::Rgb(space), "{format:?}");
                std::fs::remove_file(path).unwrap();
            }
        }
    }
}

fn check_profile(profile: &Profile, space: RgbSpace) {
    assert_eq!(profile.version, [4, 0x30], "{space:?}");
    assert_eq!(&profile.class, b"mntr");
    assert!(!profile.description.is_empty());
    // A version 4 display profile's media white is the PCS illuminant.
    for (stored, d50) in profile.white_point.iter().zip([0.9642, 1.0, 0.8249]) {
        assert!(
            (stored - d50).abs() < 1e-4,
            "{space:?}: {:?}",
            profile.white_point
        );
    }
    let ([red, green, blue], white) = profile.chromaticities();
    let expected = space.primaries();
    for (name, got, want) in [
        ("red", red, expected.red),
        ("green", green, expected.green),
        ("blue", blue, expected.blue),
        ("white", white, expected.white),
    ] {
        assert!(
            (got.x - want.x).abs() < 5e-4 && (got.y - want.y).abs() < 5e-4,
            "{space:?} {name}: {got:?} vs {want:?}"
        );
    }
    for curve in &profile.curves {
        for step in 0..=1000 {
            let encoded = step as f64 / 1000.0;
            let want = f64::from(space.decode(encoded as f32));
            let got = curve.eval(encoded);
            assert!(
                (got - want).abs() < 2e-4,
                "{space:?} at {encoded}: {got} vs {want}"
            );
        }
    }
}

/// A saturated red inside sRGB, exported in sRGB and in Display P3: in P3,
/// whose red primary is more saturated, the same colour needs less red and
/// a little green and blue; read back through each file's own profile, both
/// are the same colour.
#[test]
fn a_saturated_colour_moves_toward_the_middle_of_the_wider_space() {
    let red = [0.7f32, 0.03, 0.02];
    let fixture = Fixture::from_fn(32, 32, |_, _| red);
    let cancel = CancelToken::new();
    let read = |space: RgbSpace| {
        let mut settings = fixture.settings_out();
        settings.format = Format::Png {
            depth: BitDepth::Sixteen,
        };
        settings.color_space = space;
        settings.naming.template = format!("{space:?}");
        let (path, _, _) = written(&export(&fixture.item(), &settings, &cancel).unwrap());
        let code = open(&path).into_rgb16().get_pixel(16, 16).0;
        let profile = Profile::parse(&icc_profile(&path).unwrap());
        let linear: [f32; 3] =
            std::array::from_fn(|c| profile.curves[c].eval(f64::from(code[c]) / 65535.0) as f32);
        (code, mat3_apply(&profile.colorants, linear))
    };
    let (srgb, srgb_xyz) = read(RgbSpace::Srgb);
    let (p3, p3_xyz) = read(RgbSpace::DisplayP3);
    assert!(p3[0] < srgb[0], "red: P3 {p3:?} vs sRGB {srgb:?}");
    assert!(p3[1] > srgb[1], "green: P3 {p3:?} vs sRGB {srgb:?}");
    assert!(p3[2] > srgb[2], "blue: P3 {p3:?} vs sRGB {srgb:?}");
    for (a, b) in srgb_xyz.iter().zip(p3_xyz) {
        assert!((a - b).abs() < 2e-3, "{srgb_xyz:?} vs {p3_xyz:?}");
    }
}
