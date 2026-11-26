//! Writes a folder of synthetic photographs, for trying the app by hand or
//! through the agent harness when no real photos are at hand.
//!
//! ```text
//! cargo run -p photo-testkit --example write_fixtures -- [long-edge]
//! ```
//!
//! It prints the folder it made and leaves it there. The folder lies in the
//! system temporary directory, the only place the testkit writes; set
//! `TMPDIR` to choose where that is. `long-edge` is the longer side of each
//! picture in pixels, 1600 when left out.
//!
//! ```text
//! 2024-03-15/dawn.jpg       JPEG with EXIF, landscape
//! 2024-03-15/harbour.dng    Bayer DNG with EXIF and an embedded preview
//! 2024-03-15/ridge.jpg      JPEG with EXIF, stored sideways (orientation 6)
//! 2024-07-04/fireworks.jpg  JPEG with EXIF
//! 2024-07-04/lake.png       8-bit PNG, no metadata
//! 2024-07-04/meadow.tif     16-bit TIFF, no metadata, portrait
//! 2024-07-04/notes.txt      not an image
//! ```

use std::fs;
use std::process::ExitCode;

use photo_core::time::DateTime;
use photo_testkit::{
    DngSpec, ExifSpec, write_dng_with, write_jpeg_with_exif, write_png8, write_tiff16,
};

fn main() -> ExitCode {
    let long = match std::env::args().nth(1) {
        None => 1600,
        Some(text) => match text.parse::<u32>() {
            Ok(long) if (64..=12_000).contains(&long) => long,
            _ => {
                eprintln!("the long edge is a number of pixels from 64 to 12000, not {text:?}");
                return ExitCode::from(2);
            }
        },
    };
    let short = long * 2 / 3;
    let root = match tempfile::Builder::new().prefix("photo-fixtures-").tempdir() {
        Ok(dir) => dir.keep(),
        Err(error) => {
            eprintln!("cannot make a folder in the temporary directory: {error}");
            return ExitCode::FAILURE;
        }
    };
    let spring = root.join("2024-03-15");
    let summer = root.join("2024-07-04");
    for folder in [&spring, &summer] {
        if let Err(error) = fs::create_dir_all(folder) {
            eprintln!("cannot make {}: {error}", folder.display());
            return ExitCode::FAILURE;
        }
    }
    let at = |month, day, hour, minute, second| {
        ExifSpec::sample_at(
            DateTime::new(2024, month, day, hour, minute, second).expect("a valid date"),
        )
    };
    let sideways = ExifSpec {
        orientation: 6,
        ..at(3, 15, 8, 5, 33)
    };
    let raw = DngSpec {
        exif: at(3, 15, 9, 20, 0),
        ..DngSpec::default()
    };

    write_jpeg_with_exif(&spring, "dawn.jpg", long, short, &at(3, 15, 7, 42, 10));
    write_jpeg_with_exif(&spring, "ridge.jpg", short, long, &sideways);
    write_dng_with(&spring, "harbour.dng", long, short, &raw);
    write_jpeg_with_exif(&summer, "fireworks.jpg", long, short, &at(7, 4, 21, 30, 45));
    write_png8(&summer, "lake.png", long, short);
    write_tiff16(&summer, "meadow.tif", short, long);
    if let Err(error) = fs::write(
        summer.join("notes.txt"),
        "Remember to charge the batteries.\n",
    ) {
        eprintln!("cannot write the stray text file: {error}");
        return ExitCode::FAILURE;
    }

    println!("{}", root.display());
    ExitCode::SUCCESS
}
