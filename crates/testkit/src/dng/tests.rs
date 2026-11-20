use super::*;
use crate::scene::patches;
use crate::tiff::read;
use photo_core::colorimetry::mat3_invert;

fn load(spec: &DngSpec, width: u32, height: u32) -> Vec<u8> {
    let dir = tempfile::tempdir().unwrap();
    let path = write_dng_with(dir.path(), "fixture.dng", width, height, spec);
    std::fs::read(path).unwrap()
}

fn plain() -> DngSpec {
    DngSpec {
        embed_previews: false,
        ..DngSpec::default()
    }
}

/// The two values of a two-element tag.
fn pair(ifd: &std::collections::BTreeMap<u16, read::Entry>, tag: u16) -> (u32, u32) {
    (ifd[&tag].u32_at(0), ifd[&tag].u32_at(1))
}

#[test]
fn the_as_shot_neutral_is_a_warm_daylight_balance() {
    let camera = dng_camera();
    let [r, g, b] = camera.as_shot_neutral;
    assert_eq!(g, 1.0);
    assert!((0.35..0.5).contains(&r), "red {r}");
    assert!((0.55..0.7).contains(&b), "blue {b}");
    let gains = camera.as_shot_gains();
    assert!(gains[0] > gains[2] && gains[2] > 1.0, "{gains:?}");
}

#[test]
fn the_balanced_matrix_sends_d65_white_to_unity() {
    let camera = dng_camera();
    let d65 = Chromaticity::D65.to_xyz().map(|v| v as f32);
    let white = mat3_apply(&camera.xyz_to_balanced_camera(), d65);
    for channel in white {
        assert!((channel - 1.0).abs() < 1e-5, "{white:?}");
    }
}

#[test]
fn the_minimal_layout_keeps_the_raw_image_in_the_first_directory() {
    let dng = load(&plain(), 64, 48);
    let ifd0 = read::ifd(&dng, read::first_ifd(&dng));
    assert_eq!(ifd0[&0x00fe].u32_at(0), 0);
    assert_eq!(ifd0[&0x0106].u32_at(0), 32803);
    assert_eq!(ifd0[&0xc612].bytes, [1, 4, 0, 0]);
    assert!(!ifd0.contains_key(&0x014a));

    let (width, height) = (ifd0[&0x0100].u32_at(0), ifd0[&0x0101].u32_at(0));
    assert_eq!((width, height), (64 + 8 + 8, 48 + 8 + 4));
    assert_eq!(ifd0[&0x0117].u32_at(0), width * height * 2);
    let strip_end = ifd0[&0x0111].u32_at(0) + ifd0[&0x0117].u32_at(0);
    assert!(strip_end as usize <= dng.len());

    // Active area (top, left, bottom, right) and the crop inside it.
    let area: Vec<u32> = (0..4).map(|i| ifd0[&0xc68d].u32_at(i)).collect();
    assert_eq!(area, [3, 5, 3 + 48 + 8, 5 + 64 + 8]);
    assert_eq!(pair(&ifd0, 0xc61f), (4, 4));
    assert_eq!(pair(&ifd0, 0xc620), (64, 48));
}

#[test]
fn the_converter_layout_links_raw_and_preview_from_a_thumbnail() {
    let dng = load(&DngSpec::default(), 64, 48);
    let ifd0 = read::ifd(&dng, read::first_ifd(&dng));
    assert_eq!(ifd0[&0x00fe].u32_at(0), 1, "a reduced image comes first");
    assert_eq!((ifd0[&0x0100].u32_at(0), ifd0[&0x0101].u32_at(0)), (8, 6));
    assert_eq!(ifd0[&0x014a].count, 2);

    let raw = read::ifd(&dng, ifd0[&0x014a].u32_at(0));
    assert_eq!(raw[&0x00fe].u32_at(0), 0);
    assert_eq!(raw[&0x0106].u32_at(0), 32803);
    assert!(!raw.contains_key(&0xc612), "camera tags stay in IFD 0");

    let preview = read::ifd(&dng, ifd0[&0x014a].u32_at(1));
    assert_eq!(preview[&0x00fe].u32_at(0), 1);
    assert_eq!(preview[&0x0103].u32_at(0), 7);
    let stored_size = (preview[&0x0100].u32_at(0), preview[&0x0101].u32_at(0));
    assert_eq!(stored_size, dng_preview_size(64, 48));
    let start = preview[&0x0111].u32_at(0) as usize;
    let jpeg = &dng[start..start + preview[&0x0117].u32_at(0) as usize];
    let decoded = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg).unwrap();
    assert_eq!((decoded.width(), decoded.height()), (32, 24));
}

#[test]
fn a_sideways_orientation_swaps_the_stored_crop() {
    let spec = DngSpec {
        exif: ExifSpec {
            orientation: 6,
            ..ExifSpec::sample()
        },
        embed_previews: false,
    };
    let dng = load(&spec, 64, 48);
    let ifd0 = read::ifd(&dng, read::first_ifd(&dng));
    assert_eq!(ifd0[&0x0112].u32_at(0), 6);
    assert_eq!(pair(&ifd0, 0xc620), (48, 64));
}

#[test]
fn the_masked_border_is_black_and_the_sun_saturates() {
    let dng = load(&plain(), 160, 120);
    let ifd0 = read::ifd(&dng, read::first_ifd(&dng));
    let width = ifd0[&0x0100].u32_at(0) as usize;
    let start = ifd0[&0x0111].u32_at(0) as usize;
    let sample = |x: usize, y: usize| {
        let at = start + (y * width + x) * 2;
        u16::from_le_bytes([dng[at], dng[at + 1]])
    };
    assert_eq!(sample(0, 0), BLACK_LEVEL);
    assert_eq!(sample(4, 2), BLACK_LEVEL);
    // The sun is at 76 %, 20 % of the picture; add crop margin and border.
    let (sun_x, sun_y) = (5 + 4 + 121, 3 + 4 + 24);
    for (dx, dy) in [(0, 0), (1, 0), (0, 1), (1, 1)] {
        assert_eq!(sample(sun_x + dx, sun_y + dy), WHITE_LEVEL);
    }
}

#[test]
fn undoing_the_camera_recovers_the_patch_colours() {
    // Read one red, one green and one blue sample from inside each patch
    // and take them back through the documented model.
    let (width, height) = (320u32, 240u32);
    let dng = load(&plain(), width, height);
    let ifd0 = read::ifd(&dng, read::first_ifd(&dng));
    let frame_width = ifd0[&0x0100].u32_at(0) as usize;
    let start = ifd0[&0x0111].u32_at(0) as usize;
    let camera = dng_camera();
    let range = f32::from(camera.white_level - camera.black_level);
    let sample = |x: usize, y: usize| {
        let at = start + (y * frame_width + x) * 2;
        let code = u16::from_le_bytes([dng[at], dng[at + 1]]);
        f32::from(code - camera.black_level) / range
    };
    let balanced_to_xyz = mat3_invert(&camera.xyz_to_balanced_camera()).unwrap();
    let xyz_to_scene = RgbSpace::Srgb.from_xyz();
    let gains = camera.as_shot_gains();

    for patch in patches(width, height) {
        // An even position inside the active area is a red site.
        let x = (patch.x as usize + CROP_MARGIN).next_multiple_of(2) + MASKED_BORDER[0];
        let y = (patch.y as usize + CROP_MARGIN).next_multiple_of(2) + MASKED_BORDER[1];
        let raw = [sample(x, y), sample(x + 1, y), sample(x + 1, y + 1)];
        let balanced = std::array::from_fn(|c| raw[c] * gains[c]);
        let recovered = mat3_apply(&xyz_to_scene, mat3_apply(&balanced_to_xyz, balanced));
        for (got, want) in recovered.iter().zip(patch.linear_srgb) {
            assert!((got - want).abs() < 2e-3, "{}: {recovered:?}", patch.name);
        }
    }
}
