use super::*;

fn gradient(width: u32, height: u32) -> Rgba8Image {
    let pixels = (0..height)
        .flat_map(|y| (0..width).flat_map(move |x| [(x * 8) as u8, (y * 8) as u8, 128, 255]))
        .collect();
    Rgba8Image {
        width,
        height,
        pixels,
        space: RgbSpace::Srgb,
    }
}

fn header(version: u64, histogram: Option<Histogram>) -> Header {
    Header {
        version,
        size: PreviewSize::Standard,
        origin: PreviewOrigin::EmbeddedPreview,
        source: Stamp {
            bytes: 12_345,
            modified_secs: -17,
            modified_nanos: 999,
        },
        histogram,
    }
}

#[test]
fn a_written_file_reads_back_with_its_header_and_pixels() {
    let dir = tempfile::tempdir().unwrap();
    let image = gradient(24, 16);
    let histogram = photo_imaging::histogram(&image);
    let path = file_path(dir.path(), PhotoId(300), PreviewSize::Standard, 7);
    write(&path, &header(7, Some(histogram.clone())), &image).unwrap();

    assert!(path.starts_with(dir.path().join("2c").join("300")));
    assert_eq!(path.file_name().unwrap(), "standard-7.jpg");
    let (read_header, read_image) = read(&path).unwrap();
    assert_eq!(read_header, header(7, Some(histogram)));
    assert_eq!((read_image.width, read_image.height), (24, 16));
    let worst = read_image
        .pixels
        .iter()
        .zip(&image.pixels)
        .map(|(a, b)| a.abs_diff(*b))
        .max()
        .unwrap();
    assert!(worst <= 12, "JPEG error {worst}");

    // Still an ordinary JPEG to anything else that reads it.
    let plain = image::open(&path).unwrap();
    assert_eq!((plain.width(), plain.height()), (24, 16));
}

#[test]
fn a_file_this_code_did_not_write_reads_as_absent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("foreign.jpg");
    let mut jpeg = Vec::new();
    JpegEncoder::new(&mut jpeg)
        .encode(&[10; 4 * 4 * 3], 4, 4, ExtendedColorType::Rgb8)
        .unwrap();
    fs::write(&path, &jpeg).unwrap();
    assert!(read(&path).is_none());
    fs::write(&path, b"not a jpeg at all").unwrap();
    assert!(read(&path).is_none());
    assert!(read(&dir.path().join("absent.jpg")).is_none());
}

#[test]
fn writing_a_version_can_delete_the_older_ones_of_the_same_size() {
    let dir = tempfile::tempdir().unwrap();
    let image = gradient(8, 8);
    let photo = PhotoId(5);
    for (size, version) in [
        (PreviewSize::Thumbnail, 1),
        (PreviewSize::Thumbnail, 2),
        (PreviewSize::Thumbnail, 4),
        (PreviewSize::Standard, 1),
    ] {
        let header = Header {
            size,
            ..header(version, None)
        };
        write(
            &file_path(dir.path(), photo, size, version),
            &header,
            &image,
        )
        .unwrap();
    }
    assert!(newer_exists(dir.path(), photo, PreviewSize::Thumbnail, 2));
    assert!(!newer_exists(dir.path(), photo, PreviewSize::Thumbnail, 4));
    assert!(!newer_exists(dir.path(), photo, PreviewSize::Standard, 1));
    assert!(!newer_exists(
        dir.path(),
        PhotoId(6),
        PreviewSize::Standard,
        1
    ));
    remove_older(dir.path(), photo, PreviewSize::Thumbnail, 4);
    let exists = |size, version| file_path(dir.path(), photo, size, version).exists();
    assert!(!exists(PreviewSize::Thumbnail, 1));
    assert!(!exists(PreviewSize::Thumbnail, 2));
    assert!(exists(PreviewSize::Thumbnail, 4));
    assert!(
        exists(PreviewSize::Standard, 1),
        "other sizes are left alone"
    );
}
