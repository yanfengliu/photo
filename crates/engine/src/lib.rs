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
//! the seam in `previews/render.rs`: a develop-pipeline render of the
//! photo's settings. A RAW photo shows its embedded camera preview until its
//! render arrives.
//!
//! # Develop
//!
//! [`Engine::open_develop`] opens one photo for editing as a
//! [`DevelopSession`]: explicit load states, a dedicated render worker that
//! always works on the newest request, draft renders while a control moves
//! and one undoable commit when it is let go ([`Engine::adjust`],
//! [`Engine::commit_edit`]). The same worker computes the red mask overlay
//! ([`Engine::show_mask_overlay`]), which follows a mask's shape as it is
//! dragged. Presets, snapshots, history, copy and paste, sync, reset and
//! Quick Develop write many photos as one undo entry.
//!
//! # Export
//!
//! [`Engine::start_export`] renders the target photos to files through
//! photo-export as a cancellable task, two at a time, and refuses any file
//! that would be the original of a photo in the catalog or would go into
//! the catalog's own folders. Named export presets live in the catalog.
//!
//! # Types from the crates below
//!
//! Everything the UI needs from the catalog, decoder and imaging crates is
//! re-exported here, so the UI depends on this crate and `photo-core` only.

#![warn(missing_docs)]

mod attributes;
mod browse;
mod clipboard;
mod collections;
mod commands;
mod develop;
mod engine;
mod error;
mod export;
mod import;
mod jobs;
mod keywording;
mod library;
mod looks;
mod lru;
mod previews;
mod quick;
mod selection;
mod sidecars;
mod sources;
mod stamp;
mod still;
mod tasks;
mod toasts;
mod undo;

pub use browse::SourceCounts;
pub use clipboard::SettingsClipboard;
pub use commands::Target;
pub use develop::{
    BeforeSettings, CanvasView, DevelopFrame, DevelopLoad, DevelopSession, MaskOverlay,
    RenderCounts,
};
pub use engine::{Engine, EngineConfig, POLL_LIMIT, PendingWork};
pub use error::EngineError;
pub use export::ExportedPhoto;
pub use import::{
    CopyLayout, DatePattern, FileIssue, ImportCandidate, ImportMode, ImportRequest, ImportScan,
    ImportSummary,
};
pub use keywording::KeywordUse;
pub use looks::{BUILT_IN_GROUP, BuiltInPreset, PresetInfo, PresetKey};
pub use previews::{LoadState, Preview, PreviewOrigin, PreviewSize, PreviewState};
pub use selection::{Movement, Selection};
pub use still::StillSettings;
pub use tasks::{TaskId, TaskInfo, TaskKind, TaskState};
pub use toasts::{Toast, ToastKind};

pub use photo_catalog::{
    CatalogError, CollectionInfo, CollectionKind, Comparison, CopyFilter, DateRange, Entity, Facet,
    FacetValue, Filter, FolderInfo, HistoryStep, IsoRange, KeywordInfo, PhotoRecord, PhotoSummary,
    RatingFilter, SmartField, SmartOp, SmartRule, SmartRules, SmartValue, SmartValueKind, Snapshot,
    Sort, SortDirection, SortKey, Source, TextField,
};
pub use photo_decode::{RASTER_EXTENSIONS, RAW_EXTENSIONS, file_kind, is_supported};
/// The tone-curve shapes the pipeline renders with: the curve editor plots
/// these, never a copy of their math. `MonotoneCubic` is a point curve
/// (Fritsch–Carlson), `ParametricShape` the region curve, `ToneCurves` the
/// whole composite of a set of settings.
pub use photo_color::tone::{MonotoneCubic, ParametricShape, ToneCurves};
pub use photo_develop::Quality;
pub use photo_export::{
    Anchor, BitDepth, ExistingFile, ExportOutcome, ExportSettings, ExtensionCase, Folder, Format,
    Location, MetadataMode, MetadataSettings, Naming, OutputSharpening, Resize, Sizing, Watermark,
};
pub use photo_imaging::{Geometry, Histogram, Plane, View};
