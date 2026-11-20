//! What one frame costs with a few thousand photos in the grid, once the
//! thumbnails in view are cached. The measured times are printed; run with
//! `--nocapture` to see them.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::Modifiers;

use crate::{Studio, TestResult, click_cell, settle};

/// Photos in the catalog.
const PHOTOS: usize = 3_000;
/// Frames timed.
const FRAMES: usize = 60;

/// The median and the slowest of `frames` timed frames.
fn time_frames(
    harness: &mut photo_ui::harness::Harness<photo_ui::PhotoApp>,
) -> (Duration, Duration) {
    let mut times: Vec<Duration> = (0..FRAMES)
        .map(|_| {
            let start = Instant::now();
            harness.step(1);
            start.elapsed()
        })
        .collect();
    times.sort();
    (times[FRAMES / 2], times[FRAMES - 1])
}

#[test]
fn a_grid_of_three_thousand_photos_draws_a_frame_well_within_sixteen_milliseconds() -> TestResult {
    let folder = tempfile::tempdir()?;
    let files: Vec<PathBuf> = (0..PHOTOS)
        .map(|index| {
            photo_testkit::write_jpeg(folder.path(), &format!("IMG_{index:04}.jpg"), 48, 32)
        })
        .collect();
    let studio = Studio::new();
    let mut harness = studio.app_with(files);
    settle(&mut harness)?;
    assert_eq!(harness.app().engine().photo_count(), PHOTOS);
    click_cell(&mut harness, "IMG_0004.jpg", Modifiers::NONE)?;
    click_cell(&mut harness, "IMG_0012.jpg", Modifiers::SHIFT)?;
    settle(&mut harness)?;

    let (median, worst) = time_frames(&mut harness);
    println!(
        "{PHOTOS} photos, {} selected, grid view, thumbnails cached: median frame {:.2} ms, slowest {:.2} ms",
        harness.app().engine().selection().selected().len(),
        median.as_secs_f64() * 1000.0,
        worst.as_secs_f64() * 1000.0
    );
    // The frame includes building the accessibility tree the harness reads,
    // which a window without a screen reader does not.
    assert!(
        median < Duration::from_millis(16),
        "median frame {median:?}"
    );

    if harness.can_render() {
        // Tessellating and drawing it on the GPU, and reading the pixels
        // back (which a window does not do): an upper bound.
        let mut renders: Vec<Duration> = (0..10)
            .map(|_| {
                // A render is kept per frame, so each one needs its own.
                harness.step(1);
                let start = Instant::now();
                let rendered = harness.render();
                let elapsed = start.elapsed();
                assert!(rendered.is_ok(), "{:?}", rendered.err());
                elapsed
            })
            .collect();
        renders.sort();
        println!(
            "the same frame tessellated, drawn on the GPU and read back: median {:.2} ms",
            renders[5].as_secs_f64() * 1000.0
        );
    }
    Ok(())
}
