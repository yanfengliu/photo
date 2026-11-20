//! The `photo` binary: decide where the data lives, then open the app on it.

use std::process::ExitCode;

use photo::repo::{self, Storage};

fn main() -> ExitCode {
    env_logger::init();
    let Some(root) = repo::storage_root() else {
        eprintln!(
            "Photo cannot find a place for its catalog: it is not running from a checkout \
             and this system has no Pictures folder."
        );
        return ExitCode::FAILURE;
    };
    let storage = Storage::under(&root);
    if let Err(error) = storage.create_dirs() {
        eprintln!(
            "Photo cannot make its catalog folder {}: {error}",
            storage.previews.display()
        );
        return ExitCode::FAILURE;
    }
    let options = photo_ui::AppOptions {
        catalog_path: storage.catalog,
        previews_dir: storage.previews,
        export_dir: storage.exports,
    };
    match photo_ui::run(options) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("Photo stopped: {error}");
            ExitCode::FAILURE
        }
    }
}
