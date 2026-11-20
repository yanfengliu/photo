//! Files that cannot be decoded: every failure is a typed error, never a
//! panic, and never a silently wrong picture.

use std::path::{Path, PathBuf};

use photo_core::meta::FileKind;
use photo_decode::{
    DecodeError, RASTER_EXTENSIONS, RAW_EXTENSIONS, decode, embedded_preview, file_kind,
    is_supported, read_metadata,
};
use photo_testkit as kit;

/// Runs all three entry points; none of them may panic.
fn poke(path: &Path) -> [Option<DecodeError>; 3] {
    [
        decode(path).err(),
        read_metadata(path).err(),
        embedded_preview(path, 64).err(),
    ]
}

fn write(dir: &Path, name: &str, bytes: &[u8]) -> PathBuf {
    let path = dir.join(name);
    std::fs::write(&path, bytes).unwrap();
    path
}

/// Deterministic noise.
fn noise(len: usize) -> Vec<u8> {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    (0..len)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 7;
            state ^= state << 17;
            (state >> 32) as u8
        })
        .collect()
}

#[test]
fn file_kind_goes_by_extension_whatever_the_case() {
    assert_eq!(file_kind("a/b/photo.JPG"), Some(FileKind::Raster));
    assert_eq!(file_kind("photo.jpeg"), Some(FileKind::Raster));
    assert_eq!(file_kind("scan.TiFf"), Some(FileKind::Raster));
    assert_eq!(file_kind("shot.ARW"), Some(FileKind::Raw));
    assert_eq!(file_kind("shot.dng"), Some(FileKind::Raw));
    assert_eq!(file_kind("shot.Cr3"), Some(FileKind::Raw));
    assert_eq!(file_kind("back.3fr"), Some(FileKind::Raw));
    assert_eq!(file_kind("notes.txt"), None);
    assert_eq!(file_kind("sidecar.xmp"), None);
    assert_eq!(file_kind("no_extension"), None);
    assert_eq!(
        file_kind(".jpg"),
        None,
        "a hidden file called .jpg has no extension"
    );
    assert_eq!(file_kind("archive.jpg.zip"), None);
    assert!(is_supported("x.png") && is_supported("x.NEF"));
    assert!(!is_supported("x.psd") && !is_supported("x"));
}

#[test]
fn extension_lists_are_lower_case_disjoint_and_complete() {
    for extension in RASTER_EXTENSIONS.iter().chain(RAW_EXTENSIONS) {
        assert_eq!(*extension, extension.to_ascii_lowercase());
        assert!(is_supported(format!("file.{extension}")), "{extension}");
    }
    assert!(
        RASTER_EXTENSIONS
            .iter()
            .all(|e| !RAW_EXTENSIONS.contains(e))
    );
    let mut raster = RASTER_EXTENSIONS.to_vec();
    raster.sort_unstable();
    assert_eq!(
        raster,
        ["bmp", "gif", "jpeg", "jpg", "png", "tif", "tiff", "webp"]
    );
    for extension in [
        "dng", "arw", "cr2", "cr3", "nef", "raf", "orf", "rw2", "pef",
    ] {
        assert!(RAW_EXTENSIONS.contains(&extension), "{extension}");
    }
}

#[test]
fn unsupported_extensions_are_refused_by_every_entry_point() {
    let dir = tempfile::tempdir().unwrap();
    // Perfectly good PNG data: the refusal is about the name, by contract.
    let png = std::fs::read(kit::write_png8(dir.path(), "ok.png", 8, 8)).unwrap();
    for name in ["notes.txt", "layers.psd", "noextension"] {
        let path = write(dir.path(), name, &png);
        for error in poke(&path) {
            assert!(
                matches!(error, Some(DecodeError::Unsupported(_))),
                "{name}: {error:?}"
            );
        }
    }
}

#[test]
fn a_missing_file_is_an_io_error() {
    let dir = tempfile::tempdir().unwrap();
    for name in [
        "absent.jpg",
        "absent.png",
        "absent.tif",
        "absent.dng",
        "absent.arw",
    ] {
        for error in poke(&dir.path().join(name)) {
            assert!(
                matches!(error, Some(DecodeError::Io(_))),
                "{name}: {error:?}"
            );
        }
    }
}

#[test]
fn a_directory_is_not_an_image() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["folder.jpg", "folder.dng"] {
        let path = dir.path().join(name);
        std::fs::create_dir(&path).unwrap();
        for error in poke(&path) {
            assert!(error.is_some(), "{name}");
        }
    }
}

#[test]
fn empty_and_garbage_files_are_typed_errors() {
    let dir = tempfile::tempdir().unwrap();
    let junk = noise(4096);
    let text = b"This is not an image, whatever its name says.\n".repeat(40);
    for extension in RASTER_EXTENSIONS
        .iter()
        .chain(["dng", "arw", "cr2", "cr3", "nef", "raf", "orf", "rw2"].iter())
    {
        for (label, bytes) in [("empty", &[][..]), ("junk", &junk[..]), ("text", &text[..])] {
            let path = write(dir.path(), &format!("{label}.{extension}"), bytes);
            for error in poke(&path) {
                assert!(
                    matches!(
                        error,
                        Some(DecodeError::Corrupt(_) | DecodeError::Unsupported(_))
                    ),
                    "{label}.{extension}: {error:?}"
                );
            }
        }
    }
}

#[test]
fn truncated_files_fail_to_decode_with_a_typed_error() {
    let dir = tempfile::tempdir().unwrap();
    let whole = [
        kit::write_jpeg(dir.path(), "t.jpg", 160, 120),
        kit::write_png8(dir.path(), "t.png", 160, 120),
        kit::write_png16(dir.path(), "u.png", 160, 120),
        kit::write_tiff16(dir.path(), "t.tif", 160, 120),
        kit::write_dng(dir.path(), "t.dng", 160, 120),
    ];
    for path in whole {
        let bytes = std::fs::read(&path).unwrap();
        let extension = path.extension().unwrap().to_str().unwrap();
        for percent in [10, 50, 90, 99] {
            let cut = write(
                dir.path(),
                &format!("cut{percent}.{extension}"),
                &bytes[..bytes.len() * percent / 100],
            );
            let error = decode(&cut).expect_err("a truncated file must not decode");
            assert!(
                matches!(error, DecodeError::Corrupt(_)),
                "{extension} at {percent} %: {error:?}"
            );
            // The other entry points may or may not need the missing part,
            // but they must answer, not panic.
            let _ = read_metadata(&cut);
            let _ = embedded_preview(&cut, 64);
        }
    }
}

#[test]
fn no_truncation_point_panics() {
    let dir = tempfile::tempdir().unwrap();
    let spec = kit::ExifSpec {
        orientation: 6,
        ..kit::ExifSpec::sample()
    };
    let whole = [
        kit::write_jpeg_with_exif(dir.path(), "s.jpg", 48, 32, &spec),
        kit::write_png8(dir.path(), "s.png", 48, 32),
        kit::write_tiff16_with_exif(dir.path(), "s.tif", 48, 32, &spec),
        kit::write_dng(dir.path(), "s.dng", 48, 32),
        kit::write_tagged(
            dir.path(),
            "p.jpg",
            48,
            32,
            photo_core::colorimetry::RgbSpace::AdobeRgb,
        ),
    ];
    for path in whole {
        let bytes = std::fs::read(&path).unwrap();
        let extension = path.extension().unwrap().to_str().unwrap();
        let step = (bytes.len() / 60).max(1);
        for length in (0..bytes.len()).step_by(step) {
            let cut = write(dir.path(), &format!("cut.{extension}"), &bytes[..length]);
            let _ = poke(&cut);
        }
    }
}

#[test]
fn flipped_bytes_never_panic() {
    // Raster formats only. The raw decoder trusts the entry counts in a
    // file's directories when it allocates, so a flipped count in a DNG can
    // exhaust memory rather than fail cleanly, which no caller can catch.
    let dir = tempfile::tempdir().unwrap();
    let scene = kit::scene(48, 32);
    let other_formats = ["f.webp", "f.bmp", "f.gif"].map(|name| {
        let path = dir.path().join(name);
        scene.save(&path).unwrap();
        path
    });
    let whole = [
        kit::write_jpeg_with_exif(dir.path(), "f.jpg", 48, 32, &kit::ExifSpec::sample()),
        kit::write_png16(dir.path(), "f.png", 48, 32),
        kit::write_tiff16_with_exif(dir.path(), "f.tif", 48, 32, &kit::ExifSpec::sample()),
        kit::write_tagged(
            dir.path(),
            "g.png",
            48,
            32,
            photo_core::colorimetry::RgbSpace::DisplayP3,
        ),
    ]
    .into_iter()
    .chain(other_formats);
    let offsets = noise(96);
    for path in whole {
        let bytes = std::fs::read(&path).unwrap();
        let extension = path.extension().unwrap().to_str().unwrap();
        for (index, pair) in offsets.chunks_exact(2).enumerate() {
            let mut damaged = bytes.clone();
            // Concentrate on the structure at both ends of the file, where
            // headers and directories live.
            let span = damaged.len().min(600);
            let offset = usize::from(pair[0]) * span / 256;
            let at = if index % 2 == 0 {
                offset
            } else {
                damaged.len() - 1 - offset
            };
            damaged[at] ^= pair[1] | 1;
            let cut = write(dir.path(), &format!("flip.{extension}"), &damaged);
            let _ = poke(&cut);
        }
    }
}

#[test]
fn an_image_too_large_to_hold_is_refused_before_allocating() {
    // A well-formed PNG that claims 60000 x 60000 pixels and then holds an
    // empty data stream: 10 GB if anyone believed the header.
    let dir = tempfile::tempdir().unwrap();
    let mut header = 60_000u32.to_be_bytes().to_vec();
    header.extend_from_slice(&60_000u32.to_be_bytes());
    header.extend_from_slice(&[8, 2, 0, 0, 0]);
    let empty_zlib_stream = [0x78, 0x9c, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01];
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    for (kind, data) in [
        (b"IHDR", &header[..]),
        (b"IDAT", &empty_zlib_stream[..]),
        (b"IEND", &[][..]),
    ] {
        let mut chunk = kind.to_vec();
        chunk.extend_from_slice(data);
        png.extend_from_slice(&(data.len() as u32).to_be_bytes());
        png.extend_from_slice(&chunk);
        png.extend_from_slice(&crc32(&chunk).to_be_bytes());
    }
    let path = write(dir.path(), "huge.png", &png);
    let error = decode(&path).expect_err("3.6 gigapixels must be refused");
    assert!(matches!(error, DecodeError::TooLarge(_)), "{error:?}");
    let error = embedded_preview(&path, 256).expect_err("the preview would need the same memory");
    assert!(matches!(error, DecodeError::TooLarge(_)), "{error:?}");
    // Reading the header alone is harmless and still reports the claim.
    let metadata = read_metadata(&path).unwrap();
    assert_eq!((metadata.width, metadata.height), (60_000, 60_000));
}

fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = u32::MAX;
    for &byte in bytes {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xedb8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

#[test]
fn errors_explain_themselves() {
    let dir = tempfile::tempdir().unwrap();
    let unsupported = decode(dir.path().join("x.txt")).unwrap_err().to_string();
    assert!(unsupported.contains("txt"), "{unsupported}");
    let missing = decode(dir.path().join("x.jpg")).unwrap_err();
    assert!(
        std::error::Error::source(&missing).is_some(),
        "an I/O error keeps its cause"
    );
}
