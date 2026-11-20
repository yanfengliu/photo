//! Key names and modifiers as the protocol spells them.
//!
//! A key is a single character (`"3"`, `"e"`, `"["`, `"\\"`) or a name, in
//! any case, with or without separators: `escape` or `esc`, `enter` or
//! `return`, `tab`, `space`, `backspace`, `delete`, `home`, `end`,
//! `page_up`, `page_down`, `left`, `right`, `up`, `down` (or `arrow_left` and
//! so on), `f1` to `f35`. Modifiers are `cmd`, `ctrl`, `shift` and `alt`,
//! held the way this platform's keyboard holds them: `cmd` is the Command key
//! on a Mac and Ctrl elsewhere, as egui's own `command` modifier is.

use egui::{Key, Modifiers};

use super::protocol::CmdError;

/// The key a protocol name stands for.
pub fn parse_key(name: &str) -> Result<Key, CmdError> {
    if let Some(key) = Key::from_name(name) {
        return Ok(key);
    }
    let wanted = normalize(name);
    let alias = match wanted.as_str() {
        "esc" => Some(Key::Escape),
        "return" => Some(Key::Enter),
        "del" => Some(Key::Delete),
        "ins" => Some(Key::Insert),
        "pgup" => Some(Key::PageUp),
        "pgdn" => Some(Key::PageDown),
        "spacebar" => Some(Key::Space),
        "arrowleft" => Some(Key::ArrowLeft),
        "arrowright" => Some(Key::ArrowRight),
        "arrowup" => Some(Key::ArrowUp),
        "arrowdown" => Some(Key::ArrowDown),
        _ => None,
    };
    alias
        .or_else(|| {
            Key::ALL
                .iter()
                .copied()
                .find(|key| normalize(key.name()) == wanted)
        })
        .ok_or_else(|| {
            CmdError::invalid(format!(
                "`{name}` is not a key; use a single character or a name such as escape, enter, tab, space, delete, left or f1"
            ))
        })
}

/// The modifiers a list of protocol names holds down.
pub fn parse_modifiers(names: &[String]) -> Result<Modifiers, CmdError> {
    let mac = cfg!(target_os = "macos");
    let mut held = Modifiers::NONE;
    for name in names {
        match normalize(name).as_str() {
            "cmd" | "command" | "meta" | "super" => {
                held.command = true;
                if mac {
                    held.mac_cmd = true;
                } else {
                    held.ctrl = true;
                }
            }
            "ctrl" | "control" => {
                held.ctrl = true;
                // Ctrl is the command key everywhere but on a Mac.
                held.command |= !mac;
            }
            "shift" => held.shift = true,
            "alt" | "option" | "opt" => held.alt = true,
            _ => {
                return Err(CmdError::invalid(format!(
                    "`{name}` is not a modifier; use cmd, ctrl, shift or alt"
                )));
            }
        }
    }
    Ok(held)
}

/// Lower case, without spaces, dashes or underscores.
fn normalize(name: &str) -> String {
    name.chars()
        .filter(|c| !matches!(c, ' ' | '_' | '-'))
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn characters_and_names_in_any_spelling_are_keys() {
        for (name, key) in [
            ("3", Key::Num3),
            ("e", Key::E),
            ("E", Key::E),
            ("[", Key::OpenBracket),
            ("\\", Key::Backslash),
            ("'", Key::Quote),
            ("escape", Key::Escape),
            ("Esc", Key::Escape),
            ("return", Key::Enter),
            ("page_up", Key::PageUp),
            ("arrow-left", Key::ArrowLeft),
            ("left", Key::ArrowLeft),
            ("space", Key::Space),
            ("delete", Key::Delete),
            ("f12", Key::F12),
        ] {
            assert_eq!(parse_key(name).unwrap(), key, "{name}");
        }
        assert!(parse_key("hyper").is_err());
        assert!(parse_key("").is_err());
    }

    #[test]
    fn cmd_is_the_command_key_of_this_platform() {
        let cmd = parse_modifiers(&["cmd".to_owned()]).unwrap();
        assert!(cmd.command && !cmd.shift && !cmd.alt);
        if cfg!(target_os = "macos") {
            assert!(cmd.mac_cmd && !cmd.ctrl);
            let ctrl = parse_modifiers(&["ctrl".to_owned()]).unwrap();
            assert!(
                ctrl.ctrl && !ctrl.command,
                "Control is not Command on a Mac"
            );
        } else {
            assert!(cmd.ctrl && !cmd.mac_cmd);
        }
        let both = parse_modifiers(&["Shift".to_owned(), "cmd".to_owned()]).unwrap();
        assert!(both.shift && both.command);
        assert!(parse_modifiers(&["hyper".to_owned()]).is_err());
        assert_eq!(parse_modifiers(&[]).unwrap(), Modifiers::NONE);
    }
}
