//! Message handling, input events, and async task orchestration for `App`.

use super::*;

impl App {
    pub(crate) fn update(&mut self, msg: Message) -> Task<Message> {
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
                    // A superseded staged load can still owe a bake, and its
                    // full-resolution decode is only ever chained off this preview
                    // completion — chain it anyway or the owed commit is lost.
                    if self.owed_local_edit_bakes.contains_key(&request_id) {
                        let preferred_source = self.preferred_base_image_source(&path);
                        return Self::full_image_load_task(path, request_id, preferred_source);
                    }
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
                    return self.fulfill_owed_local_edit_bake(request_id, result);
                }

                match result {
                    Ok(mut loaded) => {
                        // A persist still racing for this path means the disk bake
                        // is stale by construction; adopt the racing request's base
                        // (the pixels its edit state was authored on) so the state
                        // neither re-applies onto a newer bake nor gets wiped.
                        let racing_covers_state =
                            if let Some(path) = self.current_image_path.as_deref() {
                                if let Some(racing) = self.newest_pending_persist_for_path(path) {
                                    let covers = racing.state == self.visible_edit_state();
                                    loaded = LoadedFullImage {
                                        image: racing.image.clone(),
                                        fingerprint: loaded.fingerprint,
                                        base_source: racing.base_source,
                                        bake_generation: None,
                                        logical_dimensions: racing.base_dimensions,
                                    };
                                    covers
                                } else {
                                    false
                                }
                            } else {
                                false
                            };
                        let absorbed = self.current_image_path.as_deref().is_some_and(|path| {
                            self.loaded_bake_absorbed_session_state(path, &loaded)
                        });
                        if let Some(path) = self.current_image_path.clone() {
                            self.base_image_sources
                                .insert(path.clone(), loaded.base_source);
                            match loaded.bake_generation {
                                Some(generation) => {
                                    self.loaded_base_generations
                                        .insert(path.clone(), generation);
                                }
                                None => {
                                    self.loaded_base_generations.remove(&path);
                                }
                            }
                            self.current_image_source_dimensions = Some(loaded.logical_dimensions);
                            if absorbed {
                                self.reset_absorbed_session_state(&path);
                            }
                        }
                        let reset_view = self.detail_load.on_full_image_loaded();
                        if let Some(fingerprint) = loaded.fingerprint {
                            self.cache_full_image_for_current_path(
                                fingerprint,
                                loaded.image.clone(),
                            );
                        }
                        self.apply_loaded_image(loaded.image, reset_view);
                        if absorbed || racing_covers_state {
                            // Either the loaded bake IS the session's latest commit,
                            // or the racing persist already carries it; nothing new
                            // to render or persist.
                            return Task::none();
                        }
                        return self.enqueue_current_local_edit_persist();
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
                let state = self.visible_edit_state();
                if self.current_image_path.is_some()
                    && self.image.is_some()
                    && state.lens_correction
                    && self.lens_override_name.is_none()
                {
                    return self.on_current_visible_render_changed();
                }
                Task::none()
            }

            Message::Viewer(evt) => self.handle_viewer(evt),

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

            Message::ThumbnailLoaded(path, Ok(base)) => {
                // Baked bases already contain their committed edits; rendering the
                // session state on top would apply those edits twice.
                let handle = match base.base_source {
                    BaseImageSource::Original => self.thumbnail_handle_for_path(&path, &base.image),
                    BaseImageSource::PersistedLocalEdit => ImageHandle::from_rgba(
                        base.image.width,
                        base.image.height,
                        base.image.pixels.clone(),
                    ),
                };
                if let Some(entry) = self.library.iter_mut().find(|e| e.path == path) {
                    entry.thumbnail_image = Some(base.image.clone());
                    entry.thumbnail_base_source = base.base_source;
                    entry.thumbnail_handle = Some(handle);
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
            Message::LocalEditPersistCompleted {
                path,
                request_id,
                result,
            } => {
                let completed_request_state = self
                    .local_edit_persist_in_flight
                    .as_ref()
                    .filter(|request| request.path == path && request.request_id == request_id)
                    .map(|request| request.state);
                if completed_request_state.is_some() {
                    self.local_edit_persist_in_flight = None;
                }
                let follow_up = match result {
                    Ok(Some(bake)) => {
                        if let (Some(generation_id), Some(state)) =
                            (bake.generation_id, completed_request_state)
                        {
                            self.last_completed_bakes.insert(
                                path.clone(),
                                CompletedBake {
                                    generation_id,
                                    state,
                                },
                            );
                        }
                        self.set_library_thumbnail_for_path(&path, bake.thumbnail);
                        Task::none()
                    }
                    Ok(None) => {
                        self.last_completed_bakes.remove(&path);
                        self.reload_library_thumbnail_after_bake_removal(&path)
                    }
                    Err(error) => {
                        log::warn!(
                            "Local edit persistence failed for {}: {}",
                            path.display(),
                            error
                        );
                        Task::none()
                    }
                };
                Task::batch([follow_up, self.start_next_local_edit_persist_if_idle()])
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
                        if self.try_reopen_current_library_image_without_reload(&path) {
                            return Task::none();
                        }
                        if self.current_image_path.as_deref() == Some(path.as_path()) {
                            self.reset_transient_detail_reopen_state();
                        }
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
                    return self.on_current_edit_committed();
                } else {
                    self.last_slider_release = Some((kind, now));
                    // Only commit if the user actually dragged (not a single track click)
                    if was_drag {
                        if let Some(path) = &self.current_image_path {
                            if let Some(history) = self.edit_histories.get_mut(path) {
                                history.commit();
                            }
                            return self.on_current_edit_committed();
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
                self.on_current_edit_committed()
            }

            Message::ResetAll => {
                let previous_rotation = self.current_rotation();
                let previous_crop = self.visible_crop();
                if let Some(path) = &self.current_image_path {
                    let history = self.edit_histories.entry(path.clone()).or_default();
                    history.reset_all();
                }
                self.preserve_actual_size_after_display_change(previous_rotation, previous_crop);
                self.on_current_edit_committed()
            }

            Message::ToggleLensCorrection => {
                if let Some(path) = &self.current_image_path {
                    let history = self.edit_histories.entry(path.clone()).or_default();
                    history.current.lens_correction = !history.current.lens_correction;
                    history.commit();
                }
                self.on_current_edit_committed()
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
                self.on_current_edit_committed()
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
                self.on_current_edit_committed()
            }

            Message::SaveEdited => {
                let Some(request) = self.current_save_request() else {
                    return Task::none();
                };
                self.save_status = Some("Saving...".to_string());
                self.save_in_flight = true;
                Task::perform(
                    async move {
                        tokio::task::spawn_blocking(move || {
                            edit::save_edited_image(
                                &request.path,
                                &request.image.pixels,
                                request.image.width,
                                request.image.height,
                                &request.state,
                                request.lens,
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
                self.save_in_flight = false;
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
                self.on_current_edit_committed()
            }

            Message::SliderTextInput(kind) => {
                let value = self
                    .current_image_path
                    .as_ref()
                    .and_then(|p| self.edit_histories.get(p))
                    .map(|h| get_slider_field(&h.current, kind))
                    .unwrap_or(0.0);
                self.editing_slider = Some(kind);
                self.slider_text_buf = slider_value_label(kind, value);
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
                self.on_current_edit_committed()
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

            Message::Harness(msg) => self.handle_harness(msg),
        }
    }

    // ---------------------------------------------------------------------------
    // Viewer interaction
    // ---------------------------------------------------------------------------

    pub(crate) fn handle_viewer(&mut self, evt: ViewerEvent) -> Task<Message> {
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
                Task::none()
            }
            ViewerEvent::Pan { delta } => {
                self.offset[0] += delta[0];
                self.offset[1] += delta[1];
                Task::none()
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
                Task::none()
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
                self.on_current_edit_committed()
            }
        }
    }

    // ---------------------------------------------------------------------------
    // Global events
    // ---------------------------------------------------------------------------

    pub(crate) fn handle_event(&mut self, event: iced::Event) -> Task<Message> {
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

    pub(crate) fn handle_key(
        &mut self,
        key: keyboard::Key,
        mods: keyboard::Modifiers,
    ) -> Task<Message> {
        use keyboard::key::Named;
        use keyboard::Key;

        match key {
            // Escape: dismiss overlays, cancel cropping, exit collection, or
            // go back to library
            Key::Named(Named::Escape) => {
                if self.context_menu.is_some() {
                    self.context_menu = None;
                } else if self.editing_collection_name.is_some() {
                    self.editing_collection_name = None;
                    self.collection_name_buf.clear();
                } else if self.tab == Tab::Detail && self.crop_mode {
                    // Cancel the crop tool (discarding any in-flight drag —
                    // the viewer drops uncommitted selections once crop mode
                    // is off) and stay on the image.
                    let previous_rotation = self.current_rotation();
                    let previous_crop = self.visible_crop();
                    self.crop_mode = false;
                    self.preserve_actual_size_after_display_change(
                        previous_rotation,
                        previous_crop,
                    );
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
                        let did_undo = history.undo();
                        if did_undo {
                            self.preserve_actual_size_after_display_change(
                                previous_rotation,
                                previous_crop,
                            );
                            return self.on_current_edit_committed();
                        }
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
                        let did_redo = history.redo();
                        if did_redo {
                            self.preserve_actual_size_after_display_change(
                                previous_rotation,
                                previous_crop,
                            );
                            return self.on_current_edit_committed();
                        }
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

    pub(crate) fn is_double_click_event(
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

    pub(crate) fn add_library_entries(&mut self, paths: &[PathBuf]) {
        for path in paths {
            if !self.library.iter().any(|e| e.path == *path) {
                self.library.push(LibraryEntry {
                    filename: path_filename_str(path).to_string(),
                    path: path.clone(),
                    thumbnail_image: None,
                    thumbnail_handle: None,
                    thumbnail_base_source: BaseImageSource::Original,
                });
            }
        }
        self.rebuild_library_indices();
    }

    pub(crate) fn filter_new_library_paths(&self, paths: Vec<PathBuf>) -> Vec<PathBuf> {
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

    pub(crate) fn import_library_paths(&mut self, new_paths: Vec<PathBuf>) -> Task<Message> {
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

    pub(crate) fn enqueue_import_cache_warm_paths(&mut self, paths: &[PathBuf]) -> Task<Message> {
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

    pub(crate) fn start_next_import_cache_warm_if_idle(&mut self) -> Task<Message> {
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
    pub(crate) fn replace_library_entries(&mut self, entries: Vec<LibraryEntry>) {
        self.library = entries;
        self.rebuild_library_indices();
        self.reset_library_navigation_state();
        self.current_image_path = None;
        self.current_image_source_dimensions = None;
        self.image = None;
    }

    #[cfg(test)]
    pub(crate) fn reset_library_navigation_state(&mut self) {
        self.library_index = None;
        self.collection_nav = None;
        self.nav = None;
    }

    #[cfg(test)]
    pub(crate) fn clear_library_entries(&mut self) {
        self.replace_library_entries(Vec::new());
    }

    #[cfg(test)]
    pub(crate) fn remove_library_entry(&mut self, index: usize) -> Option<LibraryEntry> {
        if index >= self.library.len() {
            return None;
        }
        let removed = self.library.remove(index);
        self.rebuild_library_indices();
        self.reset_library_navigation_state();
        if self.current_image_path.as_ref() == Some(&removed.path) {
            self.current_image_path = None;
            self.current_image_source_dimensions = None;
            self.image = None;
        }
        Some(removed)
    }

    pub(crate) fn rebuild_library_indices(&mut self) {
        self.library_indices_by_path = self
            .library
            .iter()
            .enumerate()
            .map(|(index, entry)| (entry.path.clone(), index))
            .collect();
    }

    pub(crate) fn load_thumbnails(paths: &[PathBuf]) -> Task<Message> {
        Task::batch(paths.iter().map(|path| {
            let p = path.clone();
            let p2 = path.clone();
            Task::perform(
                async move {
                    let result: Result<LoadedThumbnailBase, String> =
                        tokio::task::spawn_blocking(move || {
                            load_library_thumbnail_base_image(&p, LOCAL_EDIT_THUMBNAIL_MAX_DIM)
                        })
                        .await
                        .map_err(|e| e.to_string())?;
                    result
                },
                move |result| Message::ThumbnailLoaded(p2.clone(), result),
            )
        }))
    }

    pub(crate) fn preferred_base_image_source(&self, path: &Path) -> BaseImageSource {
        self.base_image_sources
            .get(path)
            .copied()
            .unwrap_or_else(|| {
                if persisted_local_edit_exists(path, LocalEditCacheVariant::Full) {
                    BaseImageSource::PersistedLocalEdit
                } else {
                    BaseImageSource::Original
                }
            })
    }

    pub(crate) fn current_base_image_source(&self) -> BaseImageSource {
        self.current_image_path
            .as_deref()
            .map(|path| self.preferred_base_image_source(path))
            .unwrap_or(BaseImageSource::Original)
    }

    pub(crate) fn thumbnail_handle_for_path(&self, path: &Path, image: &ImageData) -> ImageHandle {
        let state = self
            .edit_histories
            .get(path)
            .map(|history| history.current)
            .unwrap_or_default();
        let lens = if self.current_image_path.as_deref() == Some(path) {
            self.current_lens_correction(state.lens_correction)
        } else {
            edit::LensCorrection::default()
        };
        let rendered =
            edit::render_edited_image(&image.pixels, image.width, image.height, &state, lens);
        ImageHandle::from_rgba(rendered.width, rendered.height, rendered.pixels)
    }

    pub(crate) fn refresh_library_thumbnail_for_path(&mut self, path: &Path) {
        let Some(&index) = self.library_indices_by_path.get(path) else {
            return;
        };
        // Baked bases already contain their committed edits; only the persist
        // pipeline may replace them (set_library_thumbnail_for_path), otherwise the
        // session state would render on top of itself.
        if matches!(
            self.library[index].thumbnail_base_source,
            BaseImageSource::PersistedLocalEdit
        ) {
            return;
        }
        let Some(base_image) = self.library[index].thumbnail_image.clone() else {
            return;
        };
        let handle = self.thumbnail_handle_for_path(path, &base_image);
        self.library[index].thumbnail_handle = Some(handle);
    }

    pub(crate) fn set_library_thumbnail_for_path(&mut self, path: &Path, image: Arc<ImageData>) {
        let Some(&index) = self.library_indices_by_path.get(path) else {
            return;
        };
        let entry = &mut self.library[index];
        entry.thumbnail_handle = Some(ImageHandle::from_rgba(
            image.width,
            image.height,
            image.pixels.clone(),
        ));
        entry.thumbnail_image = Some(image);
        entry.thumbnail_base_source = BaseImageSource::PersistedLocalEdit;
    }

    /// After a bake is removed (edits reset to default), the stored base may hold
    /// baked pixels, so the original thumbnail reloads asynchronously. The stale
    /// handle stays visible until the reload lands to avoid a blank flash.
    pub(crate) fn reload_library_thumbnail_after_bake_removal(
        &mut self,
        path: &Path,
    ) -> Task<Message> {
        let Some(&index) = self.library_indices_by_path.get(path) else {
            return Task::none();
        };
        let entry = &mut self.library[index];
        entry.thumbnail_image = None;
        entry.thumbnail_base_source = BaseImageSource::Original;
        Self::load_thumbnails(&[path.to_path_buf()])
    }

    pub(crate) fn current_local_edit_persist_request(&mut self) -> Option<LocalEditPersistRequest> {
        if self.detail_load.blocks_save() {
            return None;
        }

        let path = self.current_image_path.clone()?;
        let image = self.image.clone()?;
        let state = self.visible_edit_state();
        if self.current_render_depends_on_pending_auto_lens_metadata(state) {
            return None;
        }
        let base_source = self.current_base_image_source();
        if self.bake_has_nothing_new(state, base_source, &path) {
            return None;
        }
        let lens = self.current_lens_correction(state.lens_correction);
        let base_dimensions = self
            .current_image_source_dimensions
            .unwrap_or((image.width, image.height));
        let request_id = self.next_local_edit_persist_request_id();

        Some(LocalEditPersistRequest {
            request_id,
            path,
            image,
            logical_dimensions: display_dimensions_for_edit_state(
                base_dimensions,
                state.rotation,
                state.crop,
            ),
            base_dimensions,
            state,
            lens,
            base_source,
        })
    }

    pub(crate) fn current_render_depends_on_pending_auto_lens_metadata(
        &self,
        state: edit::EditState,
    ) -> bool {
        state.lens_correction && self.lens_override_name.is_none() && self.detail_load.exif_loading
    }

    pub(crate) fn next_local_edit_persist_request_id(&self) -> u64 {
        self.local_edit_persist_in_flight
            .as_ref()
            .map(|request| request.request_id)
            .unwrap_or(0)
            .max(
                self.pending_local_edit_persist_requests
                    .back()
                    .map(|request| request.request_id)
                    .unwrap_or(0),
            )
            + 1
    }

    /// Snapshots a bake obligation for the image being navigated away from while
    /// its full-resolution decode is still in flight. Committed edits made during
    /// the staged-preview window cannot bake until that decode lands; without this
    /// the stale completion would be discarded and the commit silently lost.
    pub(crate) fn register_owed_local_edit_bake_for_superseded_load(&mut self) {
        let Some(path) = self.current_image_path.clone() else {
            return;
        };
        let state = self
            .edit_histories
            .get(&path)
            .map(|history| history.current)
            .unwrap_or_default();
        if !self.detail_load.is_loading() {
            // Nothing will arrive for this request. The only commit at risk here is
            // one still waiting on EXIF metadata; surface it instead of losing it
            // silently (it bakes on the next open of that image).
            if !state.is_default()
                && self.current_render_depends_on_pending_auto_lens_metadata(state)
            {
                log::warn!(
                    "Navigated away from {} while its EXIF metadata was loading; the commit bakes on next open",
                    path.display()
                );
            }
            return;
        }
        let base_source = self.preferred_base_image_source(&path);
        if self.bake_has_nothing_new(state, base_source, &path) {
            return;
        }
        let lens = self.current_lens_correction(state.lens_correction);
        self.owed_local_edit_bakes.insert(
            self.detail_load.request_id,
            OwedLocalEditBake { path, lens },
        );
    }

    /// A default-state image owes nothing when it was never baked (nothing to
    /// persist) or when its base already IS the bake still on disk (an identity
    /// re-bake would rewrite identical full-resolution pixels). A default state
    /// over an `Original` base with an existing bake still owes (pending reset:
    /// persisting it removes the bake), and a default state over a baked base
    /// that a session persist has advanced PAST is a revert that must re-bake —
    /// "default" no longer matches the disk.
    fn bake_has_nothing_new(
        &self,
        state: edit::EditState,
        base_source: BaseImageSource,
        path: &Path,
    ) -> bool {
        if !state.is_default() {
            return false;
        }
        match base_source {
            BaseImageSource::Original => {
                !persisted_local_edit_exists(path, LocalEditCacheVariant::Full)
            }
            BaseImageSource::PersistedLocalEdit => {
                if self.newest_pending_persist_for_path(path).is_some() {
                    return false;
                }
                match (
                    self.last_completed_bakes.get(path),
                    self.loaded_base_generations.get(path),
                ) {
                    (None, _) => true,
                    (Some(bake), Some(loaded)) => bake.generation_id == *loaded,
                    (Some(_), None) => false,
                }
            }
        }
    }

    /// True when the freshly loaded base is the very bake this session last
    /// committed for the path: its pixels already contain the session's edit
    /// state, so that state must reset rather than render (and re-bake) on top.
    pub(crate) fn loaded_bake_absorbed_session_state(
        &self,
        path: &Path,
        loaded: &LoadedFullImage,
    ) -> bool {
        matches!(loaded.base_source, BaseImageSource::PersistedLocalEdit)
            && loaded.bake_generation.is_some()
            && loaded.bake_generation
                == self
                    .last_completed_bakes
                    .get(path)
                    .map(|bake| bake.generation_id)
    }

    /// The newest persist still racing for this path, if any. While one exists,
    /// any disk bake is stale by construction and the request itself holds the
    /// exact base its edit state was authored on.
    pub(crate) fn newest_pending_persist_for_path(
        &self,
        path: &Path,
    ) -> Option<&LocalEditPersistRequest> {
        self.pending_local_edit_persist_requests
            .iter()
            .rev()
            .find(|request| request.path == path)
            .or_else(|| {
                self.local_edit_persist_in_flight
                    .as_ref()
                    .filter(|request| request.path == path)
            })
    }

    /// Replaces the path's history outright: undo/redo entries are anchored to
    /// the pre-absorption base, and replaying any of them onto the absorbed bake
    /// would double-apply. Commits the bake never absorbed (made during the
    /// reload window, with no racing persist to carry them) cannot be expressed
    /// against the new base — they are discarded with a visible notice.
    pub(crate) fn reset_absorbed_session_state(&mut self, path: &Path) {
        let baked_state = self.last_completed_bakes.get(path).map(|bake| bake.state);
        if let Some(history) = self.edit_histories.get_mut(path) {
            let lost_commits = baked_state != Some(history.current);
            *history = edit::UndoHistory::new();
            if lost_commits {
                self.save_status = Some(
                    "Adjustments made while the photo was reloading could not be kept".to_string(),
                );
            }
        }
        // Post-absorption, the absorbed state IS the identity relative to the
        // re-anchored base. Normalizing the record keeps a later re-absorption of
        // the same bake from comparing against the pre-reset state and falsely
        // re-firing the lost-commits notice.
        if let Some(record) = self.last_completed_bakes.get_mut(path) {
            record.state = edit::EditState::default();
        }
    }

    pub(crate) fn fulfill_owed_local_edit_bake(
        &mut self,
        request_id: u64,
        result: Result<LoadedFullImage, String>,
    ) -> Task<Message> {
        let Some(owed) = self.owed_local_edit_bakes.remove(&request_id) else {
            return Task::none();
        };
        match result {
            Ok(loaded) => {
                let state = self
                    .edit_histories
                    .get(&owed.path)
                    .map(|history| history.current)
                    .unwrap_or_default();
                // A persist racing for this path makes the stale decode's bake
                // obsolete. If the racing request already carries the current
                // state, the owed bake is redundant; if the owed state is NEWER
                // (committed under blocks_save after the racing persist started),
                // it must bake on the racing request's base — the pixels the
                // session state is actually anchored to — not the stale decode.
                let racing_snapshot =
                    self.newest_pending_persist_for_path(&owed.path)
                        .map(|racing| {
                            (
                                racing.image.clone(),
                                racing.base_source,
                                racing.base_dimensions,
                                racing.state,
                            )
                        });
                if let Some((image, base_source, base_dimensions, racing_state)) = racing_snapshot {
                    self.base_image_sources
                        .insert(owed.path.clone(), base_source);
                    if racing_state == state {
                        return Task::none();
                    }
                    let request = LocalEditPersistRequest {
                        request_id: self.next_local_edit_persist_request_id(),
                        path: owed.path,
                        image,
                        logical_dimensions: display_dimensions_for_edit_state(
                            base_dimensions,
                            state.rotation,
                            state.crop,
                        ),
                        base_dimensions,
                        state,
                        lens: owed.lens,
                        base_source,
                    };
                    return self.enqueue_local_edit_persist(request);
                }
                // The session's edit state for this path is relative to the base
                // this decode actually used. Record that base, exactly as the
                // current-request arm does, or the next reopen would resolve the
                // freshly written bake as its base and apply the still-held edit
                // state on top of it a second time.
                self.base_image_sources
                    .insert(owed.path.clone(), loaded.base_source);
                if self.loaded_bake_absorbed_session_state(&owed.path, &loaded) {
                    self.reset_absorbed_session_state(&owed.path);
                    return Task::none();
                }
                // Nothing-new for a stale decode is judged against ITS base: a
                // default state skips only when no session persist exists at all.
                // With one on record (and this decode's generation differing —
                // absorbed already returned above), the default state is a revert
                // and this decode's older base is exactly the right one to re-bake.
                let nothing_new = state.is_default()
                    && match loaded.base_source {
                        BaseImageSource::Original => {
                            !persisted_local_edit_exists(&owed.path, LocalEditCacheVariant::Full)
                        }
                        BaseImageSource::PersistedLocalEdit => {
                            !self.last_completed_bakes.contains_key(&owed.path)
                        }
                    };
                if nothing_new {
                    return Task::none();
                }
                let request = LocalEditPersistRequest {
                    request_id: self.next_local_edit_persist_request_id(),
                    path: owed.path,
                    image: loaded.image,
                    logical_dimensions: display_dimensions_for_edit_state(
                        loaded.logical_dimensions,
                        state.rotation,
                        state.crop,
                    ),
                    base_dimensions: loaded.logical_dimensions,
                    state,
                    lens: owed.lens,
                    base_source: loaded.base_source,
                };
                self.enqueue_local_edit_persist(request)
            }
            Err(error) => {
                log::warn!(
                    "Dropping owed local-edit bake for {}: full-resolution load failed: {}",
                    owed.path.display(),
                    error
                );
                Task::none()
            }
        }
    }

    pub(crate) fn enqueue_current_local_edit_persist(&mut self) -> Task<Message> {
        let Some(request) = self.current_local_edit_persist_request() else {
            return Task::none();
        };
        self.enqueue_local_edit_persist(request)
    }

    pub(crate) fn enqueue_local_edit_persist(
        &mut self,
        request: LocalEditPersistRequest,
    ) -> Task<Message> {
        if self.local_edit_persist_in_flight.is_none() {
            self.local_edit_persist_in_flight = Some(request.clone());
            return Self::local_edit_persist_task(request);
        }

        self.pending_local_edit_persist_requests
            .retain(|pending| pending.path != request.path);
        self.pending_local_edit_persist_requests.push_back(request);
        Task::none()
    }

    pub(crate) fn start_next_local_edit_persist_if_idle(&mut self) -> Task<Message> {
        if self.local_edit_persist_in_flight.is_some() {
            return Task::none();
        }

        let Some(request) = self.pending_local_edit_persist_requests.pop_front() else {
            return Task::none();
        };

        self.local_edit_persist_in_flight = Some(request.clone());
        Self::local_edit_persist_task(request)
    }

    pub(crate) fn local_edit_persist_task(request: LocalEditPersistRequest) -> Task<Message> {
        let message_path = request.path.clone();
        let request_id = request.request_id;
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || persist_local_edit(&request))
                    .await
                    .map_err(|e| e.to_string())?
            },
            move |result| Message::LocalEditPersistCompleted {
                path: message_path.clone(),
                request_id,
                result,
            },
        )
    }

    pub(crate) fn on_current_visible_render_changed(&mut self) -> Task<Message> {
        if let Some(request) = self.current_local_edit_persist_request() {
            return self.enqueue_local_edit_persist(request);
        }

        if let Some(path) = self.current_image_path.clone() {
            self.refresh_library_thumbnail_for_path(&path);
        }
        Task::none()
    }

    pub(crate) fn on_current_edit_committed(&mut self) -> Task<Message> {
        self.on_current_visible_render_changed()
    }

    pub(crate) fn import_cache_warm_task(path: PathBuf) -> Task<Message> {
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

    pub(crate) fn open_file_dialog(&self) -> Task<Message> {
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

    pub(crate) fn lens_profile_for_exif(
        &self,
        exif_info: &lens::ExifInfo,
    ) -> Option<lens::LensProfile> {
        let maker = if exif_info.lens_make.is_empty() {
            &exif_info.camera_make
        } else {
            &exif_info.lens_make
        };
        self.lens_db
            .find_lens(maker, &exif_info.lens_model)
            .cloned()
    }

    pub(crate) fn refresh_auto_lens_profile(&mut self) {
        if self.lens_override_name.is_none() {
            self.current_lens_profile = self
                .current_exif
                .as_ref()
                .and_then(|exif_info| self.lens_profile_for_exif(exif_info));
        }
    }

    pub(crate) fn apply_loaded_image(&mut self, data: Arc<ImageData>, reset_view: bool) {
        self.image = Some(data);
        self.image_id += 1;
        if reset_view {
            self.zoom = 1.0;
            self.offset = [0.0, 0.0];
            self.crop_mode = false;
        }
        self.error = None;
    }

    pub(crate) fn preview_load_task(path: PathBuf, request_id: u64) -> Task<Message> {
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

    pub(crate) fn full_image_load_task(
        path: PathBuf,
        request_id: u64,
        preferred_source: BaseImageSource,
    ) -> Task<Message> {
        Task::perform(
            async move {
                let result: Result<LoadedFullImage, String> =
                    tokio::task::spawn_blocking(move || load_full_image(&path, preferred_source))
                        .await
                        .map_err(|e| e.to_string())?;
                result
            },
            move |result| Message::ImageLoaded { request_id, result },
        )
    }

    pub(crate) fn exif_load_task(path: PathBuf, request_id: u64) -> Task<Message> {
        Task::perform(
            async move {
                tokio::task::spawn_blocking(move || lens::read_exif(&path))
                    .await
                    .unwrap_or(None)
            },
            move |exif| Message::ExifLoaded { request_id, exif },
        )
    }

    pub(crate) fn start_follow_up_load(&self, path: PathBuf, request_id: u64) -> Task<Message> {
        let preferred_source = self.preferred_base_image_source(&path);
        Task::batch([
            Self::full_image_load_task(path.clone(), request_id, preferred_source),
            Self::exif_load_task(path, request_id),
        ])
    }

    pub(crate) fn cache_full_image_for_current_path(
        &mut self,
        fingerprint: SourceFileFingerprint,
        image: Arc<ImageData>,
    ) {
        let Some(path) = self.current_image_path.as_deref() else {
            return;
        };
        let base_source = self.current_base_image_source();
        let logical_dimensions = self
            .current_image_source_dimensions
            .unwrap_or((image.width, image.height));
        self.session_full_image_cache.insert(
            path,
            fingerprint,
            image,
            base_source,
            logical_dimensions,
        );
    }

    pub(crate) fn displayed_full_image_for_path(
        &self,
        path: &Path,
        expected_base_source: BaseImageSource,
    ) -> Option<Arc<ImageData>> {
        if self.current_image_path.as_deref() != Some(path) {
            return None;
        }
        if !self.session_full_image_cache.contains_path(path) {
            return None;
        }
        if !self
            .session_full_image_cache
            .entry_matches_base_source(path, expected_base_source)
        {
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

    pub(crate) fn try_reopen_current_library_image_without_reload(&mut self, path: &Path) -> bool {
        let preferred_source = self.preferred_base_image_source(path);
        if self
            .displayed_full_image_for_path(path, preferred_source)
            .is_none()
        {
            return false;
        }

        self.clear_library_drag_state();
        self.reset_transient_detail_reopen_state();
        true
    }

    pub(crate) fn clear_library_drag_state(&mut self) {
        self.drag_state = None;
        self.sidebar_hover_collection = None;
    }

    pub(crate) fn reset_transient_detail_reopen_state(&mut self) {
        self.error = None;
        self.save_status = None;
        self.zoom = 1.0;
        self.offset = [0.0, 0.0];
        self.crop_mode = false;
    }

    pub(crate) fn start_load(&mut self, path: PathBuf) -> Task<Message> {
        self.clear_library_drag_state();
        self.register_owed_local_edit_bake_for_superseded_load();
        let preferred_source = self.preferred_base_image_source(&path);
        let displayed_full_image = self.displayed_full_image_for_path(&path, preferred_source);
        let displayed_logical_dimensions = displayed_full_image
            .as_ref()
            .and(self.current_image_source_dimensions);
        let request_id = self.detail_load.begin_request();
        self.current_image_path = Some(path.clone());
        self.current_image_source_dimensions = None;
        self.error = None;
        self.save_status = None;
        self.current_exif = None;
        if self.lens_override_name.is_none() {
            self.current_lens_profile = None;
        }

        if let Some(image) = displayed_full_image {
            self.current_image_source_dimensions =
                Some(displayed_logical_dimensions.unwrap_or((image.width, image.height)));
            let reset_view = self.detail_load.on_full_image_loaded();
            self.apply_loaded_image(image, reset_view);
            return Self::exif_load_task(path, request_id);
        }

        let cached_full_image = self.session_full_image_cache.get(&path, preferred_source);
        if let Some(hit) = cached_full_image {
            self.current_image_source_dimensions = Some(hit.logical_dimensions);
            let reset_view = self.detail_load.on_full_image_loaded();
            self.apply_loaded_image(hit.image, reset_view);
            return Self::exif_load_task(path, request_id);
        }

        self.image = None;
        if nav::is_raw_file(&path)
            && !matches!(preferred_source, BaseImageSource::PersistedLocalEdit)
        {
            Self::preview_load_task(path, request_id)
        } else {
            Task::batch([
                Self::full_image_load_task(path.clone(), request_id, preferred_source),
                Self::exif_load_task(path, request_id),
            ])
        }
    }

    // ---------------------------------------------------------------------------
    // View
    // ---------------------------------------------------------------------------
}
