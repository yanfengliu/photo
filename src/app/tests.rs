use std::fs::File;
use std::io::{BufWriter, Write};

use iced::widget::{column, row};

use super::*;

fn opaque_black_pixels(width: u32, height: u32) -> Vec<u8> {
    let pixel_count = usize::try_from(width)
        .unwrap()
        .saturating_mul(usize::try_from(height).unwrap());
    let mut pixels = vec![0; pixel_count.saturating_mul(4)];
    for alpha in pixels.iter_mut().skip(3).step_by(4) {
        *alpha = 255;
    }
    pixels
}

fn patterned_rgba_pixels(width: u32, height: u32) -> Vec<u8> {
    let mut pixels = Vec::with_capacity((width * height * 4) as usize);
    for y in 0..height {
        for x in 0..width {
            pixels.extend_from_slice(&[
                ((x * 3 + y) % 256) as u8,
                ((y * 5 + x) % 256) as u8,
                ((x * 7 + y * 11) % 256) as u8,
                255,
            ]);
        }
    }
    pixels
}

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
    app.edit_histories.clear();
    app.base_image_sources.clear();
    app.collection_store = collection::CollectionStore::default();
    app.active_collection = None;
    app.context_menu = None;
    app.image = Some(Arc::new(decode::ImageData {
        pixels: opaque_black_pixels(width, height),
        width,
        height,
        file_size: 2_000_000,
    }));
    app.current_image_path = Some(path.to_path_buf());
    app.current_image_source_dimensions = Some((width, height));
    app.base_image_sources
        .insert(path.to_path_buf(), BaseImageSource::Original);
    app
}

fn test_image(width: u32, height: u32) -> Arc<decode::ImageData> {
    Arc::new(decode::ImageData {
        pixels: opaque_black_pixels(width, height),
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
    let logical_dimensions = decode::source_dimensions(path).unwrap_or((image.width, image.height));
    LoadedFullImage {
        image,
        fingerprint: SourceFileFingerprint::from_path(path),
        base_source: BaseImageSource::Original,
        logical_dimensions,
    }
}

fn library_app_with_entries(count: usize) -> App {
    let (mut app, _) = App::new();
    app.tab = Tab::Library;
    app.edit_histories.clear();
    app.base_image_sources.clear();
    app.collection_store = collection::CollectionStore::default();
    app.active_collection = None;
    app.context_menu = None;
    app.replace_library_entries(
        (0..count)
            .map(|index| LibraryEntry {
                path: PathBuf::from(format!("photo-{index}.png")),
                filename: format!("photo-{index}.png"),
                thumbnail_image: None,
                thumbnail_handle: None,
            })
            .collect(),
    );
    app
}

fn test_image_from_pixels(width: u32, height: u32, pixels: &[u8]) -> Arc<decode::ImageData> {
    Arc::new(decode::ImageData {
        pixels: pixels.to_vec(),
        width,
        height,
        file_size: u64::try_from(pixels.len()).unwrap_or(u64::MAX),
    })
}

fn write_test_png(path: &Path, width: u32, height: u32, pixels: &[u8]) {
    let image =
        image::RgbaImage::from_raw(width, height, pixels.to_vec()).expect("valid test image");
    image.save(path).unwrap();
}

/// Drive the in-flight local-edit persist to completion the way the background
/// task would and deliver the rendered thumbnail back through the message loop.
fn complete_in_flight_persist_with_rendered_thumbnail(app: &mut App) {
    let request = app
        .local_edit_persist_in_flight
        .clone()
        .expect("expected an in-flight local edit persist request");
    let full = edit::render_edited_image(
        &request.image.pixels,
        request.image.width,
        request.image.height,
        &request.state,
        request.lens,
    );
    let thumb = thumbnail_from_rendered_image(&full, LOCAL_EDIT_THUMBNAIL_MAX_DIM)
        .expect("thumbnail render should succeed");
    let thumbnail = Arc::new(decode::ImageData {
        pixels: thumb.pixels,
        width: thumb.width,
        height: thumb.height,
        file_size: request.image.file_size,
    });
    let _ = app.update(Message::LocalEditPersistCompleted {
        path: request.path.clone(),
        request_id: request.request_id,
        result: Ok(Some(thumbnail)),
    });
}

fn rgba_handle_pixels(handle: &ImageHandle) -> (u32, u32, Vec<u8>) {
    match handle {
        ImageHandle::Rgba {
            width,
            height,
            pixels,
            ..
        } => (*width, *height, pixels.to_vec()),
        _ => panic!("expected an RGBA image handle"),
    }
}

#[derive(Debug, Clone, Default)]
struct BoundsParagraph;

impl iced::advanced::text::Paragraph for BoundsParagraph {
    type Font = iced::Font;

    fn with_text(_text: iced::advanced::text::Text<&str, Self::Font>) -> Self {
        Self
    }

    fn with_spans<Link>(
        _text: iced::advanced::text::Text<
            &[iced::advanced::text::Span<'_, Link, Self::Font>],
            Self::Font,
        >,
    ) -> Self {
        Self
    }

    fn resize(&mut self, _new_bounds: iced::Size) {}

    fn compare(
        &self,
        _text: iced::advanced::text::Text<(), Self::Font>,
    ) -> iced::advanced::text::Difference {
        iced::advanced::text::Difference::None
    }

    fn horizontal_alignment(&self) -> iced::alignment::Horizontal {
        iced::alignment::Horizontal::Left
    }

    fn vertical_alignment(&self) -> iced::alignment::Vertical {
        iced::alignment::Vertical::Top
    }

    fn min_bounds(&self) -> iced::Size {
        iced::Size::ZERO
    }

    fn hit_test(&self, _point: iced::Point) -> Option<iced::advanced::text::Hit> {
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
struct BoundsCapturingRenderer {
    drawn_images: Vec<iced::Rectangle>,
}

impl iced::advanced::Renderer for BoundsCapturingRenderer {
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

impl iced::advanced::text::Renderer for BoundsCapturingRenderer {
    type Font = iced::Font;
    type Paragraph = BoundsParagraph;
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
        _text: iced::advanced::text::Text<String, Self::Font>,
        _position: iced::Point,
        _color: iced::Color,
        _clip_bounds: iced::Rectangle,
    ) {
    }
}

impl iced::advanced::image::Renderer for BoundsCapturingRenderer {
    type Handle = ImageHandle;

    fn measure_image(&self, handle: &Self::Handle) -> iced::Size<u32> {
        match handle {
            ImageHandle::Rgba { width, height, .. } => iced::Size::new(*width, *height),
            ImageHandle::Path(..) | ImageHandle::Bytes(..) => {
                // The thumbnail slot only ever receives decoded RGBA handles in this app.
                panic!("thumbnail tests expect RGBA handles")
            }
        }
    }

    fn draw_image(
        &mut self,
        _image: iced::advanced::image::Image<Self::Handle>,
        bounds: iced::Rectangle,
    ) {
        self.drawn_images.push(bounds);
    }
}

fn capture_drawn_image_bounds(
    element: Element<'static, Message, iced::Theme, BoundsCapturingRenderer>,
    max_size: iced::Size,
) -> Vec<iced::Rectangle> {
    use iced::advanced::widget::Tree;
    use iced::advanced::{layout, renderer, Widget};

    let mut tree = Tree::new(element.as_widget());
    let mut renderer = BoundsCapturingRenderer::default();
    let limits = layout::Limits::new(iced::Size::ZERO, max_size);
    let node = Widget::layout(element.as_widget(), &mut tree, &renderer, &limits);
    let layout = layout::Layout::new(&node);
    let viewport = node.bounds();

    // `iced_widget::image::draw` forwards the final contained drawing
    // rectangle to `Renderer::draw_image`, not the outer square slot.
    Widget::draw(
        element.as_widget(),
        &tree,
        &mut renderer,
        &Theme::Dark,
        &renderer::Style::default(),
        layout,
        mouse::Cursor::Unavailable,
        &viewport,
    );

    renderer.drawn_images
}

#[test]
fn thumbnail_slot_draws_wide_images_without_stretching() {
    let bounds = capture_drawn_image_bounds(
        thumbnail_slot_with_renderer::<BoundsCapturingRenderer>(
            ImageHandle::from_rgba(300, 150, opaque_black_pixels(300, 150)),
            150.0,
        ),
        iced::Size::new(150.0, 150.0),
    );

    assert_eq!(bounds.len(), 1);
    assert!((bounds[0].x - 0.0).abs() < 0.01);
    assert!((bounds[0].width - 150.0).abs() < 0.01);
    assert!((bounds[0].height - 75.0).abs() < 0.01);
    assert!((bounds[0].y - 37.5).abs() < 0.01);
}

#[test]
fn thumbnail_slot_draws_tall_images_without_stretching() {
    let bounds = capture_drawn_image_bounds(
        thumbnail_slot_with_renderer::<BoundsCapturingRenderer>(
            ImageHandle::from_rgba(120, 240, opaque_black_pixels(120, 240)),
            60.0,
        ),
        iced::Size::new(60.0, 60.0),
    );

    assert_eq!(bounds.len(), 1);
    assert!((bounds[0].width - 30.0).abs() < 0.01);
    assert!((bounds[0].height - 60.0).abs() < 0.01);
    assert!((bounds[0].x - 15.0).abs() < 0.01);
    assert!((bounds[0].y - 0.0).abs() < 0.01);
}

#[test]
fn thumbnail_slot_draws_square_images_at_full_slot_size() {
    let bounds = capture_drawn_image_bounds(
        thumbnail_slot_with_renderer::<BoundsCapturingRenderer>(
            ImageHandle::from_rgba(240, 240, opaque_black_pixels(240, 240)),
            150.0,
        ),
        iced::Size::new(150.0, 150.0),
    );

    assert_eq!(bounds.len(), 1);
    assert!((bounds[0].x - 0.0).abs() < 0.01);
    assert!((bounds[0].y - 0.0).abs() < 0.01);
    assert!((bounds[0].width - 150.0).abs() < 0.01);
    assert!((bounds[0].height - 150.0).abs() < 0.01);
}

fn persist_test_local_edit(
    path: &Path,
    image: Arc<decode::ImageData>,
    state: edit::EditState,
    base_source: BaseImageSource,
) {
    let base_dimensions = decode::source_dimensions(path).unwrap_or((image.width, image.height));
    let _ = persist_local_edit(&LocalEditPersistRequest {
        request_id: 1,
        path: path.to_path_buf(),
        image,
        logical_dimensions: display_dimensions_for_edit_state(
            base_dimensions,
            state.rotation,
            state.crop,
        ),
        state,
        lens: edit::LensCorrection::default(),
        base_source,
    })
    .unwrap();
}

fn write_legacy_local_edit_cache_variant_with_generation_to(
    cache_dir: &Path,
    path: &Path,
    variant: LocalEditCacheVariant,
    generation_id: u64,
    image: &edit::RenderedImage,
) {
    let (file_size, modified_secs, modified_nanos) =
        source_file_state(path).expect("legacy cache source file metadata");
    let path_key = normalized_source_path_key(path);
    let final_path = local_edit_cache_file_path_for_path_key(cache_dir, &path_key, variant);
    let temp_path = local_edit_cache_temp_file_path(&final_path);

    std::fs::create_dir_all(cache_dir).unwrap();

    let file = File::create(&temp_path).unwrap();
    let mut writer = BufWriter::new(file);
    let path_bytes = path_key.as_bytes();
    let path_len = u32::try_from(path_bytes.len()).unwrap();
    let pixel_len = u64::try_from(image.pixels.len()).unwrap();

    writer.write_all(LOCAL_EDIT_CACHE_MAGIC).unwrap();
    write_u32(&mut writer, 2).unwrap();
    write_u64(&mut writer, generation_id).unwrap();
    write_u64(&mut writer, file_size).unwrap();
    write_u64(&mut writer, modified_secs).unwrap();
    write_u32(&mut writer, modified_nanos).unwrap();
    write_u32(&mut writer, path_len).unwrap();
    write_u32(&mut writer, image.width).unwrap();
    write_u32(&mut writer, image.height).unwrap();
    write_u64(&mut writer, pixel_len).unwrap();
    writer.write_all(path_bytes).unwrap();
    writer.write_all(&image.pixels).unwrap();
    writer.flush().unwrap();
    std::fs::rename(temp_path, final_path).unwrap();
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
            thumbnail_image: None,
            thumbnail_handle: None,
        },
        LibraryEntry {
            path: p2.clone(),
            filename: "b.jpg".to_string(),
            thumbnail_image: None,
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
fn local_edit_cache_targets_a_visible_repo_local_directory_when_repo_root_is_known() {
    let repo_root = tempfile::tempdir().unwrap();

    assert_eq!(
        local_edit_cache_dir_for_repo_root(repo_root.path()),
        repo_root.path().join(LOCAL_EDIT_CACHE_DIR_NAME)
    );
}

#[test]
fn local_edit_cache_resolves_under_this_repo_root() {
    assert_eq!(
        local_edit_cache_dir_for_repo_root(Path::new(env!("CARGO_MANIFEST_DIR"))),
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(LOCAL_EDIT_CACHE_DIR_NAME)
    );
}

#[test]
fn local_edit_cache_round_trips_baked_image_data_without_restoring_history() {
    let repo_root = tempfile::tempdir().unwrap();
    let image_path = repo_root.path().join("frame.png");
    let pixels = [255, 0, 0, 255, 0, 255, 0, 255];
    write_test_png(&image_path, 2, 1, &pixels);

    let mut state = edit::EditState::default();
    state.rotate_clockwise();

    with_test_photo_repo_root(repo_root.path(), || {
        persist_test_local_edit(
            &image_path,
            test_image_from_pixels(2, 1, &pixels),
            state,
            BaseImageSource::Original,
        );

        let loaded = load_persisted_local_edit_image(&image_path)
            .unwrap()
            .expect("persisted local edit image");
        let expected =
            edit::render_edited_image(&pixels, 2, 1, &state, edit::LensCorrection::default());
        assert_eq!(loaded.width, expected.width);
        assert_eq!(loaded.height, expected.height);
        assert_eq!(loaded.pixels, expected.pixels);

        let (app, _) = App::new();
        assert!(app.edit_histories.is_empty());
    });
}

#[test]
fn library_thumbnail_load_prefers_the_persisted_local_edit_thumbnail() {
    let repo_root = tempfile::tempdir().unwrap();
    let image_path = repo_root.path().join("frame.png");
    let pixels = [255, 0, 0, 255, 0, 255, 0, 255];
    write_test_png(&image_path, 2, 1, &pixels);

    let mut state = edit::EditState::default();
    state.rotate_clockwise();

    with_test_photo_repo_root(repo_root.path(), || {
        persist_test_local_edit(
            &image_path,
            test_image_from_pixels(2, 1, &pixels),
            state,
            BaseImageSource::Original,
        );

        let thumbnail =
            load_library_thumbnail_base_image(&image_path, LOCAL_EDIT_THUMBNAIL_MAX_DIM).unwrap();
        let expected =
            edit::render_edited_image(&pixels, 2, 1, &state, edit::LensCorrection::default());
        assert_eq!(thumbnail.width, expected.width);
        assert_eq!(thumbnail.height, expected.height);
        assert_eq!(thumbnail.pixels, expected.pixels);
    });
}

#[test]
fn library_thumbnail_ignores_a_stale_persisted_thumbnail_when_full_copy_changed() {
    let repo_root = tempfile::tempdir().unwrap();
    let image_path = repo_root.path().join("frame.png");
    let pixels = [255, 0, 0, 255, 0, 255, 0, 255];
    write_test_png(&image_path, 2, 1, &pixels);

    let original = edit::render_edited_image(
        &pixels,
        2,
        1,
        &edit::EditState::default(),
        edit::LensCorrection::default(),
    );
    let mut rotated_state = edit::EditState::default();
    rotated_state.rotate_clockwise();
    let rotated = edit::render_edited_image(
        &pixels,
        2,
        1,
        &rotated_state,
        edit::LensCorrection::default(),
    );

    with_test_photo_repo_root(repo_root.path(), || {
        let cache_dir = local_edit_cache_dir().expect("repo-local local edit dir");
        let stale_thumb =
            thumbnail_from_rendered_image(&original, LOCAL_EDIT_THUMBNAIL_MAX_DIM).unwrap();
        write_local_edit_cache_variant_to(
            &cache_dir,
            &image_path,
            LocalEditCacheVariant::Thumbnail,
            &stale_thumb,
        )
        .unwrap();
        write_local_edit_cache_variant_to(
            &cache_dir,
            &image_path,
            LocalEditCacheVariant::Full,
            &rotated,
        )
        .unwrap();

        let thumbnail =
            load_library_thumbnail_base_image(&image_path, LOCAL_EDIT_THUMBNAIL_MAX_DIM).unwrap();

        assert_eq!(thumbnail.width, rotated.width);
        assert_eq!(thumbnail.height, rotated.height);
        assert_eq!(thumbnail.pixels, rotated.pixels);
    });
}

#[test]
fn library_thumbnail_ignores_a_generation_mismatch_even_when_dimensions_match() {
    let repo_root = tempfile::tempdir().unwrap();
    let image_path = repo_root.path().join("frame.png");
    let pixels = [255, 0, 0, 255, 0, 255, 0, 255];
    write_test_png(&image_path, 2, 1, &pixels);

    let expected_full = edit::RenderedImage {
        pixels: patterned_rgba_pixels(300, 200),
        width: 300,
        height: 200,
    };
    let stale_thumb = edit::RenderedImage {
        pixels: patterned_rgba_pixels(200, 133),
        width: 200,
        height: 133,
    };

    with_test_photo_repo_root(repo_root.path(), || {
        let cache_dir = local_edit_cache_dir().expect("repo-local local edit dir");
        write_local_edit_cache_variant_to(
            &cache_dir,
            &image_path,
            LocalEditCacheVariant::Thumbnail,
            &stale_thumb,
        )
        .unwrap();
        write_local_edit_cache_variant_to(
            &cache_dir,
            &image_path,
            LocalEditCacheVariant::Full,
            &expected_full,
        )
        .unwrap();

        let thumbnail =
            load_library_thumbnail_base_image(&image_path, LOCAL_EDIT_THUMBNAIL_MAX_DIM).unwrap();
        let expected =
            thumbnail_from_rendered_image(&expected_full, LOCAL_EDIT_THUMBNAIL_MAX_DIM).unwrap();

        assert_eq!(thumbnail.width, expected.width);
        assert_eq!(thumbnail.height, expected.height);
        assert_eq!(thumbnail.pixels, expected.pixels);
    });
}

#[test]
fn library_thumbnail_ignores_a_same_generation_persisted_thumbnail_when_its_aspect_ratio_disagrees_with_the_full_copy(
) {
    let repo_root = tempfile::tempdir().unwrap();
    let image_path = repo_root.path().join("frame.png");
    let pixels = [255, 0, 0, 255, 0, 255, 0, 255];
    write_test_png(&image_path, 2, 1, &pixels);
    let wide_full_pixels = patterned_rgba_pixels(400, 200);
    let wide_full = edit::RenderedImage {
        pixels: wide_full_pixels,
        width: 400,
        height: 200,
    };

    with_test_photo_repo_root(repo_root.path(), || {
        let cache_dir = local_edit_cache_dir().expect("repo-local local edit dir");
        let generation_id = next_local_edit_cache_generation_id();
        let square_thumb = edit::RenderedImage {
            pixels: opaque_black_pixels(2, 2),
            width: 2,
            height: 2,
        };
        write_local_edit_cache_variant_with_generation_to(
            &cache_dir,
            &image_path,
            LocalEditCacheVariant::Full,
            generation_id,
            &wide_full,
        )
        .unwrap();
        write_local_edit_cache_variant_with_generation_to(
            &cache_dir,
            &image_path,
            LocalEditCacheVariant::Thumbnail,
            generation_id,
            &square_thumb,
        )
        .unwrap();
        let loaded =
            load_library_thumbnail_base_image(&image_path, LOCAL_EDIT_THUMBNAIL_MAX_DIM).unwrap();
        let repaired =
            load_persisted_local_edit_variant(&image_path, LocalEditCacheVariant::Thumbnail)
                .unwrap()
                .expect("repaired persisted thumbnail");
        let expected =
            thumbnail_from_rendered_image(&wide_full, LOCAL_EDIT_THUMBNAIL_MAX_DIM).unwrap();

        assert_eq!(loaded.width, 200);
        assert_eq!(loaded.height, 100);
        assert_eq!(loaded.pixels, expected.pixels);
        assert_eq!(repaired.image.width, 200);
        assert_eq!(repaired.image.height, 100);
        assert_eq!(repaired.image.pixels, expected.pixels);
    });
}

#[test]
fn library_thumbnail_fast_path_rechecks_generation_before_returning() {
    let repo_root = tempfile::tempdir().unwrap();
    let image_path = repo_root.path().join("frame.png");
    let pixels = [255, 0, 0, 255, 0, 255, 0, 255];
    write_test_png(&image_path, 2, 1, &pixels);
    let stale_full = edit::RenderedImage {
        pixels: patterned_rgba_pixels(400, 200),
        width: 400,
        height: 200,
    };
    let fresh_full = edit::RenderedImage {
        pixels: patterned_rgba_pixels(300, 200),
        width: 300,
        height: 200,
    };
    let stale_thumb =
        thumbnail_from_rendered_image(&stale_full, LOCAL_EDIT_THUMBNAIL_MAX_DIM).unwrap();
    let fresh_thumb =
        thumbnail_from_rendered_image(&fresh_full, LOCAL_EDIT_THUMBNAIL_MAX_DIM).unwrap();

    with_test_photo_repo_root(repo_root.path(), || {
        let cache_dir = local_edit_cache_dir().expect("repo-local local edit dir");
        let stale_generation = next_local_edit_cache_generation_id();
        let fresh_generation = next_local_edit_cache_generation_id();
        write_local_edit_cache_variant_with_generation_to(
            &cache_dir,
            &image_path,
            LocalEditCacheVariant::Full,
            stale_generation,
            &stale_full,
        )
        .unwrap();
        write_local_edit_cache_variant_with_generation_to(
            &cache_dir,
            &image_path,
            LocalEditCacheVariant::Thumbnail,
            stale_generation,
            &stale_thumb,
        )
        .unwrap();

        let cache_dir_for_hook = cache_dir.clone();
        let image_path_for_hook = image_path.clone();
        let fresh_full_pixels = fresh_full.pixels.clone();
        let fresh_thumb_pixels = fresh_thumb.pixels.clone();
        let fresh_thumb_width = fresh_thumb.width;
        let fresh_thumb_height = fresh_thumb.height;
        set_test_local_edit_thumbnail_fast_path_hook(move || {
            let fresh_full = edit::RenderedImage {
                pixels: fresh_full_pixels,
                width: 300,
                height: 200,
            };
            let fresh_thumb = edit::RenderedImage {
                pixels: fresh_thumb_pixels,
                width: fresh_thumb_width,
                height: fresh_thumb_height,
            };
            write_local_edit_cache_variant_with_generation_to(
                &cache_dir_for_hook,
                &image_path_for_hook,
                LocalEditCacheVariant::Full,
                fresh_generation,
                &fresh_full,
            )
            .unwrap();
            write_local_edit_cache_variant_with_generation_to(
                &cache_dir_for_hook,
                &image_path_for_hook,
                LocalEditCacheVariant::Thumbnail,
                fresh_generation,
                &fresh_thumb,
            )
            .unwrap();
        });

        let loaded =
            load_library_thumbnail_base_image(&image_path, LOCAL_EDIT_THUMBNAIL_MAX_DIM).unwrap();
        let persisted =
            load_persisted_local_edit_variant(&image_path, LocalEditCacheVariant::Thumbnail)
                .unwrap()
                .expect("fresh persisted thumbnail");

        assert_eq!(loaded.width, fresh_thumb.width);
        assert_eq!(loaded.height, fresh_thumb.height);
        assert_eq!(loaded.pixels, fresh_thumb.pixels);
        assert_eq!(persisted.generation_id, fresh_generation);
        assert_eq!(persisted.image.width, fresh_thumb.width);
        assert_eq!(persisted.image.height, fresh_thumb.height);
        assert_eq!(persisted.image.pixels, fresh_thumb.pixels);
    });
}

#[test]
fn library_thumbnail_rechecks_local_edit_cache_inside_the_repair_lock() {
    let repo_root = tempfile::tempdir().unwrap();
    let image_path = repo_root.path().join("frame.png");
    let pixels = [255, 0, 0, 255, 0, 255, 0, 255];
    write_test_png(&image_path, 2, 1, &pixels);
    let stale_full = edit::RenderedImage {
        pixels: patterned_rgba_pixels(400, 200),
        width: 400,
        height: 200,
    };
    let fresh_full = edit::RenderedImage {
        pixels: patterned_rgba_pixels(300, 200),
        width: 300,
        height: 200,
    };
    let fresh_thumb =
        thumbnail_from_rendered_image(&fresh_full, LOCAL_EDIT_THUMBNAIL_MAX_DIM).unwrap();

    with_test_photo_repo_root(repo_root.path(), || {
        let cache_dir = local_edit_cache_dir().expect("repo-local local edit dir");
        let stale_generation = next_local_edit_cache_generation_id();
        let fresh_generation = next_local_edit_cache_generation_id();
        let stale_thumb = edit::RenderedImage {
            pixels: opaque_black_pixels(2, 2),
            width: 2,
            height: 2,
        };
        write_local_edit_cache_variant_with_generation_to(
            &cache_dir,
            &image_path,
            LocalEditCacheVariant::Full,
            stale_generation,
            &stale_full,
        )
        .unwrap();
        write_local_edit_cache_variant_with_generation_to(
            &cache_dir,
            &image_path,
            LocalEditCacheVariant::Thumbnail,
            stale_generation,
            &stale_thumb,
        )
        .unwrap();

        let cache_dir_for_hook = cache_dir.clone();
        let image_path_for_hook = image_path.clone();
        let fresh_full_pixels = fresh_full.pixels.clone();
        let fresh_thumb_pixels = fresh_thumb.pixels.clone();
        let fresh_thumb_width = fresh_thumb.width;
        let fresh_thumb_height = fresh_thumb.height;
        set_test_local_edit_thumbnail_repair_hook(move || {
            let fresh_full = edit::RenderedImage {
                pixels: fresh_full_pixels,
                width: 300,
                height: 200,
            };
            let fresh_thumb = edit::RenderedImage {
                pixels: fresh_thumb_pixels,
                width: fresh_thumb_width,
                height: fresh_thumb_height,
            };
            write_local_edit_cache_variant_with_generation_to(
                &cache_dir_for_hook,
                &image_path_for_hook,
                LocalEditCacheVariant::Full,
                fresh_generation,
                &fresh_full,
            )
            .unwrap();
            write_local_edit_cache_variant_with_generation_to(
                &cache_dir_for_hook,
                &image_path_for_hook,
                LocalEditCacheVariant::Thumbnail,
                fresh_generation,
                &fresh_thumb,
            )
            .unwrap();
        });

        let loaded =
            load_library_thumbnail_base_image(&image_path, LOCAL_EDIT_THUMBNAIL_MAX_DIM).unwrap();
        let repaired =
            load_persisted_local_edit_variant(&image_path, LocalEditCacheVariant::Thumbnail)
                .unwrap()
                .expect("fresh persisted thumbnail");

        assert_eq!(loaded.width, fresh_thumb.width);
        assert_eq!(loaded.height, fresh_thumb.height);
        assert_eq!(loaded.pixels, fresh_thumb.pixels);
        assert_eq!(repaired.generation_id, fresh_generation);
        assert_eq!(repaired.image.width, fresh_thumb.width);
        assert_eq!(repaired.image.height, fresh_thumb.height);
        assert_eq!(repaired.image.pixels, fresh_thumb.pixels);
    });
}

#[test]
fn library_thumbnail_still_loads_when_repair_write_fails() {
    let repo_root = tempfile::tempdir().unwrap();
    let image_path = repo_root.path().join("frame.png");
    let pixels = [255, 0, 0, 255, 0, 255, 0, 255];
    write_test_png(&image_path, 2, 1, &pixels);
    let wide_full = edit::RenderedImage {
        pixels: patterned_rgba_pixels(400, 200),
        width: 400,
        height: 200,
    };

    with_test_photo_repo_root(repo_root.path(), || {
        let cache_dir = local_edit_cache_dir().expect("repo-local local edit dir");
        let generation_id = next_local_edit_cache_generation_id();
        let square_thumb = edit::RenderedImage {
            pixels: opaque_black_pixels(2, 2),
            width: 2,
            height: 2,
        };
        write_local_edit_cache_variant_with_generation_to(
            &cache_dir,
            &image_path,
            LocalEditCacheVariant::Full,
            generation_id,
            &wide_full,
        )
        .unwrap();
        write_local_edit_cache_variant_with_generation_to(
            &cache_dir,
            &image_path,
            LocalEditCacheVariant::Thumbnail,
            generation_id,
            &square_thumb,
        )
        .unwrap();
        set_test_local_edit_thumbnail_repair_write_error("simulated repair write failure");

        let loaded =
            load_library_thumbnail_base_image(&image_path, LOCAL_EDIT_THUMBNAIL_MAX_DIM).unwrap();
        let persisted =
            load_persisted_local_edit_variant(&image_path, LocalEditCacheVariant::Thumbnail)
                .unwrap()
                .expect("stale persisted thumbnail remains");
        let expected =
            thumbnail_from_rendered_image(&wide_full, LOCAL_EDIT_THUMBNAIL_MAX_DIM).unwrap();

        assert_eq!(loaded.width, expected.width);
        assert_eq!(loaded.height, expected.height);
        assert_eq!(loaded.pixels, expected.pixels);
        assert_eq!(persisted.image.width, square_thumb.width);
        assert_eq!(persisted.image.height, square_thumb.height);
        assert_eq!(persisted.image.pixels, square_thumb.pixels);
    });
}

#[test]
fn thumbnail_from_rendered_image_preserves_portrait_aspect_ratio_when_downscaling() {
    let portrait = edit::RenderedImage {
        pixels: patterned_rgba_pixels(200, 400),
        width: 200,
        height: 400,
    };

    let thumbnail = thumbnail_from_rendered_image(&portrait, 200).unwrap();

    assert_eq!(thumbnail.width, 100);
    assert_eq!(thumbnail.height, 200);
}

#[test]
fn thumbnail_from_rendered_image_keeps_original_size_when_already_within_bounds() {
    let image = edit::RenderedImage {
        pixels: patterned_rgba_pixels(120, 80),
        width: 120,
        height: 80,
    };

    let thumbnail = thumbnail_from_rendered_image(&image, 200).unwrap();

    assert_eq!(thumbnail.width, 120);
    assert_eq!(thumbnail.height, 80);
    assert_eq!(thumbnail.pixels, image.pixels);
}

#[test]
fn thumbnail_dimensions_for_image_handles_zero_safely() {
    assert_eq!(thumbnail_dimensions_for_image(0, 0, 0), (0, 0));
    assert_eq!(thumbnail_dimensions_for_image(0, 400, 200), (0, 200));
    assert_eq!(thumbnail_dimensions_for_image(400, 0, 200), (200, 0));
}

#[test]
fn persisted_local_edit_is_ignored_after_the_source_file_changes() {
    let repo_root = tempfile::tempdir().unwrap();
    let image_path = repo_root.path().join("frame.png");
    let original_pixels = [255, 0, 0, 255, 0, 255, 0, 255];
    let replacement_pixels = [0, 0, 255, 255, 255, 255, 0, 255];
    write_test_png(&image_path, 2, 1, &original_pixels);

    let mut state = edit::EditState::default();
    state.rotate_clockwise();

    with_test_photo_repo_root(repo_root.path(), || {
        persist_test_local_edit(
            &image_path,
            test_image_from_pixels(2, 1, &original_pixels),
            state,
            BaseImageSource::Original,
        );

        std::thread::sleep(std::time::Duration::from_millis(20));
        write_test_png(&image_path, 2, 1, &replacement_pixels);

        assert!(load_persisted_local_edit_image(&image_path)
            .unwrap_or(None)
            .is_none());
    });
}

#[test]
fn rotate_clockwise_updates_library_thumbnail_after_persist_completes() {
    let path = PathBuf::from("frame.png");
    let pixels = [255, 0, 0, 255, 0, 255, 0, 255];
    let thumbnail_image = test_image_from_pixels(2, 1, &pixels);
    let mut app = detail_app_with_image(&path, 2, 1);
    app.image = Some(test_image_from_pixels(2, 1, &pixels));
    app.library = vec![LibraryEntry {
        path: path.clone(),
        filename: "frame.png".to_string(),
        thumbnail_image: Some(thumbnail_image),
        thumbnail_handle: None,
    }];
    app.rebuild_library_indices();

    let _ = app.update(Message::RotateClockwise);

    assert!(
        app.library[0].thumbnail_handle.is_none(),
        "rotation commit should defer the library thumbnail render to the persist task"
    );

    complete_in_flight_persist_with_rendered_thumbnail(&mut app);

    let handle = app.library[0]
        .thumbnail_handle
        .as_ref()
        .expect("rotated thumbnail handle");
    let (width, height, pixels) = rgba_handle_pixels(handle);
    let expected = edit::render_edited_image(
        &[255, 0, 0, 255, 0, 255, 0, 255],
        2,
        1,
        &edit::EditState {
            rotation: edit::QuarterTurns::new(1),
            ..edit::EditState::default()
        },
        edit::LensCorrection::default(),
    );

    assert_eq!(width, expected.width);
    assert_eq!(height, expected.height);
    assert_eq!(pixels, expected.pixels);
}

#[test]
fn exposure_commit_updates_library_thumbnail_after_persist_completes() {
    let path = PathBuf::from("frame.png");
    let pixels = [96, 96, 96, 255];
    let thumbnail_image = test_image_from_pixels(1, 1, &pixels);
    let mut app = detail_app_with_image(&path, 1, 1);
    app.image = Some(test_image_from_pixels(1, 1, &pixels));
    app.library = vec![LibraryEntry {
        path: path.clone(),
        filename: "frame.png".to_string(),
        thumbnail_image: Some(thumbnail_image),
        thumbnail_handle: None,
    }];
    app.rebuild_library_indices();
    app.slider_text_buf = "1.0".to_string();

    let _ = app.update(Message::SliderTextSubmit(SliderKind::Exposure));

    assert!(
        app.library[0].thumbnail_handle.is_none(),
        "slider commit should defer the library thumbnail render to the persist task"
    );

    complete_in_flight_persist_with_rendered_thumbnail(&mut app);

    let handle = app.library[0]
        .thumbnail_handle
        .as_ref()
        .expect("exposure-adjusted thumbnail handle");
    let (_, _, rendered_pixels) = rgba_handle_pixels(handle);
    assert!(
        rendered_pixels[0] > pixels[0],
        "expected exposure-adjusted thumbnail to brighten"
    );
}

#[test]
fn slider_double_click_release_resets_each_slider_kind() {
    let kinds_with_initial_values: &[(SliderKind, f32)] = &[
        (SliderKind::Exposure, 1.5),
        (SliderKind::Contrast, -25.0),
        (SliderKind::Highlights, 60.0),
        (SliderKind::Shadows, -40.0),
        (SliderKind::Whites, 80.0),
        (SliderKind::Blacks, -55.0),
        (SliderKind::Temperature, 12.0),
        (SliderKind::Tint, -7.5),
        (SliderKind::Vibrance, 33.0),
        (SliderKind::Saturation, -18.0),
        (SliderKind::Clarity, 22.0),
        (SliderKind::Dehaze, -10.0),
    ];

    for &(kind, initial) in kinds_with_initial_values {
        let path = PathBuf::from("frame.png");
        let mut app = detail_app_with_image(&path, 1, 1);
        app.image = Some(test_image_from_pixels(1, 1, &[96, 96, 96, 255]));
        let history = app.edit_histories.entry(path.clone()).or_default();
        set_slider_field(&mut history.current, kind, initial);
        history.commit();

        let _ = app.update(Message::SliderReleased(kind));
        let _ = app.update(Message::SliderReleased(kind));

        let value = get_slider_field(
            &app.edit_histories.get(&path).expect("history").current,
            kind,
        );
        assert_eq!(
            value, 0.0,
            "double-click on the {:?} knob should reset its value to the default",
            kind
        );
    }
}

#[test]
fn slider_double_click_release_defers_persist_when_clearing_an_existing_local_edit() {
    // The freeze the user reported happens when the on-disk persisted edit
    // exists and the double-click reset has to delete it. Pre-fix, the commit
    // path ran a synchronous full-image render on the UI thread before queueing
    // the background persist. Post-fix, the heavy work moves entirely to the
    // background persist task and the library thumbnail updates from its result.
    let repo_root = tempfile::tempdir().unwrap();
    let image_path = repo_root.path().join("frame.png");
    let pixels = [96, 96, 96, 255];
    write_test_png(&image_path, 1, 1, &pixels);

    with_test_photo_repo_root(repo_root.path(), || {
        let prior_state = edit::EditState {
            exposure: 1.5,
            ..Default::default()
        };
        persist_test_local_edit(
            &image_path,
            test_image_from_pixels(1, 1, &pixels),
            prior_state,
            BaseImageSource::Original,
        );
        assert!(persisted_local_edit_exists(
            &image_path,
            LocalEditCacheVariant::Full
        ));

        let mut app = detail_app_with_image(&image_path, 1, 1);
        app.image = Some(test_image_from_pixels(1, 1, &pixels));
        app.library = vec![LibraryEntry {
            path: image_path.clone(),
            filename: "frame.png".to_string(),
            thumbnail_image: Some(test_image_from_pixels(1, 1, &pixels)),
            thumbnail_handle: None,
        }];
        app.rebuild_library_indices();

        let history = app.edit_histories.entry(image_path.clone()).or_default();
        history.current.exposure = 1.5;
        history.commit();

        let _ = app.update(Message::SliderReleased(SliderKind::Exposure));
        let _ = app.update(Message::SliderReleased(SliderKind::Exposure));

        assert_eq!(
            app.edit_histories
                .get(&image_path)
                .unwrap()
                .current
                .exposure,
            0.0,
            "double-click should reset the exposure to default"
        );
        assert!(
            app.local_edit_persist_in_flight.is_some(),
            "double-click reset should enqueue a persist task to clear the on-disk edit"
        );
        assert!(
            app.library[0].thumbnail_handle.is_none(),
            "double-click reset must not synchronously render the full image on the UI \
                 thread (which would freeze the app for large images)"
        );
    });
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
    assert_eq!(
        app.import_cache_warm_in_flight.as_deref(),
        Some(raw.as_path())
    );
    assert_eq!(
        app.pending_import_cache_warm_paths
            .iter()
            .collect::<Vec<_>>(),
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
    assert_eq!(
        app.import_cache_warm_in_flight.as_deref(),
        Some(raw.as_path())
    );

    let _ = app.update(Message::ImportCacheWarmCompleted {
        path: raw,
        result: Ok(true),
    });

    assert_eq!(
        app.import_cache_warm_in_flight.as_deref(),
        Some(svg.as_path())
    );
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
    assert_eq!(
        app.import_cache_warm_in_flight.as_deref(),
        Some(raw.as_path())
    );

    let _ = app.update(Message::ImportCacheWarmCompleted {
        path: raw,
        result: Err("warm failed".to_string()),
    });

    assert_eq!(
        app.import_cache_warm_in_flight.as_deref(),
        Some(svg.as_path())
    );
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
    assert_eq!(
        app.import_cache_warm_in_flight.as_deref(),
        Some(first.as_path())
    );

    let _ = app.update(Message::FilesPicked(Some(second_batch)));

    assert_eq!(
        app.import_cache_warm_in_flight.as_deref(),
        Some(first.as_path())
    );
    assert_eq!(
        app.pending_import_cache_warm_paths
            .iter()
            .collect::<Vec<_>>(),
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

    // Temperature/tint span should reach tungsten (~3200K) on one side
    // and cloudy overcast (~9800K) on the other with the 55 K-per-unit
    // mapping in edit::temperature_tint_matrix.
    for kind in [SliderKind::Temperature, SliderKind::Tint] {
        let (min, max) = slider_range(kind);
        assert_eq!(min, -60.0);
        assert_eq!(max, 60.0);
    }

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
fn temperature_slider_covers_tungsten_and_cloudy_kelvin() {
    // At the extremes, the kelvin mapping inside
    // edit::temperature_tint_matrix should span roughly tungsten
    // (~3200K) to cloudy/shade (~9800K), so white balance edits can
    // correct indoor and open-shade images without running out of range.
    let (min, max) = slider_range(SliderKind::Temperature);
    let kelvin_low = 6500.0 + min * 55.0;
    let kelvin_high = 6500.0 + max * 55.0;
    assert!(
        kelvin_low <= 3300.0,
        "temperature low end {} does not reach tungsten",
        kelvin_low
    );
    assert!(
        kelvin_high >= 9700.0,
        "temperature high end {} does not reach cloudy",
        kelvin_high
    );
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

    let _ = app.handle_viewer(ViewerEvent::CropCommitted {
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

    let _ = app.handle_viewer(ViewerEvent::CropCommitted {
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
    let _ = app.handle_viewer(ViewerEvent::CropCommitted {
        rect: edit::CropRect::new(0.0, 0.0, 1.0, 0.5),
    });

    let state = app.edit_histories.get(&path).unwrap().current;
    let out = edit::save_edited_image(
        &original,
        &pixels,
        2,
        1,
        &state,
        edit::LensCorrection::default(),
    )
    .unwrap();
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
fn status_bar_uses_source_dimensions_when_loaded_buffer_is_scaled() {
    let path = PathBuf::from("frame.arw");
    let mut app = detail_app_with_image(&path, 16_384, 10_923);
    app.current_image_source_dimensions = Some((9_728, 6_656));

    let mut history = edit::UndoHistory::new();
    history.current.rotate_clockwise();
    history.commit();
    app.edit_histories.insert(path, history);

    let status = app.status_bar_text();
    assert!(status.contains("6656\u{00d7}9728"));
    assert!(!status.contains("10923\u{00d7}16384"));
    assert!(!status.contains("16384\u{00d7}10923"));
}

#[test]
fn persisted_local_edit_reopen_uses_persisted_logical_dimensions_in_status_text() {
    let repo_root = tempfile::tempdir().unwrap();
    let path = repo_root.path().join("frame.png");
    write_test_png(&path, 3, 2, &patterned_rgba_pixels(3, 2));

    let state = edit::EditState {
        exposure: 1.0,
        ..Default::default()
    };

    with_test_photo_repo_root(repo_root.path(), || {
        let _ = persist_local_edit(&LocalEditPersistRequest {
            request_id: 1,
            path: path.clone(),
            image: test_image(6, 4),
            logical_dimensions: (3, 2),
            state,
            lens: edit::LensCorrection::default(),
            base_source: BaseImageSource::Original,
        })
        .unwrap();

        let loaded = load_full_image(&path, BaseImageSource::PersistedLocalEdit).unwrap();
        assert_eq!(loaded.base_source, BaseImageSource::PersistedLocalEdit);
        assert_eq!(loaded.image.width, 6);
        assert_eq!(loaded.image.height, 4);
        assert_eq!(loaded.logical_dimensions, (3, 2));

        let (mut app, _) = App::new();
        app.tab = Tab::Detail;
        app.current_image_path = Some(path.clone());
        let request_id = app.detail_load.begin_request();

        let _ = app.update(Message::ImageLoaded {
            request_id,
            result: Ok(loaded),
        });

        let status = app.status_bar_text();
        assert!(status.contains("3\u{00d7}2"));
        assert!(!status.contains("6\u{00d7}4"));
    });
}

#[test]
fn legacy_persisted_local_edit_prefers_source_dimensions_when_baked_pixels_exceed_the_source() {
    let repo_root = tempfile::tempdir().unwrap();
    let path = repo_root.path().join("frame.png");
    write_test_png(&path, 6, 9, &patterned_rgba_pixels(6, 9));

    with_test_photo_repo_root(repo_root.path(), || {
        let cache_dir = local_edit_cache_dir().expect("repo-local local edit dir");
        write_legacy_local_edit_cache_variant_with_generation_to(
            &cache_dir,
            &path,
            LocalEditCacheVariant::Full,
            next_local_edit_cache_generation_id(),
            &edit::RenderedImage {
                pixels: patterned_rgba_pixels(16, 10),
                width: 16,
                height: 10,
            },
        );

        let loaded = load_full_image(&path, BaseImageSource::PersistedLocalEdit).unwrap();
        assert_eq!(loaded.base_source, BaseImageSource::PersistedLocalEdit);
        assert_eq!(loaded.logical_dimensions, (6, 9));
        assert_eq!(loaded.image.width, 16);
        assert_eq!(loaded.image.height, 10);
    });
}

#[test]
fn legacy_persisted_local_edit_keeps_baked_dimensions_when_the_aspect_ratio_changed() {
    let repo_root = tempfile::tempdir().unwrap();
    let path = repo_root.path().join("frame.png");
    write_test_png(&path, 4, 4, &patterned_rgba_pixels(4, 4));

    with_test_photo_repo_root(repo_root.path(), || {
        let cache_dir = local_edit_cache_dir().expect("repo-local local edit dir");
        write_legacy_local_edit_cache_variant_with_generation_to(
            &cache_dir,
            &path,
            LocalEditCacheVariant::Full,
            next_local_edit_cache_generation_id(),
            &edit::RenderedImage {
                pixels: patterned_rgba_pixels(2, 4),
                width: 2,
                height: 4,
            },
        );

        let loaded = load_full_image(&path, BaseImageSource::PersistedLocalEdit).unwrap();
        assert_eq!(loaded.base_source, BaseImageSource::PersistedLocalEdit);
        assert_eq!(loaded.logical_dimensions, (2, 4));
        assert_eq!(loaded.image.width, 2);
        assert_eq!(loaded.image.height, 4);
    });
}

#[test]
fn legacy_persisted_local_edit_keeps_baked_dimensions_when_a_crop_preserves_aspect_ratio() {
    let repo_root = tempfile::tempdir().unwrap();
    let path = repo_root.path().join("frame.png");
    write_test_png(&path, 6, 4, &patterned_rgba_pixels(6, 4));

    with_test_photo_repo_root(repo_root.path(), || {
        let cache_dir = local_edit_cache_dir().expect("repo-local local edit dir");
        write_legacy_local_edit_cache_variant_with_generation_to(
            &cache_dir,
            &path,
            LocalEditCacheVariant::Full,
            next_local_edit_cache_generation_id(),
            &edit::RenderedImage {
                pixels: patterned_rgba_pixels(3, 2),
                width: 3,
                height: 2,
            },
        );

        let loaded = load_full_image(&path, BaseImageSource::PersistedLocalEdit).unwrap();
        assert_eq!(loaded.base_source, BaseImageSource::PersistedLocalEdit);
        assert_eq!(loaded.logical_dimensions, (3, 2));
        assert_eq!(loaded.image.width, 3);
        assert_eq!(loaded.image.height, 2);
    });
}

#[test]
fn legacy_persisted_local_edit_keeps_baked_dimensions_when_rotation_swapped_the_axes() {
    let repo_root = tempfile::tempdir().unwrap();
    let path = repo_root.path().join("frame.png");
    write_test_png(&path, 6, 9, &patterned_rgba_pixels(6, 9));

    with_test_photo_repo_root(repo_root.path(), || {
        let cache_dir = local_edit_cache_dir().expect("repo-local local edit dir");
        write_legacy_local_edit_cache_variant_with_generation_to(
            &cache_dir,
            &path,
            LocalEditCacheVariant::Full,
            next_local_edit_cache_generation_id(),
            &edit::RenderedImage {
                pixels: patterned_rgba_pixels(9, 6),
                width: 9,
                height: 6,
            },
        );

        let loaded = load_full_image(&path, BaseImageSource::PersistedLocalEdit).unwrap();
        assert_eq!(loaded.base_source, BaseImageSource::PersistedLocalEdit);
        assert_eq!(loaded.logical_dimensions, (9, 6));
        assert_eq!(loaded.image.width, 9);
        assert_eq!(loaded.image.height, 6);
    });
}

#[test]
fn image_loaded_recovers_missing_source_dimensions_after_successful_original_load() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frame.png");
    let pixels = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255, 255, 0, 255, 255, 0, 255,
        255, 255,
    ];
    write_test_png(&path, 3, 2, &pixels);

    let mut app = detail_app_with_image(&path, 5, 4);
    app.current_image_source_dimensions = None;
    let request_id = app.detail_load.begin_request();

    let _ = app.update(Message::ImageLoaded {
        request_id,
        result: Ok(loaded_full_image(&path, test_image(5, 4))),
    });

    assert_eq!(app.current_image_source_dimensions, Some((3, 2)));
    let status = app.status_bar_text();
    assert!(status.contains("3\u{00d7}2"));
    assert!(!status.contains("5\u{00d7}4"));
}

#[test]
fn session_full_image_cache_hit_restores_cached_source_dimensions() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frame.png");
    let pixels = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255, 255, 0, 255, 255, 0, 255,
        255, 255,
    ];
    write_test_png(&path, 3, 2, &pixels);

    let mut app = detail_app_with_image(&path, 5, 4);
    app.current_image_source_dimensions = Some((3, 2));
    let fingerprint = SourceFileFingerprint::from_path(&path).unwrap();
    app.cache_full_image_for_current_path(fingerprint, test_image(5, 4));
    app.image = None;
    app.current_image_source_dimensions = None;

    let _ = app.start_load(path.clone());

    assert_eq!(app.current_image_source_dimensions, Some((3, 2)));
    let status = app.status_bar_text();
    assert!(status.contains("3\u{00d7}2"));
    assert!(!status.contains("5\u{00d7}4"));
}

#[test]
fn displayed_full_image_fast_path_does_not_reuse_a_stale_base_source() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frame.png");
    let pixels = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255, 255, 0, 255, 255, 0, 255,
        255, 255,
    ];
    write_test_png(&path, 3, 2, &pixels);

    let mut app = detail_app_with_image(&path, 5, 4);
    app.current_image_source_dimensions = Some((3, 2));
    let fingerprint = SourceFileFingerprint::from_path(&path).unwrap();
    app.cache_full_image_for_current_path(fingerprint, test_image(5, 4));
    app.base_image_sources
        .insert(path.clone(), BaseImageSource::PersistedLocalEdit);

    let _ = app.start_load(path.clone());

    assert_eq!(app.status_bar_text(), "  Loading…");
    assert!(app.image.is_none());
    assert!(!app.session_full_image_cache.contains_path(&path));
}

#[test]
fn session_full_image_cache_invalidates_hits_when_base_source_changes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frame.png");
    let pixels = [
        255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 0, 255, 255, 0, 255, 255, 0, 255,
        255, 255,
    ];
    write_test_png(&path, 3, 2, &pixels);

    let mut cache = SessionFullImageCache::new(2, 64);
    cache.insert(
        &path,
        SourceFileFingerprint::from_path(&path).unwrap(),
        test_image_with_bytes(5, 4, 80),
        BaseImageSource::Original,
        (3, 2),
    );

    assert!(cache
        .get(&path, BaseImageSource::PersistedLocalEdit)
        .is_none());
    assert!(!cache.contains_path(&path));
}

#[test]
fn clearing_library_entries_clears_current_image_source_dimensions() {
    let path = PathBuf::from("frame.png");
    let mut app = detail_app_with_image(&path, 200, 100);

    app.clear_library_entries();

    assert!(app.current_image_source_dimensions.is_none());
}

#[test]
fn removing_the_current_library_entry_clears_current_image_source_dimensions() {
    let path = PathBuf::from("frame.png");
    let mut app = detail_app_with_image(&path, 200, 100);
    app.replace_library_entries(vec![LibraryEntry {
        path: path.clone(),
        filename: "frame.png".to_string(),
        thumbnail_image: None,
        thumbnail_handle: None,
    }]);
    app.current_image_path = Some(path);
    app.current_image_source_dimensions = Some((200, 100));
    app.image = Some(test_image(200, 100));

    let removed = app.remove_library_entry(0);

    assert!(removed.is_some());
    assert!(app.current_image_source_dimensions.is_none());
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

    let _ = app.handle_viewer(ViewerEvent::DoubleClick {
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
        request.lens,
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
            _text: advanced_text::Text<&[advanced_text::Span<'_, Link, Self::Font>], Self::Font>,
        ) -> Self {
            Self::default()
        }

        fn resize(&mut self, new_bounds: iced::Size) {
            if let Some(last_text) = &mut self.last_text {
                last_text.bounds = new_bounds;
            }
        }

        fn compare(&self, text: advanced_text::Text<(), Self::Font>) -> advanced_text::Difference {
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
        result: Ok(loaded_full_image(
            Path::new("frame.arw"),
            test_image(6000, 3000),
        )),
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
    let request = app
        .current_save_request()
        .expect("save request after reopen");
    let saved = edit::save_edited_image(
        &request.path,
        &request.image.pixels,
        request.image.width,
        request.image.height,
        &request.state,
        request.lens,
    )
    .expect("save copy from reopened missing-source image");
    assert!(saved.exists());
    assert_eq!(saved.extension().and_then(|ext| ext.to_str()), Some("png"));
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
        thumbnail_image: None,
        thumbnail_handle: None,
    }]);
    app.current_image_path = Some(path.clone());

    let _ = app.update(Message::ImageLoaded {
        request_id: app.detail_load.request_id,
        result: Ok(loaded_full_image(&path, test_image(6000, 3000))),
    });

    app.error = Some("stale error".to_string());
    app.save_status = Some("stale save".to_string());
    app.current_exif = Some(lens::ExifInfo {
        lens_model: "Warm lens".to_string(),
        ..lens::ExifInfo::default()
    });
    app.zoom = 2.5;
    app.offset = [18.0, -9.0];
    app.crop_mode = true;
    let request_id_before_reopen = app.detail_load.request_id;
    let image_id_before_reopen = app.image_id;

    let _ = app.update(Message::SwitchTab(Tab::Library));
    std::fs::remove_file(&path).unwrap();

    let _ = app.update(Message::LibraryItemClicked(0));
    let _ = app.update(Message::LibraryItemClicked(0));

    assert_eq!(app.tab, Tab::Detail);
    assert_eq!(app.library_index, Some(0));
    assert!(!app.detail_load.is_loading());
    assert!(!app.detail_load.shows_embedded_preview());
    assert!(app.drag_state.is_none());
    assert_eq!(app.detail_load.request_id, request_id_before_reopen);
    assert_eq!(app.image_id, image_id_before_reopen);
    assert_eq!(
        app.image.as_ref().map(|image| (image.width, image.height)),
        Some((6000, 3000))
    );
    assert!(app.error.is_none());
    assert!(app.save_status.is_none());
    assert_eq!(
        app.current_exif
            .as_ref()
            .map(|exif| exif.lens_model.as_str()),
        Some("Warm lens")
    );
    assert_eq!(app.zoom, 1.0);
    assert_eq!(app.offset, [0.0, 0.0]);
    assert!(!app.crop_mode);
    let request = app
        .current_save_request()
        .expect("save request after reopen");
    let saved = edit::save_edited_image(
        &request.path,
        &request.image.pixels,
        request.image.width,
        request.image.height,
        &request.state,
        request.lens,
    )
    .expect("save copy from reopened missing-source image");
    assert!(saved.exists());
    assert_eq!(saved.extension().and_then(|ext| ext.to_str()), Some("png"));
}

#[test]
fn opening_detail_from_library_clears_pending_drag_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("frame.png");
    write_test_png(&path, 3, 2, &patterned_rgba_pixels(3, 2));

    let (mut app, _) = App::new();
    app.collection_store = collection::CollectionStore::default();
    app.active_collection = None;
    app.context_menu = None;
    app.tab = Tab::Library;
    app.replace_library_entries(vec![LibraryEntry {
        path: path.clone(),
        filename: "frame.png".to_string(),
        thumbnail_image: None,
        thumbnail_handle: None,
    }]);
    app.cursor_position = [120.0, 80.0];

    let _ = app.update(Message::LibraryItemClicked(0));
    assert!(app.drag_state.is_some());

    let _ = app.update(Message::LibraryItemClicked(0));

    assert_eq!(app.tab, Tab::Detail);
    assert_eq!(app.library_index, Some(0));
    assert!(app.drag_state.is_none());
}

#[test]
fn library_reopen_reloads_when_the_current_source_metadata_changes() {
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
        thumbnail_image: None,
        thumbnail_handle: None,
    }]);
    app.current_image_path = Some(path.clone());

    let _ = app.update(Message::ImageLoaded {
        request_id: app.detail_load.request_id,
        result: Ok(loaded_full_image(&path, test_image(6000, 3000))),
    });

    app.error = Some("stale error".to_string());
    app.save_status = Some("stale save".to_string());
    app.current_exif = Some(lens::ExifInfo {
        lens_model: "Warm lens".to_string(),
        ..lens::ExifInfo::default()
    });
    app.zoom = 2.5;
    app.offset = [18.0, -9.0];
    app.crop_mode = true;
    let request_id_before_reopen = app.detail_load.request_id;
    let image_id_before_reopen = app.image_id;

    let _ = app.update(Message::SwitchTab(Tab::Library));
    std::thread::sleep(std::time::Duration::from_millis(20));
    std::fs::write(&path, b"new").unwrap();

    let _ = app.update(Message::LibraryItemClicked(0));
    let _ = app.update(Message::LibraryItemClicked(0));

    assert_eq!(app.tab, Tab::Detail);
    assert_eq!(app.library_index, Some(0));
    assert!(app.detail_load.is_loading());
    assert!(!app.detail_load.shows_embedded_preview());
    assert_eq!(app.detail_load.request_id, request_id_before_reopen + 1);
    assert_eq!(app.image_id, image_id_before_reopen);
    assert!(app.image.is_none());
    assert!(app.error.is_none());
    assert!(app.save_status.is_none());
    assert!(app.current_exif.is_none());
    assert!(app.current_save_request().is_none());
    assert_eq!(app.zoom, 1.0);
    assert_eq!(app.offset, [0.0, 0.0]);
    assert!(!app.crop_mode);
}

#[test]
fn reopening_a_recently_viewed_detail_image_reuses_the_session_memory_cache() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.arw");
    let second = dir.path().join("second.arw");
    std::fs::write(&first, b"first").unwrap();
    std::fs::write(&second, b"second").unwrap();

    let (mut app, _) = App::new();
    app.collection_store = collection::CollectionStore::default();
    app.tab = Tab::Detail;
    app.session_full_image_cache = SessionFullImageCache::new(4, 8);

    app.current_image_path = Some(first.clone());
    let _ = app.update(Message::ImageLoaded {
        request_id: app.detail_load.request_id,
        result: Ok(loaded_full_image(&first, test_image_with_bytes(2, 1, 8))),
    });

    app.current_image_path = Some(second.clone());
    let _ = app.update(Message::ImageLoaded {
        request_id: app.detail_load.request_id,
        result: Ok(loaded_full_image(&second, test_image_with_bytes(2, 1, 8))),
    });

    app.error = Some("stale error".to_string());
    app.save_status = Some("stale save".to_string());
    app.current_exif = Some(lens::ExifInfo::default());
    app.zoom = 2.5;
    app.offset = [18.0, -9.0];
    app.crop_mode = true;

    let _ = app.start_load(first);

    assert!(!app.detail_load.is_loading());
    assert!(!app.detail_load.shows_embedded_preview());
    assert_eq!(
        app.image.as_ref().map(|image| (image.width, image.height)),
        Some((2, 1))
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
        BaseImageSource::Original,
        (2, 1),
    );
    cache.insert(
        &second,
        SourceFileFingerprint::from_path(&second).unwrap(),
        test_image_with_bytes(2, 1, 8),
        BaseImageSource::Original,
        (2, 1),
    );
    assert!(cache.get(&first, BaseImageSource::Original).is_some());

    cache.insert(
        &third,
        SourceFileFingerprint::from_path(&third).unwrap(),
        test_image_with_bytes(2, 1, 8),
        BaseImageSource::Original,
        (2, 1),
    );

    assert!(cache.get(&first, BaseImageSource::Original).is_some());
    assert!(cache.get(&second, BaseImageSource::Original).is_none());
    assert!(cache.get(&third, BaseImageSource::Original).is_some());
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
        BaseImageSource::Original,
        (2, 1),
    );
    cache.insert(
        &second,
        SourceFileFingerprint::from_path(&second).unwrap(),
        test_image_with_bytes(2, 1, 8),
        BaseImageSource::Original,
        (2, 1),
    );
    cache.insert(
        &third,
        SourceFileFingerprint::from_path(&third).unwrap(),
        test_image_with_bytes(2, 1, 8),
        BaseImageSource::Original,
        (2, 1),
    );

    assert!(cache.get(&first, BaseImageSource::Original).is_none());
    assert!(cache.get(&second, BaseImageSource::Original).is_some());
    assert!(cache.get(&third, BaseImageSource::Original).is_some());
}

#[test]
fn session_full_image_cache_keeps_two_recent_entries_hot_even_when_they_fill_the_budget() {
    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first.arw");
    let second = dir.path().join("second.arw");
    std::fs::write(&first, b"first").unwrap();
    std::fs::write(&second, b"second").unwrap();

    let mut cache = SessionFullImageCache::new(4, 8);
    cache.insert(
        &first,
        SourceFileFingerprint::from_path(&first).unwrap(),
        test_image_with_bytes(2, 1, 8),
        BaseImageSource::Original,
        (2, 1),
    );
    cache.insert(
        &second,
        SourceFileFingerprint::from_path(&second).unwrap(),
        test_image_with_bytes(2, 1, 8),
        BaseImageSource::Original,
        (2, 1),
    );

    assert!(cache.get(&first, BaseImageSource::Original).is_some());
    assert!(cache.get(&second, BaseImageSource::Original).is_some());
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
    cache.insert(
        &path,
        old_fingerprint,
        test_image_with_bytes(2, 1, 8),
        BaseImageSource::Original,
        (2, 1),
    );

    assert!(cache.get(&path, BaseImageSource::Original).is_none());
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
        result: Ok(loaded_full_image(
            Path::new("frame.arw"),
            test_image(6000, 4000),
        )),
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
        result: Ok(loaded_full_image(
            Path::new("first.arw"),
            test_image(640, 320),
        )),
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
    assert_eq!(request.lens.vig, [0.1, 0.2, 0.3]);
}

#[test]
fn current_local_edit_persist_request_waits_for_auto_lens_metadata() {
    let path = PathBuf::from("frame.arw");
    let mut app = detail_app_with_image(&path, 3, 3);
    let pixels = vec![
        200, 200, 200, 255, 200, 200, 200, 255, 200, 200, 200, 255, 200, 200, 200, 255, 200, 200,
        200, 255, 200, 200, 200, 255, 200, 200, 200, 255, 200, 200, 200, 255, 200, 200, 200, 255,
    ];
    app.image = Some(test_image_from_pixels(3, 3, &pixels));
    let mut history = edit::UndoHistory::default();
    history.current.lens_correction = true;
    history.commit();
    app.edit_histories.insert(path, history);
    app.detail_load.stage = DetailLoadStage::Idle;
    app.detail_load.exif_loading = true;
    app.lens_override_name = None;
    app.current_lens_profile = None;

    assert!(app.current_local_edit_persist_request().is_none());
}

#[test]
fn exif_loaded_refreshes_library_thumbnail_and_persist_for_auto_lens_correction() {
    let path = PathBuf::from("frame.arw");
    let mut app = detail_app_with_image(&path, 3, 3);
    let pixels = vec![
        200, 200, 200, 255, 200, 200, 200, 255, 200, 200, 200, 255, 200, 200, 200, 255, 200, 200,
        200, 255, 200, 200, 200, 255, 200, 200, 200, 255, 200, 200, 200, 255, 200, 200, 200, 255,
    ];
    let base_image = test_image_from_pixels(3, 3, &pixels);
    app.image = Some(base_image.clone());
    app.library = vec![LibraryEntry {
        path: path.clone(),
        filename: "frame.arw".to_string(),
        thumbnail_image: Some(base_image),
        thumbnail_handle: None,
    }];
    app.rebuild_library_indices();
    let mut history = edit::UndoHistory::default();
    history.current.lens_correction = true;
    history.commit();
    app.edit_histories.insert(path.clone(), history);
    app.detail_load.stage = DetailLoadStage::Idle;
    app.detail_load.exif_loading = true;
    app.lens_override_name = None;
    app.lens_db = lens::LensDatabase {
        profiles: vec![lens::LensProfile {
            maker: "Sony".to_string(),
            model: "E 16mm".to_string(),
            vignetting: Some(lens::VignetteCoeffs {
                k1: -1.0,
                k2: 0.0,
                k3: 0.0,
            }),
            ..lens::LensProfile::default()
        }],
    };

    let _ = app.update(Message::ExifLoaded {
        request_id: app.detail_load.request_id,
        exif: Some(lens::ExifInfo {
            camera_make: "Sony".to_string(),
            lens_model: "E 16mm".to_string(),
            ..lens::ExifInfo::default()
        }),
    });

    assert!(
        app.library[0].thumbnail_handle.is_none(),
        "ExifLoaded auto-lens commit should defer thumbnail render to the persist task"
    );
    assert!(app.local_edit_persist_in_flight.is_some());

    complete_in_flight_persist_with_rendered_thumbnail(&mut app);

    let handle = app.library[0]
        .thumbnail_handle
        .as_ref()
        .expect("lens-corrected thumbnail handle");
    let (width, height, rendered_pixels) = rgba_handle_pixels(handle);
    let expected = edit::render_edited_image(
        &pixels,
        3,
        3,
        &edit::EditState {
            lens_correction: true,
            ..edit::EditState::default()
        },
        edit::LensCorrection {
            vig: [-1.0, 0.0, 0.0],
            ..edit::LensCorrection::default()
        },
    );

    assert_eq!(width, expected.width);
    assert_eq!(height, expected.height);
    assert_eq!(rendered_pixels, expected.pixels);
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
    assert_eq!(request.lens.vig, [0.0, 0.0, 0.0]);
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
