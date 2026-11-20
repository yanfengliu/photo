//! Finding photos: a source, a filter and a sort order in, grid cells out.
//!
//! Every query is one SQL statement, compiled from enums and bound values
//! (see `compile`), and each row carries everything a grid cell shows, so the
//! caller never has to follow up per photo.

mod compile;

use std::path::PathBuf;

use photo_core::meta::{
    CollectionId, ColorLabel, FileKind, Flag, FolderId, KeywordId, PhotoId, Rating,
};
use photo_core::time::DateTime;
use rusqlite::Row;

use crate::catalog::Catalog;
use crate::codec::{
    date_from_sql, flag_from_sql, kind_from_sql, label_from_sql, rating_from_sql, to_u32, to_u64,
};
use crate::error::Result;
use crate::sql::{Sql, id_list};

/// Where a query looks.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Source {
    /// Every photo in the catalog.
    #[default]
    AllPhotos,
    /// The photos whose file is in a folder, and optionally in any folder
    /// beneath it.
    Folder {
        /// The folder.
        id: FolderId,
        /// Whether to include the photos of every folder whose path lies
        /// inside this folder's path.
        include_subfolders: bool,
    },
    /// A collection: its members; for a smart collection the photos matching
    /// its rules; for a collection set the members of every regular
    /// collection inside it, at any depth (smart collections in the set do
    /// not contribute).
    Collection(CollectionId),
    /// The built-in quick collection.
    QuickCollection,
    /// The photos added by the newest import run that has photos in the
    /// catalog (a run that added nothing is passed over).
    PreviousImport,
    /// The photos carrying a keyword or any keyword beneath it.
    Keyword(KeywordId),
}

/// How a rating is compared.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Comparison {
    /// The rating or higher.
    AtLeast,
    /// The rating or lower.
    AtMost,
    /// Exactly the rating.
    Exactly,
}

/// A rating condition, such as "at least three stars".
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RatingFilter {
    /// How a photo's rating is compared with `rating`.
    pub comparison: Comparison,
    /// The rating to compare with.
    pub rating: Rating,
}

/// Whether to show only originals or only virtual copies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CopyFilter {
    /// Only photos that own their file.
    Masters,
    /// Only virtual copies.
    VirtualCopies,
}

/// An inclusive range of capture times; an absent end is open. Photos with
/// no capture time are never inside a range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct DateRange {
    /// The earliest capture time included.
    pub from: Option<DateTime>,
    /// The latest capture time included.
    pub to: Option<DateTime>,
}

/// An inclusive ISO range; an absent end is open. Photos with no recorded
/// ISO are never inside a range.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct IsoRange {
    /// The lowest ISO included.
    pub min: Option<u32>,
    /// The highest ISO included.
    pub max: Option<u32>,
}

/// Conditions a photo must meet. Every field that is set must hold; the
/// default filter has nothing set and matches every photo.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    /// Free text. Every white-space separated word must appear in the file
    /// name, title, caption, the name of a keyword, the camera or the lens
    /// (each word may be found in a different one). Matching ignores letter
    /// case and treats every character literally. Only the first
    /// [`Filter::MAX_WORDS`] distinct words are used.
    pub text: String,
    /// Any of these flag states; empty for no condition.
    pub flags: Vec<Flag>,
    /// A condition on the star rating.
    pub rating: Option<RatingFilter>,
    /// Any of these labels (`ColorLabel::None` for unlabelled); empty for no
    /// condition.
    pub labels: Vec<ColorLabel>,
    /// Only originals, or only virtual copies.
    pub copies: Option<CopyFilter>,
    /// Only RAW files, or only raster files.
    pub kind: Option<FileKind>,
    /// `Some(true)` for photos whose settings differ from the default for
    /// their kind, `Some(false)` for the rest.
    pub edited: Option<bool>,
    /// Any of these camera names, exactly as [`Facet::Camera`] lists them
    /// (the empty name selects photos with no camera recorded); empty for no
    /// condition.
    pub cameras: Vec<String>,
    /// Any of these lens names, as for `cameras`.
    pub lenses: Vec<String>,
    /// Only photos captured inside this range.
    pub capture: Option<DateRange>,
    /// Only photos whose ISO is inside this range.
    pub iso: Option<IsoRange>,
}

impl Filter {
    /// How many distinct words of [`Filter::text`] are used; further words
    /// are ignored. Each word is one more nested condition and SQLite limits
    /// how deep conditions may nest; far fewer words already pin a photo
    /// down.
    pub const MAX_WORDS: usize = 32;

    /// True when no condition is set, so every photo matches.
    pub fn is_empty(&self) -> bool {
        // Destructured without `..` so a new field cannot be forgotten here.
        let Self {
            text,
            flags,
            rating,
            labels,
            copies,
            kind,
            edited,
            cameras,
            lenses,
            capture,
            iso,
        } = self;
        text.trim().is_empty()
            && flags.is_empty()
            && rating.is_none()
            && labels.is_empty()
            && copies.is_none()
            && kind.is_none()
            && edited.is_none()
            && cameras.is_empty()
            && lenses.is_empty()
            && capture.is_none()
            && iso.is_none()
    }
}

/// What to order by.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SortKey {
    /// When the picture was taken; photos with no capture time sort lowest.
    #[default]
    CaptureTime,
    /// The order in which photos were added to the catalog.
    Added,
    /// File name in natural order (`IMG_2` before `IMG_10`), ignoring case.
    FileName,
    /// Star rating, unrated lowest.
    Rating,
    /// Rejected, then unflagged, then picked.
    Pick,
    /// Unlabelled, then red, yellow, green, blue, purple.
    Label,
    /// When the develop settings last changed; never-edited photos sort
    /// lowest.
    EditTime,
    /// File format (extension).
    FileType,
    /// The user's own order inside a collection. For any other source this
    /// is the same as [`SortKey::Added`].
    Custom,
}

/// Which way to order.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum SortDirection {
    /// Lowest first.
    #[default]
    Ascending,
    /// Highest first: the exact reverse of ascending.
    Descending,
}

/// A sort order. Photos that are equal under the key are ordered by capture
/// time, then file name, then the order they were added, so the result is
/// always fully determined; descending is the exact reverse of ascending.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct Sort {
    /// What to order by.
    pub key: SortKey,
    /// Which way.
    pub direction: SortDirection,
}

/// A complete query. The default is every photo, by capture time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PhotoQuery {
    /// Where to look.
    pub source: Source,
    /// Which of the source's photos to keep.
    pub filter: Filter,
    /// The order of the result.
    pub sort: Sort,
}

impl From<Source> for PhotoQuery {
    /// Everything in `source`, in the default order.
    fn from(source: Source) -> Self {
        Self {
            source,
            ..Self::default()
        }
    }
}

/// What a grid cell needs to draw one photo.
#[derive(Clone, Debug, PartialEq)]
pub struct PhotoSummary {
    /// The photo's id.
    pub id: PhotoId,
    /// The file's name, without its directory.
    pub file_name: String,
    /// Where the file is.
    pub path: PathBuf,
    /// Whether the file is a finished rendering or sensor data.
    pub kind: FileKind,
    /// Star rating.
    pub rating: Rating,
    /// Pick / reject flag.
    pub flag: Flag,
    /// Colour label.
    pub label: ColorLabel,
    /// Pixel width of the source file, upright, before any crop.
    pub width: u32,
    /// Pixel height of the source file, upright, before any crop.
    pub height: u32,
    /// When the picture was taken, if the file says.
    pub capture_time: Option<DateTime>,
    /// With `id`, identifies the rendering to show.
    pub develop_version: u64,
    /// True when the develop settings differ from the default for the kind.
    pub has_edits: bool,
    /// For a virtual copy, the photo that owns the file.
    pub master_id: Option<PhotoId>,
    /// For a virtual copy, its name.
    pub copy_name: Option<String>,
    /// True when the photo is in the quick collection.
    pub in_quick_collection: bool,
    /// True when the file was reported gone.
    pub missing: bool,
}

impl PhotoSummary {
    /// True when this is a virtual copy rather than the photo that owns the
    /// file.
    pub fn is_virtual_copy(&self) -> bool {
        self.master_id.is_some()
    }
}

/// A metadata dimension the Library filter can list values for.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Facet {
    /// The camera's name, as [`Filter::cameras`] takes it.
    Camera,
    /// The lens's name, as [`Filter::lenses`] takes it.
    Lens,
    /// The ISO setting, as a decimal number.
    Iso,
    /// The calendar day of capture, as `YYYY-MM-DD`.
    CaptureDate,
}

/// One distinct value of a facet and how many photos have it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FacetValue {
    /// The value as text; empty for photos where it is not recorded.
    pub value: String,
    /// How many photos in view have this value.
    pub count: u64,
}

/// The summary columns, in the order `summary_from_row` reads them. `?` is
/// the quick collection's id.
const SUMMARY_COLUMNS: &str = "SELECT p.id, p.file_name, p.path, p.kind, p.rating, p.flag, p.label,
        p.width, p.height, p.capture_time, p.develop_version, p.has_edits, p.master_id, p.copy_name, p.missing,
        EXISTS (SELECT 1 FROM collection_photos quick WHERE quick.collection_id = ";

fn summary_from_row(row: &Row<'_>) -> rusqlite::Result<PhotoSummary> {
    Ok(PhotoSummary {
        id: PhotoId(row.get(0)?),
        file_name: row.get(1)?,
        path: PathBuf::from(row.get::<_, String>(2)?),
        kind: kind_from_sql(row.get(3)?),
        rating: rating_from_sql(row.get(4)?),
        flag: flag_from_sql(row.get(5)?),
        label: label_from_sql(row.get(6)?),
        width: to_u32(row.get(7)?),
        height: to_u32(row.get(8)?),
        capture_time: date_from_sql(9, row.get(9)?)?,
        develop_version: to_u64(row.get(10)?),
        has_edits: row.get(11)?,
        master_id: row.get::<_, Option<i64>>(12)?.map(PhotoId),
        copy_name: row.get(13)?,
        missing: row.get(14)?,
        in_quick_collection: row.get(15)?,
    })
}

impl Catalog {
    fn summary_select(&self) -> Sql {
        let mut sql = Sql::new(SUMMARY_COLUMNS);
        sql.bind(self.quick_collection.0)
            .push(" AND quick.photo_id = p.id)");
        sql
    }

    fn run<T>(
        &self,
        sql: &Sql,
        map: impl FnMut(&Row<'_>) -> rusqlite::Result<T>,
    ) -> Result<Vec<T>> {
        let mut stmt = self.conn.prepare_cached(sql.text())?;
        let rows = stmt
            .query_map(sql.params(), map)?
            .collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    }

    /// The photos a query selects, in its order, each with everything a grid
    /// cell shows.
    ///
    /// Fails with a typed not-found error if the source refers to a folder,
    /// collection or keyword that does not exist.
    pub fn query(&self, query: &PhotoQuery) -> Result<Vec<PhotoSummary>> {
        let mut sql = self.summary_select();
        let custom_order = self.push_selection(&mut sql, &query.source, &query.filter)?;
        compile::push_order(&mut sql, query.sort, custom_order);
        self.run(&sql, summary_from_row)
    }

    /// Only the ids a query selects, in its order.
    pub fn query_ids(&self, query: &PhotoQuery) -> Result<Vec<PhotoId>> {
        let mut sql = Sql::new("SELECT p.id");
        let custom_order = self.push_selection(&mut sql, &query.source, &query.filter)?;
        compile::push_order(&mut sql, query.sort, custom_order);
        self.run(&sql, |row| row.get(0).map(PhotoId))
    }

    /// The grid cells for specific photos, one per id in the order given.
    /// Ids that are not in the catalog are skipped.
    pub fn summaries(&self, ids: &[PhotoId]) -> Result<Vec<PhotoSummary>> {
        let mut sql = self.summary_select();
        sql.push(" FROM json_each(")
            .bind(id_list(ids.iter().map(|id| id.0)));
        sql.push(") AS wanted JOIN photos p ON p.id = wanted.value ORDER BY wanted.key");
        self.run(&sql, summary_from_row)
    }

    /// How many photos a source holds; always the length of the unfiltered
    /// query on that source, so virtual copies count.
    pub fn count(&self, source: &Source) -> Result<u64> {
        self.count_matching(source, &Filter::default())
    }

    /// How many photos of a source pass a filter.
    pub fn count_matching(&self, source: &Source, filter: &Filter) -> Result<u64> {
        let mut sql = Sql::new("SELECT COUNT(*)");
        self.push_selection(&mut sql, source, filter)?;
        let count: i64 = self
            .conn
            .prepare_cached(sql.text())?
            .query_row(sql.params(), |row| row.get(0))?;
        Ok(to_u64(count))
    }

    /// The distinct values of a facet among the photos of a source that pass
    /// a filter, each with its photo count, in ascending order of value
    /// (numerically for ISO). Photos where the value is not recorded are
    /// grouped under the empty value, listed first.
    pub fn facet_values(
        &self,
        source: &Source,
        filter: &Filter,
        facet: Facet,
    ) -> Result<Vec<FacetValue>> {
        let (value, order) = compile::facet_columns(facet);
        let mut sql = Sql::new("SELECT ");
        sql.push(value).push(", COUNT(*)");
        self.push_selection(&mut sql, source, filter)?;
        sql.push(" GROUP BY ")
            .push(order)
            .push(" ORDER BY ")
            .push(order);
        self.run(&sql, |row| {
            Ok(FacetValue {
                value: row.get(0)?,
                count: to_u64(row.get(1)?),
            })
        })
    }
}
