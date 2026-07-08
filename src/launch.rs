//! CLI-argument parsing and process-wide launch options.
//!
//! `main()` parses argv once into [`CliOptions`] and publishes them here;
//! `App::new` and the harness read them back. When `main` has not published
//! (unit tests constructing `App` directly), accessors fall back to the
//! legacy behavior of treating `argv[1]` as an optional image path.

use std::path::PathBuf;
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub(crate) struct CliOptions {
    pub(crate) harness: Option<HarnessLaunch>,
    pub(crate) image_path: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct HarnessLaunch {
    pub(crate) port: u16,
    pub(crate) real_storage: bool,
}

/// Parses process arguments (without the program name). Unknown `--flags` are
/// ignored with a warning so older scripts keep launching newer builds; the
/// first bare argument is the optional image path (the pre-harness contract).
pub(crate) fn parse_cli_args(args: &[String]) -> CliOptions {
    let mut options = CliOptions::default();
    let mut real_storage_requested = false;

    for arg in args {
        if arg == "--harness" {
            options.harness.get_or_insert(HarnessLaunch {
                port: crate::harness::DEFAULT_HARNESS_PORT,
                real_storage: false,
            });
        } else if let Some(port_text) = arg.strip_prefix("--harness=") {
            match port_text.parse::<u16>() {
                Ok(port) => {
                    let launch = options.harness.get_or_insert(HarnessLaunch {
                        port,
                        real_storage: false,
                    });
                    launch.port = port;
                }
                Err(_) => {
                    log::warn!("ignoring --harness with invalid port {port_text:?}");
                }
            }
        } else if arg == "--harness-real-storage" {
            real_storage_requested = true;
        } else if arg.starts_with("--") {
            log::warn!("ignoring unknown flag {arg:?}");
        } else if options.image_path.is_none() {
            options.image_path = Some(PathBuf::from(arg));
        } else {
            log::warn!("ignoring extra positional argument {arg:?}");
        }
    }

    match (&mut options.harness, real_storage_requested) {
        (Some(launch), true) => launch.real_storage = true,
        (None, true) => {
            log::warn!("--harness-real-storage has no effect without --harness");
        }
        _ => {}
    }

    options
}

static LAUNCH_OPTIONS: OnceLock<CliOptions> = OnceLock::new();

/// Publishes the parsed options. Only `main()` calls this; a second call is a
/// no-op so a misuse cannot panic the app at startup.
pub(crate) fn set_options(options: CliOptions) {
    let _ = LAUNCH_OPTIONS.set(options);
}

/// The optional image path to open at startup. Falls back to the legacy
/// direct-argv read when `main` has not published options (e.g. unit tests).
pub(crate) fn cli_image_path() -> Option<PathBuf> {
    if let Some(options) = LAUNCH_OPTIONS.get() {
        return options.image_path.clone();
    }
    std::env::args()
        .nth(1)
        .filter(|arg| !arg.starts_with("--"))
        .map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn no_args_yields_defaults() {
        let options = parse_cli_args(&[]);
        assert_eq!(options, CliOptions::default());
    }

    #[test]
    fn bare_path_is_image_path_without_harness() {
        let options = parse_cli_args(&args(&["photos/a.jpg"]));
        assert_eq!(options.image_path, Some(PathBuf::from("photos/a.jpg")));
        assert_eq!(options.harness, None);
    }

    #[test]
    fn harness_flag_uses_default_port() {
        let options = parse_cli_args(&args(&["--harness"]));
        assert_eq!(
            options.harness,
            Some(HarnessLaunch {
                port: crate::harness::DEFAULT_HARNESS_PORT,
                real_storage: false
            })
        );
    }

    #[test]
    fn harness_flag_accepts_explicit_port_and_zero() {
        let options = parse_cli_args(&args(&["--harness=9000"]));
        assert_eq!(options.harness.as_ref().map(|h| h.port), Some(9000));

        let options = parse_cli_args(&args(&["--harness=0"]));
        assert_eq!(options.harness.as_ref().map(|h| h.port), Some(0));
    }

    #[test]
    fn invalid_harness_port_is_ignored() {
        let options = parse_cli_args(&args(&["--harness=notaport"]));
        assert_eq!(options.harness, None);
    }

    #[test]
    fn real_storage_applies_only_with_harness() {
        let options = parse_cli_args(&args(&["--harness", "--harness-real-storage"]));
        assert_eq!(options.harness.as_ref().map(|h| h.real_storage), Some(true));

        let options = parse_cli_args(&args(&["--harness-real-storage"]));
        assert_eq!(options.harness, None);
    }

    #[test]
    fn harness_and_image_path_combine_in_any_order() {
        let options = parse_cli_args(&args(&["photos/a.jpg", "--harness=7000"]));
        assert_eq!(options.image_path, Some(PathBuf::from("photos/a.jpg")));
        assert_eq!(options.harness.as_ref().map(|h| h.port), Some(7000));

        let options = parse_cli_args(&args(&["--harness=7000", "photos/a.jpg"]));
        assert_eq!(options.image_path, Some(PathBuf::from("photos/a.jpg")));
        assert_eq!(options.harness.as_ref().map(|h| h.port), Some(7000));
    }

    #[test]
    fn unknown_flags_are_ignored_and_extra_positionals_dropped() {
        let options = parse_cli_args(&args(&["--wat", "a.jpg", "b.jpg"]));
        assert_eq!(options.image_path, Some(PathBuf::from("a.jpg")));
        assert_eq!(options.harness, None);
    }
}
