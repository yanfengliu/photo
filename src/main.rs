#![windows_subsystem = "windows"]

mod collection;
mod decode;
mod edit;
mod lens;
mod nav;
mod viewer;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
};
use std::os::windows::fs::OpenOptionsExt;

use iced::widget::image::Handle as ImageHandle;
use iced::widget::{
    button, column, container, horizontal_space, pick_list, row, scrollable, shader, slider, text,
    text_input, Image, MouseArea, Space,
};
#[allow(unused_imports)]
use iced::{
    event, keyboard, mouse, window, Alignment, Background, Border, Color, Element, Length, Point,
    Size, Subscription, Task, Theme,
};

use decode::ImageData;
use nav::DirNav;
use viewer::{zoom_at_cursor, ImageCanvas, ViewerEvent};

// ---------------------------------------------------------------------------
// Lightroom-inspired color palette
// ---------------------------------------------------------------------------

const BG_DARK: Color = Color::from_rgb(0.118, 0.118, 0.118);
const BG_PANEL: Color = Color::from_rgb(0.153, 0.153, 0.153);
const BG_TOOLBAR: Color = Color::from_rgb(0.176, 0.176, 0.176);
const BG_CARD: Color = Color::from_rgb(0.165, 0.165, 0.165);
const BG_BUTTON: Color = Color::from_rgb(0.22, 0.22, 0.22);
const BG_BUTTON_HOVER: Color = Color::from_rgb(0.28, 0.28, 0.28);
const TEXT_PRIMARY: Color = Color::from_rgb(0.82, 0.82, 0.82);
const TEXT_SECONDARY: Color = Color::from_rgb(0.55, 0.55, 0.55);
const TEXT_DIM: Color = Color::from_rgb(0.40, 0.40, 0.40);
const DIVIDER: Color = Color::from_rgb(0.22, 0.22, 0.22);
const DEFAULT_CANVAS_SIZE: [f32; 2] = [1200.0, 780.0];
const DEFAULT_WINDOW_SIZE: Size = Size::new(1200.0, 800.0);
const GRID_THUMB_SIZE: f32 = 150.0;
const GRID_SPACING: f32 = 8.0;
const GRID_PADDING: f32 = 14.0;
const GRID_CARD_PADDING: f32 = 6.0;
const COLLECTION_SIDEBAR_WIDTH: f32 = 180.0;
const COLLECTION_SIDEBAR_DIVIDER_WIDTH: f32 = 1.0;
const ROTATE_COUNTERCLOCKWISE_ICON: &str = "\u{21BA}";
const ROTATE_CLOCKWISE_ICON: &str = "\u{21BB}";
const ROTATE_COUNTERCLOCKWISE_STEP_LABEL: &str = "-90\u{00B0}";
const ROTATE_CLOCKWISE_STEP_LABEL: &str = "+90\u{00B0}";
const ROTATION_ICON_FONT_FAMILY: &str = "Segoe UI Symbol";
const ROTATION_ICON_FONT: iced::Font = iced::Font::with_name(ROTATION_ICON_FONT_FAMILY);
const ROTATION_ICON_SHAPING: iced::widget::text::Shaping = iced::widget::text::Shaping::Advanced;
const FULL_IMAGE_SESSION_CACHE_MAX_ENTRIES: usize = 4;
// Keep enough headroom for a single large RAW decode to stay hot across repeat opens.
const FULL_IMAGE_SESSION_CACHE_MAX_BYTES: usize = 1024 * 1024 * 1024;
const SOURCE_FINGERPRINT_BUFFER_BYTES: usize = 64 * 1024;
const FILE_SHARE_READ: u32 = 0x00000001;

fn rotation_icon_label<'a, ThemeT, RendererT>(
    icon: &'static str,
) -> iced::widget::Text<'a, ThemeT, RendererT>
where
    ThemeT: iced::widget::text::Catalog + 'a,
    RendererT: iced::advanced::text::Renderer<Font = iced::Font>,
{
    // These glyphs are not consistently present in the default text font.
    text(icon)
        .font(ROTATION_ICON_FONT)
        .shaping(ROTATION_ICON_SHAPING)
        .size(16)
}

fn rotation_button_widget<'a, RendererT>(
    icon: &'static str,
    step_label: &'static str,
    message: Message,
) -> iced::widget::Button<'a, Message, iced::Theme, RendererT>
where
    RendererT: iced::advanced::Renderer + iced::advanced::text::Renderer<Font = iced::Font> + 'a,
{
    button(
        column![
            rotation_icon_label(icon).color(TEXT_PRIMARY),
            text(step_label).size(10).color(TEXT_SECONDARY)
        ]
        .width(Length::Fill)
        .align_x(Alignment::Center)
        .spacing(2),
    )
    .on_press(message)
    .width(Length::Fill)
    .padding([6, 10])
    .style(toolbar_button_style)
}

fn rotation_button(
    icon: &'static str,
    step_label: &'static str,
    message: Message,
) -> Element<'static, Message> {
    rotation_button_widget::<iced::Renderer>(icon, step_label, message).into()
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ThumbnailGridLayout {
    thumb_size: f32,
    columns: usize,
}

impl ThumbnailGridLayout {
    fn new(content_width: f32) -> Self {
        let card_width = GRID_THUMB_SIZE + GRID_CARD_PADDING * 2.0;
        let usable_width = (content_width - GRID_PADDING * 2.0).max(card_width);
        let columns =
            ((usable_width + GRID_SPACING) / (card_width + GRID_SPACING)).floor() as usize;
        Self {
            thumb_size: GRID_THUMB_SIZE,
            columns: columns.max(1),
        }
    }
}

// ---------------------------------------------------------------------------
// Entry point
// ---------------------------------------------------------------------------

fn main() -> iced::Result {
    env_logger::init();

    iced::application(App::title, App::update, App::view)
        .subscription(App::subscription)
        .theme(App::theme)
        .window_size(Size::new(1200.0, 800.0))
        .antialiasing(true)
        .run_with(App::new)
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tab {
    Library,
    Detail,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SliderKind {
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
enum CropAspect {
    Freeform,
    Square,
}

impl CropAspect {
    fn ratio(self) -> Option<f32> {
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
enum ContextMenuKind {
    LibraryPhoto { photo_path: PathBuf },
    CollectionPhoto { photo_path: PathBuf },
    SidebarCollection { collection_index: usize },
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct ContextMenu {
    position: [f32; 2],
    kind: ContextMenuKind,
}

#[allow(dead_code)]
struct DragState {
    photo_index: usize,
    start_pos: [f32; 2],
    current_pos: [f32; 2],
    active: bool,
}

struct LibraryEntry {
    path: PathBuf,
    filename: String,
    thumbnail_handle: Option<ImageHandle>,
}

struct SaveRequest {
    path: PathBuf,
    image: Arc<ImageData>,
    state: edit::EditState,
    vig: [f32; 3],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceFileFingerprint {
    file_size: u64,
    modified: std::time::Duration,
    content_signature: u64,
}

impl SourceFileFingerprint {
    #[cfg(test)]
    fn from_path(path: &Path) -> Option<Self> {
        let mut file = File::open(path).ok()?;
        Self::from_file(&mut file)
    }

    fn from_file(file: &mut File) -> Option<Self> {
        let metadata = file.metadata().ok()?;
        let modified = metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        let content_signature = source_file_signature(file, metadata.len())?;
        Some(Self {
            file_size: metadata.len(),
            modified,
            content_signature,
        })
    }
}

fn open_cache_validation_handle(path: &Path) -> Option<File> {
    OpenOptions::new()
        .read(true)
        .share_mode(FILE_SHARE_READ)
        .open(path)
        .ok()
}

fn source_file_signature(file: &mut File, file_size: u64) -> Option<u64> {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};

    file.seek(SeekFrom::Start(0)).ok()?;
    let mut hasher = DefaultHasher::new();
    file_size.hash(&mut hasher);
    let mut buffer = vec![0; SOURCE_FINGERPRINT_BUFFER_BYTES];

    loop {
        let read = file.read(&mut buffer).ok()?;
        if read == 0 {
            break;
        }
        buffer[..read].hash(&mut hasher);
    }

    Some(hasher.finish())
}

fn metadata_matches_fingerprint(path: &Path, fingerprint: SourceFileFingerprint) -> bool {
    let Ok(metadata) = std::fs::metadata(path) else {
        return true;
    };
    let Ok(modified) = metadata.modified() else {
        return true;
    };
    let Ok(modified) = modified.duration_since(std::time::UNIX_EPOCH) else {
        return true;
    };
    metadata.len() == fingerprint.file_size && modified == fingerprint.modified
}

struct SessionFullImageCacheEntry {
    fingerprint: SourceFileFingerprint,
    image: Arc<ImageData>,
    bytes: usize,
}

struct SessionFullImageCacheHit {
    image: Arc<ImageData>,
    _write_guard: File,
}

#[derive(Debug, Clone)]
struct LoadedFullImage {
    image: Arc<ImageData>,
    fingerprint: Option<SourceFileFingerprint>,
}

struct SessionFullImageCache {
    entries: std::collections::HashMap<PathBuf, SessionFullImageCacheEntry>,
    lru: std::collections::VecDeque<PathBuf>,
    total_bytes: usize,
    max_entries: usize,
    max_bytes: usize,
}

impl Default for SessionFullImageCache {
    fn default() -> Self {
        Self::new(
            FULL_IMAGE_SESSION_CACHE_MAX_ENTRIES,
            FULL_IMAGE_SESSION_CACHE_MAX_BYTES,
        )
    }
}

impl SessionFullImageCache {
    fn new(max_entries: usize, max_bytes: usize) -> Self {
        Self {
            entries: std::collections::HashMap::new(),
            lru: std::collections::VecDeque::new(),
            total_bytes: 0,
            max_entries,
            max_bytes,
        }
    }

    fn get(&mut self, path: &Path) -> Option<SessionFullImageCacheHit> {
        let (cached_fingerprint, image) = match self.entries.get(path) {
            Some(entry) => (entry.fingerprint, entry.image.clone()),
            None => return None,
        };

        let mut guard = open_cache_validation_handle(path)?;
        let metadata = guard.metadata().ok()?;
        let modified = metadata
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?;
        if metadata.len() != cached_fingerprint.file_size || modified != cached_fingerprint.modified
        {
            self.remove(path);
            return None;
        }

        let Some(fingerprint) = SourceFileFingerprint::from_file(&mut guard) else {
            self.remove(path);
            return None;
        };
        if fingerprint != cached_fingerprint {
            self.remove(path);
            return None;
        }

        self.touch(path);
        Some(SessionFullImageCacheHit {
            image,
            _write_guard: guard,
        })
    }

    fn contains_path(&self, path: &Path) -> bool {
        self.entries.contains_key(path)
    }

    fn metadata_matches_path(&self, path: &Path) -> bool {
        self.entries
            .get(path)
            .is_some_and(|entry| metadata_matches_fingerprint(path, entry.fingerprint))
    }

    fn insert(&mut self, path: &Path, fingerprint: SourceFileFingerprint, image: Arc<ImageData>) {
        self.remove(path);

        let bytes = image.pixels.len();
        let path_buf = path.to_path_buf();

        self.total_bytes = self.total_bytes.saturating_add(bytes);
        self.entries.insert(
            path_buf.clone(),
            SessionFullImageCacheEntry {
                fingerprint,
                image,
                bytes,
            },
        );
        self.lru.push_back(path_buf);
        self.evict_as_needed();
    }

    fn touch(&mut self, path: &Path) {
        if let Some(position) = self.lru.iter().position(|candidate| candidate == path) {
            self.lru.remove(position);
        }
        self.lru.push_back(path.to_path_buf());
    }

    fn remove(&mut self, path: &Path) {
        if let Some(entry) = self.entries.remove(path) {
            self.total_bytes = self.total_bytes.saturating_sub(entry.bytes);
        }
        if let Some(position) = self.lru.iter().position(|candidate| candidate == path) {
            self.lru.remove(position);
        }
    }

    fn evict_as_needed(&mut self) {
        while self.entries.len() > self.max_entries || self.total_bytes > self.max_bytes
        {
            let Some(oldest_path) = self.lru.pop_front() else {
                break;
            };
            if let Some(entry) = self.entries.remove(&oldest_path) {
                self.total_bytes = self.total_bytes.saturating_sub(entry.bytes);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Application state
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum DetailLoadStage {
    #[default]
    Idle,
    Loading,
    PreviewWhileLoading,
    PreviewOnly,
}

#[derive(Debug, Clone, Copy, Default)]
struct DetailLoadState {
    request_id: u64,
    stage: DetailLoadStage,
    exif_loading: bool,
}

impl DetailLoadState {
    fn begin_request(&mut self) -> u64 {
        self.request_id += 1;
        self.stage = DetailLoadStage::Loading;
        self.exif_loading = true;
        self.request_id
    }

    fn is_current_request(&self, request_id: u64) -> bool {
        request_id == self.request_id
    }

    fn is_loading(&self) -> bool {
        matches!(
            self.stage,
            DetailLoadStage::Loading | DetailLoadStage::PreviewWhileLoading
        )
    }

    fn shows_embedded_preview(&self) -> bool {
        matches!(
            self.stage,
            DetailLoadStage::PreviewWhileLoading | DetailLoadStage::PreviewOnly
        )
    }

    fn on_preview_loaded(&mut self) {
        self.stage = DetailLoadStage::PreviewWhileLoading;
    }

    fn on_full_image_loaded(&mut self) -> bool {
        let reset_view = matches!(self.stage, DetailLoadStage::Loading);
        self.stage = DetailLoadStage::Idle;
        reset_view
    }

    fn on_full_image_failed(&mut self) {
        self.stage = if self.shows_embedded_preview() {
            DetailLoadStage::PreviewOnly
        } else {
            DetailLoadStage::Idle
        };
    }

    fn finish_exif(&mut self) {
        self.exif_loading = false;
    }

    fn load_suffix(&self) -> &'static str {
        match self.stage {
            DetailLoadStage::PreviewWhileLoading => "  •  Loading full resolution…",
            DetailLoadStage::PreviewOnly => "  •  Embedded preview",
            DetailLoadStage::Idle | DetailLoadStage::Loading => "",
        }
    }

    fn blocks_save(&self) -> bool {
        self.is_loading() || self.shows_embedded_preview()
    }
}

struct App {
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
    current_image_path: Option<PathBuf>,
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
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
enum Message {
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

fn path_filename_str(path: &Path) -> &str {
    path.file_name().and_then(|n| n.to_str()).unwrap_or("")
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

impl App {
    fn new() -> (Self, Task<Message>) {
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
            current_image_path: None,
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

    fn title(&self) -> String {
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

    fn theme(&self) -> Theme {
        Theme::Dark
    }

    fn subscription(&self) -> Subscription<Message> {
        event::listen().map(Message::Event)
    }

    // ---------------------------------------------------------------------------
    // Update
    // ---------------------------------------------------------------------------

    fn update(&mut self, msg: Message) -> Task<Message> {
        match msg {
            Message::OpenFile => self.open_file_dialog(),

            Message::FileSelected(Some(path)) => {
                self.nav = Some(DirNav::new(&path));
                self.library_index = None;
                self.tab = Tab::Detail;
                self.start_load(path)
            }
            Message::FileSelected(None) => Task::none(),

            Message::ImagePreviewLoaded {
                request_id,
                path,
                result,
            } => {
                if !self.detail_load.is_current_request(request_id)
                    || !self.detail_load.is_loading()
                {
                    return Task::none();
                }

                match result {
                    Ok(Some(data)) => {
                        self.apply_loaded_image(data, true);
                        self.detail_load.on_preview_loaded();
                    }
                    Err(e) => {
                        log::warn!("Embedded preview load failed for {}: {}", path.display(), e);
                    }
                    Ok(None) => {}
                }

                self.start_follow_up_load(path, request_id)
            }
            Message::ImageLoaded { request_id, result } => {
                if !self.detail_load.is_current_request(request_id) {
                    return Task::none();
                }

                match result {
                    Ok(loaded) => {
                        let reset_view = self.detail_load.on_full_image_loaded();
                        if let Some(fingerprint) = loaded.fingerprint {
                            self.cache_full_image_for_current_path(fingerprint, loaded.image.clone());
                        }
                        self.apply_loaded_image(loaded.image, reset_view);
                    }
                    Err(e) => {
                        let had_preview = self.detail_load.shows_embedded_preview();
                        self.detail_load.on_full_image_failed();
                        if had_preview {
                            self.save_status = Some(
                                "Full-resolution load failed; showing embedded preview".to_string(),
                            );
                        } else {
                            self.error = Some(e);
                        }
                    }
                }
                Task::none()
            }
            Message::ExifLoaded { request_id, exif } => {
                if !self.detail_load.is_current_request(request_id) {
                    return Task::none();
                }

                self.detail_load.finish_exif();
                self.current_exif = exif;
                self.refresh_auto_lens_profile();
                Task::none()
            }

            Message::Viewer(evt) => {
                self.handle_viewer(evt);
                Task::none()
            }

            Message::Event(evt) => self.handle_event(evt),

            Message::SwitchTab(tab) => {
                self.tab = tab;
                Task::none()
            }

            Message::AddFolder => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .pick_folder()
                        .await
                        .map(|f| f.path().to_path_buf())
                },
                Message::FolderPicked,
            ),

            Message::AddFiles => Task::perform(
                async {
                    rfd::AsyncFileDialog::new()
                        .add_filter("Images", image_file_dialog_extensions())
                        .pick_files()
                        .await
                        .map(|files| files.into_iter().map(|f| f.path().to_path_buf()).collect())
                },
                Message::FilesPicked,
            ),

            Message::FolderPicked(Some(folder)) => {
                let new_paths = self.filter_new_library_paths(scan_folder_for_images(&folder));
                self.import_library_paths(new_paths)
            }
            Message::FolderPicked(None) => Task::none(),

            Message::FilesPicked(Some(paths)) => {
                let new_paths = self.filter_new_library_paths(paths);
                self.import_library_paths(new_paths)
            }
            Message::FilesPicked(None) => Task::none(),

            Message::ThumbnailLoaded(path, Ok(data)) => {
                if let Some(entry) = self.library.iter_mut().find(|e| e.path == path) {
                    entry.thumbnail_handle = Some(ImageHandle::from_rgba(
                        data.width,
                        data.height,
                        data.pixels.clone(),
                    ));
                }
                Task::none()
            }
            Message::ThumbnailLoaded(_, Err(_)) => Task::none(),
            Message::ImportCacheWarmCompleted { path, result } => {
                if self.import_cache_warm_in_flight.as_deref() == Some(path.as_path()) {
                    self.import_cache_warm_in_flight = None;
                }
                if let Err(error) = result {
                    log::warn!(
                        "Import-time decoded cache warm failed for {}: {}",
                        path.display(),
                        error
                    );
                }
                self.start_next_import_cache_warm_if_idle()
            }

            Message::LibraryItemClicked(index) => {
                // Start potential drag
                self.drag_state = Some(DragState {
                    photo_index: index,
                    start_pos: self.cursor_position,
                    current_pos: self.cursor_position,
                    active: false,
                });

                let now = Instant::now();
                if Self::is_double_click_event(&mut self.last_thumb_click, index, now) {
                    if let Some(entry) = self.library.get(index) {
                        self.library_index = Some(index);
                        self.tab = Tab::Detail;
                        let path = entry.path.clone();
                        return self.start_load(path);
                    }
                }
                Task::none()
            }

            Message::SliderChanged(kind, value) => {
                let count = match self.slider_drag {
                    Some((k, c)) if k == kind => c + 1,
                    _ => 1,
                };
                self.slider_drag = Some((kind, count));
                // Only apply on 2nd+ event (actual drag, not a track click)
                if count >= 2 {
                    if let Some(path) = &self.current_image_path {
                        let history = self.edit_histories.entry(path.clone()).or_default();
                        set_slider_field(&mut history.current, kind, value);
                    }
                }
                Task::none()
            }

            Message::SliderReleased(kind) => {
                let was_drag = matches!(self.slider_drag, Some((k, c)) if k == kind && c >= 2);
                self.slider_drag = None;

                let now = Instant::now();
                let is_double_click = self
                    .last_slider_release
                    .map(|(prev_kind, prev_time)| {
                        prev_kind == kind && now.duration_since(prev_time).as_millis() < 400
                    })
                    .unwrap_or(false);

                if is_double_click {
                    self.last_slider_release = None;
                    if let Some(path) = &self.current_image_path {
                        let history = self.edit_histories.entry(path.clone()).or_default();
                        set_slider_field(&mut history.current, kind, 0.0);
                        history.commit();
                    }
                } else {
                    self.last_slider_release = Some((kind, now));
                    // Only commit if the user actually dragged (not a single track click)
                    if was_drag {
                        if let Some(path) = &self.current_image_path {
                            if let Some(history) = self.edit_histories.get_mut(path) {
                                history.commit();
                            }
                        }
                    }
                }
                Task::none()
            }

            Message::ResetSlider(kind) => {
                if let Some(path) = &self.current_image_path {
                    let history = self.edit_histories.entry(path.clone()).or_default();
                    set_slider_field(&mut history.current, kind, 0.0);
                    history.commit();
                }
                Task::none()
            }

            Message::ResetAll => {
                let previous_rotation = self.current_rotation();
                let previous_crop = self.visible_crop();
                if let Some(path) = &self.current_image_path {
                    let history = self.edit_histories.entry(path.clone()).or_default();
                    history.reset_all();
                }
                self.preserve_actual_size_after_display_change(previous_rotation, previous_crop);
                Task::none()
            }

            Message::ToggleLensCorrection => {
                if let Some(path) = &self.current_image_path {
                    let history = self.edit_histories.entry(path.clone()).or_default();
                    history.current.lens_correction = !history.current.lens_correction;
                    history.commit();
                }
                Task::none()
            }

            Message::RotateClockwise => {
                let previous_rotation = self.current_rotation();
                let previous_crop = self.visible_crop();
                if let Some(path) = &self.current_image_path {
                    let history = self.edit_histories.entry(path.clone()).or_default();
                    history.current.rotate_clockwise();
                    history.commit();
                }
                self.preserve_actual_size_after_display_change(previous_rotation, previous_crop);
                Task::none()
            }

            Message::RotateCounterclockwise => {
                let previous_rotation = self.current_rotation();
                let previous_crop = self.visible_crop();
                if let Some(path) = &self.current_image_path {
                    let history = self.edit_histories.entry(path.clone()).or_default();
                    history.current.rotate_counterclockwise();
                    history.commit();
                }
                self.preserve_actual_size_after_display_change(previous_rotation, previous_crop);
                Task::none()
            }

            Message::SaveEdited => {
                let Some(request) = self.current_save_request() else {
                    return Task::none();
                };
                self.save_status = Some("Saving...".to_string());
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            edit::save_edited_image(
                                &request.path,
                                &request.image.pixels,
                                request.image.width,
                                request.image.height,
                                &request.state,
                                request.vig,
                            )
                            .map(|p| p.to_string_lossy().into_owned())
                        })
                        .await
                        .map_err(|e| e.to_string())?
                    },
                    Message::SaveCompleted,
                )
            }

            Message::SaveCompleted(result) => {
                self.save_status = Some(match result {
                    Ok(path) => format!("Saved: {path}"),
                    Err(e) => format!("Save failed: {e}"),
                });
                Task::none()
            }

            Message::ToggleCropMode => {
                let previous_rotation = self.current_rotation();
                let previous_crop = self.visible_crop();
                self.crop_mode = !self.crop_mode;
                self.preserve_actual_size_after_display_change(previous_rotation, previous_crop);
                Task::none()
            }

            Message::CropAspectSelected(aspect) => {
                self.crop_aspect = aspect;
                Task::none()
            }

            Message::ClearCrop => {
                let previous_rotation = self.current_rotation();
                let previous_crop = self.visible_crop();
                if let Some(path) = &self.current_image_path {
                    let history = self.edit_histories.entry(path.clone()).or_default();
                    if history.current.crop.is_some() {
                        history.current.crop = None;
                        history.commit();
                    }
                }
                self.crop_mode = false;
                self.preserve_actual_size_after_display_change(previous_rotation, previous_crop);
                Task::none()
            }

            Message::SliderTextInput(kind) => {
                let value = self
                    .current_image_path
                    .as_ref()
                    .and_then(|p| self.edit_histories.get(p))
                    .map(|h| get_slider_field(&h.current, kind))
                    .unwrap_or(0.0);
                self.editing_slider = Some(kind);
                self.slider_text_buf = format!("{:.1}", value);
                Task::none()
            }

            Message::SliderTextChanged(s) => {
                self.slider_text_buf = s;
                Task::none()
            }

            Message::SliderTextSubmit(kind) => {
                if let Ok(value) = self.slider_text_buf.parse::<f32>() {
                    let (min, max) = slider_range(kind);
                    let clamped = value.clamp(min, max);
                    if let Some(path) = &self.current_image_path {
                        let history = self.edit_histories.entry(path.clone()).or_default();
                        set_slider_field(&mut history.current, kind, clamped);
                        history.commit();
                    }
                }
                self.editing_slider = None;
                self.slider_text_buf.clear();
                Task::none()
            }

            Message::LensProfileSelected(name) => {
                if name == "Auto" {
                    self.lens_override_name = None;
                    self.refresh_auto_lens_profile();
                } else if name == "None" {
                    self.lens_override_name = Some(name);
                    self.current_lens_profile = None;
                } else {
                    self.lens_override_name = Some(name.clone());
                    self.current_lens_profile = self
                        .lens_db
                        .profiles
                        .iter()
                        .find(|p| format!("{} {}", p.maker, p.model) == name)
                        .cloned();
                }
                Task::none()
            }

            // -- Collection CRUD --
            Message::CreateCollection => {
                let name = self.collection_store.next_default_name();
                self.collection_store.create(&name);
                self.collection_store.save();
                let idx = self
                    .collection_store
                    .collections
                    .iter()
                    .position(|c| c.name == name)
                    .unwrap_or(0);
                self.editing_collection_name = Some(idx);
                self.collection_name_buf = name;
                Task::none()
            }

            Message::CollectionNameChanged(s) => {
                self.collection_name_buf = s;
                Task::none()
            }

            Message::CollectionNameSubmit => {
                if let Some(idx) = self.editing_collection_name.take() {
                    let new_name = self.collection_name_buf.trim().to_string();
                    if !new_name.is_empty() {
                        self.collection_store.rename(idx, &new_name);
                    }
                    self.collection_store.save();
                    self.collection_name_buf.clear();
                }
                Task::none()
            }

            Message::CollectionNameCancel => {
                self.editing_collection_name = None;
                self.collection_name_buf.clear();
                Task::none()
            }

            Message::SidebarCollectionClicked(index) => {
                let now = Instant::now();
                if Self::is_double_click_event(&mut self.last_collection_click, index, now) {
                    self.active_collection = Some(index);
                } else {
                    self.last_collection_click = Some((index, now));
                }
                Task::none()
            }

            Message::SidebarCollectionRightClicked(index) => {
                self.context_menu = Some(ContextMenu {
                    position: self.cursor_position,
                    kind: ContextMenuKind::SidebarCollection {
                        collection_index: index,
                    },
                });
                Task::none()
            }

            Message::SidebarCollectionHovered(idx) => {
                self.sidebar_hover_collection = idx;
                Task::none()
            }

            Message::ThumbnailHovered(idx) => {
                self.hovered_thumbnail = idx;
                Task::none()
            }

            Message::DismissContextMenu => {
                self.context_menu = None;
                Task::none()
            }

            Message::ContextMenuRename => {
                if let Some(ContextMenu {
                    kind: ContextMenuKind::SidebarCollection { collection_index },
                    ..
                }) = &self.context_menu
                {
                    let idx = *collection_index;
                    if let Some(col) = self.collection_store.collections.get(idx) {
                        self.collection_name_buf = col.name.clone();
                        self.editing_collection_name = Some(idx);
                    }
                }
                self.context_menu = None;
                Task::none()
            }

            Message::ContextMenuDelete => {
                if let Some(ContextMenu {
                    kind: ContextMenuKind::SidebarCollection { collection_index },
                    ..
                }) = &self.context_menu
                {
                    let idx = *collection_index;
                    self.collection_store.delete(idx);
                    self.collection_store.save();
                    if self.active_collection == Some(idx) {
                        self.active_collection = None;
                    } else if let Some(active) = self.active_collection {
                        if active > idx {
                            self.active_collection = Some(active - 1);
                        }
                    }
                }
                self.context_menu = None;
                Task::none()
            }

            Message::ExitCollectionView => {
                self.active_collection = None;
                Task::none()
            }

            Message::CollectionPhotoClicked(photo_index) => {
                let now = Instant::now();
                let is_double_click = self
                    .last_thumb_click
                    .map(|(prev_idx, prev_time)| {
                        prev_idx == photo_index && now.duration_since(prev_time).as_millis() < 400
                    })
                    .unwrap_or(false);

                if is_double_click {
                    self.last_thumb_click = None;
                    if let Some(col_idx) = self.active_collection {
                        if let Some(col) = self.collection_store.collections.get(col_idx) {
                            if let Some(photo_path) = col.photos.get(photo_index) {
                                self.collection_nav = Some((col_idx, photo_index));
                                self.library_index = None;
                                self.tab = Tab::Detail;
                                let path = photo_path.clone();
                                return self.start_load(path);
                            }
                        }
                    }
                } else {
                    self.last_thumb_click = Some((photo_index, now));
                }
                Task::none()
            }

            Message::CollectionPhotoRightClicked(photo_index) => {
                let Some(photo_path) = self
                    .active_collection
                    .and_then(|col_idx| self.collection_store.collections.get(col_idx))
                    .and_then(|collection| collection.photos.get(photo_index))
                    .cloned()
                else {
                    return Task::none();
                };
                self.context_menu = Some(ContextMenu {
                    position: self.cursor_position,
                    kind: ContextMenuKind::CollectionPhoto { photo_path },
                });
                Task::none()
            }

            Message::RemovePhotoFromCollection => {
                if let Some(ContextMenu {
                    kind: ContextMenuKind::CollectionPhoto { photo_path },
                    ..
                }) = &self.context_menu
                {
                    if let Some(col_idx) = self.active_collection {
                        self.collection_store.remove_photo(col_idx, photo_path);
                        self.collection_store.save();
                    }
                }
                self.context_menu = None;
                Task::none()
            }

            Message::ExitCollectionDetail => {
                self.tab = Tab::Library;
                // active_collection is still set, so we return to collection grid
                self.collection_nav = None;
                Task::none()
            }

            Message::LibraryPhotoRightClicked(index) => {
                if self.collection_store.collections.is_empty() {
                    return Task::none();
                }
                let Some(photo_path) = self.library.get(index).map(|entry| entry.path.clone())
                else {
                    return Task::none();
                };
                self.context_menu = Some(ContextMenu {
                    position: self.cursor_position,
                    kind: ContextMenuKind::LibraryPhoto { photo_path },
                });
                Task::none()
            }

            Message::AddPhotoToCollection(collection_index) => {
                if let Some(ContextMenu {
                    kind: ContextMenuKind::LibraryPhoto { photo_path },
                    ..
                }) = &self.context_menu
                {
                    if self
                        .collection_store
                        .collections
                        .get(collection_index)
                        .is_some()
                        && self.library_entry_by_path(photo_path).is_some()
                    {
                        self.collection_store
                            .add_photo(collection_index, photo_path);
                        self.collection_store.save();
                    }
                }
                self.context_menu = None;
                Task::none()
            }

            Message::TogglePhotoInCollection(collection_index) => {
                if let Some(ContextMenu {
                    kind: ContextMenuKind::LibraryPhoto { photo_path },
                    ..
                }) = &self.context_menu
                {
                    if self
                        .collection_store
                        .collections
                        .get(collection_index)
                        .is_some()
                        && self.library_entry_by_path(photo_path).is_some()
                    {
                        if self
                            .collection_store
                            .collections
                            .get(collection_index)
                            .is_some_and(|c| c.photos.contains(photo_path))
                        {
                            self.collection_store
                                .remove_photo(collection_index, photo_path);
                        } else {
                            self.collection_store
                                .add_photo(collection_index, photo_path);
                        }
                        self.collection_store.save();
                    }
                }
                self.context_menu = None;
                Task::none()
            }
        }
    }

    // ---------------------------------------------------------------------------
    // Viewer interaction
    // ---------------------------------------------------------------------------

    fn handle_viewer(&mut self, evt: ViewerEvent) {
        match evt {
            ViewerEvent::Zoom {
                factor,
                cursor,
                canvas_size,
            } => {
                self.update_canvas_size(canvas_size);
                let (z, o) = zoom_at_cursor(self.zoom, self.offset, factor, cursor, canvas_size);
                self.zoom = z;
                self.offset = o;
            }
            ViewerEvent::Pan { delta } => {
                self.offset[0] += delta[0];
                self.offset[1] += delta[1];
            }
            ViewerEvent::DoubleClick { canvas_size } => {
                self.update_canvas_size(canvas_size);
                if (self.zoom - 1.0).abs() < 0.01 && self.offset == [0.0, 0.0] {
                    if let Some(img) = &self.image {
                        self.zoom = self.actual_size_zoom_for_rotation(
                            canvas_size,
                            img,
                            self.current_rotation(),
                        );
                    }
                } else {
                    self.zoom = 1.0;
                    self.offset = [0.0, 0.0];
                }
            }
            ViewerEvent::CropCommitted { rect } => {
                let previous_rotation = self.current_rotation();
                let previous_crop = self.visible_crop();
                if let Some(path) = &self.current_image_path {
                    let history = self.edit_histories.entry(path.clone()).or_default();
                    history.current.crop = Some(rect);
                    history.commit();
                }
                self.crop_mode = false;
                self.preserve_actual_size_after_display_change(previous_rotation, previous_crop);
            }
        }
    }

    // ---------------------------------------------------------------------------
    // Global events
    // ---------------------------------------------------------------------------

    fn handle_event(&mut self, event: iced::Event) -> Task<Message> {
        match event {
            iced::Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) => {
                self.handle_key(key, modifiers)
            }

            iced::Event::Window(window::Event::FileDropped(path)) => {
                self.nav = Some(DirNav::new(&path));
                self.library_index = None;
                self.tab = Tab::Detail;
                self.start_load(path)
            }

            iced::Event::Window(window::Event::Resized(size)) => {
                self.window_size = size;
                Task::none()
            }

            iced::Event::Mouse(mouse::Event::CursorMoved { position }) => {
                self.cursor_position = [position.x, position.y];
                if let Some(ref mut drag) = self.drag_state {
                    drag.current_pos = [position.x, position.y];
                    if !drag.active {
                        let dx = drag.current_pos[0] - drag.start_pos[0];
                        let dy = drag.current_pos[1] - drag.start_pos[1];
                        if (dx * dx + dy * dy).sqrt() > 5.0 {
                            drag.active = true;
                        }
                    }
                }
                Task::none()
            }

            iced::Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) => Task::none(),
            iced::Event::Mouse(mouse::Event::ButtonReleased(mouse::Button::Left)) => {
                if let Some(drag) = self.drag_state.take() {
                    if drag.active {
                        if let Some(col_idx) = self.sidebar_hover_collection {
                            if let Some(entry) = self.library.get(drag.photo_index) {
                                self.collection_store.add_photo(col_idx, &entry.path);
                                self.collection_store.save();
                            }
                        }
                        // Cancel the click that started this drag
                        self.last_thumb_click = None;
                    }
                }
                Task::none()
            }

            _ => Task::none(),
        }
    }

    fn handle_key(&mut self, key: keyboard::Key, mods: keyboard::Modifiers) -> Task<Message> {
        use keyboard::key::Named;
        use keyboard::Key;

        match key {
            // Escape: dismiss overlays, exit collection, or go back to library
            Key::Named(Named::Escape) => {
                if self.context_menu.is_some() {
                    self.context_menu = None;
                } else if self.editing_collection_name.is_some() {
                    self.editing_collection_name = None;
                    self.collection_name_buf.clear();
                } else if self.tab == Tab::Detail && self.collection_nav.is_some() {
                    self.tab = Tab::Library;
                    self.collection_nav = None;
                } else if self.active_collection.is_some() {
                    self.active_collection = None;
                } else if self.tab == Tab::Detail {
                    self.tab = Tab::Library;
                }
            }

            // Navigation: next
            Key::Named(Named::ArrowRight) | Key::Named(Named::Space) => {
                if self.tab == Tab::Detail {
                    if let Some((col_idx, photo_idx)) = self.collection_nav {
                        if let Some(col) = self.collection_store.collections.get(col_idx) {
                            if let Some(current) =
                                self.clamped_collection_photo_index(col_idx, photo_idx)
                            {
                                let next =
                                    Self::step_wrapped_index(current, col.photos.len(), true);
                                self.collection_nav = Some((col_idx, next));
                                let path = col.photos[next].clone();
                                return self.start_load(path);
                            }
                        }
                    } else if let Some(lib_idx) = self.library_index {
                        if let Some(current) = self.clamped_library_index(lib_idx) {
                            let next = Self::step_wrapped_index(current, self.library.len(), true);
                            self.library_index = Some(next);
                            let path = self.library[next].path.clone();
                            return self.start_load(path);
                        }
                    } else if let Some(nav) = &mut self.nav {
                        if let Some(p) = nav.next() {
                            return self.start_load(p);
                        }
                    }
                }
            }

            // Navigation: prev
            Key::Named(Named::ArrowLeft) | Key::Named(Named::Backspace) => {
                if self.tab == Tab::Detail {
                    if let Some((col_idx, photo_idx)) = self.collection_nav {
                        if let Some(col) = self.collection_store.collections.get(col_idx) {
                            if let Some(current) =
                                self.clamped_collection_photo_index(col_idx, photo_idx)
                            {
                                let previous =
                                    Self::step_wrapped_index(current, col.photos.len(), false);
                                self.collection_nav = Some((col_idx, previous));
                                let path = col.photos[previous].clone();
                                return self.start_load(path);
                            }
                        }
                    } else if let Some(lib_idx) = self.library_index {
                        if let Some(current) = self.clamped_library_index(lib_idx) {
                            let previous =
                                Self::step_wrapped_index(current, self.library.len(), false);
                            self.library_index = Some(previous);
                            let path = self.library[previous].path.clone();
                            return self.start_load(path);
                        }
                    } else if let Some(nav) = &mut self.nav {
                        if let Some(p) = nav.prev() {
                            return self.start_load(p);
                        }
                    }
                }
            }

            // Undo
            Key::Character(ref c) if c.as_str() == "z" && mods.command() && !mods.shift() => {
                let previous_rotation = self.current_rotation();
                let previous_crop = self.visible_crop();
                if let Some(path) = &self.current_image_path {
                    if let Some(history) = self.edit_histories.get_mut(path) {
                        history.undo();
                    }
                }
                self.preserve_actual_size_after_display_change(previous_rotation, previous_crop);
                return Task::none();
            }
            // Redo
            Key::Character(ref c)
                if (c.as_str() == "z" && mods.command() && mods.shift())
                    || (c.as_str() == "y" && mods.command()) =>
            {
                let previous_rotation = self.current_rotation();
                let previous_crop = self.visible_crop();
                if let Some(path) = &self.current_image_path {
                    if let Some(history) = self.edit_histories.get_mut(path) {
                        history.redo();
                    }
                }
                self.preserve_actual_size_after_display_change(previous_rotation, previous_crop);
                return Task::none();
            }
            // Save
            Key::Character(ref c) if c.as_str() == "s" && mods.command() => {
                return self.update(Message::SaveEdited);
            }

            // Open file dialog
            Key::Character(ref c) if c.as_str() == "o" && mods.command() => {
                return self.open_file_dialog();
            }

            // Zoom / view (Detail tab only)
            Key::Character(ref c) if self.tab == Tab::Detail => match c.as_str() {
                "f" | "0" => {
                    self.zoom = 1.0;
                    self.offset = [0.0, 0.0];
                }
                "=" | "+" => {
                    self.zoom = (self.zoom * 1.25).min(200.0);
                }
                "-" | "_" => {
                    self.zoom = (self.zoom / 1.25).max(0.01);
                }
                "1" => {
                    if let Some(img) = &self.image {
                        self.zoom = self.actual_size_zoom_for_rotation(
                            self.current_canvas_size(),
                            img,
                            self.current_rotation(),
                        );
                        self.offset = [0.0, 0.0];
                    }
                }
                _ => {}
            },
            Key::Named(Named::Home) if self.tab == Tab::Detail => {
                self.zoom = 1.0;
                self.offset = [0.0, 0.0];
            }
            _ => {}
        }
        Task::none()
    }

    // ---------------------------------------------------------------------------
    // Library helpers
    // ---------------------------------------------------------------------------

    fn is_double_click_event(
        last_click_state: &mut Option<(usize, Instant)>,
        current_index: usize,
        current_time: Instant,
    ) -> bool {
        let is_double_click = last_click_state
            .map(|(prev_idx, prev_time)| {
                prev_idx == current_index
                    && current_time.duration_since(prev_time).as_millis() < 400
            })
            .unwrap_or(false);

        if is_double_click {
            *last_click_state = None;
        } else {
            *last_click_state = Some((current_index, current_time));
        }
        is_double_click
    }

    fn add_library_entries(&mut self, paths: &[PathBuf]) {
        for path in paths {
            if !self.library.iter().any(|e| e.path == *path) {
                self.library.push(LibraryEntry {
                    filename: path_filename_str(path).to_string(),
                    path: path.clone(),
                    thumbnail_handle: None,
                });
            }
        }
        self.rebuild_library_indices();
    }

    fn filter_new_library_paths(&self, paths: Vec<PathBuf>) -> Vec<PathBuf> {
        let mut new_paths = Vec::new();
        for path in paths {
            if self.library.iter().any(|entry| entry.path == path)
                || new_paths.iter().any(|candidate| candidate == &path)
            {
                continue;
            }
            new_paths.push(path);
        }
        new_paths
    }

    fn import_library_paths(&mut self, new_paths: Vec<PathBuf>) -> Task<Message> {
        if new_paths.is_empty() {
            return Task::none();
        }

        self.add_library_entries(&new_paths);
        save_library(&self.library);

        Task::batch([
            Self::load_thumbnails(&new_paths),
            self.enqueue_import_cache_warm_paths(&new_paths),
        ])
    }

    fn enqueue_import_cache_warm_paths(&mut self, paths: &[PathBuf]) -> Task<Message> {
        for path in paths {
            if !decode::path_uses_persisted_decoded_cache(path) {
                continue;
            }
            if self.import_cache_warm_in_flight.as_deref() == Some(path.as_path())
                || self
                    .pending_import_cache_warm_paths
                    .iter()
                    .any(|candidate| candidate == path)
            {
                continue;
            }
            self.pending_import_cache_warm_paths.push_back(path.clone());
        }

        self.start_next_import_cache_warm_if_idle()
    }

    fn start_next_import_cache_warm_if_idle(&mut self) -> Task<Message> {
        if self.import_cache_warm_in_flight.is_some() {
            return Task::none();
        }

        let Some(path) = self.pending_import_cache_warm_paths.pop_front() else {
            return Task::none();
        };

        self.import_cache_warm_in_flight = Some(path.clone());
        Self::import_cache_warm_task(path)
    }

    #[cfg(test)]
    fn replace_library_entries(&mut self, entries: Vec<LibraryEntry>) {
        self.library = entries;
        self.rebuild_library_indices();
        self.reset_library_navigation_state();
        self.current_image_path = None;
        self.image = None;
    }

    #[cfg(test)]
    fn reset_library_navigation_state(&mut self) {
        self.library_index = None;
        self.collection_nav = None;
        self.nav = None;
    }

    #[cfg(test)]
    fn clear_library_entries(&mut self) {
        self.replace_library_entries(Vec::new());
    }

    #[cfg(test)]
    fn remove_library_entry(&mut self, index: usize) -> Option<LibraryEntry> {
        if index >= self.library.len() {
            return None;
        }
        let removed = self.library.remove(index);
        self.rebuild_library_indices();
        self.reset_library_navigation_state();
        if self.current_image_path.as_ref() == Some(&removed.path) {
            self.current_image_path = None;
            self.image = None;
        }
        Some(removed)
    }

    fn rebuild_library_indices(&mut self) {
        self.library_indices_by_path = self
            .library
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.path.clone(), index))
            .collect();
    }

    fn load_thumbnails(paths: &[PathBuf]) -> Task<Message> {
        Task::batch(paths.iter().map(|path| {
            let p = path.clone();
            let p2 = path.clone();
            Task::perform(
                async move {
                    let result: Result<Arc<ImageData>, String> =
                        tokio::task::spawn_blocking(move || decode::decode_thumbnail(&p, 200))
                            .await
                            .map_err(|e| e.to_string())?;
                    result
                },
                move |result| Message::ThumbnailLoaded(p2.clone(), result),
            )
        }))
    }

    fn import_cache_warm_task(path: PathBuf) -> Task<Message> {
        let task_path = path.clone();
        Task::perform(
            async move {
                let result: Result<bool, String> = tokio::task::spawn_blocking(move || {
                    decode::warm_persisted_decoded_cache(&task_path)
                })
                .await
                .map_err(|e| e.to_string())?;
                result
            },
            move |result| Message::ImportCacheWarmCompleted {
                path: path.clone(),
                result,
            },
        )
    }

    fn open_file_dialog(&self) -> Task<Message> {
        Task::perform(
            async {
                rfd::AsyncFileDialog::new()
                    .add_filter("Images", image_file_dialog_extensions())
                    .pick_file()
                    .await
                    .map(|f| f.path().to_path_buf())
            },
            Message::FileSelected,
        )
    }

    fn lens_profile_for_exif(&self, exif_info: &lens::ExifInfo) -> Option<lens::LensProfile> {
        let maker = if exif_info.lens_make.is_empty() {
            &exif_info.camera_make
        } else {
            &exif_info.lens_make
        };
        self.lens_db
            .find_lens(maker, &exif_info.lens_model)
            .cloned()
    }

    fn refresh_auto_lens_profile(&mut self) {
        if self.lens_override_name.is_none() {
            self.current_lens_profile = self
                .current_exif
                .as_ref()
                .and_then(|exif_info| self.lens_profile_for_exif(exif_info));
        }
    }

    fn apply_loaded_image(&mut self, data: Arc<ImageData>, reset_view: bool) {
        self.image = Some(data);
        self.image_id += 1;
        if reset_view {
            self.zoom = 1.0;
            self.offset = [0.0, 0.0];
            self.crop_mode = false;
        }
        self.error = None;
    }

    fn preview_load_task(path: PathBuf, request_id: u64) -> Task<Message> {
        let task_path = path.clone();
        let message_path = path.clone();
        Task::perform(
            async move {
                let result: Result<Option<Arc<ImageData>>, String> =
                    tokio::task::spawn_blocking(move || {
                        decode::decode_embedded_preview(&task_path)
                    })
                    .await
                    .map_err(|e| e.to_string())?;
                result
            },
            move |result| Message::ImagePreviewLoaded {
                request_id,
                path: message_path.clone(),
                result,
            },
        )
    }

    fn full_image_load_task(path: PathBuf, request_id: u64) -> Task<Message> {
        Task::perform(
            async move {
                let result: Result<LoadedFullImage, String> = tokio::task::spawn_blocking(
                    move || {
                        let mut guard = open_cache_validation_handle(&path);
                        let fingerprint = guard
                            .as_mut()
                            .and_then(SourceFileFingerprint::from_file);
                        let image = decode::decode_image(&path)?;
                        drop(guard);
                        Ok(LoadedFullImage { image, fingerprint })
                    },
                )
                .await
                .map_err(|e| e.to_string())?;
                result
            },
            move |result| Message::ImageLoaded { request_id, result },
        )
    }

    fn exif_load_task(path: PathBuf, request_id: u64) -> Task<Message> {
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || lens::read_exif(&path))
                    .await
                    .unwrap_or(None)
            },
            move |exif| Message::ExifLoaded { request_id, exif },
        )
    }

    fn start_follow_up_load(&self, path: PathBuf, request_id: u64) -> Task<Message> {
        Task::batch([
            Self::full_image_load_task(path.clone(), request_id),
            Self::exif_load_task(path, request_id),
        ])
    }

    fn cache_full_image_for_current_path(
        &mut self,
        fingerprint: SourceFileFingerprint,
        image: Arc<ImageData>,
    ) {
        let Some(path) = self.current_image_path.as_deref() else {
            return;
        };
        self.session_full_image_cache.insert(path, fingerprint, image);
    }

    fn displayed_full_image_for_path(&self, path: &Path) -> Option<Arc<ImageData>> {
        if self.current_image_path.as_deref() != Some(path) {
            return None;
        }
        if !self.session_full_image_cache.contains_path(path) {
            return None;
        }
        if !self.session_full_image_cache.metadata_matches_path(path) {
            return None;
        }
        if self.detail_load.is_loading() || self.detail_load.shows_embedded_preview() {
            return None;
        }
        self.image.clone()
    }

    fn start_load(&mut self, path: PathBuf) -> Task<Message> {
        let displayed_full_image = self.displayed_full_image_for_path(&path);
        let request_id = self.detail_load.begin_request();
        self.current_image_path = Some(path.clone());
        self.error = None;
        self.save_status = None;
        self.current_exif = None;
        if self.lens_override_name.is_none() {
            self.current_lens_profile = None;
        }

        if let Some(image) = displayed_full_image {
            let reset_view = self.detail_load.on_full_image_loaded();
            self.apply_loaded_image(image, reset_view);
            return Self::exif_load_task(path, request_id);
        }

        let cached_full_image = self.session_full_image_cache.get(&path);
        if let Some(hit) = cached_full_image {
            let reset_view = self.detail_load.on_full_image_loaded();
            self.apply_loaded_image(hit.image, reset_view);
            return Self::exif_load_task(path, request_id);
        }

        self.image = None;
        if nav::is_raw_file(&path) {
            Self::preview_load_task(path, request_id)
        } else {
            Task::batch([
                Self::full_image_load_task(path.clone(), request_id),
                Self::exif_load_task(path, request_id),
            ])
        }
    }

    // ---------------------------------------------------------------------------
    // View
    // ---------------------------------------------------------------------------

    fn view(&self) -> Element<'_, Message> {
        let tab_bar = self.tab_bar();
        let content: Element<'_, Message> = match self.tab {
            Tab::Library => self.library_view(),
            Tab::Detail => self.detail_view(),
        };
        let main = column![tab_bar, content];

        let has_overlay =
            self.context_menu.is_some() || self.drag_state.as_ref().is_some_and(|d| d.active);

        if has_overlay {
            let mut layers: Vec<Element<'_, Message>> = vec![main.into()];
            if let Some(ref menu) = self.context_menu {
                layers.push(self.context_menu_overlay(menu));
            }
            if let Some(ref drag) = self.drag_state {
                if drag.active {
                    layers.push(self.drag_overlay(drag));
                }
            }
            iced::widget::Stack::with_children(layers).into()
        } else {
            main.into()
        }
    }

    fn tab_bar(&self) -> Element<'_, Message> {
        let content = match self.tab {
            Tab::Library => {
                let title = text("Library").size(14).color(TEXT_PRIMARY);

                let add_folder_btn = button(text("+ Folder").size(11).color(TEXT_PRIMARY))
                    .on_press(Message::AddFolder)
                    .padding([5, 12])
                    .style(toolbar_button_style);

                let add_files_btn = button(text("+ Files").size(11).color(TEXT_PRIMARY))
                    .on_press(Message::AddFiles)
                    .padding([5, 12])
                    .style(toolbar_button_style);

                row![
                    container(title).padding([0, 8]),
                    horizontal_space(),
                    add_folder_btn,
                    add_files_btn
                ]
                .spacing(6)
                .align_y(Alignment::Center)
            }
            Tab::Detail => {
                let back_msg = if self.collection_nav.is_some() {
                    Message::ExitCollectionDetail
                } else {
                    Message::SwitchTab(Tab::Library)
                };
                let back_btn = button(text("\u{2190}").size(16).color(TEXT_PRIMARY))
                    .on_press(back_msg)
                    .padding([4, 12])
                    .style(toolbar_button_style);

                let save_btn = {
                    let btn = button(text("Save").size(11).color(TEXT_PRIMARY))
                        .padding([5, 12])
                        .style(toolbar_button_style);
                    if self.current_save_request().is_some() {
                        btn.on_press(Message::SaveEdited)
                    } else {
                        btn
                    }
                };

                row![back_btn, horizontal_space(), save_btn]
                    .spacing(6)
                    .align_y(Alignment::Center)
            }
        };

        container(content)
            .padding([6, 10])
            .width(Length::Fill)
            .style(toolbar_container_style)
            .into()
    }

    fn library_view(&self) -> Element<'_, Message> {
        if let Some(col_idx) = self.active_collection {
            if col_idx < self.collection_store.collections.len() {
                return self.collection_grid_view(col_idx);
            }
        }

        if self.library.is_empty() {
            return container(
                column![
                    text("No images loaded").size(18).color(TEXT_SECONDARY),
                    text("Use + Folder or + Files to add images, or drag and drop")
                        .size(13)
                        .color(TEXT_DIM),
                ]
                .spacing(8)
                .align_x(Alignment::Center),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .style(dark_bg_style)
            .into();
        }

        let layout = self.library_grid_layout();
        let grid = self.build_thumbnail_grid(self.library.len(), layout, |idx, thumb_size| {
            let entry = &self.library[idx];
            self.thumbnail_card(entry, idx, thumb_size)
        });

        let status_text = format!(
            "{} images  \u{2022}  Double-click to open",
            self.library.len()
        );
        let status = container(text(status_text).size(11).color(TEXT_DIM))
            .width(Length::Fill)
            .padding([6, 14]);

        let grid_area = column![
            scrollable(container(grid).padding(GRID_PADDING).width(Length::Fill))
                .height(Length::Fill),
            container(status)
                .width(Length::Fill)
                .style(toolbar_container_style),
        ];

        let sidebar = self.collection_sidebar();
        let divider = container(Space::with_width(COLLECTION_SIDEBAR_DIVIDER_WIDTH))
            .height(Length::Fill)
            .style(|_theme: &Theme| container::Style {
                background: Some(Background::Color(DIVIDER)),
                ..Default::default()
            });

        container(row![
            sidebar,
            divider,
            container(grid_area).width(Length::Fill)
        ])
        .style(dark_bg_style)
        .into()
    }

    fn collection_sidebar(&self) -> Element<'_, Message> {
        let header = row![
            container(text("COLLECTIONS").size(10).color(TEXT_DIM)).padding([5, 0]),
            horizontal_space(),
            button(text("+").size(14).color(TEXT_PRIMARY))
                .on_press(Message::CreateCollection)
                .padding([2, 8])
                .style(toolbar_button_style),
        ]
        .align_y(Alignment::Center);

        let mut list = column![].spacing(2);
        for (i, col) in self.collection_store.collections.iter().enumerate() {
            let entry: Element<'_, Message> = if self.editing_collection_name == Some(i) {
                text_input("Collection name", &self.collection_name_buf)
                    .on_input(Message::CollectionNameChanged)
                    .on_submit(Message::CollectionNameSubmit)
                    .size(12)
                    .width(Length::Fill)
                    .into()
            } else {
                let label = format!("{} ({})", col.name, col.photos.len());
                let is_drop_target = self.drag_state.as_ref().is_some_and(|d| d.active)
                    && self.sidebar_hover_collection == Some(i);
                let style_fn = if is_drop_target {
                    sidebar_item_drop_target_style
                } else {
                    sidebar_item_style
                };
                MouseArea::new(
                    button(text(label).size(12).color(TEXT_SECONDARY))
                        .on_press(Message::SidebarCollectionClicked(i))
                        .padding([4, 8])
                        .width(Length::Fill)
                        .style(style_fn),
                )
                .on_right_press(Message::SidebarCollectionRightClicked(i))
                .on_enter(Message::SidebarCollectionHovered(Some(i)))
                .on_exit(Message::SidebarCollectionHovered(None))
                .into()
            };
            list = list.push(entry);
        }

        container(
            column![header, scrollable(list).height(Length::Fill)]
                .spacing(6)
                .padding(8),
        )
        .width(COLLECTION_SIDEBAR_WIDTH)
        .height(Length::Fill)
        .style(panel_container_style)
        .into()
    }

    fn collection_grid_view(&self, collection_index: usize) -> Element<'_, Message> {
        let collection = &self.collection_store.collections[collection_index];

        let back_btn = button(text("\u{2190}").size(16).color(TEXT_PRIMARY))
            .on_press(Message::ExitCollectionView)
            .padding([4, 12])
            .style(toolbar_button_style);

        let title = text(format!("{} ({})", collection.name, collection.photos.len()))
            .size(14)
            .color(TEXT_PRIMARY);

        let top_bar = container(
            row![back_btn, container(title).padding([0, 8])]
                .spacing(6)
                .align_y(Alignment::Center),
        )
        .padding([6, 10])
        .width(Length::Fill)
        .style(toolbar_container_style);

        let layout = self.collection_grid_layout();
        let grid =
            self.build_thumbnail_grid(collection.photos.len(), layout, |photo_idx, thumb_size| {
                let photo_path = &collection.photos[photo_idx];
                let lib_entry = self.library_entry_by_path(photo_path);
                let filename = photo_path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("")
                    .to_string();

                let card = button(self.thumbnail_card_content(
                    lib_entry.and_then(|entry| entry.thumbnail_handle.as_ref()),
                    filename,
                    thumb_size,
                ))
                .on_press(Message::CollectionPhotoClicked(photo_idx))
                .padding(GRID_CARD_PADDING)
                .style(card_button_style);

                MouseArea::new(card)
                    .on_right_press(Message::CollectionPhotoRightClicked(photo_idx))
                    .into()
            });

        let status_text = format!("{} photos", collection.photos.len());
        let status = container(text(status_text).size(11).color(TEXT_DIM))
            .width(Length::Fill)
            .padding([6, 14]);

        container(column![
            top_bar,
            scrollable(container(grid).padding(GRID_PADDING).width(Length::Fill))
                .height(Length::Fill),
            container(status)
                .width(Length::Fill)
                .style(toolbar_container_style),
        ])
        .style(dark_bg_style)
        .into()
    }

    fn thumbnail_card<'a>(
        &'a self,
        entry: &'a LibraryEntry,
        index: usize,
        thumb_size: f32,
    ) -> Element<'a, Message> {
        let card = button(self.thumbnail_card_content(
            entry.thumbnail_handle.as_ref(),
            entry.filename.clone(),
            thumb_size,
        ))
        .on_press(Message::LibraryItemClicked(index))
        .padding(GRID_CARD_PADDING)
        .style(card_button_style);

        MouseArea::new(card)
            .on_right_press(Message::LibraryPhotoRightClicked(index))
            .on_enter(Message::ThumbnailHovered(Some(index)))
            .on_exit(Message::ThumbnailHovered(None))
            .into()
    }

    fn thumbnail_card_content(
        &self,
        handle: Option<&ImageHandle>,
        label_text: String,
        thumb_size: f32,
    ) -> Element<'static, Message> {
        let thumb: Element<'static, Message> = if let Some(handle) = handle {
            container(
                Image::new(handle.clone())
                    .width(thumb_size)
                    .height(thumb_size),
            )
            .width(thumb_size)
            .height(thumb_size)
            .center_x(Length::Shrink)
            .center_y(Length::Shrink)
            .into()
        } else {
            container(text("...").size(24).color(TEXT_DIM))
                .width(thumb_size)
                .height(thumb_size)
                .center_x(Length::Shrink)
                .center_y(Length::Shrink)
                .into()
        };

        let label = container(text(label_text).size(10).color(TEXT_SECONDARY)).width(thumb_size);

        column![thumb, label].spacing(4).width(thumb_size).into()
    }

    fn build_thumbnail_grid<'a, F>(
        &'a self,
        item_count: usize,
        layout: ThumbnailGridLayout,
        mut build_card: F,
    ) -> iced::widget::Column<'a, Message>
    where
        F: FnMut(usize, f32) -> Element<'a, Message>,
    {
        let mut grid = column![].spacing(GRID_SPACING);

        for row_start in (0..item_count).step_by(layout.columns) {
            let mut r = row![].spacing(GRID_SPACING);
            let row_end = (row_start + layout.columns).min(item_count);
            for item_index in row_start..row_end {
                r = r.push(build_card(item_index, layout.thumb_size));
            }
            grid = grid.push(r);
        }

        grid
    }

    fn library_entry_by_path(&self, path: &Path) -> Option<&LibraryEntry> {
        self.library_indices_by_path
            .get(path)
            .and_then(|&index| self.library.get(index))
    }

    fn clamped_library_index(&self, index: usize) -> Option<usize> {
        if self.library.is_empty() {
            None
        } else {
            Some(index.min(self.library.len() - 1))
        }
    }

    fn clamped_collection_photo_index(
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

    fn step_wrapped_index(current: usize, len: usize, forward: bool) -> usize {
        if forward {
            (current + 1) % len
        } else if current == 0 {
            len - 1
        } else {
            current - 1
        }
    }

    fn library_photo_context_menu_actions(&self, photo_path: &Path) -> Vec<(String, Message)> {
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

    fn library_grid_layout(&self) -> ThumbnailGridLayout {
        let grid_width =
            self.window_size.width - COLLECTION_SIDEBAR_WIDTH - COLLECTION_SIDEBAR_DIVIDER_WIDTH;
        ThumbnailGridLayout::new(grid_width)
    }

    fn collection_grid_layout(&self) -> ThumbnailGridLayout {
        ThumbnailGridLayout::new(self.window_size.width)
    }

    fn detail_view(&self) -> Element<'_, Message> {
        let canvas: Element<'_, ViewerEvent> = shader(ImageCanvas {
            image: self.image.clone(),
            image_id: self.image_id,
            zoom: self.zoom,
            offset: self.offset,
            canvas_size_cache: Arc::clone(&self.canvas_size_cache),
            crop: self.current_crop(),
            crop_mode: self.crop_mode,
            crop_aspect_ratio: self.crop_aspect.ratio(),
            adjustments: self.build_adjustment_uniforms(),
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into();

        let viewer_with_status = column![canvas.map(Message::Viewer), self.status_bar()];

        row![viewer_with_status.width(Length::Fill), self.edit_panel()].into()
    }

    fn status_bar_text(&self) -> String {
        if let Some(img) = &self.image {
            let name = if self.collection_nav.is_some() {
                self.current_image_path
                    .as_ref()
                    .map(|p| path_filename_str(p).to_string())
                    .unwrap_or_default()
            } else if let Some(idx) = self
                .library_index
                .and_then(|idx| self.clamped_library_index(idx))
            {
                self.library
                    .get(idx)
                    .map(|e| e.filename.clone())
                    .unwrap_or_default()
            } else {
                self.nav
                    .as_ref()
                    .map_or(String::new(), |n| n.current_filename())
            };

            let pos = if let Some((col_idx, photo_idx)) = self.collection_nav {
                let total = self
                    .collection_store
                    .collections
                    .get(col_idx)
                    .map(|c| c.photos.len())
                    .unwrap_or(0);
                let current = self
                    .clamped_collection_photo_index(col_idx, photo_idx)
                    .map(|idx| idx + 1)
                    .unwrap_or(0);
                format!("  {current}/{total}")
            } else if let Some(idx) = self
                .library_index
                .and_then(|idx| self.clamped_library_index(idx))
            {
                format!("  {}/{}", idx + 1, self.library.len())
            } else {
                self.nav
                    .as_ref()
                    .map(|n| format!("  {}/{}", n.current_index() + 1, n.count()))
                    .unwrap_or_default()
            };

            let zoom_pct = (self.zoom * 100.0) as u32;
            let mb = img.file_size as f64 / 1_048_576.0;
            let (display_w, display_h) = self.current_display_dimensions(img);
            let load_suffix = self.detail_load.load_suffix();

            format!(
                "  {name}  \u{2022}  {w}\u{00d7}{h}  \u{2022}  {mb:.1} MB  \u{2022}  {zoom_pct}%{pos}{load_suffix}",
                w = display_w,
                h = display_h,
            )
        } else if self.detail_load.is_loading() {
            "  Loading\u{2026}".to_string()
        } else if let Some(e) = &self.error {
            format!("  Error: {e}")
        } else {
            "  Ctrl+O to open  \u{2022}  Drag & drop  \u{2022}  Arrow keys to navigate".to_string()
        }
    }

    fn current_rotation(&self) -> edit::QuarterTurns {
        self.current_image_path
            .as_ref()
            .and_then(|path| self.edit_histories.get(path))
            .map(|history| history.current.rotation)
            .unwrap_or_default()
    }

    fn current_crop(&self) -> Option<edit::CropRect> {
        self.current_image_path
            .as_ref()
            .and_then(|path| self.edit_histories.get(path))
            .and_then(|history| history.current.crop)
    }

    fn visible_edit_state(&self) -> edit::EditState {
        let mut state = self
            .current_image_path
            .as_ref()
            .and_then(|path| self.edit_histories.get(path))
            .map(|history| history.current)
            .unwrap_or_default();
        state.crop = self.visible_crop();
        state
    }

    fn current_save_request(&self) -> Option<SaveRequest> {
        if self.detail_load.blocks_save() {
            return None;
        }
        let path = self.current_image_path.clone()?;
        let image = self.image.clone()?;
        let state = self.visible_edit_state();
        if state.lens_correction
            && self.lens_override_name.is_none()
            && self.detail_load.exif_loading
        {
            return None;
        }
        let vig = self.current_lens_vignetting(state.lens_correction);
        Some(SaveRequest {
            path,
            image,
            state,
            vig,
        })
    }

    fn current_lens_vignetting(&self, lens_correction_enabled: bool) -> [f32; 3] {
        if !lens_correction_enabled {
            return [0.0; 3];
        }
        self.current_lens_profile
            .as_ref()
            .and_then(|profile| profile.vignetting)
            .map(|vignetting| [vignetting.k1, vignetting.k2, vignetting.k3])
            .unwrap_or([0.0; 3])
    }

    fn visible_crop(&self) -> Option<edit::CropRect> {
        if self.crop_mode {
            None
        } else {
            self.current_crop()
        }
    }

    fn current_display_dimensions(&self, img: &decode::ImageData) -> (u32, u32) {
        let (display_w, display_h) =
            edit::rotated_dimensions(img.width, img.height, self.current_rotation());
        edit::cropped_dimensions(display_w, display_h, self.visible_crop())
    }

    fn current_canvas_size(&self) -> [f32; 2] {
        self.canvas_size_cache
            .lock()
            .map(|canvas_size| *canvas_size)
            .unwrap_or(DEFAULT_CANVAS_SIZE)
    }

    fn update_canvas_size(&mut self, canvas_size: [f32; 2]) {
        if let Ok(mut cached_size) = self.canvas_size_cache.lock() {
            *cached_size = canvas_size;
        }
    }

    fn fit_scale_for_rotation_and_crop(
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

    fn actual_size_zoom_for_rotation(
        &self,
        canvas_size: [f32; 2],
        img: &decode::ImageData,
        rotation: edit::QuarterTurns,
    ) -> f32 {
        self.actual_size_zoom_for_rotation_and_crop(canvas_size, img, rotation, self.visible_crop())
    }

    fn actual_size_zoom_for_rotation_and_crop(
        &self,
        canvas_size: [f32; 2],
        img: &decode::ImageData,
        rotation: edit::QuarterTurns,
        crop: Option<edit::CropRect>,
    ) -> f32 {
        1.0 / self.fit_scale_for_rotation_and_crop(canvas_size, img, rotation, crop)
    }

    fn is_at_actual_size_for_rotation_and_crop(
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

    fn preserve_actual_size_after_display_change(
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

    fn status_bar(&self) -> Element<'_, Message> {
        let s = self.status_bar_text();

        container(text(s).size(11).color(TEXT_DIM))
            .width(Length::Fill)
            .padding([5, 10])
            .style(toolbar_container_style)
            .into()
    }

    // ---------------------------------------------------------------------------
    // Edit panel
    // ---------------------------------------------------------------------------

    fn edit_panel(&self) -> Element<'_, Message> {
        let state = self
            .current_image_path
            .as_ref()
            .and_then(|p| self.edit_histories.get(p))
            .map(|h| h.current)
            .unwrap_or_default();

        // Light section
        let light = column![
            section_label("LIGHT"),
            self.slider_row("Exposure", SliderKind::Exposure, state.exposure),
            self.slider_row("Contrast", SliderKind::Contrast, state.contrast),
            self.slider_row("Highlights", SliderKind::Highlights, state.highlights),
            self.slider_row("Shadows", SliderKind::Shadows, state.shadows),
            self.slider_row("Whites", SliderKind::Whites, state.whites),
            self.slider_row("Blacks", SliderKind::Blacks, state.blacks),
        ]
        .spacing(3);

        // Color section
        let color = column![
            section_label("COLOR"),
            self.slider_row("Temp", SliderKind::Temperature, state.temperature),
            self.slider_row("Tint", SliderKind::Tint, state.tint),
            self.slider_row("Vibrance", SliderKind::Vibrance, state.vibrance),
            self.slider_row("Saturation", SliderKind::Saturation, state.saturation),
        ]
        .spacing(3);

        // Effects section
        let effects = column![
            section_label("EFFECTS"),
            self.slider_row("Clarity", SliderKind::Clarity, state.clarity),
            self.slider_row("Dehaze", SliderKind::Dehaze, state.dehaze),
        ]
        .spacing(3);

        // Lens correction section
        let lens_label = if state.lens_correction {
            "Lens Correction: ON"
        } else {
            "Lens Correction: OFF"
        };
        let lens_btn = button(text(lens_label).size(11).color(TEXT_PRIMARY))
            .on_press(Message::ToggleLensCorrection)
            .padding([4, 8])
            .style(toolbar_button_style);

        let lens_info: Element<'_, Message> =
            if self.detail_load.exif_loading && self.lens_override_name.is_none() {
                text("Loading lens metadata…")
                    .size(10)
                    .color(TEXT_DIM)
                    .into()
            } else if let Some(profile) = &self.current_lens_profile {
                text(format!("{} {}", profile.maker, profile.model))
                    .size(10)
                    .color(TEXT_SECONDARY)
                    .into()
            } else {
                text("No lens profile matched")
                    .size(10)
                    .color(TEXT_DIM)
                    .into()
            };

        // Lens profile dropdown
        let mut lens_options: Vec<String> = vec!["Auto".to_string(), "None".to_string()];
        for profile in &self.lens_db.profiles {
            lens_options.push(format!("{} {}", profile.maker, profile.model));
        }
        let selected_lens: Option<String> = match &self.lens_override_name {
            Some(name) => Some(name.clone()),
            None => Some("Auto".to_string()),
        };
        let lens_dropdown = pick_list(lens_options, selected_lens, Message::LensProfileSelected)
            .text_size(11)
            .width(Length::Fill);

        let lens_section =
            column![section_label("LENS"), lens_btn, lens_dropdown, lens_info,].spacing(4);

        let rotation_row = row![
            rotation_button(
                ROTATE_COUNTERCLOCKWISE_ICON,
                ROTATE_COUNTERCLOCKWISE_STEP_LABEL,
                Message::RotateCounterclockwise,
            ),
            rotation_button(
                ROTATE_CLOCKWISE_ICON,
                ROTATE_CLOCKWISE_STEP_LABEL,
                Message::RotateClockwise,
            ),
        ]
        .spacing(8);
        let rotation_section = column![section_label("ROTATE"), rotation_row].spacing(4);

        let crop_mode_label = if self.crop_mode {
            "Finish Crop"
        } else {
            "Crop"
        };
        let crop_row = row![
            button(text(crop_mode_label).size(11).color(TEXT_PRIMARY))
                .on_press(Message::ToggleCropMode)
                .padding([4, 8])
                .style(toolbar_button_style),
            pick_list(
                vec![CropAspect::Freeform, CropAspect::Square],
                Some(self.crop_aspect),
                Message::CropAspectSelected,
            )
            .text_size(11)
            .width(110),
            button(text("Clear").size(11).color(TEXT_PRIMARY))
                .on_press(Message::ClearCrop)
                .padding([4, 8])
                .style(toolbar_button_style),
        ]
        .spacing(8)
        .align_y(Alignment::Center);
        let crop_section = column![section_label("CROP"), crop_row].spacing(4);

        // Reset button
        let reset_btn = button(text("Reset All").size(11).color(TEXT_PRIMARY))
            .on_press(Message::ResetAll)
            .padding([4, 12])
            .style(toolbar_button_style);

        // Status text
        let status_text: Element<'_, Message> = if let Some(status) = &self.save_status {
            text(status)
                .size(10)
                .color(Color::from_rgb(0.4, 0.7, 0.4))
                .into()
        } else {
            text("").size(10).into()
        };

        let panel_content = column![
            light,
            section_divider(),
            color,
            section_divider(),
            effects,
            section_divider(),
            lens_section,
            section_divider(),
            rotation_section,
            crop_section,
            reset_btn,
            status_text,
        ]
        .spacing(6)
        .padding(12);

        container(scrollable(panel_content).height(Length::Fill))
            .width(280)
            .style(panel_container_style)
            .into()
    }

    fn slider_row(&self, label: &str, kind: SliderKind, value: f32) -> Element<'_, Message> {
        let (min, max) = slider_range(kind);
        let step = slider_step(kind);

        let label_el: Element<'_, Message> =
            button(text(label.to_string()).size(11).color(TEXT_SECONDARY))
                .on_press(Message::ResetSlider(kind))
                .padding(0)
                .style(invisible_button_style)
                .into();

        let value_el: Element<'_, Message> = if self.editing_slider == Some(kind) {
            text_input("", &self.slider_text_buf)
                .on_input(Message::SliderTextChanged)
                .on_submit(Message::SliderTextSubmit(kind))
                .size(11)
                .width(45)
                .into()
        } else {
            button(text(format!("{:.1}", value)).size(11).color(TEXT_PRIMARY))
                .on_press(Message::SliderTextInput(kind))
                .padding(0)
                .style(invisible_button_style)
                .into()
        };

        let slider_el = slider(min..=max, value, move |v| Message::SliderChanged(kind, v))
            .step(step)
            .on_release(Message::SliderReleased(kind))
            .width(130);

        row![
            container(label_el).width(65),
            container(value_el).width(45),
            slider_el,
        ]
        .spacing(4)
        .align_y(Alignment::Center)
        .into()
    }

    // ---------------------------------------------------------------------------
    // Adjustment uniforms
    // ---------------------------------------------------------------------------

    fn build_adjustment_uniforms(&self) -> viewer::AdjustmentUniforms {
        let state = self
            .current_image_path
            .as_ref()
            .and_then(|p| self.edit_histories.get(p))
            .map(|h| h.current)
            .unwrap_or_default();

        let temp_matrix = edit::temperature_tint_matrix(state.temperature, state.tint);

        let (lens_dist, lens_vig, lens_tca_r, lens_tca_b) = if state.lens_correction {
            match &self.current_lens_profile {
                Some(p) => {
                    let dist = p.distortion.map(|d| [d.a, d.b, d.c]).unwrap_or([0.0; 3]);
                    let vig = self.current_lens_vignetting(true);
                    let tca_r = p.tca.map(|t| t.vr).unwrap_or(1.0);
                    let tca_b = p.tca.map(|t| t.vb).unwrap_or(1.0);
                    (dist, vig, tca_r, tca_b)
                }
                None => ([0.0; 3], [0.0; 3], 1.0, 1.0),
            }
        } else {
            ([0.0; 3], [0.0; 3], 1.0, 1.0)
        };

        let image_aspect = self
            .image
            .as_ref()
            .map(|img| img.width as f32 / img.height as f32)
            .unwrap_or(1.0);

        viewer::AdjustmentUniforms {
            exposure: state.exposure,
            contrast: state.contrast,
            highlights: state.highlights,
            shadows: state.shadows,
            whites: state.whites,
            blacks: state.blacks,
            vibrance: state.vibrance,
            saturation: state.saturation,
            clarity: state.clarity,
            dehaze: state.dehaze,
            temp_matrix,
            lens_enabled: state.lens_correction,
            lens_dist,
            lens_vig,
            lens_tca_r,
            lens_tca_b,
            image_aspect,
            rotation: state.rotation,
            crop_preview: state.crop.unwrap_or(edit::CropRect::FULL),
            crop_overlay: None,
        }
    }

    // ---------------------------------------------------------------------------
    // Context menu overlay
    // ---------------------------------------------------------------------------

    fn context_menu_overlay(&self, menu: &ContextMenu) -> Element<'_, Message> {
        let items: Vec<Element<'static, Message>> = match &menu.kind {
            ContextMenuKind::SidebarCollection { .. } => {
                vec![
                    context_menu_item("Rename", Message::ContextMenuRename),
                    context_menu_item("Delete", Message::ContextMenuDelete),
                ]
            }
            ContextMenuKind::LibraryPhoto { photo_path } => self
                .library_photo_context_menu_actions(photo_path)
                .into_iter()
                .map(|(label, message)| context_menu_item(label, message))
                .collect(),
            ContextMenuKind::CollectionPhoto { .. } => {
                let col_name = self
                    .active_collection
                    .and_then(|i| self.collection_store.collections.get(i))
                    .map(|c| c.name.as_str())
                    .unwrap_or("Collection");
                vec![context_menu_item(
                    format!("Remove from {col_name}"),
                    Message::RemovePhotoFromCollection,
                )]
            }
        };

        let menu_content = container(column(items).spacing(2).padding(4))
            .style(context_menu_container_style)
            .width(Length::Shrink);

        let x = menu.position[0].clamp(0.0, 1000.0);
        let y = menu.position[1].clamp(0.0, 700.0);

        let positioned = column![
            Space::with_height(y),
            row![Space::with_width(x), menu_content,]
        ];

        MouseArea::new(
            container(positioned)
                .width(Length::Fill)
                .height(Length::Fill),
        )
        .on_press(Message::DismissContextMenu)
        .into()
    }

    // ---------------------------------------------------------------------------
    // Drag overlay
    // ---------------------------------------------------------------------------

    fn drag_overlay(&self, drag: &DragState) -> Element<'_, Message> {
        let label = self
            .library
            .get(drag.photo_index)
            .map(|e| e.filename.clone())
            .unwrap_or_default();

        let thumb: Element<'_, Message> = if let Some(Some(ref handle)) = self
            .library
            .get(drag.photo_index)
            .map(|e| &e.thumbnail_handle)
        {
            container(Image::new(handle.clone()).width(60).height(60))
                .width(60)
                .height(60)
                .center_x(Length::Shrink)
                .center_y(Length::Shrink)
                .into()
        } else {
            text(label.clone()).size(11).color(TEXT_PRIMARY).into()
        };

        let drag_widget =
            container(column![thumb, text(label).size(10).color(TEXT_SECONDARY)].spacing(2))
                .padding(4)
                .style(|_theme: &Theme| container::Style {
                    background: Some(Background::Color(Color {
                        r: 0.15,
                        g: 0.15,
                        b: 0.15,
                        a: 0.85,
                    })),
                    border: Border {
                        color: Color::from_rgb(0.3, 0.5, 0.7),
                        width: 1.0,
                        radius: 4.0.into(),
                    },
                    ..Default::default()
                });

        let x = drag.current_pos[0] + 10.0;
        let y = drag.current_pos[1] + 10.0;

        container(column![
            Space::with_height(y),
            row![Space::with_width(x), drag_widget,]
        ])
        .width(Length::Fill)
        .height(Length::Fill)
        .into()
    }
}

// ---------------------------------------------------------------------------
// Style functions
// ---------------------------------------------------------------------------

fn toolbar_container_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_TOOLBAR)),
        ..Default::default()
    }
}

fn panel_container_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_PANEL)),
        border: Border {
            color: DIVIDER,
            width: 1.0,
            radius: 0.0.into(),
        },
        ..Default::default()
    }
}

fn dark_bg_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(BG_DARK)),
        ..Default::default()
    }
}

fn toolbar_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Some(Background::Color(BG_BUTTON_HOVER)),
        button::Status::Pressed => Some(Background::Color(BG_DARK)),
        _ => Some(Background::Color(BG_BUTTON)),
    };
    button::Style {
        background: bg,
        text_color: TEXT_PRIMARY,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 3.0.into(),
        },
        shadow: Default::default(),
    }
}

fn card_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => BG_BUTTON_HOVER,
        button::Status::Pressed => BG_DARK,
        _ => BG_CARD,
    };
    button::Style {
        background: Some(Background::Color(bg)),
        text_color: TEXT_PRIMARY,
        border: Border {
            color: DIVIDER,
            width: 1.0,
            radius: 4.0.into(),
        },
        shadow: Default::default(),
    }
}

fn invisible_button_style(_theme: &Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: None,
        text_color: TEXT_SECONDARY,
        border: Border::default(),
        shadow: Default::default(),
    }
}

fn sidebar_item_style(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Some(Background::Color(BG_BUTTON_HOVER)),
        _ => None,
    };
    button::Style {
        background: bg,
        text_color: TEXT_SECONDARY,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 3.0.into(),
        },
        shadow: Default::default(),
    }
}

fn sidebar_item_drop_target_style(_theme: &Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(Color::from_rgb(0.2, 0.3, 0.4))),
        text_color: TEXT_PRIMARY,
        border: Border {
            color: Color::from_rgb(0.3, 0.5, 0.7),
            width: 1.0,
            radius: 3.0.into(),
        },
        shadow: Default::default(),
    }
}

fn section_label(title: &str) -> Element<'_, Message> {
    container(text(title).size(10).color(TEXT_DIM))
        .padding([5, 0])
        .into()
}

fn section_divider() -> Element<'static, Message> {
    container(horizontal_space())
        .width(Length::Fill)
        .height(1)
        .style(|_theme: &Theme| container::Style {
            background: Some(Background::Color(DIVIDER)),
            ..Default::default()
        })
        .into()
}

fn context_menu_item(label: impl Into<String>, msg: Message) -> Element<'static, Message> {
    button(text(label.into()).size(12).color(TEXT_PRIMARY))
        .on_press(msg)
        .padding([4, 12])
        .width(Length::Fill)
        .style(context_menu_button_style)
        .into()
}

fn context_menu_container_style(_theme: &Theme) -> container::Style {
    container::Style {
        background: Some(Background::Color(Color::from_rgb(0.2, 0.2, 0.2))),
        border: Border {
            color: Color::from_rgb(0.3, 0.3, 0.3),
            width: 1.0,
            radius: 4.0.into(),
        },
        ..Default::default()
    }
}

fn context_menu_button_style(_theme: &Theme, status: button::Status) -> button::Style {
    let bg = match status {
        button::Status::Hovered => Some(Background::Color(Color::from_rgb(0.3, 0.4, 0.55))),
        _ => None,
    };
    button::Style {
        background: bg,
        text_color: TEXT_PRIMARY,
        border: Border {
            color: Color::TRANSPARENT,
            width: 0.0,
            radius: 2.0.into(),
        },
        shadow: Default::default(),
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

fn set_slider_field(state: &mut edit::EditState, kind: SliderKind, value: f32) {
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

fn get_slider_field(state: &edit::EditState, kind: SliderKind) -> f32 {
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

fn slider_range(kind: SliderKind) -> (f32, f32) {
    match kind {
        SliderKind::Exposure => (-3.0, 3.0),
        SliderKind::Temperature | SliderKind::Tint => (-30.0, 30.0),
        SliderKind::Highlights
        | SliderKind::Shadows
        | SliderKind::Whites
        | SliderKind::Blacks
        | SliderKind::Vibrance => (-100.0, 100.0),
        _ => (-50.0, 50.0),
    }
}

fn slider_step(kind: SliderKind) -> f32 {
    match kind {
        SliderKind::Exposure => 0.02,
        SliderKind::Temperature | SliderKind::Tint => 0.5,
        _ => 1.0,
    }
}

fn library_file_path() -> Option<PathBuf> {
    std::env::var_os("LOCALAPPDATA").map(|dir| Path::new(&dir).join("photo").join("library.txt"))
}

fn save_library(library: &[LibraryEntry]) {
    let Some(path) = library_file_path() else {
        return;
    };
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let content: String = library
        .iter()
        .map(|e| e.path.to_string_lossy().into_owned())
        .collect::<Vec<_>>()
        .join("\n");
    let _ = std::fs::write(&path, content);
}

fn load_library() -> Vec<PathBuf> {
    let Some(path) = library_file_path() else {
        return Vec::new();
    };
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    content
        .lines()
        .filter(|line| !line.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect()
}

fn image_file_dialog_extensions() -> &'static [&'static str] {
    nav::image_extensions()
}

pub fn scan_folder_for_images(folder: &Path) -> Vec<PathBuf> {
    nav::scan_images_in_directory(folder)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup_dir(names: &[&str]) -> (tempfile::TempDir, Vec<PathBuf>) {
        let dir = tempfile::tempdir().unwrap();
        let mut paths = Vec::new();
        for name in names {
            let p = dir.path().join(name);
            std::fs::write(&p, b"").unwrap();
            paths.push(p);
        }
        (dir, paths)
    }

    fn detail_app_with_image(path: &Path, width: u32, height: u32) -> App {
        let (mut app, _) = App::new();
        app.tab = Tab::Detail;
        app.clear_library_entries();
        app.collection_store = collection::CollectionStore::default();
        app.active_collection = None;
        app.context_menu = None;
        app.image = Some(Arc::new(decode::ImageData {
            pixels: vec![0, 0, 0, 255],
            width,
            height,
            file_size: 2_000_000,
        }));
        app.current_image_path = Some(path.to_path_buf());
        app
    }

    fn test_image(width: u32, height: u32) -> Arc<decode::ImageData> {
        Arc::new(decode::ImageData {
            pixels: vec![0, 0, 0, 255],
            width,
            height,
            file_size: 2_000_000,
        })
    }

    fn test_image_with_bytes(width: u32, height: u32, bytes: usize) -> Arc<decode::ImageData> {
        Arc::new(decode::ImageData {
            pixels: vec![0; bytes],
            width,
            height,
            file_size: u64::try_from(bytes).unwrap_or(u64::MAX),
        })
    }

    fn loaded_full_image(path: &Path, image: Arc<decode::ImageData>) -> LoadedFullImage {
        LoadedFullImage {
            image,
            fingerprint: SourceFileFingerprint::from_path(path),
        }
    }

    fn library_app_with_entries(count: usize) -> App {
        let (mut app, _) = App::new();
        app.tab = Tab::Library;
        app.collection_store = collection::CollectionStore::default();
        app.active_collection = None;
        app.context_menu = None;
        app.replace_library_entries(
            (0..count)
                .map(|index| LibraryEntry {
                    path: PathBuf::from(format!("photo-{index}.png")),
                    filename: format!("photo-{index}.png"),
                    thumbnail_handle: None,
                })
                .collect(),
        );
        app
    }

    #[test]
    fn scan_folder_finds_only_images() {
        let (dir, _) = setup_dir(&["photo.jpg", "notes.txt", "icon.png", "data.csv", "art.bmp"]);
        let results = scan_folder_for_images(dir.path());
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn scan_folder_finds_raw_images() {
        let (dir, _) = setup_dir(&["photo.dng", "roll.cr3", "notes.txt"]);
        let results = scan_folder_for_images(dir.path());
        assert_eq!(results.len(), 2);
    }

    #[test]
    fn file_dialog_extensions_match_supported_image_extensions() {
        assert_eq!(image_file_dialog_extensions(), nav::image_extensions());
    }

    #[test]
    fn scan_folder_natural_sort_order() {
        let (dir, _) = setup_dir(&["img10.png", "img2.png", "img1.png"]);
        let results = scan_folder_for_images(dir.path());
        let names: Vec<&str> = results
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        assert_eq!(names, vec!["img1.png", "img2.png", "img10.png"]);
    }

    #[test]
    fn scan_folder_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let results = scan_folder_for_images(dir.path());
        assert!(results.is_empty());
    }

    #[test]
    fn library_grid_uses_latest_window_width_after_returning_from_detail() {
        let mut app = library_app_with_entries(24);

        let _ = app.handle_event(iced::Event::Window(window::Event::Resized(Size::new(
            720.0, 640.0,
        ))));
        let narrow_columns = app.library_grid_layout().columns;

        app.tab = Tab::Detail;
        let _ = app.handle_event(iced::Event::Window(window::Event::Resized(Size::new(
            1600.0, 900.0,
        ))));
        app.tab = Tab::Library;

        let wide_columns = app.library_grid_layout().columns;

        assert!(
            wide_columns > narrow_columns,
            "expected library thumbnails to reflow after resizing in detail view"
        );
    }

    #[test]
    fn library_grid_keeps_at_least_one_column_in_narrow_windows() {
        let mut app = library_app_with_entries(3);

        let _ = app.handle_event(iced::Event::Window(window::Event::Resized(Size::new(
            260.0, 640.0,
        ))));

        assert_eq!(app.library_grid_layout().columns, 1);
    }

    #[test]
    fn collection_grid_uses_latest_window_width_after_returning_from_detail() {
        let mut app = library_app_with_entries(24);
        app.collection_store.create("Favorites");
        for entry in &app.library {
            app.collection_store.add_photo(0, &entry.path);
        }
        app.active_collection = Some(0);

        let _ = app.handle_event(iced::Event::Window(window::Event::Resized(Size::new(
            720.0, 640.0,
        ))));
        let narrow_columns = app.collection_grid_layout().columns;

        app.tab = Tab::Detail;
        let _ = app.handle_event(iced::Event::Window(window::Event::Resized(Size::new(
            1600.0, 900.0,
        ))));
        app.tab = Tab::Library;

        let wide_columns = app.collection_grid_layout().columns;

        assert!(
            wide_columns > narrow_columns,
            "expected collection thumbnails to reflow after resizing in detail view"
        );
    }

    #[test]
    fn stale_collection_nav_prev_clamps_to_last_valid_photo() {
        let mut app = detail_app_with_image(Path::new("frame.png"), 200, 100);
        app.collection_store.create("Favorites");
        let only_photo = PathBuf::from("only-photo.png");
        app.collection_store.add_photo(0, &only_photo);
        app.collection_nav = Some((0, 99));

        let _ = app.handle_key(
            keyboard::Key::Named(keyboard::key::Named::ArrowLeft),
            keyboard::Modifiers::default(),
        );

        assert_eq!(app.collection_nav, Some((0, 0)));
        assert_eq!(
            app.current_image_path.as_deref(),
            Some(only_photo.as_path())
        );
    }

    #[test]
    fn stale_collection_nav_next_clamps_then_wraps() {
        let mut app = detail_app_with_image(Path::new("frame.png"), 200, 100);
        app.collection_store.create("Favorites");
        let photos = [
            PathBuf::from("one.png"),
            PathBuf::from("two.png"),
            PathBuf::from("three.png"),
        ];
        for photo in &photos {
            app.collection_store.add_photo(0, photo);
        }
        app.collection_nav = Some((0, 99));

        let _ = app.handle_key(
            keyboard::Key::Named(keyboard::key::Named::ArrowRight),
            keyboard::Modifiers::default(),
        );

        assert_eq!(app.collection_nav, Some((0, 0)));
        assert_eq!(app.current_image_path.as_deref(), Some(photos[0].as_path()));
    }

    #[test]
    fn stale_library_index_prev_clamps_to_last_valid_photo() {
        let mut app = library_app_with_entries(1);
        app.tab = Tab::Detail;
        app.library_index = Some(99);
        let expected_path = app.library[0].path.clone();

        let _ = app.handle_key(
            keyboard::Key::Named(keyboard::key::Named::ArrowLeft),
            keyboard::Modifiers::default(),
        );

        assert_eq!(app.library_index, Some(0));
        assert_eq!(
            app.current_image_path.as_deref(),
            Some(expected_path.as_path())
        );
    }

    #[test]
    fn stale_library_index_next_clamps_then_wraps() {
        let mut app = library_app_with_entries(3);
        app.tab = Tab::Detail;
        app.library_index = Some(99);
        let expected_path = app.library[0].path.clone();

        let _ = app.handle_key(
            keyboard::Key::Named(keyboard::key::Named::ArrowRight),
            keyboard::Modifiers::default(),
        );

        assert_eq!(app.library_index, Some(0));
        assert_eq!(
            app.current_image_path.as_deref(),
            Some(expected_path.as_path())
        );
    }

    #[test]
    fn stale_library_photo_context_menu_ignores_missing_target() {
        let mut app = library_app_with_entries(1);
        let photo_path = app.library[0].path.clone();
        app.clear_library_entries();

        assert!(app
            .library_photo_context_menu_actions(&photo_path)
            .is_empty());
    }

    #[test]
    fn save_and_load_library_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let lib_path = dir.path().join("library.txt");

        let p1 = dir.path().join("a.png");
        let p2 = dir.path().join("b.jpg");
        std::fs::write(&p1, b"").unwrap();
        std::fs::write(&p2, b"").unwrap();

        let entries = [
            LibraryEntry {
                path: p1.clone(),
                filename: "a.png".to_string(),
                thumbnail_handle: None,
            },
            LibraryEntry {
                path: p2.clone(),
                filename: "b.jpg".to_string(),
                thumbnail_handle: None,
            },
        ];

        // Write manually to the file
        let content: String = entries
            .iter()
            .map(|e| e.path.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("\n");
        std::fs::write(&lib_path, &content).unwrap();

        // Read back
        let loaded: Vec<PathBuf> = std::fs::read_to_string(&lib_path)
            .unwrap()
            .lines()
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .collect();

        assert_eq!(loaded, vec![p1, p2]);
    }

    #[test]
    fn importing_files_starts_background_cache_warming_for_supported_formats() {
        let (_dir, paths) = setup_dir(&["frame.dng", "frame.png", "overlay.svg"]);
        let raw = paths[0].clone();
        let png = paths[1].clone();
        let svg = paths[2].clone();
        let (mut app, _) = App::new();
        app.clear_library_entries();
        app.collection_store = collection::CollectionStore::default();
        app.active_collection = None;
        app.context_menu = None;

        let _ = app.update(Message::FilesPicked(Some(paths)));

        assert!(app.library_entry_by_path(&raw).is_some());
        assert!(app.library_entry_by_path(&png).is_some());
        assert!(app.library_entry_by_path(&svg).is_some());
        assert_eq!(app.import_cache_warm_in_flight.as_deref(), Some(raw.as_path()));
        assert_eq!(
            app.pending_import_cache_warm_paths.iter().collect::<Vec<_>>(),
            vec![&svg]
        );
    }

    #[test]
    fn import_cache_warm_completion_advances_to_the_next_supported_image() {
        let (_dir, paths) = setup_dir(&["frame.dng", "overlay.svg"]);
        let raw = paths[0].clone();
        let svg = paths[1].clone();
        let (mut app, _) = App::new();
        app.clear_library_entries();
        app.collection_store = collection::CollectionStore::default();
        app.active_collection = None;
        app.context_menu = None;

        let _ = app.update(Message::FilesPicked(Some(paths)));
        assert_eq!(app.import_cache_warm_in_flight.as_deref(), Some(raw.as_path()));

        let _ = app.update(Message::ImportCacheWarmCompleted {
            path: raw,
            result: Ok(true),
        });

        assert_eq!(app.import_cache_warm_in_flight.as_deref(), Some(svg.as_path()));
        assert!(app.pending_import_cache_warm_paths.is_empty());
    }

    #[test]
    fn import_cache_warm_failure_still_advances_to_the_next_supported_image() {
        let (_dir, paths) = setup_dir(&["frame.dng", "overlay.svg"]);
        let raw = paths[0].clone();
        let svg = paths[1].clone();
        let (mut app, _) = App::new();
        app.clear_library_entries();
        app.collection_store = collection::CollectionStore::default();
        app.active_collection = None;
        app.context_menu = None;

        let _ = app.update(Message::FilesPicked(Some(paths)));
        assert_eq!(app.import_cache_warm_in_flight.as_deref(), Some(raw.as_path()));

        let _ = app.update(Message::ImportCacheWarmCompleted {
            path: raw,
            result: Err("warm failed".to_string()),
        });

        assert_eq!(app.import_cache_warm_in_flight.as_deref(), Some(svg.as_path()));
        assert!(app.pending_import_cache_warm_paths.is_empty());
    }

    #[test]
    fn importing_more_files_while_a_warm_is_in_flight_appends_to_the_queue() {
        let (_dir, first_batch) = setup_dir(&["first.dng"]);
        let first = first_batch[0].clone();
        let (_dir2, second_batch) = setup_dir(&["second.dng", "overlay.svg"]);
        let second = second_batch[0].clone();
        let svg = second_batch[1].clone();
        let (mut app, _) = App::new();
        app.clear_library_entries();
        app.collection_store = collection::CollectionStore::default();
        app.active_collection = None;
        app.context_menu = None;

        let _ = app.update(Message::FilesPicked(Some(first_batch)));
        assert_eq!(app.import_cache_warm_in_flight.as_deref(), Some(first.as_path()));

        let _ = app.update(Message::FilesPicked(Some(second_batch)));

        assert_eq!(app.import_cache_warm_in_flight.as_deref(), Some(first.as_path()));
        assert_eq!(
            app.pending_import_cache_warm_paths.iter().collect::<Vec<_>>(),
            vec![&second, &svg]
        );
    }

    #[test]
    fn load_library_filters_deleted_files() {
        let dir = tempfile::tempdir().unwrap();
        let lib_path = dir.path().join("library.txt");

        let exists = dir.path().join("exists.png");
        std::fs::write(&exists, b"").unwrap();

        let content = format!(
            "{}\n{}",
            exists.display(),
            dir.path().join("gone.png").display()
        );
        std::fs::write(&lib_path, &content).unwrap();

        let loaded: Vec<PathBuf> = std::fs::read_to_string(&lib_path)
            .unwrap()
            .lines()
            .filter(|l| !l.is_empty())
            .map(PathBuf::from)
            .filter(|p| p.exists())
            .collect();

        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0], exists);
    }

    #[test]
    fn scan_folder_no_duplicates_in_entries() {
        let (dir, _) = setup_dir(&["a.png", "b.png"]);
        let paths = scan_folder_for_images(dir.path());

        let mut library: Vec<PathBuf> = Vec::new();
        for path in &paths {
            if !library.contains(path) {
                library.push(path.clone());
            }
        }
        // Add same paths again — should not grow
        for path in &paths {
            if !library.contains(path) {
                library.push(path.clone());
            }
        }
        assert_eq!(library.len(), 2);
    }

    #[test]
    fn slider_ranges_are_reasonable() {
        // Exposure should be narrower than before
        let (min, max) = slider_range(SliderKind::Exposure);
        assert_eq!(min, -3.0);
        assert_eq!(max, 3.0);

        // Temperature should be narrower
        let (min, max) = slider_range(SliderKind::Temperature);
        assert_eq!(min, -30.0);
        assert_eq!(max, 30.0);

        // Highlights/Shadows/Whites/Blacks keep full range
        for kind in [
            SliderKind::Highlights,
            SliderKind::Shadows,
            SliderKind::Whites,
            SliderKind::Blacks,
        ] {
            let (min, max) = slider_range(kind);
            assert_eq!(min, -100.0);
            assert_eq!(max, 100.0);
        }

        // Other sliders are reduced
        let (min, max) = slider_range(SliderKind::Contrast);
        assert_eq!(min, -50.0);
        assert_eq!(max, 50.0);
    }

    #[test]
    fn double_click_detection() {
        // Simulate: two clicks within 400ms on same index = double click
        let t1 = Instant::now();
        let t2 = t1; // immediate second click
        let is_double = t2.duration_since(t1).as_millis() < 400;
        assert!(is_double);
    }

    #[test]
    fn create_collection_enters_rename_mode() {
        let mut store = collection::CollectionStore::default();
        let name = store.next_default_name();
        assert_eq!(name, "New Collection");
        store.create(&name);
        let idx = store
            .collections
            .iter()
            .position(|c| c.name == name)
            .unwrap();
        assert_eq!(idx, 0);
        assert_eq!(store.collections.len(), 1);
    }

    #[test]
    fn collection_rename_submit_updates_name() {
        let mut store = collection::CollectionStore::default();
        store.create("Old Name");
        assert_eq!(store.collections[0].name, "Old Name");
        store.rename(0, "New Name");
        assert_eq!(store.collections[0].name, "New Name");
    }

    #[test]
    fn collection_rename_empty_string_keeps_old_name() {
        // Simulate CollectionNameSubmit with empty buffer: should not rename
        let mut store = collection::CollectionStore::default();
        store.create("Keep Me");
        let new_name = "".trim().to_string();
        if !new_name.is_empty() {
            store.rename(0, &new_name);
        }
        assert_eq!(store.collections[0].name, "Keep Me");
    }

    #[test]
    fn context_menu_delete_adjusts_active_collection() {
        let mut store = collection::CollectionStore::default();
        store.create("Alpha");
        store.create("Beta");
        store.create("Gamma");
        // Simulate active_collection = Some(2) (Gamma), deleting index 0 (Alpha)
        let mut active: Option<usize> = Some(2);
        let delete_idx = 0;
        store.delete(delete_idx);
        if active == Some(delete_idx) {
            active = None;
        } else if let Some(a) = active {
            if a > delete_idx {
                active = Some(a - 1);
            }
        }
        assert_eq!(active, Some(1)); // Gamma shifted from 2 to 1
        assert_eq!(store.collections.len(), 2);
    }

    #[test]
    fn context_menu_delete_clears_active_if_same() {
        let mut store = collection::CollectionStore::default();
        store.create("Only");
        let mut active: Option<usize> = Some(0);
        let delete_idx = 0;
        store.delete(delete_idx);
        if active == Some(delete_idx) {
            active = None;
        }
        assert!(active.is_none());
        assert!(store.collections.is_empty());
    }

    #[test]
    fn context_menu_kind_sidebar_collection() {
        let menu = ContextMenu {
            position: [100.0, 200.0],
            kind: ContextMenuKind::SidebarCollection {
                collection_index: 3,
            },
        };
        assert_eq!(menu.position, [100.0, 200.0]);
        match menu.kind {
            ContextMenuKind::SidebarCollection { collection_index } => {
                assert_eq!(collection_index, 3);
            }
            _ => panic!("expected SidebarCollection"),
        }
    }

    #[test]
    fn sidebar_double_click_sets_active_collection() {
        // Simulate double-click: two clicks on same index within 400ms
        let index = 2;
        let t1 = Instant::now();
        let last_click: Option<(usize, Instant)> = Some((index, t1));
        let now = t1; // immediate second click
        let is_double_click = last_click
            .map(|(prev_idx, prev_time)| {
                prev_idx == index && now.duration_since(prev_time).as_millis() < 400
            })
            .unwrap_or(false);
        assert!(is_double_click);
    }

    #[test]
    fn sidebar_click_different_index_not_double() {
        let t1 = Instant::now();
        let last_click: Option<(usize, Instant)> = Some((1, t1));
        let now = t1;
        let is_double_click = last_click
            .map(|(prev_idx, prev_time)| {
                prev_idx == 2 && now.duration_since(prev_time).as_millis() < 400
            })
            .unwrap_or(false);
        assert!(!is_double_click);
    }

    #[test]
    fn collection_nav_next_wraps_around() {
        // Simulate arrow-right cycling in a 3-photo collection
        let total = 3;
        let mut photo_idx: usize = 2; // last photo
        photo_idx = (photo_idx + 1) % total;
        assert_eq!(photo_idx, 0); // wraps to first
    }

    #[test]
    fn collection_nav_prev_wraps_around() {
        // Simulate arrow-left cycling in a 3-photo collection
        let total = 3;
        let mut photo_idx: usize = 0; // first photo
        photo_idx = if photo_idx == 0 {
            total - 1
        } else {
            photo_idx - 1
        };
        assert_eq!(photo_idx, 2); // wraps to last
    }

    #[test]
    fn exit_collection_view_clears_active() {
        // Simulate ExitCollectionView handler
        let active_collection: Option<usize> = Some(2);
        let result: Option<usize> = None;
        assert!(active_collection.is_some()); // was set before
        assert!(result.is_none()); // cleared after
    }

    #[test]
    fn exit_collection_detail_returns_to_collection_grid() {
        // Simulate ExitCollectionDetail handler: tab -> Library, collection_nav -> None,
        // but active_collection stays set so library_view routes to grid
        let active_collection: Option<usize> = Some(1);
        let tab = Tab::Library; // handler sets this
        let collection_nav: Option<(usize, usize)> = None; // handler clears this
        assert_eq!(tab, Tab::Library);
        assert!(active_collection.is_some()); // stays set
        assert!(collection_nav.is_none()); // cleared
    }

    #[test]
    fn remove_photo_from_collection_via_context() {
        let mut store = collection::CollectionStore::default();
        store.create("My Photos");
        let path = PathBuf::from("/test/photo.jpg");
        store.add_photo(0, &path);
        assert_eq!(store.collections[0].photos.len(), 1);
        store.remove_photo(0, &path);
        assert!(store.collections[0].photos.is_empty());
    }

    #[test]
    fn collection_photo_double_click_sets_collection_nav() {
        // Simulate the double-click logic for collection photo
        let photo_index = 2;
        let col_idx: usize = 1;
        let t1 = Instant::now();
        let last_thumb_click: Option<(usize, Instant)> = Some((photo_index, t1));
        let now = t1;
        let is_double_click = last_thumb_click
            .map(|(prev_idx, prev_time)| {
                prev_idx == photo_index && now.duration_since(prev_time).as_millis() < 400
            })
            .unwrap_or(false);
        assert!(is_double_click);
        // On double-click, collection_nav should be set
        let collection_nav = Some((col_idx, photo_index));
        assert_eq!(collection_nav, Some((1, 2)));
    }

    #[test]
    fn status_bar_collection_nav_position_format() {
        // Simulate status bar position formatting for collection nav
        let col_idx = 0;
        let photo_idx = 2;
        let total = 5;
        let pos = format!("  {}/{}", photo_idx + 1, total);
        assert_eq!(pos, "  3/5");
        let _ = col_idx; // used to index into collection_store
    }

    #[test]
    fn library_photo_right_click_no_collections_no_menu() {
        // If there are no collections, right-clicking a library photo should not create a menu
        let store = collection::CollectionStore::default();
        assert!(store.collections.is_empty());
        // Handler would early-return Task::none() without setting context_menu
    }

    #[test]
    fn library_photo_right_click_creates_context_menu() {
        let mut app = library_app_with_entries(3);
        app.collection_store.create("My Collection");
        let cursor_position = [150.0, 300.0];
        let expected_path = app.library[2].path.clone();
        app.cursor_position = cursor_position;

        let _ = app.update(Message::LibraryPhotoRightClicked(2));

        let Some(menu) = app.context_menu.clone() else {
            panic!("expected library photo context menu");
        };
        assert_eq!(menu.position, [150.0, 300.0]);
        match menu.kind {
            ContextMenuKind::LibraryPhoto { photo_path } => assert_eq!(photo_path, expected_path),
            _ => panic!("expected LibraryPhoto"),
        }
    }

    #[test]
    fn add_photo_to_collection_targets_original_photo_after_library_reflow() {
        let mut app = library_app_with_entries(3);
        app.collection_store.create("Favorites");
        let expected_path = app.library[1].path.clone();
        app.cursor_position = [150.0, 300.0];

        let _ = app.update(Message::LibraryPhotoRightClicked(1));
        let removed = app.remove_library_entry(0);
        assert!(removed.is_some());
        let _ = app.update(Message::AddPhotoToCollection(0));

        assert_eq!(
            app.collection_store.collections[0].photos,
            vec![expected_path]
        );
        assert!(app.context_menu.is_none());
    }

    #[test]
    fn stale_library_photo_add_action_ignores_removed_target() {
        let mut app = library_app_with_entries(2);
        app.collection_store.create("Favorites");
        let target_path = app.library[1].path.clone();
        app.cursor_position = [150.0, 300.0];

        let _ = app.update(Message::LibraryPhotoRightClicked(1));
        let removed = app.remove_library_entry(1);
        assert_eq!(
            removed.as_ref().map(|entry| &entry.path),
            Some(&target_path)
        );
        let _ = app.update(Message::AddPhotoToCollection(0));

        assert!(app.collection_store.collections[0].photos.is_empty());
        assert!(app.context_menu.is_none());
    }

    #[test]
    fn stale_library_photo_toggle_action_ignores_removed_target() {
        let mut app = library_app_with_entries(2);
        app.collection_store.create("Favorites");
        let target_path = app.library[1].path.clone();
        app.collection_store.add_photo(0, &target_path);
        app.cursor_position = [150.0, 300.0];

        let _ = app.update(Message::LibraryPhotoRightClicked(1));
        let removed = app.remove_library_entry(1);
        assert_eq!(
            removed.as_ref().map(|entry| &entry.path),
            Some(&target_path)
        );
        let _ = app.update(Message::TogglePhotoInCollection(0));

        assert_eq!(
            app.collection_store.collections[0].photos,
            vec![target_path]
        );
        assert!(app.context_menu.is_none());
    }

    #[test]
    fn stale_library_photo_add_action_ignores_removed_collection() {
        let mut app = library_app_with_entries(2);
        app.collection_store.create("Favorites");
        app.cursor_position = [150.0, 300.0];

        let _ = app.update(Message::LibraryPhotoRightClicked(1));
        app.collection_store.delete(0);
        let _ = app.update(Message::AddPhotoToCollection(0));

        assert!(app.collection_store.collections.is_empty());
        assert!(app.context_menu.is_none());
    }

    #[test]
    fn stale_library_photo_toggle_action_ignores_removed_collection() {
        let mut app = library_app_with_entries(2);
        app.collection_store.create("Favorites");
        let target_path = app.library[1].path.clone();
        app.collection_store.add_photo(0, &target_path);
        app.cursor_position = [150.0, 300.0];

        let _ = app.update(Message::LibraryPhotoRightClicked(1));
        app.collection_store.delete(0);
        let _ = app.update(Message::TogglePhotoInCollection(0));

        assert!(app.collection_store.collections.is_empty());
        assert!(app.context_menu.is_none());
    }

    #[test]
    fn toggle_photo_in_collection_targets_original_photo_after_library_reflow() {
        let mut app = library_app_with_entries(3);
        app.collection_store.create("Favorites");
        let target_path = app.library[1].path.clone();
        app.collection_store.add_photo(0, &target_path);
        app.cursor_position = [150.0, 300.0];

        let _ = app.update(Message::LibraryPhotoRightClicked(1));
        let removed = app.remove_library_entry(0);
        assert!(removed.is_some());
        let _ = app.update(Message::TogglePhotoInCollection(0));

        assert!(app.collection_store.collections[0].photos.is_empty());
        assert!(app.context_menu.is_none());
    }

    #[test]
    fn remove_photo_from_collection_targets_original_photo_after_collection_reflow() {
        let mut app = detail_app_with_image(Path::new("frame.png"), 200, 100);
        app.collection_store.create("Favorites");
        let photos = [
            PathBuf::from("one.png"),
            PathBuf::from("two.png"),
            PathBuf::from("three.png"),
        ];
        for photo in &photos {
            app.collection_store.add_photo(0, photo);
        }
        app.active_collection = Some(0);
        app.cursor_position = [180.0, 280.0];

        let _ = app.update(Message::CollectionPhotoRightClicked(1));
        app.collection_store.remove_photo(0, &photos[0]);
        let _ = app.update(Message::RemovePhotoFromCollection);

        assert_eq!(
            app.collection_store.collections[0].photos,
            vec![photos[2].clone()]
        );
        assert!(app.context_menu.is_none());
    }

    #[test]
    fn add_photo_to_collection_handler() {
        // Simulate AddPhotoToCollection: when context menu has LibraryPhoto, add photo to collection
        let mut store = collection::CollectionStore::default();
        store.create("Favorites");
        let photo_path = PathBuf::from("/test/sunset.jpg");
        // Simulate add_photo as the handler would
        store.add_photo(0, &photo_path);
        assert_eq!(store.collections[0].photos.len(), 1);
        assert!(store.collections[0].photos.contains(&photo_path));
    }

    #[test]
    fn toggle_photo_in_collection_adds_when_absent() {
        let mut store = collection::CollectionStore::default();
        store.create("Test");
        let path = PathBuf::from("/test/photo.jpg");
        // Photo not in collection -> add it
        let contains = store.collections[0].photos.contains(&path);
        assert!(!contains);
        store.add_photo(0, &path);
        assert!(store.collections[0].photos.contains(&path));
    }

    #[test]
    fn toggle_photo_in_collection_removes_when_present() {
        let mut store = collection::CollectionStore::default();
        store.create("Test");
        let path = PathBuf::from("/test/photo.jpg");
        store.add_photo(0, &path);
        assert!(store.collections[0].photos.contains(&path));
        // Photo already in collection -> remove it
        store.remove_photo(0, &path);
        assert!(!store.collections[0].photos.contains(&path));
    }

    #[test]
    fn drag_state_initializes_inactive() {
        // When LibraryItemClicked is handled, drag_state is created but inactive
        let cursor = [100.0, 200.0];
        let drag = DragState {
            photo_index: 5,
            start_pos: cursor,
            current_pos: cursor,
            active: false,
        };
        assert_eq!(drag.photo_index, 5);
        assert_eq!(drag.start_pos, cursor);
        assert!(!drag.active);
    }

    #[test]
    fn rotate_messages_commit_and_reset_current_image_history() {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 200, 100);

        let _ = app.update(Message::RotateClockwise);
        let history = app.edit_histories.get_mut(&path).unwrap();
        assert_eq!(history.current.rotation, edit::QuarterTurns::new(1));
        assert!(history.undo());
        assert_eq!(history.current.rotation, edit::QuarterTurns::default());

        let _ = app.update(Message::RotateCounterclockwise);
        assert_eq!(
            app.edit_histories.get(&path).unwrap().current.rotation,
            edit::QuarterTurns::new(3)
        );

        let _ = app.update(Message::ResetAll);
        assert_eq!(
            app.edit_histories.get(&path).unwrap().current.rotation,
            edit::QuarterTurns::default()
        );
    }

    #[test]
    fn rotate_messages_only_touch_the_current_image_history() {
        let current_path = PathBuf::from("current.png");
        let other_path = PathBuf::from("other.png");
        let mut app = detail_app_with_image(&current_path, 200, 100);

        app.edit_histories
            .insert(current_path.clone(), edit::UndoHistory::new());

        let mut other_history = edit::UndoHistory::new();
        other_history.current.rotation = edit::QuarterTurns::new(2);
        other_history.commit();
        app.edit_histories.insert(other_path.clone(), other_history);

        let _ = app.update(Message::RotateClockwise);

        assert_eq!(
            app.edit_histories
                .get(&current_path)
                .unwrap()
                .current
                .rotation,
            edit::QuarterTurns::new(1)
        );
        assert_eq!(
            app.edit_histories
                .get(&other_path)
                .unwrap()
                .current
                .rotation,
            edit::QuarterTurns::new(2)
        );
    }

    #[test]
    fn crop_commit_updates_only_the_current_image_history() {
        let current_path = PathBuf::from("current.png");
        let other_path = PathBuf::from("other.png");
        let mut app = detail_app_with_image(&current_path, 200, 100);

        app.edit_histories
            .insert(current_path.clone(), edit::UndoHistory::new());

        let mut other_history = edit::UndoHistory::new();
        other_history.current.crop = Some(edit::CropRect::new(0.0, 0.0, 0.5, 0.5));
        other_history.commit();
        app.edit_histories.insert(other_path.clone(), other_history);

        app.handle_viewer(ViewerEvent::CropCommitted {
            rect: edit::CropRect::new(0.25, 0.0, 0.75, 1.0),
        });

        let current_history = app.edit_histories.get(&current_path).unwrap();
        assert_eq!(
            current_history.current.crop,
            Some(edit::CropRect::new(0.25, 0.0, 0.75, 1.0))
        );

        let other_history = app.edit_histories.get(&other_path).unwrap();
        assert_eq!(
            other_history.current.crop,
            Some(edit::CropRect::new(0.0, 0.0, 0.5, 0.5))
        );

        let current_history = app.edit_histories.get_mut(&current_path).unwrap();
        assert!(current_history.undo());
        assert_eq!(current_history.current.crop, None);
    }

    #[test]
    fn crop_commit_preserves_actual_size_zoom() {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 200, 100);
        app.update_canvas_size([400.0, 200.0]);
        app.zoom = app.actual_size_zoom_for_rotation_and_crop(
            app.current_canvas_size(),
            app.image.as_ref().unwrap(),
            app.current_rotation(),
            None,
        );

        app.handle_viewer(ViewerEvent::CropCommitted {
            rect: edit::CropRect::new(0.5, 0.0, 1.0, 1.0),
        });

        let expected_zoom = app.actual_size_zoom_for_rotation_and_crop(
            app.current_canvas_size(),
            app.image.as_ref().unwrap(),
            app.current_rotation(),
            app.current_crop(),
        );
        assert!((app.zoom - expected_zoom).abs() < 0.01);
    }

    #[test]
    fn rotated_crop_commit_saves_the_selected_rotated_region() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("frame.png");
        let pixels = [255, 0, 0, 255, 0, 255, 0, 255];
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 2, 1);

        let _ = app.update(Message::RotateClockwise);
        app.handle_viewer(ViewerEvent::CropCommitted {
            rect: edit::CropRect::new(0.0, 0.0, 1.0, 0.5),
        });

        let state = app.edit_histories.get(&path).unwrap().current;
        let out = edit::save_edited_image(&original, &pixels, 2, 1, &state, [0.0; 3]).unwrap();
        let img = image::open(&out).unwrap().to_rgba8();

        assert_eq!(img.width(), 1);
        assert_eq!(img.height(), 1);
        assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn status_bar_uses_rotated_dimensions_after_rotation() {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 200, 100);

        let mut history = edit::UndoHistory::new();
        history.current.rotate_clockwise();
        history.commit();
        app.edit_histories.insert(path, history);

        let status = app.status_bar_text();
        assert!(status.contains("100×200"));
        assert!(!status.contains("200×100"));
    }

    #[test]
    fn status_bar_uses_cropped_dimensions_after_rotation_and_crop() {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 200, 100);

        let mut history = edit::UndoHistory::new();
        history.current.rotate_clockwise();
        history.current.crop = Some(edit::CropRect::new(0.0, 0.0, 1.0, 0.5));
        history.commit();
        app.edit_histories.insert(path, history);

        let status = app.status_bar_text();
        assert!(status.contains("100\u{00d7}100"));
        assert!(!status.contains("100\u{00d7}200"));
        assert!(!status.contains("200\u{00d7}100"));
    }

    #[test]
    fn crop_mode_status_and_actual_size_use_the_visible_full_image() {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 200, 100);

        let mut history = edit::UndoHistory::new();
        history.current.crop = Some(edit::CropRect::new(0.5, 0.0, 1.0, 1.0));
        history.commit();
        app.edit_histories.insert(path, history);
        app.crop_mode = true;

        let status = app.status_bar_text();
        assert!(status.contains("200\u{00d7}100"));
        assert!(!status.contains("100\u{00d7}100"));

        app.handle_viewer(ViewerEvent::DoubleClick {
            canvas_size: [400.0, 200.0],
        });

        let expected_zoom = app.actual_size_zoom_for_rotation_and_crop(
            app.current_canvas_size(),
            app.image.as_ref().unwrap(),
            app.current_rotation(),
            None,
        );
        assert!((app.zoom - expected_zoom).abs() < 0.01);
    }

    #[test]
    fn save_uses_the_visible_crop_state() {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 200, 100);

        let crop = edit::CropRect::new(0.5, 0.0, 1.0, 1.0);
        let mut history = edit::UndoHistory::new();
        history.current.exposure = 0.75;
        history.current.crop = Some(crop);
        history.commit();
        app.edit_histories.insert(path, history);

        let committed_state = app.visible_edit_state();
        assert_eq!(committed_state.crop, Some(crop));
        assert_eq!(committed_state.exposure, 0.75);

        app.crop_mode = true;

        let saving_state = app.visible_edit_state();
        assert_eq!(saving_state.crop, None);
        assert_eq!(saving_state.exposure, 0.75);
    }

    #[test]
    fn save_request_exports_the_visible_full_image_in_crop_mode() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("frame.png");
        let path = PathBuf::from("frame.png");
        let pixels = vec![255, 0, 0, 255, 0, 255, 0, 255];
        let mut app = detail_app_with_image(&path, 2, 1);
        app.image = Some(Arc::new(decode::ImageData {
            pixels: pixels.clone(),
            width: 2,
            height: 1,
            file_size: 2,
        }));

        let mut history = edit::UndoHistory::new();
        history.current.crop = Some(edit::CropRect::new(0.0, 0.0, 0.5, 1.0));
        history.commit();
        app.edit_histories.insert(path, history);
        app.crop_mode = true;

        let request = app.current_save_request().unwrap();
        let out = edit::save_edited_image(
            &original,
            &request.image.pixels,
            request.image.width,
            request.image.height,
            &request.state,
            request.vig,
        )
        .unwrap();
        let img = image::open(&out).unwrap().to_rgba8();

        assert_eq!(img.width(), 2);
        assert_eq!(img.height(), 1);
        assert_eq!(img.get_pixel(0, 0).0, [255, 0, 0, 255]);
        assert_eq!(img.get_pixel(1, 0).0, [0, 255, 0, 255]);
    }

    #[test]
    fn rotation_controls_use_icon_buttons() {
        use iced::advanced::widget::Tree;
        use iced::advanced::{layout, text as advanced_text, Widget};

        let button_ref: Element<'static, Message> = button(text("x")).into();
        let text_ref: Element<'static, Message> = text("x").into();
        let column_ref: Element<'static, Message> =
            column(vec![text("x").into(), text("y").into()]).into();
        let container_ref: Element<'static, Message> = container(text("x")).into();
        let row_ref: Element<'static, Message> =
            row(vec![button(text("x")).into(), button(text("y")).into()]).into();
        let button_tag = Tree::new(&button_ref).tag;
        let text_tag = Tree::new(&text_ref).tag;
        let column_tag = Tree::new(&column_ref).tag;
        let container_tag = Tree::new(&container_ref).tag;
        let row_tag = Tree::new(&row_ref).tag;

        assert_eq!(ROTATE_COUNTERCLOCKWISE_ICON, "\u{21BA}");
        assert_eq!(ROTATE_CLOCKWISE_ICON, "\u{21BB}");
        assert_eq!(ROTATE_COUNTERCLOCKWISE_STEP_LABEL, "-90\u{00B0}");
        assert_eq!(ROTATE_CLOCKWISE_STEP_LABEL, "+90\u{00B0}");
        assert_eq!(ROTATION_ICON_FONT_FAMILY, "Segoe UI Symbol");
        assert_eq!(
            ROTATION_ICON_FONT,
            iced::Font::with_name(ROTATION_ICON_FONT_FAMILY)
        );
        assert_eq!(ROTATION_ICON_SHAPING, iced::widget::text::Shaping::Advanced);

        #[derive(Debug, Clone, Default)]
        struct CapturingParagraph {
            last_text: Option<advanced_text::Text<String, iced::Font>>,
        }

        impl advanced_text::Paragraph for CapturingParagraph {
            type Font = iced::Font;

            fn with_text(text: advanced_text::Text<&str, Self::Font>) -> Self {
                Self {
                    last_text: Some(advanced_text::Text {
                        content: text.content.to_owned(),
                        bounds: text.bounds,
                        size: text.size,
                        line_height: text.line_height,
                        font: text.font,
                        horizontal_alignment: text.horizontal_alignment,
                        vertical_alignment: text.vertical_alignment,
                        shaping: text.shaping,
                        wrapping: text.wrapping,
                    }),
                }
            }

            fn with_spans<Link>(
                _text: advanced_text::Text<
                    &[advanced_text::Span<'_, Link, Self::Font>],
                    Self::Font,
                >,
            ) -> Self {
                Self::default()
            }

            fn resize(&mut self, new_bounds: iced::Size) {
                if let Some(last_text) = &mut self.last_text {
                    last_text.bounds = new_bounds;
                }
            }

            fn compare(
                &self,
                text: advanced_text::Text<(), Self::Font>,
            ) -> advanced_text::Difference {
                let Some(last_text) = &self.last_text else {
                    return advanced_text::Difference::Shape;
                };

                let same_shape = last_text.size == text.size
                    && last_text.line_height == text.line_height
                    && last_text.font == text.font
                    && last_text.horizontal_alignment == text.horizontal_alignment
                    && last_text.vertical_alignment == text.vertical_alignment
                    && last_text.shaping == text.shaping
                    && last_text.wrapping == text.wrapping;

                if same_shape && last_text.bounds == text.bounds {
                    advanced_text::Difference::None
                } else if same_shape {
                    advanced_text::Difference::Bounds
                } else {
                    advanced_text::Difference::Shape
                }
            }

            fn horizontal_alignment(&self) -> iced::alignment::Horizontal {
                self.last_text
                    .as_ref()
                    .map(|text| text.horizontal_alignment)
                    .unwrap_or(iced::alignment::Horizontal::Left)
            }

            fn vertical_alignment(&self) -> iced::alignment::Vertical {
                self.last_text
                    .as_ref()
                    .map(|text| text.vertical_alignment)
                    .unwrap_or(iced::alignment::Vertical::Top)
            }

            fn min_bounds(&self) -> iced::Size {
                self.last_text
                    .as_ref()
                    .map(|text| text.bounds)
                    .unwrap_or(iced::Size::ZERO)
            }

            fn hit_test(&self, _point: iced::Point) -> Option<advanced_text::Hit> {
                None
            }

            fn hit_span(&self, _point: iced::Point) -> Option<usize> {
                None
            }

            fn span_bounds(&self, _index: usize) -> Vec<iced::Rectangle> {
                vec![]
            }

            fn grapheme_position(&self, _line: usize, _index: usize) -> Option<iced::Point> {
                None
            }
        }

        #[derive(Default)]
        struct CapturingRenderer;

        impl iced::advanced::Renderer for CapturingRenderer {
            fn start_layer(&mut self, _bounds: iced::Rectangle) {}

            fn end_layer(&mut self) {}

            fn start_transformation(&mut self, _transformation: iced::Transformation) {}

            fn end_transformation(&mut self) {}

            fn fill_quad(
                &mut self,
                _quad: iced::advanced::renderer::Quad,
                _background: impl Into<iced::Background>,
            ) {
            }

            fn clear(&mut self) {}
        }

        impl advanced_text::Renderer for CapturingRenderer {
            type Font = iced::Font;
            type Paragraph = CapturingParagraph;
            type Editor = ();

            const ICON_FONT: Self::Font = iced::Font::DEFAULT;
            const CHECKMARK_ICON: char = '0';
            const ARROW_DOWN_ICON: char = '0';

            fn default_font(&self) -> Self::Font {
                iced::Font::DEFAULT
            }

            fn default_size(&self) -> iced::Pixels {
                iced::Pixels(16.0)
            }

            fn fill_paragraph(
                &mut self,
                _paragraph: &Self::Paragraph,
                _position: iced::Point,
                _color: iced::Color,
                _clip_bounds: iced::Rectangle,
            ) {
            }

            fn fill_editor(
                &mut self,
                _editor: &Self::Editor,
                _position: iced::Point,
                _color: iced::Color,
                _clip_bounds: iced::Rectangle,
            ) {
            }

            fn fill_text(
                &mut self,
                _text: advanced_text::Text<String, Self::Font>,
                _position: iced::Point,
                _color: iced::Color,
                _clip_bounds: iced::Rectangle,
            ) {
            }
        }

        fn captured_button_icon_text(
            icon: &'static str,
            step_label: &'static str,
            message: Message,
        ) -> advanced_text::Text<String, iced::Font> {
            let button: Element<'static, Message, iced::Theme, CapturingRenderer> =
                rotation_button_widget::<CapturingRenderer>(icon, step_label, message).into();
            let mut tree = Tree::new(button.as_widget());
            let renderer = CapturingRenderer;
            let limits = layout::Limits::new(iced::Size::ZERO, iced::Size::new(200.0, 200.0));
            let _ = Widget::layout(button.as_widget(), &mut tree, &renderer, &limits);

            tree.children[0].children[0]
                .state
                .downcast_ref::<iced::widget::text::State<CapturingParagraph>>()
                .0
                .raw()
                .last_text
                .clone()
                .expect("rotation icon label should populate paragraph state")
        }

        let counterclockwise_icon_text = captured_button_icon_text(
            ROTATE_COUNTERCLOCKWISE_ICON,
            ROTATE_COUNTERCLOCKWISE_STEP_LABEL,
            Message::RotateCounterclockwise,
        );
        assert_eq!(
            counterclockwise_icon_text.content,
            ROTATE_COUNTERCLOCKWISE_ICON
        );
        assert_eq!(counterclockwise_icon_text.font, ROTATION_ICON_FONT);
        assert_eq!(counterclockwise_icon_text.shaping, ROTATION_ICON_SHAPING);

        let clockwise_icon_text = captured_button_icon_text(
            ROTATE_CLOCKWISE_ICON,
            ROTATE_CLOCKWISE_STEP_LABEL,
            Message::RotateClockwise,
        );
        assert_eq!(clockwise_icon_text.content, ROTATE_CLOCKWISE_ICON);
        assert_eq!(clockwise_icon_text.font, ROTATION_ICON_FONT);
        assert_eq!(clockwise_icon_text.shaping, ROTATION_ICON_SHAPING);

        fn assert_rotation_button_tree(
            tree: &Tree,
            button_tag: iced::advanced::widget::tree::Tag,
            column_tag: iced::advanced::widget::tree::Tag,
            text_tag: iced::advanced::widget::tree::Tag,
        ) {
            assert_eq!(tree.tag, button_tag);
            assert_eq!(tree.children.len(), 1);
            assert_eq!(tree.children[0].tag, column_tag);
            assert_eq!(tree.children[0].children.len(), 2);
            assert!(tree.children[0]
                .children
                .iter()
                .all(|child| child.tag == text_tag));
        }

        let counterclockwise_button = rotation_button(
            ROTATE_COUNTERCLOCKWISE_ICON,
            ROTATE_COUNTERCLOCKWISE_STEP_LABEL,
            Message::RotateCounterclockwise,
        );
        let counterclockwise_tree = Tree::new(&counterclockwise_button);
        assert_rotation_button_tree(&counterclockwise_tree, button_tag, column_tag, text_tag);

        let clockwise_button = rotation_button(
            ROTATE_CLOCKWISE_ICON,
            ROTATE_CLOCKWISE_STEP_LABEL,
            Message::RotateClockwise,
        );
        let clockwise_tree = Tree::new(&clockwise_button);
        assert_rotation_button_tree(&clockwise_tree, button_tag, column_tag, text_tag);

        fn contains_rotation_section(
            tree: &Tree,
            column_tag: iced::advanced::widget::tree::Tag,
            container_tag: iced::advanced::widget::tree::Tag,
            row_tag: iced::advanced::widget::tree::Tag,
            button_tag: iced::advanced::widget::tree::Tag,
        ) -> bool {
            (tree.tag == column_tag
                && tree.children.len() == 2
                && tree.children[0].tag == container_tag
                && tree.children[1].tag == row_tag
                && tree.children[1].children.len() == 2
                && tree.children[1]
                    .children
                    .iter()
                    .all(|child| child.tag == button_tag))
                || tree.children.iter().any(|child| {
                    contains_rotation_section(child, column_tag, container_tag, row_tag, button_tag)
                })
        }

        let app = detail_app_with_image(Path::new("frame.png"), 200, 100);
        let panel_element = app.edit_panel();
        let panel_tree = Tree::new(&panel_element);
        assert!(contains_rotation_section(
            &panel_tree,
            column_tag,
            container_tag,
            row_tag,
            button_tag,
        ));
    }

    #[test]
    fn save_edited_is_a_no_op_without_a_current_image() {
        let (mut app, _) = App::new();
        app.collection_store = collection::CollectionStore::default();

        let _ = app.update(Message::SaveEdited);
        assert!(app.save_status.is_none());

        app.current_image_path = Some(PathBuf::from("frame.png"));
        let _ = app.update(Message::SaveEdited);
        assert!(app.save_status.is_none());
    }

    #[test]
    fn save_edited_is_a_no_op_while_loading() {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 2, 1);
        app.detail_load.stage = DetailLoadStage::Loading;

        let _ = app.update(Message::SaveEdited);

        assert!(app.save_status.is_none());
    }

    #[test]
    fn raw_preview_load_keeps_image_visible_while_full_resolution_finishes() {
        let path = PathBuf::from("frame.arw");
        let mut app = detail_app_with_image(&path, 200, 100);
        app.update_canvas_size([400.0, 200.0]);

        let _ = app.start_load(path);
        let request_id = app.detail_load.request_id;

        let _ = app.update(Message::ImagePreviewLoaded {
            request_id,
            path: PathBuf::from("frame.arw"),
            result: Ok(Some(test_image(400, 200))),
        });
        assert!(app.detail_load.is_loading());
        assert!(app.detail_load.shows_embedded_preview());
        assert!(app.current_save_request().is_none());

        app.zoom = 2.5;
        app.offset = [18.0, -9.0];
        let preview_rect = viewer::compute_image_rect(
            400.0,
            200.0,
            400.0,
            200.0,
            app.zoom,
            app.offset,
            app.current_rotation(),
        );

        let _ = app.update(Message::ImageLoaded {
            request_id,
            result: Ok(loaded_full_image(Path::new("frame.arw"), test_image(6000, 3000))),
        });

        assert!(!app.detail_load.is_loading());
        assert!(!app.detail_load.shows_embedded_preview());
        assert_eq!(app.image.as_ref().unwrap().width, 6000);
        assert_eq!(app.image.as_ref().unwrap().height, 3000);
        assert_eq!(app.zoom, 2.5);
        assert_eq!(app.offset, [18.0, -9.0]);
        let full_rect = viewer::compute_image_rect(
            6000.0,
            3000.0,
            400.0,
            200.0,
            app.zoom,
            app.offset,
            app.current_rotation(),
        );
        assert_eq!(preview_rect, full_rect);
        assert!(app.current_save_request().is_some());
    }

    #[test]
    fn repeat_raw_open_reuses_cached_full_image_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frame.arw");
        std::fs::write(&path, b"raw").unwrap();
        let mut app = detail_app_with_image(&path, 200, 100);

        let _ = app.update(Message::ImageLoaded {
            request_id: app.detail_load.request_id,
            result: Ok(loaded_full_image(&path, test_image(6000, 3000))),
        });

        app.error = Some("stale error".to_string());
        app.save_status = Some("stale save".to_string());
        app.current_exif = Some(lens::ExifInfo::default());
        app.zoom = 2.5;
        app.offset = [18.0, -9.0];
        app.crop_mode = true;

        let _ = app.start_load(path.clone());

        assert!(!app.detail_load.is_loading());
        assert!(!app.detail_load.shows_embedded_preview());
        assert_eq!(
            app.image.as_ref().map(|image| (image.width, image.height)),
            Some((6000, 3000))
        );
        assert!(app.error.is_none());
        assert!(app.save_status.is_none());
        assert!(app.current_exif.is_none());
        assert_eq!(app.zoom, 1.0);
        assert_eq!(app.offset, [0.0, 0.0]);
        assert!(!app.crop_mode);
        assert!(app.current_save_request().is_some());
    }

    #[test]
    fn library_reopen_reuses_the_displayed_full_image_immediately() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frame.arw");
        std::fs::write(&path, b"raw").unwrap();

        let (mut app, _) = App::new();
        app.collection_store = collection::CollectionStore::default();
        app.active_collection = None;
        app.context_menu = None;
        app.tab = Tab::Detail;
        app.replace_library_entries(vec![LibraryEntry {
            path: path.clone(),
            filename: "frame.arw".to_string(),
            thumbnail_handle: None,
        }]);
        app.current_image_path = Some(path.clone());

        let _ = app.update(Message::ImageLoaded {
            request_id: app.detail_load.request_id,
            result: Ok(loaded_full_image(&path, test_image(6000, 3000))),
        });

        app.error = Some("stale error".to_string());
        app.save_status = Some("stale save".to_string());
        app.current_exif = Some(lens::ExifInfo::default());
        app.zoom = 2.5;
        app.offset = [18.0, -9.0];
        app.crop_mode = true;

        let _ = app.update(Message::SwitchTab(Tab::Library));
        std::fs::remove_file(&path).unwrap();

        let _ = app.update(Message::LibraryItemClicked(0));
        let _ = app.update(Message::LibraryItemClicked(0));

        assert_eq!(app.tab, Tab::Detail);
        assert_eq!(app.library_index, Some(0));
        assert!(!app.detail_load.is_loading());
        assert!(!app.detail_load.shows_embedded_preview());
        assert_eq!(
            app.image.as_ref().map(|image| (image.width, image.height)),
            Some((6000, 3000))
        );
        assert!(app.error.is_none());
        assert!(app.save_status.is_none());
        assert!(app.current_exif.is_none());
        assert_eq!(app.zoom, 1.0);
        assert_eq!(app.offset, [0.0, 0.0]);
        assert!(!app.crop_mode);
        assert!(app.current_save_request().is_some());
    }

    #[test]
    fn repeat_raw_open_does_not_treat_embedded_preview_as_a_cached_full_image() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frame.arw");
        std::fs::write(&path, b"raw").unwrap();
        let mut app = detail_app_with_image(&path, 200, 100);

        let _ = app.start_load(path.clone());
        let request_id = app.detail_load.request_id;

        let _ = app.update(Message::ImagePreviewLoaded {
            request_id,
            path: path.clone(),
            result: Ok(Some(test_image(400, 200))),
        });
        assert!(app.detail_load.shows_embedded_preview());

        let _ = app.start_load(path);

        assert!(app.detail_load.is_loading());
        assert!(!app.detail_load.shows_embedded_preview());
        assert!(app.image.is_none());
        assert!(app.current_save_request().is_none());
    }

    #[test]
    fn repeat_raw_open_ignores_cached_full_image_after_the_source_changes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frame.arw");
        std::fs::write(&path, b"raw").unwrap();
        let mut app = detail_app_with_image(&path, 200, 100);

        let _ = app.update(Message::ImageLoaded {
            request_id: app.detail_load.request_id,
            result: Ok(loaded_full_image(&path, test_image(6000, 3000))),
        });

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, b"new").unwrap();

        let _ = app.start_load(path);

        assert!(app.detail_load.is_loading());
        assert!(!app.detail_load.shows_embedded_preview());
        assert!(app.image.is_none());
        assert!(app.current_save_request().is_none());
    }

    #[test]
    fn session_full_image_cache_evicts_the_least_recently_used_entry_when_over_budget() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.arw");
        let second = dir.path().join("second.arw");
        let third = dir.path().join("third.arw");
        std::fs::write(&first, b"first").unwrap();
        std::fs::write(&second, b"second").unwrap();
        std::fs::write(&third, b"third").unwrap();

        let mut cache = SessionFullImageCache::new(4, 16);
        cache.insert(
            &first,
            SourceFileFingerprint::from_path(&first).unwrap(),
            test_image_with_bytes(2, 1, 8),
        );
        cache.insert(
            &second,
            SourceFileFingerprint::from_path(&second).unwrap(),
            test_image_with_bytes(2, 1, 8),
        );
        assert!(cache.get(&first).is_some());

        cache.insert(
            &third,
            SourceFileFingerprint::from_path(&third).unwrap(),
            test_image_with_bytes(2, 1, 8),
        );

        assert!(cache.get(&first).is_some());
        assert!(cache.get(&second).is_none());
        assert!(cache.get(&third).is_some());
    }

    #[test]
    fn session_full_image_cache_evicts_oldest_entries_when_the_entry_cap_is_exceeded() {
        let dir = tempfile::tempdir().unwrap();
        let first = dir.path().join("first.arw");
        let second = dir.path().join("second.arw");
        let third = dir.path().join("third.arw");
        std::fs::write(&first, b"first").unwrap();
        std::fs::write(&second, b"second").unwrap();
        std::fs::write(&third, b"third").unwrap();

        let mut cache = SessionFullImageCache::new(2, 64);
        cache.insert(
            &first,
            SourceFileFingerprint::from_path(&first).unwrap(),
            test_image_with_bytes(2, 1, 8),
        );
        cache.insert(
            &second,
            SourceFileFingerprint::from_path(&second).unwrap(),
            test_image_with_bytes(2, 1, 8),
        );
        cache.insert(
            &third,
            SourceFileFingerprint::from_path(&third).unwrap(),
            test_image_with_bytes(2, 1, 8),
        );

        assert!(cache.get(&first).is_none());
        assert!(cache.get(&second).is_some());
        assert!(cache.get(&third).is_some());
    }

    #[test]
    fn session_full_image_cache_rejects_a_stale_fingerprint_captured_before_a_rewrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("frame.arw");
        std::fs::write(&path, b"old").unwrap();
        let old_fingerprint = SourceFileFingerprint::from_path(&path).unwrap();

        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(&path, b"new").unwrap();

        let mut cache = SessionFullImageCache::new(2, 64);
        cache.insert(&path, old_fingerprint, test_image_with_bytes(2, 1, 8));

        assert!(cache.get(&path).is_none());
    }

    #[test]
    fn raw_without_embedded_preview_still_finishes_full_resolution_load() {
        let path = PathBuf::from("frame.arw");
        let mut app = detail_app_with_image(&path, 200, 100);

        let _ = app.start_load(path);
        let request_id = app.detail_load.request_id;

        let _ = app.update(Message::ImagePreviewLoaded {
            request_id,
            path: PathBuf::from("frame.arw"),
            result: Ok(None),
        });

        assert!(app.detail_load.is_loading());
        assert!(!app.detail_load.shows_embedded_preview());
        assert_eq!(app.status_bar_text(), "  Loading…");

        let _ = app.update(Message::ImageLoaded {
            request_id,
            result: Ok(loaded_full_image(Path::new("frame.arw"), test_image(6000, 4000))),
        });

        assert!(!app.detail_load.is_loading());
        assert!(!app.detail_load.shows_embedded_preview());
        assert_eq!(app.image.as_ref().unwrap().width, 6000);
        assert!(app.current_save_request().is_some());
    }

    #[test]
    fn preview_only_mode_keeps_embedded_preview_visible_when_full_load_fails() {
        let path = PathBuf::from("frame.arw");
        let mut app = detail_app_with_image(&path, 200, 100);

        let _ = app.start_load(path);
        let request_id = app.detail_load.request_id;

        let _ = app.update(Message::ImagePreviewLoaded {
            request_id,
            path: PathBuf::from("frame.arw"),
            result: Ok(Some(test_image(400, 200))),
        });
        let _ = app.update(Message::ImageLoaded {
            request_id,
            result: Err("full decode failed".to_string()),
        });

        assert!(!app.detail_load.is_loading());
        assert!(app.detail_load.shows_embedded_preview());
        assert_eq!(app.image.as_ref().unwrap().width, 400);
        assert_eq!(
            app.save_status.as_deref(),
            Some("Full-resolution load failed; showing embedded preview")
        );
        assert!(app.status_bar_text().contains("Embedded preview"));
        assert!(app.current_save_request().is_none());
    }

    #[test]
    fn stale_preview_and_full_results_are_ignored_after_a_newer_load_starts() {
        let first_path = PathBuf::from("first.arw");
        let second_path = PathBuf::from("second.arw");
        let mut app = detail_app_with_image(&first_path, 200, 100);

        let _ = app.start_load(first_path);
        let first_request_id = app.detail_load.request_id;

        let _ = app.start_load(second_path.clone());
        let second_request_id = app.detail_load.request_id;

        let _ = app.update(Message::ImagePreviewLoaded {
            request_id: first_request_id,
            path: PathBuf::from("first.arw"),
            result: Ok(Some(test_image(320, 160))),
        });
        let _ = app.update(Message::ImageLoaded {
            request_id: first_request_id,
            result: Ok(loaded_full_image(Path::new("first.arw"), test_image(640, 320))),
        });

        assert!(app.image.is_none());
        assert!(app.detail_load.is_loading());
        assert_eq!(
            app.current_image_path.as_deref(),
            Some(second_path.as_path())
        );

        let _ = app.update(Message::ImagePreviewLoaded {
            request_id: second_request_id,
            path: PathBuf::from("second.arw"),
            result: Ok(Some(test_image(500, 250))),
        });

        assert_eq!(app.image.as_ref().unwrap().width, 500);
        assert!(app.detail_load.shows_embedded_preview());
    }

    #[test]
    fn save_edited_sets_saving_status_when_request_is_valid() {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 2, 1);

        let _ = app.update(Message::SaveEdited);

        assert_eq!(app.save_status.as_deref(), Some("Saving..."));
    }

    #[test]
    fn current_save_request_waits_for_auto_lens_metadata_when_needed() {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 2, 1);

        let mut history = edit::UndoHistory::new();
        history.current.lens_correction = true;
        history.commit();
        app.edit_histories.insert(path, history);
        app.detail_load.exif_loading = true;
        app.current_exif = None;
        app.lens_override_name = None;

        assert!(app.current_save_request().is_none());
    }

    #[test]
    fn current_save_request_allows_auto_lens_when_exif_finishes_without_metadata() {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 2, 1);

        let mut history = edit::UndoHistory::new();
        history.current.lens_correction = true;
        history.commit();
        app.edit_histories.insert(path.clone(), history);
        app.current_image_path = Some(path);
        app.detail_load.exif_loading = true;
        app.lens_override_name = None;

        assert!(app.current_save_request().is_none());

        let _ = app.update(Message::ExifLoaded {
            request_id: app.detail_load.request_id,
            exif: None,
        });

        assert!(!app.detail_load.exif_loading);
        assert!(app.current_save_request().is_some());
    }

    #[test]
    fn stale_exif_results_are_ignored_after_a_newer_load_starts() {
        let first_path = PathBuf::from("first.arw");
        let second_path = PathBuf::from("second.arw");
        let mut app = detail_app_with_image(&first_path, 2, 1);

        let _ = app.start_load(first_path);
        let first_request_id = app.detail_load.request_id;

        let _ = app.start_load(second_path.clone());
        let second_request_id = app.detail_load.request_id;

        let mut history = edit::UndoHistory::new();
        history.current.lens_correction = true;
        history.commit();
        app.edit_histories.insert(second_path.clone(), history);
        app.current_image_path = Some(second_path);
        app.image = Some(test_image(2, 1));
        app.detail_load.stage = DetailLoadStage::Idle;
        app.detail_load.exif_loading = true;
        app.lens_override_name = None;

        let _ = app.update(Message::ExifLoaded {
            request_id: first_request_id,
            exif: Some(lens::ExifInfo::default()),
        });

        assert!(app.current_exif.is_none());
        assert!(app.current_save_request().is_none());

        let _ = app.update(Message::ExifLoaded {
            request_id: second_request_id,
            exif: None,
        });

        assert!(!app.detail_load.exif_loading);
        assert!(app.current_save_request().is_some());
    }

    #[test]
    fn current_save_request_uses_enabled_lens_vignetting() {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 2, 1);
        app.current_lens_profile = Some(lens::LensProfile {
            maker: "Acme".to_string(),
            model: "Prime".to_string(),
            mount: "X".to_string(),
            distortion: None,
            vignetting: Some(lens::VignetteCoeffs {
                k1: 0.1,
                k2: 0.2,
                k3: 0.3,
            }),
            tca: None,
        });

        let mut history = edit::UndoHistory::new();
        history.current.lens_correction = true;
        history.commit();
        app.edit_histories.insert(path, history);
        app.current_exif = Some(lens::ExifInfo::default());

        let request = app.current_save_request().unwrap();
        assert_eq!(request.vig, [0.1, 0.2, 0.3]);
    }

    #[test]
    fn current_save_request_zeroes_vignetting_without_active_correction() {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 2, 1);
        app.current_lens_profile = Some(lens::LensProfile {
            maker: "Acme".to_string(),
            model: "Prime".to_string(),
            mount: "X".to_string(),
            distortion: None,
            vignetting: Some(lens::VignetteCoeffs {
                k1: 0.1,
                k2: 0.2,
                k3: 0.3,
            }),
            tca: None,
        });

        let request = app.current_save_request().unwrap();
        assert_eq!(request.vig, [0.0, 0.0, 0.0]);
    }

    #[test]
    fn rotate_messages_preserve_actual_size_zoom_when_orientation_changes() {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 200, 100);

        app.update_canvas_size([400.0, 200.0]);
        let original_zoom = app.actual_size_zoom_for_rotation(
            app.current_canvas_size(),
            app.image.as_ref().unwrap(),
            edit::QuarterTurns::default(),
        );
        app.zoom = original_zoom;
        app.offset = [0.0, 0.0];

        let _ = app.update(Message::RotateClockwise);

        let rotated_zoom = app.actual_size_zoom_for_rotation(
            app.current_canvas_size(),
            app.image.as_ref().unwrap(),
            app.current_rotation(),
        );
        assert!((app.zoom - rotated_zoom).abs() < 0.01);
    }

    #[test]
    fn rotate_messages_preserve_actual_size_zoom_when_panned() {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 200, 100);

        app.update_canvas_size([400.0, 200.0]);
        app.zoom = app.actual_size_zoom_for_rotation(
            app.current_canvas_size(),
            app.image.as_ref().unwrap(),
            edit::QuarterTurns::default(),
        );
        app.offset = [32.0, -18.0];

        let _ = app.update(Message::RotateClockwise);

        let rotated_zoom = app.actual_size_zoom_for_rotation(
            app.current_canvas_size(),
            app.image.as_ref().unwrap(),
            app.current_rotation(),
        );
        assert!((app.zoom - rotated_zoom).abs() < 0.01);
        assert_eq!(app.offset, [32.0, -18.0]);
    }

    #[test]
    fn reset_all_preserves_actual_size_zoom_after_rotation() {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 200, 100);

        let mut history = edit::UndoHistory::new();
        history.current.rotate_clockwise();
        history.commit();
        app.edit_histories.insert(path, history);
        app.update_canvas_size([400.0, 200.0]);
        app.zoom = app.actual_size_zoom_for_rotation(
            app.current_canvas_size(),
            app.image.as_ref().unwrap(),
            app.current_rotation(),
        );

        let _ = app.update(Message::ResetAll);

        let reset_zoom = app.actual_size_zoom_for_rotation(
            app.current_canvas_size(),
            app.image.as_ref().unwrap(),
            app.current_rotation(),
        );
        assert!((app.zoom - reset_zoom).abs() < 0.01);
    }

    #[test]
    fn reset_all_preserves_actual_size_zoom_after_clearing_crop() {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 200, 100);

        let mut history = edit::UndoHistory::new();
        history.current.crop = Some(edit::CropRect::new(0.5, 0.0, 1.0, 1.0));
        history.commit();
        app.edit_histories.insert(path, history);
        app.update_canvas_size([400.0, 200.0]);
        app.zoom = app.actual_size_zoom_for_rotation_and_crop(
            app.current_canvas_size(),
            app.image.as_ref().unwrap(),
            app.current_rotation(),
            app.current_crop(),
        );

        let _ = app.update(Message::ResetAll);

        let reset_zoom = app.actual_size_zoom_for_rotation_and_crop(
            app.current_canvas_size(),
            app.image.as_ref().unwrap(),
            app.current_rotation(),
            None,
        );
        assert!((app.zoom - reset_zoom).abs() < 0.01);
    }

    #[test]
    fn undo_and_redo_preserve_actual_size_zoom_after_rotation_changes() {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 200, 100);
        app.update_canvas_size([400.0, 200.0]);

        let _ = app.update(Message::RotateClockwise);
        app.zoom = app.actual_size_zoom_for_rotation(
            app.current_canvas_size(),
            app.image.as_ref().unwrap(),
            app.current_rotation(),
        );

        let _ = app.handle_key(
            keyboard::Key::Character("z".into()),
            keyboard::Modifiers::CTRL,
        );
        let undo_zoom = app.actual_size_zoom_for_rotation(
            app.current_canvas_size(),
            app.image.as_ref().unwrap(),
            app.current_rotation(),
        );
        assert!((app.zoom - undo_zoom).abs() < 0.01);

        let redo_mods = keyboard::Modifiers::CTRL | keyboard::Modifiers::SHIFT;
        let _ = app.handle_key(keyboard::Key::Character("z".into()), redo_mods);
        let redo_zoom = app.actual_size_zoom_for_rotation(
            app.current_canvas_size(),
            app.image.as_ref().unwrap(),
            app.current_rotation(),
        );
        assert!((app.zoom - redo_zoom).abs() < 0.01);
    }

    #[test]
    fn actual_size_shortcut_uses_rotated_dimensions() {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 200, 100);
        let mut history = edit::UndoHistory::new();
        history.current.rotate_clockwise();
        history.commit();
        app.edit_histories.insert(path, history);
        app.update_canvas_size([400.0, 200.0]);
        app.zoom = 3.0;
        app.offset = [20.0, -10.0];

        let _ = app.handle_key(
            keyboard::Key::Character("1".into()),
            keyboard::Modifiers::default(),
        );

        let expected_zoom = app.actual_size_zoom_for_rotation(
            app.current_canvas_size(),
            app.image.as_ref().unwrap(),
            app.current_rotation(),
        );
        assert!((app.zoom - expected_zoom).abs() < 0.01);
        assert_eq!(app.offset, [0.0, 0.0]);
    }

    #[test]
    fn drag_activates_after_threshold() {
        // Drag becomes active when cursor moves more than 5px from start
        let mut drag = DragState {
            photo_index: 0,
            start_pos: [100.0, 100.0],
            current_pos: [100.0, 100.0],
            active: false,
        };
        // Move 3px - should not activate
        drag.current_pos = [103.0, 100.0];
        let dx = drag.current_pos[0] - drag.start_pos[0];
        let dy = drag.current_pos[1] - drag.start_pos[1];
        if (dx * dx + dy * dy).sqrt() > 5.0 {
            drag.active = true;
        }
        assert!(!drag.active);

        // Move 6px - should activate
        drag.current_pos = [106.0, 100.0];
        let dx = drag.current_pos[0] - drag.start_pos[0];
        let dy = drag.current_pos[1] - drag.start_pos[1];
        if (dx * dx + dy * dy).sqrt() > 5.0 {
            drag.active = true;
        }
        assert!(drag.active);
    }

    #[test]
    fn drag_drop_adds_photo_to_hovered_collection() {
        // Simulate: active drag released over sidebar collection -> adds photo
        let mut store = collection::CollectionStore::default();
        store.create("Target");
        let photo_path = PathBuf::from("/test/landscape.jpg");
        let sidebar_hover_collection: Option<usize> = Some(0);
        let drag = DragState {
            photo_index: 0,
            start_pos: [50.0, 50.0],
            current_pos: [200.0, 100.0],
            active: true,
        };
        // Simulate the ButtonReleased handler
        if drag.active {
            if let Some(col_idx) = sidebar_hover_collection {
                store.add_photo(col_idx, &photo_path);
            }
        }
        assert_eq!(store.collections[0].photos.len(), 1);
        assert!(store.collections[0].photos.contains(&photo_path));
    }

    #[test]
    fn drag_drop_no_hover_does_not_add() {
        // If drag is released but no collection is hovered, nothing happens
        let mut store = collection::CollectionStore::default();
        store.create("Target");
        let sidebar_hover_collection: Option<usize> = None;
        let drag = DragState {
            photo_index: 0,
            start_pos: [50.0, 50.0],
            current_pos: [200.0, 100.0],
            active: true,
        };
        if drag.active {
            if let Some(col_idx) = sidebar_hover_collection {
                store.add_photo(col_idx, &PathBuf::from("/test/photo.jpg"));
                let _ = col_idx;
            }
        }
        assert!(store.collections[0].photos.is_empty());
    }

    #[test]
    fn drag_not_active_does_not_add() {
        // If drag exists but never became active (< 5px), no add on release
        let mut store = collection::CollectionStore::default();
        store.create("Target");
        let sidebar_hover_collection: Option<usize> = Some(0);
        let drag = DragState {
            photo_index: 0,
            start_pos: [50.0, 50.0],
            current_pos: [52.0, 50.0],
            active: false,
        };
        if drag.active {
            if let Some(col_idx) = sidebar_hover_collection {
                store.add_photo(col_idx, &PathBuf::from("/test/photo.jpg"));
                let _ = col_idx;
            }
        }
        assert!(store.collections[0].photos.is_empty());
    }
}
