//! File names and folders: the name a template gives (the `template` module
//! replaces its tokens), the characters a file name may not hold, and the
//! numbered names of the unique-name policy.

use std::path::{Path, PathBuf};

use crate::item::ExportItem;
use crate::settings::{ExportSettings, ExtensionCase, Folder};
use crate::template::{Values, expand};

/// Longest stem kept, in bytes: room is left for a `-NNN` suffix and the
/// extension inside the 255-byte limit of common file systems.
const MAX_STEM_BYTES: usize = 200;

/// The stem used when neither the template nor the original name leaves
/// anything usable.
const LAST_RESORT: &str = "photo";

/// Names Windows reserves for devices, whatever the extension: Microsoft's
/// list ("Naming Files, Paths, and Namespaces"), which includes the ports
/// numbered 0 and with the Latin-1 superscript digits ¹, ² and ³.
const RESERVED: [&str; 30] = [
    "CON", "PRN", "AUX", "NUL", "COM0", "COM1", "COM2", "COM3", "COM4", "COM5", "COM6", "COM7",
    "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT0", "LPT1", "LPT2", "LPT3", "LPT4", "LPT5", "LPT6",
    "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
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
        let values = Values {
            name: &original,
            text: &settings.naming.custom_text,
            sequence: item.sequence,
            date: item.metadata.capture_time.map(|time| time.date_string()),
        };
        let extension = settings.format.extension();
        Self {
            folder: base.join(subfolder(&location.subfolder)),
            stem: stem(&settings.naming.template, &values),
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

/// The file name without extension that `template` gives: its tokens
/// replaced, made safe to use, and never empty. When nothing usable is
/// left (no token has a value and only separators were typed, say) it is
/// the original file's name.
fn stem(template: &str, values: &Values) -> String {
    let mut stem = sanitize(&expand(template, values));
    if stem.is_empty() {
        stem = sanitize(values.name);
    }
    if stem.is_empty() {
        stem = LAST_RESORT.to_string();
    }
    stem
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

/// True for the characters no file name may hold; [`sanitize`] turns each
/// into `_`.
pub(crate) fn forbidden(c: char) -> bool {
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

    /// Review finding E4: the superscript forms survived. The whole class is
    /// Microsoft's list of reserved names, bare or followed by extensions.
    #[test]
    fn every_windows_device_name_is_marked_with_or_without_extensions() {
        let mut devices: Vec<String> = ["CON", "PRN", "AUX", "NUL"].map(String::from).to_vec();
        for port in ["COM", "LPT"] {
            for digit in "0123456789¹²³".chars() {
                devices.push(format!("{port}{digit}"));
            }
        }
        let is_device = |stem: &str| devices.iter().any(|d| d.eq_ignore_ascii_case(stem));
        for device in &devices {
            for name in [
                device.clone(),
                device.to_lowercase(),
                format!("{device}.txt"),
                format!("{device} .tar.gz"),
            ] {
                let safe = sanitize(&name);
                let stem = safe.split('.').next().unwrap().trim_end();
                assert!(!is_device(stem), "{name:?} became {safe:?}");
                assert_eq!(safe.replacen('_', "", 1), name, "only one mark is added");
            }
        }
        for kept in ["COM10", "COM", "LPT⁴", "CONSOLE", "AUXILIARY", "nul-x"] {
            assert_eq!(sanitize(kept), kept);
        }
    }

    /// When no token has a value, the name is the original file's rather
    /// than an empty one or a lone separator; a template that gives a name
    /// is made safe like any other.
    #[test]
    fn a_template_that_leaves_nothing_falls_back_to_the_original_name() {
        let values = |name, text| Values {
            name,
            text,
            sequence: 7,
            date: None,
        };
        for template in [
            "{text}",
            "{date}-{text}",
            "{date} _-. {text}",
            "-{text}-",
            "...",
        ] {
            assert_eq!(
                stem(template, &values("beach", "")),
                "beach",
                "{template:?}"
            );
        }
        assert_eq!(
            stem("{date}-{text}-{seq:3}", &values("beach", "coast")),
            "coast-007"
        );
        assert_eq!(stem("{date}/{text}:{name}", &values("beach", "")), "beach");
        assert_eq!(stem("{name}_{text}", &values("b/each", "")), "b_each");
        // Not even the original has a usable name.
        assert_eq!(stem("{text}", &values("", "")), LAST_RESORT);
        assert_eq!(stem("{name}-{text}", &values("...", "")), LAST_RESORT);
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
