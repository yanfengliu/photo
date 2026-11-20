//! Changing what photos are marked with: ratings, flags, labels, text,
//! orientation, virtual copies and removal. Every change is one catalog
//! write for the whole target, is undoable per photo, and confirms itself
//! with a toast.

use photo_catalog::{DevelopEdit, PhotoRecord, TextField};
use photo_core::develop::{DevelopSettings, Orientation};
use photo_core::meta::{ColorLabel, Flag, PhotoId, Rating};

use crate::commands::Target;
use crate::engine::Engine;
use crate::error::EngineError;
use crate::undo::{Command, Presence, Values};

impl Engine {
    fn records(&self, target: &Target) -> Result<Vec<PhotoRecord>, EngineError> {
        let photos = self.resolve(target);
        if photos.is_empty() {
            return Ok(Vec::new());
        }
        Ok(self.catalog.photos(&photos)?)
    }

    /// Give every target photo this rating ("Set Rating to 3").
    pub fn set_rating(&mut self, target: Target, rating: Rating) -> Result<(), EngineError> {
        self.change_ratings(
            &target,
            |_| rating,
            |_| format!("Set Rating to {}", rating.stars()),
        )
    }

    /// One star more for every target photo, five at most.
    pub fn increase_rating(&mut self, target: Target) -> Result<(), EngineError> {
        self.change_ratings(&target, Rating::increased, |single| match single {
            Some(rating) => format!("Set Rating to {}", rating.stars()),
            None => "Increase Rating".to_string(),
        })
    }

    /// One star fewer for every target photo, none at least.
    pub fn decrease_rating(&mut self, target: Target) -> Result<(), EngineError> {
        self.change_ratings(&target, Rating::decreased, |single| match single {
            Some(rating) => format!("Set Rating to {}", rating.stars()),
            None => "Decrease Rating".to_string(),
        })
    }

    /// Rate each target photo `next(its rating)`. `label` gets the one new
    /// rating when the target is a single photo.
    fn change_ratings(
        &mut self,
        target: &Target,
        next: impl Fn(Rating) -> Rating,
        label: impl FnOnce(Option<Rating>) -> String,
    ) -> Result<(), EngineError> {
        let records = self.records(target)?;
        if records.is_empty() {
            return Ok(());
        }
        let single = match records.as_slice() {
            [only] => Some(next(only.rating)),
            _ => None,
        };
        let values = Values::changes(records.iter().map(|r| (r.id, r.rating, next(r.rating))));
        self.apply_and_record(label(single), values.map(Command::Ratings))
    }

    /// Flag every target photo ("Flag as Pick").
    pub fn set_flag(&mut self, target: Target, flag: Flag) -> Result<(), EngineError> {
        let records = self.records(&target)?;
        if records.is_empty() {
            return Ok(());
        }
        let label = match flag {
            Flag::Picked => "Flag as Pick",
            Flag::Rejected => "Flag as Rejected",
            Flag::Unflagged => "Unflag",
        };
        let values = Values::changes(records.iter().map(|r| (r.id, r.flag, flag)));
        self.apply_and_record(label.to_string(), values.map(Command::Flags))
    }

    /// Give every target photo this colour label, except that a photo that
    /// already has it loses it. [`ColorLabel::None`] clears every label.
    pub fn toggle_label(&mut self, target: Target, label: ColorLabel) -> Result<(), EngineError> {
        let records = self.records(&target)?;
        if records.is_empty() {
            return Ok(());
        }
        let next = |current: ColorLabel| {
            if current == label {
                ColorLabel::None
            } else {
                label
            }
        };
        let values = Values::changes(records.iter().map(|r| (r.id, r.label, next(r.label))));
        let cleared = records.iter().all(|r| next(r.label) == ColorLabel::None);
        let message = if cleared {
            "Clear Color Label".to_string()
        } else {
            format!("Set Color Label to {}", label.name())
        };
        self.apply_and_record(message, values.map(Command::Labels))
    }

    /// Set one text field of every target photo.
    pub fn set_text(
        &mut self,
        target: Target,
        field: TextField,
        text: &str,
    ) -> Result<(), EngineError> {
        let records = self.records(&target)?;
        if records.is_empty() {
            return Ok(());
        }
        let current = |record: &PhotoRecord| match field {
            TextField::Title => record.title.clone(),
            TextField::Caption => record.caption.clone(),
            TextField::Creator => record.creator.clone(),
            TextField::Copyright => record.copyright.clone(),
        };
        let values = Values::changes(records.iter().map(|r| (r.id, current(r), text.to_string())));
        let name = match field {
            TextField::Title => "Title",
            TextField::Caption => "Caption",
            TextField::Creator => "Creator",
            TextField::Copyright => "Copyright",
        };
        self.apply_and_record(
            format!("Set {name}"),
            values.map(|v| Command::Text(field, v)),
        )
    }

    /// Turn every target photo a quarter turn counter-clockwise.
    pub fn rotate_left(&mut self, target: Target) -> Result<(), EngineError> {
        self.reorient(&target, "Rotate Left", Orientation::rotated_ccw)
    }

    /// Turn every target photo a quarter turn clockwise.
    pub fn rotate_right(&mut self, target: Target) -> Result<(), EngineError> {
        self.reorient(&target, "Rotate Right", Orientation::rotated_cw)
    }

    /// Mirror every target photo left to right, as it is seen now.
    pub fn flip_horizontal(&mut self, target: Target) -> Result<(), EngineError> {
        self.reorient(&target, "Flip Horizontal", Orientation::flipped_horizontal)
    }

    /// Mirror every target photo top to bottom, as it is seen now.
    pub fn flip_vertical(&mut self, target: Target) -> Result<(), EngineError> {
        self.reorient(&target, "Flip Vertical", Orientation::flipped_vertical)
    }

    /// Change the orientation in each photo's develop settings, as one
    /// develop edit per photo with a history step, in one catalog write.
    fn reorient(
        &mut self,
        target: &Target,
        label: &str,
        turn: fn(Orientation) -> Orientation,
    ) -> Result<(), EngineError> {
        let photos = self.resolve(target);
        if photos.is_empty() {
            return Ok(());
        }
        let mut turned: Vec<(PhotoId, DevelopSettings)> = Vec::with_capacity(photos.len());
        for photo in photos {
            let mut settings = self.catalog.develop_settings(photo)?;
            settings.orientation = turn(settings.orientation);
            turned.push((photo, settings));
        }
        let edits: Vec<DevelopEdit<'_>> = turned
            .iter()
            .map(|(photo, settings)| DevelopEdit {
                photo: *photo,
                settings,
                history_label: Some(label),
            })
            .collect();
        let changes = self.catalog.apply_develop_edits(&edits)?;
        self.record(label.to_string(), Command::Develop(changes));
        Ok(())
    }

    /// Make a virtual copy of every target photo and select the copies.
    /// Returns the new photos' ids.
    pub fn create_virtual_copies(&mut self, target: Target) -> Result<Vec<PhotoId>, EngineError> {
        let sources = self.resolve(&target);
        let mut created = Vec::with_capacity(sources.len());
        // The catalog makes one copy per call.
        for source in &sources {
            match self.catalog.create_virtual_copy(*source) {
                Ok(copy) => created.push(copy),
                Err(error) => {
                    if !created.is_empty()
                        && let Err(undo) = self.catalog.remove_photos(&created)
                    {
                        log::error!("could not take back the copies already made: {undo}");
                    }
                    return Err(error.into());
                }
            }
        }
        if created.is_empty() {
            return Ok(created);
        }
        let label = match created.len() {
            1 => "Create Virtual Copy".to_string(),
            count => format!("Create {count} Virtual Copies"),
        };
        self.record(label, Command::Copies(Presence::Present(created.clone())));
        self.selection
            .set(&self.browser, &created, created.first().copied());
        Ok(created)
    }

    /// Remove every target photo (and the virtual copies of any master
    /// among them) from the catalog. The files are not touched; undo brings
    /// everything back under the same ids.
    pub fn remove_from_catalog(&mut self, target: Target) -> Result<(), EngineError> {
        let photos = self.resolve(&target);
        if photos.is_empty() {
            return Ok(());
        }
        let removed = self.catalog.remove_photos(&photos)?;
        let label = match removed.photo_ids().len() {
            1 => "Remove Photo from Catalog".to_string(),
            count => format!("Remove {count} Photos from Catalog"),
        };
        self.record(
            label,
            Command::Removal(Presence::Removed(Box::new(removed))),
        );
        Ok(())
    }
}
