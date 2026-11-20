//! The application engine: everything the app does, with no UI toolkit in
//! sight. Owns the open catalog, background jobs, previews, undo and the
//! selection, and exposes them as plain method calls on one [`Engine`].
//!
//! See `docs/work/12_lightroom-workflow-restart/design.md`, §7.
//!
//! # How it runs
//!
//! The UI owns one [`Engine`] and calls it from one thread. The engine owns
//! the catalog on that thread; background work (scanning, importing,
//! previews, sidecars) runs on worker threads that never touch the catalog.
//! A worker's result reaches the engine's state, and the catalog, only in
//! [`Engine::poll`], which the UI calls once per frame. [`Engine::set_wake`]
//! lets the workers ask for that frame. [`Engine::is_idle`] and
//! [`Engine::wait_idle`] cover every piece of background work, which is
//! what tests and the automation harness wait on.
//!
//! # What can be undone
//!
//! Every change the user makes to the catalog goes through one path: it is
//! applied, recorded on the undo stack with a label, confirmed with a toast
//! ("Set Rating to 3"), and the lists it affects are re-read. Ratings,
//! flags, labels, text, keywords, orientation, virtual copies, removal,
//! collection membership, metadata read from sidecars, and creating,
//! renaming, moving and deleting collections and keywords all undo and redo
//! exactly, per photo and under the same ids. The undo stack lasts for the
//! session; it is never stored.
//!
//! # Previews
//!
//! See [`Engine::preview`]. Every preview's pixels come from one function,
//! the seam in `previews/render.rs`, which a develop-pipeline render
//! replaces without touching the caching, validation or scheduling around
//! it.
//!
//! # Types from the crates below
//!
//! Everything the UI needs from the catalog, decoder and imaging crates is
//! re-exported here, so the UI depends on this crate and `photo-core` only.

#![warn(missing_docs)]

mod attributes;
mod browse;
mod collections;
mod commands;
mod engine;
mod error;
mod import;
mod jobs;
mod keywording;
mod library;
mod previews;
mod selection;
mod sidecars;
mod tasks;
mod toasts;
mod undo;

pub use browse::SourceCounts;
pub use commands::Target;
pub use engine::{Engine, EngineConfig, POLL_LIMIT, PendingWork};
pub use error::EngineError;
pub use import::{
    CopyLayout, DatePattern, FileIssue, ImportCandidate, ImportMode, ImportRequest, ImportScan,
    ImportSummary,
};
pub use previews::{LoadState, Preview, PreviewOrigin, PreviewSize, PreviewState};
pub use selection::{Movement, Selection};
pub use tasks::{TaskId, TaskInfo, TaskKind, TaskState};
pub use toasts::{Toast, ToastKind};

pub use photo_catalog::{
    CatalogError, CollectionInfo, CollectionKind, Comparison, CopyFilter, DateRange, Entity, Facet,
    FacetValue, Filter, FolderInfo, HistoryStep, IsoRange, KeywordInfo, PhotoRecord, PhotoSummary,
    RatingFilter, SmartField, SmartOp, SmartRule, SmartRules, SmartValue, SmartValueKind, Sort,
    SortDirection, SortKey, Source, TextField,
};
pub use photo_decode::{RASTER_EXTENSIONS, RAW_EXTENSIONS, file_kind, is_supported};
pub use photo_imaging::Histogram;
