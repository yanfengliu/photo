//! Asking the render worker for the session's frames, and taking them in:
//! the canvas, the before view and the preset hover preview.

use std::sync::Arc;

use photo_catalog::{CatalogError, Entity};
use photo_core::develop::DevelopSettings;
use photo_core::meta::{FileKind, PhotoId, SnapshotId};
use photo_develop::Quality;

use super::worker::{Delivery, Order, Outcome, Purpose};
use super::{BeforeSettings, DevelopFrame, DevelopLoad, develop_view, tools};
use crate::engine::Engine;
use crate::error::EngineError;

impl Engine {
    /// Ask the render worker for a frame of the open photo for `purpose`.
    /// Does nothing until the photo is decoded and the canvas has a view,
    /// or when there is nothing to show for that purpose.
    pub(crate) fn render_session(&mut self, purpose: Purpose, quality: Quality) {
        let Some(session) = self.develop.as_mut() else {
            return;
        };
        let (Some(source), Some(canvas)) = (session.source.clone(), session.canvas) else {
            return;
        };
        let (settings, view) = match purpose {
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
            quality,
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
        let shown = DevelopFrame {
            image: Arc::new(frame.image),
            view: delivery.view,
            version: delivery.version,
            quality: delivery.quality,
            stage_times: frame.stage_times.into(),
            render_time: frame.render_time,
            histogram: Arc::new(frame.histogram),
        };
        match delivery.purpose {
            Purpose::Main => {
                session.frame = Some(shown);
                session.reach(DevelopLoad::Rendered);
            }
            Purpose::Before => session.before_frame = Some(shown),
            Purpose::Look => {
                if let Some((key, _, _)) = &session.look {
                    session.look_frame = Some((*key, shown));
                }
            }
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
