//! How a slider spreads its values along the track.

/// A mapping between values and positions along a slider track.
///
/// A position is a fraction of the track: 0 is the left end, 1 the right.
/// Most sliders are [`Scale::LINEAR`]. A range that spans orders of magnitude
/// (colour temperature in kelvin) needs [`Scale::LOG`], or the low end, where
/// a small change matters most, is squeezed into a few pixels.
#[derive(Clone, Copy, Debug)]
pub struct Scale {
    to_position: fn(value: f32, min: f32, max: f32) -> f32,
    to_value: fn(position: f32, min: f32, max: f32) -> f32,
}

impl Scale {
    /// Equal steps of value take equal lengths of track.
    pub const LINEAR: Self = Self {
        to_position: linear_position,
        to_value: linear_value,
    };

    /// Equal ratios of value take equal lengths of track. Needs a range above
    /// zero; any other range falls back to linear.
    pub const LOG: Self = Self {
        to_position: log_position,
        to_value: log_value,
    };

    /// A custom mapping. `to_position(value, min, max)` must rise steadily
    /// from 0 at `min` to 1 at `max`, and `to_value(position, min, max)` must
    /// be its inverse.
    pub const fn new(
        to_position: fn(value: f32, min: f32, max: f32) -> f32,
        to_value: fn(position: f32, min: f32, max: f32) -> f32,
    ) -> Self {
        Self {
            to_position,
            to_value,
        }
    }

    /// Where `value` sits along a track that runs from `min` to `max`,
    /// clamped to 0 to 1.
    pub fn position(self, value: f32, min: f32, max: f32) -> f32 {
        let position = (self.to_position)(value.clamp(min, max), min, max);
        if position.is_finite() {
            position.clamp(0.0, 1.0)
        } else {
            0.0
        }
    }

    /// The value at `position` (0 to 1) along a track that runs from `min`
    /// to `max`, clamped to the range.
    pub fn value(self, position: f32, min: f32, max: f32) -> f32 {
        let value = (self.to_value)(position.clamp(0.0, 1.0), min, max);
        if value.is_finite() {
            value.clamp(min, max)
        } else {
            min
        }
    }
}

fn linear_position(value: f32, min: f32, max: f32) -> f32 {
    (value - min) / (max - min)
}

fn linear_value(position: f32, min: f32, max: f32) -> f32 {
    min + position * (max - min)
}

fn log_position(value: f32, min: f32, max: f32) -> f32 {
    if min <= 0.0 {
        return linear_position(value, min, max);
    }
    (value / min).ln() / (max / min).ln()
}

fn log_value(position: f32, min: f32, max: f32) -> f32 {
    if min <= 0.0 {
        return linear_value(position, min, max);
    }
    min * (max / min).powf(position)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_scales_pin_the_ends_of_the_range() {
        for scale in [Scale::LINEAR, Scale::LOG] {
            assert_eq!(scale.position(2000.0, 2000.0, 50_000.0), 0.0);
            assert!((scale.position(50_000.0, 2000.0, 50_000.0) - 1.0).abs() < 1.0e-6);
            assert!((scale.value(0.0, 2000.0, 50_000.0) - 2000.0).abs() < 0.01);
            assert!((scale.value(1.0, 2000.0, 50_000.0) - 50_000.0).abs() < 1.0);
        }
    }

    #[test]
    fn value_and_position_are_inverses() {
        for scale in [Scale::LINEAR, Scale::LOG] {
            for step in 0..=20 {
                let position = step as f32 / 20.0;
                let value = scale.value(position, 2000.0, 50_000.0);
                let back = scale.position(value, 2000.0, 50_000.0);
                assert!(
                    (back - position).abs() < 1.0e-4,
                    "{scale:?} at {position}: {back}"
                );
            }
        }
    }

    #[test]
    fn positions_rise_with_value() {
        for scale in [Scale::LINEAR, Scale::LOG] {
            let positions: Vec<f32> = (0..=48)
                .map(|step| scale.position(2000.0 + 1000.0 * step as f32, 2000.0, 50_000.0))
                .collect();
            assert!(
                positions.windows(2).all(|pair| pair[0] < pair[1]),
                "{scale:?}"
            );
        }
    }

    #[test]
    fn a_log_scale_gives_the_low_end_room() {
        // Daylight (5500 K) is 7% of the way along a linear kelvin track,
        // which leaves tungsten to daylight almost no travel.
        let linear = Scale::LINEAR.position(5500.0, 2000.0, 50_000.0);
        let log = Scale::LOG.position(5500.0, 2000.0, 50_000.0);
        assert!(linear < 0.08);
        assert!(log > 0.3);
    }

    #[test]
    fn a_log_scale_over_a_signed_range_falls_back_to_linear() {
        assert_eq!(Scale::LOG.position(0.0, -100.0, 100.0), 0.5);
        assert_eq!(Scale::LOG.value(0.75, -100.0, 100.0), 50.0);
    }

    #[test]
    fn out_of_range_and_non_finite_input_is_tamed() {
        assert_eq!(Scale::LINEAR.position(900.0, -100.0, 100.0), 1.0);
        assert_eq!(Scale::LINEAR.position(f32::NAN, -100.0, 100.0), 0.0);
        assert_eq!(Scale::LINEAR.value(7.0, -100.0, 100.0), 100.0);
        assert_eq!(Scale::LINEAR.value(f32::NAN, -100.0, 100.0), -100.0);
    }

    #[test]
    fn a_custom_scale_is_used_as_given() {
        fn squared(value: f32, min: f32, max: f32) -> f32 {
            ((value - min) / (max - min)).sqrt()
        }
        fn square(position: f32, min: f32, max: f32) -> f32 {
            min + position * position * (max - min)
        }
        let scale = Scale::new(squared, square);
        assert_eq!(scale.position(25.0, 0.0, 100.0), 0.5);
        assert_eq!(scale.value(0.5, 0.0, 100.0), 25.0);
    }
}
