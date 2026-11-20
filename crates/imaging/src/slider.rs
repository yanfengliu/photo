//! Reading settings defensively.
//!
//! Settings reach this crate from sidecar files, presets and pastes as well
//! as from the sliders that produced them. A value outside its range is
//! clamped rather than trusted, and a NaN is never allowed into a buffer,
//! where the next spatial filter would spread it across the picture.

/// A 0 to 100 setting as a fraction, `0.0..=1.0`. Values outside the range
/// are clamped; a NaN counts as 0.
pub(crate) fn fraction(percent: f32) -> f64 {
    if percent.is_nan() {
        0.0
    } else {
        f64::from(percent.clamp(0.0, 100.0)) / 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fractions_are_clamped_and_never_nan() {
        assert_eq!(fraction(0.0), 0.0);
        assert_eq!(fraction(25.0), 0.25);
        assert_eq!(fraction(100.0), 1.0);
        assert_eq!(fraction(250.0), 1.0);
        assert_eq!(fraction(-3.0), 0.0);
        assert_eq!(fraction(f32::INFINITY), 1.0);
        assert_eq!(fraction(f32::NEG_INFINITY), 0.0);
        assert_eq!(fraction(f32::NAN), 0.0);
    }
}
