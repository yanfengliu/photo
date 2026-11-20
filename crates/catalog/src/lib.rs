//! The catalog: a SQLite database holding every photo record, its metadata,
//! organisation and parametric develop state. The only crate that speaks SQL.
//!
//! See `docs/work/12_lightroom-workflow-restart/design.md` §6.2.
//!
//! # What this crate guarantees
//!
//! - **The file is safe.** A catalog is opened with foreign keys enforced, a
//!   write-ahead log and commits that are flushed to the disk before the
//!   call returns. A file written by a newer schema, or one that is not a
//!   Photo catalog at all, is refused with a typed error before a single
//!   byte of it is changed. A catalog whose last writer went down mid-write
//!   is recovered to its last completed change when it is opened. A schema
//!   upgrade happens completely or not at all. The price is one disk flush
//!   per mutating call, so write what belongs together in one call (every
//!   setter takes a batch) and not once per item or per slider movement.
//! - **Mutations are atomic.** Every `&mut self` method runs in one
//!   transaction: it either happens completely or, on `Err`, not at all.
//! - **Nothing dangles.** Removing a photo, collection or keyword removes
//!   every row that depended on it.
//! - **User text is never SQL.** Statements are assembled from compile-time
//!   fragments chosen by enums; every caller-supplied value is a bound
//!   parameter. Text matching is literal (no wildcards) and ignores letter
//!   case, for non-ASCII letters too.
//! - **Mutations can be undone.** Setters take one value per photo, develop
//!   writes return a [`DevelopChange`], membership changes report exactly
//!   what they added or removed, and removing photos or deleting keywords
//!   or collections returns what went ([`RemovedPhotos`],
//!   [`DeletedKeywords`], [`DeletedCollections`]) to be put back under the
//!   same ids, so a caller's undo stack can restore the previous state
//!   exactly. Undone in reverse order, every restore is exact; one that
//!   could only be partial (a photo's keyword is still deleted, say) is
//!   refused rather than dropping anything.
//! - **Unreadable data is never papered over.** Stored JSON this build
//!   cannot read (damaged, or from a newer version) is reported or passed
//!   over, and always left in the file as it is; it is never replaced by
//!   defaults.
//! - **No real data is touched by accident.** Nothing here computes a
//!   default location; every catalog is opened at a path the caller chose,
//!   or in memory.
//!
//! Time stamps in this API are Unix seconds. Paths must be valid UTF-8; they
//! are compared exactly as given, except that `/` and `\` are the same
//! separator in a Windows-shaped path: one that starts with a drive letter
//! or with `\\`. Hand in absolute paths as the host itself spells them (as
//! `std::path::absolute` returns them): a Windows root typed as
//! `//server/share` or `/photos` is not recognisably a Windows path here,
//! and the files under it would be filed under the wrong folders.

#![warn(missing_docs)]

mod catalog;
mod codec;
mod collections;
mod develop;
mod error;
mod folders;
mod identify;
mod imports;
mod keywords;
mod membership;
mod migrate;
mod paths;
mod photos;
mod presets;
mod query;
mod removal;
mod rows;
mod schema;
mod settings;
mod smart;
mod snapshots;
mod sql;
mod stats;
mod text;
mod tree;

pub use catalog::Catalog;
pub use collections::{CollectionInfo, CollectionKind, DeletedCollections};
pub use develop::{DevelopChange, DevelopEdit, HistoryStep};
pub use error::{CatalogError, Entity, Result};
pub use folders::FolderInfo;
pub use imports::ImportRun;
pub use keywords::{DeletedKeywords, KeywordInfo};
pub use membership::CollectionEntry;
pub use photos::{NewPhoto, PhotoRecord, TextField};
pub use presets::Preset;
pub use query::{
    Comparison, CopyFilter, DateRange, Facet, FacetValue, Filter, IsoRange, PhotoQuery,
    PhotoSummary, RatingFilter, Sort, SortDirection, SortKey, Source,
};
pub use removal::RemovedPhotos;
pub use smart::{SmartField, SmartOp, SmartRule, SmartRules, SmartValue, SmartValueKind};
pub use snapshots::Snapshot;
pub use stats::CatalogStats;
