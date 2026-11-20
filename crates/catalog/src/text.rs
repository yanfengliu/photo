//! Text handling SQLite cannot do on its own: Unicode case folding for
//! matching, and a sort key that puts `IMG_2` before `IMG_10`.

use std::fmt::Write as _;

use rusqlite::Connection;
use rusqlite::functions::FunctionFlags;

/// Name of the SQL function that applies [`fold`]. Only ever used in queries,
/// never in the schema, so a catalog stays readable by any SQLite tool.
pub(crate) const FOLD_FUNCTION: &str = "photo_fold";

/// Fold text for case-insensitive comparison. The same function runs on both
/// sides of every text match (stored value in SQL, needle in Rust), which is
/// what makes matching agree for non-ASCII letters too.
///
/// The fold works one character at a time, so folding commutes with taking
/// a substring: if a needle occurs in a text, the folded needle occurs in the
/// folded text. (`str::to_lowercase` does not have that property: it spells
/// a Greek sigma differently at the end of a word.) Each character is
/// lower-cased, with three additions so that every spelling of a letter
/// meets: final sigma becomes sigma, sharp s becomes `ss`, and dotted
/// capital I becomes `i`. Combining sequences are not normalised.
///
/// `photos.file_name_key` is computed with this function, so changing what
/// it returns needs a migration that recomputes that column.
pub(crate) fn fold(text: &str) -> String {
    if text.is_ascii() {
        return text.to_ascii_lowercase();
    }
    let mut folded = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            'ς' => folded.push('σ'),
            'ß' | 'ẞ' => folded.push_str("ss"),
            'İ' => folded.push('i'),
            other => folded.extend(other.to_lowercase()),
        }
    }
    folded
}

/// Make [`fold`] callable from SQL as `photo_fold(text)`; `NULL` stays `NULL`.
pub(crate) fn register_functions(conn: &Connection) -> rusqlite::Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8
        | FunctionFlags::SQLITE_DETERMINISTIC
        | FunctionFlags::SQLITE_INNOCUOUS;
    conn.create_scalar_function(FOLD_FUNCTION, 1, flags, |context| {
        let text = context
            .get_raw(0)
            .as_str_or_null()
            .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))?;
        Ok(text.map(fold))
    })
}

/// A key whose plain byte order is the natural order of file names: case is
/// ignored and runs of digits compare as numbers.
///
/// Each digit run is written without leading zeros and prefixed by its length
/// in three digits, so a longer (larger) number sorts after a shorter one.
/// Names that differ only in case or leading zeros share a key; callers break
/// that tie on the name itself.
pub(crate) fn natural_key(name: &str) -> String {
    let lowered = fold(name);
    let mut key = String::with_capacity(lowered.len() + 8);
    let mut rest = lowered.as_str();
    while !rest.is_empty() {
        let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
        if digits == 0 {
            let other = rest
                .find(|c: char| c.is_ascii_digit())
                .unwrap_or(rest.len());
            key.push_str(&rest[..other]);
            rest = &rest[other..];
        } else {
            let number = rest[..digits].trim_start_matches('0');
            let number = if number.is_empty() { "0" } else { number };
            // Writing to a String cannot fail.
            let _ = write!(key, "{:03}{number}", number.len().min(999));
            rest = &rest[digits..];
        }
    }
    key
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(names: &[&str]) -> Vec<String> {
        let mut names: Vec<String> = names.iter().map(|name| name.to_string()).collect();
        names.sort_by_key(|name| natural_key(name));
        names
    }

    #[test]
    fn numbers_inside_names_compare_by_value() {
        assert_eq!(
            sorted(&[
                "IMG_10.jpg",
                "IMG_2.jpg",
                "IMG_1.jpg",
                "IMG_100.jpg",
                "IMG_20.jpg"
            ]),
            [
                "IMG_1.jpg",
                "IMG_2.jpg",
                "IMG_10.jpg",
                "IMG_20.jpg",
                "IMG_100.jpg"
            ]
        );
    }

    #[test]
    fn case_is_ignored_and_leading_zeros_do_not_change_the_value() {
        assert_eq!(natural_key("IMG_007.JPG"), natural_key("img_7.jpg"));
        assert!(natural_key("b.jpg") > natural_key("A.jpg"));
        assert!(natural_key("a9") < natural_key("a010"));
    }

    #[test]
    fn a_name_sorts_before_its_own_extensions() {
        assert!(natural_key("shot") < natural_key("shot1"));
        assert!(natural_key("shot1") < natural_key("shot1a"));
        assert!(natural_key("1.jpg") < natural_key("a.jpg"));
    }

    #[test]
    fn several_numbers_in_one_name_each_compare_by_value() {
        assert_eq!(
            sorted(&["2024-10-2_9.raw", "2024-9-12_1.raw", "2024-10-2_10.raw"]),
            ["2024-9-12_1.raw", "2024-10-2_9.raw", "2024-10-2_10.raw"]
        );
    }

    #[test]
    fn zero_and_very_long_numbers_are_handled() {
        assert_eq!(natural_key("000"), "0010");
        let long = "7".repeat(1200);
        assert_eq!(natural_key(&long), format!("999{long}"));
    }

    #[test]
    fn folding_is_unicode_aware() {
        assert_eq!(fold("ÖSTERREICH Été"), "österreich été");
        assert_eq!(fold("IMG_0042.JPG"), "img_0042.jpg");
    }

    #[test]
    fn every_spelling_of_a_letter_folds_to_the_same_text() {
        // Greek sigma has two lower-case forms; which one `str::to_lowercase`
        // picks depends on what follows it.
        assert_ne!("ΚΟΣ".to_lowercase(), "ΚΟΣΜΟΣ".to_lowercase()[..6]);
        assert_eq!(fold("ΚΟΣΜΟΣ"), "κοσμοσ");
        assert_eq!(fold("κοσμος"), "κοσμοσ");
        assert_eq!(fold("Straße"), fold("STRASSE"));
        assert_eq!(fold("STRAẞE"), "strasse");
        assert_eq!(fold("İstanbul"), "istanbul");
    }

    #[test]
    fn folding_a_part_gives_a_part_of_the_folded_whole() {
        // The property text matching relies on: a needle cut out of a text
        // is found in it after folding, wherever the cut falls.
        for text in [
            "ΚΟΣΜΟΣ ΟΔΟΣ",
            "Straße İstanbul ẞ",
            "Été À L'ÉCOLE",
            "plain ASCII 123",
            "日本 Ǆ ǅ ǆ",
        ] {
            let whole = fold(text);
            for (start, _) in text.char_indices() {
                for end in text[start..]
                    .char_indices()
                    .map(|(offset, c)| start + offset + c.len_utf8())
                {
                    let part = fold(&text[start..end]);
                    assert!(whole.contains(&part), "{:?} of {text:?}", &text[start..end]);
                }
                let (head, tail) = text.split_at(start);
                assert_eq!(format!("{}{}", fold(head), fold(tail)), whole);
            }
        }
    }

    #[test]
    fn the_sql_function_matches_the_rust_one_and_passes_null_through() {
        let conn = Connection::open_in_memory().unwrap();
        register_functions(&conn).unwrap();
        let folded: String = conn
            .query_row("SELECT photo_fold('ÉCLAIR Nikon')", [], |row| row.get(0))
            .unwrap();
        assert_eq!(folded, fold("ÉCLAIR Nikon"));
        let null: Option<String> = conn
            .query_row("SELECT photo_fold(NULL)", [], |row| row.get(0))
            .unwrap();
        assert_eq!(null, None);
    }
}
