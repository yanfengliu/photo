//! A 5 x 7 bitmap font for contact-sheet labels: upper-case letters,
//! digits and the punctuation the labels use. Lower case is drawn as upper
//! case; anything else is a blank.

/// Rows of a glyph, top first; bit 4 is the leftmost column.
fn glyph(c: char) -> [u8; 7] {
    match c.to_ascii_uppercase() {
        'A' => [14, 17, 17, 31, 17, 17, 17],
        'B' => [30, 17, 17, 30, 17, 17, 30],
        'C' => [14, 17, 16, 16, 16, 17, 14],
        'D' => [28, 18, 17, 17, 17, 18, 28],
        'E' => [31, 16, 16, 30, 16, 16, 31],
        'F' => [31, 16, 16, 30, 16, 16, 16],
        'G' => [14, 17, 16, 23, 17, 17, 15],
        'H' => [17, 17, 17, 31, 17, 17, 17],
        'I' => [14, 4, 4, 4, 4, 4, 14],
        'J' => [7, 2, 2, 2, 2, 18, 12],
        'K' => [17, 18, 20, 24, 20, 18, 17],
        'L' => [16, 16, 16, 16, 16, 16, 31],
        'M' => [17, 27, 21, 21, 17, 17, 17],
        'N' => [17, 17, 25, 21, 19, 17, 17],
        'O' => [14, 17, 17, 17, 17, 17, 14],
        'P' => [30, 17, 17, 30, 16, 16, 16],
        'Q' => [14, 17, 17, 17, 21, 18, 13],
        'R' => [30, 17, 17, 30, 20, 18, 17],
        'S' => [15, 16, 16, 14, 1, 1, 30],
        'T' => [31, 4, 4, 4, 4, 4, 4],
        'U' => [17, 17, 17, 17, 17, 17, 14],
        'V' => [17, 17, 17, 17, 17, 10, 4],
        'W' => [17, 17, 17, 21, 21, 21, 10],
        'X' => [17, 17, 10, 4, 10, 17, 17],
        'Y' => [17, 17, 10, 4, 4, 4, 4],
        'Z' => [31, 1, 2, 4, 8, 16, 31],
        '0' => [14, 17, 19, 21, 25, 17, 14],
        '1' => [4, 12, 4, 4, 4, 4, 14],
        '2' => [14, 17, 1, 2, 4, 8, 31],
        '3' => [31, 2, 4, 2, 1, 17, 14],
        '4' => [2, 6, 10, 18, 31, 2, 2],
        '5' => [31, 16, 30, 1, 1, 17, 14],
        '6' => [6, 8, 16, 30, 17, 17, 14],
        '7' => [31, 1, 2, 4, 8, 8, 8],
        '8' => [14, 17, 17, 14, 17, 17, 14],
        '9' => [14, 17, 17, 15, 1, 2, 12],
        '-' => [0, 0, 0, 31, 0, 0, 0],
        '+' => [0, 4, 4, 31, 4, 4, 0],
        '.' => [0, 0, 0, 0, 0, 12, 12],
        ',' => [0, 0, 0, 0, 12, 4, 8],
        ':' => [0, 12, 12, 0, 12, 12, 0],
        '/' => [1, 1, 2, 4, 8, 16, 16],
        '(' => [2, 4, 8, 8, 8, 4, 2],
        ')' => [8, 4, 2, 2, 2, 4, 8],
        '=' => [0, 0, 31, 0, 31, 0, 0],
        '%' => [24, 25, 2, 4, 8, 19, 3],
        '_' => [0, 0, 0, 0, 0, 0, 31],
        _ => [0; 7],
    }
}

/// Pixels one glyph advances, at scale 1.
pub const ADVANCE: u32 = 6;
/// Height of a line of text, at scale 1.
pub const LINE: u32 = 7;

/// Draw `text` into an RGB buffer of `width` pixels per row, its top-left
/// at `(x, y)`, each font pixel `scale` pixels square. Text past the right
/// edge is cut off.
pub fn draw(
    pixels: &mut [u8],
    width: u32,
    x: u32,
    y: u32,
    scale: u32,
    text: &str,
    colour: [u8; 3],
) {
    let height = pixels.len() as u32 / 3 / width;
    for (index, c) in text.chars().enumerate() {
        let left = x + index as u32 * ADVANCE * scale;
        for (row, bits) in glyph(c).iter().enumerate() {
            for column in 0..5u32 {
                if bits & (16 >> column) == 0 {
                    continue;
                }
                for dy in 0..scale {
                    for dx in 0..scale {
                        let (px, py) = (left + column * scale + dx, y + row as u32 * scale + dy);
                        if px < width && py < height {
                            let at = ((py * width + px) * 3) as usize;
                            pixels[at..at + 3].copy_from_slice(&colour);
                        }
                    }
                }
            }
        }
    }
}
