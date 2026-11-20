//! File names and folders: the naming template, the characters a file name
//! may not hold, and the numbered names of the unique-name policy.

use std::path::{Path, PathBuf};

use crate::item::ExportItem;
use crate::settings::{ExportSettings, ExtensionCase, Folder, Naming};

/// Longest stem kept, in bytes: room is left for a `-NNN` suffix and the
/// extension inside the 255-byte limit of common file systems.
const MAX_STEM_BYTES: usize = 200;

/// The stem used when neither the template nor the original name leaves
/// anything usable.
const LAST_RESORT: &str = "photo";

/// Names Windows reserves for devices, whatever the extension.
const RESERVED: [&str; 22] = [
    "CON", "PRN", "AUX", "NUL", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7", "COM8",
    "COM9", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9",
];

/// Where an item's file goes and what it is called, before the
/// existing-file policy is applied.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct NamePlan {
    /// The folder, subfolder included.
    pub(crate) folder: PathBuf,
    /// The file name without extension, already safe to use.
    pub(crate) stem: String,
    /// The extension in the requested case, without the dot.
    pub(crate) extension: String,
}

impl NamePlan {
    /// The plan for `item` under `settings`.
    pub(crate) fn new(item: &ExportItem, settings: &ExportSettings) -> Self {
        let location = &settings.location;
        let base = match &location.folder {
            Folder::Specific(path) => path.clone(),
            Folder::SameAsOriginal => match item.source_path.parent() {
                Some(parent) if !parent.as_os_str().is_empty() => parent.to_path_buf(),
                _ => PathBuf::from("."),
            },
        };
        let original = original_stem(item.source_path);
        let mut stem = sanitize(&expand(&settings.naming, item, &original));
        if stem.is_empty() {
            stem = sanitize(&original);
        }
        if stem.is_empty() {
            stem = LAST_RESORT.to_string();
        }
        let extension = settings.format.extension();
        Self {
            folder: base.join(subfolder(&location.subfolder)),
            stem,
            extension: match settings.naming.extension_case {
                ExtensionCase::Lower => extension.to_string(),
                ExtensionCase::Upper => extension.to_ascii_uppercase(),
            },
        }
    }

    /// Candidate `number`: the plain name for 1, `stem-N.ext` after that.
    pub(crate) fn candidate(&self, number: u32) -> PathBuf {
        let name = if number <= 1 {
            format!("{}.{}", self.stem, self.extension)
        } else {
            format!("{}-{number}.{}", self.stem, self.extension)
        };
        self.folder.join(name)
    }

    /// The first candidate, from `from` on, that names nothing on disk.
    pub(crate) fn first_free(&self, from: u32) -> u32 {
        (from.max(1)..u32::MAX)
            .find(|&number| !occupied(&self.candidate(number)))
            .unwrap_or(u32::MAX)
    }
}

/// True when something (a file, a folder, even a dangling link) has this
/// name.
pub(crate) fn occupied(path: &Path) -> bool {
    path.symlink_metadata().is_ok()
}

fn original_stem(path: &Path) -> String {
    path.file_stem()
        .map(|stem| stem.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// The template with its tokens replaced.
fn expand(naming: &Naming, item: &ExportItem, original: &str) -> String {
    let mut out = String::new();
    let mut rest = naming.template.as_str();
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let token = &after[..close];
        match token_value(token, naming, item, original) {
            Some(value) => out.push_str(&value),
            None => {
                out.push('{');
                out.push_str(token);
                out.push('}');
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// What a token stands for, or `None` when it is not a token.
fn token_value(token: &str, naming: &Naming, item: &ExportItem, original: &str) -> Option<String> {
    let token = token.trim().to_ascii_lowercase();
    match token.as_str() {
        "name" => Some(original.to_string()),
        "text" => Some(naming.custom_text.clone()),
        "seq" => Some(item.sequence.to_string()),
        "date" => Some(
            item.metadata
                .capture_time
                .map(|time| time.date_string())
                .unwrap_or_default(),
        ),
        _ => {
            let digits: usize = token.strip_prefix("seq:")?.trim().parse().ok()?;
            (1..=9)
                .contains(&digits)
                .then(|| format!("{:0digits$}", item.sequence))
        }
    }
}

/// `text` made safe as a file name: forbidden characters replaced by `_`,
/// leading and trailing dots and whitespace removed, Windows device names
/// suffixed with `_`, and the length capped.
pub(crate) fn sanitize(text: &str) -> String {
    let replaced: String = text
        .chars()
        .map(|c| if forbidden(c) { '_' } else { c })
        .collect();
    let mut name = replaced
        .trim_matches(|c: char| c.is_whitespace() || c == '.')
        .to_string();
    // Windows treats `NUL.txt` and `NUL.tar.gz` as the device too, so the
    // mark goes right after the device name.
    let device = name.split('.').next().unwrap_or_default().trim_end();
    let is_device = RESERVED
        .iter()
        .any(|reserved| reserved.eq_ignore_ascii_case(device));
    let device_end = device.len();
    if is_device {
        name.insert(device_end, '_');
    }
    if name.len() > MAX_STEM_BYTES {
        let mut end = MAX_STEM_BYTES;
        while !name.is_char_boundary(end) {
            end -= 1;
        }
        name.truncate(end);
        name = name
            .trim_end_matches(|c: char| c.is_whitespace() || c == '.')
            .to_string();
    }
    name
}

fn forbidden(c: char) -> bool {
    matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') || c.is_control()
}

/// The subfolder setting as a relative path of sanitized components.
fn subfolder(text: &str) -> PathBuf {
    text.split(['/', '\\'])
        .map(sanitize)
        .filter(|component| !component.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizing_replaces_forbidden_characters_and_trims_dots() {
        assert_eq!(sanitize("a/b\\c:d*e?f\"g<h>i|j"), "a_b_c_d_e_f_g_h_i_j");
        assert_eq!(sanitize("  ..hidden.. "), "hidden");
        assert_eq!(sanitize("tab\there"), "tab_here");
        assert_eq!(sanitize("..."), "");
        assert_eq!(sanitize("con"), "con_");
        assert_eq!(sanitize("LPT1.backup"), "LPT1_.backup");
        assert_eq!(sanitize("console"), "console");
    }

    #[test]
    fn long_names_are_cut_on_a_character_boundary() {
        let long = "é".repeat(150);
        let cut = sanitize(&long);
        assert!(cut.len() <= MAX_STEM_BYTES);
        assert!(cut.chars().all(|c| c == 'é'));
    }

    #[test]
    fn subfolders_stay_inside_the_chosen_folder() {
        assert_eq!(subfolder(""), PathBuf::new());
        assert_eq!(subfolder("web/large"), Path::new("web").join("large"));
        assert_eq!(subfolder("../../etc"), PathBuf::from("etc"));
        assert_eq!(subfolder("/abs\\olute/"), Path::new("abs").join("olute"));
        assert_eq!(subfolder("C:/x"), Path::new("C_").join("x"));
    }
}
