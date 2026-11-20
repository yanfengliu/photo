//! The sensor layouts other than a plain Bayer mosaic.

use super::*;

#[test]
fn a_flat_x_trans_field_comes_back_exactly() {
    let camera = camera(X_TRANS, "RGB");
    let cfa = camera.cfa.clone();
    // An active area that starts in the middle of the 6 x 6 pattern, and a
    // sensor too narrow for rawler's own interpolators.
    for (width, height, active) in [
        (36usize, 30usize, None),
        (41, 33, Some((5usize, 3usize, 32usize, 26usize))),
        (7, 40, None),
    ] {
        let cfa = cfa.clone();
        let mut raw = mosaic(camera.clone(), width, height, GAINS, |row, column| {
            FLAT[cfa.color_at(row, column)]
        });
        let (x, y, wide, high) = active.unwrap_or((0, 0, width, height));
        raw.active_area = Some(Rect::new(Point::new(x, y), Dim2::new(wide, high)));
        let image = develop_upright(raw);
        assert_eq!((image.width as usize, image.height as usize), (wide, high));
        for pixel in image.pixels.as_chunks::<3>().0 {
            assert_close(
                *pixel,
                FLAT.map(expected),
                1,
                &format!("x-trans {width} x {height}"),
            );
        }
        assert_eq!(camera_color(&image).as_shot_gains, [2.2, 1.0, 1.6]);
    }
}

#[test]
fn three_sample_raw_data_is_scaled_cropped_and_left_as_it_is() {
    let (width, height) = (12usize, 10usize);
    let pixel = |row: usize, column: usize| [column as f32 / 16.0, row as f32 / 16.0, 0.25];
    let data: Vec<u16> = (0..height)
        .flat_map(|row| (0..width).map(move |column| (row, column)))
        .flat_map(|(row, column)| pixel(row, column).map(code))
        .collect();
    let mut raw = RawImage::new_with_data(
        camera("", "RGB"),
        RawImageData::Integer(data),
        width * 3,
        height,
        3,
        [1.0, 1.0, 1.0, f32::NAN],
        RawPhotometricInterpretation::LinearRaw,
        Some(BlackLevel::new(&[BLACK; 3], 1, 1, 3)),
        Some(WhiteLevel::new(vec![u32::from(WHITE); 3])),
        false,
    );
    raw.crop_area = Some(Rect::new(Point::new(2, 1), Dim2::new(8, 6)));
    let image = develop_upright(raw);
    assert_eq!((image.width, image.height), (8, 6));
    for (x, y) in [(0u32, 0u32), (7, 5), (3, 2)] {
        let wanted = pixel(y as usize + 1, x as usize + 2).map(expected);
        assert_eq!(image.pixel(x, y), wanted, "at {x},{y}");
    }
    assert_eq!(camera_color(&image).as_shot_gains, [1.0; 3]);
}

#[test]
fn a_monochrome_sensor_is_grey_in_all_three_channels() {
    let raw = mosaic(camera("", "RGB"), 10, 8, [f32::NAN; 4], |row, column| {
        (row * 10 + column) as f32 / 80.0
    });
    let image = develop_upright(raw);
    assert_eq!((image.width, image.height), (10, 8));
    for (x, y) in [(0u32, 0u32), (9, 7), (4, 3)] {
        let level = expected((y * 10 + x) as f32 / 80.0);
        assert_eq!(image.pixel(x, y), [level; 3]);
    }
    let color = camera_color(&image);
    assert_eq!(color.as_shot_gains, [1.0; 3]);
    assert_eq!(color.xyz_to_camera, RgbSpace::Srgb.from_xyz());
}

#[test]
fn a_four_colour_sensor_is_folded_into_a_three_channel_camera() {
    // Canon PowerShot G1: a GMYC mosaic with planes ordered G, M, C, Y.
    let g1: [[f32; 3]; 4] = [
        [-0.5686, 1.03, 0.2223],
        [0.4725, -0.1157, 0.4383],
        [-0.6128, 1.0783, 0.6163],
        [-0.2688, 1.2093, 0.0604],
    ];
    let response =
        |xyz: [f32; 3]| g1.map(|row| row.iter().zip(xyz).map(|(a, b)| a * b).sum::<f32>());
    let xyz = [0.30, 0.34, 0.20];
    let sensor = response(xyz);
    let white = response([0.9642, 1.0, 0.8251]);

    let mut camera = camera("GMYC", "GMCY");
    camera.color_matrix =
        HashMap::from([(Illuminant::D65, g1.iter().flatten().copied().collect())]);
    let cfa = camera.cfa.clone();
    let planes = camera.plane_color.plane_lookup_table();
    let raw = mosaic(camera, 20, 16, white.map(|w| 1.0 / w), |row, column| {
        sensor[planes[cfa.color_at(row, column)]]
    });
    let image = develop_upright(raw);
    assert_eq!((image.width, image.height), (20, 16));

    // Every pixel is what the virtual camera's matrix predicts for the
    // colour, and that camera's gains neutralize the as-shot white.
    let color = camera_color(&image);
    let predicted: [f32; 3] = std::array::from_fn(|c| {
        color.xyz_to_camera[c]
            .iter()
            .zip(xyz)
            .map(|(a, b)| a * b)
            .sum()
    });
    for (x, y) in [(0, 0), (19, 15), (9, 7)] {
        let got = image.pixel(x, y).map(|v| f32::from(v) / 65535.0);
        for (g, p) in got.iter().zip(predicted) {
            assert!((g - p).abs() < 2e-3, "{got:?} vs {predicted:?}");
        }
    }
    let white_seen: [f32; 3] = std::array::from_fn(|c| {
        let cone: f32 = color.xyz_to_camera[c]
            .iter()
            .zip([0.9642, 1.0, 0.8251])
            .map(|(a, b)| a * b)
            .sum();
        cone * color.as_shot_gains[c]
    });
    assert!(
        (white_seen[0] - white_seen[1]).abs() < 2e-3
            && (white_seen[2] - white_seen[1]).abs() < 2e-3
    );
}

#[test]
fn fuji_rotated_data_is_turned_back_and_cropped() {
    let camera = camera("RGGB", "RGB");
    let cfa = camera.cfa.clone();
    let mut raw = mosaic(camera, 64, 64, GAINS, |row, column| {
        FLAT[cfa.color_at(row, column)]
    });
    raw.fuji_rotation_width = Some(40);
    // 40 and 24 times the square root of two.
    let image = develop_upright(raw.clone());
    assert_eq!((image.width, image.height), (56, 33));
    assert_close(
        image.pixel(28, 16),
        FLAT.map(expected),
        2,
        "centre of the turned picture",
    );

    raw.crop_area = Some(Rect::new(Point::new(20, 10), Dim2::new(16, 12)));
    let cropped = develop_upright(raw);
    assert_eq!((cropped.width, cropped.height), (16, 12));
    assert_close(
        cropped.pixel(8, 6),
        FLAT.map(expected),
        2,
        "centre of the crop",
    );
}
