//! Colour panels: the colour mixer (HSL and black-and-white mix), colour
//! grading and calibration.

use serde::{Deserialize, Serialize};

/// The eight hue bands of the colour mixer, in hue order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColorBand {
    Red,
    Orange,
    Yellow,
    Green,
    Aqua,
    Blue,
    Purple,
    Magenta,
}

impl ColorBand {
    pub const COUNT: usize = 8;
    pub const ALL: [Self; Self::COUNT] = [
        Self::Red,
        Self::Orange,
        Self::Yellow,
        Self::Green,
        Self::Aqua,
        Self::Blue,
        Self::Purple,
        Self::Magenta,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Red => "Red",
            Self::Orange => "Orange",
            Self::Yellow => "Yellow",
            Self::Green => "Green",
            Self::Aqua => "Aqua",
            Self::Blue => "Blue",
            Self::Purple => "Purple",
            Self::Magenta => "Magenta",
        }
    }
}

/// Per-band adjustments, each -100 to 100, indexed by [`ColorBand::index`].
/// `bw_mix` is used instead of the other three when the treatment is black
/// and white.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorMixer {
    pub hue: [f32; ColorBand::COUNT],
    pub saturation: [f32; ColorBand::COUNT],
    pub luminance: [f32; ColorBand::COUNT],
    pub bw_mix: [f32; ColorBand::COUNT],
}

impl ColorMixer {
    /// True when the colour (HSL) part does nothing.
    pub fn hsl_is_identity(&self) -> bool {
        let zero = |values: &[f32]| values.iter().all(|v| *v == 0.0);
        zero(&self.hue) && zero(&self.saturation) && zero(&self.luminance)
    }
}

/// One colour-grading wheel: a tint (`hue` 0 to 360 degrees, `saturation`
/// 0 to 100) and a `luminance` offset (-100 to 100).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GradeWheel {
    pub hue: f32,
    pub saturation: f32,
    pub luminance: f32,
}

impl GradeWheel {
    pub fn is_identity(&self) -> bool {
        self.saturation == 0.0 && self.luminance == 0.0
    }
}

/// Colour grading: tints for shadows, midtones and highlights plus a global
/// one. `blending` (0 to 100) sets how much the three ranges overlap;
/// `balance` (-100 to 100) shifts the split between shadows and highlights.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ColorGrading {
    pub shadows: GradeWheel,
    pub midtones: GradeWheel,
    pub highlights: GradeWheel,
    pub global: GradeWheel,
    pub blending: f32,
    pub balance: f32,
}

impl Default for ColorGrading {
    fn default() -> Self {
        Self {
            shadows: GradeWheel::default(),
            midtones: GradeWheel::default(),
            highlights: GradeWheel::default(),
            global: GradeWheel::default(),
            blending: 50.0,
            balance: 0.0,
        }
    }
}

impl ColorGrading {
    pub fn is_identity(&self) -> bool {
        self.shadows.is_identity()
            && self.midtones.is_identity()
            && self.highlights.is_identity()
            && self.global.is_identity()
    }
}

/// Calibration: a tint for the shadows and a hue/saturation shift for each
/// primary, all -100 to 100.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Calibration {
    pub shadows_tint: f32,
    pub red_hue: f32,
    pub red_saturation: f32,
    pub green_hue: f32,
    pub green_saturation: f32,
    pub blue_hue: f32,
    pub blue_saturation: f32,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn band_index_matches_position_in_all() {
        for (position, band) in ColorBand::ALL.into_iter().enumerate() {
            assert_eq!(band.index(), position);
        }
    }

    #[test]
    fn a_wheel_with_hue_but_no_saturation_does_nothing() {
        let wheel = GradeWheel {
            hue: 200.0,
            ..Default::default()
        };
        assert!(wheel.is_identity());
        assert!(ColorGrading::default().is_identity());
    }
}
