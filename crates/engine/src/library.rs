//! What the Library shows and how the user moves through it: the source,
//! filter and sort; the photo list; the panel lists; the selection; the full
//! record of any photo. Every accessor here is a cheap read of state the
//! engine already holds, unless it says otherwise.

use photo_catalog::{
    CatalogError, CollectionInfo, Entity, Facet, FacetValue, Filter, FolderInfo, HistoryStep,
    KeywordInfo, PhotoRecord, PhotoSummary, Sort, Source,
};
use photo_core::develop::DevelopSettings;
use photo_core::meta::PhotoId;
use photo_imaging::Geometry;

use crate::browse::{Browser, Refresh, SourceCounts};
use crate::engine::Engine;
use crate::error::EngineError;
use crate::selection::{Listing, Movement, Selection};

/// What keeping one displayed size costs, in bytes, key included.
const SHAPE_COST: usize = 32;

impl Engine {
    /// Where the list looks.
    pub fn source(&self) -> &Source {
        &self.browser.query().source
    }

    /// Which of the source's photos the list keeps.
    pub fn filter(&self) -> &Filter {
        &self.browser.query().filter
    }

    /// The list's order.
    pub fn sort(&self) -> Sort {
        self.browser.query().sort
    }

    /// Show another source. A folder, collection or keyword that does not
    /// exist is refused and the list stays as it was.
    pub fn set_source(&mut self, source: Source) -> Result<(), EngineError> {
        if let Some((entity, id)) = self.unknown(&source) {
            return Err(CatalogError::NotFound { entity, id }.into());
        }
        self.requery(|browser| browser.set_source(source))
    }

    /// Filter the list.
    pub fn set_filter(&mut self, filter: Filter) -> Result<(), EngineError> {
        self.requery(|browser| browser.set_filter(filter))
    }

    /// Order the list.
    pub fn set_sort(&mut self, sort: Sort) -> Result<(), EngineError> {
        self.requery(|browser| browser.set_sort(sort))
    }

    /// The record a source refers to that the catalog does not hold.
    fn unknown(&self, source: &Source) -> Option<(Entity, i64)> {
        match *source {
            Source::AllPhotos | Source::QuickCollection | Source::PreviousImport => None,
            Source::Folder { id, .. } => (!self.folders().iter().any(|folder| folder.id == id))
                .then_some((Entity::Folder, id.0)),
            Source::Collection(id) => (id != self.quick_collection()
                && !self
                    .collections()
                    .iter()
                    .any(|collection| collection.id == id))
            .then_some((Entity::Collection, id.0)),
            Source::Keyword(id) => (!self.keywords().iter().any(|keyword| keyword.id == id))
                .then_some((Entity::Keyword, id.0)),
        }
    }

    /// Change the query and re-read the list; on failure put the query
    /// back as it was.
    fn requery(&mut self, change: impl FnOnce(&mut Browser)) -> Result<(), EngineError> {
        let before = self.browser.query().clone();
        change(&mut self.browser);
        if let Err(error) = self.refresh(&Refresh::list()) {
            self.browser.set_source(before.source);
            self.browser.set_filter(before.filter);
            self.browser.set_sort(before.sort);
            return Err(error);
        }
        Ok(())
    }

    /// The photos the source, filter and sort select, in order.
    pub fn photos(&self) -> &[PhotoSummary] {
        self.browser.photos()
    }

    /// Where a photo is in [`Engine::photos`]. Constant time.
    pub fn index_of(&self, photo: PhotoId) -> Option<usize> {
        self.browser.index_of(photo)
    }

    /// The summary of a photo in the list.
    pub fn summary(&self, photo: PhotoId) -> Option<&PhotoSummary> {
        self.browser.summary(photo)
    }

    /// The full-resolution size a photo shows at, as turned and cropped by
    /// its develop settings: what its thumbnail's shape will be, so that a
    /// grid cell's placeholder keeps that shape before the thumbnail
    /// arrives. Worked out from the catalog without decoding anything, and
    /// kept until the photo's develop version changes. A photo whose
    /// settings cannot be read shows upright and uncropped. `None` for a
    /// photo the catalog does not hold or whose size it does not know.
    pub fn displayed_size(&mut self, photo: PhotoId) -> Option<(u32, u32)> {
        let (version, width, height) = match self.browser.summary(photo) {
            Some(summary) => (summary.develop_version, summary.width, summary.height),
            None => {
                let record = self.catalog.photo(photo).ok()?;
                let metadata = &record.metadata;
                (record.develop_version, metadata.width, metadata.height)
            }
        };
        if width == 0 || height == 0 {
            return None;
        }
        if let Some((kept, size)) = self.shapes.get(&photo)
            && *kept == version
        {
            return Some(*size);
        }
        let size = match self.catalog.develop_settings(photo) {
            Ok(settings) => Geometry::from_settings(width, height, &settings).output_size(),
            Err(CatalogError::NewerFormat { .. }) => (width, height),
            Err(error) => {
                log::warn!("cannot work out how photo {} is shown: {error}", photo.0);
                return None;
            }
        };
        self.shapes.insert(photo, (version, size), SHAPE_COST);
        Some(size)
    }

    /// Every folder with its photo count, ordered by path.
    pub fn folders(&self) -> &[FolderInfo] {
        self.browser.folders()
    }

    /// Every collection, set and smart collection (not the quick
    /// collection) with its photo count, in name order.
    pub fn collections(&self) -> &[CollectionInfo] {
        self.browser.collections()
    }

    /// Every keyword with the number of photos it is assigned to directly,
    /// in name order.
    pub fn keywords(&self) -> &[KeywordInfo] {
        self.browser.keywords()
    }

    /// Photo counts of All Photographs, the Quick Collection and Previous
    /// Import.
    pub fn source_counts(&self) -> SourceCounts {
        self.browser.counts()
    }

    /// The values of a metadata facet among the photos of the current
    /// source that pass the filter (ignoring the filter's own condition on
    /// that facet), with counts. Computed on first use after each change of
    /// the list, then cached.
    pub fn facet_values(&self, facet: Facet) -> Result<Vec<FacetValue>, EngineError> {
        Ok(self.browser.facet_values(&self.catalog, facet)?)
    }

    /// Everything the catalog knows about a photo, whether or not it is in
    /// the list. Reads the catalog.
    pub fn photo(&self, photo: PhotoId) -> Result<PhotoRecord, EngineError> {
        Ok(self.catalog.photo(photo)?)
    }

    /// A photo's develop settings. Reads the catalog.
    pub fn develop_settings(&self, photo: PhotoId) -> Result<DevelopSettings, EngineError> {
        Ok(self.catalog.develop_settings(photo)?)
    }

    /// A photo's develop history, oldest first. Reads the catalog.
    pub fn history(&self, photo: PhotoId) -> Result<Vec<HistoryStep>, EngineError> {
        Ok(self.catalog.history(photo)?)
    }

    /// The selection.
    pub fn selection(&self) -> &Selection {
        &self.selection
    }

    /// The active photo, if any.
    pub fn active_photo(&self) -> Option<PhotoId> {
        self.selection.active()
    }

    /// Select only `photo`. Photos not in the list cannot be selected;
    /// returns whether the selection changed.
    pub fn select_only(&mut self, photo: PhotoId) -> bool {
        self.keeping_previous(|engine| engine.selection.select_only(&engine.browser, photo))
    }

    /// Add `photo` to the selection or take it out; see
    /// [`Selection`] for how the active photo follows.
    pub fn toggle_selected(&mut self, photo: PhotoId) -> bool {
        self.keeping_previous(|engine| engine.selection.toggle(&engine.browser, photo))
    }

    /// Make a photo that is already selected the active one, keeping the
    /// selection as it is; it becomes the range anchor too, as clicking it
    /// would make it. Returns false, changing nothing, when it is not
    /// selected.
    pub fn make_active(&mut self, photo: PhotoId) -> bool {
        self.keeping_previous(|engine| engine.selection.make_active(photo))
    }

    /// Select the range from the anchor to `photo`.
    pub fn extend_selection_to(&mut self, photo: PhotoId) -> bool {
        self.keeping_previous(|engine| engine.selection.extend_to(&engine.browser, photo))
    }

    /// Select every photo in the list.
    pub fn select_all(&mut self) {
        self.keeping_previous(|engine| engine.selection.select_all(&engine.browser));
    }

    /// Run a selection change, remembering the photo that was active before
    /// it when the active photo changes (for "paste from previous").
    fn keeping_previous<T>(&mut self, change: impl FnOnce(&mut Engine) -> T) -> T {
        let before = self.selection.active();
        let result = change(self);
        if before.is_some() && self.selection.active() != before {
            self.previous_active = before;
        }
        result
    }

    /// Select nothing.
    pub fn select_none(&mut self) {
        self.selection.clear();
    }

    /// Move the active photo; with `extend`, grow the range from the anchor
    /// instead of selecting the photo alone. Returns false when the list is
    /// empty.
    pub fn move_active(&mut self, movement: Movement, extend: bool) -> bool {
        self.keeping_previous(|engine| {
            engine
                .selection
                .move_active(&engine.browser, movement, extend)
        })
    }

    /// How many photos the list holds.
    pub fn photo_count(&self) -> usize {
        self.browser.count()
    }

    /// How many photos the current source holds before the filter (virtual
    /// copies included): the "of" in "12 of 340 photos". Kept up to date
    /// with the list.
    pub fn source_photo_count(&self) -> u64 {
        self.browser.source_total()
    }
}
