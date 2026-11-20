//! How numbers and switches are spelled in a sidecar.

use std::fmt::Display;
use std::str::FromStr;

/// A floating-point type a sidecar number is read into.
pub(crate) trait Number: Copy + Default + Display + FromStr + PartialOrd {
    fn is_finite_number(self) -> bool;
}

impl Number for f32 {
    fn is_finite_number(self) -> bool {
        self.is_finite()
    }
}

impl Number for f64 {
    fn is_finite_number(self) -> bool {
        self.is_finite()
    }
}

/// Reads a decimal number, with or without a leading `+`. Anything that is
/// not a finite number is `None`.
pub(crate) fn parse_number<T: Number>(text: &str) -> Option<T> {
    text.trim()
        .parse()
        .ok()
        .filter(|value: &T| value.is_finite_number())
}

/// Writes a number in the shortest form that reads back as exactly the same
/// value. Positive values of a `signed` quantity get a leading `+`, the way
/// other applications write them. A value that is not finite has no form.
pub(crate) fn format_number<T: Number>(value: T, signed: bool) -> Option<String> {
    let zero = T::default();
    if !value.is_finite_number() {
        None
    } else if value == zero {
        Some("0".to_owned())
    } else if signed && value > zero {
        Some(format!("+{value}"))
    } else {
        Some(value.to_string())
    }
}

/// Reads `True` or `False` in any capitalisation, or `1` or `0`.
pub(crate) fn parse_bool(text: &str) -> Option<bool> {
    let text = text.trim();
    if text.eq_ignore_ascii_case("true") || text == "1" {
        Some(true)
    } else if text.eq_ignore_ascii_case("false") || text == "0" {
        Some(false)
    } else {
        None
    }
}

/// Writes a switch the way Camera Raw properties spell it.
pub(crate) const fn format_bool(value: bool) -> &'static str {
    if value { "True" } else { "False" }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn numbers_take_an_optional_plus_and_surrounding_space() {
        assert_eq!(parse_number::<f32>("+0.35"), Some(0.35));
        assert_eq!(parse_number::<f32>(" -12 "), Some(-12.0));
        assert_eq!(parse_number::<f32>("5350"), Some(5350.0));
        assert_eq!(parse_number::<f64>("0.031746"), Some(0.031746));
        for text in [
            "",
            "+",
            "high",
            "1,5",
            "12px",
            "NaN",
            "inf",
            "-infinity",
            "1e999",
        ] {
            assert_eq!(parse_number::<f32>(text), None, "{text:?}");
        }
    }

    #[test]
    fn numbers_are_written_short_signed_and_exact() {
        assert_eq!(format_number(0.35_f32, true).as_deref(), Some("+0.35"));
        assert_eq!(format_number(-12.0_f32, true).as_deref(), Some("-12"));
        assert_eq!(format_number(40.0_f32, false).as_deref(), Some("40"));
        assert_eq!(format_number(40.0_f32, true).as_deref(), Some("+40"));
        assert_eq!(format_number(0.0_f32, true).as_deref(), Some("0"));
        assert_eq!(format_number(-0.0_f32, true).as_deref(), Some("0"));
        assert_eq!(format_number(0.5_f64, false).as_deref(), Some("0.5"));
        assert_eq!(format_number(f32::NAN, true), None);
        assert_eq!(format_number(f64::INFINITY, false), None);

        let mut state = 0x9e37_79b9_u32;
        for _ in 0..10_000 {
            state = state.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            let value = f32::from_bits(state);
            if value.is_finite() {
                let text = format_number(value, state.is_multiple_of(2)).unwrap();
                assert_eq!(parse_number::<f32>(&text), Some(value), "{text}");
            }
        }
    }

    #[test]
    fn switches_are_read_leniently_and_written_capitalised() {
        assert_eq!(parse_bool("True"), Some(true));
        assert_eq!(parse_bool(" false "), Some(false));
        assert_eq!(parse_bool("1"), Some(true));
        assert_eq!(parse_bool("0"), Some(false));
        assert_eq!(parse_bool("yes"), None);
        assert_eq!((format_bool(true), format_bool(false)), ("True", "False"));
    }
}
