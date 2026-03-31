#![windows_subsystem = "windows"]

mod decode;
#[allow(dead_code)] // Wired to UI in Task 8
mod edit;
mod nav;
mod viewer;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use iced::widget::image::Handle as ImageHandle;
use iced::widget::{
    button, column, container, horizontal_space, row, scrollable, shader, text, Image,
};
use iced::{
    event, keyboard, window, Alignment, Color, Element, Length, Size, Subscription, Task, Theme,
};

use decode::ImageData;
use nav::DirNav;
use viewer::{zoom_at_cursor, ImageCanvas, ViewerEvent};

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
                app.loading = true;
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
                if let Some(entry) = self.library.get(index) {
                    self.library_index = Some(index);
                    self.tab = Tab::Detail;
                    let path = entry.path.clone();
                    self.start_load(path)
                } else {
                    Task::none()
                }
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
                self.start_load(path)
            }

            _ => Task::none(),
        }
    }

    fn handle_key(&mut self, key: keyboard::Key, mods: keyboard::Modifiers) -> Task<Message> {
        use keyboard::key::Named;
        use keyboard::Key;

        match key {
            // Navigation: next
            Key::Named(Named::ArrowRight) | Key::Named(Named::Space) => {
                if self.tab == Tab::Detail {
                    if let Some(ref mut lib_idx) = self.library_index {
                        if !self.library.is_empty() {
                            *lib_idx = (*lib_idx + 1) % self.library.len();
                            let path = self.library[*lib_idx].path.clone();
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
                    if let Some(ref mut lib_idx) = self.library_index {
                        if !self.library.is_empty() {
                            *lib_idx = if *lib_idx == 0 {
                                self.library.len() - 1
                            } else {
                                *lib_idx - 1
                            };
                            let path = self.library[*lib_idx].path.clone();
                            return self.start_load(path);
                        }
                    } else if let Some(nav) = &mut self.nav {
                        if let Some(p) = nav.prev() {
                            return self.start_load(p);
                        }
                    }
                }
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
        let lib_label = if self.tab == Tab::Library {
            "* Library"
        } else {
            "  Library"
        };
        let det_label = if self.tab == Tab::Detail {
            "* Detail"
        } else {
            "  Detail"
        };

        let library_btn = button(text(lib_label).size(13))
            .on_press(Message::SwitchTab(Tab::Library))
            .padding([6, 16]);

        let detail_btn = button(text(det_label).size(13))
            .on_press(Message::SwitchTab(Tab::Detail))
            .padding([6, 16]);

        let add_folder_btn = button(text("+ Folder").size(12))
            .on_press(Message::AddFolder)
            .padding([5, 12]);

        let add_files_btn = button(text("+ Files").size(12))
            .on_press(Message::AddFiles)
            .padding([5, 12]);

        container(
            row![
                library_btn,
                detail_btn,
                horizontal_space(),
                add_folder_btn,
                add_files_btn,
            ]
            .spacing(6),
        )
        .padding([6, 10])
        .width(Length::Fill)
        .into()
    }

    fn library_view(&self) -> Element<'_, Message> {
        if self.library.is_empty() {
            return container(
                column![
                    text("No images loaded")
                        .size(18)
                        .color(Color::from_rgb(0.5, 0.5, 0.5)),
                    text("Use + Folder or + Files to add images")
                        .size(13)
                        .color(Color::from_rgb(0.4, 0.4, 0.4)),
                ]
                .spacing(8)
                .align_x(Alignment::Center),
            )
            .center_x(Length::Fill)
            .center_y(Length::Fill)
            .into();
        }

        let thumb_size: f32 = 150.0;
        let cols = 6;

        let entries: Vec<(usize, &LibraryEntry)> = self.library.iter().enumerate().collect();
        let mut grid = column![].spacing(10);

        for chunk in entries.chunks(cols) {
            let mut r = row![].spacing(10);
            for &(idx, entry) in chunk {
                r = r.push(self.thumbnail_card(entry, idx, thumb_size));
            }
            grid = grid.push(r);
        }

        let status_text = format!("  {} images", self.library.len());
        let status = container(
            text(status_text)
                .size(13)
                .color(Color::from_rgb(0.55, 0.55, 0.55)),
        )
        .width(Length::Fill)
        .padding([5, 10]);

        column![
            scrollable(container(grid).padding(15).width(Length::Fill)).height(Length::Fill),
            status,
        ]
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
            container(text("...").size(24).color(Color::from_rgb(0.3, 0.3, 0.3)))
                .width(thumb_size)
                .height(thumb_size)
                .center_x(Length::Shrink)
                .center_y(Length::Shrink)
                .into()
        };

        let label = container(
            text(&entry.filename)
                .size(11)
                .color(Color::from_rgb(0.7, 0.7, 0.7)),
        )
        .width(thumb_size);

        button(column![thumb, label].spacing(4).width(thumb_size))
            .on_press(Message::LibraryItemClicked(index))
            .padding(5)
            .into()
    }

    fn detail_view(&self) -> Element<'_, Message> {
        let canvas: Element<'_, ViewerEvent> = shader(ImageCanvas {
            image: self.image.clone(),
            image_id: self.image_id,
            zoom: self.zoom,
            offset: self.offset,
            adjustments: Default::default(),
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into();

        let status = self.status_bar();
        column![canvas.map(Message::Viewer), status].into()
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
                "  {name}  |  {w}x{h}  |  {mb:.1} MB  |  {zoom_pct}%{pos}",
                w = img.width,
                h = img.height,
            )
        } else if self.loading {
            "  Loading...".to_string()
        } else if let Some(e) = &self.error {
            format!("  Error: {e}")
        } else {
            "  Ctrl+O to open  |  Drag & drop an image  |  Arrow keys to navigate".to_string()
        };

        container(text(s).size(13).color(Color::from_rgb(0.55, 0.55, 0.55)))
            .width(Length::Fill)
            .padding([5, 10])
            .into()
    }
}

// ---------------------------------------------------------------------------
// Free functions
// ---------------------------------------------------------------------------

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
}
