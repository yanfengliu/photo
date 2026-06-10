//! Lightroom-inspired color palette and iced style functions.

use iced::widget::{button, container};
use iced::{Background, Border, Color, Theme};

pub(crate) const BG_DARK: Color = Color::from_rgb(0.118, 0.118, 0.118);
pub(crate) const BG_PANEL: Color = Color::from_rgb(0.153, 0.153, 0.153);
pub(crate) const BG_TOOLBAR: Color = Color::from_rgb(0.176, 0.176, 0.176);
pub(crate) const BG_CARD: Color = Color::from_rgb(0.165, 0.165, 0.165);
pub(crate) const BG_BUTTON: Color = Color::from_rgb(0.22, 0.22, 0.22);
pub(crate) const BG_BUTTON_HOVER: Color = Color::from_rgb(0.28, 0.28, 0.28);
pub(crate) const TEXT_PRIMARY: Color = Color::from_rgb(0.82, 0.82, 0.82);
pub(crate) const TEXT_SECONDARY: Color = Color::from_rgb(0.55, 0.55, 0.55);
pub(crate) const TEXT_DIM: Color = Color::from_rgb(0.40, 0.40, 0.40);
pub(crate) const DIVIDER: Color = Color::from_rgb(0.22, 0.22, 0.22);
pub(crate) fn toolbar_container_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_TOOLBAR)),
        ..Default::default()
    }
}

pub(crate) fn panel_container_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_PANEL)),
        border: Border {
            color: DIVIDER,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

pub(crate) fn dark_bg_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_DARK)),
        ..Default::default()
    }
}

pub(crate) fn toolbar_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Some(Background::Color(BG_BUTTON_HOVER)),
        button::Status::Pressed => Some(Background::Color(BG_DARK)),
        _ => Some(Background::Color(BG_BUTTON)),
    };
    button::Style {
        background: bg,
        text_color: TEXT_PRIMARY,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 3.0.into(),
        },
        shadow: Default::default(),
    }
}

pub(crate) fn card_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => BG_BUTTON_HOVER,
        button::Status::Pressed => BG_DARK,
        _ => BG_CARD,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: TEXT_PRIMARY,
        border: Border {
            color: DIVIDER,
            width: 1.0,
            radius: 4.0.into(),
        },
        shadow: Default::default(),
    }
}

pub(crate) fn invisible_button_style(_theme: &Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: None,
        text_color: TEXT_SECONDARY,
        border: Border::default(),
        shadow: Default::default(),
    }
}

pub(crate) fn sidebar_item_style(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Some(Background::Color(BG_BUTTON_HOVER)),
        _ => None,
    };
    button::Style {
        background: bg,
        text_color: TEXT_SECONDARY,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 3.0.into(),
        },
        shadow: Default::default(),
    }
}

pub(crate) fn sidebar_item_drop_target_style(
    _theme: &Theme,
    _status: button::Status,
) -> button::Style {
    button::Style {
        background: Some(Background::Color(Color::from_rgb(0.2, 0.3, 0.4))),
        text_color: TEXT_PRIMARY,
        border: Border {
            color: Color::from_rgb(0.3, 0.5, 0.7),
            width: 1.0,
            radius: 3.0.into(),
        },
        shadow: Default::default(),
    }
}

pub(crate) fn context_menu_container_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgb(0.2, 0.2, 0.2))),
        border: Border {
            color: Color::from_rgb(0.3, 0.3, 0.3),
            width: 1.0,
            radius: 4.0.into(),
        },
        ..Default::default()
    }
}

pub(crate) fn context_menu_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Some(Background::Color(Color::from_rgb(0.3, 0.4, 0.55))),
        _ => None,
    };
    button::Style {
        background: bg,
        text_color: TEXT_PRIMARY,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 2.0.into(),
        },
        shadow: Default::default(),
    }
}
