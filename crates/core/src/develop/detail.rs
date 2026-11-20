//! The Detail panel (sharpening and noise reduction) and the Effects panel
//! (post-crop vignette and grain).

use serde::{Deserialize, Serialize};

/// Capture sharpening. `amount` 0 to 150, `radius` 0.5 to 3.0 pixels at full
/// resolution, `detail` and `masking` 0 to 100.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Sharpening {
    pub amount: f32,
    pub radius: f32,
    pub detail: f32,
    pub masking: f32,
}

impl Default for Sharpening {
    fn default() -> Self {
        Self {
            amount: 0.0,
            radius: 1.0,
            detail: 25.0,
            masking: 0.0,
        }
    }
}

/// Noise reduction, every field 0 to 100.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct NoiseReduction {
    pub luminance: f32,
    pub luminance_detail: f32,
    pub luminance_contrast: f32,
    pub color: f32,
    pub color_detail: f32,
    pub color_smoothness: f32,
}

impl Default for NoiseReduction {
    fn default() -> Self {
        Self {
            luminance: 0.0,
            luminance_detail: 50.0,
            luminance_contrast: 0.0,
            color: 0.0,
            color_detail: 50.0,
            color_smoothness: 50.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Detail {
    pub sharpening: Sharpening,
    pub noise_reduction: NoiseReduction,
}

/// Post-crop vignette. `amount` and `roundness` -100 to 100 (negative
/// `amount` darkens the corners); `midpoint`, `feather` and `highlights`
/// 0 to 100.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Vignette {
    pub amount: f32,
    pub midpoint: f32,
    pub roundness: f32,
    pub feather: f32,
    pub highlights: f32,
}

impl Default for Vignette {
    fn default() -> Self {
        Self {
            amount: 0.0,
            midpoint: 50.0,
            roundness: 0.0,
            feather: 50.0,
            highlights: 0.0,
        }
    }
}

/// Film grain, every field 0 to 100.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Grain {
    pub amount: f32,
    pub size: f32,
    pub roughness: f32,
}

impl Default for Grain {
    fn default() -> Self {
        Self {
            amount: 0.0,
            size: 25.0,
            roughness: 50.0,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Effects {
    pub vignette: Vignette,
    pub grain: Grain,
}
