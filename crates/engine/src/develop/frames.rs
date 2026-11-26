//! Asking the render worker for the session's frames, and taking them in:
//! the mask overlay, the canvas, the before view and the preset hover
//! preview.

use std::sync::Arc;

use photo_catalog::{CatalogError, Entity};
use photo_core::develop::DevelopSettings;
use photo_core::meta::{FileKind, PhotoId, SnapshotId};
use photo_develop::Quality;

use super::worker::{Delivery, Order, Outcome, Purpose, Work};
use super::{BeforeSettings, DevelopFrame, DevelopLoad, MaskOverlay, develop_view, tools};
use crate::engine::Engine;
use crate::error::EngineError;

impl Engine {
    /// Ask the render worker for a frame of the open photo for `purpose`
    /// (the overlay's coverage, for [`Purpose::Overlay`]). Does nothing
    /// until the photo is decoded and the canvas has a view, or when there
    /// is nothing to show for that purpose.
    pub(crate) fn render_session(&mut self, purpose: Purpose, quality: Quality) {
        let Some(session) = self.develop.as_mut() else {
            return;
        };
        let (Some(source), Some(canvas)) = (session.source.clone(), session.canvas) else {
            return;
        };
        let mut work = Work::Render(quality);
        let (settings, view) = match purpose {
            Purpose::Overlay => {
                let Some(mask) = session.overlay_mask else {
                    return;
                };
                if mask >= session.settings.masks.len() {
                    // The mask was deleted: there is nothing left to show.
                    session.overlay_mask = None;
                    session.overlay = None;
                    return;
                }
                work = Work::Alpha(mask);
                let view = develop_view(&source, &session.settings, canvas);
                (session.settings.clone(), view)
            }
            Purpose::Main => {
                let view = develop_view(&source, &session.settings, canvas);
                (session.settings.clone(), view)
            }
            Purpose::Before => {
                let Some((_, before)) = &session.before else {
                    return;
                };
                let shown = tools::with_geometry(before, &session.settings);
                let view = develop_view(&source, &session.settings, canvas);
                (shown, view)
            }
            Purpose::Look => {
                let Some((_, look, (width, height))) = &session.look else {
                    return;
                };
                let view = photo_develop::fit_view(&source, look, *width, *height);
                (look.clone(), view)
            }
        };
        self.next_render += 1;
        let order = Order {
            id: self.next_render,
            purpose,
            generation: session.generation,
            source,
            settings,
            version: session.version,
            view,
            work,
        };
        session.counts.requested += 1;
        self.renders.insert(order.id);
        if let Some(dropped) = self.renderer.order(order) {
            self.renders.remove(&dropped);
            if let Some(session) = self.develop.as_mut() {
                session.counts.superseded += 1;
            }
        }
    }

    /// A render the worker has finished with.
    pub(crate) fn render_delivered(&mut self, delivery: Delivery) -> bool {
        self.renders.remove(&delivery.id);
        let Some(session) = self.session_of(delivery.generation) else {
            return false;
        };
        let frame = match delivery.outcome {
            Outcome::Rendered(frame) => *frame,
            Outcome::Alpha(alpha) => {
                session.counts.rendered += 1;
                let Work::Alpha(mask) = delivery.work else {
                    return false;
                };
                // An overlay of a mask no longer shown is dropped.
                if session.overlay_mask != Some(mask) {
                    return false;
                }
                session.overlay = alpha.map(|alpha| MaskOverlay {
                    mask,
                    alpha: Arc::new(alpha),
                    view: delivery.view,
                    version: delivery.version,
                });
                return true;
            }
            Outcome::Cancelled => {
                session.counts.superseded += 1;
                return false;
            }
            Outcome::Failed(message) => {
                session.counts.failed += 1;
                self.toasts
                    .error(format!("The photo could not be rendered: {message}"));
                return true;
            }
        };
        session.counts.rendered += 1;
        let Work::Render(quality) = delivery.work else {
            return false;
        };
        let shown = DevelopFrame {
            image: Arc::new(frame.image),
            view: delivery.view,
            version: delivery.version,
            quality,
            stage_times: frame.stage_times.into(),
            render_time: frame.render_time,
            histogram: Arc::new(frame.histogram),
        };
        match delivery.purpose {
            Purpose::Main => {
                session.frame = Some(shown);
                session.reach(DevelopLoad::Rendered);
            }
            // A frame asked for before the choice changed, or was turned
            // off, is not what is shown now.
            Purpose::Before => {
                if session.before.is_none() || delivery.id <= session.before_since {
                    return false;
                }
                session.before_frame = Some(shown);
            }
            Purpose::Look => {
                let Some((key, _, _)) = &session.look else {
                    return false;
                };
                if delivery.id <= session.look_since {
                    return false;
                }
                session.look_frame = Some((*key, shown));
            }
            Purpose::Overlay => return false,
        }
        true
    }

    /// The settings a choice of before view shows for `photo`.
    pub(crate) fn before_base(
        &self,
        photo: PhotoId,
        kind: FileKind,
        choice: BeforeSettings,
    ) -> Result<DevelopSettings, EngineError> {
        Ok(match choice {
            BeforeSettings::Default => DevelopSettings::default_for(kind),
            BeforeSettings::Snapshot(id) => self.snapshot_settings(photo, id)?,
            BeforeSettings::HistoryStep(step) => self.catalog.history_settings(step)?,
        })
    }

    fn snapshot_settings(
        &self,
        photo: PhotoId,
        id: SnapshotId,
    ) -> Result<DevelopSettings, EngineError> {
        self.catalog
            .snapshots(photo)?
            .into_iter()
            .find(|snapshot| snapshot.id == id)
            .map(|snapshot| snapshot.settings)
            .ok_or_else(|| {
                CatalogError::NotFound {
                    entity: Entity::Snapshot,
                    id: id.0,
                }
                .into()
            })
    }
}
