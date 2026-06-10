//! Staged Detail-load lifecycle state.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum DetailLoadStage {
    #[default]
    Idle,
    Loading,
    PreviewWhileLoading,
    PreviewOnly,
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct DetailLoadState {
    pub(crate) request_id: u64,
    pub(crate) stage: DetailLoadStage,
    pub(crate) exif_loading: bool,
}

impl DetailLoadState {
    pub(crate) fn begin_request(&mut self) -> u64 {
        self.request_id += 1;
        self.stage = DetailLoadStage::Loading;
        self.exif_loading = true;
        self.request_id
    }

    pub(crate) fn is_current_request(&self, request_id: u64) -> bool {
        request_id == self.request_id
    }

    pub(crate) fn is_loading(&self) -> bool {
        matches!(
            self.stage,
            DetailLoadStage::Loading | DetailLoadStage::PreviewWhileLoading
        )
    }

    pub(crate) fn shows_embedded_preview(&self) -> bool {
        matches!(
            self.stage,
            DetailLoadStage::PreviewWhileLoading | DetailLoadStage::PreviewOnly
        )
    }

    pub(crate) fn on_preview_loaded(&mut self) {
        self.stage = DetailLoadStage::PreviewWhileLoading;
    }

    pub(crate) fn on_full_image_loaded(&mut self) -> bool {
        let reset_view = matches!(self.stage, DetailLoadStage::Loading);
        self.stage = DetailLoadStage::Idle;
        reset_view
    }

    pub(crate) fn on_full_image_failed(&mut self) {
        self.stage = if self.shows_embedded_preview() {
            DetailLoadStage::PreviewOnly
        } else {
            DetailLoadStage::Idle
        };
    }

    pub(crate) fn finish_exif(&mut self) {
        self.exif_loading = false;
    }

    pub(crate) fn load_suffix(&self) -> &'static str {
        match self.stage {
            DetailLoadStage::PreviewWhileLoading => "  •  Loading full resolution…",
            DetailLoadStage::PreviewOnly => "  •  Embedded preview",
            DetailLoadStage::Idle | DetailLoadStage::Loading => "",
        }
    }

    pub(crate) fn blocks_save(&self) -> bool {
        self.is_loading() || self.shows_embedded_preview()
    }
}
