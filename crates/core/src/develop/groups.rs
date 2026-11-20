//! Groups of settings that travel together when copying, pasting, syncing
//! and saving presets.

use serde::{Deserialize, Serialize};

use super::DevelopSettings;

/// A named slice of [`DevelopSettings`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SettingsGroup {
    Treatment,
    WhiteBalance,
    Tone,
    Presence,
    ToneCurve,
    ColorMixer,
    ColorGrading,
    Sharpening,
    NoiseReduction,
    LensCorrections,
    Transform,
    Vignette,
    Grain,
    Calibration,
    Crop,
    Healing,
    Masking,
}

impl SettingsGroup {
    pub const ALL: [Self; 17] = [
        Self::Treatment,
        Self::WhiteBalance,
        Self::Tone,
        Self::Presence,
        Self::ToneCurve,
        Self::ColorMixer,
        Self::ColorGrading,
        Self::Sharpening,
        Self::NoiseReduction,
        Self::LensCorrections,
        Self::Transform,
        Self::Vignette,
        Self::Grain,
        Self::Calibration,
        Self::Crop,
        Self::Healing,
        Self::Masking,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Treatment => "Treatment & Profile",
            Self::WhiteBalance => "White Balance",
            Self::Tone => "Tone",
            Self::Presence => "Presence",
            Self::ToneCurve => "Tone Curve",
            Self::ColorMixer => "Color Mixer",
            Self::ColorGrading => "Color Grading",
            Self::Sharpening => "Sharpening",
            Self::NoiseReduction => "Noise Reduction",
            Self::LensCorrections => "Lens Corrections",
            Self::Transform => "Transform",
            Self::Vignette => "Post-Crop Vignetting",
            Self::Grain => "Grain",
            Self::Calibration => "Calibration",
            Self::Crop => "Crop",
            Self::Healing => "Healing",
            Self::Masking => "Masking",
        }
    }

    fn bit(self) -> u32 {
        1 << self as u32
    }
}

/// A set of [`SettingsGroup`]s.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(from = "Vec<SettingsGroup>", into = "Vec<SettingsGroup>")]
pub struct GroupSet(u32);

impl GroupSet {
    pub const NONE: Self = Self(0);

    pub fn all() -> Self {
        SettingsGroup::ALL.into_iter().collect()
    }

    /// What copy and sync offer by default: everything that is safe to apply
    /// to a different frame. Crop, healing and masks are tied to one
    /// composition, so they are opt-in.
    pub fn default_copy() -> Self {
        Self::all()
            .without(SettingsGroup::Crop)
            .without(SettingsGroup::Healing)
            .without(SettingsGroup::Masking)
    }

    pub fn contains(self, group: SettingsGroup) -> bool {
        self.0 & group.bit() != 0
    }

    pub fn with(self, group: SettingsGroup) -> Self {
        Self(self.0 | group.bit())
    }

    pub fn without(self, group: SettingsGroup) -> Self {
        Self(self.0 & !group.bit())
    }

    pub fn set(&mut self, group: SettingsGroup, included: bool) {
        *self = if included {
            self.with(group)
        } else {
            self.without(group)
        };
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn iter(self) -> impl Iterator<Item = SettingsGroup> {
        SettingsGroup::ALL
            .into_iter()
            .filter(move |group| self.contains(*group))
    }
}

impl FromIterator<SettingsGroup> for GroupSet {
    fn from_iter<I: IntoIterator<Item = SettingsGroup>>(groups: I) -> Self {
        groups.into_iter().fold(Self::NONE, Self::with)
    }
}

impl From<Vec<SettingsGroup>> for GroupSet {
    fn from(groups: Vec<SettingsGroup>) -> Self {
        groups.into_iter().collect()
    }
}

impl From<GroupSet> for Vec<SettingsGroup> {
    fn from(set: GroupSet) -> Self {
        set.iter().collect()
    }
}

impl DevelopSettings {
    /// Overwrite the chosen groups of `self` with those of `source`, leaving
    /// everything else untouched.
    pub fn copy_groups_from(&mut self, source: &Self, groups: GroupSet) {
        for group in groups.iter() {
            match group {
                SettingsGroup::Treatment => {
                    self.treatment = source.treatment;
                    self.profile = source.profile;
                }
                SettingsGroup::WhiteBalance => self.white_balance = source.white_balance,
                SettingsGroup::Tone => self.tone = source.tone,
                SettingsGroup::Presence => self.presence = source.presence,
                SettingsGroup::ToneCurve => self.tone_curve = source.tone_curve.clone(),
                SettingsGroup::ColorMixer => self.color_mixer = source.color_mixer,
                SettingsGroup::ColorGrading => self.color_grading = source.color_grading,
                SettingsGroup::Sharpening => self.detail.sharpening = source.detail.sharpening,
                SettingsGroup::NoiseReduction => {
                    self.detail.noise_reduction = source.detail.noise_reduction;
                }
                SettingsGroup::LensCorrections => self.lens = source.lens,
                SettingsGroup::Transform => self.transform = source.transform,
                SettingsGroup::Vignette => self.effects.vignette = source.effects.vignette,
                SettingsGroup::Grain => self.effects.grain = source.effects.grain,
                SettingsGroup::Calibration => self.calibration = source.calibration,
                SettingsGroup::Crop => {
                    self.crop = source.crop;
                    self.orientation = source.orientation;
                }
                SettingsGroup::Healing => self.spots = source.spots.clone(),
                SettingsGroup::Masking => self.masks = source.masks.clone(),
            }
        }
    }

    /// The groups in which `self` and `other` differ.
    pub fn differing_groups(&self, other: &Self) -> GroupSet {
        SettingsGroup::ALL
            .into_iter()
            .filter(|group| {
                let mut probe = self.clone();
                probe.copy_groups_from(other, GroupSet::NONE.with(*group));
                probe != *self
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edited() -> DevelopSettings {
        let mut settings = DevelopSettings::default();
        settings.tone.exposure = 1.0;
        settings.presence.clarity = 30.0;
        settings.crop.angle = 3.0;
        settings.effects.grain.amount = 20.0;
        settings
    }

    #[test]
    fn copying_every_group_reproduces_the_source() {
        let source = edited();
        let mut target = DevelopSettings::default();
        target.copy_groups_from(&source, GroupSet::all());
        assert_eq!(target, source);
    }

    #[test]
    fn copying_one_group_leaves_the_others_alone() {
        let source = edited();
        let mut target = DevelopSettings::default();
        target.copy_groups_from(&source, GroupSet::NONE.with(SettingsGroup::Tone));
        assert_eq!(target.tone.exposure, 1.0);
        assert_eq!(target.presence.clarity, 0.0);
        assert_eq!(target.crop.angle, 0.0);
    }

    #[test]
    fn default_copy_leaves_out_what_is_tied_to_one_composition() {
        let set = GroupSet::default_copy();
        assert!(set.contains(SettingsGroup::Tone));
        for group in [
            SettingsGroup::Crop,
            SettingsGroup::Healing,
            SettingsGroup::Masking,
        ] {
            assert!(!set.contains(group));
        }
    }

    #[test]
    fn differing_groups_names_exactly_what_changed() {
        let differing = DevelopSettings::default().differing_groups(&edited());
        let expected: GroupSet = [
            SettingsGroup::Tone,
            SettingsGroup::Presence,
            SettingsGroup::Crop,
            SettingsGroup::Grain,
        ]
        .into_iter()
        .collect();
        assert_eq!(differing, expected);
    }

    #[test]
    fn a_group_set_serializes_as_a_list_of_names() {
        let set = GroupSet::NONE
            .with(SettingsGroup::Tone)
            .with(SettingsGroup::ToneCurve);
        let json = serde_json::to_string(&set).unwrap();
        assert_eq!(json, r#"["tone","tone_curve"]"#);
        assert_eq!(serde_json::from_str::<GroupSet>(&json).unwrap(), set);
    }
}
