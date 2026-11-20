//! The pick / reject flag toggle.

use egui::{Response, Ui};
use photo_core::meta::Flag;

use super::icons::{Icon, IconButton};

/// Two toggle buttons that show and edit `flag`: pick and reject.
///
/// Clicking a button sets that flag; clicking the one that is already on
/// removes the flag. The buttons are named `"{name} pick"` and
/// `"{name} reject"`. The returned response is marked changed when the flag
/// was edited.
pub fn flag_toggle(ui: &mut Ui, name: &str, flag: &mut Flag) -> Response {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let pick = button(
            ui,
            Icon::FlagPick,
            &format!("{name} pick"),
            Flag::Picked,
            flag,
        );
        let reject = button(
            ui,
            Icon::FlagReject,
            &format!("{name} reject"),
            Flag::Rejected,
            flag,
        );
        pick.union(reject)
    })
    .inner
}

fn button(ui: &mut Ui, icon: Icon, label: &str, target: Flag, flag: &mut Flag) -> Response {
    let mut response = ui.add(IconButton::new(icon, label).selected(*flag == target));
    if response.clicked() {
        *flag = toggled(*flag, target);
        response.mark_changed();
    }
    response
}

/// The flag after clicking the button for `target` while `current` is set.
fn toggled(current: Flag, target: Flag) -> Flag {
    if current == target {
        Flag::Unflagged
    } else {
        target
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clicking_a_flag_sets_it_and_clicking_it_again_removes_it() {
        assert_eq!(toggled(Flag::Unflagged, Flag::Picked), Flag::Picked);
        assert_eq!(toggled(Flag::Picked, Flag::Picked), Flag::Unflagged);
        assert_eq!(toggled(Flag::Picked, Flag::Rejected), Flag::Rejected);
        assert_eq!(toggled(Flag::Rejected, Flag::Rejected), Flag::Unflagged);
    }
}
