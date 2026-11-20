//! EXIF orientation: where each pixel of the upright picture is found in
//! the grid the file stores.

/// The transform that turns a stored pixel grid upright, as given by the
/// EXIF `Orientation` tag.
///
/// It is held as the recipe for the reverse lookup, because that is how an
/// image is filled: for each upright pixel, where in the stored grid does it
/// come from?
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Orientation {
    /// The upright x axis runs along the stored y axis (and vice versa).
    transposed: bool,
    /// The stored x coordinate counts from the right.
    mirror_x: bool,
    /// The stored y coordinate counts from the bottom.
    mirror_y: bool,
}

impl Orientation {
    /// Stored the way it should be shown.
    pub(crate) const UPRIGHT: Self = Self::new(false, false, false);

    const fn new(transposed: bool, mirror_x: bool, mirror_y: bool) -> Self {
        Self {
            transposed,
            mirror_x,
            mirror_y,
        }
    }

    /// The orientation for an EXIF tag value. Values outside 1 to 8 are what
    /// broken writers produce and mean "unknown", which is treated as upright.
    pub(crate) fn from_exif(code: u32) -> Self {
        match code {
            2 => Self::new(false, true, false),
            3 => Self::new(false, true, true),
            4 => Self::new(false, false, true),
            5 => Self::new(true, false, false),
            6 => Self::new(true, false, true),
            7 => Self::new(true, true, true),
            8 => Self::new(true, true, false),
            _ => Self::UPRIGHT,
        }
    }

    /// Size of the upright picture for a stored grid of `width` x `height`.
    pub(crate) fn upright_size(self, width: usize, height: usize) -> (usize, usize) {
        if self.transposed {
            (height, width)
        } else {
            (width, height)
        }
    }

    /// The stored position `(x, y)` that shows at upright position `(x, y)`,
    /// for a stored grid of `width` x `height`.
    #[inline]
    pub(crate) fn stored_position(
        self,
        x: usize,
        y: usize,
        width: usize,
        height: usize,
    ) -> (usize, usize) {
        let (along, down) = if self.transposed { (y, x) } else { (x, y) };
        (
            if self.mirror_x {
                width - 1 - along
            } else {
                along
            },
            if self.mirror_y {
                height - 1 - down
            } else {
                down
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stored 3 x 2 grid numbered in reading order, turned upright.
    fn upright(code: u32) -> (Vec<u8>, usize, usize) {
        let stored = [0u8, 1, 2, 3, 4, 5];
        let orientation = Orientation::from_exif(code);
        let (width, height) = orientation.upright_size(3, 2);
        let mut out = Vec::new();
        for y in 0..height {
            for x in 0..width {
                let (sx, sy) = orientation.stored_position(x, y, 3, 2);
                out.push(stored[sy * 3 + sx]);
            }
        }
        (out, width, height)
    }

    #[test]
    fn upright_and_mirrored_grids_keep_their_size() {
        assert_eq!(upright(1), (vec![0, 1, 2, 3, 4, 5], 3, 2));
        assert_eq!(upright(2), (vec![2, 1, 0, 5, 4, 3], 3, 2));
        assert_eq!(upright(3), (vec![5, 4, 3, 2, 1, 0], 3, 2));
        assert_eq!(upright(4), (vec![3, 4, 5, 0, 1, 2], 3, 2));
    }

    #[test]
    fn sideways_grids_are_turned() {
        // 6: the camera was turned so the stored grid needs a quarter turn
        // clockwise; the stored left column, read bottom to top, becomes the
        // top row.
        assert_eq!(upright(6), (vec![3, 0, 4, 1, 5, 2], 2, 3));
        // 8: a quarter turn anticlockwise; the stored right column, read top
        // to bottom, becomes the top row.
        assert_eq!(upright(8), (vec![2, 5, 1, 4, 0, 3], 2, 3));
        // 5 and 7: the two diagonal flips.
        assert_eq!(upright(5), (vec![0, 3, 1, 4, 2, 5], 2, 3));
        assert_eq!(upright(7), (vec![5, 2, 4, 1, 3, 0], 2, 3));
    }

    #[test]
    fn values_outside_one_to_eight_mean_upright() {
        for code in [0, 9, 255, 65535, u32::MAX] {
            assert_eq!(Orientation::from_exif(code), Orientation::UPRIGHT);
        }
        assert_eq!(Orientation::from_exif(1), Orientation::UPRIGHT);
    }

    #[test]
    fn every_orientation_visits_each_stored_pixel_exactly_once() {
        for code in 1..=8 {
            let (mut pixels, width, height) = upright(code);
            assert_eq!(width * height, 6);
            pixels.sort_unstable();
            assert_eq!(pixels, [0, 1, 2, 3, 4, 5], "orientation {code}");
        }
    }
}
