//! Textures of the previews on screen.
//!
//! A texture is valid for exactly one photo, preview size and develop
//! version. The cache keeps one per photo and size: when the engine has
//! pixels for a newer version, they replace the old texture, and until they
//! do the old one is still shown, so an edit never blanks a cell.
//!
//! Uploads are rationed: at most [`UPLOAD_BUDGET`] bytes of pixels a frame
//! (but always at least one preview), so a scroll that brings fifty new
//! thumbnails into view spreads them over a few frames instead of stalling
//! one. What waited asks for the next frame. Textures not drawn for a while
//! are dropped once a size's memory bound is reached.

use std::collections::HashMap;
use std::hash::Hash;
use std::path::{Path, PathBuf};

use egui::{ColorImage, TextureHandle, TextureId, TextureOptions, Vec2, vec2};
use photo_core::meta::PhotoId;
use photo_engine::{Engine, Preview, PreviewSize, PreviewState};

/// Bytes of pixels uploaded per frame, beyond the first preview.
const UPLOAD_BUDGET: usize = 6 << 20;
/// Bytes of thumbnail textures kept.
const THUMBNAIL_MEMORY: usize = 160 << 20;
/// Bytes of standard-preview textures kept.
const STANDARD_MEMORY: usize = 96 << 20;
/// Bytes of import-candidate textures kept.
const CANDIDATE_MEMORY: usize = 48 << 20;

/// A texture ready to draw: the current version's, or while that is not
/// uploaded yet, an older one.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Shown {
    pub(crate) id: TextureId,
    /// Width divided by height.
    pub(crate) aspect: f32,
    /// The size in pixels.
    pub(crate) pixels: Vec2,
}

struct Slot {
    version: u64,
    handle: TextureHandle,
    pixels: Vec2,
    bytes: usize,
    used: u64,
}

impl Slot {
    fn shown(&self) -> Shown {
        Shown {
            id: self.handle.id(),
            aspect: self.pixels.x / self.pixels.y.max(1.0),
            pixels: self.pixels,
        }
    }
}

/// Textures by key, bounded by the bytes they hold.
struct Shelf<K> {
    slots: HashMap<K, Slot>,
    bytes: usize,
    bound: usize,
}

impl<K: Hash + Eq + Clone> Shelf<K> {
    fn new(bound: usize) -> Self {
        Self {
            slots: HashMap::new(),
            bytes: 0,
            bound,
        }
    }

    fn insert(&mut self, key: K, slot: Slot) {
        self.bytes += slot.bytes;
        if let Some(old) = self.slots.insert(key, slot) {
            self.bytes -= old.bytes;
        }
    }

    /// Drop the textures used longest ago, never one used this frame, until
    /// the bound holds.
    fn evict(&mut self, frame: u64) {
        if self.bytes <= self.bound {
            return;
        }
        let mut by_age: Vec<(u64, K)> = self
            .slots
            .iter()
            .filter(|(_, slot)| slot.used < frame)
            .map(|(key, slot)| (slot.used, key.clone()))
            .collect();
        by_age.sort_by_key(|(used, _)| *used);
        for (_, key) in by_age {
            if self.bytes <= self.bound {
                break;
            }
            if let Some(slot) = self.slots.remove(&key) {
                self.bytes -= slot.bytes;
            }
        }
    }

    fn clear(&mut self) {
        self.slots.clear();
        self.bytes = 0;
    }
}

/// The texture cache. See the [module documentation](self).
pub(crate) struct Textures {
    thumbnails: Shelf<PhotoId>,
    standard: Shelf<PhotoId>,
    candidates: Shelf<PathBuf>,
    frame: u64,
    uploaded: usize,
    deferred: bool,
}

impl Default for Textures {
    fn default() -> Self {
        Self {
            thumbnails: Shelf::new(THUMBNAIL_MEMORY),
            standard: Shelf::new(STANDARD_MEMORY),
            candidates: Shelf::new(CANDIDATE_MEMORY),
            frame: 0,
            uploaded: 0,
            deferred: false,
        }
    }
}

impl Textures {
    /// Start a frame: a fresh upload budget.
    pub(crate) fn begin_frame(&mut self, frame: u64) {
        self.frame = frame;
        self.uploaded = 0;
        self.deferred = false;
    }

    /// End a frame: drop what is over the memory bounds. Returns whether an
    /// upload had to wait, so another frame is wanted.
    pub(crate) fn end_frame(&mut self) -> bool {
        self.thumbnails.evict(self.frame);
        self.standard.evict(self.frame);
        self.candidates.evict(self.frame);
        self.deferred
    }

    /// The texture of `photo` at `size`, asking the engine for the pixels
    /// (it answers at once, and starts making them if it has none). An
    /// older version is returned while the current one waits.
    pub(crate) fn photo(
        &mut self,
        ctx: &egui::Context,
        engine: &mut Engine,
        photo: PhotoId,
        size: PreviewSize,
    ) -> Option<Shown> {
        let state = engine.preview(photo, size);
        let frame = self.frame;
        let shelf = match size {
            PreviewSize::Thumbnail => &mut self.thumbnails,
            PreviewSize::Standard => &mut self.standard,
        };
        let ready = state.ready();
        if let Some(slot) = shelf.slots.get_mut(&photo) {
            slot.used = frame;
            if ready.is_none_or(|preview| preview.version() == slot.version) {
                return Some(slot.shown());
            }
        }
        let preview = ready?;
        if !upload_allowed(self.uploaded, preview.image().pixels.len()) {
            self.deferred = true;
            return shelf.slots.get(&photo).map(Slot::shown);
        }
        let name = format!("photo-{photo}-{size:?}");
        let slot = upload(ctx, &name, preview, frame)?;
        self.uploaded += slot.bytes;
        let shown = slot.shown();
        shelf.insert(photo, slot);
        Some(shown)
    }

    /// The thumbnail of a file offered for import.
    pub(crate) fn candidate(
        &mut self,
        ctx: &egui::Context,
        engine: &mut Engine,
        path: &Path,
    ) -> Option<Shown> {
        let frame = self.frame;
        if let Some(slot) = self.candidates.slots.get_mut(path) {
            slot.used = frame;
            return Some(slot.shown());
        }
        let state = engine.candidate_thumbnail(path);
        let PreviewState::Ready(preview) = state else {
            return None;
        };
        if !upload_allowed(self.uploaded, preview.image().pixels.len()) {
            self.deferred = true;
            return None;
        }
        let name = format!("candidate-{}", path.display());
        let slot = upload(ctx, &name, &preview, frame)?;
        self.uploaded += slot.bytes;
        let shown = slot.shown();
        self.candidates.insert(path.to_path_buf(), slot);
        Some(shown)
    }

    /// Forget the import candidates' textures (the import dialog closed).
    pub(crate) fn clear_candidates(&mut self) {
        self.candidates.clear();
    }
}

/// Whether `bytes` more may be uploaded with `uploaded` bytes already sent
/// this frame: always the first preview, and others while the budget lasts.
fn upload_allowed(uploaded: usize, bytes: usize) -> bool {
    uploaded == 0 || uploaded + bytes <= UPLOAD_BUDGET
}

/// Upload a preview's pixels. `None` for pixels that do not match their
/// stated size, which are never sent to the GPU.
fn upload(ctx: &egui::Context, name: &str, preview: &Preview, frame: u64) -> Option<Slot> {
    let image = preview.image();
    let (width, height) = (image.width as usize, image.height as usize);
    let expected = width.checked_mul(height)?.checked_mul(4)?;
    if width == 0 || height == 0 || image.pixels.len() != expected {
        log::warn!("{name}: {width} x {height} pixels do not match their buffer");
        return None;
    }
    // Previews are opaque, so their bytes are already premultiplied.
    let color = ColorImage::from_rgba_premultiplied([width, height], &image.pixels);
    let handle = ctx.load_texture(name, color, TextureOptions::LINEAR);
    Some(Slot {
        version: preview.version(),
        handle,
        pixels: vec2(width as f32, height as f32),
        bytes: expected,
        used: frame,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A slot of `bytes`, last drawn in frame `used`.
    fn slot(ctx: &egui::Context, bytes: usize, used: u64) -> Slot {
        let image = ColorImage::filled([1, 1], egui::Color32::GRAY);
        Slot {
            version: 1,
            handle: ctx.load_texture("test", image, TextureOptions::LINEAR),
            pixels: vec2(1.0, 1.0),
            bytes,
            used,
        }
    }

    #[test]
    fn the_first_upload_of_a_frame_always_goes_and_later_ones_while_the_budget_lasts() {
        assert!(
            upload_allowed(0, UPLOAD_BUDGET * 3),
            "one standard preview is never refused"
        );
        assert!(upload_allowed(1 << 20, 1 << 20));
        assert!(!upload_allowed(UPLOAD_BUDGET - 10, 1 << 20));
    }

    #[test]
    fn over_its_bound_a_shelf_drops_what_was_drawn_longest_ago_but_never_this_frame() {
        let ctx = egui::Context::default();
        let mut shelf = Shelf::new(100);
        shelf.insert(1, slot(&ctx, 60, 3));
        shelf.insert(2, slot(&ctx, 60, 1));
        shelf.insert(3, slot(&ctx, 60, 5));
        shelf.evict(5);
        assert!(shelf.slots.contains_key(&3), "drawn this frame");
        assert!(!shelf.slots.contains_key(&2), "the oldest goes first");
        assert!(!shelf.slots.contains_key(&1));
        assert_eq!(shelf.bytes, 60);

        shelf.insert(3, slot(&ctx, 20, 6));
        assert_eq!(shelf.bytes, 20, "a replaced texture's bytes are given back");
    }
}
