//! Presets, snapshots and history steps: saved settings applied to photos.
//! Applying any of them is undoable; creating, renaming and deleting
//! presets and snapshots is not (the catalog keeps no restore data for
//! them).

use photo_catalog::{CatalogError, Entity, Snapshot};
use photo_core::develop::{
    CurvePoint, DevelopSettings, GroupSet, PointCurve, SettingsGroup, Treatment, WbPreset,
    WhiteBalance,
};
use photo_core::meta::{FileKind, HistoryId, PhotoId, PresetId, SnapshotId};

use crate::commands::Target;
use crate::engine::Engine;
use crate::error::EngineError;
use crate::undo::Command;

/// The group the built-in presets are listed under.
pub const BUILT_IN_GROUP: &str = "Photo Looks";

/// The looks that come with the application.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BuiltInPreset {
    /// Black and white with an S-shaped curve and extra clarity.
    MonoContrast,
    /// Richer colour: vibrance, saturation and a little clarity.
    Vivid,
    /// Gentler detail: less clarity and texture.
    Soft,
    /// A warmer white balance.
    Warm,
    /// A cooler white balance.
    Cool,
    /// Lifted blacks, softened whites and fine grain.
    Faded,
}

impl BuiltInPreset {
    /// Every built-in preset, in the order they are listed.
    pub const ALL: [Self; 6] = [
        Self::MonoContrast,
        Self::Vivid,
        Self::Soft,
        Self::Warm,
        Self::Cool,
        Self::Faded,
    ];

    /// The name shown in the Presets panel.
    pub fn name(self) -> &'static str {
        match self {
            Self::MonoContrast => "Mono Contrast",
            Self::Vivid => "Vivid Colour",
            Self::Soft => "Soft Detail",
            Self::Warm => "Warm Light",
            Self::Cool => "Cool Light",
            Self::Faded => "Faded Print",
        }
    }

    /// The settings groups the preset sets; applying it leaves every other
    /// group as it was.
    pub fn groups(self) -> GroupSet {
        let groups: &[SettingsGroup] = match self {
            Self::MonoContrast => &[
                SettingsGroup::Treatment,
                SettingsGroup::ToneCurve,
                SettingsGroup::Presence,
            ],
            Self::Vivid | Self::Soft => &[SettingsGroup::Presence],
            Self::Warm | Self::Cool => &[SettingsGroup::WhiteBalance],
            Self::Faded => &[SettingsGroup::ToneCurve, SettingsGroup::Grain],
        };
        groups.iter().copied().collect()
    }

    /// The preset's settings for a photo of `kind`. The white balance looks
    /// shift a raster file's relative temperature; a RAW file's white
    /// balance is absolute, so for it they choose the Cloudy (warm) or
    /// Fluorescent (cool) illuminant.
    pub fn settings(self, kind: FileKind) -> DevelopSettings {
        let mut settings = DevelopSettings::default_for(kind);
        match self {
            Self::MonoContrast => {
                settings.treatment = Treatment::BlackAndWhite;
                settings.tone_curve.parametric.lights = 20.0;
                settings.tone_curve.parametric.darks = -20.0;
                settings.presence.clarity = 20.0;
            }
            Self::Vivid => {
                settings.presence.vibrance = 40.0;
                settings.presence.saturation = 10.0;
                settings.presence.clarity = 10.0;
            }
            Self::Soft => {
                settings.presence.clarity = -25.0;
                settings.presence.texture = -15.0;
            }
            Self::Warm => settings.white_balance = shifted(kind, 25.0, WbPreset::Cloudy),
            Self::Cool => settings.white_balance = shifted(kind, -25.0, WbPreset::Fluorescent),
            Self::Faded => {
                settings.tone_curve.rgb = PointCurve::new([
                    CurvePoint::new(0.0, 0.1),
                    CurvePoint::new(0.5, 0.52),
                    CurvePoint::new(1.0, 0.94),
                ]);
                settings.effects.grain.amount = 15.0;
            }
        }
        settings
    }
}

/// A warmer or cooler white balance for a photo of `kind`.
fn shifted(kind: FileKind, relative: f32, illuminant: WbPreset) -> WhiteBalance {
    match (kind, illuminant.illuminant()) {
        (FileKind::Raw, Some((temp, tint))) => WhiteBalance {
            preset: illuminant,
            temp,
            tint,
        },
        _ => WhiteBalance {
            preset: WbPreset::Custom,
            temp: relative,
            tint: 0.0,
        },
    }
}

/// Which preset.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PresetKey {
    /// One that comes with the application.
    BuiltIn(BuiltInPreset),
    /// One the user saved in the catalog.
    User(PresetId),
}

/// A preset as the Presets panel lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PresetInfo {
    /// Which preset.
    pub key: PresetKey,
    /// The group it is listed under.
    pub group: String,
    /// Its name.
    pub name: String,
    /// The settings groups it applies.
    pub groups: GroupSet,
}

/// A preset ready to apply.
pub(crate) struct Resolved {
    name: String,
    groups: GroupSet,
    stored: Option<DevelopSettings>,
    built_in: Option<BuiltInPreset>,
}

impl Resolved {
    /// `base` with the preset's groups laid over it, for a photo of `kind`.
    pub(crate) fn applied_to(&self, base: &DevelopSettings, kind: FileKind) -> DevelopSettings {
        let preset = match (&self.stored, self.built_in) {
            (Some(stored), _) => stored.clone(),
            (None, Some(built_in)) => built_in.settings(kind),
            (None, None) => base.clone(),
        };
        let mut applied = base.clone();
        applied.copy_groups_from(&preset, self.groups);
        applied.sanitize_for(kind);
        applied
    }
}

impl Engine {
    /// Every preset: the built-in ones first, then the user's in group and
    /// name order. Reads the catalog.
    pub fn presets(&self) -> Result<Vec<PresetInfo>, EngineError> {
        let built_in = BuiltInPreset::ALL.into_iter().map(|preset| PresetInfo {
            key: PresetKey::BuiltIn(preset),
            group: BUILT_IN_GROUP.to_string(),
            name: preset.name().to_string(),
            groups: preset.groups(),
        });
        let stored = self
            .catalog
            .presets()?
            .into_iter()
            .map(|preset| PresetInfo {
                key: PresetKey::User(preset.id),
                group: preset.group,
                name: preset.name,
                groups: preset.groups,
            });
        Ok(built_in.chain(stored).collect())
    }

    pub(crate) fn resolve_preset(&self, key: &PresetKey) -> Result<Resolved, EngineError> {
        match *key {
            PresetKey::BuiltIn(preset) => Ok(Resolved {
                name: preset.name().to_string(),
                groups: preset.groups(),
                stored: None,
                built_in: Some(preset),
            }),
            PresetKey::User(id) => {
                let preset = self
                    .catalog
                    .presets()?
                    .into_iter()
                    .find(|preset| preset.id == id)
                    .ok_or(CatalogError::NotFound {
                        entity: Entity::Preset,
                        id: id.0,
                    })?;
                Ok(Resolved {
                    name: preset.name,
                    groups: preset.groups,
                    stored: Some(preset.settings),
                    built_in: None,
                })
            }
        }
    }

    /// The settings a new preset is taken from: the photo open in Develop,
    /// with its pending edit committed, else the active photo.
    fn current_settings(&mut self) -> Result<DevelopSettings, EngineError> {
        let photo = match &self.develop {
            Some(session) => session.photo,
            None => self.selection.active().ok_or(EngineError::NoActivePhoto)?,
        };
        self.settings_now(photo)
    }

    /// Save the chosen `groups` of the current settings (the photo open in
    /// Develop, else the active photo) as a user preset.
    pub fn create_preset(
        &mut self,
        group: &str,
        name: &str,
        groups: GroupSet,
    ) -> Result<PresetId, EngineError> {
        let settings = self.current_settings()?;
        let id = self.catalog.create_preset(group, name, &settings, groups)?;
        self.toasts
            .info(format!("Create Preset \"{}\"", name.trim()));
        Ok(id)
    }

    /// Give a user preset a new group and name.
    pub fn rename_preset(
        &mut self,
        id: PresetId,
        group: &str,
        name: &str,
    ) -> Result<(), EngineError> {
        self.catalog.rename_preset(id, group, name)?;
        self.toasts
            .info(format!("Rename Preset to \"{}\"", name.trim()));
        Ok(())
    }

    /// Delete a user preset. Photos it was applied to keep their settings.
    pub fn delete_preset(&mut self, id: PresetId) -> Result<(), EngineError> {
        self.catalog.delete_preset(id)?;
        self.toasts.info("Delete Preset");
        Ok(())
    }

    /// Apply a preset's groups to every target photo, as one undoable step.
    pub fn apply_preset(&mut self, preset: &PresetKey, target: Target) -> Result<(), EngineError> {
        let resolved = self.resolve_preset(preset)?;
        let photos = self.resolve(&target);
        let editable = self.editable_settings(&photos)?;
        let label = format!("Preset: {}", resolved.name);
        self.write_develop(&label, editable, |_, current, kind| {
            resolved.applied_to(current, kind)
        })
    }

    /// A photo's snapshots, oldest first. Reads the catalog.
    pub fn snapshots(&self, photo: PhotoId) -> Result<Vec<Snapshot>, EngineError> {
        Ok(self.catalog.snapshots(photo)?)
    }

    /// Save the photo's current settings as a snapshot. For the photo open
    /// in Develop that is what it shows, with its pending edit committed.
    pub fn create_snapshot(
        &mut self,
        photo: PhotoId,
        name: &str,
    ) -> Result<SnapshotId, EngineError> {
        let settings = self.settings_now(photo)?;
        let id = self.catalog.create_snapshot(photo, name, &settings)?;
        self.toasts
            .info(format!("Create Snapshot \"{}\"", name.trim()));
        Ok(id)
    }

    /// Rename a snapshot.
    pub fn rename_snapshot(&mut self, id: SnapshotId, name: &str) -> Result<(), EngineError> {
        self.catalog.rename_snapshot(id, name)?;
        self.toasts
            .info(format!("Rename Snapshot to \"{}\"", name.trim()));
        Ok(())
    }

    /// Delete a snapshot.
    pub fn delete_snapshot(&mut self, id: SnapshotId) -> Result<(), EngineError> {
        self.catalog.delete_snapshot(id)?;
        self.toasts.info("Delete Snapshot");
        Ok(())
    }

    /// Give the photo a snapshot's settings, as one undoable step.
    pub fn apply_snapshot(&mut self, photo: PhotoId, id: SnapshotId) -> Result<(), EngineError> {
        let snapshot = self
            .catalog
            .snapshots(photo)?
            .into_iter()
            .find(|snapshot| snapshot.id == id)
            .ok_or(CatalogError::NotFound {
                entity: Entity::Snapshot,
                id: id.0,
            })?;
        let editable = self.editable_settings(&[photo])?;
        let label = format!("Snapshot: {}", snapshot.name);
        self.write_develop(&label, editable, |_, _, _| snapshot.settings.clone())
    }

    /// Take the photo to one of its own history steps, as one undoable step.
    /// The later steps stay listed until the next edit replaces them.
    pub fn go_to_history_step(
        &mut self,
        photo: PhotoId,
        step: HistoryId,
    ) -> Result<(), EngineError> {
        self.editable_settings(&[photo])?;
        let label = self
            .catalog
            .history(photo)?
            .into_iter()
            .find(|entry| entry.id == step)
            .map_or_else(
                || "History Step".to_string(),
                |entry| format!("History: {}", entry.label),
            );
        let change = self.catalog.go_to_history_step(photo, step)?;
        self.record(label, Command::Develop(vec![change]));
        Ok(())
    }
}
