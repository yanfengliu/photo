//! Frames of every Library state, rendered on the GPU at 1600 x 1000
//! points, at 1x and 2x, to `target/ui-snapshots/library-*.png`, for a
//! person to look at. On a machine with no GPU nothing is rendered and the
//! tests only drive the states.

use std::path::{Path, PathBuf};

use egui::{Key, Modifiers, Role};
use image::{Rgb, RgbImage};
use photo_core::time::DateTime;
use photo_testkit::ExifSpec;
use photo_ui::PhotoApp;
use photo_ui::harness::{Harness, RenderError};
use tempfile::TempDir;

use crate::{Studio, TestResult, click_cell, settle};

/// Where a frame goes: `target/ui-snapshots` of the workspace.
fn snapshot_path(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/ui-snapshots")
        .join(name)
}

/// Render the resting frame as `library-{name}.png` (with `@2x` at two
/// pixels per point), or say why not.
pub fn snap(harness: &mut Harness<PhotoApp>, name: &str, scale: u32) -> TestResult {
    settle(harness)?;
    // Let toasts and animations finish, so the frame is the resting one,
    // with no pointer drawn or hovering.
    harness.pointer_leave()?;
    harness.advance(3.0);
    harness.run()?;
    snap_now(harness, name, scale)
}

/// Render the frame as it is, while work may still be running.
pub fn snap_now(harness: &mut Harness<PhotoApp>, name: &str, scale: u32) -> TestResult {
    let suffix = if scale == 1 {
        String::new()
    } else {
        format!("@{scale}x")
    };
    let file = format!("library-{name}{suffix}.png");
    let window = harness.ctx().content_rect().size() * scale as f32;
    match harness.save_png(snapshot_path(&file)) {
        Ok(()) => {
            let image = image::open(snapshot_path(&file))?;
            assert_eq!(
                (image.width(), image.height()),
                (window.x as u32, window.y as u32)
            );
            Ok(())
        }
        Err(RenderError::NoAdapter(reason)) => {
            eprintln!("{file} not rendered, no GPU adapter: {reason}");
            Ok(())
        }
        Err(other) => Err(other.into()),
    }
}

/// A made-up landscape: a sky, a far ridge, a near hill and a sun, in the
/// colours of `palette`, so the grid has photos that differ.
fn landscape(width: u32, height: u32, palette: [[u8; 3]; 4], sun: (f32, f32)) -> RgbImage {
    let mix = |a: [u8; 3], b: [u8; 3], t: f32| {
        let t = t.clamp(0.0, 1.0);
        let channel = |i: usize| (f32::from(a[i]) + (f32::from(b[i]) - f32::from(a[i])) * t) as u8;
        [channel(0), channel(1), channel(2)]
    };
    let [sky, horizon, far, near] = palette;
    RgbImage::from_fn(width, height, |x, y| {
        let u = x as f32 / width as f32;
        let v = y as f32 / height as f32;
        let ridge = 0.55 + 0.06 * (u * 7.0 + palette[0][0] as f32).sin() + 0.03 * (u * 19.0).sin();
        let hill = 0.72 + 0.08 * (u * 3.0 + 1.3).sin();
        let mut color = if v > hill {
            mix(near, [10, 12, 10], (v - hill) * 2.5)
        } else if v > ridge {
            mix(far, near, (v - ridge) * 3.0)
        } else {
            mix(sky, horizon, v / ridge)
        };
        let aspect = width as f32 / height as f32;
        let distance = ((u - sun.0) * aspect).hypot(v - sun.1);
        if v < ridge {
            let glow = (-distance * distance / 0.01).exp();
            color = mix(color, [255, 240, 210], glow);
        }
        Rgb(color)
    })
}

/// A shoot of photos to look at: generated landscapes in two dated folders
/// and four photos with full camera metadata.
fn shoot() -> Result<(TempDir, Vec<PathBuf>), Box<dyn std::error::Error>> {
    let dir = tempfile::tempdir()?;
    let coast = dir.path().join("2024-05-02 Coast");
    let hills = dir.path().join("2024-06-18 Hills");
    std::fs::create_dir(&coast)?;
    std::fs::create_dir(&hills)?;
    let palettes: [[[u8; 3]; 4]; 6] = [
        [[40, 64, 120], [230, 150, 110], [70, 60, 90], [30, 40, 50]],
        [
            [120, 170, 210],
            [220, 230, 235],
            [90, 120, 80],
            [50, 80, 40],
        ],
        [[20, 20, 50], [170, 80, 60], [50, 30, 50], [20, 15, 25]],
        [
            [150, 190, 220],
            [245, 225, 190],
            [160, 130, 90],
            [110, 80, 50],
        ],
        [[70, 110, 140], [190, 210, 200], [60, 90, 100], [30, 55, 60]],
        [
            [200, 140, 120],
            [250, 210, 150],
            [120, 90, 110],
            [60, 45, 60],
        ],
    ];
    let sizes = [
        (1200, 800),
        (800, 1200),
        (1200, 900),
        (1280, 720),
        (1000, 1000),
        (1200, 800),
    ];
    let mut files = Vec::new();
    for index in 0..12 {
        let folder = if index < 7 { &coast } else { &hills };
        let (width, height) = sizes[index % sizes.len()];
        let sun = (0.2 + 0.06 * index as f32, 0.18 + 0.03 * (index % 4) as f32);
        let image = landscape(width, height, palettes[index % palettes.len()], sun);
        let path = folder.join(format!("DSC_{:04}.jpg", 4100 + index));
        image.save_with_format(&path, image::ImageFormat::Jpeg)?;
        files.push(path);
    }
    for (index, (width, height)) in [(1500, 1000), (1000, 1500), (1500, 1000), (1600, 900)]
        .into_iter()
        .enumerate()
    {
        let time = DateTime::new(2024, 6, 18, 7 + index as u8, 12, 40).ok_or("a date")?;
        let spec = ExifSpec {
            iso: Some([100, 200, 400, 800][index]),
            ..ExifSpec::sample_at(time)
        };
        let name = format!("IMG_{:04}.jpg", 2210 + index);
        files.push(photo_testkit::write_jpeg_with_exif(
            &hills, &name, width, height, &spec,
        ));
    }
    Ok((dir, files))
}

/// Give the shoot ratings, flags, labels, keywords, a collection set with a
/// collection in it, and a smart collection, the way a user would.
fn organise(harness: &mut Harness<PhotoApp>) -> TestResult {
    let marks: [(&str, &[Key]); 6] = [
        ("DSC_4100.jpg", &[Key::Num5, Key::P]),
        ("DSC_4102.jpg", &[Key::Num3, Key::Num6]),
        ("DSC_4103.jpg", &[Key::Num4, Key::Num8, Key::B]),
        ("DSC_4105.jpg", &[Key::X]),
        ("DSC_4108.jpg", &[Key::Num2, Key::Num9]),
        ("IMG_2211.jpg", &[Key::Num4, Key::P, Key::B]),
    ];
    for (name, keys) in marks {
        click_cell(harness, name, Modifiers::NONE)?;
        for key in keys {
            harness.press_key(*key, Modifiers::NONE)?;
        }
    }
    harness.click(Role::Button, "Create collection")?;
    harness.click(Role::Button, "New Collection Set…")?;
    harness.type_text("Trips")?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    harness.press_key(Key::N, Modifiers::COMMAND)?;
    harness.type_text("Coast 2024")?;
    harness.click(Role::ComboBox, "Inside set")?;
    harness.click(Role::Button, "Trips")?;
    harness.click(Role::Button, "Create")?;
    harness.press_key(Key::N, Modifiers::COMMAND)?;
    harness.type_text("Portfolio")?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    harness.click(Role::Button, "Create collection")?;
    harness.click(Role::Button, "New Smart Collection…")?;
    harness.set_text(
        Role::TextInput,
        "Smart collection name",
        "Four stars and up",
    )?;
    harness.click(Role::ComboBox, "Rule 1 operator")?;
    harness.click(Role::Button, "is at least")?;
    harness.click(Role::Button, "Rule 1 rating 4")?;
    harness.click(Role::Button, "Create")?;

    click_cell(harness, "DSC_4100.jpg", Modifiers::NONE)?;
    click_cell(harness, "DSC_4103.jpg", Modifiers::COMMAND)?;
    let from = harness
        .control(Role::ListBoxOption, "DSC_4100.jpg")?
        .rect
        .center();
    let to = harness
        .control(Role::TreeItem, "Collection Portfolio")?
        .rect
        .center();
    harness.drag(from, to)?;
    harness.set_text(Role::TextInput, "Add keywords", "coast, evening")?;
    click_cell(harness, "IMG_2211.jpg", Modifiers::NONE)?;
    harness.set_text(Role::TextInput, "Add keywords", "hills, morning")?;
    harness.set_text(Role::TextInput, "Title", "Ridge at first light")?;
    Ok(())
}

/// Every state, at `scale` pixels per point.
fn every_state(scale: u32) -> TestResult {
    let studio = Studio::new();
    let mut harness = studio.app_at(scale as f32);
    harness.run()?;
    snap(&mut harness, "empty", scale)?;

    let (shoot_dir, files) = shoot()?;
    harness.press_key(Key::I, Modifiers::COMMAND | Modifiers::SHIFT)?;
    let root = shoot_dir.path().display().to_string();
    harness.set_text(Role::TextInput, "Source folder", &root)?;
    snap(&mut harness, "import-dialog", scale)?;
    harness.click(Role::Button, &format!("Import {} Photos", files.len()))?;
    settle(&mut harness)?;
    organise(&mut harness)?;
    harness.click(Role::TreeItem, "All Photographs")?;
    click_cell(&mut harness, "IMG_2211.jpg", Modifiers::NONE)?;
    click_cell(&mut harness, "DSC_4100.jpg", Modifiers::COMMAND)?;
    click_cell(&mut harness, "DSC_4103.jpg", Modifiers::COMMAND)?;
    click_cell(&mut harness, "IMG_2211.jpg", Modifiers::COMMAND)?;
    click_cell(&mut harness, "IMG_2211.jpg", Modifiers::COMMAND)?;
    snap(&mut harness, "grid", scale)?;

    harness.press_key(Key::E, Modifiers::NONE)?;
    snap(&mut harness, "loupe", scale)?;
    harness.press_key(Key::Space, Modifiers::NONE)?;
    snap(&mut harness, "loupe-1to1", scale)?;
    harness.press_key(Key::C, Modifiers::NONE)?;
    snap(&mut harness, "compare", scale)?;
    harness.press_key(Key::N, Modifiers::NONE)?;
    snap(&mut harness, "survey", scale)?;

    harness.press_key(Key::G, Modifiers::NONE)?;
    harness.click(Role::Button, "Attribute filter")?;
    harness.click(Role::Button, "Filter rating 3")?;
    snap(&mut harness, "filter-attribute", scale)?;
    harness.click(Role::Button, "No filter")?;
    harness.click(Role::Button, "Metadata filter")?;
    snap(&mut harness, "filter-metadata", scale)?;
    harness.click(Role::Button, "Text filter")?;
    harness.set_text(Role::TextInput, "Search text", "DSC_410")?;
    snap(&mut harness, "filter-text", scale)?;
    harness.click(Role::Button, "No filter")?;

    harness.click(Role::TreeItem, "Smart Collection Four stars and up")?;
    snap(&mut harness, "smart-collection", scale)?;
    harness.click(Role::TreeItem, "All Photographs")?;
    harness.click(Role::Button, "Create collection")?;
    harness.click(Role::Button, "New Smart Collection…")?;
    harness.click(Role::Button, "Add Rule")?;
    snap(&mut harness, "dialog-smart", scale)?;
    harness.press_key(Key::Escape, Modifiers::NONE)?;
    click_cell(&mut harness, "DSC_4105.jpg", Modifiers::NONE)?;
    harness.press_key(Key::Backspace, Modifiers::NONE)?;
    snap(&mut harness, "dialog-remove", scale)?;
    harness.press_key(Key::Escape, Modifiers::NONE)?;

    harness.press_key(Key::Tab, Modifiers::SHIFT)?;
    snap(&mut harness, "edges-hidden", scale)?;
    harness.press_key(Key::Tab, Modifiers::SHIFT)?;
    drop(shoot_dir);
    Ok(())
}

#[test]
fn every_library_state_renders_at_1x() -> TestResult {
    every_state(1)
}

#[test]
fn every_library_state_renders_at_2x() -> TestResult {
    every_state(2)
}

/// The snapshot folder is inside the workspace's target folder.
#[test]
fn snapshots_go_to_the_workspace_target_folder() {
    let path = snapshot_path("x.png");
    assert!(path.starts_with(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target")));
}
