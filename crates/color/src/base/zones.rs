//! The scene-linear gain that realises Highlights and Shadows in front of
//! the base curve: `k(b) = c⁻¹(decode(Z(b))) / c⁻¹(decode(b))`, with `c` the
//! scalar scene-to-display curve, `Z` the perceptual zone curve and `b` the
//! smoothed base of a pixel.

use std::sync::LazyLock;

use crate::lut::Lut;
use crate::tone::{ZoneCurve, decode_d};

use super::curve::{CURVES, shoulder_inverse, soft_clip_inverse};

// ---- Tunable constants ----------------------------------------------------

/// Samples of the zone-gain tables. They are indexed by
/// `1 - (1 - base)^(1/4)`, which spends a quarter of the samples on the last
/// 0.3% below white, where a base curve packs the headroom.
const ZONE_CELLS: usize = 2048;
/// Bases closer to white than this get the gain of this base. In `f32` a
/// base this close to white still resolves to 0.6%, but one much closer
/// cannot be told from white, and the gain there must not change faster
/// than the base can say. On the standard profile it is the base of a scene
/// 2.5 EV above the sensor's clip point.
const BASE_LIMIT: f64 = 1.0 - 1.0e-5;
/// Bounds on the zone gain (six stops either way), far beyond what the zone
/// curve asks for; they only keep a degenerate base from producing an
/// absurd multiplier.
const ZONE_GAIN_RANGE: (f64, f64) = (1.0 / 64.0, 64.0);
/// A masked pixel with its own Highlights or Shadows reads its zone gain
/// from [`LOG_SCENE`] when both its base and its target are at least this;
/// darker ones, where the inverse is a power law that a table over the base
/// resolves badly, evaluate it directly.
const LOG_TABLE_START: f64 = 0.05;

// ---------------------------------------------------------------------------

/// Which scalar curve follows the exposure gain.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Rendering {
    /// A raster source: gain and shoulder.
    Raster,
    /// A camera source with the curved profile at this index of [`CURVES`].
    Curve(usize),
    /// A camera source with the linear profile: gain and soft clip.
    Linear,
}

impl Rendering {
    /// The already-gained scene value that this rendering maps to display
    /// value `display`. `gain` only matters for a raster source, whose
    /// shoulder depends on it.
    fn inverse(self, gain: f64, display: f64) -> f64 {
        match self {
            Self::Raster if gain > 1.0 => shoulder_inverse(gain, display),
            Self::Raster => display.max(0.0),
            Self::Curve(index) => CURVES[index].exact.inverse(display),
            Self::Linear => soft_clip_inverse(display),
        }
    }
}

/// The scene-linear gain that moves a flat area whose perceptual display
/// value is `base` to where `zone` puts it. One wherever the zone curve is
/// the identity, and for a base that is not a positive number. The exposure
/// gain cancels out of the ratio except through a raster shoulder.
pub(super) fn zone_gain(rendering: Rendering, gain: f64, zone: &ZoneCurve, base: f64) -> f64 {
    if base.is_nan() || base <= 0.0 {
        return 1.0;
    }
    let base = base.min(BASE_LIMIT);
    let target = zone.eval_d(base);
    if target == base {
        return 1.0;
    }
    let from = rendering.inverse(gain, decode_d(base));
    let to = rendering.inverse(gain, decode_d(target));
    if from > 0.0 && from.is_finite() && to.is_finite() {
        (to / from).clamp(ZONE_GAIN_RANGE.0, ZONE_GAIN_RANGE.1)
    } else {
        1.0
    }
}

/// Position of a base in a zone-gain table, 0 to 1 (NaN reads as black).
#[inline(always)]
fn zone_index(base: f32) -> f32 {
    let distance = (1.0 - base.max(0.0)).max(0.0);
    1.0 - distance.sqrt().sqrt()
}

/// The base at a position in a zone-gain table (inverse of [`zone_index`]).
fn zone_base(index: f64) -> f64 {
    1.0 - (1.0 - index).powi(4)
}

/// For each curved profile, the natural log of the scene value (before the
/// exposure gain, which cancels out) that lands on each base, over
/// [`zone_index`]: the zone gain of any zone curve is then the exponential
/// of a difference of two lookups. Built once per process, the first time a
/// masked pixel has its own Highlights or Shadows.
static LOG_SCENE: LazyLock<[Lut; 3]> = LazyLock::new(|| {
    [0, 1, 2].map(|curve| {
        Lut::build(ZONE_CELLS, 1.0, |index| {
            let base = zone_base(index).clamp(0.5 * LOG_TABLE_START, BASE_LIMIT);
            CURVES[curve].exact.inverse(decode_d(base)).ln()
        })
    })
});

/// The zone gain of a zone curve that is not the global one (a masked
/// pixel's). For a curved profile it comes from [`LOG_SCENE`]; otherwise,
/// and in the deep shadows, it is evaluated directly.
pub(super) fn local_zone_gain(
    rendering: Rendering,
    gain: f32,
    curve: &ZoneCurve,
    base: f32,
) -> f32 {
    let base = f64::from(base);
    if let Rendering::Curve(index) = rendering
        && base >= LOG_TABLE_START
    {
        let base = base.min(BASE_LIMIT);
        let target = curve.eval_d(base);
        if target >= LOG_TABLE_START {
            let table = &LOG_SCENE[index];
            let log = |b: f64| table.eval(zone_index(b as f32));
            let ratio = f64::from(log(target) - log(base)).exp();
            return ratio.clamp(ZONE_GAIN_RANGE.0, ZONE_GAIN_RANGE.1) as f32;
        }
    }
    zone_gain(rendering, f64::from(gain), curve, base) as f32
}

/// The global zone curve and its gain tabulated over [`zone_index`].
#[derive(Clone, Debug)]
pub(super) struct Zones {
    pub(super) curve: ZoneCurve,
    gain: Lut,
}

impl Zones {
    /// Tabulate `curve`'s gain for a rendering at the global exposure gain;
    /// `None` when the curve is the identity.
    pub(super) fn new(rendering: Rendering, gain: f32, curve: ZoneCurve) -> Option<Self> {
        (!curve.is_identity()).then(|| Self {
            curve,
            gain: Lut::build(ZONE_CELLS, 1.0, |index| {
                zone_gain(rendering, f64::from(gain), &curve, zone_base(index))
            }),
        })
    }

    /// The tabulated gain for a pixel's base.
    #[inline(always)]
    pub(super) fn at(&self, base: f32) -> f32 {
        self.gain.eval(zone_index(base))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use photo_core::meta::FileKind;

    const RENDERINGS: [(Rendering, f64, FileKind); 6] = [
        (Rendering::Raster, 1.0, FileKind::Raster),
        (Rendering::Raster, 0.5, FileKind::Raster),
        (Rendering::Raster, 4.0, FileKind::Raster),
        (Rendering::Curve(0), 1.0, FileKind::Raw),
        (Rendering::Curve(2), 1.0, FileKind::Raw),
        (Rendering::Linear, 1.0, FileKind::Raw),
    ];

    fn probe_bases() -> Vec<f64> {
        let mut bases: Vec<f64> = (1..1000).map(|step| f64::from(step) / 1000.0).collect();
        bases.extend((3..=50).map(|tenths| 1.0 - 10.0f64.powf(-f64::from(tenths) / 10.0)));
        bases
    }

    #[test]
    fn the_zone_gain_puts_a_flat_area_where_the_zone_curve_says() {
        for (rendering, gain, kind) in RENDERINGS {
            for (highlights, shadows) in [(-100.0, 100.0), (100.0, -100.0)] {
                let zone = ZoneCurve::new(highlights, shadows, kind);
                for step in 1..1000 {
                    let base = f64::from(step) / 1000.0;
                    let from = rendering.inverse(gain, decode_d(base));
                    let k = zone_gain(rendering, gain, &zone, base);
                    // Rendering the scaled scene value gives decode(Z(base));
                    // checked through the inverse, which is exact.
                    let landed = rendering.inverse(gain, decode_d(zone.eval_d(base)));
                    assert!(
                        (k * from / landed - 1.0).abs() < 1e-9,
                        "{rendering:?} gain {gain} at base {base}"
                    );
                }
                // Outside (0, 1) the zone curve is the identity; at and
                // beyond the limit the gain is that of the limit.
                assert_eq!(zone_gain(rendering, gain, &zone, 0.0), 1.0);
                assert_eq!(zone_gain(rendering, gain, &zone, f64::NAN), 1.0);
                let limit = zone_gain(rendering, gain, &zone, BASE_LIMIT);
                assert_eq!(zone_gain(rendering, gain, &zone, 1.0), limit);
                assert_eq!(zone_gain(rendering, gain, &zone, 7.0), limit);
                assert!(limit > 0.0 && limit.is_finite());
            }
        }
    }

    #[test]
    fn the_zone_table_tracks_the_exact_gain_up_to_the_limit() {
        for (rendering, gain, kind) in RENDERINGS {
            let zone = ZoneCurve::new(-100.0, 100.0, kind);
            let zones = Zones::new(rendering, gain as f32, zone).unwrap();
            for base in probe_bases() {
                let exact = zone_gain(rendering, gain, &zone, base);
                let read = f64::from(zones.at(base as f32));
                assert!(
                    (read / exact - 1.0).abs() < 2e-3,
                    "{rendering:?} gain {gain} at base {base}: {read} vs {exact}"
                );
            }
        }
        assert_eq!(zone_index(f32::NAN), 0.0);
        assert_eq!(zone_index(-1.0), 0.0);
        assert_eq!(zone_index(2.0), 1.0);
        let identity = ZoneCurve::new(0.0, 0.0, FileKind::Raw);
        assert!(Zones::new(Rendering::Curve(0), 1.0, identity).is_none());
    }

    #[test]
    fn a_masked_zone_gain_matches_the_exact_one() {
        for rendering in [
            Rendering::Curve(0),
            Rendering::Curve(1),
            Rendering::Curve(2),
        ] {
            for (highlights, shadows) in [(-100.0, 100.0), (60.0, -40.0), (-30.0, 0.0)] {
                let zone = ZoneCurve::new(highlights, shadows, FileKind::Raw);
                for base in probe_bases() {
                    let exact = zone_gain(rendering, 1.0, &zone, base);
                    // The exposure gain cancels out for a curved profile.
                    let read = f64::from(local_zone_gain(rendering, 3.7, &zone, base as f32));
                    assert!(
                        (read / exact - 1.0).abs() < 2e-3,
                        "{rendering:?} {highlights}/{shadows} at {base}: {read} vs {exact}"
                    );
                }
            }
        }
    }
}
