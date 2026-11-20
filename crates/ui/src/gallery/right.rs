//! The gallery's right column: histogram, the Basic panel, the tone curve
//! and the colour wheels, as the Develop module will stack them.

use egui::{Align, Layout, RichText, Ui, vec2};
use photo_core::develop::{Adj, DevelopSettings, describe_change};
use photo_core::meta::FileKind;

use super::{GalleryApp, demo};
use crate::theme::{palette, space};
use crate::widgets::{AdjSlider, ColorWheel, CurveEditor, Panel, Scale, histogram, panel_column};

/// The gallery's photo is a camera RAW file: temperature is in kelvin.
const KIND: FileKind = FileKind::Raw;

/// Show the right column.
pub(super) fn show(app: &mut GalleryApp, ui: &mut Ui) {
    let GalleryApp {
        settings,
        defaults,
        histogram: counts,
        clipping,
        right_panels: panels,
        curve_enabled,
        grading_enabled,
        ..
    } = app;
    // Sliders report "finished" when an edit ends; that is when the real app
    // records an undo step. The gallery confirms it with a toast.
    let mut finished = false;

    panel_column(ui, "right-panels", |ui| {
        Panel::new("Histogram").show(ui, panels, |ui| {
            histogram(ui, counts, clipping, 92.0);
            ui.add_space(space::XS);
            ui.horizontal(|ui| {
                for reading in ["ISO 200", "35 mm", "f/8", "1/250 s"] {
                    ui.label(RichText::new(reading).color(palette::TEXT_DIM).small());
                    ui.add_space(space::MD);
                }
            });
        });

        Panel::new("Basic").show(ui, panels, |ui| {
            let mut row = |ui: &mut Ui, adj| finished |= slider(ui, settings, defaults, adj, None);
            caption(ui, "White Balance");
            row(ui, Adj::Temperature);
            row(ui, Adj::Tint);
            caption(ui, "Tone");
            row(ui, Adj::Exposure);
            row(ui, Adj::Contrast);
            row(ui, Adj::Highlights);
            row(ui, Adj::Shadows);
            row(ui, Adj::Whites);
            row(ui, Adj::Blacks);
            caption(ui, "Presence");
            row(ui, Adj::Texture);
            row(ui, Adj::Clarity);
            row(ui, Adj::Dehaze);
            row(ui, Adj::Vibrance);
            row(ui, Adj::Saturation);
        });

        Panel::new("Tone Curve")
            .enable_switch(curve_enabled)
            .show(ui, panels, |ui| {
                let drawn = settings.tone_curve.rgb.clone();
                let sample = |x: f32| demo::smooth_curve(drawn.points(), x);
                let curve = &mut settings.tone_curve.rgb;
                let response = CurveEditor::new("Tone curve", curve, &sample)
                    .backdrop(&counts.luma)
                    .size([ui.available_width(), 176.0])
                    .show(ui);
                finished |= response.finished;
            });

        Panel::new("Color Grading")
            .enable_switch(grading_enabled)
            .show(ui, panels, |ui| {
                let grading = &mut settings.color_grading;
                ui.horizontal_top(|ui| {
                    // Whole points, so the three columns sit on the pixel grid.
                    let gaps = 2.0 * ui.spacing().item_spacing.x;
                    let column = vec2(((ui.available_width() - gaps) / 3.0).floor(), 0.0);
                    for (name, wheel, luminance) in [
                        ("Shadows", &mut grading.shadows, Adj::GradeShadowsLuminance),
                        (
                            "Midtones",
                            &mut grading.midtones,
                            Adj::GradeMidtonesLuminance,
                        ),
                        (
                            "Highlights",
                            &mut grading.highlights,
                            Adj::GradeHighlightsLuminance,
                        ),
                    ] {
                        ui.allocate_ui_with_layout(column, Layout::top_down(Align::Min), |ui| {
                            let response = ColorWheel::new(name, wheel, luminance.spec())
                                .diameter(78.0)
                                .show(ui);
                            finished |= response.finished;
                        });
                    }
                });
            });

        Panel::new("Detail").show(ui, panels, |ui| {
            let mut row = |ui: &mut Ui, adj, prefix| {
                finished |= slider(ui, settings, defaults, adj, Some(prefix));
            };
            caption(ui, "Sharpening");
            row(ui, Adj::SharpenAmount, "Sharpening");
            row(ui, Adj::SharpenRadius, "Sharpening");
            row(ui, Adj::SharpenDetail, "Sharpening");
            row(ui, Adj::SharpenMasking, "Sharpening");
            caption(ui, "Noise Reduction");
            row(ui, Adj::NoiseLuminance, "Noise");
            row(ui, Adj::NoiseLuminanceDetail, "Noise Luminance");
            row(ui, Adj::NoiseColor, "Noise");
            row(ui, Adj::NoiseColorDetail, "Noise Color");
        });

        Panel::new("Effects").show(ui, panels, |ui| {
            let mut row = |ui: &mut Ui, adj, prefix| {
                finished |= slider(ui, settings, defaults, adj, Some(prefix));
            };
            caption(ui, "Vignette");
            row(ui, Adj::VignetteAmount, "Vignette");
            row(ui, Adj::VignetteMidpoint, "Vignette");
            row(ui, Adj::VignetteFeather, "Vignette");
            caption(ui, "Grain");
            row(ui, Adj::GrainAmount, "Grain");
            row(ui, Adj::GrainSize, "Grain");
            row(ui, Adj::GrainRoughness, "Grain");
        });
    });

    if finished {
        app.commit_edit();
    }
}

impl GalleryApp {
    /// An edit of the develop settings has ended: name it and make it the
    /// state the next edit is compared against.
    fn commit_edit(&mut self) {
        // A drag that ends back where it began also reports "finished", so
        // the preview it drove can settle; there is nothing to record then.
        if self.committed == self.settings {
            return;
        }
        let label = describe_change(&self.committed, &self.settings, KIND);
        self.toasts.push(label);
        self.committed = self.settings.clone();
    }
}

/// One develop slider bound to `adj` in `settings`. Returns whether an edit
/// finished this frame.
fn slider(
    ui: &mut Ui,
    settings: &mut DevelopSettings,
    defaults: &DevelopSettings,
    adj: Adj,
    prefix: Option<&str>,
) -> bool {
    let mut value = adj.get(settings);
    let mut slider = AdjSlider::new(adj.spec_for(KIND), &mut value, adj.get(defaults));
    if adj == Adj::Temperature {
        // Kelvin spans 2000 to 50000: a linear track would squeeze the whole
        // useful range into its first tenth.
        slider = slider.scale(Scale::LOG);
    }
    if let Some(prefix) = prefix {
        slider = slider.name_prefix(prefix);
    }
    let response = slider.show(ui);
    if response.changed {
        adj.set(settings, value);
    }
    response.finished
}

/// A caption above a group of sliders.
fn caption(ui: &mut Ui, text: &str) {
    ui.add_space(space::SM);
    ui.label(RichText::new(text).color(palette::TEXT_DIM).small());
}
