//! Contracts of the output transform (DESIGN §5.8).

use photo_color::output::OutputTransform;
use photo_core::colorimetry::{RgbSpace, mat3_apply, rgb_to_rgb};

fn in_gamut_samples() -> Vec<[f32; 3]> {
    // Colours inside sRGB (the smallest space), expressed in working RGB.
    let to_working = rgb_to_rgb(RgbSpace::Srgb, RgbSpace::Rec2020);
    let mut samples = Vec::new();
    for r in 0..=4 {
        for g in 0..=4 {
            for b in 0..=4 {
                samples.push(mat3_apply(
                    &to_working,
                    [r as f32 / 4.0, g as f32 / 4.0, b as f32 / 4.0],
                ));
            }
        }
    }
    samples
}

#[test]
fn in_gamut_colours_are_converted_and_encoded_exactly() {
    for space in RgbSpace::ALL {
        let output = OutputTransform::new(space);
        assert_eq!(output.space(), space);
        let matrix = rgb_to_rgb(RgbSpace::Rec2020, space);
        for working in in_gamut_samples() {
            // Compared in linear light: a pure-gamma encoding has an
            // infinite slope at black, which would magnify the last bit of
            // a channel that sits exactly on the gamut boundary.
            let expected = mat3_apply(&matrix, working).map(|v| v.clamp(0.0, 1.0));
            let linear = output.to_linear(working);
            for (a, b) in linear.iter().zip(expected) {
                assert!(
                    (a - b).abs() < 2e-6,
                    "{space:?} {working:?}: {linear:?} vs {expected:?}"
                );
            }
            assert_eq!(
                output.encode(working),
                linear.map(|v| space.encode(v).clamp(0.0, 1.0))
            );
        }
    }
}

#[test]
fn the_end_points_and_middle_grey_land_on_their_codes() {
    for space in RgbSpace::ALL {
        let output = OutputTransform::new(space);
        assert_eq!(output.encode_u8([0.0; 3]), [0; 3]);
        assert_eq!(output.encode_u8([1.0; 3]), [255; 3]);
        assert_eq!(output.encode_u16([0.0; 3]), [0; 3]);
        assert_eq!(output.encode_u16([1.0; 3]), [65535; 3]);
        assert!((output.encode([1.0; 3])[0] - 1.0).abs() < 1e-6);
        assert_eq!(output.to_linear([1.0; 3]), [1.0; 3]);
    }
    assert_eq!(
        OutputTransform::new(RgbSpace::Srgb).encode_u8([0.18; 3]),
        [118; 3]
    );
}

#[test]
fn output_never_leaves_zero_to_one() {
    let wild = [
        [0.0, 0.0, 0.0],
        [-0.5, 0.2, 0.9],
        [-1.0, -1.0, -1.0],
        [4.0, 9.0, 16.0],
        [1.0e30, 0.0, 0.0],
        [0.0, 1.0, 0.0],
        [0.0, 0.0, 1.0],
        [1.0, 0.0, 0.0],
        [f32::NAN, 0.5, 0.5],
        [f32::INFINITY, 0.5, 0.5],
        [0.5, f32::NEG_INFINITY, 0.5],
    ];
    for space in RgbSpace::ALL {
        let output = OutputTransform::new(space);
        for working in wild {
            for values in [output.to_linear(working), output.encode(working)] {
                assert!(
                    values.iter().all(|v| (0.0..=1.0).contains(v)),
                    "{space:?} {working:?}: {values:?}"
                );
            }
            // The integer paths cannot leave their range by type; they must
            // simply not panic.
            let _ = (output.encode_u8(working), output.encode_u16(working));
        }
    }
}

#[test]
fn out_of_gamut_colours_are_desaturated_toward_their_luminance() {
    let output = OutputTransform::new(RgbSpace::Srgb);
    let matrix = rgb_to_rgb(RgbSpace::Rec2020, RgbSpace::Srgb);
    let luma = RgbSpace::Srgb.to_xyz()[1];
    let y = |rgb: [f32; 3]| rgb[0] * luma[0] + rgb[1] * luma[1] + rgb[2] * luma[2];
    // Saturated working-space colours that sRGB cannot hold.
    for working in [
        [0.1, 0.5, 0.1],
        [0.5, 0.02, 0.02],
        [0.05, 0.1, 0.5],
        [0.3, 0.3, 0.02],
    ] {
        let raw = mat3_apply(&matrix, working);
        assert!(
            raw.iter().any(|v| *v < 0.0),
            "{working:?} was meant to be out of gamut"
        );
        let mapped = output.to_linear(working);
        assert!(mapped.iter().all(|v| *v >= 0.0));
        assert!(
            mapped.iter().any(|v| *v < 1e-6),
            "desaturation stops as soon as the colour is in gamut"
        );
        assert!(
            (y(mapped) - y(raw)).abs() < 1e-5,
            "luminance is kept: {} vs {}",
            y(mapped),
            y(raw)
        );
        // The order of the channels, and so the hue family, is kept.
        for i in 0..3 {
            for j in 0..3 {
                if raw[i] > raw[j] {
                    assert!(mapped[i] >= mapped[j], "{raw:?} -> {mapped:?}");
                }
            }
        }
        // A per-channel clip would have changed the luminance.
        let clipped = raw.map(|v| v.max(0.0));
        assert!((y(clipped) - y(raw)).abs() > 10.0 * (y(mapped) - y(raw)).abs());
    }
}

#[test]
fn neutral_in_is_neutral_out_in_every_space() {
    for space in RgbSpace::ALL {
        let output = OutputTransform::new(space);
        for step in 0..=1000 {
            let level = step as f32 / 800.0;
            let bytes = output.encode_u8([level; 3]);
            assert!(
                bytes[0] == bytes[1] && bytes[1] == bytes[2],
                "{space:?} at {level}: {bytes:?}"
            );
            let words = output.encode_u16([level; 3]);
            assert!(
                words[0] == words[1] && words[1] == words[2],
                "{space:?} at {level}: {words:?}"
            );
        }
    }
}

#[test]
fn the_eight_bit_table_agrees_with_exact_rounding() {
    for space in RgbSpace::ALL {
        let output = OutputTransform::new(space);
        let (mut previous, mut exact, total) = (0u8, 0u32, 20_000u32);
        for step in 0..=total {
            let level = (step as f32 / total as f32).powi(2);
            let byte = output.encode_u8([level; 3])[0];
            let ideal = (space.encode(level) * 255.0).round();
            assert!(
                (f32::from(byte) - ideal).abs() <= 1.0,
                "{space:?} at {level}: {byte} vs {ideal}"
            );
            // Never off by a visible amount: within a tenth of a code value
            // of where exact rounding would switch.
            assert!(
                (f32::from(byte) - space.encode(level) * 255.0).abs() <= 0.6,
                "{space:?} at {level}"
            );
            exact += u32::from(f32::from(byte) == ideal);
            assert!(
                byte >= previous,
                "{space:?}: the ramp went backwards at {level}"
            );
            previous = byte;
        }
        assert!(
            exact * 100 >= total * 97,
            "{space:?}: only {exact} of {total} matched exact rounding"
        );
    }
}

#[test]
fn sixteen_bit_output_is_the_exact_transfer_function() {
    for space in RgbSpace::ALL {
        let output = OutputTransform::new(space);
        for step in 0..=5000 {
            let level = step as f32 / 5000.0;
            let word = output.encode_u16([level; 3])[0];
            assert_eq!(
                word,
                (space.encode(level) * 65535.0 + 0.5) as u16,
                "{space:?} at {level}"
            );
        }
    }
}

#[test]
fn a_wider_output_space_keeps_more_of_a_saturated_colour() {
    let saturated = [0.1, 0.5, 0.1];
    let narrow = OutputTransform::new(RgbSpace::Srgb).to_linear(saturated);
    let wide = OutputTransform::new(RgbSpace::Rec2020).to_linear(saturated);
    assert_eq!(wide, saturated, "the working space is its own output space");
    assert!(narrow.iter().any(|v| *v < 1e-6));
}
