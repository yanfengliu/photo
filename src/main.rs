//! The `photo` binary: read the command line, decide where the data lives,
//! then open the app on it, in a window or behind the agent harness.

use std::path::PathBuf;
use std::process::ExitCode;

use photo::harness::{HarnessConfig, Look, RunDir, Server, StopReason, run::STORAGE_DIR};
use photo::launch::{self, Args};
use photo::repo::{self, Storage};
use photo_ui::AppOptions;

fn main() -> ExitCode {
    env_logger::init();
    let args = match Args::parse(std::env::args_os().skip(1)) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("{message}");
            return ExitCode::from(2);
        }
    };
    if let Some(path) = &args.open
        && let Err(error) = launch::openable(path)
    {
        eprintln!("Photo cannot open {}: {error}", path.display());
        return ExitCode::FAILURE;
    }
    let result = match args.harness {
        Some(port) => run_harness(port, args.real_storage, args.open),
        None => run_window(args.open),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

/// The app in a window, on the stores `repo` finds.
fn run_window(open: Option<PathBuf>) -> Result<(), String> {
    let storage = storage()?;
    let options = AppOptions {
        catalog_path: storage.catalog,
        previews_dir: storage.previews,
        export_dir: storage.exports,
    };
    let result = match open {
        None => photo_ui::run(options),
        Some(path) => launch::run_window_with(options, &path),
    };
    result.map_err(|error| format!("Photo stopped: {error}"))
}

/// The app behind the agent harness. Unless the user's real stores are
/// asked for, every store is pointed into the run directory before
/// anything resolves a path.
fn run_harness(port: u16, real_storage: bool, open: Option<PathBuf>) -> Result<(), String> {
    let runs = repo::harness_runs();
    let run = RunDir::create(&runs).map_err(|error| {
        format!(
            "Photo cannot make a run directory in {}: {error}",
            runs.dir.display()
        )
    })?;
    if !real_storage && !repo::set_runtime_storage_root(run.path().join(STORAGE_DIR)) {
        return Err(
            "the storage root was already chosen, so the session cannot be sandboxed".to_owned(),
        );
    }
    let storage = storage()?;
    let paths_base = repo::repo_root()
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_default();
    let config = HarnessConfig {
        port,
        run,
        storage,
        sandboxed: !real_storage,
        open,
        look: Look::default(),
        paths_base,
    };
    let server = Server::bind(config)?;
    let session = server.session();
    println!(
        "Photo harness listening on 127.0.0.1:{} (run directory {}; {} storage)",
        session.port,
        session.run_dir.display(),
        if session.sandboxed {
            "sandboxed"
        } else {
            "real"
        },
    );
    if !runs.in_checkout {
        println!(
            "Photo is not running from a checkout, so the run is kept in the system temporary directory."
        );
    }
    match server.run()? {
        StopReason::Quit => Ok(()),
    }
}

/// The stores under the storage root, with their folders made.
fn storage() -> Result<Storage, String> {
    let root = repo::storage_root().ok_or_else(|| {
        "Photo cannot find a place for its catalog: it is not running from a checkout \
         and this system has no Pictures folder."
            .to_owned()
    })?;
    let storage = Storage::under(&root);
    storage.create_dirs().map_err(|error| {
        format!(
            "Photo cannot make its catalog folder {}: {error}",
            storage.previews.display()
        )
    })?;
    Ok(storage)
}
