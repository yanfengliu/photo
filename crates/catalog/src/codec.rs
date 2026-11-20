//! How photo-core values are represented in SQLite columns.
//!
//! Every mapping is spelled out, never an `as` cast of an enum, so reordering
//! an enum in photo-core cannot silently change what a stored number means.

use photo_core::meta::{ColorLabel, FileKind, Flag, Rating};
use photo_core::time::DateTime;
use rusqlite::types::Type;

pub(crate) fn kind_to_sql(kind: FileKind) -> i64 {
    match kind {
        FileKind::Raster => 0,
        FileKind::Raw => 1,
    }
}

pub(crate) fn kind_from_sql(value: i64) -> FileKind {
    if value == 1 {
        FileKind::Raw
    } else {
        FileKind::Raster
    }
}

pub(crate) fn label_to_sql(label: ColorLabel) -> i64 {
    match label {
        ColorLabel::None => 0,
        ColorLabel::Red => 1,
        ColorLabel::Yellow => 2,
        ColorLabel::Green => 3,
        ColorLabel::Blue => 4,
        ColorLabel::Purple => 5,
    }
}

pub(crate) fn label_from_sql(value: i64) -> ColorLabel {
    match value {
        1 => ColorLabel::Red,
        2 => ColorLabel::Yellow,
        3 => ColorLabel::Green,
        4 => ColorLabel::Blue,
        5 => ColorLabel::Purple,
        _ => ColorLabel::None,
    }
}

pub(crate) fn flag_to_sql(flag: Flag) -> i64 {
    i64::from(flag.to_i8())
}

pub(crate) fn flag_from_sql(value: i64) -> Flag {
    match value {
        v if v < 0 => Flag::Rejected,
        0 => Flag::Unflagged,
        _ => Flag::Picked,
    }
}

pub(crate) fn rating_to_sql(rating: Rating) -> i64 {
    i64::from(rating.stars())
}

pub(crate) fn rating_from_sql(value: i64) -> Rating {
    Rating::new(u8::try_from(value.clamp(0, i64::from(Rating::MAX))).unwrap_or(0))
}

/// Unsigned counters and sizes are stored as SQLite's signed 64-bit integer.
pub(crate) fn to_i64(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}

pub(crate) fn to_u64(value: i64) -> u64 {
    u64::try_from(value).unwrap_or(0)
}

pub(crate) fn to_u32(value: i64) -> u32 {
    u32::try_from(value.max(0)).unwrap_or(u32::MAX)
}

/// Capture times are stored as ISO 8601 text, which sorts chronologically.
pub(crate) fn date_from_sql(
    column: usize,
    text: Option<String>,
) -> rusqlite::Result<Option<DateTime>> {
    text.map(|text| {
        text.parse::<DateTime>().map_err(|error| {
            rusqlite::Error::FromSqlConversionFailure(column, Type::Text, Box::new(error))
        })
    })
    .transpose()
}

/// Catalog time stamps are microseconds; the public API speaks Unix seconds.
pub(crate) fn unix_seconds(micros: i64) -> i64 {
    micros.div_euclid(1_000_000)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_enum_value_survives_its_column_form() {
        for kind in [FileKind::Raster, FileKind::Raw] {
            assert_eq!(kind_from_sql(kind_to_sql(kind)), kind);
        }
        for label in ColorLabel::ALL {
            assert_eq!(label_from_sql(label_to_sql(label)), label);
        }
        for flag in [Flag::Rejected, Flag::Unflagged, Flag::Picked] {
            assert_eq!(flag_from_sql(flag_to_sql(flag)), flag);
        }
        for stars in 0..=Rating::MAX {
            assert_eq!(
                rating_from_sql(rating_to_sql(Rating::new(stars))),
                Rating::new(stars)
            );
        }
    }

    #[test]
    fn the_stored_numbers_are_pinned() {
        // These numbers are in catalogs on disk; changing one is a migration.
        assert_eq!(
            (kind_to_sql(FileKind::Raster), kind_to_sql(FileKind::Raw)),
            (0, 1)
        );
        assert_eq!(ColorLabel::ALL.map(label_to_sql), [0, 1, 2, 3, 4, 5]);
        assert_eq!(
            [Flag::Rejected, Flag::Unflagged, Flag::Picked].map(flag_to_sql),
            [-1, 0, 1]
        );
    }

    #[test]
    fn out_of_range_numbers_saturate_instead_of_wrapping() {
        assert_eq!(to_i64(u64::MAX), i64::MAX);
        assert_eq!(to_u64(-5), 0);
        assert_eq!(to_u32(-5), 0);
        assert_eq!(to_u32(i64::MAX), u32::MAX);
        assert_eq!(unix_seconds(-1), -1);
        assert_eq!(unix_seconds(1_999_999), 1);
    }
}
