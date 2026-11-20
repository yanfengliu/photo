//! The Metadata panel: what the active photo's file says about it (name,
//! folder, capture time, camera, lens, exposure, ISO, focal length,
//! dimensions) and its title, caption, creator and copyright, which are
//! edited in place and saved on Enter or when the field is left.

use egui::{Align, Label, Layout, RichText, TextEdit, Ui, vec2};
use photo_core::meta::PhotoId;
use photo_engine::{PhotoRecord, Target, TextField};

use crate::app::Cx;
use crate::app::actions::Action;
use crate::app::text;
use crate::theme::{palette, size};

/// Width of the label column.
const LABEL: f32 = 86.0;

/// The editable fields, in the order they are shown.
const FIELDS: [(TextField, &str); 4] = [
    (TextField::Title, "Title"),
    (TextField::Caption, "Caption"),
    (TextField::Creator, "Creator"),
    (TextField::Copyright, "Copyright"),
];

/// The text of the editable fields as typed, for the photo they belong to.
#[derive(Clone, Debug, Default)]
pub(crate) struct MetadataDraft {
    photo: Option<PhotoId>,
    fields: [String; 4],
}

/// The value of `field` in `record`.
fn value(record: &PhotoRecord, field: TextField) -> &str {
    match field {
        TextField::Title => &record.title,
        TextField::Caption => &record.caption,
        TextField::Creator => &record.creator,
        TextField::Copyright => &record.copyright,
    }
}

/// Show the panel.
pub(super) fn show(ui: &mut Ui, cx: &mut Cx<'_>) {
    let Some(photo) = cx.engine.active_photo() else {
        ui.label(RichText::new("Select a photo to see its metadata.").color(palette::TEXT_DIM));
        return;
    };
    let generation = cx.generation;
    let Some(record) = cx.ui.record.get(cx.engine, photo, generation).cloned() else {
        ui.label(RichText::new("The photo's record cannot be read.").color(palette::TEXT_DIM));
        return;
    };
    readings(ui, &record);
    ui.add_space(size::ROW / 2.0);
    let draft = &mut cx.ui.right.metadata;
    if draft.photo != Some(photo) {
        draft.photo = Some(photo);
        for (slot, (field, _)) in draft.fields.iter_mut().zip(FIELDS) {
            *slot = value(&record, field).to_owned();
        }
    }
    let mut commits = Vec::new();
    for (index, (field, label)) in FIELDS.into_iter().enumerate() {
        let id = ui.make_persistent_id(("metadata", label));
        let saved = value(&record, field);
        let slot = &mut draft.fields[index];
        // A field not being edited follows the catalog (an undo, say).
        if !ui.memory(|memory| memory.has_focus(id)) && slot != saved {
            *slot = saved.to_owned();
        }
        ui.horizontal(|ui| {
            label_cell(ui, label);
            let response = ui
                .add(
                    TextEdit::singleline(slot)
                        .id(id)
                        .desired_width(f32::INFINITY),
                )
                .accessible_name(label);
            let left = response.lost_focus();
            let set_from_outside = response.changed() && !response.has_focus();
            if (left || set_from_outside) && slot != saved {
                commits.push((field, slot.clone()));
            }
        });
    }
    for (field, text) in commits {
        cx.act(Action::SetText(Target::Photos(vec![photo]), field, text));
    }
}

/// The read-only rows.
fn readings(ui: &mut Ui, record: &PhotoRecord) {
    let metadata = &record.metadata;
    let folder = record
        .path
        .parent()
        .map(text::file_name)
        .unwrap_or_default();
    let dimensions = (metadata.width > 0 && metadata.height > 0)
        .then(|| format!("{} × {}", metadata.width, metadata.height));
    let rows = [
        ("File Name", Some(record.file_name.clone())),
        ("Folder", Some(folder)),
        (
            "Capture Time",
            metadata.capture_time.as_ref().map(text::date_time),
        ),
        ("Camera", metadata.camera_name()),
        ("Lens", metadata.lens.clone()),
        ("Exposure", text::exposure(metadata)),
        ("ISO", metadata.iso.map(|iso| iso.to_string())),
        ("Focal Length", text::focal_length(metadata)),
        ("Dimensions", dimensions),
    ];
    for (label, shown) in rows {
        ui.horizontal(|ui| {
            label_cell(ui, label);
            let shown = shown.filter(|text| !text.is_empty());
            let (text, color) = match &shown {
                Some(text) => (text.as_str(), palette::TEXT),
                None => ("—", palette::TEXT_OFF),
            };
            ui.add(Label::new(RichText::new(text).color(color)).truncate());
        });
    }
}

/// A field's label, right-aligned in the label column.
fn label_cell(ui: &mut Ui, label: &str) {
    ui.allocate_ui_with_layout(
        vec2(LABEL, size::ROW),
        Layout::right_to_left(Align::Center),
        |ui| ui.label(RichText::new(label).color(palette::TEXT_DIM)),
    );
}
