//! The masking tool's panel: the list of masks (show or hide each, select
//! one), and for the selected mask its name, invert and delete, its shapes
//! (add one, cut one out, remove one), the overlay switch, the amount and
//! the local adjustments, and the options of the selected shape.

use egui::{Align, Button, Layout, RichText, TextEdit, Ui, vec2};
use photo_core::develop::{AdjSpec, LocalAdj, MaskShape, SettingsGroup};

use super::brush;
use super::Armed;
use crate::app::Cx;
use crate::app::develop::right::Editing;
use crate::app::develop::tools::{MaskKind, edit};
use crate::theme::{palette, size, space};
use crate::widgets::{AdjSlider, Icon, IconButton, ListRow};

/// How much of a mask's adjustments is applied.
const AMOUNT: AdjSpec = AdjSpec {
    key: "mask_amount",
    label: "Amount",
    min: 0.0,
    max: 100.0,
    step: 1.0,
    decimals: 0,
    group: SettingsGroup::Masking,
};

/// How soft a radial gradient's edge is.
const RADIAL_FEATHER: AdjSpec = AdjSpec {
    key: "radial_feather",
    label: "Feather",
    min: 0.0,
    max: 100.0,
    step: 1.0,
    decimals: 0,
    group: SettingsGroup::Masking,
};

/// The kinds of shape, as the menus list them.
const KINDS: [(MaskKind, &str); 3] = [
    (MaskKind::Linear, "Linear Gradient"),
    (MaskKind::Radial, "Radial Gradient"),
    (MaskKind::Brush, "Brush"),
];

/// A dim caption.
fn caption(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(palette::TEXT_DIM).small());
}

/// Show the panel.
pub(crate) fn panel(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing) {
    let masks = &editing.settings.masks;
    {
        let tool = &mut cx.ui.develop.tools.mask;
        if tool.selected.is_some_and(|index| index >= masks.len()) {
            (tool.selected, tool.component) = (None, 0);
        }
        if let Some(mask) = tool.selected
            && tool.component >= masks[mask].components.len()
        {
            tool.component = 0;
        }
    }
    if masks.is_empty() {
        caption(
            ui,
            "Choose a gradient or the brush above, then drag on the photo to make a mask.",
        );
    }
    list(ui, cx, editing);
    match cx.ui.develop.tools.mask.selected {
        Some(index) => selected(ui, cx, editing, index),
        None => {
            if cx.ui.develop.tools.mask.painting(masks) {
                brush_options(ui, cx);
            }
        }
    }
}

/// One row per mask: its switch and its name.
fn list(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing) {
    for (index, mask) in editing.settings.masks.iter().enumerate() {
        let number = index + 1;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = space::XS;
            let switch = IconButton::new(Icon::Eye, &format!("Enable mask {number}"))
                .selected(mask.enabled)
                .side(size::ROW);
            if ui.add(switch).clicked() {
                let mut settings = editing.settings.clone();
                settings.masks[index].enabled = !mask.enabled;
                edit(cx, settings);
            }
            let name = format!("Mask {number}: {}", mask.name);
            let tool = &mut cx.ui.develop.tools.mask;
            let row = ListRow::new(&name, &mask.name)
                .selected(tool.selected == Some(index))
                .show(ui);
            if row.clicked {
                (tool.selected, tool.component, tool.armed) = (Some(index), 0, None);
            }
        });
    }
}

/// Everything about the selected mask.
fn selected(ui: &mut Ui, cx: &mut Cx<'_>, editing: &Editing, index: usize) {
    let mask = &editing.settings.masks[index];
    let change = |cx: &mut Cx<'_>, apply: &dyn Fn(&mut photo_core::develop::Mask)| {
        let mut settings = editing.settings.clone();
        apply(&mut settings.masks[index]);
        edit(cx, settings);
    };

    ui.add_space(space::SM);
    ui.horizontal(|ui| {
        ui.label(RichText::new("Name").color(palette::TEXT));
        let mut name = mask.name.clone();
        let field = ui
            .add(
                TextEdit::singleline(&mut name)
                    .id_salt(("mask-name", index))
                    .desired_width(f32::INFINITY),
            )
            .accessible_name("Mask name");
        if field.changed() {
            change(cx, &|mask| mask.name.clone_from(&name));
        }
    });
    ui.horizontal(|ui| {
        let mut invert = mask.invert;
        if ui
            .checkbox(&mut invert, "Invert")
            .accessible_name("Invert mask")
            .changed()
        {
            change(cx, &|mask| mask.invert = invert);
        }
        let mut overlay = cx.ui.develop.tools.mask.overlay;
        if ui
            .checkbox(&mut overlay, "Overlay")
            .accessible_name("Show mask overlay")
            .on_hover_text("Show where the mask covers the photo, in red (O)")
            .changed()
        {
            cx.ui.develop.tools.mask.overlay = overlay;
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add(Button::new("Delete Mask"))
                .on_hover_text("Remove this mask (Delete)")
                .clicked()
            {
                super::delete_selected(cx);
            }
        });
    });

    ui.add_space(space::SM);
    caption(ui, "Shapes");
    let shown = cx.ui.develop.tools.mask.component;
    for (place, component) in mask.components.iter().enumerate() {
        let cut = if component.subtract { " (cut out)" } else { "" };
        let label = format!("{}{cut}", component.shape.label());
        let name = format!("Shape {}: {label}", place + 1);
        let row = ListRow::new(&name, &label)
            .depth(1)
            .selected(shown == place)
            .show(ui);
        if row.clicked {
            let tool = &mut cx.ui.develop.tools.mask;
            (tool.component, tool.armed) = (place, None);
        }
    }
    ui.horizontal(|ui| {
        for (subtract, title, hint) in [
            (false, "Add", "Add a shape to this mask: choose one, then drag on the photo"),
            (true, "Subtract", "Cut a shape out of this mask: choose one, then drag on the photo"),
        ] {
            let armed = cx.ui.develop.tools.mask.armed;
            let lit = armed.is_some_and(|armed| armed.into == Some(subtract));
            let title = RichText::new(title).color(if lit {
                palette::ACCENT_STRONG
            } else {
                palette::TEXT
            });
            ui.menu_button(title, |ui| {
                for (kind, label) in KINDS {
                    if ui.button(label).clicked() {
                        cx.ui.develop.tools.mask.armed = Some(Armed {
                            kind,
                            into: Some(subtract),
                        });
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text(hint);
        }
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if ui
                .add_enabled(mask.components.len() > 1, Button::new("Remove Shape"))
                .clicked()
            {
                change(cx, &|mask| {
                    if shown < mask.components.len() {
                        mask.components.remove(shown);
                    }
                });
                cx.ui.develop.tools.mask.component = 0;
            }
        });
    });

    ui.add_space(space::SM);
    let mut amount = mask.amount;
    if AdjSlider::new(AMOUNT, &mut amount, 100.0)
        .name_prefix("Mask")
        .show(ui)
        .changed
    {
        change(cx, &|mask| mask.amount = amount);
    }
    for adj in LocalAdj::ALL {
        let mut value = adj.get(&mask.adjust);
        if AdjSlider::new(adj.spec(), &mut value, 0.0)
            .name_prefix("Mask")
            .show(ui)
            .changed
        {
            change(cx, &|mask| adj.set(&mut mask.adjust, adj.spec().clamp(value)));
        }
    }

    let armed = cx.ui.develop.tools.mask.armed;
    let shape = mask.components.get(shown).map(|component| &component.shape);
    match (armed.map(|armed| armed.kind), shape) {
        (Some(MaskKind::Brush), _) | (None, Some(MaskShape::Brush { .. })) => {
            brush_options(ui, cx);
        }
        (
            None,
            Some(MaskShape::Radial {
                feather, invert, ..
            }),
        ) => {
            ui.add_space(space::SM);
            caption(ui, "Radial Gradient");
            let (mut soft, mut inverted) = (*feather, *invert);
            let slid = AdjSlider::new(RADIAL_FEATHER, &mut soft, 50.0)
                .name_prefix("Radial")
                .show(ui)
                .changed;
            let ticked = ui
                .checkbox(&mut inverted, "Invert")
                .accessible_name("Invert radial")
                .on_hover_text("Cover everything but the ellipse")
                .changed();
            if slid || ticked {
                change(cx, &|mask| {
                    if let Some(MaskShape::Radial {
                        feather, invert, ..
                    }) = mask.components.get_mut(shown).map(|part| &mut part.shape)
                    {
                        (*feather, *invert) = (soft, inverted);
                    }
                });
            }
        }
        _ => {}
    }
}

/// The brush the next stroke is painted with.
fn brush_options(ui: &mut Ui, cx: &mut Cx<'_>) {
    ui.add_space(space::SM);
    caption(ui, "Brush");
    let options = &mut cx.ui.develop.tools.mask.brush;
    AdjSlider::new(brush::SIZE, &mut options.size, 3.0)
        .name_prefix("Brush")
        .show(ui);
    AdjSlider::new(brush::FEATHER, &mut options.feather, 50.0)
        .name_prefix("Brush")
        .show(ui);
    AdjSlider::new(brush::FLOW, &mut options.flow, 100.0)
        .name_prefix("Brush")
        .show(ui);
    ui.allocate_ui(vec2(ui.available_width(), 0.0), |ui| {
        caption(ui, "Drag on the photo to paint; hold Alt to erase.");
    });
}
