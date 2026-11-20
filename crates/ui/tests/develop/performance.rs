//! How long a slider drag takes to show: from the pointer moving the
//! Exposure slider to a frame of that value on the canvas, measured
//! through the harness on a 12-megapixel raw file fitted to the canvas of
//! a 1600 x 1000 window at two pixels per point (a high-density screen).
//! The UI runs as a test build, unoptimised; the pipeline crates are
//! optimised as in every build. The times are printed; run with
//! `--nocapture` to see them.

use std::time::{Duration, Instant};

use egui::pos2;

use crate::{Studio, TestResult, develop, session, slider, track_x};

/// Moves of the slider timed.
const MOVES: usize = 12;

#[test]
fn a_slider_drag_shows_its_frame_promptly() -> TestResult {
    let dir = tempfile::tempdir()?;
    let raw = photo_testkit::write_dng(dir.path(), "large.dng", 4000, 3000);
    let studio = Studio::new();
    let mut harness = studio.app_with_at(vec![raw], 2.0);
    develop(&mut harness, "large.dng")?;
    let control = slider(&harness, "Exposure")?;
    let y = control.rect.center().y;
    harness.pointer_down(pos2(track_x(&control, 0.0), y))?;
    let mut times: Vec<Duration> = Vec::with_capacity(MOVES);
    for step in 1..=MOVES {
        let value = step as f32 * 0.1;
        let start = Instant::now();
        harness.pointer_move(pos2(track_x(&control, value), y))?;
        // Frames run until the canvas shows the frame of the newest value.
        let mut frames = 0;
        while !harness.app().canvas_shows().current {
            harness.step(1);
            frames += 1;
            assert!(frames < 100_000, "the frame never came");
        }
        times.push(start.elapsed());
    }
    // A hand that never pauses: the slider moves every frame for a while,
    // and the canvas must keep showing new frames, not wait for the stop.
    let mut shown = Vec::new();
    let started = Instant::now();
    let mut value = MOVES as f32 * 0.1;
    let mut moves = 0;
    let mut values = Vec::new();
    while started.elapsed() < Duration::from_millis(1500) || shown.len() < 2 {
        value = (value - 0.01).max(-2.0);
        harness.pointer_move(pos2(track_x(&control, value), y))?;
        moves += 1;
        let now = harness.app().canvas_shows().version;
        if now.is_some() && shown.last() != Some(&now) {
            shown.push(now);
        }
        let slider_value = slider(&harness, "Exposure")?.value;
        if values.last() != Some(&slider_value) {
            values.push(slider_value);
        }
        assert!(
            started.elapsed() < Duration::from_secs(30),
            "frames stopped: {shown:?}"
        );
    }
    println!(
        "moving without a pause for {:.1} s: {moves} moves, {} slider values, {} frames shown",
        started.elapsed().as_secs_f64(),
        values.len(),
        shown.len()
    );
    // Asking for every value would cancel every render until the hand
    // stopped; a few frames on the way, even on a busy machine, show it
    // does not.
    assert!(
        shown.len() >= 3,
        "frames keep coming while the slider moves: {} for {} values",
        shown.len(),
        values.len()
    );
    harness.pointer_up(pos2(track_x(&control, value), y))?;
    crate::settle(&mut harness)?;
    let frame = session(&harness).frame().ok_or("a frame")?;
    let view = frame.view();
    times.sort();
    let median = times[MOVES / 2];
    println!(
        "slider drag to frame on screen, {} x {} frame of a 4000 x 3000 raw: median {:.1} ms, slowest {:.1} ms, fastest {:.1} ms; full render {:.1} ms",
        view.width(),
        view.height(),
        median.as_secs_f64() * 1000.0,
        times[MOVES - 1].as_secs_f64() * 1000.0,
        times[0].as_secs_f64() * 1000.0,
        frame.render_time().as_secs_f64() * 1000.0,
    );
    assert!(median < Duration::from_secs(2), "median {median:?}");
    Ok(())
}
