//! Local edits: healing spots and masked adjustments.
//!
//! Every position here is in normalized coordinates of the **source** frame
//! (see [`super::geometry`]), so local edits stay attached to image content
//! when the photo is later rotated, cropped or perspective-corrected. Lengths
//! (`radius`) are fractions of the source's long edge so that circles stay
//! circular in non-square images.

use serde::{Deserialize, Serialize};

use crate::geom::Point;

/// How a healing spot fills its target.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SpotMode {
    /// Copy texture from the source area, matching tone and colour to the
    /// surroundings of the target.
    #[default]
    Heal,
    /// Copy the source area as it is.
    Clone,
}

/// A circular retouching spot.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Spot {
    pub mode: SpotMode,
    /// Centre of the area being repaired.
    pub target: Point,
    /// Centre of the area the repair is sampled from.
    pub source: Point,
    /// Fraction of the source's long edge.
    pub radius: f32,
    /// 0 (hard edge) to 100 (soft from the centre).
    pub feather: f32,
    /// 0 to 100.
    pub opacity: f32,
}

impl Spot {
    /// Smallest and largest allowed `radius`.
    pub const MIN_RADIUS: f32 = 0.001;
    pub const MAX_RADIUS: f32 = 0.25;

    /// This spot with every number forced into range, or `None` when a
    /// position is not finite and the spot cannot be placed at all.
    pub fn sanitized(mut self) -> Option<Self> {
        let placed = [self.target, self.source]
            .iter()
            .all(|p| p.x.is_finite() && p.y.is_finite());
        if !placed {
            return None;
        }
        let in_range = |value: f32, min: f32, max: f32, fallback: f32| {
            if value.is_finite() {
                value.clamp(min, max)
            } else {
                fallback
            }
        };
        self.radius = in_range(self.radius, Self::MIN_RADIUS, Self::MAX_RADIUS, 0.02);
        self.feather = in_range(self.feather, 0.0, 100.0, 50.0);
        self.opacity = in_range(self.opacity, 0.0, 100.0, 100.0);
        Some(self)
    }
}

/// Adjustments a mask applies on top of the global ones. `exposure` is in
/// stops (-4 to 4); everything else runs from -100 to 100. `temp` and `tint`
/// are always relative offsets, whatever the source kind.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct LocalAdjustments {
    pub exposure: f32,
    pub contrast: f32,
    pub highlights: f32,
    pub shadows: f32,
    pub whites: f32,
    pub blacks: f32,
    pub temp: f32,
    pub tint: f32,
    pub saturation: f32,
    pub texture: f32,
    pub clarity: f32,
    pub dehaze: f32,
    pub sharpness: f32,
    pub noise: f32,
}

impl LocalAdjustments {
    pub fn is_identity(&self) -> bool {
        *self == Self::default()
    }
}

/// One painted stroke of a brush mask.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct BrushStroke {
    /// The path of the brush centre.
    pub points: Vec<Point>,
    /// Fraction of the source's long edge.
    pub radius: f32,
    /// 0 (hard edge) to 100 (soft from the centre).
    pub feather: f32,
    /// Strength of one pass, 1 to 100.
    pub flow: f32,
    /// Erase instead of paint.
    pub erase: bool,
}

/// The shape of one mask component.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MaskShape {
    /// Full effect on the `start` side fading to none at `end`, constant
    /// along the direction perpendicular to the line between them.
    Linear { start: Point, end: Point },
    /// An ellipse: full effect inside (or outside when `invert` is set),
    /// fading across the feathered edge.
    Radial {
        center: Point,
        /// Half-width as a fraction of the source's long edge.
        radius_x: f32,
        /// Half-height as a fraction of the source's long edge.
        radius_y: f32,
        /// Rotation of the ellipse in degrees.
        angle: f32,
        /// 0 (hard edge) to 100.
        feather: f32,
        invert: bool,
    },
    /// Freehand painted strokes.
    Brush { strokes: Vec<BrushStroke> },
}

impl MaskShape {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Linear { .. } => "Linear Gradient",
            Self::Radial { .. } => "Radial Gradient",
            Self::Brush { .. } => "Brush",
        }
    }
}

/// One shape contributing to a mask, either adding to it or cutting from it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct MaskComponent {
    pub shape: MaskShape,
    #[serde(default)]
    pub subtract: bool,
}

/// A mask: the combination of its components, and the adjustments applied
/// where it covers the image.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mask {
    pub name: String,
    pub enabled: bool,
    pub invert: bool,
    /// Overall strength of the adjustments, 0 to 100.
    pub amount: f32,
    pub components: Vec<MaskComponent>,
    pub adjust: LocalAdjustments,
}

impl Default for Mask {
    fn default() -> Self {
        Self {
            name: String::new(),
            enabled: true,
            invert: false,
            amount: 100.0,
            components: Vec::new(),
            adjust: LocalAdjustments::default(),
        }
    }
}

impl Mask {
    /// A new mask made of a single shape, named after it.
    pub fn from_shape(shape: MaskShape, number: usize) -> Self {
        Self {
            name: format!("{} {number}", shape.label()),
            components: vec![MaskComponent {
                shape,
                subtract: false,
            }],
            ..Self::default()
        }
    }

    /// True when rendering can skip this mask entirely.
    pub fn is_inert(&self) -> bool {
        !self.enabled
            || self.amount == 0.0
            || self.components.is_empty()
            || self.adjust.is_identity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn radial() -> MaskShape {
        MaskShape::Radial {
            center: Point::new(0.5, 0.5),
            radius_x: 0.2,
            radius_y: 0.1,
            angle: 0.0,
            feather: 50.0,
            invert: false,
        }
    }

    #[test]
    fn a_mask_without_adjustments_is_inert() {
        let mut mask = Mask::from_shape(radial(), 1);
        assert_eq!(mask.name, "Radial Gradient 1");
        assert!(mask.is_inert());
        mask.adjust.exposure = 1.0;
        assert!(!mask.is_inert());
        mask.enabled = false;
        assert!(mask.is_inert());
    }

    #[test]
    fn shapes_serialize_with_a_kind_tag() {
        let json = serde_json::to_string(&radial()).unwrap();
        assert!(json.contains(r#""kind":"radial""#), "{json}");
        assert_eq!(serde_json::from_str::<MaskShape>(&json).unwrap(), radial());
    }
}
