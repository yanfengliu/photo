//! Manual lens vignetting correction: a position-dependent gain applied to
//! the input stage's output (DESIGN §5.2, stage 3), defined in source
//! coordinates because the lens projected onto the sensor, not onto the
//! crop.

use photo_core::develop::LensCorrections;

/// Exposure change, in stops, at the corners for `vignetting` ±100.
const CORNER_STOPS: f32 = 1.0;

/// The correction compiled for one source size.
///
/// The gain at a point is `2^(a · r^p)`: `r` is the distance from the
/// source centre divided by the half-diagonal (1 at the corners), `a` is
/// `vignetting / 100` stops (positive brightens the corners, as the slider
/// says) and `p = 2^(1 + 2 · midpoint / 100)` runs from 2 (a broad
/// correction reaching far toward the centre) to 8 (the corners only). It
/// is the even-power radial falloff of polynomial vignetting models (such
/// as lensfun's `pa` model, `1 + k1 r² + k2 r⁴ + k3 r⁶`) reduced to one
/// term whose power the midpoint chooses, written in stops so that +100
/// and -100 undo each other. It is 1 at the centre, smooth, and monotonic
/// along every ray.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct LensVignetting {
    stops: f32,
    power: f32,
    /// Twice the width and height over the diagonal: turns a normalized
    /// offset from the centre into a fraction of the half-diagonal.
    scale: (f32, f32),
}

impl LensVignetting {
    /// `None` when the correction is off (or not a number).
    pub(crate) fn new(lens: &LensCorrections, source_size: (u32, u32)) -> Option<Self> {
        let amount = lens.vignetting;
        if !amount.is_finite() || amount == 0.0 {
            return None;
        }
        let midpoint = if lens.vignetting_midpoint.is_finite() {
            lens.vignetting_midpoint.clamp(0.0, 100.0)
        } else {
            50.0
        };
        let (width, height) = (source_size.0.max(1) as f32, source_size.1.max(1) as f32);
        let diagonal = width.hypot(height);
        Some(Self {
            stops: amount.clamp(-100.0, 100.0) / 100.0 * CORNER_STOPS,
            power: (1.0 + 2.0 * midpoint / 100.0).exp2(),
            scale: (2.0 * width / diagonal, 2.0 * height / diagonal),
        })
    }

    /// The gain at normalized source position `(u, v)` (0 to 1 across the
    /// source's width and height).
    #[inline]
    pub(crate) fn gain(&self, u: f32, v: f32) -> f32 {
        let dx = (u - 0.5) * self.scale.0;
        let dy = (v - 0.5) * self.scale.1;
        let r2 = (dx * dx + dy * dy).min(4.0);
        (self.stops * r2.powf(self.power / 2.0)).exp2()
    }

    /// `rgb` times the gain at `(u, v)`.
    #[inline]
    pub(crate) fn apply(&self, rgb: [f32; 3], u: f32, v: f32) -> [f32; 3] {
        let gain = self.gain(u, v);
        [rgb[0] * gain, rgb[1] * gain, rgb[2] * gain]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn positive_vignetting_brightens_the_corners_and_leaves_the_centre() {
        let lens = LensCorrections {
            vignetting: 100.0,
            ..Default::default()
        };
        let correction = LensVignetting::new(&lens, (600, 400)).unwrap();
        assert_eq!(correction.gain(0.5, 0.5), 1.0);
        assert!((correction.gain(0.0, 0.0) - 2.0).abs() < 1e-5);
        assert!(correction.gain(0.75, 0.5) < correction.gain(1.0, 0.5));
        assert!(LensVignetting::new(&LensCorrections::default(), (600, 400)).is_none());
    }
}
