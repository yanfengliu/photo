//! The Photo application's own code: what the `photo` binary and the
//! `harnessctl` driver share. Everything with behaviour lives in the crates
//! under `crates/`; this package decides where data lives and starts things,
//! including the agent harness that drives the app from outside.

#![warn(missing_docs)]

pub mod harness;
pub mod harnessctl;
pub mod launch;
pub mod repo;
