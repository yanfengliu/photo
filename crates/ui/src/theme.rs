//! The visual system: one neutral dark palette, a spacing scale, type sizes
//! and rounding, and the egui style built from them.
//!
//! Everything the widgets paint takes its numbers from here, so the look is
//! changed in one place. Call [`apply`] once when the app is created.

use egui::epaint::Shadow;
use egui::style::{HandleShape, ScrollStyle, Selection, WidgetVisuals};
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, CornerRadius, FontId, Margin, Stroke, TextStyle, vec2};

/// Colours.
///
/// All chrome is exactly neutral (red = green = blue) so the interface never
/// biases colour judgement. The only hues are the single accent, the colour
/// labels a photographer assigns, and the warning and channel colours.
pub mod palette {
    use egui::Color32;

    /// The work area behind photos: near black.
    pub const WORK_AREA: Color32 = Color32::from_gray(20);
    /// Panel columns and bars.
    pub const PANEL: Color32 = Color32::from_gray(40);
    /// Panel headers: one step up from the panel they sit on.
    pub const HEADER: Color32 = Color32::from_gray(48);
    /// Wells that hold a plot (histogram, curve): one step down from the panel.
    pub const WELL: Color32 = Color32::from_gray(26);
    /// Raised controls at rest: buttons, switches, slider tracks.
    pub const RAISED: Color32 = Color32::from_gray(62);
    /// A raised control under the pointer.
    pub const HOVER: Color32 = Color32::from_gray(78);
    /// A raised control while it is pressed.
    pub const PRESSED: Color32 = Color32::from_gray(52);
    /// Hairlines that separate panels and bars.
    pub const SEPARATOR: Color32 = Color32::from_gray(18);
    /// Outlines of controls and faint grid lines inside wells.
    pub const OUTLINE: Color32 = Color32::from_gray(70);
    /// Normal text.
    pub const TEXT: Color32 = Color32::from_gray(208);
    /// Secondary text: labels of lesser importance, counts, units.
    pub const TEXT_DIM: Color32 = Color32::from_gray(140);
    /// Emphasised text and the foreground of hovered or active controls.
    pub const TEXT_STRONG: Color32 = Color32::from_gray(244);
    /// Text and marks that are switched off.
    pub const TEXT_OFF: Color32 = Color32::from_gray(92);

    /// The one accent: slider fills, switches that are on, keyboard focus.
    pub const ACCENT: Color32 = Color32::from_rgb(112, 150, 196);
    /// The accent under the pointer or while pressed.
    pub const ACCENT_STRONG: Color32 = Color32::from_rgb(150, 184, 224);
    /// The accent as a quiet background (selected text, default buttons).
    pub const ACCENT_DIM: Color32 = Color32::from_rgb(52, 74, 104);

    /// Selection tier 0: the mat an unselected photo cell sits on.
    pub const CELL: Color32 = Color32::from_gray(46);
    /// Selection tier 1: a selected cell.
    pub const CELL_SELECTED: Color32 = Color32::from_gray(80);
    /// Selection tier 2: the active (most selected) cell.
    pub const CELL_ACTIVE: Color32 = Color32::from_gray(118);

    /// The red colour label.
    pub const LABEL_RED: Color32 = Color32::from_rgb(204, 72, 66);
    /// The yellow colour label.
    pub const LABEL_YELLOW: Color32 = Color32::from_rgb(214, 186, 62);
    /// The green colour label.
    pub const LABEL_GREEN: Color32 = Color32::from_rgb(86, 168, 92);
    /// The blue colour label.
    pub const LABEL_BLUE: Color32 = Color32::from_rgb(70, 126, 208);
    /// The purple colour label.
    pub const LABEL_PURPLE: Color32 = Color32::from_rgb(150, 98, 196);

    /// Something needs attention (a missing file, a failed job).
    pub const WARNING: Color32 = Color32::from_rgb(226, 166, 62);
    /// Something went wrong.
    pub const ERROR: Color32 = Color32::from_rgb(216, 84, 76);
    /// Shadow clipping.
    pub const CLIP_SHADOWS: Color32 = Color32::from_rgb(70, 126, 208);
    /// Highlight clipping.
    pub const CLIP_HIGHLIGHTS: Color32 = Color32::from_rgb(216, 84, 76);

    /// Red channel of a histogram. The four channel colours are additive
    /// (they brighten what is under them) and sum to a light grey, so where
    /// two channels overlap the mix reads as yellow, cyan or magenta and
    /// where all overlap it reads as neutral.
    pub const CHANNEL_RED: Color32 = Color32::from_rgb_additive(108, 20, 18);
    /// Green channel of a histogram. Additive.
    pub const CHANNEL_GREEN: Color32 = Color32::from_rgb_additive(18, 90, 20);
    /// Blue channel of a histogram. Additive.
    pub const CHANNEL_BLUE: Color32 = Color32::from_rgb_additive(18, 34, 108);
    /// Luminance of a histogram. Additive.
    pub const CHANNEL_LUMA: Color32 = Color32::from_rgb_additive(18, 18, 18);
}

/// The spacing scale, in points.
pub mod space {
    /// Hairline gaps: between a control and its own parts.
    pub const XS: f32 = 2.0;
    /// Between related controls.
    pub const SM: f32 = 4.0;
    /// Between a label and its control.
    pub const MD: f32 = 6.0;
    /// Padding inside panels.
    pub const LG: f32 = 10.0;
    /// Between groups.
    pub const XL: f32 = 16.0;
}

/// Fixed sizes of controls, in points.
pub mod size {
    /// Height of one row of controls (a slider, a button, a list row).
    pub const ROW: f32 = 20.0;
    /// Height of a panel header.
    pub const HEADER: f32 = 24.0;
    /// Edge of an icon.
    pub const ICON: f32 = 16.0;
    /// Edge of an icon button.
    pub const ICON_BUTTON: f32 = 24.0;
    /// Width of the label column of a slider.
    pub const SLIDER_LABEL: f32 = 78.0;
    /// Width of the value column of a slider.
    pub const SLIDER_VALUE: f32 = 40.0;
    /// Thickness of a slider track.
    pub const TRACK: f32 = 2.0;
    /// Width and height of a slider handle.
    pub const HANDLE: [f32; 2] = [7.0, 13.0];
    /// Width of a panel column.
    pub const COLUMN: f32 = 280.0;
}

/// Type sizes, in points.
pub mod text {
    /// Captions, counts, index numbers.
    pub const SMALL: f32 = 10.5;
    /// Labels, values, buttons.
    pub const BODY: f32 = 12.0;
    /// Panel headers (set in spaced capitals).
    pub const HEADER: f32 = 10.5;
    /// Dialog titles and the app name.
    pub const TITLE: f32 = 14.0;
    /// Extra space between the capitals of a header.
    pub const HEADER_TRACKING: f32 = 1.1;
}

/// Corner radii, in points.
pub mod radius {
    /// Small marks: swatches, badges, slider tracks.
    pub const SM: u8 = 2;
    /// Controls: buttons, fields, photo cells.
    pub const MD: u8 = 3;
    /// Floating surfaces: dialogs, toasts, menus.
    pub const LG: u8 = 6;
}

/// The font of normal text.
pub fn body_font() -> FontId {
    FontId::proportional(text::BODY)
}

/// The font of captions.
pub fn small_font() -> FontId {
    FontId::proportional(text::SMALL)
}

/// The font of dialog titles.
pub fn title_font() -> FontId {
    FontId::proportional(text::TITLE)
}

/// `text` set as a panel header: spaced capitals in `color`.
///
/// The bundled fonts have no true small capitals, so this is the usual
/// substitute: full capitals at caption size with open tracking.
pub fn header_text(text: &str, color: Color32) -> LayoutJob {
    LayoutJob::single_section(
        text.to_uppercase(),
        TextFormat {
            font_id: FontId::proportional(text::HEADER),
            extra_letter_spacing: text::HEADER_TRACKING,
            color,
            ..Default::default()
        },
    )
}

/// `a` blended towards `b` by `t` (0 gives `a`, 1 gives `b`).
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    a.lerp_to_gamma(b, t.clamp(0.0, 1.0))
}

/// The frame of a panel column or bar: panel fill with the standard padding.
pub fn panel_frame() -> egui::Frame {
    egui::Frame::new().fill(palette::PANEL)
}

/// The frame of the work area: near black, no padding.
pub fn work_area_frame() -> egui::Frame {
    egui::Frame::new().fill(palette::WORK_AREA)
}

/// Install the visual system on `ctx`.
///
/// Call once when the app is created. Both of egui's theme slots get the same
/// dark style, so the operating system's light mode cannot change the look.
pub fn apply(ctx: &egui::Context) {
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(configure);
}

fn configure(style: &mut egui::Style) {
    style.text_styles = [
        (TextStyle::Small, small_font()),
        (TextStyle::Body, body_font()),
        (TextStyle::Button, body_font()),
        (TextStyle::Heading, title_font()),
        (TextStyle::Monospace, FontId::monospace(text::BODY - 0.5)),
    ]
    .into();
    style.drag_value_text_style = TextStyle::Body;

    let spacing = &mut style.spacing;
    spacing.item_spacing = vec2(space::MD, space::XS);
    spacing.button_padding = vec2(space::LG, space::XS);
    spacing.interact_size = vec2(32.0, size::ROW);
    spacing.indent = 14.0;
    spacing.icon_width = 13.0;
    spacing.icon_width_inner = 7.0;
    spacing.icon_spacing = space::MD;
    spacing.slider_rail_height = size::TRACK;
    spacing.window_margin = Margin::same(14);
    spacing.menu_margin = Margin::same(6);
    spacing.scroll = ScrollStyle::thin();

    style.interaction.selectable_labels = false;
    style.interaction.tooltip_delay = 0.6;

    // Whether the text cursor blinks is the host's choice (a test harness
    // turns it off), not part of the look.
    let blink = style.visuals.text_cursor.blink;
    style.visuals = visuals();
    style.visuals.text_cursor.blink = blink;
}

fn visuals() -> egui::Visuals {
    let rounding = CornerRadius::same(radius::MD);
    let widget = |fill: Color32, outline: Color32, foreground: Color32| WidgetVisuals {
        bg_fill: fill,
        weak_bg_fill: fill,
        bg_stroke: Stroke::new(1.0, outline),
        corner_radius: rounding,
        fg_stroke: Stroke::new(1.0, foreground),
        expansion: 0.0,
    };

    let mut visuals = egui::Visuals::dark();
    visuals.widgets.noninteractive = widget(palette::PANEL, palette::SEPARATOR, palette::TEXT);
    visuals.widgets.inactive = widget(palette::RAISED, palette::RAISED, palette::TEXT);
    visuals.widgets.hovered = widget(palette::HOVER, palette::HOVER, palette::TEXT_STRONG);
    visuals.widgets.active = widget(palette::PRESSED, palette::ACCENT, palette::TEXT_STRONG);
    visuals.widgets.open = widget(palette::PRESSED, palette::OUTLINE, palette::TEXT_STRONG);
    visuals.selection = Selection {
        bg_fill: palette::ACCENT_DIM,
        stroke: Stroke::new(1.0, palette::TEXT_STRONG),
    };
    visuals.weak_text_color = Some(palette::TEXT_DIM);
    visuals.hyperlink_color = palette::ACCENT;
    visuals.faint_bg_color = palette::HEADER;
    visuals.extreme_bg_color = palette::WELL;
    visuals.code_bg_color = palette::HEADER;
    visuals.warn_fg_color = palette::WARNING;
    visuals.error_fg_color = palette::ERROR;
    visuals.panel_fill = palette::PANEL;
    visuals.window_fill = palette::PANEL;
    visuals.window_stroke = Stroke::new(1.0, palette::OUTLINE);
    visuals.window_corner_radius = CornerRadius::same(radius::LG);
    visuals.menu_corner_radius = CornerRadius::same(radius::MD);
    visuals.window_shadow = Shadow {
        offset: [0, 10],
        blur: 28,
        spread: 0,
        color: Color32::from_black_alpha(140),
    };
    visuals.popup_shadow = Shadow {
        offset: [0, 4],
        blur: 12,
        spread: 0,
        color: Color32::from_black_alpha(120),
    };
    visuals.text_cursor.stroke = Stroke::new(1.0, palette::TEXT_STRONG);
    visuals.indent_has_left_vline = false;
    visuals.slider_trailing_fill = true;
    visuals.handle_shape = HandleShape::Rect { aspect_ratio: 0.5 };
    visuals.disabled_alpha = 0.4;
    visuals
}

#[cfg(test)]
mod tests {
    use super::*;

    fn is_neutral(color: Color32) -> bool {
        color.r() == color.g() && color.g() == color.b()
    }

    #[test]
    fn chrome_is_exactly_neutral() {
        for color in [
            palette::WORK_AREA,
            palette::PANEL,
            palette::HEADER,
            palette::WELL,
            palette::RAISED,
            palette::HOVER,
            palette::PRESSED,
            palette::SEPARATOR,
            palette::OUTLINE,
            palette::TEXT,
            palette::TEXT_DIM,
            palette::TEXT_STRONG,
            palette::TEXT_OFF,
            palette::CELL,
            palette::CELL_SELECTED,
            palette::CELL_ACTIVE,
        ] {
            assert!(is_neutral(color), "{color:?} has a colour cast");
        }
    }

    #[test]
    fn histogram_channels_sum_to_a_neutral_that_is_not_blown_out() {
        let channels = [
            palette::CHANNEL_RED,
            palette::CHANNEL_GREEN,
            palette::CHANNEL_BLUE,
            palette::CHANNEL_LUMA,
        ];
        assert!(channels.iter().all(|channel| channel.is_additive()));
        let sum = |pick: fn(&Color32) -> u8| -> u32 {
            u32::from(pick(&palette::WELL))
                + channels.iter().map(|c| u32::from(pick(c))).sum::<u32>()
        };
        let (red, green, blue) = (sum(Color32::r), sum(Color32::g), sum(Color32::b));
        assert!(
            red.abs_diff(green) <= 4 && green.abs_diff(blue) <= 4,
            "{red} {green} {blue}"
        );
        assert!(red < 240, "full overlap must stay below white: {red}");
    }

    #[test]
    fn selection_tiers_get_lighter_from_the_work_area_up() {
        let tiers = [
            palette::WORK_AREA,
            palette::CELL,
            palette::CELL_SELECTED,
            palette::CELL_ACTIVE,
        ];
        assert!(tiers.windows(2).all(|pair| pair[0].r() < pair[1].r()));
    }

    #[test]
    fn apply_installs_the_same_dark_style_in_both_theme_slots() {
        let ctx = egui::Context::default();
        apply(&ctx);
        assert_eq!(ctx.theme(), egui::Theme::Dark);
        for theme in [egui::Theme::Dark, egui::Theme::Light] {
            let style = ctx.style_of(theme);
            assert_eq!(style.visuals.panel_fill, palette::PANEL);
            assert_eq!(style.spacing.interact_size.y, size::ROW);
            assert!(!style.interaction.selectable_labels);
            assert_eq!(TextStyle::Body.resolve(&style).size, text::BODY);
        }
    }

    #[test]
    fn apply_keeps_the_hosts_cursor_blink_setting() {
        let ctx = egui::Context::default();
        ctx.all_styles_mut(|style| style.visuals.text_cursor.blink = false);
        apply(&ctx);
        assert!(!ctx.global_style().visuals.text_cursor.blink);
    }

    #[test]
    fn headers_are_set_in_capitals() {
        let job = header_text("Tone Curve", palette::TEXT);
        assert_eq!(job.text, "TONE CURVE");
        assert_eq!(
            job.sections[0].format.extra_letter_spacing,
            text::HEADER_TRACKING
        );
    }
}
