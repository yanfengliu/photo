//! Collapsible panels.

use egui::{Key, Modifiers, PointerButton, Role};
use photo_core::develop::Adj;
use photo_ui::harness::{Harness, HarnessBuilder, Host};
use photo_ui::widgets::{Panel, PanelGroup, adj_slider, panel_column};

use crate::TestResult;

struct Column {
    group: PanelGroup,
    curve_enabled: bool,
    exposure: f32,
    toggles: u32,
    switch_flips: u32,
}

fn column(open: &[&str]) -> Harness<Host<Column>> {
    let state = Column {
        group: PanelGroup::with_open(open.iter().copied()),
        curve_enabled: true,
        exposure: 0.0,
        toggles: 0,
        switch_flips: 0,
    };
    HarnessBuilder::new()
        .size([300.0, 400.0])
        .strict_glyphs(true)
        .build_ui(state, |ui, state| {
            panel_column(ui, "column", |ui| {
                let basic = Panel::new("Basic").show(ui, &mut state.group, |ui| {
                    adj_slider(ui, Adj::Exposure.spec(), &mut state.exposure, 0.0);
                });
                let curve = Panel::new("Tone Curve")
                    .enable_switch(&mut state.curve_enabled)
                    .show(ui, &mut state.group, |ui| {
                        ui.label("The curve goes here");
                    });
                let detail = Panel::new("Detail").show(ui, &mut state.group, |ui| {
                    ui.label("Sharpening goes here");
                });
                state.toggles += [basic.toggled, curve.toggled, detail.toggled]
                    .into_iter()
                    .map(u32::from)
                    .sum::<u32>();
                state.switch_flips += u32::from(curve.switch_changed);
            });
        })
}

fn open_panels(harness: &Harness<Host<Column>>) -> Vec<String> {
    let mut open: Vec<String> = harness
        .app()
        .state
        .group
        .open_panels()
        .map(str::to_owned)
        .collect();
    open.sort();
    open
}

#[test]
fn collapsing_a_panel_hides_its_content() -> TestResult {
    let mut harness = column(&["Basic"]);
    harness.run()?;
    assert_eq!(harness.count(Role::Slider, "Exposure"), 1);
    assert_eq!(
        harness.control(Role::DisclosureTriangle, "Basic")?.expanded,
        Some(true)
    );
    assert_eq!(harness.count(Role::Label, "Sharpening goes here"), 0);

    harness.click(Role::DisclosureTriangle, "Basic")?;
    assert_eq!(harness.count(Role::Slider, "Exposure"), 0);
    assert_eq!(
        harness.control(Role::DisclosureTriangle, "Basic")?.expanded,
        Some(false)
    );

    harness.click(Role::DisclosureTriangle, "Basic")?;
    harness.click(Role::DisclosureTriangle, "Detail")?;
    assert_eq!(harness.count(Role::Slider, "Exposure"), 1);
    assert_eq!(harness.count(Role::Label, "Sharpening goes here"), 1);
    assert_eq!(harness.app().state.toggles, 3);
    Ok(())
}

#[test]
fn the_enable_switch_toggles_without_collapsing_the_panel() -> TestResult {
    let mut harness = column(&["Tone Curve"]);
    harness.run()?;
    assert_eq!(
        harness.control(Role::Switch, "Enable Tone Curve")?.toggled,
        Some(true)
    );

    harness.click(Role::Switch, "Enable Tone Curve")?;
    assert!(!harness.app().state.curve_enabled);
    assert_eq!(
        harness.control(Role::Switch, "Enable Tone Curve")?.toggled,
        Some(false)
    );
    assert_eq!(harness.app().state.switch_flips, 1);
    assert_eq!(
        open_panels(&harness),
        ["Tone Curve"],
        "the header was not clicked"
    );
    assert_eq!(harness.count(Role::Label, "The curve goes here"), 1);

    harness.click(Role::Switch, "Enable Tone Curve")?;
    assert!(harness.app().state.curve_enabled);
    Ok(())
}

#[test]
fn in_solo_mode_opening_a_panel_closes_the_others() -> TestResult {
    let mut harness = column(&["Basic", "Tone Curve"]);
    harness.app_mut().state.group.set_solo(true);
    harness.run()?;
    assert_eq!(open_panels(&harness), ["Tone Curve"]);

    harness.click(Role::DisclosureTriangle, "Detail")?;
    assert_eq!(open_panels(&harness), ["Detail"]);
    assert_eq!(harness.count(Role::Label, "The curve goes here"), 0);

    harness.click(Role::DisclosureTriangle, "Basic")?;
    assert_eq!(open_panels(&harness), ["Basic"]);
    assert_eq!(harness.count(Role::Slider, "Exposure"), 1);
    assert_eq!(harness.count(Role::Label, "Sharpening goes here"), 0);
    Ok(())
}

#[test]
fn alt_clicking_a_header_switches_solo_mode() -> TestResult {
    let mut harness = column(&["Basic", "Tone Curve"]);
    harness.run()?;
    let header = harness
        .control(Role::DisclosureTriangle, "Detail")?
        .rect
        .center();

    harness.click_at_with(header, PointerButton::Primary, Modifiers::ALT)?;
    assert!(harness.app().state.group.solo());
    assert_eq!(open_panels(&harness), ["Detail"]);

    let header = harness
        .control(Role::DisclosureTriangle, "Detail")?
        .rect
        .center();
    harness.click_at_with(header, PointerButton::Primary, Modifiers::ALT)?;
    assert!(!harness.app().state.group.solo());
    harness.click(Role::DisclosureTriangle, "Basic")?;
    assert_eq!(open_panels(&harness), ["Basic", "Detail"]);
    Ok(())
}

#[test]
fn a_focused_header_opens_with_the_keyboard() -> TestResult {
    let mut harness = column(&[]);
    harness.run()?;
    harness.focus(Role::DisclosureTriangle, "Detail")?;
    harness.press_key(Key::Enter, Modifiers::NONE)?;
    assert_eq!(open_panels(&harness), ["Detail"]);
    harness.press_key(Key::Space, Modifiers::NONE)?;
    assert!(open_panels(&harness).is_empty());
    Ok(())
}
