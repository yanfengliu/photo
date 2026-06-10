//! View composition: tab bar, library/collection grids, detail editor, overlays.

use super::*;
use iced::widget::{column, row};

impl App {
    pub(crate) fn view(&self) -> Element<'_, Message> {
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

    pub(crate) fn tab_bar(&self) -> Element<'_, Message> {
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

    pub(crate) fn library_view(&self) -> Element<'_, Message> {
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

    pub(crate) fn collection_sidebar(&self) -> Element<'_, Message> {
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

    pub(crate) fn collection_grid_view(&self, collection_index: usize) -> Element<'_, Message> {
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

    pub(crate) fn thumbnail_card<'a>(
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

    pub(crate) fn thumbnail_card_content(
        &self,
        handle: Option<&ImageHandle>,
        label_text: String,
        thumb_size: f32,
    ) -> Element<'static, Message> {
        let thumb: Element<'static, Message> = if let Some(handle) = handle {
            thumbnail_slot(handle.clone(), thumb_size)
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

    pub(crate) fn build_thumbnail_grid<'a, F>(
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

    pub(crate) fn detail_view(&self) -> Element<'_, Message> {
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

    pub(crate) fn status_bar_text(&self) -> String {
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

    pub(crate) fn status_bar(&self) -> Element<'_, Message> {
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

    pub(crate) fn edit_panel(&self) -> Element<'_, Message> {
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

    pub(crate) fn slider_row(
        &self,
        label: &str,
        kind: SliderKind,
        value: f32,
    ) -> Element<'_, Message> {
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

    pub(crate) fn build_adjustment_uniforms(&self) -> viewer::AdjustmentUniforms {
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

    pub(crate) fn context_menu_overlay(&self, menu: &ContextMenu) -> Element<'_, Message> {
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

    pub(crate) fn drag_overlay(&self, drag: &DragState) -> Element<'_, Message> {
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
            thumbnail_slot(handle.clone(), 60.0)
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
