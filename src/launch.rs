//! Starting Photo: the command line, and opening the photo it names.
//!
//! ```text
//! photo [PATH]                                  the app in a window
//! photo --harness[=PORT] [--harness-real-storage] [PATH]
//!                                               the app driven over localhost
//! ```
//!
//! A path opens that photo at startup, harness or not: imported in place if
//! the catalog does not hold it yet, made the active photo, and shown in
//! the loupe. `photo_ui` does the opening (`open_in_engine`) and owns the
//! window (`run_with`); this module only puts the two together.

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use photo_ui::{AppOptions, Module, View};

/// The port `--harness` listens on unless told otherwise.
pub const DEFAULT_PORT: u16 = 7878;

/// How long opening a photo may take: its import is a metadata read.
pub const OPEN_TIMEOUT: Duration = Duration::from_secs(120);

/// What the command line asks for.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Args {
    /// Run headless behind the agent harness, on this port (0: any free one).
    pub harness: Option<u16>,
    /// Let a harness session use the user's real catalog.
    pub real_storage: bool,
    /// The photo to open at startup.
    pub open: Option<PathBuf>,
}

/// How the command line is used, for error messages.
pub const USAGE: &str = "usage: photo [--harness[=PORT] [--harness-real-storage]] [PATH]";

impl Args {
    /// Read the arguments after the program name.
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut parsed = Self::default();
        let mut paths_only = false;
        for arg in args {
            let text = arg.to_string_lossy().into_owned();
            if paths_only || !text.starts_with("--") {
                if parsed.open.is_some() {
                    return Err(format!("only one photo can be opened\n{USAGE}"));
                }
                parsed.open = Some(PathBuf::from(arg));
                continue;
            }
            match text.split_once('=') {
                _ if text == "--" => paths_only = true,
                _ if text == "--harness" => parsed.harness = Some(DEFAULT_PORT),
                Some(("--harness", port)) => {
                    let port = port
                        .parse()
                        .map_err(|_| format!("`{port}` is not a port number\n{USAGE}"))?;
                    parsed.harness = Some(port);
                }
                _ if text == "--harness-real-storage" => parsed.real_storage = true,
                _ => return Err(format!("unknown option `{text}`\n{USAGE}")),
            }
        }
        if parsed.real_storage && parsed.harness.is_none() {
            return Err(format!(
                "--harness-real-storage only goes with --harness\n{USAGE}"
            ));
        }
        Ok(parsed)
    }
}

/// The app in a window with the photo at `path` open in the loupe: the
/// photo is opened in the engine first, so a file that cannot be imported
/// is reported before any window appears.
pub fn run_window_with(options: AppOptions, path: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let mut engine = options.open_engine()?;
    let opened = photo_ui::open_in_engine(&mut engine, path, OPEN_TIMEOUT)?;
    log::info!(
        "opened {} (photo {})",
        opened.path.display(),
        opened.photo.0
    );
    photo_ui::run_with(engine, options, Module::Library, View::Loupe)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        Args::parse(args.iter().map(OsString::from))
    }

    #[test]
    fn the_harness_listens_on_7878_unless_given_a_port() {
        assert_eq!(parse(&[]).unwrap(), Args::default());
        assert_eq!(parse(&["--harness"]).unwrap().harness, Some(DEFAULT_PORT));
        assert_eq!(parse(&["--harness=0"]).unwrap().harness, Some(0));
        assert_eq!(parse(&["--harness=9001"]).unwrap().harness, Some(9001));
        assert!(parse(&["--harness=port"]).is_err());
        assert!(parse(&["--harness=70000"]).is_err());
    }

    #[test]
    fn a_bare_path_is_the_photo_to_open_harness_or_not() {
        let window = parse(&["shot.jpg"]).unwrap();
        assert_eq!(
            (window.harness, window.open),
            (None, Some(PathBuf::from("shot.jpg")))
        );
        let harness = parse(&["--harness", "--harness-real-storage", "raw.dng"]).unwrap();
        assert!(harness.real_storage);
        assert_eq!(harness.open, Some(PathBuf::from("raw.dng")));
        assert_eq!(
            parse(&["--", "--odd-name.jpg"]).unwrap().open,
            Some(PathBuf::from("--odd-name.jpg"))
        );
        assert!(parse(&["a.jpg", "b.jpg"]).is_err());
    }

    #[test]
    fn real_storage_only_goes_with_the_harness_and_unknown_options_are_refused() {
        assert!(parse(&["--harness-real-storage"]).is_err());
        assert!(parse(&["--fullscreen"]).is_err());
    }
}
