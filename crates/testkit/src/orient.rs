//! EXIF orientation, from the writer's side: given the picture as a viewer
//! should see it, produce the pixel grid a camera would have stored.
//!
//! Written straight from the EXIF definition ("the 0th row is the visual
//! top/bottom/left/right, the 0th column is the visual left/right/top/bottom")
//! so that it checks, rather than mirrors, the decoder's own mapping.

/// Which visual edge the stored 0th row and 0th column lie on.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

fn edges(orientation: u8) -> (Edge, Edge) {
    match orientation {
        1 => (Edge::Top, Edge::Left),
        2 => (Edge::Top, Edge::Right),
        3 => (Edge::Bottom, Edge::Right),
        4 => (Edge::Bottom, Edge::Left),
        5 => (Edge::Left, Edge::Top),
        6 => (Edge::Right, Edge::Top),
        7 => (Edge::Right, Edge::Bottom),
        8 => (Edge::Left, Edge::Bottom),
        other => panic!("EXIF orientation must be 1 to 8, got {other}"),
    }
}

/// The stored grid size for an upright `width` x `height` picture: swapped
/// for the four orientations that turn the camera on its side.
pub(crate) fn stored_size(orientation: u8, width: u32, height: u32) -> (u32, u32) {
    match edges(orientation).0 {
        Edge::Top | Edge::Bottom => (width, height),
        Edge::Left | Edge::Right => (height, width),
    }
}

/// Rearranges an upright, row-major `width` x `height` image into the grid a
/// camera would store with `orientation`. Returns the pixels and the stored
/// width and height.
pub(crate) fn to_stored<T: Copy>(
    upright: &[T],
    width: usize,
    height: usize,
    orientation: u8,
) -> (Vec<T>, usize, usize) {
    assert_eq!(upright.len(), width * height);
    let (row0, column0) = edges(orientation);
    let sideways = matches!(row0, Edge::Left | Edge::Right);
    let (stored_width, stored_height) = if sideways {
        (height, width)
    } else {
        (width, height)
    };
    let mut stored = Vec::with_capacity(upright.len());
    for stored_y in 0..stored_height {
        for stored_x in 0..stored_width {
            // Stored rows run along the visual axis named by `row0`, stored
            // columns along the one named by `column0`.
            let (x, y) = if sideways {
                let x = if row0 == Edge::Left {
                    stored_y
                } else {
                    width - 1 - stored_y
                };
                let y = if column0 == Edge::Top {
                    stored_x
                } else {
                    height - 1 - stored_x
                };
                (x, y)
            } else {
                let x = if column0 == Edge::Left {
                    stored_x
                } else {
                    width - 1 - stored_x
                };
                let y = if row0 == Edge::Top {
                    stored_y
                } else {
                    height - 1 - stored_y
                };
                (x, y)
            };
            stored.push(upright[y * width + x]);
        }
    }
    (stored, stored_width, stored_height)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 3 x 2 picture whose pixels are numbered in reading order.
    const UPRIGHT: [u8; 6] = [0, 1, 2, 3, 4, 5];

    #[test]
    fn orientation_one_stores_the_picture_as_is() {
        assert_eq!(to_stored(&UPRIGHT, 3, 2, 1), (UPRIGHT.to_vec(), 3, 2));
    }

    #[test]
    fn mirrored_and_half_turned_grids_keep_their_size() {
        assert_eq!(to_stored(&UPRIGHT, 3, 2, 2).0, [2, 1, 0, 5, 4, 3]);
        assert_eq!(to_stored(&UPRIGHT, 3, 2, 3).0, [5, 4, 3, 2, 1, 0]);
        assert_eq!(to_stored(&UPRIGHT, 3, 2, 4).0, [3, 4, 5, 0, 1, 2]);
    }

    #[test]
    fn sideways_grids_swap_width_and_height() {
        // Orientation 6: the stored top row is the picture's right edge, read
        // from the top down; a viewer turns it a quarter turn clockwise.
        assert_eq!(to_stored(&UPRIGHT, 3, 2, 6), (vec![2, 5, 1, 4, 0, 3], 2, 3));
        // Orientation 8: the stored top row is the left edge read bottom-up.
        assert_eq!(to_stored(&UPRIGHT, 3, 2, 8), (vec![3, 0, 4, 1, 5, 2], 2, 3));
        // Orientations 5 and 7 are the two diagonal flips.
        assert_eq!(to_stored(&UPRIGHT, 3, 2, 5).0, [0, 3, 1, 4, 2, 5]);
        assert_eq!(to_stored(&UPRIGHT, 3, 2, 7).0, [5, 2, 4, 1, 3, 0]);
        assert_eq!(stored_size(5, 3, 2), (2, 3));
        assert_eq!(stored_size(4, 3, 2), (3, 2));
    }

    #[test]
    #[should_panic(expected = "1 to 8")]
    fn orientation_zero_is_rejected() {
        stored_size(0, 1, 1);
    }
}
