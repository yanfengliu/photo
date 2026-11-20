//! The table of mask-local adjustments: the counterpart of [`super::Adj`] for
//! the sliders a mask carries.

use super::adjust::AdjSpec;
use super::groups::SettingsGroup;
use super::local::LocalAdjustments;

macro_rules! local_adjustments {
    ($(
        $variant:ident: $key:literal, $label:literal, $min:literal ..= $max:literal,
        step $step:literal, decimals $decimals:literal, $field:ident;
    )*) => {
        /// One scalar adjustment of a mask.
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
        pub enum LocalAdj {
            $($variant,)*
        }

        impl LocalAdj {
            /// Every local adjustment, in the order a mask panel shows them.
            pub const ALL: &'static [LocalAdj] = &[$(LocalAdj::$variant,)*];

            /// Key, label, range and step. Keys are prefixed `local_` so they
            /// never collide with a global adjustment's key.
            pub fn spec(self) -> AdjSpec {
                match self {
                    $(LocalAdj::$variant => AdjSpec {
                        key: $key,
                        label: $label,
                        min: $min,
                        max: $max,
                        step: $step,
                        decimals: $decimals,
                        group: SettingsGroup::Masking,
                    },)*
                }
            }

            /// Read this adjustment's value.
            pub fn get(self, adjust: &LocalAdjustments) -> f32 {
                match self {
                    $(LocalAdj::$variant => adjust.$field,)*
                }
            }

            /// Write this adjustment's value as given; callers clamp with
            /// [`AdjSpec::clamp`].
            pub fn set(self, adjust: &mut LocalAdjustments, value: f32) {
                match self {
                    $(LocalAdj::$variant => adjust.$field = value,)*
                }
            }
        }
    };
}

local_adjustments! {
    Exposure: "local_exposure", "Exposure", -4.0..=4.0, step 0.01, decimals 2, exposure;
    Contrast: "local_contrast", "Contrast", -100.0..=100.0, step 1.0, decimals 0, contrast;
    Highlights: "local_highlights", "Highlights", -100.0..=100.0, step 1.0, decimals 0, highlights;
    Shadows: "local_shadows", "Shadows", -100.0..=100.0, step 1.0, decimals 0, shadows;
    Whites: "local_whites", "Whites", -100.0..=100.0, step 1.0, decimals 0, whites;
    Blacks: "local_blacks", "Blacks", -100.0..=100.0, step 1.0, decimals 0, blacks;
    Temperature: "local_temperature", "Temp", -100.0..=100.0, step 1.0, decimals 0, temp;
    Tint: "local_tint", "Tint", -100.0..=100.0, step 1.0, decimals 0, tint;
    Saturation: "local_saturation", "Saturation", -100.0..=100.0, step 1.0, decimals 0, saturation;
    Texture: "local_texture", "Texture", -100.0..=100.0, step 1.0, decimals 0, texture;
    Clarity: "local_clarity", "Clarity", -100.0..=100.0, step 1.0, decimals 0, clarity;
    Dehaze: "local_dehaze", "Dehaze", -100.0..=100.0, step 1.0, decimals 0, dehaze;
    Sharpness: "local_sharpness", "Sharpness", -100.0..=100.0, step 1.0, decimals 0, sharpness;
    Noise: "local_noise", "Noise", -100.0..=100.0, step 1.0, decimals 0, noise;
}

impl LocalAdj {
    /// Look a local adjustment up by its [`AdjSpec::key`].
    pub fn from_key(key: &str) -> Option<LocalAdj> {
        Self::ALL.iter().copied().find(|adj| adj.spec().key == key)
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use super::*;
    use crate::develop::Adj;

    #[test]
    fn keys_are_unique_and_distinct_from_the_global_adjustments() {
        let global: HashSet<&str> = Adj::all().map(|adj| adj.spec().key).collect();
        let mut seen = HashSet::new();
        for adj in LocalAdj::ALL {
            let key = adj.spec().key;
            assert!(seen.insert(key), "duplicate key {key}");
            assert!(
                !global.contains(key),
                "{key} collides with a global adjustment"
            );
            assert_eq!(LocalAdj::from_key(key), Some(*adj));
        }
        assert_eq!(LocalAdj::from_key("exposure"), None);
    }

    #[test]
    fn set_then_get_round_trips_without_touching_other_adjustments() {
        for adj in LocalAdj::ALL {
            let mut adjust = LocalAdjustments::default();
            let spec = adj.spec();
            assert!((spec.min..=spec.max).contains(&adj.get(&adjust)), "{adj:?}");
            let value = spec.min + (spec.max - spec.min) * 0.75;
            adj.set(&mut adjust, value);
            for other in LocalAdj::ALL {
                let expected = if other == adj { value } else { 0.0 };
                assert_eq!(other.get(&adjust), expected, "{adj:?} affected {other:?}");
            }
        }
    }

    #[test]
    fn the_table_covers_every_field_of_local_adjustments() {
        let mut adjust = LocalAdjustments::default();
        for adj in LocalAdj::ALL {
            adj.set(&mut adjust, 1.0);
        }
        let json = serde_json::to_value(adjust).unwrap();
        let fields = json.as_object().unwrap();
        assert_eq!(fields.len(), LocalAdj::ALL.len());
        assert!(fields.values().all(|value| value.as_f64() == Some(1.0)));
    }
}
