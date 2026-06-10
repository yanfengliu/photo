//! Reusable widget builders: rotation buttons, thumbnail slots, grid layout, menu items.

use crate::app::Message;
use crate::theme::*;
use iced::widget::image::Handle as ImageHandle;
use iced::widget::{button, column, container, horizontal_space, text, Image};
use iced::{Alignment, Background, Element, Length, Theme};

pub(crate) const GRID_THUMB_SIZE: f32 = 150.0;
pub(crate) const GRID_SPACING: f32 = 8.0;
pub(crate) const GRID_PADDING: f32 = 14.0;
pub(crate) const GRID_CARD_PADDING: f32 = 6.0;
pub(crate) const ROTATE_COUNTERCLOCKWISE_ICON: &str = "\u{21BA}";
pub(crate) const ROTATE_CLOCKWISE_ICON: &str = "\u{21BB}";
pub(crate) const ROTATE_COUNTERCLOCKWISE_STEP_LABEL: &str = "-90\u{00B0}";
pub(crate) const ROTATE_CLOCKWISE_STEP_LABEL: &str = "+90\u{00B0}";
pub(crate) const ROTATION_ICON_FONT_FAMILY: &str = "Segoe UI Symbol";
pub(crate) const ROTATION_ICON_FONT: iced::Font = iced::Font::with_name(ROTATION_ICON_FONT_FAMILY);
pub(crate) const ROTATION_ICON_SHAPING: iced::widget::text::Shaping =
    iced::widget::text::Shaping::Advanced;
pub(crate) fn rotation_icon_label<'a, ThemeT, RendererT>(
    icon: &'static str,
) -> iced::widget::Text<'a, ThemeT, RendererT>
where
    ThemeT: iced::widget::text::Catalog + 'a,
    RendererT: iced::advanced::text::Renderer<Font = iced::Font>,
{
    // These glyphs are not consistently present in the default text font.
    text(icon)
        .font(ROTATION_ICON_FONT)
        .shaping(ROTATION_ICON_SHAPING)
        .size(16)
}

pub(crate) fn rotation_button_widget<'a, RendererT>(
    icon: &'static str,
    step_label: &'static str,
    message: Message,
) -> iced::widget::Button<'a, Message, iced::Theme, RendererT>
where
    RendererT: iced::advanced::Renderer + iced::advanced::text::Renderer<Font = iced::Font> + 'a,
{
    button(
        column![
            rotation_icon_label(icon).color(TEXT_PRIMARY),
            text(step_label).size(10).color(TEXT_SECONDARY)
        ]
        .width(Length::Fill)
        .align_x(Alignment::Center)
        .spacing(2),
    )
    .on_press(message)
    .width(Length::Fill)
    .padding([6, 10])
    .style(toolbar_button_style)
}

pub(crate) fn rotation_button(
    icon: &'static str,
    step_label: &'static str,
    message: Message,
) -> Element<'static, Message> {
    rotation_button_widget::<iced::Renderer>(icon, step_label, message).into()
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct ThumbnailGridLayout {
    pub(crate) thumb_size: f32,
    pub(crate) columns: usize,
}

impl ThumbnailGridLayout {
    pub(crate) fn new(content_width: f32) -> Self {
        let card_width = GRID_THUMB_SIZE + GRID_CARD_PADDING * 2.0;
        let usable_width = (content_width - GRID_PADDING * 2.0).max(card_width);
        let columns =
            ((usable_width + GRID_SPACING) / (card_width + GRID_SPACING)).floor() as usize;
        Self {
            thumb_size: GRID_THUMB_SIZE,
            columns: columns.max(1),
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

pub(crate) fn thumbnail_slot_with_renderer<'a, RendererT>(
    handle: ImageHandle,
    slot_size: f32,
) -> Element<'a, Message, iced::Theme, RendererT>
where
    RendererT:
        iced::advanced::Renderer + iced::advanced::image::Renderer<Handle = ImageHandle> + 'a,
{
    container(
        Image::new(handle)
            .width(slot_size)
            .height(slot_size)
            .content_fit(iced::ContentFit::Contain),
    )
    .width(slot_size)
    .height(slot_size)
    .center_x(Length::Shrink)
    .center_y(Length::Shrink)
    .into()
}

pub(crate) fn thumbnail_slot(handle: ImageHandle, slot_size: f32) -> Element<'static, Message> {
    thumbnail_slot_with_renderer::<iced::Renderer>(handle, slot_size)
}

// ---------------------------------------------------------------------------
// Application state
// ---------------------------------------------------------------------------

pub(crate) fn section_label(title: &str) -> Element<'_, Message> {
    container(text(title).size(10).color(TEXT_DIM))
        .padding([5, 0])
        .into()
}

pub(crate) fn section_divider() -> Element<'static, Message> {
    container(horizontal_space())
        .width(Length::Fill)
        .height(1)
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(DIVIDER)),
            ..Default::default()
        })
        .into()
}

pub(crate) fn context_menu_item(
    label: impl Into<String>,
    msg: Message,
) -> Element<'static, Message> {
    button(text(label.into()).size(12).color(TEXT_PRIMARY))
        .on_press(msg)
        .padding([4, 12])
        .width(Length::Fill)
        .style(context_menu_button_style)
        .into()
}
