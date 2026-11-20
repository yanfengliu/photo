//! The Photo application's own code: what the `photo` binary and the
//! `harnessctl` driver share. Everything with behaviour lives in the crates
//! under `crates/`; this package decides where data lives and starts things.

#![warn(missing_docs)]

pub mod repo;
