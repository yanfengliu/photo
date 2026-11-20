//! A small stroke font for watermarks: every printable ASCII character as
//! a few straight strokes, drawn with round ends at any size.
//!
//! Glyphs are designed on a grid 12 units tall from the top of the capitals
//! (y = 0) to the baseline (y = 12); most are 8 units wide, and commas,
//! brackets and the underscore reach a little above or below. Round letters
//! are drawn as octagons. Lower-case letters are the capitals at small-caps
//! size ([`SMALL_CAPS`]), which keeps the font small and reads well in a
//! watermark.

use std::sync::OnceLock;

use photo_core::geom::Point;

/// Height of the capitals, in font units.
pub(crate) const CAP_HEIGHT: f32 = 12.0;
/// Half the stroke width, in font units.
pub(crate) const HALF_STROKE: f32 = 0.85;
/// Space between two glyphs' strokes, in font units.
pub(crate) const GAP: f32 = 3.0;
/// The advance of a space, in font units.
const SPACE: f32 = 6.0;
/// Size of a lower-case letter relative to its capital.
const SMALL_CAPS: f32 = 0.75;

/// Strokes per character from `!` (0x21) to `~` (0x7E), one string each.
/// Strokes are separated by `/`; each is a run of `x,y` points joined by
/// straight lines, and a single point is a dot. Lower-case letters reuse
/// the capitals.
const STROKES: [&str; 94] = [
    "1,0 1,8 / 1,12",                                                      // !
    "0,0 0,3 / 4,0 4,3",                                                   // "
    "3,1 2,11 / 6,1 5,11 / 0,4 8,4 / 0,8 8,8",                             // #
    "8,3 6,1 2,1 0,3 0,4 2,6 6,6 8,8 8,9 6,11 2,11 0,9 / 4,-1 4,13",       // $
    "8,0 0,12 / 0,0 2,0 2,2 0,2 0,0 / 6,10 8,10 8,12 6,12 6,10",           // %
    "8,12 1,4 1,2 3,0 5,0 6,2 6,3 0,8 0,10 2,12 5,12 8,8",                 // &
    "0,0 0,3",                                                             // '
    "3,-1 1,2 0,6 1,10 3,13",                                              // (
    "0,-1 2,2 3,6 2,10 0,13",                                              // )
    "4,2 4,10 / 1,4 7,8 / 7,4 1,8",                                        // *
    "4,2 4,10 / 0,6 8,6",                                                  // +
    "1,11 0,14",                                                           // ,
    "0,6 6,6",                                                             // -
    "0,12",                                                                // .
    "0,12 8,0",                                                            // /
    "2,0 6,0 8,2 8,10 6,12 2,12 0,10 0,2 2,0 / 7,2 1,10",                  // 0
    "1,2 4,0 4,12 / 1,12 7,12",                                            // 1
    "0,2 2,0 6,0 8,2 8,4 0,12 8,12",                                       // 2
    "0,1 2,0 6,0 8,2 8,4 6,6 3,6 / 6,6 8,8 8,10 6,12 2,12 0,11",           // 3
    "6,12 6,0 0,8 8,8",                                                    // 4
    "8,0 1,0 0,5 6,5 8,7 8,10 6,12 1,12 0,11",                             // 5
    "7,0 3,0 0,4 0,10 2,12 6,12 8,10 8,8 6,6 2,6 0,8",                     // 6
    "0,0 8,0 3,12",                                                        // 7
    "2,6 0,4 0,2 2,0 6,0 8,2 8,4 6,6 2,6 0,8 0,10 2,12 6,12 8,10 8,8 6,6", // 8
    "8,4 6,6 2,6 0,4 0,2 2,0 6,0 8,2 8,8 5,12 1,12",                       // 9
    "0,4 / 0,12",                                                          // :
    "1,4 / 1,11 0,14",                                                     // ;
    "8,1 0,6 8,11",                                                        // <
    "0,4 8,4 / 0,8 8,8",                                                   // =
    "0,1 8,6 0,11",                                                        // >
    "0,2 2,0 6,0 8,2 8,4 4,7 4,8 / 4,12",                                  // ?
    "6,8 6,4 3,4 2,6 2,7 3,8 6,8 8,7 8,2 6,0 2,0 0,2 0,10 2,12 7,12",      // @
    "0,12 4,0 8,12 / 1,8 7,8",                                             // A
    "0,0 0,12 / 0,0 5,0 7,2 7,4 5,6 0,6 / 0,6 6,6 8,8 8,10 6,12 0,12",     // B
    "8,2 6,0 2,0 0,2 0,10 2,12 6,12 8,10",                                 // C
    "0,0 5,0 8,3 8,9 5,12 0,12 0,0",                                       // D
    "8,0 0,0 0,12 8,12 / 0,6 6,6",                                         // E
    "8,0 0,0 0,12 / 0,6 6,6",                                              // F
    "8,2 6,0 2,0 0,2 0,10 2,12 6,12 8,10 8,6 5,6",                         // G
    "0,0 0,12 / 8,0 8,12 / 0,6 8,6",                                       // H
    "0,0 4,0 / 2,0 2,12 / 0,12 4,12",                                      // I
    "8,0 8,10 6,12 2,12 0,10",                                             // J
    "0,0 0,12 / 8,0 0,8 / 3,5 8,12",                                       // K
    "0,0 0,12 8,12",                                                       // L
    "0,12 0,0 5,7 10,0 10,12",                                             // M
    "0,12 0,0 8,12 8,0",                                                   // N
    "2,0 6,0 8,2 8,10 6,12 2,12 0,10 0,2 2,0",                             // O
    "0,12 0,0 6,0 8,2 8,4 6,6 0,6",                                        // P
    "2,0 6,0 8,2 8,10 6,12 2,12 0,10 0,2 2,0 / 5,9 8,13",                  // Q
    "0,12 0,0 6,0 8,2 8,4 6,6 0,6 / 3,6 8,12",                             // R
    "8,2 6,0 2,0 0,2 0,4 2,6 6,6 8,8 8,10 6,12 2,12 0,10",                 // S
    "0,0 8,0 / 4,0 4,12",                                                  // T
    "0,0 0,10 2,12 6,12 8,10 8,0",                                         // U
    "0,0 4,12 8,0",                                                        // V
    "0,0 2,12 5,5 8,12 10,0",                                              // W
    "0,0 8,12 / 8,0 0,12",                                                 // X
    "0,0 4,6 8,0 / 4,6 4,12",                                              // Y
    "0,0 8,0 0,12 8,12",                                                   // Z
    "4,-1 0,-1 0,13 4,13",                                                 // [
    "0,0 8,12",                                                            // \
    "0,-1 4,-1 4,13 0,13",                                                 // ]
    "0,4 4,0 8,4",                                                         // ^
    "0,14 8,14",                                                           // _
    "0,0 2,3",                                                             // `
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "", // a to m: capitals
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",
    "",                               // n to z: capitals
    "4,-1 2,0 2,5 0,6 2,7 2,12 4,13", // {
    "0,-1 0,13",                      // |
    "0,-1 2,0 2,5 4,6 2,7 2,12 0,13", // }
    "0,7 2,5 4,6 6,7 8,5",            // ~
];

/// One drawable character: its strokes, moved so the leftmost point is at
/// x = 0, and how far the pen moves after it.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Glyph {
    /// Line segments, a dot being a segment of zero length.
    pub(crate) segments: Vec<(Point, Point)>,
    /// Width of the strokes' centre lines, in font units.
    pub(crate) width: f32,
    /// Pen movement after this glyph, in font units.
    pub(crate) advance: f32,
}

/// The glyph `c` is drawn with: printable ASCII as itself (lower case as
/// small capitals), anything else as `?`. Pass text through [`printable`]
/// first to keep `©`.
pub(crate) fn glyph(c: char) -> &'static Glyph {
    static GLYPHS: OnceLock<Vec<Glyph>> = OnceLock::new();
    let glyphs = GLYPHS.get_or_init(|| (0x20u8..=0x7e).map(|code| build(code as char)).collect());
    let code = if (' '..='~').contains(&c) { c } else { '?' };
    &glyphs[code as usize - 0x20]
}

/// The text a watermark actually draws: `©` spelled `(c)`, characters the
/// font lacks as `?`, line breaks and tabs as spaces.
pub(crate) fn printable(text: &str) -> String {
    text.chars()
        .flat_map(|c| match c {
            '©' => "(c)".chars().collect::<Vec<_>>(),
            c if c.is_whitespace() => vec![' '],
            c if (' '..='~').contains(&c) => vec![c],
            _ => vec!['?'],
        })
        .collect()
}

fn build(c: char) -> Glyph {
    if c == ' ' {
        return Glyph {
            segments: Vec::new(),
            width: 0.0,
            advance: SPACE,
        };
    }
    let (design, scale) = if c.is_ascii_lowercase() {
        (c.to_ascii_uppercase(), SMALL_CAPS)
    } else {
        (c, 1.0)
    };
    let points: Vec<Vec<Point>> = STROKES[design as usize - 0x21]
        .split('/')
        .map(|stroke| {
            stroke
                .split_whitespace()
                .filter_map(|point| {
                    let (x, y) = point.split_once(',')?;
                    let (x, y): (f32, f32) = (x.parse().ok()?, y.parse().ok()?);
                    // Small capitals sit on the baseline.
                    Some(Point::new(x * scale, CAP_HEIGHT - (CAP_HEIGHT - y) * scale))
                })
                .collect()
        })
        .filter(|stroke: &Vec<Point>| !stroke.is_empty())
        .collect();
    let left = points
        .iter()
        .flatten()
        .map(|p| p.x)
        .fold(f32::INFINITY, f32::min);
    let right = points
        .iter()
        .flatten()
        .map(|p| p.x)
        .fold(f32::NEG_INFINITY, f32::max);
    let shift = |p: &Point| Point::new(p.x - left, p.y);
    let segments = points
        .iter()
        .flat_map(|stroke| -> Vec<(Point, Point)> {
            if stroke.len() == 1 {
                vec![(shift(&stroke[0]), shift(&stroke[0]))]
            } else {
                stroke
                    .windows(2)
                    .map(|pair| (shift(&pair[0]), shift(&pair[1])))
                    .collect()
            }
        })
        .collect();
    let width = right - left;
    Glyph {
        segments,
        width,
        advance: width + GAP,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_printable_character_has_strokes_that_parse() {
        for code in 0x21u8..=0x7e {
            let c = code as char;
            let glyph = glyph(c);
            assert!(!glyph.segments.is_empty(), "{c:?} has no strokes");
            let points = glyph.segments.iter().flat_map(|(a, b)| [a, b]);
            for point in points {
                assert!((0.0..=10.0).contains(&point.x), "{c:?}: {point:?}");
                assert!((-1.0..=14.0).contains(&point.y), "{c:?}: {point:?}");
            }
            assert!(glyph.advance >= GAP && glyph.width >= 0.0, "{c:?}");
        }
        // Every point in the table has two integer coordinates, so none was
        // silently dropped by the parser.
        for (index, strokes) in STROKES.iter().enumerate() {
            for point in strokes.split(['/', ' ']).filter(|p| !p.is_empty()) {
                let parsed = point
                    .split_once(',')
                    .map(|(x, y)| (x.parse::<i32>(), y.parse::<i32>()));
                assert!(
                    matches!(parsed, Some((Ok(_), Ok(_)))),
                    "entry {index}: {point:?}"
                );
            }
        }
    }

    #[test]
    fn lower_case_is_small_capitals_on_the_baseline() {
        let (capital, small) = (glyph('E'), glyph('e'));
        assert_eq!(capital.segments.len(), small.segments.len());
        assert!((small.width - capital.width * SMALL_CAPS).abs() < 1e-5);
        let lowest = |g: &Glyph| {
            g.segments
                .iter()
                .map(|(a, b)| a.y.max(b.y))
                .fold(0.0, f32::max)
        };
        let highest = |g: &Glyph| {
            g.segments
                .iter()
                .map(|(a, b)| a.y.min(b.y))
                .fold(99.0, f32::min)
        };
        assert_eq!(lowest(small), lowest(capital));
        assert!((highest(small) - CAP_HEIGHT * (1.0 - SMALL_CAPS)).abs() < 1e-5);
    }

    #[test]
    fn text_is_reduced_to_what_the_font_draws() {
        assert_eq!(printable("© 2026 Ünal\tPhoto"), "(c) 2026 ?nal Photo");
        assert_eq!(glyph('é'), glyph('?'));
        assert_eq!(glyph(' ').advance, SPACE);
    }
}
