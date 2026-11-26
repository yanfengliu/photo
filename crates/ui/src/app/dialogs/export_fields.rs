//! The Export dialog's file settings: format with its quality or bit depth,
//! colour space, sizing, output sharpening, metadata and the watermark.
//! Each control edits the export settings in place.

use egui::{ComboBox, DragValue, RichText, TextEdit, Ui};
use photo_core::colorimetry::RgbSpace;
use photo_engine::{
    Anchor, BitDepth, ExportSettings, Format, MetadataMode, OutputSharpening, Resize, Watermark,
};

use super::export::{row, section};
use crate::theme::palette;

/// Values kept while the control that uses them is switched off, so that
/// switching back finds them.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct Kept {
    quality: u8,
    depth: BitDepth,
    watermark: Watermark,
}

impl Default for Kept {
    fn default() -> Self {
        Self {
            quality: 90,
            depth: BitDepth::Eight,
            watermark: Watermark::default(),
        }
    }
}

impl Kept {
    /// Take what `settings` use as what is kept.
    pub(super) fn follow(&mut self, settings: &ExportSettings) {
        match settings.format {
            Format::Jpeg { quality } => self.quality = quality,
            Format::Png { depth } | Format::Tiff { depth } => self.depth = depth,
        }
        if let Some(watermark) = &settings.watermark {
            self.watermark.clone_from(watermark);
        }
    }
}

/// The file formats, as the menu lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Jpeg,
    Png,
    Tiff,
}

impl Kind {
    const ALL: [(Self, &'static str); 3] = [
        (Self::Jpeg, "JPEG"),
        (Self::Png, "PNG"),
        (Self::Tiff, "TIFF"),
    ];

    fn of(format: Format) -> Self {
        match format {
            Format::Jpeg { .. } => Self::Jpeg,
            Format::Png { .. } => Self::Png,
            Format::Tiff { .. } => Self::Tiff,
        }
    }

    fn label(self) -> &'static str {
        Self::ALL
            .iter()
            .find(|(kind, _)| *kind == self)
            .map_or("", |(_, label)| label)
    }
}

/// The ways to size the picture, as the menu lists them, each with the
/// size it starts at.
const SIZES: [(&str, Resize); 6] = [
    ("Full size", Resize::Original),
    ("Long edge", Resize::LongEdge { pixels: 2048 }),
    ("Short edge", Resize::ShortEdge { pixels: 1080 }),
    (
        "Fit within",
        Resize::Fit {
            width: 1920,
            height: 1080,
        },
    ),
    ("Megapixels", Resize::Megapixels { megapixels: 2.0 }),
    ("Percent", Resize::Percent { percent: 50.0 }),
];

/// Which entry of [`SIZES`] a resize is.
fn size_index(resize: Resize) -> usize {
    match resize {
        Resize::Original => 0,
        Resize::LongEdge { .. } => 1,
        Resize::ShortEdge { .. } => 2,
        Resize::Fit { .. } => 3,
        Resize::Megapixels { .. } => 4,
        Resize::Percent { .. } => 5,
    }
}

/// What the menu calls each place a watermark can sit.
fn anchor_label(anchor: Anchor) -> &'static str {
    match anchor {
        Anchor::TopLeft => "Top left",
        Anchor::Top => "Top",
        Anchor::TopRight => "Top right",
        Anchor::Left => "Left",
        Anchor::Center => "Centre",
        Anchor::Right => "Right",
        Anchor::BottomLeft => "Bottom left",
        Anchor::Bottom => "Bottom",
        Anchor::BottomRight => "Bottom right",
    }
}

/// A dim word after a number.
fn unit(ui: &mut Ui, text: &str) {
    ui.label(RichText::new(text).color(palette::TEXT_DIM));
}

/// Show every section.
pub(super) fn show(ui: &mut Ui, settings: &mut ExportSettings, kept: &mut Kept) {
    section(ui, "File Settings");
    format_rows(ui, settings, kept);
    section(ui, "Image Sizing");
    sizing_row(ui, settings);
    section(ui, "Output");
    output_rows(ui, settings);
    section(ui, "Watermark");
    watermark_rows(ui, settings, kept);
}

fn format_rows(ui: &mut Ui, settings: &mut ExportSettings, kept: &mut Kept) {
    row(ui, "Format", |ui| {
        let was = Kind::of(settings.format);
        let mut kind = was;
        ComboBox::from_id_salt("export-format")
            .selected_text(was.label())
            .width(90.0)
            .show_ui(ui, |ui| {
                for (choice, label) in Kind::ALL {
                    ui.selectable_value(&mut kind, choice, label);
                }
            })
            .response
            .accessible_name("Export format");
        if kind != was {
            settings.format = match kind {
                Kind::Jpeg => Format::Jpeg {
                    quality: kept.quality,
                },
                Kind::Png => Format::Png { depth: kept.depth },
                Kind::Tiff => Format::Tiff { depth: kept.depth },
            };
        }
        match &mut settings.format {
            Format::Jpeg { quality } => {
                unit(ui, "Quality");
                ui.add(DragValue::new(quality).range(0..=100))
                    .accessible_name("JPEG quality");
                kept.quality = *quality;
            }
            Format::Png { depth } | Format::Tiff { depth } => {
                ui.radio_value(depth, BitDepth::Eight, "8 bits");
                ui.radio_value(depth, BitDepth::Sixteen, "16 bits");
                kept.depth = *depth;
            }
        }
    });
    row(ui, "Colour space", |ui| {
        ComboBox::from_id_salt("export-colour-space")
            .selected_text(settings.color_space.name())
            .width(140.0)
            .show_ui(ui, |ui| {
                for space in RgbSpace::ALL {
                    ui.selectable_value(&mut settings.color_space, space, space.name());
                }
            })
            .response
            .accessible_name("Colour space");
        unit(ui, "Its profile is embedded in every file.");
    });
}

fn sizing_row(ui: &mut Ui, settings: &mut ExportSettings) {
    let sizing = &mut settings.sizing;
    row(ui, "Size", |ui| {
        let was = size_index(sizing.resize);
        let mut index = was;
        ComboBox::from_id_salt("export-resize")
            .selected_text(SIZES[was].0)
            .width(110.0)
            .show_ui(ui, |ui| {
                for (choice, (label, _)) in SIZES.iter().enumerate() {
                    ui.selectable_value(&mut index, choice, *label);
                }
            })
            .response
            .accessible_name("Resize");
        if index != was {
            sizing.resize = SIZES[index].1;
        }
        match &mut sizing.resize {
            Resize::Original => {}
            Resize::LongEdge { pixels } | Resize::ShortEdge { pixels } => {
                ui.add(DragValue::new(pixels).range(1..=65_535))
                    .accessible_name("Edge pixels");
                unit(ui, "px");
            }
            Resize::Fit { width, height } => {
                ui.add(DragValue::new(width).range(1..=65_535))
                    .accessible_name("Fit width");
                unit(ui, "x");
                ui.add(DragValue::new(height).range(1..=65_535))
                    .accessible_name("Fit height");
                unit(ui, "px");
            }
            Resize::Megapixels { megapixels } => {
                ui.add(DragValue::new(megapixels).range(0.01..=500.0).speed(0.1))
                    .accessible_name("Megapixels");
                unit(ui, "MP");
            }
            Resize::Percent { percent } => {
                ui.add(DragValue::new(percent).range(1.0..=400.0).speed(0.5))
                    .accessible_name("Size percent");
                unit(ui, "%");
            }
        }
        ui.checkbox(&mut sizing.dont_enlarge, "Don't enlarge");
    });
}

fn output_rows(ui: &mut Ui, settings: &mut ExportSettings) {
    row(ui, "Sharpening", |ui| {
        let label = |amount: OutputSharpening| match amount {
            OutputSharpening::Off => "Off",
            OutputSharpening::Low => "Low",
            OutputSharpening::Standard => "Standard",
            OutputSharpening::High => "High",
        };
        ComboBox::from_id_salt("export-sharpening")
            .selected_text(label(settings.sharpening))
            .width(110.0)
            .show_ui(ui, |ui| {
                for amount in [
                    OutputSharpening::Off,
                    OutputSharpening::Low,
                    OutputSharpening::Standard,
                    OutputSharpening::High,
                ] {
                    ui.selectable_value(&mut settings.sharpening, amount, label(amount));
                }
            })
            .response
            .accessible_name("Output sharpening");
        unit(ui, "For the screen, after sizing.");
    });
    row(ui, "Metadata", |ui| {
        let metadata = &mut settings.metadata;
        let label = |mode: MetadataMode| match mode {
            MetadataMode::All => "All",
            MetadataMode::CopyrightAndContact => "Copyright and contact only",
            MetadataMode::None => "None",
        };
        ComboBox::from_id_salt("export-metadata")
            .selected_text(label(metadata.include))
            .width(200.0)
            .show_ui(ui, |ui| {
                for mode in [
                    MetadataMode::All,
                    MetadataMode::CopyrightAndContact,
                    MetadataMode::None,
                ] {
                    ui.selectable_value(&mut metadata.include, mode, label(mode));
                }
            })
            .response
            .accessible_name("Metadata");
        ui.checkbox(&mut metadata.strip_location, "Remove location");
    });
}

fn watermark_rows(ui: &mut Ui, settings: &mut ExportSettings, kept: &mut Kept) {
    let mut used = settings.watermark.is_some();
    row(ui, "", |ui| {
        if ui.checkbox(&mut used, "Add a watermark").changed() {
            settings.watermark = used.then(|| kept.watermark.clone());
        }
    });
    let Some(watermark) = &mut settings.watermark else {
        return;
    };
    row(ui, "Text", |ui| {
        ui.add(
            TextEdit::singleline(&mut watermark.text)
                .id_salt("export-watermark")
                .hint_text("One line, over the picture")
                .desired_width(280.0),
        )
        .accessible_name("Watermark text");
    });
    row(ui, "Place", |ui| {
        ComboBox::from_id_salt("export-watermark-place")
            .selected_text(anchor_label(watermark.anchor))
            .width(120.0)
            .show_ui(ui, |ui| {
                for anchor in Anchor::ALL {
                    ui.selectable_value(&mut watermark.anchor, anchor, anchor_label(anchor));
                }
            })
            .response
            .accessible_name("Watermark position");
        unit(ui, "Size");
        ui.add(
            DragValue::new(&mut watermark.size)
                .range(0.5..=25.0)
                .speed(0.1),
        )
        .accessible_name("Watermark size");
        unit(ui, "% of the short side");
        unit(ui, "Opacity");
        ui.add(DragValue::new(&mut watermark.opacity).range(0.0..=100.0))
            .accessible_name("Watermark opacity");
    });
    kept.watermark.clone_from(watermark);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_way_to_size_has_its_menu_entry() {
        for (index, (_, resize)) in SIZES.iter().enumerate() {
            assert_eq!(size_index(*resize), index);
        }
    }

    #[test]
    fn what_a_switched_off_control_held_is_kept_for_switching_back() {
        let mut kept = Kept::default();
        let mut settings = ExportSettings {
            format: Format::Jpeg { quality: 72 },
            ..ExportSettings::default()
        };
        settings.watermark = Some(Watermark {
            text: "A. Photographer".to_owned(),
            ..Watermark::default()
        });
        kept.follow(&settings);
        assert_eq!(kept.quality, 72);
        assert_eq!(kept.watermark.text, "A. Photographer");
        settings.format = Format::Tiff {
            depth: BitDepth::Sixteen,
        };
        kept.follow(&settings);
        assert_eq!((kept.quality, kept.depth), (72, BitDepth::Sixteen));
        assert_eq!(Kind::of(settings.format).label(), "TIFF");
    }
}
