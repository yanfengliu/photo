//! The gallery: every widget of the kit in one window, arranged the way the
//! real screens will use them.
//!
//! It exists to look at and to poke: a left column of collapsible panels, a
//! grid and a filmstrip of photo cells in every state, and a right column
//! with a histogram, a full Basic panel of develop sliders, the tone-curve
//! editor and the colour wheels. Nothing here is application logic; values
//! are edited in place and the only consequence is a toast.

mod center;
mod demo;
mod left;
mod right;
mod textures;

use egui::{Align, Layout, Margin, RichText};
use photo_core::develop::{CurvePoint, DevelopSettings, PointCurve, WbPreset};
use photo_core::meta::FileKind;

pub use demo::Photo;
use textures::Picture;

use crate::theme::{self, palette, space};
use crate::widgets::{ClippingToggles, HistogramData, PanelGroup, SelectionTier, Toasts};

/// Height of the bar across the top of the window.
const TOP_BAR: f32 = 30.0;
/// Width of the left panel column.
const LEFT_COLUMN: f32 = 264.0;
/// Width of the right panel column.
const RIGHT_COLUMN: f32 = 300.0;

/// The module shown as chosen in the top bar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Module {
    Library,
    Develop,
}

/// The view shown as chosen in the toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum View {
    Grid,
    Loupe,
    Compare,
    Survey,
}

/// The gallery app. See the [module documentation](self).
pub struct GalleryApp {
    pictures: Vec<Picture>,
    photos: Vec<Photo>,
    settings: DevelopSettings,
    /// What a double-click resets each slider to.
    defaults: DevelopSettings,
    /// The settings as of the last finished edit.
    committed: DevelopSettings,
    histogram: HistogramData,
    clipping: ClippingToggles,
    left_panels: PanelGroup,
    right_panels: PanelGroup,
    curve_enabled: bool,
    grading_enabled: bool,
    module: Module,
    view: View,
    collections: Vec<String>,
    /// The name being typed while the New Collection dialog is open.
    new_collection: Option<String>,
    toasts: Toasts,
}

impl GalleryApp {
    /// Build the gallery: apply the theme and generate its stand-in photos.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        theme::apply(&cc.egui_ctx);
        let pictures = textures::generate(&cc.egui_ctx);
        let photos = demo::roll(pictures.len());

        // A camera RAW photo as it comes in, then a believable first edit.
        let mut defaults = DevelopSettings::default_for(FileKind::Raw);
        defaults.white_balance.preset = WbPreset::Custom;
        defaults.white_balance.temp = 5500.0;
        defaults.white_balance.tint = 10.0;
        let mut settings = defaults.clone();
        settings.white_balance.temp = 6150.0;
        settings.tone.exposure = 0.35;
        settings.tone.contrast = 12.0;
        settings.tone.highlights = -42.0;
        settings.tone.shadows = 28.0;
        settings.tone.blacks = -8.0;
        settings.presence.clarity = 10.0;
        settings.presence.vibrance = 18.0;
        settings.tone_curve.rgb = PointCurve::new([
            CurvePoint::new(0.0, 0.0),
            CurvePoint::new(0.27, 0.2),
            CurvePoint::new(0.72, 0.8),
            CurvePoint::new(1.0, 1.0),
        ]);
        settings.color_grading.shadows.hue = 222.0;
        settings.color_grading.shadows.saturation = 34.0;
        settings.color_grading.highlights.hue = 46.0;
        settings.color_grading.highlights.saturation = 22.0;
        settings.color_grading.midtones.luminance = 6.0;

        Self {
            pictures,
            photos,
            committed: settings.clone(),
            settings,
            defaults,
            histogram: demo::histogram(),
            clipping: ClippingToggles::default(),
            left_panels: PanelGroup::with_open([
                "Navigator",
                "Catalog",
                "Folders",
                "Collections",
                "Icons",
            ]),
            right_panels: PanelGroup::with_open([
                "Histogram",
                "Basic",
                "Tone Curve",
                "Color Grading",
            ]),
            curve_enabled: true,
            grading_enabled: true,
            module: Module::Develop,
            view: View::Grid,
            collections: vec![
                "Portfolio".to_owned(),
                "Iceland 2026".to_owned(),
                "To print".to_owned(),
            ],
            new_collection: None,
            toasts: Toasts::new(),
        }
    }

    /// The develop settings the right column edits.
    pub fn settings(&self) -> &DevelopSettings {
        &self.settings
    }

    /// The photos shown in the grid and the filmstrip.
    pub fn photos(&self) -> &[Photo] {
        &self.photos
    }

    /// The names in the Collections panel.
    pub fn collections(&self) -> &[String] {
        &self.collections
    }

    /// The photo the panels describe, if any.
    fn active(&self) -> Option<&Photo> {
        self.photos
            .iter()
            .find(|photo| photo.tier == SelectionTier::Active)
    }

    /// Apply `edit` to every selected photo and confirm it with a toast.
    fn edit_selection(&mut self, message: String, edit: impl Fn(&mut Photo)) {
        self.photos
            .iter_mut()
            .filter(|photo| photo.tier != SelectionTier::None)
            .for_each(edit);
        self.toasts.push(message);
    }

    fn top_bar(&mut self, ui: &mut egui::Ui) {
        let frame = theme::panel_frame().inner_margin(Margin::symmetric(12, 0));
        egui::Panel::top("top-bar")
            .exact_size(TOP_BAR)
            .frame(frame)
            .show(ui, |ui| {
                ui.horizontal_centered(|ui| {
                    let name = RichText::new("Photo")
                        .font(theme::title_font())
                        .color(palette::TEXT_STRONG);
                    ui.label(name);
                    ui.add_space(space::LG);
                    ui.label(RichText::new("Widget gallery").color(palette::TEXT_DIM));
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        // Right to left: the last module is added first.
                        for (module, label) in
                            [(Module::Develop, "Develop"), (Module::Library, "Library")]
                        {
                            if ui.selectable_label(self.module == module, label).clicked() {
                                self.module = module;
                            }
                        }
                    });
                });
            });
    }
}

impl eframe::App for GalleryApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.top_bar(ui);
        egui::Panel::left("left-column")
            .exact_size(LEFT_COLUMN)
            .resizable(false)
            .frame(theme::panel_frame())
            .show(ui, |ui| left::show(self, ui));
        egui::Panel::right("right-column")
            .exact_size(RIGHT_COLUMN)
            .resizable(false)
            .frame(theme::panel_frame())
            .show(ui, |ui| right::show(self, ui));
        // Added after the columns, so these two span only the space between.
        center::filmstrip(self, ui);
        center::toolbar(self, ui);
        let work_area = egui::CentralPanel::default()
            .frame(theme::work_area_frame())
            .show(ui, |ui| {
                center::grid(self, ui);
                ui.max_rect()
            })
            .inner;

        self.toasts.show(ui.ctx(), work_area);
        left::new_collection_dialog(self, ui.ctx());
    }
}
