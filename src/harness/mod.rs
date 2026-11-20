//! The agent harness: Photo driven over localhost by an agent the way a user
//! drives it, and measured while it runs (`AGENTS.md`, "Agent harness";
//! design §8.7; the guide is `docs/guides/agent-harness.md`).
//!
//! `photo --harness[=PORT]` starts the real [`photo_ui::PhotoApp`] inside
//! `photo_ui::harness`, headless, rendering real GPU frames offscreen, and
//! serves JSON lines on 127.0.0.1 (see [`protocol`]). Each launch keeps its
//! record in a run directory (see [`run`]). Every action goes through the
//! app's own input: clicks and drags on controls found by their accessible
//! names, key presses, typed text; reading goes through the engine's and
//! the app's read-only accessors and the accessibility tree.
//!
//! The pieces: [`server`] owns the socket and the run directory's records;
//! `driver` the running app; `observed` the wrapper that lets the harness
//! see each frame, and `tree` the accessibility tree it copies; `commands`
//! what each command does; [`stats`] the pixel statistics.

mod commands;
mod driver;
mod gpu;
mod keys;
mod observed;
pub mod protocol;
pub mod run;
pub mod server;
pub mod stats;
mod tree;

use std::path::PathBuf;

pub use commands::COMMANDS;
pub use driver::Look;
pub use run::{RunDir, SessionInfo};
pub use server::{Server, StopReason};

use crate::repo::Storage;

/// Everything a session needs to start.
pub struct HarnessConfig {
    /// The port to listen on; 0 picks a free one.
    pub port: u16,
    /// The run directory, already made.
    pub run: RunDir,
    /// Where the catalog, previews and exports live.
    pub storage: Storage,
    /// False when `storage` is the user's real one.
    pub sandboxed: bool,
    /// A photo to open at startup, as the bare path argument asks.
    pub open: Option<PathBuf>,
    /// The window size and pixel density.
    pub look: Look,
    /// Where relative photo and folder paths in requests are taken from.
    pub paths_base: PathBuf,
}
