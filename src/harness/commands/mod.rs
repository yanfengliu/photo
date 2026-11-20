//! The commands. Each reads its parameters, acts on the running app the way
//! a user would (or reads what it shows), and answers with data or an
//! error. An action's answer means the gesture was performed; `wait_idle`
//! is how a client learns that the work it started has finished.

mod images;
mod import;
mod input;
mod observe;

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::driver::Driver;
use super::protocol::{CmdError, Code, Params};
use super::run::{RunDir, SessionInfo};

/// Every command, in the order the guide lists them.
pub const COMMANDS: &[&str] = &[
    "ping",
    "observe",
    "observe_library",
    "screenshot",
    "image_stats",
    "compare_images",
    "dump_render",
    "import_folder",
    "open",
    "click",
    "double_click",
    "key",
    "type_text",
    "drag",
    "hover",
    "set_slider",
    "reset_slider",
    "wait_idle",
    "quit",
];

/// What a command works with.
pub struct Session<'a> {
    /// The running app.
    pub driver: &'a mut Driver,
    /// The run directory, for artifacts and relative paths.
    pub run: &'a mut RunDir,
    /// How the session can be reached and what it stores where.
    pub info: &'a SessionInfo,
    /// Where relative paths of photos and folders are taken from.
    pub paths_base: &'a Path,
    /// The id of the request being answered, recorded with artifacts.
    pub request_id: &'a Value,
}

impl Session<'_> {
    /// A photo or folder path a request gave: relative ones are taken from
    /// the checkout (where `test_photos/` lives).
    pub fn photo_path(&self, path: &str) -> PathBuf {
        let given = Path::new(path);
        if given.is_absolute() {
            given.to_path_buf()
        } else {
            self.paths_base.join(given)
        }
    }
}

/// What a command asks of the session besides its answer.
pub enum Outcome {
    /// Answer and carry on.
    Answer(Value),
    /// Answer, then end the session.
    Quit(Value),
}

/// Run the command `cmd` with `params`.
pub fn run(cmd: &str, params: &Params<'_>, session: &mut Session<'_>) -> Result<Outcome, CmdError> {
    let data = match cmd {
        "ping" => observe::ping(params, session)?,
        "observe" => observe::observe(params, session)?,
        "observe_library" => observe::observe_library(params, session)?,
        "screenshot" => images::screenshot(params, session)?,
        "image_stats" => images::image_stats(params, session)?,
        "compare_images" => images::compare_images(params, session)?,
        "dump_render" => images::dump_render(params, session)?,
        "import_folder" => import::import_folder(params, session)?,
        "open" => import::open(params, session)?,
        "click" => input::click(params, session, false)?,
        "double_click" => input::click(params, session, true)?,
        "key" => input::key(params, session)?,
        "type_text" => input::type_text(params, session)?,
        "drag" => input::drag(params, session)?,
        "hover" => input::hover(params, session)?,
        "set_slider" => input::set_slider(params, session)?,
        "reset_slider" => input::reset_slider(params, session)?,
        "wait_idle" => input::wait_idle(params, session)?,
        "quit" => {
            params.only(&[])?;
            return Ok(Outcome::Quit(json!({
                "stopping": true,
                "manifest": session.run.path().join(super::run::MANIFEST_FILE),
            })));
        }
        other => {
            return Err(CmdError::new(
                Code::Unsupported,
                format!(
                    "there is no command `{other}`; the commands are {}",
                    COMMANDS.join(", ")
                ),
            ));
        }
    };
    Ok(Outcome::Answer(data))
}

/// The answer of an action: what was done, with the warnings settling gave.
fn performed(mut data: Value, warnings: Vec<String>) -> Value {
    if !warnings.is_empty() {
        data["warnings"] = json!(warnings);
    }
    data
}
