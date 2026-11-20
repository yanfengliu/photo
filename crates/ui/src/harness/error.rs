//! What can go wrong while driving or rendering an app.

use std::fmt;

use egui::accesskit::Role;

/// Why a harness action failed.
#[derive(Clone, Debug, PartialEq)]
pub enum HarnessError {
    /// No control with this role and name is on screen.
    NotFound {
        /// The role that was asked for.
        role: Role,
        /// The accessible name that was asked for.
        name: String,
    },
    /// More than one control has this role and name, so the request is
    /// ambiguous. Give the widgets distinguishing name prefixes.
    Ambiguous {
        /// The role that was asked for.
        role: Role,
        /// The accessible name that was asked for.
        name: String,
        /// How many controls matched.
        count: usize,
    },
    /// The node with this role and name has no rectangle on screen: it is a
    /// container (a dialog, the toast area), which can be counted but not
    /// described or clicked.
    NoBounds {
        /// The role that was asked for.
        role: Role,
        /// The accessible name that was asked for.
        name: String,
    },
    /// The UI kept asking for repaints and never came to rest.
    NotSettled(String),
    /// Interactive controls without an accessible name: neither a test nor
    /// assistive technology can reach them. Each entry describes one.
    UnnamedControls(Vec<String>),
}

impl fmt::Display for HarnessError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound { role, name } => write!(f, "no {role:?} named {name:?} is on screen"),
            Self::Ambiguous { role, name, count } => {
                write!(f, "{count} controls are {role:?} named {name:?}")
            }
            Self::NoBounds { role, name } => {
                write!(f, "the {role:?} named {name:?} has no rectangle on screen")
            }
            Self::NotSettled(detail) => write!(f, "the UI did not settle: {detail}"),
            Self::UnnamedControls(controls) => write!(
                f,
                "{} interactive control(s) have no accessible name: {}",
                controls.len(),
                controls.join("; ")
            ),
        }
    }
}

impl std::error::Error for HarnessError {}

/// Why a frame could not be rendered or saved.
#[derive(Clone, Debug, PartialEq)]
pub enum RenderError {
    /// This machine has no usable GPU adapter. Callers that only want a
    /// picture when one can be made treat this as "skip".
    NoAdapter(String),
    /// An adapter exists but rendering failed.
    Failed(String),
    /// The image could not be written.
    Save(String),
}

impl fmt::Display for RenderError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoAdapter(detail) => write!(f, "no GPU adapter to render with: {detail}"),
            Self::Failed(detail) => write!(f, "rendering failed: {detail}"),
            Self::Save(detail) => write!(f, "could not save the image: {detail}"),
        }
    }
}

impl std::error::Error for RenderError {}
