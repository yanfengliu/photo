#![windows_subsystem = "windows"]

mod decode;
mod nav;
mod viewer;

use std::path::PathBuf;
use std::sync::Arc;

use iced::widget::{column, container, shader, text};
use iced::{event, keyboard, window, Color, Element, Length, Size, Subscription, Task, Theme};

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
// Application state
// ---------------------------------------------------------------------------

struct App {
    image: Option<Arc<ImageData>>,
    image_id: u64,
    zoom: f32,        // 1.0 = fit to window
    offset: [f32; 2], // pan in logical pixels
    canvas_size: [f32; 2],
    nav: Option<DirNav>,
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
}

// ---------------------------------------------------------------------------
// Lifecycle
// ---------------------------------------------------------------------------

impl App {
    fn new() -> (Self, Task<Message>) {
        let mut app = App {
            image: None,
            image_id: 0,
            zoom: 1.0,
            offset: [0.0, 0.0],
            canvas_size: [1200.0, 780.0],
            nav: None,
            loading: false,
            error: None,
        };

        // Open file from command-line argument
        let args: Vec<String> = std::env::args().collect();
        let task = if args.len() > 1 {
            let path = PathBuf::from(&args[1]);
            if path.exists() {
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

        (app, task)
    }

    fn title(&self) -> String {
        match &self.nav {
            Some(nav) if !nav.current_filename().is_empty() => {
                format!("Photo \u{2014} {}", nav.current_filename())
            }
            _ => "Photo".to_string(),
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
                    // Currently fit -> go to actual size
                    if let Some(img) = &self.image {
                        let fit = (canvas_size[0] / img.width as f32)
                            .min(canvas_size[1] / img.height as f32);
                        self.zoom = 1.0 / fit;
                    }
                } else {
                    // Reset to fit
                    self.zoom = 1.0;
                    self.offset = [0.0, 0.0];
                }
            }
        }
    }

    // ---------------------------------------------------------------------------
    // Global events (keyboard, file drop)
    // ---------------------------------------------------------------------------

    fn handle_event(&mut self, event: iced::Event) -> Task<Message> {
        match event {
            iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key, modifiers, ..
            }) => self.handle_key(key, modifiers),

            iced::Event::Window(window::Event::FileDropped(path)) => {
                self.nav = Some(DirNav::new(&path));
                self.start_load(path)
            }

            _ => Task::none(),
        }
    }

    fn handle_key(
        &mut self,
        key: keyboard::Key,
        mods: keyboard::Modifiers,
    ) -> Task<Message> {
        use keyboard::key::Named;
        use keyboard::Key;

        match key {
            // Navigation
            Key::Named(Named::ArrowRight) | Key::Named(Named::Space) => {
                if let Some(nav) = &mut self.nav {
                    if let Some(p) = nav.next() {
                        return self.start_load(p);
                    }
                }
            }
            Key::Named(Named::ArrowLeft) | Key::Named(Named::Backspace) => {
                if let Some(nav) = &mut self.nav {
                    if let Some(p) = nav.prev() {
                        return self.start_load(p);
                    }
                }
            }

            // Open
            Key::Character(ref c) if c.as_str() == "o" && mods.command() => {
                return self.open_file_dialog();
            }

            // Zoom / view
            Key::Character(ref c) => match c.as_str() {
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
                    // Actual-size (1 image pixel = 1 screen pixel)
                    if let Some(img) = &self.image {
                        let cs = self.canvas_size;
                        let fit = (cs[0] / img.width as f32).min(cs[1] / img.height as f32);
                        self.zoom = 1.0 / fit;
                        self.offset = [0.0, 0.0];
                    }
                }
                _ => {}
            },
            Key::Named(Named::Home) => {
                self.zoom = 1.0;
                self.offset = [0.0, 0.0];
            }
            _ => {}
        }
        Task::none()
    }

    // ---------------------------------------------------------------------------
    // Helpers
    // ---------------------------------------------------------------------------

    fn open_file_dialog(&self) -> Task<Message> {
        Task::perform(
            async {
                rfd::AsyncFileDialog::new()
                    .add_filter("Images", &[
                        "jpg", "jpeg", "png", "gif", "bmp", "tiff", "tif", "webp", "svg",
                        "svgz", "ico", "tga", "qoi", "hdr", "exr",
                    ])
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
        let canvas: Element<ViewerEvent> = shader(ImageCanvas {
            image: self.image.clone(),
            image_id: self.image_id,
            zoom: self.zoom,
            offset: self.offset,
        })
        .width(Length::Fill)
        .height(Length::Fill)
        .into();

        let status = self.status_bar();

        column![canvas.map(Message::Viewer), status].into()
    }

    fn status_bar(&self) -> Element<'_, Message> {
        let s = if let Some(img) = &self.image {
            let name = self.nav.as_ref().map_or(String::new(), |n| n.current_filename());
            let pos = self
                .nav
                .as_ref()
                .map(|n| format!("  {}/{}", n.current_index() + 1, n.count()))
                .unwrap_or_default();
            let zoom_pct = (self.zoom * 100.0) as u32;
            let mb = img.file_size as f64 / 1_048_576.0;
            format!(
                "  {name}  \u{2502}  {w}\u{00d7}{h}  \u{2502}  {mb:.1} MB  \u{2502}  {zoom_pct}%{pos}",
                w = img.width,
                h = img.height,
            )
        } else if self.loading {
            "  Loading\u{2026}".to_string()
        } else if let Some(e) = &self.error {
            format!("  Error: {e}")
        } else {
            "  Ctrl+O to open  \u{2502}  Drag & drop an image  \u{2502}  \u{2190}\u{2192} navigate".to_string()
        };

        container(text(s).size(13).color(Color::from_rgb(0.55, 0.55, 0.55)))
            .width(Length::Fill)
            .padding([5, 10])
            .into()
    }
}
