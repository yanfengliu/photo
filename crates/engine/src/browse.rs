//! Browsing: the current source, filter and sort, the photo list they
//! select, and the cached lists the Library's panels show.
//!
//! The list is re-read from the catalog after every change that could alter
//! which photos it holds or their order. A change that can do neither (a
//! rating, when nothing filters or sorts by rating) only re-reads the
//! summaries of the photos it touched, in place.

use std::cell::RefCell;
use std::collections::HashMap;

use photo_catalog::{
    Catalog, CatalogError, CollectionInfo, CollectionKind, Facet, FacetValue, Filter, FolderInfo,
    KeywordInfo, PhotoQuery, PhotoSummary, Sort, SortKey, Source, TextField,
};
use photo_core::meta::{CollectionId, PhotoId};

use crate::selection::Listing;

/// Photo counts for the Catalog panel's built-in sources.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct SourceCounts {
    /// Every photo in the catalog, virtual copies included.
    pub all_photos: u64,
    /// The photos in the quick collection.
    pub quick_collection: u64,
    /// The photos of the newest import that added any.
    pub previous_import: u64,
}

/// A per-photo attribute a change can touch.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Attribute {
    Rating,
    Flag,
    Label,
    Text(TextField),
    Keywords,
    Develop,
    Missing,
}

/// What a mutation changed, so that only what it could affect is re-read.
#[derive(Clone, Debug)]
pub(crate) enum Change {
    /// An attribute of these photos changed.
    Attribute(Attribute, Vec<PhotoId>),
    /// Photos were added to the catalog or removed from it.
    Photos,
    /// These photos went into or out of a collection.
    Membership(CollectionId, Vec<PhotoId>),
    /// Collections or keywords were created, renamed, moved or deleted.
    Organisation,
}

/// What to re-read.
#[derive(Clone, Debug, Default)]
pub(crate) struct Refresh {
    list: bool,
    in_place: Vec<PhotoId>,
    folders: bool,
    collections: bool,
    keywords: bool,
    counts: bool,
}

impl Refresh {
    /// Everything.
    pub(crate) fn all() -> Self {
        Self {
            list: true,
            in_place: Vec::new(),
            folders: true,
            collections: true,
            keywords: true,
            counts: true,
        }
    }

    /// Only the photo list.
    pub(crate) fn list() -> Self {
        Self {
            list: true,
            ..Self::default()
        }
    }
}

/// The browsing state and everything cached from it.
#[derive(Debug)]
pub(crate) struct Browser {
    quick_collection: CollectionId,
    query: PhotoQuery,
    photos: Vec<PhotoSummary>,
    index: HashMap<PhotoId, usize>,
    /// How many photos the source holds before the filter.
    source_total: u64,
    folders: Vec<FolderInfo>,
    collections: Vec<CollectionInfo>,
    keywords: Vec<KeywordInfo>,
    counts: SourceCounts,
    /// Facet values for the current list, computed when first asked for.
    facets: RefCell<HashMap<Facet, Vec<FacetValue>>>,
}

impl Listing for Browser {
    fn count(&self) -> usize {
        self.photos.len()
    }

    fn id_at(&self, index: usize) -> Option<PhotoId> {
        self.photos.get(index).map(|photo| photo.id)
    }

    fn index_of(&self, photo: PhotoId) -> Option<usize> {
        self.index.get(&photo).copied()
    }
}

impl Browser {
    /// Nothing loaded yet: all photos by capture time, with nothing cached.
    pub(crate) fn new(quick_collection: CollectionId) -> Self {
        Self {
            quick_collection,
            query: PhotoQuery::default(),
            photos: Vec::new(),
            index: HashMap::new(),
            source_total: 0,
            folders: Vec::new(),
            collections: Vec::new(),
            keywords: Vec::new(),
            counts: SourceCounts::default(),
            facets: RefCell::new(HashMap::new()),
        }
    }

    pub(crate) fn query(&self) -> &PhotoQuery {
        &self.query
    }

    pub(crate) fn photos(&self) -> &[PhotoSummary] {
        &self.photos
    }

    pub(crate) fn summary(&self, photo: PhotoId) -> Option<&PhotoSummary> {
        self.index_of(photo)
            .and_then(|index| self.photos.get(index))
    }

    pub(crate) fn folders(&self) -> &[FolderInfo] {
        &self.folders
    }

    pub(crate) fn collections(&self) -> &[CollectionInfo] {
        &self.collections
    }

    pub(crate) fn keywords(&self) -> &[KeywordInfo] {
        &self.keywords
    }

    pub(crate) fn counts(&self) -> SourceCounts {
        self.counts
    }

    pub(crate) fn source_total(&self) -> u64 {
        self.source_total
    }

    pub(crate) fn set_source(&mut self, source: Source) {
        self.query.source = source;
    }

    pub(crate) fn set_filter(&mut self, filter: Filter) {
        self.query.filter = filter;
    }

    pub(crate) fn set_sort(&mut self, sort: Sort) {
        self.query.sort = sort;
    }

    /// Add to `refresh` what `change` makes stale.
    pub(crate) fn plan(&self, change: &Change, refresh: &mut Refresh) {
        match change {
            Change::Attribute(attribute, photos) => {
                if self.list_depends_on(*attribute) {
                    refresh.list = true;
                } else {
                    refresh.in_place.extend_from_slice(photos);
                }
                if *attribute != Attribute::Missing && self.has_smart_collections() {
                    refresh.collections = true;
                }
                if *attribute == Attribute::Keywords {
                    refresh.keywords = true;
                }
            }
            Change::Photos => *refresh = Refresh::all(),
            Change::Membership(collection, photos) => {
                refresh.collections = true;
                let quick = *collection == self.quick_collection;
                refresh.counts |= quick;
                if matches!(
                    self.query.source,
                    Source::Collection(_) | Source::QuickCollection
                ) {
                    refresh.list = true;
                } else if quick {
                    refresh.in_place.extend_from_slice(photos);
                }
            }
            Change::Organisation => {
                refresh.list = true;
                refresh.collections = true;
                refresh.keywords = true;
            }
        }
    }

    fn has_smart_collections(&self) -> bool {
        self.collections
            .iter()
            .any(|collection| collection.kind == CollectionKind::Smart)
    }

    /// Whether a change of `attribute` can alter which photos the list
    /// holds or their order.
    fn list_depends_on(&self, attribute: Attribute) -> bool {
        let PhotoQuery {
            source,
            filter,
            sort,
        } = &self.query;
        let smart_source = match source {
            Source::Collection(id) => self
                .collections
                .iter()
                .find(|collection| collection.id == *id)
                .is_none_or(|collection| collection.kind == CollectionKind::Smart),
            _ => false,
        };
        let searched = !filter.text.trim().is_empty();
        let direct = match attribute {
            Attribute::Rating => filter.rating.is_some() || sort.key == SortKey::Rating,
            Attribute::Flag => !filter.flags.is_empty() || sort.key == SortKey::Pick,
            Attribute::Label => !filter.labels.is_empty() || sort.key == SortKey::Label,
            Attribute::Text(TextField::Title | TextField::Caption) => searched,
            Attribute::Text(TextField::Creator | TextField::Copyright) => false,
            Attribute::Keywords => searched || matches!(source, Source::Keyword(_)),
            Attribute::Develop => filter.edited.is_some() || sort.key == SortKey::EditTime,
            Attribute::Missing => false,
        };
        direct || (smart_source && attribute != Attribute::Missing)
    }

    /// Re-read what `refresh` asks for. Returns the list as it was, in
    /// order, when the list was re-queried, so the selection can be carried
    /// over.
    pub(crate) fn apply(
        &mut self,
        catalog: &Catalog,
        refresh: &Refresh,
    ) -> Result<Option<Vec<PhotoId>>, CatalogError> {
        if refresh.folders {
            self.folders = catalog.folders()?;
        }
        if refresh.collections {
            self.collections = catalog.collections()?;
        }
        if refresh.keywords {
            self.keywords = catalog.keywords()?;
        }
        if refresh.counts {
            self.counts = SourceCounts {
                all_photos: catalog.count(&Source::AllPhotos)?,
                quick_collection: catalog.count(&Source::QuickCollection)?,
                previous_import: catalog.count(&Source::PreviousImport)?,
            };
        }
        if refresh.list {
            return self.requery(catalog).map(Some);
        }
        if !refresh.in_place.is_empty() {
            self.update_in_place(catalog, &refresh.in_place)?;
        }
        Ok(None)
    }

    /// Run the query again, and count the source without the filter. A
    /// source that no longer exists (or a smart collection whose rules
    /// cannot be read) falls back to all photos.
    fn requery(&mut self, catalog: &Catalog) -> Result<Vec<PhotoId>, CatalogError> {
        let photos = match catalog.query(&self.query) {
            Ok(photos) => photos,
            Err(
                error @ (CatalogError::NotFound { .. }
                | CatalogError::Corrupt { .. }
                | CatalogError::InvalidRule(_)),
            ) if self.query.source != Source::AllPhotos => {
                log::warn!("showing all photos: the source cannot be listed ({error})");
                self.query.source = Source::AllPhotos;
                catalog.query(&self.query)?
            }
            Err(error) => return Err(error),
        };
        self.source_total = if self.query.filter == Filter::default() {
            photos.len() as u64
        } else {
            catalog.count(&self.query.source)?
        };
        let previous = self.photos.iter().map(|photo| photo.id).collect();
        self.index = photos
            .iter()
            .enumerate()
            .map(|(index, photo)| (photo.id, index))
            .collect();
        self.photos = photos;
        self.facets.borrow_mut().clear();
        Ok(previous)
    }

    /// Replace the summaries of `photos` with fresh ones, keeping the list's
    /// membership and order.
    fn update_in_place(
        &mut self,
        catalog: &Catalog,
        photos: &[PhotoId],
    ) -> Result<(), CatalogError> {
        let listed: Vec<PhotoId> = photos
            .iter()
            .copied()
            .filter(|photo| self.index.contains_key(photo))
            .collect();
        for summary in catalog.summaries(&listed)? {
            if let Some(index) = self.index.get(&summary.id) {
                self.photos[*index] = summary;
            }
        }
        Ok(())
    }

    /// The values of `facet` among the photos of the current source that
    /// pass every condition of the current filter except the one on this
    /// facet itself, so a column of the metadata filter keeps listing the
    /// values one could switch to.
    pub(crate) fn facet_values(
        &self,
        catalog: &Catalog,
        facet: Facet,
    ) -> Result<Vec<FacetValue>, CatalogError> {
        if let Some(values) = self.facets.borrow().get(&facet) {
            return Ok(values.clone());
        }
        let mut filter = self.query.filter.clone();
        match facet {
            Facet::Camera => filter.cameras.clear(),
            Facet::Lens => filter.lenses.clear(),
            Facet::Iso => filter.iso = None,
            Facet::CaptureDate => filter.capture = None,
        }
        let values = catalog.facet_values(&self.query.source, &filter, facet)?;
        self.facets.borrow_mut().insert(facet, values.clone());
        Ok(values)
    }
}
