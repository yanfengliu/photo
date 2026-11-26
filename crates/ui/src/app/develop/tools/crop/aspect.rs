//! What the crop tool does to the rectangle's shape: the aspect menu and
//! the ratio it locks. (Quarter turns and mirrors, which carry the crop
//! with the photo, are `photo_core::develop::DevelopSettings::turn`.)

use egui::Vec2;
use photo_core::develop::CropConstraint;

/// An entry of the aspect menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Aspect {
    /// The photo's own shape, whichever way it is turned.
    Original,
    /// The shape the camera recorded, as a ratio that stays as it is.
    AsShot,
    /// A named ratio, short side first; it is laid the way the rectangle
    /// lies.
    Ratio(f32, f32),
    /// The ratio typed into the two fields.
    Custom,
}

/// The aspect menu, top to bottom.
pub(crate) const ASPECTS: [(&str, Aspect); 8] = [
    ("Original", Aspect::Original),
    ("As Shot", Aspect::AsShot),
    ("1 : 1", Aspect::Ratio(1.0, 1.0)),
    ("4 : 5", Aspect::Ratio(4.0, 5.0)),
    ("5 : 7", Aspect::Ratio(5.0, 7.0)),
    ("2 : 3", Aspect::Ratio(2.0, 3.0)),
    ("16 : 9", Aspect::Ratio(9.0, 16.0)),
    ("Custom", Aspect::Custom),
];

impl Aspect {
    /// What the menu calls it.
    pub(crate) fn label(self) -> &'static str {
        ASPECTS
            .iter()
            .find(|(_, aspect)| *aspect == self)
            .map_or("Custom", |(label, _)| label)
    }

    /// The constraint it sets, for a frame of `frame` pixels and a
    /// rectangle that lies `landscape` or not.
    pub(crate) fn constraint(
        self,
        frame: Vec2,
        landscape: bool,
        custom: (f32, f32),
    ) -> CropConstraint {
        let (width, height) = match self {
            Self::Original => return CropConstraint::Original,
            Self::AsShot => (frame.x, frame.y),
            Self::Ratio(short, long) if landscape => (long, short),
            Self::Ratio(short, long) => (short, long),
            Self::Custom => custom,
        };
        CropConstraint::Ratio { width, height }.sanitized()
    }

    /// The menu entry a stored constraint is; `None` for no constraint.
    pub(crate) fn of(constraint: CropConstraint, frame: Vec2) -> Option<Self> {
        let (width, height) = match constraint {
            CropConstraint::Original => return Some(Self::Original),
            CropConstraint::Free => return None,
            CropConstraint::Ratio { width, height } => (width, height),
        };
        let same = |a: f32, b: f32| (a / b - width / height).abs() < 1.0e-3 * (width / height);
        let named = ASPECTS.iter().find_map(|(_, aspect)| match aspect {
            Self::Ratio(short, long) if same(*short, *long) || same(*long, *short) => Some(*aspect),
            _ => None,
        });
        Some(named.unwrap_or(if same(frame.x, frame.y) {
            Self::AsShot
        } else {
            Self::Custom
        }))
    }
}

/// The ratio (width over height) a constraint holds the rectangle to, in a
/// frame of `frame` pixels; `None` when it is free.
pub(crate) fn locked_ratio(constraint: CropConstraint, frame: Vec2) -> Option<f32> {
    match constraint.sanitized() {
        CropConstraint::Original => Some(frame.x / frame.y),
        CropConstraint::Free => None,
        CropConstraint::Ratio { width, height } => Some(width / height),
    }
}

#[cfg(test)]
mod tests;
