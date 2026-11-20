//! The gallery: every widget driven in place, and the snapshots.

use std::path::PathBuf;

use egui::{Key, Modifiers, PointerButton, Pos2, Role};
use photo_core::develop::CurvePoint;
use photo_ui::gallery::GalleryApp;
use photo_ui::harness::{Harness, HarnessBuilder, HarnessError, RenderError};
use photo_ui::widgets::Icon;
use photo_ui::widgets::color_wheel::handle_pos;
use photo_ui::widgets::curve_editor::point_pos;

use crate::{TestResult, across};

fn gallery(pixels_per_point: f32) -> Harness<GalleryApp> {
    HarnessBuilder::new()
        .size([1600.0, 1000.0])
        .pixels_per_point(pixels_per_point)
        .strict_glyphs(true)
        .build(GalleryApp::new)
}

/// The gallery at rest. Fails if any of its controls has no accessible name.
fn settled() -> Result<Harness<GalleryApp>, HarnessError> {
    let mut harness = gallery(1.0);
    harness.run()?;
    Ok(harness)
}

/// What the cell of the photo called `name` says about it.
fn cell(harness: &Harness<GalleryApp>, name: &str) -> Result<String, HarnessError> {
    let cell = harness.control(Role::ListBoxOption, name)?;
    Ok(cell.text.unwrap_or_default())
}

/// The middle of the cell of the photo called `name`.
fn cell_center(harness: &Harness<GalleryApp>, name: &str) -> Result<Pos2, HarnessError> {
    Ok(harness.control(Role::ListBoxOption, name)?.rect.center())
}

/// Whether the cell of the photo called `name` is part of the selection.
fn is_selected(harness: &Harness<GalleryApp>, name: &str) -> Result<bool, HarnessError> {
    Ok(harness.control(Role::ListBoxOption, name)?.selected == Some(true))
}

/// Whether a toast saying `text` is up.
fn toast(harness: &Harness<GalleryApp>, text: &str) -> bool {
    harness.count(Role::Label, text) == 1
}

// ---- Right column ----

#[test]
fn sliders_edit_the_develop_settings_and_name_the_step() -> TestResult {
    let mut harness = settled()?;
    assert_eq!(harness.control(Role::Slider, "Temp")?.value, Some(6150.0));

    // Through the accessibility tree, the way an automation script would.
    harness.set_value(Role::Slider, "Exposure", 1.2)?;
    assert!((harness.app().settings().tone.exposure - 1.2).abs() < 1.0e-6);
    let field = harness.control(Role::TextInput, "Exposure")?;
    assert_eq!(field.text.as_deref(), Some("+1.20"));
    assert!(
        toast(&harness, "Exposure +1.20"),
        "a finished edit is named in a toast"
    );

    // By keyboard.
    harness.focus(Role::Slider, "Contrast")?;
    harness.press_key(Key::ArrowRight, Modifiers::NONE)?;
    assert_eq!(harness.app().settings().tone.contrast, 13.0);
    assert!(toast(&harness, "Contrast +13"));

    // By typing a number.
    harness.set_text(Role::TextInput, "Tint", "25")?;
    assert_eq!(harness.app().settings().white_balance.tint, 25.0);
    assert!(toast(&harness, "Tint +25"));
    Ok(())
}

#[test]
fn an_edit_that_ends_where_it_began_is_not_recorded() -> TestResult {
    let mut harness = settled()?;
    let track = harness.control(Role::Slider, "Contrast")?.rect;
    // Contrast starts at +12 on a track from -100 to 100.
    let handle = across(track, 0.56);

    harness.pointer_down(handle)?;
    harness.pointer_move(handle + egui::vec2(30.0, 0.0))?;
    assert!(harness.app().settings().tone.contrast > 12.0);
    harness.pointer_move(handle)?;
    harness.pointer_up(handle)?;
    assert_eq!(harness.app().settings().tone.contrast, 12.0);
    assert!(!toast(&harness, "No Change"));
    Ok(())
}

#[test]
fn panels_collapse_and_their_switches_toggle() -> TestResult {
    let mut harness = settled()?;
    assert_eq!(harness.count(Role::Canvas, "Tone curve"), 1);

    harness.click(Role::DisclosureTriangle, "Tone Curve")?;
    assert_eq!(
        harness.count(Role::Canvas, "Tone curve"),
        0,
        "a closed panel hides its content"
    );
    let header = harness.control(Role::DisclosureTriangle, "Tone Curve")?;
    assert_eq!(header.expanded, Some(false));
    harness.click(Role::DisclosureTriangle, "Tone Curve")?;
    assert_eq!(harness.count(Role::Canvas, "Tone curve"), 1);

    let switch = harness.control(Role::Switch, "Enable Color Grading")?;
    assert_eq!(switch.toggled, Some(true));
    harness.click(Role::Switch, "Enable Color Grading")?;
    let switch = harness.control(Role::Switch, "Enable Color Grading")?;
    assert_eq!(switch.toggled, Some(false));
    assert_eq!(
        harness.count(Role::ColorWell, "Shadows wheel"),
        1,
        "switched off, not closed"
    );

    // Detail starts closed. Its sliders repeat labels ("Amount", "Detail")
    // that other panels use, so each carries a prefix in its name.
    assert_eq!(harness.count(Role::Slider, "Sharpening Amount"), 0);
    harness.click(Role::DisclosureTriangle, "Detail")?;
    harness.set_value(Role::Slider, "Sharpening Amount", 60.0)?;
    assert_eq!(harness.app().settings().detail.sharpening.amount, 60.0);
    assert_eq!(harness.count(Role::Slider, "Sharpening Detail"), 1);
    assert_eq!(harness.count(Role::Slider, "Noise Luminance Detail"), 1);
    assert_eq!(harness.count(Role::Slider, "Noise Color Detail"), 1);
    Ok(())
}

#[test]
fn the_histogram_indicators_toggle() -> TestResult {
    let mut harness = settled()?;
    assert_eq!(harness.count(Role::Image, "Histogram"), 1);
    let highlights = harness.control(Role::Button, "Show highlight clipping")?;
    assert_eq!(highlights.toggled, Some(false));

    harness.click(Role::Button, "Show highlight clipping")?;
    let highlights = harness.control(Role::Button, "Show highlight clipping")?;
    assert_eq!(highlights.toggled, Some(true));
    let shadows = harness.control(Role::Button, "Show shadow clipping")?;
    assert_eq!(shadows.toggled, Some(false));
    Ok(())
}

#[test]
fn the_tone_curve_is_edited_on_its_plot() -> TestResult {
    let mut harness = settled()?;
    let plot = harness.control(Role::Canvas, "Tone curve")?;
    assert_eq!(
        plot.text.as_deref(),
        Some("0.00,0.00 0.27,0.20 0.72,0.80 1.00,1.00")
    );

    // A press on the curve adds a point there (the curve passes x = 0.5 at
    // about y = 0.51).
    let on_curve = point_pos(plot.rect, CurvePoint::new(0.5, 0.51));
    harness.click_at(on_curve)?;
    let points = harness.app().settings().tone_curve.rgb.points().to_vec();
    assert_eq!(points.len(), 5);
    assert!((points[2].x - 0.5).abs() < 0.01 && (points[2].y - 0.51).abs() < 0.02);

    // A double-click on a point removes it.
    harness.double_click_at(point_pos(plot.rect, points[2]))?;
    assert_eq!(harness.app().settings().tone_curve.rgb.points().len(), 4);

    // A drag moves a point.
    harness.drag(
        point_pos(plot.rect, CurvePoint::new(0.27, 0.2)),
        point_pos(plot.rect, CurvePoint::new(0.3, 0.4)),
    )?;
    let plot = harness.control(Role::Canvas, "Tone curve")?;
    assert_eq!(
        plot.text.as_deref(),
        Some("0.00,0.00 0.30,0.40 0.72,0.80 1.00,1.00")
    );
    Ok(())
}

#[test]
fn each_colour_wheel_tints_its_own_range() -> TestResult {
    let mut harness = settled()?;
    let disc = harness.control(Role::ColorWell, "Midtones wheel")?;
    assert_eq!(disc.text.as_deref(), Some("hue 0, saturation 0"));

    harness.drag(disc.rect.center(), handle_pos(disc.rect, 120.0, 50.0))?;
    let grading = &harness.app().settings().color_grading;
    assert_eq!(
        (grading.midtones.hue, grading.midtones.saturation),
        (120.0, 50.0)
    );
    assert_eq!(
        (grading.shadows.hue, grading.shadows.saturation),
        (222.0, 34.0)
    );
    let disc = harness.control(Role::ColorWell, "Midtones wheel")?;
    assert_eq!(disc.text.as_deref(), Some("hue 120, saturation 50"));

    harness.set_value(Role::Slider, "Midtones Luminance", -12.0)?;
    assert_eq!(
        harness.app().settings().color_grading.midtones.luminance,
        -12.0
    );
    assert_eq!(
        harness.control(Role::Slider, "Highlights Luminance")?.value,
        Some(0.0)
    );
    Ok(())
}

// ---- Centre ----

#[test]
fn the_toolbar_changes_every_selected_photo() -> TestResult {
    let mut harness = settled()?;
    assert_eq!(
        cell(&harness, "DSC_4123.ARW")?,
        "active, 5 stars, pick, edited"
    );

    harness.click(Role::Button, "Rating 2")?;
    assert_eq!(
        cell(&harness, "DSC_4123.ARW")?,
        "active, 2 stars, pick, edited"
    );
    assert_eq!(
        cell(&harness, "Filmstrip DSC_4123.ARW")?,
        "active, 2 stars, pick, edited"
    );
    assert_eq!(
        cell(&harness, "DSC_4122.ARW")?,
        "selected, 2 stars, pick, red, edited"
    );
    assert_eq!(
        cell(&harness, "DSC_4121.ARW")?,
        "3 stars, pick, edited",
        "not selected"
    );
    assert!(toast(&harness, "Set Rating to 2"));

    harness.click(Role::Button, "Flag reject")?;
    assert_eq!(
        cell(&harness, "DSC_4123.ARW")?,
        "active, 2 stars, rejected, edited"
    );
    assert!(toast(&harness, "Set as Rejected"));

    harness.click(Role::Button, "Label green")?;
    assert_eq!(
        cell(&harness, "DSC_4123.ARW")?,
        "active, 2 stars, rejected, green, edited"
    );
    assert!(toast(&harness, "Set Label to Green"));

    harness.click(Role::Button, "Rotate clockwise")?;
    assert!(toast(&harness, "Rotated Right"));
    Ok(())
}

#[test]
fn the_view_buttons_show_which_view_is_chosen() -> TestResult {
    let mut harness = settled()?;
    assert_eq!(
        harness.control(Role::Button, "Grid view")?.toggled,
        Some(true)
    );

    harness.click(Role::Button, "Loupe view")?;
    assert_eq!(
        harness.control(Role::Button, "Loupe view")?.toggled,
        Some(true)
    );
    assert_eq!(
        harness.control(Role::Button, "Grid view")?.toggled,
        Some(false)
    );
    Ok(())
}

#[test]
fn cells_select_the_way_a_file_browser_does() -> TestResult {
    let mut harness = settled()?;
    assert_eq!(harness.count(Role::Label, "20 photos, 4 selected"), 1);

    // A plain click makes that photo the only selected one.
    harness.click(Role::ListBoxOption, "DSC_4126.ARW")?;
    assert_eq!(
        cell(&harness, "DSC_4126.ARW")?,
        "active, green, edited, virtual copy"
    );
    assert!(!is_selected(&harness, "DSC_4123.ARW")?);
    assert_eq!(harness.count(Role::Label, "20 photos, 1 selected"), 1);
    assert_eq!(
        harness.count(Role::Label, "DSC_4126.ARW"),
        1,
        "the navigator follows"
    );

    // Shift extends the selection from the active photo.
    let end = cell_center(&harness, "DSC_4129.ARW")?;
    harness.click_at_with(end, PointerButton::Primary, Modifiers::SHIFT)?;
    for name in ["DSC_4127.ARW", "DSC_4128.ARW", "DSC_4129.ARW"] {
        assert!(is_selected(&harness, name)?, "{name}");
    }
    assert!(!is_selected(&harness, "DSC_4130.ARW")?);
    assert!(cell(&harness, "DSC_4126.ARW")?.starts_with("active"));

    // Command (Ctrl on other platforms) adds one photo and takes it out again.
    let extra = cell_center(&harness, "DSC_4121.ARW")?;
    harness.click_at_with(extra, PointerButton::Primary, Modifiers::COMMAND)?;
    assert!(is_selected(&harness, "DSC_4121.ARW")?);
    harness.click_at_with(extra, PointerButton::Primary, Modifiers::COMMAND)?;
    assert!(!is_selected(&harness, "DSC_4121.ARW")?);

    // Taking the active photo out hands its role to the next selected one.
    let active = cell_center(&harness, "DSC_4126.ARW")?;
    harness.click_at_with(active, PointerButton::Primary, Modifiers::COMMAND)?;
    assert!(!is_selected(&harness, "DSC_4126.ARW")?);
    assert!(cell(&harness, "DSC_4127.ARW")?.starts_with("active"));

    // The filmstrip shows the same photos and selects the same way.
    harness.click(Role::ListBoxOption, "Filmstrip DSC_4135.ARW")?;
    assert_eq!(
        cell(&harness, "DSC_4135.ARW")?,
        "active, 2 stars, blue, edited"
    );
    assert!(!is_selected(&harness, "DSC_4127.ARW")?);
    Ok(())
}

#[test]
fn cells_tell_a_double_click_a_context_click_and_a_drag_apart() -> TestResult {
    let mut harness = settled()?;
    let active = cell_center(&harness, "DSC_4123.ARW")?;
    let other = cell_center(&harness, "DSC_4130.ARW")?;

    // A drag from a selected photo takes the whole selection along.
    harness.drag(active, other)?;
    assert!(toast(&harness, "Dragging 4 photos"));

    harness.secondary_click_at(other)?;
    assert!(toast(&harness, "Context menu for DSC_4130.ARW"));
    assert!(
        !is_selected(&harness, "DSC_4130.ARW")?,
        "a context click does not select"
    );

    harness.double_click(Role::ListBoxOption, "DSC_4126.ARW")?;
    assert!(toast(&harness, "Opened DSC_4126.ARW"));

    // A drag from any other photo is of that photo alone.
    harness.drag(other, active)?;
    assert!(toast(&harness, "Dragging 1 photo"));
    assert!(cell(&harness, "DSC_4130.ARW")?.starts_with("active"));
    Ok(())
}

// ---- Left column ----

#[test]
fn a_collection_is_made_through_the_dialog() -> TestResult {
    let mut harness = settled()?;
    assert_eq!(harness.count(Role::Button, "Create"), 0);

    harness.click(Role::Button, "New collection")?;
    assert!(
        harness.control(Role::Button, "Create")?.disabled,
        "a collection needs a name"
    );
    harness.click(Role::TextInput, "Collection name")?;
    harness.type_text("Best of 2026")?;
    harness.click(Role::Button, "Create")?;

    assert_eq!(
        harness.count(Role::Button, "Create"),
        0,
        "the dialog is gone"
    );
    let newest = harness.app().collections().last().map(String::as_str);
    assert_eq!(newest, Some("Best of 2026"));
    assert!(toast(&harness, "Created Collection \"Best of 2026\""));

    // Escape closes the dialog without making anything.
    harness.click(Role::Button, "New collection")?;
    harness.press_key(Key::Escape, Modifiers::NONE)?;
    assert_eq!(harness.count(Role::Button, "Create"), 0);
    assert_eq!(harness.app().collections().len(), 4);
    Ok(())
}

#[test]
fn every_icon_is_a_named_button() -> TestResult {
    let harness = settled()?;
    for icon in Icon::ALL {
        let name = format!("{} icon", icon.name());
        assert_eq!(harness.count(Role::Button, &name), 1, "{name}");
    }
    assert_eq!(harness.control(Role::Button, "Idle button")?.toggled, None);
    assert_eq!(
        harness.control(Role::Button, "Selected button")?.toggled,
        Some(true)
    );
    assert!(harness.control(Role::Button, "Disabled button")?.disabled);
    Ok(())
}

// ---- Snapshots ----

/// Where rendered frames go: `target/ui-snapshots` of the workspace.
fn snapshot(name: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/ui-snapshots")
        .join(name)
}

/// Save a frame, or say why not on a machine that cannot render.
fn save(harness: &mut Harness<GalleryApp>, name: &str) -> TestResult {
    match harness.save_png(snapshot(name)) {
        Ok(()) => Ok(()),
        Err(RenderError::NoAdapter(reason)) => {
            eprintln!("{name} not rendered, no GPU adapter: {reason}");
            Ok(())
        }
        Err(other) => Err(other.into()),
    }
}

#[test]
fn the_gallery_renders_at_1x_and_2x() -> TestResult {
    let mut tracks = Vec::new();
    for (pixels_per_point, name) in [(1.0, "gallery.png"), (2.0, "gallery@2x.png")] {
        let mut harness = gallery(pixels_per_point);
        harness.run()?;
        save(&mut harness, name)?;
        if harness.can_render() {
            let image = image::open(snapshot(name))?;
            let scale = pixels_per_point as u32;
            assert_eq!(
                (image.width(), image.height()),
                (1600 * scale, 1000 * scale)
            );
        }
        tracks.push(harness.control(Role::Slider, "Exposure")?.rect);
    }
    // Controls are reported in points, whatever the pixel density.
    let apart = (tracks[0].min - tracks[1].min).length() + (tracks[0].max - tracks[1].max).length();
    assert!(apart < 1.0, "{tracks:?}");
    Ok(())
}
