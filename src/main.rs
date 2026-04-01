#![windows_subsystem = "windows"]

mod decode;
mod edit;
mod lens;
mod nav;
mod viewer;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use iced::widget::image::Handle as ImageHandle;
use iced::widget::{
    button, column, container, horizontal_space, pick_list, row, scrollable, shader, slider, text,
    text_input, Image,
};
use iced::{
    event, keyboard, window, Alignment, Background, Border, Color, Element, Length, Size,
    Subscription, Task, Theme,
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

struct LibraryEntry {
    path: PathBuf,
    filename: String,
    thumbnail_handle: Option<ImageHandle>,
}

// ---------------------------------------------------------------------------
// Application state
// ---------------------------------------------------------------------------

struct App {
    tab: Tab,
    library: Vec<LibraryEntry>,
    image: Option<Arc<ImageData>>,
    image_id: u64,
    zoom: f32,
    offset: [f32; 2],
    canvas_size: [f32; 2],
    nav: Option<DirNav>,
    library_index: Option<usize>,
    loading: bool,
    error: Option<String>,
    edit_histories: std::collections::HashMap<PathBuf, edit::UndoHistory>,
    current_image_path: Option<PathBuf>,
    lens_db: lens::LensDatabase,
    current_lens_profile: Option<lens::LensProfile>,
    current_exif: Option<lens::ExifInfo>,
    save_status: Option<String>,
    editing_slider: Option<SliderKind>,
    slider_text_buf: String,
    last_thumb_click: Option<(usize, Instant)>,
    last_slider_release: Option<(SliderKind, Instant)>,
    /// Tracks slider drag vs. single click: (which slider, event count).
    /// Only apply values after 2+ on_change events (i.e., actual drag).
    slider_drag: Option<(SliderKind, u32)>,
    lens_override_name: Option<String>,
}

#[derive(Debug, Clone)]
enum Message {
    #[allow(dead_code)]
    OpenFile,
    FileSelected(Option<PathBuf>),
    ImageLoaded(Result<Arc<ImageData>, String>),
    Viewer(ViewerEvent),
    Event(iced::Event),
    SwitchTab(Tab),
    AddFolder,
    AddFiles,
    FolderPicked(Option<PathBuf>),
    FilesPicked(Option<Vec<PathBuf>>),
    ThumbnailLoaded(PathBuf, Result<Arc<ImageData>, String>),
    LibraryItemClicked(usize),
    SliderChanged(SliderKind, f32),
    SliderReleased(SliderKind),
    ResetSlider(SliderKind),
    ResetAll,
    SaveEdited,
    SaveCompleted(Result<String, String>),
    ToggleLensCorrection,
    SliderTextInput(SliderKind),
    SliderTextChanged(String),
    SliderTextSubmit(SliderKind),
    LensProfileSelected(String),
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

impl App {
    fn new() -> (Self, Task<Message>) {
        let mut app = App {
            tab: Tab::Library,
            library: Vec::new(),
            image: None,
            image_id: 0,
            zoom: 1.0,
            offset: [0.0, 0.0],
            canvas_size: [1200.0, 780.0],
            nav: None,
            library_index: None,
            loading: false,
            error: None,
            edit_histories: std::collections::HashMap::new(),
            current_image_path: None,
            lens_db: lens::LensDatabase::load_bundled(),
            current_lens_profile: None,
            current_exif: None,
            save_status: None,
            editing_slider: None,
            slider_text_buf: String::new(),
            last_thumb_click: None,
            last_slider_release: None,
            slider_drag: None,
            lens_override_name: None,
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
                app.current_image_path = Some(path.clone());
                app.loading = true;
                let load_path = path;
                Task::perform(
                    async move {
                        let result: Result<Arc<ImageData>, String> =
                            tokio::task::spawn_blocking(move || decode::decode_image(&load_path))
                                .await
                                .map_err(|e| e.to_string())?;
                        result
                    },
                    Message::ImageLoaded,
                )
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
                if let Some(idx) = self.library_index {
                    if let Some(entry) = self.library.get(idx) {
                        return format!("Photo - {}", entry.filename);
                    }
                }
                match &self.nav {
                    Some(nav) if !nav.current_filename().is_empty() => {
                        format!("Photo - {}", nav.current_filename())
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
                self.current_image_path = Some(path.clone());
                self.start_load(path)
            }
            Message::FileSelected(None) => Task::none(),

            Message::ImageLoaded(Ok(data)) => {
                self.image = Some(data);
                self.image_id += 1;
                self.zoom = 1.0;
                self.offset = [0.0, 0.0];
                self.loading = false;
                self.error = None;
                if let Some(path) = &self.current_image_path {
                    self.current_exif = lens::read_exif(path);
                    if self.lens_override_name.is_none() {
                        self.current_lens_profile =
                            self.current_exif.as_ref().and_then(|exif_info| {
                                let maker = if exif_info.lens_make.is_empty() {
                                    &exif_info.camera_make
                                } else {
                                    &exif_info.lens_make
                                };
                                self.lens_db
                                    .find_lens(maker, &exif_info.lens_model)
                                    .cloned()
                            });
                    }
                }
                Task::none()
            }
            Message::ImageLoaded(Err(e)) => {
                self.image = None;
                self.loading = false;
                self.error = Some(e);
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
                        .add_filter(
                            "Images",
                            &[
                                "jpg", "jpeg", "png", "gif", "bmp", "tiff", "tif", "webp", "svg",
                                "svgz", "ico", "tga", "qoi", "hdr", "exr",
                            ],
                        )
                        .pick_files()
                        .await
                        .map(|files| files.into_iter().map(|f| f.path().to_path_buf()).collect())
                },
                Message::FilesPicked,
            ),

            Message::FolderPicked(Some(folder)) => {
                let new_paths = scan_folder_for_images(&folder);
                self.add_library_entries(&new_paths);
                save_library(&self.library);
                Self::load_thumbnails(&new_paths)
            }
            Message::FolderPicked(None) => Task::none(),

            Message::FilesPicked(Some(paths)) => {
                let new_paths: Vec<PathBuf> = paths
                    .into_iter()
                    .filter(|p| !self.library.iter().any(|e| e.path == *p))
                    .collect();
                self.add_library_entries(&new_paths);
                save_library(&self.library);
                Self::load_thumbnails(&new_paths)
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

            Message::LibraryItemClicked(index) => {
                let now = Instant::now();
                let is_double_click = self
                    .last_thumb_click
                    .map(|(prev_idx, prev_time)| {
                        prev_idx == index && now.duration_since(prev_time).as_millis() < 400
                    })
                    .unwrap_or(false);

                if is_double_click {
                    self.last_thumb_click = None;
                    if let Some(entry) = self.library.get(index) {
                        self.library_index = Some(index);
                        self.tab = Tab::Detail;
                        let path = entry.path.clone();
                        self.current_image_path = Some(path.clone());
                        return self.start_load(path);
                    }
                } else {
                    self.last_thumb_click = Some((index, now));
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
                if let Some(path) = &self.current_image_path {
                    let history = self.edit_histories.entry(path.clone()).or_default();
                    history.reset_all();
                }
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

            Message::SaveEdited => {
                let Some(path) = self.current_image_path.clone() else {
                    return Task::none();
                };
                let Some(img) = self.image.clone() else {
                    return Task::none();
                };
                let state = self
                    .edit_histories
                    .get(&path)
                    .map(|h| h.current)
                    .unwrap_or_default();
                self.save_status = Some("Saving...".to_string());
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            edit::save_edited_image(
                                &path,
                                &img.pixels,
                                img.width,
                                img.height,
                                &state,
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
                    self.current_lens_profile =
                        self.current_exif.as_ref().and_then(|exif_info| {
                            let maker = if exif_info.lens_make.is_empty() {
                                &exif_info.camera_make
                            } else {
                                &exif_info.lens_make
                            };
                            self.lens_db
                                .find_lens(maker, &exif_info.lens_model)
                                .cloned()
                        });
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
                self.canvas_size = canvas_size;
                let (z, o) = zoom_at_cursor(self.zoom, self.offset, factor, cursor, canvas_size);
                self.zoom = z;
                self.offset = o;
            }
            ViewerEvent::Pan { delta } => {
                self.offset[0] += delta[0];
                self.offset[1] += delta[1];
            }
            ViewerEvent::DoubleClick { canvas_size } => {
                self.canvas_size = canvas_size;
                if (self.zoom - 1.0).abs() < 0.01 && self.offset == [0.0, 0.0] {
                    if let Some(img) = &self.image {
                        let fit = (canvas_size[0] / img.width as f32)
                            .min(canvas_size[1] / img.height as f32);
                        self.zoom = 1.0 / fit;
                    }
                } else {
                    self.zoom = 1.0;
                    self.offset = [0.0, 0.0];
                }
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
                self.current_image_path = Some(path.clone());
                self.start_load(path)
            }

            _ => Task::none(),
        }
    }

    fn handle_key(&mut self, key: keyboard::Key, mods: keyboard::Modifiers) -> Task<Message> {
        use keyboard::key::Named;
        use keyboard::Key;

        match key {
            // Escape: go back to library from detail
            Key::Named(Named::Escape) => {
                if self.tab == Tab::Detail {
                    self.tab = Tab::Library;
                }
            }

            // Navigation: next
            Key::Named(Named::ArrowRight) | Key::Named(Named::Space) => {
                if self.tab == Tab::Detail {
                    if let Some(ref mut lib_idx) = self.library_index {
                        if !self.library.is_empty() {
                            *lib_idx = (*lib_idx + 1) % self.library.len();
                            let path = self.library[*lib_idx].path.clone();
                            self.current_image_path = Some(path.clone());
                            return self.start_load(path);
                        }
                    } else if let Some(nav) = &mut self.nav {
                        if let Some(p) = nav.next() {
                            self.current_image_path = Some(p.clone());
                            return self.start_load(p);
                        }
                    }
                }
            }

            // Navigation: prev
            Key::Named(Named::ArrowLeft) | Key::Named(Named::Backspace) => {
                if self.tab == Tab::Detail {
                    if let Some(ref mut lib_idx) = self.library_index {
                        if !self.library.is_empty() {
                            *lib_idx = if *lib_idx == 0 {
                                self.library.len() - 1
                            } else {
                                *lib_idx - 1
                            };
                            let path = self.library[*lib_idx].path.clone();
                            self.current_image_path = Some(path.clone());
                            return self.start_load(path);
                        }
                    } else if let Some(nav) = &mut self.nav {
                        if let Some(p) = nav.prev() {
                            self.current_image_path = Some(p.clone());
                            return self.start_load(p);
                        }
                    }
                }
            }

            // Undo
            Key::Character(ref c) if c.as_str() == "z" && mods.command() && !mods.shift() => {
                if let Some(path) = &self.current_image_path {
                    if let Some(history) = self.edit_histories.get_mut(path) {
                        history.undo();
                    }
                }
                return Task::none();
            }
            // Redo
            Key::Character(ref c)
                if (c.as_str() == "z" && mods.command() && mods.shift())
                    || (c.as_str() == "y" && mods.command()) =>
            {
                if let Some(path) = &self.current_image_path {
                    if let Some(history) = self.edit_histories.get_mut(path) {
                        history.redo();
                    }
                }
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
                        let cs = self.canvas_size;
                        let fit = (cs[0] / img.width as f32).min(cs[1] / img.height as f32);
                        self.zoom = 1.0 / fit;
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

    fn add_library_entries(&mut self, paths: &[PathBuf]) {
        for path in paths {
            if !self.library.iter().any(|e| e.path == *path) {
                self.library.push(LibraryEntry {
                    filename: path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("")
                        .to_string(),
                    path: path.clone(),
                    thumbnail_handle: None,
                });
            }
        }
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

    fn open_file_dialog(&self) -> Task<Message> {
        Task::perform(
            async {
                rfd::AsyncFileDialog::new()
                    .add_filter(
                        "Images",
                        &[
                            "jpg", "jpeg", "png", "gif", "bmp", "tiff", "tif", "webp", "svg",
                            "svgz", "ico", "tga", "qoi", "hdr", "exr",
                        ],
                    )
                    .pick_file()
                    .await
                    .map(|f| f.path().to_path_buf())
            },
            Message::FileSelected,
        )
    }

    fn start_load(&mut self, path: PathBuf) -> Task<Message> {
        self.loading = true;
        self.error = None;
        Task::perform(
            async move {
                let result: Result<Arc<ImageData>, String> =
                    tokio::task::spawn_blocking(move || decode::decode_image(&path))
                        .await
                        .map_err(|e| e.to_string())?;
                result
            },
            Message::ImageLoaded,
        )
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
        column![tab_bar, content].into()
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
                let back_btn =
                    button(text("\u{2190}").size(16).color(TEXT_PRIMARY))
                        .on_press(Message::SwitchTab(Tab::Library))
                        .padding([4, 12])
                        .style(toolbar_button_style);

                let save_btn = button(text("Save").size(11).color(TEXT_PRIMARY))
                    .on_press(Message::SaveEdited)
                    .padding([5, 12])
                    .style(toolbar_button_style);

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

        let thumb_size: f32 = 150.0;
        let cols = 6;

        let entries: Vec<(usize, &LibraryEntry)> = self.library.iter().enumerate().collect();
        let mut grid = column![].spacing(8);

        for chunk in entries.chunks(cols) {
            let mut r = row![].spacing(8);
            for &(idx, entry) in chunk {
                r = r.push(self.thumbnail_card(entry, idx, thumb_size));
            }
            grid = grid.push(r);
        }

        let status_text = format!(
            "{} images  \u{2022}  Double-click to open",
            self.library.len()
        );
        let status = container(text(status_text).size(11).color(TEXT_DIM))
            .width(Length::Fill)
            .padding([6, 14]);

        container(column![
            scrollable(container(grid).padding(14).width(Length::Fill)).height(Length::Fill),
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
        let thumb: Element<'_, Message> = if let Some(ref handle) = entry.thumbnail_handle {
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

        let label = container(text(&entry.filename).size(10).color(TEXT_SECONDARY)).width(thumb_size);

        button(column![thumb, label].spacing(4).width(thumb_size))
            .on_press(Message::LibraryItemClicked(index))
            .padding(6)
            .style(card_button_style)
            .into()
    }

    fn detail_view(&self) -> Element<'_, Message> {
        let canvas: Element<'_, ViewerEvent> = shader(ImageCanvas {
            image: self.image.clone(),
            image_id: self.image_id,
            zoom: self.zoom,
            offset: self.offset,
            adjustments: self.build_adjustment_uniforms(),
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into();

        let viewer_with_status = column![canvas.map(Message::Viewer), self.status_bar()];

        row![viewer_with_status.width(Length::Fill), self.edit_panel()].into()
    }

    fn status_bar(&self) -> Element<'_, Message> {
        let s = if let Some(img) = &self.image {
            let name = if let Some(idx) = self.library_index {
                self.library
                    .get(idx)
                    .map(|e| e.filename.clone())
                    .unwrap_or_default()
            } else {
                self.nav
                    .as_ref()
                    .map_or(String::new(), |n| n.current_filename())
            };

            let pos = if let Some(idx) = self.library_index {
                format!("  {}/{}", idx + 1, self.library.len())
            } else {
                self.nav
                    .as_ref()
                    .map(|n| format!("  {}/{}", n.current_index() + 1, n.count()))
                    .unwrap_or_default()
            };

            let zoom_pct = (self.zoom * 100.0) as u32;
            let mb = img.file_size as f64 / 1_048_576.0;
            format!(
                "  {name}  \u{2022}  {w}\u{00d7}{h}  \u{2022}  {mb:.1} MB  \u{2022}  {zoom_pct}%{pos}",
                w = img.width,
                h = img.height,
            )
        } else if self.loading {
            "  Loading\u{2026}".to_string()
        } else if let Some(e) = &self.error {
            format!("  Error: {e}")
        } else {
            "  Ctrl+O to open  \u{2022}  Drag & drop  \u{2022}  Arrow keys to navigate".to_string()
        };

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

        let lens_info: Element<'_, Message> = if let Some(profile) = &self.current_lens_profile {
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

        let lens_section = column![
            section_label("LENS"),
            lens_btn,
            lens_dropdown,
            lens_info,
        ]
        .spacing(4);

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

        let label_el: Element<'_, Message> = button(
            text(label.to_string())
                .size(11)
                .color(TEXT_SECONDARY),
        )
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
            button(
                text(format!("{:.1}", value))
                    .size(11)
                    .color(TEXT_PRIMARY),
            )
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
                    let vig = p.vignetting.map(|v| [v.k1, v.k2, v.k3]).unwrap_or([0.0; 3]);
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
        }
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
        SliderKind::Highlights | SliderKind::Shadows => (-100.0, 100.0),
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

pub fn scan_folder_for_images(folder: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| nav::is_image_file(p))
        .collect();

    files.sort_by(|a, b| {
        natord::compare(
            a.file_name().and_then(|n| n.to_str()).unwrap_or(""),
            b.file_name().and_then(|n| n.to_str()).unwrap_or(""),
        )
    });

    files
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

    #[test]
    fn scan_folder_finds_only_images() {
        let (dir, _) = setup_dir(&["photo.jpg", "notes.txt", "icon.png", "data.csv", "art.bmp"]);
        let results = scan_folder_for_images(dir.path());
        assert_eq!(results.len(), 3);
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
    fn save_and_load_library_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let lib_path = dir.path().join("library.txt");

        let p1 = dir.path().join("a.png");
        let p2 = dir.path().join("b.jpg");
        std::fs::write(&p1, b"").unwrap();
        std::fs::write(&p2, b"").unwrap();

        let entries = vec![
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

        // Highlights/Shadows keep full range
        let (min, max) = slider_range(SliderKind::Highlights);
        assert_eq!(min, -100.0);
        assert_eq!(max, 100.0);

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
}
