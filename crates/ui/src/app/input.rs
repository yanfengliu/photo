//! The Library's keyboard shortcuts (design §8.5).
//!
//! [`shortcut`] maps one key press to what it means, matching modifiers
//! exactly (Command is Ctrl on Windows and Linux, as egui's `command`
//! modifier already is). [`take_shortcuts`] takes the presses that are
//! shortcuts out of the frame's input, so no widget acts on them as well.
//! Nothing here runs while a text field has focus or a dialog is up.

use egui::{Event, FocusDirection, Key, Modifiers};
use photo_core::meta::{ColorLabel, Flag};

use super::state::View;

/// Which way an arrow key moves.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Arrow {
    Left,
    Right,
    Up,
    Down,
}

/// What a key press asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shortcut {
    View(View),
    Develop,
    ToggleZoom,
    Rating(u8),
    RatingDown,
    RatingUp,
    Flag(Flag),
    Label(ColorLabel),
    QuickCollection,
    Arrow { arrow: Arrow, extend: bool },
    First,
    Last,
    Undo,
    Redo,
    SelectAll,
    SelectNone,
    Delete,
    RotateLeft,
    RotateRight,
    VirtualCopy,
    Import,
    NewCollection,
    FindText,
    ToggleSides,
    ToggleEdges,
    ToggleToolbar,
}

impl Shortcut {
    /// Whether holding the key down repeats it. Only movement does: a held
    /// rating key must not set the same rating sixty times a second.
    fn repeats(self) -> bool {
        matches!(self, Self::Arrow { .. } | Self::RatingDown | Self::RatingUp)
    }
}

/// What `key` pressed with `modifiers` means, if anything.
pub(crate) fn shortcut(key: Key, modifiers: Modifiers) -> Option<Shortcut> {
    // Alt, and Control on a Mac, are not part of any Library shortcut.
    if modifiers.alt || (modifiers.ctrl && !modifiers.command) {
        return None;
    }
    let (command, shift) = (modifiers.command, modifiers.shift);
    let arrow = |arrow| {
        (!command).then_some(Shortcut::Arrow {
            arrow,
            extend: shift,
        })
    };
    match key {
        Key::ArrowLeft => return arrow(Arrow::Left),
        Key::ArrowRight => return arrow(Arrow::Right),
        Key::ArrowUp => return arrow(Arrow::Up),
        Key::ArrowDown => return arrow(Arrow::Down),
        _ => {}
    }
    let found = match (command, shift, key) {
        (false, false, Key::G) => Shortcut::View(View::Grid),
        (false, false, Key::E) => Shortcut::View(View::Loupe),
        (false, false, Key::C) => Shortcut::View(View::Compare),
        (false, false, Key::N) => Shortcut::View(View::Survey),
        (false, false, Key::D) => Shortcut::Develop,
        (false, false, Key::Space | Key::Z) => Shortcut::ToggleZoom,
        (false, false, Key::Num0) => Shortcut::Rating(0),
        (false, false, Key::Num1) => Shortcut::Rating(1),
        (false, false, Key::Num2) => Shortcut::Rating(2),
        (false, false, Key::Num3) => Shortcut::Rating(3),
        (false, false, Key::Num4) => Shortcut::Rating(4),
        (false, false, Key::Num5) => Shortcut::Rating(5),
        (false, false, Key::OpenBracket) => Shortcut::RatingDown,
        (false, false, Key::CloseBracket) => Shortcut::RatingUp,
        (false, false, Key::P) => Shortcut::Flag(Flag::Picked),
        (false, false, Key::X) => Shortcut::Flag(Flag::Rejected),
        (false, false, Key::U) => Shortcut::Flag(Flag::Unflagged),
        (false, false, Key::Num6) => Shortcut::Label(ColorLabel::Red),
        (false, false, Key::Num7) => Shortcut::Label(ColorLabel::Yellow),
        (false, false, Key::Num8) => Shortcut::Label(ColorLabel::Green),
        (false, false, Key::Num9) => Shortcut::Label(ColorLabel::Blue),
        (false, false, Key::B) => Shortcut::QuickCollection,
        (false, false, Key::Home) => Shortcut::First,
        (false, false, Key::End) => Shortcut::Last,
        (false, false, Key::Delete | Key::Backspace) => Shortcut::Delete,
        (false, false, Key::T) => Shortcut::ToggleToolbar,
        (false, false, Key::Tab) => Shortcut::ToggleSides,
        (false, true, Key::Tab) => Shortcut::ToggleEdges,
        (true, false, Key::Z) => Shortcut::Undo,
        (true, true, Key::Z) => Shortcut::Redo,
        (true, false, Key::A) => Shortcut::SelectAll,
        (true, false, Key::D) => Shortcut::SelectNone,
        (true, false, Key::OpenBracket) => Shortcut::RotateLeft,
        (true, false, Key::CloseBracket) => Shortcut::RotateRight,
        (true, false, Key::Quote) => Shortcut::VirtualCopy,
        (true, true, Key::I) => Shortcut::Import,
        (true, false, Key::N) => Shortcut::NewCollection,
        (true, false, Key::F) => Shortcut::FindText,
        _ => return None,
    };
    Some(found)
}

/// Take this frame's shortcut presses out of its input, oldest first.
///
/// A Tab that is a shortcut also cancels the keyboard-focus move egui
/// started for it when the frame began.
pub(crate) fn take_shortcuts(ctx: &egui::Context) -> Vec<Shortcut> {
    let mut found = Vec::new();
    ctx.input_mut(|input| {
        input.events.retain(|event| {
            let Event::Key {
                key,
                pressed,
                repeat,
                modifiers,
                ..
            } = event
            else {
                return true;
            };
            let Some(shortcut) = shortcut(*key, *modifiers) else {
                return true;
            };
            if *pressed && (!*repeat || shortcut.repeats()) {
                found.push(shortcut);
            }
            false
        });
    });
    if found
        .iter()
        .any(|shortcut| matches!(shortcut, Shortcut::ToggleSides | Shortcut::ToggleEdges))
    {
        ctx.memory_mut(|memory| memory.move_focus(FocusDirection::None));
    }
    found
}

#[cfg(test)]
mod tests {
    use super::*;

    const SHIFT_COMMAND: Modifiers = Modifiers {
        alt: false,
        ctrl: false,
        shift: true,
        mac_cmd: false,
        command: true,
    };

    #[test]
    fn plain_keys_rate_flag_label_and_switch_views() {
        let plain = |key| shortcut(key, Modifiers::NONE);
        assert_eq!(plain(Key::Num0), Some(Shortcut::Rating(0)));
        assert_eq!(plain(Key::Num5), Some(Shortcut::Rating(5)));
        assert_eq!(plain(Key::P), Some(Shortcut::Flag(Flag::Picked)));
        assert_eq!(plain(Key::X), Some(Shortcut::Flag(Flag::Rejected)));
        assert_eq!(plain(Key::U), Some(Shortcut::Flag(Flag::Unflagged)));
        assert_eq!(plain(Key::Num6), Some(Shortcut::Label(ColorLabel::Red)));
        assert_eq!(plain(Key::Num9), Some(Shortcut::Label(ColorLabel::Blue)));
        assert_eq!(plain(Key::G), Some(Shortcut::View(View::Grid)));
        assert_eq!(plain(Key::E), Some(Shortcut::View(View::Loupe)));
        assert_eq!(plain(Key::C), Some(Shortcut::View(View::Compare)));
        assert_eq!(plain(Key::N), Some(Shortcut::View(View::Survey)));
        assert_eq!(plain(Key::Space), Some(Shortcut::ToggleZoom));
        assert_eq!(plain(Key::Backspace), Some(Shortcut::Delete));
        assert_eq!(plain(Key::Q), None);
    }

    #[test]
    fn modifiers_are_matched_exactly() {
        assert_eq!(shortcut(Key::Z, Modifiers::COMMAND), Some(Shortcut::Undo));
        assert_eq!(shortcut(Key::Z, SHIFT_COMMAND), Some(Shortcut::Redo));
        assert_eq!(shortcut(Key::I, SHIFT_COMMAND), Some(Shortcut::Import));
        assert_eq!(shortcut(Key::I, Modifiers::COMMAND), None);
        assert_eq!(shortcut(Key::N, Modifiers::COMMAND), Some(Shortcut::NewCollection));
        assert_eq!(shortcut(Key::Tab, Modifiers::SHIFT), Some(Shortcut::ToggleEdges));
        assert_eq!(shortcut(Key::Num3, Modifiers::COMMAND), None, "not a rating");
        assert_eq!(shortcut(Key::G, Modifiers::ALT), None);
        let mac_control = Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        };
        assert_eq!(shortcut(Key::A, mac_control), None);
    }

    #[test]
    fn shift_extends_an_arrow_and_only_movement_repeats() {
        assert_eq!(
            shortcut(Key::ArrowRight, Modifiers::SHIFT),
            Some(Shortcut::Arrow {
                arrow: Arrow::Right,
                extend: true
            })
        );
        assert_eq!(shortcut(Key::ArrowRight, Modifiers::COMMAND), None);
        assert!(shortcut(Key::ArrowDown, Modifiers::NONE).is_some_and(Shortcut::repeats));
        assert!(!Shortcut::Rating(3).repeats());
    }
}
