//! A small civil date-time, enough for capture times, folder-by-date import
//! and file-naming templates without a date dependency.
//!
//! Capture times are kept as the camera's wall clock ("naive local"): that is
//! what EXIF records and what a photographer means by "when I took this".

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

/// A calendar date and wall-clock time with one-second resolution.
///
/// Orders chronologically, and its ISO 8601 text form (`2026-11-19T20:44:05`)
/// sorts the same way, so it can be stored and compared as text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DateTime {
    pub year: i32,
    pub month: u8,
    pub day: u8,
    pub hour: u8,
    pub minute: u8,
    pub second: u8,
}

impl DateTime {
    /// Build a date-time, rejecting impossible calendar values.
    pub fn new(year: i32, month: u8, day: u8, hour: u8, minute: u8, second: u8) -> Option<Self> {
        let valid = (1..=12).contains(&month)
            && day >= 1
            && day <= days_in_month(year, month)
            && hour < 24
            && minute < 60
            && second < 60
            && (0..=9999).contains(&year);
        valid.then_some(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }

    /// The UTC date-time for a count of seconds since 1970-01-01T00:00:00Z.
    pub fn from_unix_seconds(seconds: i64) -> Option<Self> {
        let days = seconds.div_euclid(86_400);
        let rem = seconds.rem_euclid(86_400);
        let (year, month, day) = civil_from_days(days);
        Self::new(
            i32::try_from(year).ok()?,
            month,
            day,
            (rem / 3600) as u8,
            (rem % 3600 / 60) as u8,
            (rem % 60) as u8,
        )
    }

    /// Seconds since the Unix epoch, reading this value as UTC.
    pub fn to_unix_seconds(&self) -> i64 {
        days_from_civil(i64::from(self.year), self.month, self.day) * 86_400
            + i64::from(self.hour) * 3600
            + i64::from(self.minute) * 60
            + i64::from(self.second)
    }

    /// Parse the EXIF form `YYYY:MM:DD HH:MM:SS`. Also accepts `-` date
    /// separators and a `T` between date and time, which some writers emit.
    pub fn parse_exif(text: &str) -> Option<Self> {
        parse(text.trim().trim_end_matches('\0'))
    }

    /// `YYYY-MM-DD`.
    pub fn date_string(&self) -> String {
        format!("{:04}-{:02}-{:02}", self.year, self.month, self.day)
    }
}

impl fmt::Display for DateTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
            self.year, self.month, self.day, self.hour, self.minute, self.second
        )
    }
}

/// The text was not a recognisable date-time.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ParseDateTimeError;

impl fmt::Display for ParseDateTimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("not a valid date-time")
    }
}

impl std::error::Error for ParseDateTimeError {}

impl FromStr for DateTime {
    type Err = ParseDateTimeError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        parse(text.trim()).ok_or(ParseDateTimeError)
    }
}

impl Serialize for DateTime {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for DateTime {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

/// Accepts `YYYY<sep>MM<sep>DD<gap>HH:MM:SS` with `-` or `:` date separators,
/// a space or `T` gap, and ignores anything after the seconds (fractions,
/// zone suffixes).
fn parse(text: &str) -> Option<DateTime> {
    let bytes = text.as_bytes();
    if bytes.len() < 19 || !text.is_char_boundary(19) {
        return None;
    }
    let number = |range: std::ops::Range<usize>| -> Option<u32> {
        let part = text.get(range)?;
        part.bytes().all(|b| b.is_ascii_digit()).then(|| part.parse().ok())?
    };
    let date_separator = |index: usize| matches!(bytes[index], b'-' | b':');
    if !date_separator(4) || !date_separator(7) || !matches!(bytes[10], b' ' | b'T') {
        return None;
    }
    if bytes[13] != b':' || bytes[16] != b':' {
        return None;
    }
    DateTime::new(
        number(0..4)? as i32,
        number(5..7)? as u8,
        number(8..10)? as u8,
        number(11..13)? as u8,
        number(14..16)? as u8,
        number(17..19)? as u8,
    )
}

fn is_leap_year(year: i32) -> bool {
    (year % 4 == 0 && year % 100 != 0) || year % 400 == 0
}

fn days_in_month(year: i32, month: u8) -> u8 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// `days_from_civil`).
fn days_from_civil(year: i64, month: u8, day: u8) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year.rem_euclid(400);
    let shifted_month = (i64::from(month) + 9) % 12;
    let day_of_year = (153 * shifted_month + 2) / 5 + i64::from(day) - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Inverse of [`days_from_civil`].
fn civil_from_days(days: i64) -> (i64, u8, u8) {
    let shifted = days + 719_468;
    let era = shifted.div_euclid(146_097);
    let day_of_era = shifted.rem_euclid(146_097);
    let year_of_era = (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted_month = (5 * day_of_year + 2) / 153;
    let day = (day_of_year - (153 * shifted_month + 2) / 5 + 1) as u8;
    let month = if shifted_month < 10 {
        shifted_month + 3
    } else {
        shifted_month - 9
    } as u8;
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_the_exif_form() {
        let parsed = DateTime::parse_exif("2024:02:29 13:05:59").unwrap();
        assert_eq!(parsed, DateTime::new(2024, 2, 29, 13, 5, 59).unwrap());
    }

    #[test]
    fn parses_iso_text_and_ignores_fractions_and_zone_suffixes() {
        let expected = DateTime::new(2026, 11, 19, 20, 44, 5).unwrap();
        assert_eq!("2026-11-19T20:44:05".parse(), Ok(expected));
        assert_eq!("2026-11-19T20:44:05.250+08:00".parse(), Ok(expected));
    }

    #[test]
    fn rejects_impossible_and_malformed_values() {
        assert_eq!(DateTime::parse_exif("2023:02:29 00:00:00"), None);
        assert_eq!(DateTime::parse_exif("2023:13:01 00:00:00"), None);
        assert_eq!(DateTime::parse_exif("2023:01:01 24:00:00"), None);
        assert_eq!(DateTime::parse_exif("0000:00:00 00:00:00"), None);
        assert_eq!(DateTime::parse_exif("    :  :     :  :  "), None);
        assert_eq!(DateTime::parse_exif("not a date"), None);
        assert_eq!(DateTime::parse_exif(""), None);
    }

    #[test]
    fn display_round_trips_through_parse() {
        let value = DateTime::new(1999, 12, 31, 23, 59, 59).unwrap();
        assert_eq!(value.to_string(), "1999-12-31T23:59:59");
        assert_eq!(value.to_string().parse(), Ok(value));
    }

    #[test]
    fn text_order_matches_chronological_order() {
        let earlier = DateTime::new(2025, 9, 30, 23, 59, 59).unwrap();
        let later = DateTime::new(2025, 10, 1, 0, 0, 0).unwrap();
        assert!(earlier < later);
        assert!(earlier.to_string() < later.to_string());
    }

    #[test]
    fn unix_seconds_round_trip_across_eras() {
        assert_eq!(
            DateTime::from_unix_seconds(0),
            DateTime::new(1970, 1, 1, 0, 0, 0)
        );
        assert_eq!(
            DateTime::from_unix_seconds(951_782_400),
            DateTime::new(2000, 2, 29, 0, 0, 0)
        );
        assert_eq!(
            DateTime::from_unix_seconds(-1),
            DateTime::new(1969, 12, 31, 23, 59, 59)
        );
        for seconds in [0i64, 86_399, 1_763_585_045, 4_102_444_800, -86_400 * 365] {
            let value = DateTime::from_unix_seconds(seconds).unwrap();
            assert_eq!(value.to_unix_seconds(), seconds);
        }
    }

    #[test]
    fn serializes_as_iso_text() {
        let value = DateTime::new(2026, 1, 2, 3, 4, 5).unwrap();
        let json = serde_json::to_string(&value).unwrap();
        assert_eq!(json, "\"2026-01-02T03:04:05\"");
        assert_eq!(serde_json::from_str::<DateTime>(&json).unwrap(), value);
    }
}
