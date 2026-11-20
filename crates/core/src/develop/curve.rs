//! The Tone Curve panel: a parametric curve plus point curves.

use serde::{Deserialize, Serialize};

/// Region-based curve: four sliders (-100 to 100) acting on tonal regions
/// whose boundaries are the three splits (0 to 100, strictly increasing).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ParametricCurve {
    pub highlights: f32,
    pub lights: f32,
    pub darks: f32,
    pub shadows: f32,
    pub split_shadows: f32,
    pub split_midtones: f32,
    pub split_highlights: f32,
}

impl Default for ParametricCurve {
    fn default() -> Self {
        Self {
            highlights: 0.0,
            lights: 0.0,
            darks: 0.0,
            shadows: 0.0,
            split_shadows: 25.0,
            split_midtones: 50.0,
            split_highlights: 75.0,
        }
    }
}

impl ParametricCurve {
    /// True when the four region sliders are all zero (the splits alone do
    /// nothing).
    pub fn is_identity(&self) -> bool {
        self.highlights == 0.0 && self.lights == 0.0 && self.darks == 0.0 && self.shadows == 0.0
    }
}

/// A control point of a point curve; both axes run from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct CurvePoint {
    pub x: f32,
    pub y: f32,
}

impl CurvePoint {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }
}

/// A curve through control points, interpolated smoothly by the pipeline.
///
/// Invariant (kept by [`PointCurve::new`]): at least two points, all within
/// 0 to 1, with strictly increasing `x`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(from = "Vec<CurvePoint>", into = "Vec<CurvePoint>")]
pub struct PointCurve {
    points: Vec<CurvePoint>,
}

impl PointCurve {
    /// Minimum horizontal distance between neighbouring points.
    pub const MIN_GAP: f32 = 1.0 / 256.0;

    /// Build a curve from arbitrary points: non-finite points are dropped,
    /// the rest are clamped, sorted, and thinned so neighbours are at least
    /// [`Self::MIN_GAP`] apart. Fewer than two usable points yields the
    /// identity.
    pub fn new(points: impl IntoIterator<Item = CurvePoint>) -> Self {
        let mut cleaned: Vec<CurvePoint> = points
            .into_iter()
            .filter(|p| p.x.is_finite() && p.y.is_finite())
            .map(|p| CurvePoint::new(p.x.clamp(0.0, 1.0), p.y.clamp(0.0, 1.0)))
            .collect();
        cleaned.sort_by(|a, b| a.x.total_cmp(&b.x));
        let mut points: Vec<CurvePoint> = Vec::with_capacity(cleaned.len());
        for point in cleaned {
            match points.last() {
                Some(last) if point.x - last.x < Self::MIN_GAP => {}
                _ => points.push(point),
            }
        }
        if points.len() < 2 {
            return Self::default();
        }
        Self { points }
    }

    pub fn points(&self) -> &[CurvePoint] {
        &self.points
    }

    /// True for the straight line from (0, 0) to (1, 1).
    pub fn is_identity(&self) -> bool {
        self.points.iter().all(|p| p.x == p.y)
    }
}

impl Default for PointCurve {
    fn default() -> Self {
        Self {
            points: vec![CurvePoint::new(0.0, 0.0), CurvePoint::new(1.0, 1.0)],
        }
    }
}

impl From<Vec<CurvePoint>> for PointCurve {
    fn from(points: Vec<CurvePoint>) -> Self {
        Self::new(points)
    }
}

impl From<PointCurve> for Vec<CurvePoint> {
    fn from(curve: PointCurve) -> Self {
        curve.points
    }
}

/// The Tone Curve panel. The `rgb` point curve applies to all channels; the
/// per-channel curves apply after it.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ToneCurve {
    pub parametric: ParametricCurve,
    pub rgb: PointCurve,
    pub red: PointCurve,
    pub green: PointCurve,
    pub blue: PointCurve,
}

impl ToneCurve {
    pub fn is_identity(&self) -> bool {
        self.parametric.is_identity()
            && self.rgb.is_identity()
            && self.red.is_identity()
            && self.green.is_identity()
            && self.blue.is_identity()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_point_curve_is_the_identity() {
        assert!(PointCurve::default().is_identity());
        assert!(ToneCurve::default().is_identity());
    }

    #[test]
    fn new_sorts_clamps_and_thins_points() {
        let curve = PointCurve::new([
            CurvePoint::new(1.5, 2.0),
            CurvePoint::new(0.5, 0.7),
            CurvePoint::new(0.5001, 0.1),
            CurvePoint::new(-1.0, -1.0),
            CurvePoint::new(f32::NAN, 0.5),
        ]);
        assert_eq!(
            curve.points(),
            [
                CurvePoint::new(0.0, 0.0),
                CurvePoint::new(0.5, 0.7),
                CurvePoint::new(1.0, 1.0),
            ]
        );
        assert!(!curve.is_identity());
    }

    #[test]
    fn fewer_than_two_usable_points_falls_back_to_identity() {
        assert_eq!(
            PointCurve::new([CurvePoint::new(0.3, 0.9)]),
            PointCurve::default()
        );
        assert_eq!(PointCurve::new([]), PointCurve::default());
    }

    #[test]
    fn deserializing_repairs_an_invalid_curve() {
        let curve: PointCurve =
            serde_json::from_str(r#"[{"x":0.8,"y":0.2},{"x":0.1,"y":0.9}]"#).unwrap();
        assert_eq!(curve.points()[0], CurvePoint::new(0.1, 0.9));
        let json = serde_json::to_string(&curve).unwrap();
        assert_eq!(serde_json::from_str::<PointCurve>(&json).unwrap(), curve);
    }
}
