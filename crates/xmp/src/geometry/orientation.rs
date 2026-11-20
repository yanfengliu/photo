//! `tiff:Orientation` codes and the arithmetic of combining orientations.

use photo_core::develop::Orientation;

/// `tiff:Orientation` values 1 to 8 in order, as quarter turns clockwise and
/// whether the picture is mirrored left to right first (TIFF 6.0, tag 274;
/// EXIF uses the same values).
const TIFF_ORIENTATIONS: [(u8, bool); 8] = [
    (0, false),
    (0, true),
    (2, false),
    (2, true),
    (3, true),
    (1, false),
    (1, true),
    (3, false),
];

/// The orientation a `tiff:Orientation` value names; anything but 1 to 8
/// names none.
pub(super) fn from_tiff(text: &str) -> Option<Orientation> {
    let code: usize = text.trim().parse().ok()?;
    let (quarter_turns, flip) = *TIFF_ORIENTATIONS.get(code.checked_sub(1)?)?;
    Some(Orientation::new(quarter_turns, flip))
}

/// The `tiff:Orientation` value for an orientation.
pub(super) fn to_tiff(orientation: Orientation) -> usize {
    let wanted = (orientation.quarter_turns % 4, orientation.flip_horizontal);
    let index = TIFF_ORIENTATIONS.iter().position(|entry| *entry == wanted);
    index.map_or(1, |index| index + 1)
}

/// The orientation that applies `first` and then `second`.
pub(super) fn then(first: Orientation, second: Orientation) -> Orientation {
    let mut combined = first;
    if second.flip_horizontal {
        combined = combined.flipped_horizontal();
    }
    for _ in 0..second.quarter_turns % 4 {
        combined = combined.rotated_cw();
    }
    combined
}

/// The orientation that undoes `orientation`.
pub(super) fn inverse(orientation: Orientation) -> Orientation {
    if orientation.flip_horizontal {
        // A mirror followed by any turn is its own inverse.
        orientation
    } else {
        Orientation::new(4 - orientation.quarter_turns % 4, false)
    }
}

#[cfg(test)]
mod tests {
    use photo_core::geom::Point;

    use super::*;

    fn all_orientations() -> impl Iterator<Item = Orientation> {
        (0..4).flat_map(|turns| [false, true].map(|flip| Orientation::new(turns, flip)))
    }

    fn close(a: Point, b: Point) -> bool {
        (a.x - b.x).abs() < 1e-5 && (a.y - b.y).abs() < 1e-5
    }

    #[test]
    fn tiff_codes_cover_the_eight_orientations_once_each() {
        let mut seen = Vec::new();
        for code in 1..=8 {
            let orientation = from_tiff(&code.to_string()).unwrap();
            assert_eq!(to_tiff(orientation), code);
            assert!(!seen.contains(&orientation));
            seen.push(orientation);
        }
        assert_eq!(seen.len(), all_orientations().count());
        for text in ["0", "9", "-1", "", "six", "1.0"] {
            assert_eq!(from_tiff(text), None, "{text:?}");
        }
        assert_eq!(from_tiff(" 6 "), Some(Orientation::new(1, false)));
    }

    #[test]
    fn tiff_codes_mean_what_the_tiff_specification_says() {
        // Where the stored top-left corner ends up on display.
        let corner = |code: &str| from_tiff(code).unwrap().forward(Point::new(0.0, 0.0));
        assert!(close(corner("1"), Point::new(0.0, 0.0)));
        assert!(close(corner("2"), Point::new(1.0, 0.0)));
        assert!(close(corner("3"), Point::new(1.0, 1.0)));
        assert!(close(corner("4"), Point::new(0.0, 1.0)));
        assert!(close(corner("5"), Point::new(0.0, 0.0)));
        assert!(close(corner("6"), Point::new(1.0, 0.0)));
        assert!(close(corner("7"), Point::new(1.0, 1.0)));
        assert!(close(corner("8"), Point::new(0.0, 1.0)));
        // 5 and 7 swap the axes where 1 and 3 do not: the stored top-right
        // corner tells them apart.
        let other = |code: &str| from_tiff(code).unwrap().forward(Point::new(1.0, 0.0));
        assert!(close(other("5"), Point::new(0.0, 1.0)));
        assert!(close(other("7"), Point::new(1.0, 0.0)));
    }

    #[test]
    fn then_composes_and_inverse_undoes() {
        let point = Point::new(0.2, 0.7);
        for first in all_orientations() {
            assert_eq!(
                then(first, inverse(first)),
                Orientation::IDENTITY,
                "{first:?}"
            );
            assert_eq!(
                then(inverse(first), first),
                Orientation::IDENTITY,
                "{first:?}"
            );
            for second in all_orientations() {
                let together = then(first, second).forward(point);
                let in_turn = second.forward(first.forward(point));
                assert!(close(together, in_turn), "{first:?} then {second:?}");
            }
        }
    }
}
