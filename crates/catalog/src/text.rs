//! Text handling SQLite cannot do on its own: Unicode case folding for
//! matching, the key under which two file names are one name, and a sort
//! key that puts `IMG_2` before `IMG_10`.

use std::fmt::Write as _;

use icu_normalizer::ComposingNormalizerBorrowed;
use rusqlite::Connection;
use rusqlite::functions::FunctionFlags;

/// Name of the SQL function that applies [`fold`]. Only ever used in queries,
/// never in the schema, so a catalog stays readable by any SQLite tool.
pub(crate) const FOLD_FUNCTION: &str = "photo_fold";

/// Name of the SQL function that applies [`same_name_key`]. Like
/// [`FOLD_FUNCTION`], only ever used in queries.
pub(crate) const NAME_KEY_FUNCTION: &str = "photo_name_key";

/// The key under which file names that a file system takes for one name
/// are equal: the name in canonical composition (Unicode's NFC), then
/// case-folded as [`fold`] does it.
///
/// A letter can be written as one character or as a letter followed by a
/// combining accent: `café` and `cafe` with U+0301 after the `e` are
/// different text, but APFS compares names in a normalised form, so they
/// are one name in a folder there, and so is `CAFÉ` where letter case is
/// ignored, as it is by default on APFS, NTFS and FAT. Composing first
/// makes the two spellings one; folding then makes the cases one.
///
/// Nothing stored is computed with this key, so it can change without a
/// migration. Text matching ([`fold`]) does not normalise.
pub fn same_name_key(name: &str) -> String {
    if name.is_ascii() {
        // ASCII is its own canonical composition.
        return name.to_ascii_lowercase();
    }
    fold(&ComposingNormalizerBorrowed::new_nfc().normalize(name))
}

/// Fold text for case-insensitive comparison. The same function runs on both
/// sides of every text match (stored value in SQL, needle in Rust), which is
/// what makes matching agree for non-ASCII letters too.
///
/// The fold works one character at a time, so folding commutes with taking
/// a substring: if a needle occurs in a text, the folded needle occurs in the
/// folded text. (`str::to_lowercase` does not have that property: it spells
/// a Greek sigma differently at the end of a word.) Each character gets
/// Unicode's full case folding: lower-casing, except for the characters in
/// [`full_folding`], so that `ß` meets `ss`, `ﬁ` meets `fi`, `µ` meets `μ`
/// and `ſ` meets `s`. Text is not normalised first, so a letter written as
/// one character and the same letter written with a combining accent stay
/// different.
///
/// `photos.file_name_key` is computed with this function, so changing what
/// it returns needs a migration that recomputes that column.
pub(crate) fn fold(text: &str) -> String {
    if text.is_ascii() {
        return text.to_ascii_lowercase();
    }
    let mut folded = String::with_capacity(text.len());
    for character in text.chars() {
        match full_folding(character) {
            Some(letters) => folded.push_str(letters),
            None => folded.extend(character.to_lowercase()),
        }
    }
    folded
}

/// Unicode's full case folding of `character`, where that is not what
/// lower-casing gives: letters that fold onto another letter (final sigma,
/// the micro sign, long s, Greek symbol forms, old Cyrillic forms) and
/// letters and ligatures that fold to several. These are the entries of
/// status C and F in Unicode's `CaseFolding.txt` that differ from
/// lower-casing, except two kinds: dotted capital I folds to a plain `i`
/// (as it does in most languages, rather than to `i` and a combining dot),
/// and the Greek letters with accents or iota subscript that only meet
/// their partners once text is normalised are left to lower-casing.
fn full_folding(character: char) -> Option<&'static str> {
    Some(match character {
        'ς' => "σ",
        'ß' | 'ẞ' => "ss",
        'İ' => "i",
        '\u{B5}' => "\u{3BC}",    // micro sign: mu
        '\u{17F}' => "s",         // long s
        '\u{345}' => "\u{3B9}",   // combining ypogegrammeni: iota
        '\u{1FBE}' => "\u{3B9}",  // prosgegrammeni: iota
        '\u{3D0}' => "\u{3B2}",   // beta symbol
        '\u{3D1}' => "\u{3B8}",   // theta symbol
        '\u{3D5}' => "\u{3C6}",   // phi symbol
        '\u{3D6}' => "\u{3C0}",   // pi symbol
        '\u{3F0}' => "\u{3BA}",   // kappa symbol
        '\u{3F1}' => "\u{3C1}",   // rho symbol
        '\u{3F5}' => "\u{3B5}",   // lunate epsilon
        '\u{1E9B}' => "\u{1E61}", // long s with dot above
        '\u{1C80}' => "\u{432}",  // old Cyrillic forms
        '\u{1C81}' => "\u{434}",
        '\u{1C82}' => "\u{43E}",
        '\u{1C83}' => "\u{441}",
        '\u{1C84}' | '\u{1C85}' => "\u{442}",
        '\u{1C86}' => "\u{44A}",
        '\u{1C87}' => "\u{463}",
        '\u{1C88}' => "\u{A64B}",
        '\u{149}' => "\u{2BC}n",  // n preceded by apostrophe
        '\u{1F0}' => "j\u{30C}",  // j with caron
        '\u{1E96}' => "h\u{331}", // letters with a mark below or above
        '\u{1E97}' => "t\u{308}",
        '\u{1E98}' => "w\u{30A}",
        '\u{1E99}' => "y\u{30A}",
        '\u{1E9A}' => "a\u{2BE}",
        '\u{FB00}' => "ff", // Latin ligatures
        '\u{FB01}' => "fi",
        '\u{FB02}' => "fl",
        '\u{FB03}' => "ffi",
        '\u{FB04}' => "ffl",
        '\u{FB05}' | '\u{FB06}' => "st",
        '\u{587}' => "\u{565}\u{582}", // Armenian ligatures
        '\u{FB13}' => "\u{574}\u{576}",
        '\u{FB14}' => "\u{574}\u{565}",
        '\u{FB15}' => "\u{574}\u{56B}",
        '\u{FB16}' => "\u{57E}\u{576}",
        '\u{FB17}' => "\u{574}\u{56D}",
        _ => return None,
    })
}

/// Make [`fold`] callable from SQL as `photo_fold(text)`, and
/// [`same_name_key`] as `photo_name_key(text)`; `NULL` stays `NULL`.
pub(crate) fn register_functions(conn: &Connection) -> rusqlite::Result<()> {
    let flags = FunctionFlags::SQLITE_UTF8
        | FunctionFlags::SQLITE_DETERMINISTIC
        | FunctionFlags::SQLITE_INNOCUOUS;
    for (name, apply) in [
        (FOLD_FUNCTION, fold as fn(&str) -> String),
        (NAME_KEY_FUNCTION, same_name_key),
    ] {
        conn.create_scalar_function(name, 1, flags, move |context| {
            let text = context
                .get_raw(0)
                .as_str_or_null()
                .map_err(|error| rusqlite::Error::UserFunctionError(Box::new(error)))?;
            Ok(text.map(apply))
        })?;
    }
    Ok(())
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
    fn letters_that_unicode_folds_differently_from_lower_casing_meet_their_partners() {
        // Full case folding, where it is not plain lower-casing.
        for (one, other) in [
            ("µm", "ΜM"),           // micro sign and Greek capital mu
            ("µ", "μ"),             // micro sign and Greek small mu
            ("ſtraſſe", "STRASSE"), // long s
            ("ﬁeld", "FIELD"),      // ligatures fold to their letters
            ("ﬂow", "FLOW"),
            ("oﬀ", "OFF"),
            ("ﬃ ﬄ", "FFI FFL"),
            ("ﬅ ﬆ", "ST ST"),
            ("ϐϑϕϖϰϱϵ", "ΒΘΦΠΚΡΕ"), // Greek symbol variants
            ("ẛ", "Ṡ"),
            ("ŉ", "ʼN"),
            ("ǰ", "J\u{30C}"),
            ("և", "ԵՒ"),
        ] {
            assert_eq!(fold(one), fold(other), "{one} and {other}");
        }
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
            "ﬁeld µm ſ ﬃ ǰ",
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

        let name = "CAFE\u{301} Été.JPG";
        let key: String = conn
            .query_row("SELECT photo_name_key(?1)", [name], |row| row.get(0))
            .unwrap();
        assert_eq!(key, same_name_key(name));
        let null: Option<String> = conn
            .query_row("SELECT photo_name_key(NULL)", [], |row| row.get(0))
            .unwrap();
        assert_eq!(null, None);
    }

    #[test]
    fn names_written_with_other_characters_for_the_same_letters_share_a_key() {
        for (one, other) in [
            ("cafe\u{301}.jpg", "CAF\u{C9}.JPG"), // combining acute, precomposed capital
            ("CAFE\u{301}.jpg", "caf\u{E9}.jpg"), // and the other way round
            ("\u{212B}ngstr\u{F6}m", "\u{C5}NGSTRO\u{308}M"), // Angstrom sign, A with ring
            ("a\u{323}\u{302}", "\u{1EAD}"),      // two accents in either order
            ("a\u{302}\u{323}", "\u{1EAC}"),
            ("Straße.tif", "STRASSE.TIF"),
            ("IMG_0001.JPG", "img_0001.jpg"),
        ] {
            assert_eq!(
                same_name_key(one),
                same_name_key(other),
                "{one:?} and {other:?}"
            );
        }
        for (one, other) in [
            ("cafe.jpg", "caf\u{E9}.jpg"),
            ("e\u{301}", "e\u{300}"),
            ("IMG_1.jpg", "IMG_01.jpg"),
        ] {
            assert_ne!(
                same_name_key(one),
                same_name_key(other),
                "{one:?} and {other:?}"
            );
        }
        assert_eq!(
            same_name_key("cafe\u{301}"),
            "caf\u{E9}",
            "composed, then folded"
        );
    }
}
