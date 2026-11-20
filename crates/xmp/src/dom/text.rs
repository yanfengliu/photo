//! Character data: what counts as blank, and the escaping between text as
//! it is written in a file and text as it is meant.

use std::borrow::Cow;

use quick_xml::escape::unescape;

/// Whether `text` is nothing but XML white space.
pub(super) fn is_blank(text: &str) -> bool {
    let is_space = |byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r');
    text.bytes().all(is_space)
}

/// `raw` with every line end written as a parser reports it: a single `\n`.
pub(super) fn unix_line_ends(raw: &str) -> Cow<'_, str> {
    if raw.contains('\r') {
        Cow::Owned(raw.replace("\r\n", "\n").replace('\r', "\n"))
    } else {
        Cow::Borrowed(raw)
    }
}

/// Character data as a parser reports it: line ends normalised, references
/// resolved.
pub(super) fn decode(raw: &str) -> String {
    let normalised = unix_line_ends(raw);
    match unescape(&normalised) {
        Ok(text) => text.into_owned(),
        // Unreachable: every text node came through `Document::parse`, which
        // checks its references, or through `escape`.
        Err(_) => normalised.into_owned(),
    }
}

/// `text` as it has to be written between tags or, when `in_attribute`,
/// between double quotes, so that it reads back unchanged. Characters XML
/// cannot represent at all are left out.
pub(super) fn escape(text: &str, in_attribute: bool) -> String {
    let mut out = String::with_capacity(text.len());
    for character in text.chars() {
        match character {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            // A bare carriage return would be read back as a line feed.
            '\r' => out.push_str("&#13;"),
            '"' if in_attribute => out.push_str("&quot;"),
            // Bare tabs and line feeds in an attribute are read back as spaces.
            '\t' if in_attribute => out.push_str("&#9;"),
            '\n' if in_attribute => out.push_str("&#10;"),
            '\t' | '\n' | '\u{20}'..='\u{d7ff}' | '\u{e000}'..='\u{fffd}' | '\u{10000}'.. => {
                out.push(character);
            }
            _ => {}
        }
    }
    out
}
