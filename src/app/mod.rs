//! Application state, messages, lifecycle, and shared state accessors.

mod update;
mod view;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crate::decode::ImageData;
use crate::nav::DirNav;
use crate::viewer::{zoom_at_cursor, ImageCanvas, ViewerEvent};
use crate::{collection, decode, edit, lens, nav, viewer};
use iced::widget::image::Handle as ImageHandle;
use iced::widget::{
    button, container, horizontal_space, pick_list, scrollable, shader, slider, text, text_input,
    MouseArea, Space,
};
#[allow(unused_imports)]
use iced::{
    event, keyboard, mouse, window, Alignment, Background, Border, Color, Element, Length, Point,
    Size, Subscription, Task, Theme,
};

use crate::detail_load::*;
use crate::library::*;
use crate::loading::*;
use crate::local_edits::*;
#[cfg(test)]
use crate::repo::*;
use crate::session_cache::*;
use crate::theme::*;
use crate::widgets::*;

pub(crate) const DEFAULT_CANVAS_SIZE: [f32; 2] = [1200.0, 780.0];
pub(crate) const DEFAULT_WINDOW_SIZE: Size = Size::new(1200.0, 800.0);
pub(crate) const COLLECTION_SIDEBAR_WIDTH: f32 = 180.0;
pub(crate) const COLLECTION_SIDEBAR_DIVIDER_WIDTH: f32 = 1.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tab {
    Library,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SliderKind {
    Exposure,
    Contrast,
    Highlights,
    Shadows,
    Whites,
    Blacks,
    Temperature,
    Tint,
    Vibrance,
    Saturation,
    Clarity,
    Dehaze,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CropAspect {
    Freeform,
    Square,
}

impl CropAspect {
    pub(crate) fn ratio(self) -> Option<f32> {
        match self {
            Self::Freeform => None,
            Self::Square => Some(1.0),
        }
    }
}

impl std::fmt::Display for CropAspect {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Freeform => write!(f, "Freeform"),
            Self::Square => write!(f, "Square"),
        }
    }
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum ContextMenuKind {
    LibraryPhoto { photo_path: PathBuf },
    CollectionPhoto { photo_path: PathBuf },
    SidebarCollection { collection_index: usize },
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) struct ContextMenu {
    position: [f32; 2],
    kind: ContextMenuKind,
}

#[allow(dead_code)]
pub(crate) struct DragState {
    photo_index: usize,
    start_pos: [f32; 2],
    current_pos: [f32; 2],
    active: bool,
}

pub(crate) struct SaveRequest {
    path: PathBuf,
    image: Arc<ImageData>,
    state: edit::EditState,
    lens: edit::LensCorrection,
}

pub(crate) struct App {
    tab: Tab,
    library: Vec<LibraryEntry>,
    library_indices_by_path: std::collections::HashMap<PathBuf, usize>,
    image: Option<Arc<ImageData>>,
    image_id: u64,
    zoom: f32,
    offset: [f32; 2],
    window_size: Size,
    canvas_size_cache: Arc<Mutex<[f32; 2]>>,
    session_full_image_cache: SessionFullImageCache,
    nav: Option<DirNav>,
    library_index: Option<usize>,
    detail_load: DetailLoadState,
    error: Option<String>,
    edit_histories: std::collections::HashMap<PathBuf, edit::UndoHistory>,
    base_image_sources: std::collections::HashMap<PathBuf, BaseImageSource>,
    current_image_path: Option<PathBuf>,
    current_image_source_dimensions: Option<(u32, u32)>,
    lens_db: lens::LensDatabase,
    current_lens_profile: Option<lens::LensProfile>,
    current_exif: Option<lens::ExifInfo>,
    save_status: Option<String>,
    crop_mode: bool,
    crop_aspect: CropAspect,
    editing_slider: Option<SliderKind>,
    slider_text_buf: String,
    last_thumb_click: Option<(usize, Instant)>,
    last_slider_release: Option<(SliderKind, Instant)>,
    /// Tracks slider drag vs. single click: (which slider, event count).
    /// Only apply values after 2+ on_change events (i.e., actual drag).
    slider_drag: Option<(SliderKind, u32)>,
    lens_override_name: Option<String>,
    collection_store: collection::CollectionStore,
    active_collection: Option<usize>,
    context_menu: Option<ContextMenu>,
    drag_state: Option<DragState>,
    editing_collection_name: Option<usize>,
    collection_name_buf: String,
    #[allow(dead_code)]
    hovered_thumbnail: Option<usize>,
    sidebar_hover_collection: Option<usize>,
    cursor_position: [f32; 2],
    last_collection_click: Option<(usize, Instant)>,
    /// When entering Detail from a collection, stores (collection_index, photo_index_within_collection).
    collection_nav: Option<(usize, usize)>,
    pending_import_cache_warm_paths: std::collections::VecDeque<PathBuf>,
    import_cache_warm_in_flight: Option<PathBuf>,
    pending_local_edit_persist_requests: std::collections::VecDeque<LocalEditPersistRequest>,
    local_edit_persist_in_flight: Option<LocalEditPersistRequest>,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
pub(crate) enum Message {
    OpenFile,
    FileSelected(Option<PathBuf>),
    ImagePreviewLoaded {
        request_id: u64,
        path: PathBuf,
        result: Result<Option<Arc<ImageData>>, String>,
    },
    ImageLoaded {
        request_id: u64,
        result: Result<LoadedFullImage, String>,
    },
    ExifLoaded {
        request_id: u64,
        exif: Option<lens::ExifInfo>,
    },
    Viewer(ViewerEvent),
    Event(iced::Event),
    SwitchTab(Tab),
    AddFolder,
    AddFiles,
    FolderPicked(Option<PathBuf>),
    FilesPicked(Option<Vec<PathBuf>>),
    ThumbnailLoaded(PathBuf, Result<Arc<ImageData>, String>),
    ImportCacheWarmCompleted {
        path: PathBuf,
        result: Result<bool, String>,
    },
    LocalEditPersistCompleted {
        path: PathBuf,
        request_id: u64,
        result: Result<Option<Arc<ImageData>>, String>,
    },
    LibraryItemClicked(usize),
    SliderChanged(SliderKind, f32),
    SliderReleased(SliderKind),
    ResetSlider(SliderKind),
    ResetAll,
    SaveEdited,
    SaveCompleted(Result<String, String>),
    ToggleCropMode,
    CropAspectSelected(CropAspect),
    ClearCrop,
    ToggleLensCorrection,
    RotateClockwise,
    RotateCounterclockwise,
    SliderTextInput(SliderKind),
    SliderTextChanged(String),
    SliderTextSubmit(SliderKind),
    LensProfileSelected(String),
    // Collections
    CreateCollection,
    CollectionNameChanged(String),
    CollectionNameSubmit,
    CollectionNameCancel,
    SidebarCollectionClicked(usize),
    SidebarCollectionRightClicked(usize),
    SidebarCollectionHovered(Option<usize>),
    ExitCollectionView,
    CollectionPhotoClicked(usize),
    CollectionPhotoRightClicked(usize),
    // Context menu
    DismissContextMenu,
    ContextMenuRename,
    ContextMenuDelete,
    AddPhotoToCollection(usize),
    RemovePhotoFromCollection,
    // Thumbnail hover
    ThumbnailHovered(Option<usize>),
    // Right-click on library thumbnail
    LibraryPhotoRightClicked(usize),
    // Toggle photo in collection
    TogglePhotoInCollection(usize),
    // Back from detail to collection grid
    ExitCollectionDetail,
}

pub(crate) fn path_filename_str(path: &Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

impl App {
    pub(crate) fn new() -> (Self, Task<Message>) {
        let canvas_size_cache = Arc::new(Mutex::new(DEFAULT_CANVAS_SIZE));
        let mut app = App {
            tab: Tab::Library,
            library: Vec::new(),
            library_indices_by_path: std::collections::HashMap::new(),
            image: None,
            image_id: 0,
            zoom: 1.0,
            offset: [0.0, 0.0],
            window_size: DEFAULT_WINDOW_SIZE,
            canvas_size_cache,
            session_full_image_cache: SessionFullImageCache::default(),
            nav: None,
            library_index: None,
            detail_load: DetailLoadState::default(),
            error: None,
            edit_histories: std::collections::HashMap::new(),
            base_image_sources: std::collections::HashMap::new(),
            current_image_path: None,
            current_image_source_dimensions: None,
            lens_db: lens::LensDatabase::load_bundled(),
            current_lens_profile: None,
            current_exif: None,
            save_status: None,
            crop_mode: false,
            crop_aspect: CropAspect::Freeform,
            editing_slider: None,
            slider_text_buf: String::new(),
            last_thumb_click: None,
            last_slider_release: None,
            slider_drag: None,
            lens_override_name: None,
            collection_store: collection::CollectionStore::load(),
            active_collection: None,
            context_menu: None,
            drag_state: None,
            editing_collection_name: None,
            collection_name_buf: String::new(),
            hovered_thumbnail: None,
            sidebar_hover_collection: None,
            cursor_position: [0.0, 0.0],
            last_collection_click: None,
            collection_nav: None,
            pending_import_cache_warm_paths: std::collections::VecDeque::new(),
            import_cache_warm_in_flight: None,
            pending_local_edit_persist_requests: std::collections::VecDeque::new(),
            local_edit_persist_in_flight: None,
        };

        // Restore saved library entries
        let saved_paths = load_library();
        app.add_library_entries(&saved_paths);
        let thumb_task = Self::load_thumbnails(&saved_paths);

        let args: Vec<String> = std::env::args().collect();
        let cli_task = if args.len() > 1 {
            let path = PathBuf::from(&args[1]);
            if path.exists() {
                app.tab = Tab::Detail;
                app.nav = Some(DirNav::new(&path));
                app.start_load(path)
            } else {
                Task::none()
            }
        } else {
            Task::none()
        };

        (app, Task::batch([thumb_task, cli_task]))
    }

    pub(crate) fn title(&self) -> String {
        match self.tab {
            Tab::Library => {
                if self.library.is_empty() {
                    "Photo - Library".to_string()
                } else {
                    format!("Photo - Library ({})", self.library.len())
                }
            }
            Tab::Detail => {
                if let Some(idx) = self
                    .library_index
                    .and_then(|idx| self.clamped_library_index(idx))
                {
                    if let Some(entry) = self.library.get(idx) {
                        return format!("Photo - {}", entry.filename);
                    }
                }
                match &self.nav {
                    Some(nav) if !nav.current_filename().is_empty() => {
                        format!("Photo - {}", path_filename_str(&nav.current_path()))
                    }
                    _ => "Photo".to_string(),
                }
            }
        }
    }

    pub(crate) fn theme(&self) -> Theme {
        Theme::Dark
    }

    pub(crate) fn subscription(&self) -> Subscription<Message> {
        event::listen().map(Message::Event)
    }

    // ---------------------------------------------------------------------------
    // Update
    // ---------------------------------------------------------------------------

    pub(crate) fn library_entry_by_path(&self, path: &Path) -> Option<&LibraryEntry> {
        self.library_indices_by_path
            .get(path)
            .and_then(|&index| self.library.get(index))
    }

    pub(crate) fn clamped_library_index(&self, index: usize) -> Option<usize> {
        if self.library.is_empty() {
            None
        } else {
            Some(index.min(self.library.len() - 1))
        }
    }

    pub(crate) fn clamped_collection_photo_index(
        &self,
        collection_index: usize,
        photo_index: usize,
    ) -> Option<usize> {
        let collection = self.collection_store.collections.get(collection_index)?;
        if collection.photos.is_empty() {
            None
        } else {
            Some(photo_index.min(collection.photos.len() - 1))
        }
    }

    pub(crate) fn step_wrapped_index(current: usize, len: usize, forward: bool) -> usize {
        if forward {
            (current + 1) % len
        } else if current == 0 {
            len - 1
        } else {
            current - 1
        }
    }

    pub(crate) fn library_photo_context_menu_actions(
        &self,
        photo_path: &Path,
    ) -> Vec<(String, Message)> {
        // Detail navigation clamps stale positions, while context-menu actions fail closed if the
        // original photo disappears before the click is handled.
        if self.library_entry_by_path(photo_path).is_none() {
            return Vec::new();
        }

        self.collection_store
            .collections
            .iter()
            .enumerate()
            .map(|(i, col)| {
                if col.photos.iter().any(|existing| existing == photo_path) {
                    (
                        format!("\u{2713} {}", col.name),
                        Message::TogglePhotoInCollection(i),
                    )
                } else {
                    (
                        format!("Add to {}", col.name),
                        Message::AddPhotoToCollection(i),
                    )
                }
            })
            .collect()
    }

    pub(crate) fn library_grid_layout(&self) -> ThumbnailGridLayout {
        let grid_width =
            self.window_size.width - COLLECTION_SIDEBAR_WIDTH - COLLECTION_SIDEBAR_DIVIDER_WIDTH;
        ThumbnailGridLayout::new(grid_width)
    }

    pub(crate) fn collection_grid_layout(&self) -> ThumbnailGridLayout {
        ThumbnailGridLayout::new(self.window_size.width)
    }

    pub(crate) fn current_rotation(&self) -> edit::QuarterTurns {
        self.current_image_path
            .as_ref()
            .and_then(|path| self.edit_histories.get(path))
            .map(|history| history.current.rotation)
            .unwrap_or_default()
    }

    pub(crate) fn current_crop(&self) -> Option<edit::CropRect> {
        self.current_image_path
            .as_ref()
            .and_then(|path| self.edit_histories.get(path))
            .and_then(|history| history.current.crop)
    }

    pub(crate) fn visible_edit_state(&self) -> edit::EditState {
        let mut state = self
            .current_image_path
            .as_ref()
            .and_then(|path| self.edit_histories.get(path))
            .map(|history| history.current)
            .unwrap_or_default();
        state.crop = self.visible_crop();
        state
    }

    pub(crate) fn current_save_request(&self) -> Option<SaveRequest> {
        if self.detail_load.blocks_save() {
            return None;
        }
        let path = self.current_image_path.clone()?;
        let image = self.image.clone()?;
        let state = self.visible_edit_state();
        if self.current_render_depends_on_pending_auto_lens_metadata(state) {
            return None;
        }
        let lens = self.current_lens_correction(state.lens_correction);
        Some(SaveRequest {
            path,
            image,
            state,
            lens,
        })
    }

    pub(crate) fn current_lens_vignetting(&self, lens_correction_enabled: bool) -> [f32; 3] {
        if !lens_correction_enabled {
            return [0.0; 3];
        }
        self.current_lens_profile
            .as_ref()
            .and_then(|profile| profile.vignetting)
            .map(|vignetting| [vignetting.k1, vignetting.k2, vignetting.k3])
            .unwrap_or([0.0; 3])
    }

    pub(crate) fn current_lens_correction(
        &self,
        lens_correction_enabled: bool,
    ) -> edit::LensCorrection {
        if !lens_correction_enabled {
            return edit::LensCorrection::default();
        }
        let dist = self
            .current_lens_profile
            .as_ref()
            .and_then(|profile| profile.distortion)
            .map(|d| [d.a, d.b, d.c])
            .unwrap_or([0.0; 3]);
        let tca = self
            .current_lens_profile
            .as_ref()
            .and_then(|profile| profile.tca);
        edit::LensCorrection {
            dist,
            vig: self.current_lens_vignetting(true),
            tca_r: tca.map(|t| t.vr).unwrap_or(1.0),
            tca_b: tca.map(|t| t.vb).unwrap_or(1.0),
        }
    }

    pub(crate) fn visible_crop(&self) -> Option<edit::CropRect> {
        if self.crop_mode {
            None
        } else {
            self.current_crop()
        }
    }

    pub(crate) fn current_display_dimensions(&self, img: &decode::ImageData) -> (u32, u32) {
        let base_dimensions = self
            .current_image_source_dimensions
            .unwrap_or((img.width, img.height));
        display_dimensions_for_edit_state(
            base_dimensions,
            self.current_rotation(),
            self.visible_crop(),
        )
    }

    pub(crate) fn current_canvas_size(&self) -> [f32; 2] {
        self.canvas_size_cache
            .lock()
            .map(|canvas_size| *canvas_size)
            .unwrap_or(DEFAULT_CANVAS_SIZE)
    }

    pub(crate) fn update_canvas_size(&mut self, canvas_size: [f32; 2]) {
        if let Ok(mut cached_size) = self.canvas_size_cache.lock() {
            *cached_size = canvas_size;
        }
    }

    pub(crate) fn fit_scale_for_rotation_and_crop(
        &self,
        canvas_size: [f32; 2],
        img: &decode::ImageData,
        rotation: edit::QuarterTurns,
        crop: Option<edit::CropRect>,
    ) -> f32 {
        let (rotated_w, rotated_h) = edit::rotated_dimensions(img.width, img.height, rotation);
        let snapped_crop = crop.map(|crop| crop.snap_to_pixels(rotated_w, rotated_h));
        let (display_w, display_h) = edit::cropped_dimensions(rotated_w, rotated_h, snapped_crop);
        (canvas_size[0] / display_w as f32).min(canvas_size[1] / display_h as f32)
    }

    pub(crate) fn actual_size_zoom_for_rotation(
        &self,
        canvas_size: [f32; 2],
        img: &decode::ImageData,
        rotation: edit::QuarterTurns,
    ) -> f32 {
        self.actual_size_zoom_for_rotation_and_crop(canvas_size, img, rotation, self.visible_crop())
    }

    pub(crate) fn actual_size_zoom_for_rotation_and_crop(
        &self,
        canvas_size: [f32; 2],
        img: &decode::ImageData,
        rotation: edit::QuarterTurns,
        crop: Option<edit::CropRect>,
    ) -> f32 {
        1.0 / self.fit_scale_for_rotation_and_crop(canvas_size, img, rotation, crop)
    }

    pub(crate) fn is_at_actual_size_for_rotation_and_crop(
        &self,
        canvas_size: [f32; 2],
        img: &decode::ImageData,
        rotation: edit::QuarterTurns,
        crop: Option<edit::CropRect>,
    ) -> bool {
        (self.zoom - self.actual_size_zoom_for_rotation_and_crop(canvas_size, img, rotation, crop))
            .abs()
            < 0.01
    }

    pub(crate) fn preserve_actual_size_after_display_change(
        &mut self,
        previous_rotation: edit::QuarterTurns,
        previous_crop: Option<edit::CropRect>,
    ) {
        let Some(img) = &self.image else {
            return;
        };
        let canvas_size = self.current_canvas_size();
        if !self.is_at_actual_size_for_rotation_and_crop(
            canvas_size,
            img,
            previous_rotation,
            previous_crop,
        ) {
            return;
        }

        let current_rotation = self.current_rotation();
        let current_crop = self.visible_crop();
        if current_rotation == previous_rotation && current_crop == previous_crop {
            return;
        }

        self.zoom = self.actual_size_zoom_for_rotation_and_crop(
            canvas_size,
            img,
            current_rotation,
            current_crop,
        );
    }
}

pub(crate) fn set_slider_field(state: &mut edit::EditState, kind: SliderKind, value: f32) {
    match kind {
        SliderKind::Exposure => state.exposure = value,
        SliderKind::Contrast => state.contrast = value,
        SliderKind::Highlights => state.highlights = value,
        SliderKind::Shadows => state.shadows = value,
        SliderKind::Whites => state.whites = value,
        SliderKind::Blacks => state.blacks = value,
        SliderKind::Temperature => state.temperature = value,
        SliderKind::Tint => state.tint = value,
        SliderKind::Vibrance => state.vibrance = value,
        SliderKind::Saturation => state.saturation = value,
        SliderKind::Clarity => state.clarity = value,
        SliderKind::Dehaze => state.dehaze = value,
    }
}

pub(crate) fn get_slider_field(state: &edit::EditState, kind: SliderKind) -> f32 {
    match kind {
        SliderKind::Exposure => state.exposure,
        SliderKind::Contrast => state.contrast,
        SliderKind::Highlights => state.highlights,
        SliderKind::Shadows => state.shadows,
        SliderKind::Whites => state.whites,
        SliderKind::Blacks => state.blacks,
        SliderKind::Temperature => state.temperature,
        SliderKind::Tint => state.tint,
        SliderKind::Vibrance => state.vibrance,
        SliderKind::Saturation => state.saturation,
        SliderKind::Clarity => state.clarity,
        SliderKind::Dehaze => state.dehaze,
    }
}

pub(crate) fn slider_range(kind: SliderKind) -> (f32, f32) {
    // Lightroom Basic-panel conventions: Exposure is ±5 EV, every other
    // slider runs -100..+100. The edit::*_amount mappings convert these UI
    // units into the internal math amounts.
    match kind {
        SliderKind::Exposure => (-5.0, 5.0),
        _ => (-100.0, 100.0),
    }
}

pub(crate) fn slider_step(kind: SliderKind) -> f32 {
    match kind {
        SliderKind::Exposure => 0.01,
        _ => 1.0,
    }
}

#[cfg(test)]
mod tests;
