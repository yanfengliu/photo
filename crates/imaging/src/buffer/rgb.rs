//! The interleaved RGB buffer.

use rayon::prelude::*;

use super::plane::Plane;

/// An interleaved RGB `f32` image in row-major order.
///
/// The colour space and encoding of the samples are the caller's business;
/// this crate only moves them around.
#[derive(Clone, Debug, PartialEq)]
pub struct RgbImage {
    width: usize,
    height: usize,
    data: Vec<[f32; 3]>,
}

impl RgbImage {
    /// Samples per pixel.
    pub const CHANNELS: usize = 3;

    /// An image of zeros.
    pub fn new(width: usize, height: usize) -> Self {
        Self::filled(width, height, [0.0; 3])
    }

    /// An image with every pixel set to `value`.
    pub fn filled(width: usize, height: usize, value: [f32; 3]) -> Self {
        Self {
            width,
            height,
            data: vec![value; width * height],
        }
    }

    /// Wrap existing pixels without copying, or `None` when the buffer length
    /// is not `width * height`.
    pub fn from_pixels(width: usize, height: usize, data: Vec<[f32; 3]>) -> Option<Self> {
        (width.checked_mul(height) == Some(data.len())).then_some(Self {
            width,
            height,
            data,
        })
    }

    /// An image whose pixel at `(x, y)` is `f(x, y)`. Rows are computed in
    /// parallel; the result is the same on any number of threads.
    pub fn from_fn(
        width: usize,
        height: usize,
        f: impl Fn(usize, usize) -> [f32; 3] + Sync,
    ) -> Self {
        let mut image = Self::new(width, height);
        image.par_rows_mut(|y, row| {
            for (x, pixel) in row.iter_mut().enumerate() {
                *pixel = f(x, y);
            }
        });
        image
    }

    /// Interleave three planes of the same size, or `None` when they differ.
    pub fn from_planes(red: &Plane, green: &Plane, blue: &Plane) -> Option<Self> {
        let size = (red.width(), red.height());
        if (green.width(), green.height()) != size || (blue.width(), blue.height()) != size {
            return None;
        }
        let mut image = Self::new(size.0, size.1);
        image.par_rows_mut(|y, row| {
            let channels = red.row(y).iter().zip(green.row(y)).zip(blue.row(y));
            for (pixel, ((&r, &g), &b)) in row.iter_mut().zip(channels) {
                *pixel = [r, g, b];
            }
        });
        Some(image)
    }

    /// Pixels per row.
    pub fn width(&self) -> usize {
        self.width
    }

    /// Number of rows.
    pub fn height(&self) -> usize {
        self.height
    }

    /// True when the image holds no pixels.
    pub fn is_empty(&self) -> bool {
        self.data.is_empty()
    }

    /// All pixels, row after row.
    pub fn pixels(&self) -> &[[f32; 3]] {
        &self.data
    }

    /// All pixels, row after row. The length cannot be changed through this.
    pub fn pixels_mut(&mut self) -> &mut [[f32; 3]] {
        &mut self.data
    }

    /// All samples as one flat `r, g, b, r, g, b, ...` slice.
    pub fn as_slice(&self) -> &[f32] {
        self.data.as_flattened()
    }

    /// The pixel buffer, without copying.
    pub fn into_pixels(self) -> Vec<[f32; 3]> {
        self.data
    }

    /// The pixel at `(x, y)`. Panics when out of bounds.
    #[inline]
    pub fn pixel(&self, x: usize, y: usize) -> [f32; 3] {
        assert!(x < self.width, "x {x} is outside the image's width");
        self.data[y * self.width + x]
    }

    /// Set the pixel at `(x, y)`. Panics when out of bounds.
    #[inline]
    pub fn set_pixel(&mut self, x: usize, y: usize, value: [f32; 3]) {
        assert!(x < self.width, "x {x} is outside the image's width");
        self.data[y * self.width + x] = value;
    }

    /// Row `y`. Panics when out of bounds.
    #[inline]
    pub fn row(&self, y: usize) -> &[[f32; 3]] {
        &self.data[y * self.width..(y + 1) * self.width]
    }

    /// Row `y`, mutable. Panics when out of bounds.
    #[inline]
    pub fn row_mut(&mut self, y: usize) -> &mut [[f32; 3]] {
        &mut self.data[y * self.width..(y + 1) * self.width]
    }

    /// Run `f(y, row)` for every row, in parallel. Each row is visited exactly
    /// once and rows cannot see each other, so the result does not depend on
    /// the thread count.
    pub fn par_rows_mut(&mut self, f: impl Fn(usize, &mut [[f32; 3]]) + Sync) {
        if self.width == 0 {
            return;
        }
        self.data
            .par_chunks_exact_mut(self.width)
            .enumerate()
            .for_each(|(y, row)| f(y, row));
    }

    /// One channel (0 red, 1 green, 2 blue) as a plane. Panics when
    /// `channel` is 3 or more.
    pub fn channel(&self, channel: usize) -> Plane {
        assert!(
            channel < Self::CHANNELS,
            "channel {channel} is not 0, 1 or 2"
        );
        self.map_to_plane(|pixel| pixel[channel])
    }

    /// A plane with `f` applied to every pixel: luminance, a dark channel,
    /// one channel of an opponent space, and so on.
    pub fn map_to_plane(&self, f: impl Fn([f32; 3]) -> f32 + Sync) -> Plane {
        let mut plane = Plane::new(self.width, self.height);
        plane.par_rows_mut(|y, row| {
            for (value, &pixel) in row.iter_mut().zip(self.row(y)) {
                *value = f(pixel);
            }
        });
        plane
    }

    /// A copy of the `width` x `height` rectangle whose top-left pixel is
    /// `(x, y)`. Panics when the rectangle does not fit inside the image.
    pub fn crop(&self, x: usize, y: usize, width: usize, height: usize) -> Self {
        assert!(
            x + width <= self.width && y + height <= self.height,
            "crop {width}x{height} at ({x}, {y}) does not fit a {}x{} image",
            self.width,
            self.height
        );
        let mut data = Vec::with_capacity(width * height);
        for row in y..y + height {
            data.extend_from_slice(&self.row(row)[x..x + width]);
        }
        Self {
            width,
            height,
            data,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn from_pixels_checks_the_length() {
        assert!(RgbImage::from_pixels(2, 2, vec![[0.0; 3]; 4]).is_some());
        assert!(RgbImage::from_pixels(2, 2, vec![[0.0; 3]; 3]).is_none());
    }

    #[test]
    fn crop_copies_the_requested_rectangle() {
        let image = RgbImage::from_fn(4, 3, |x, y| [x as f32, y as f32, 0.0]);
        assert_eq!(image.crop(3, 2, 1, 1).pixels(), [[3.0, 2.0, 0.0]]);
        assert_eq!(image.pixel(1, 2), [1.0, 2.0, 0.0]);
        assert_eq!(image.row(1)[3], [3.0, 1.0, 0.0]);
    }

    #[test]
    fn channels_split_and_join() {
        let image = RgbImage::from_fn(3, 2, |x, y| [x as f32, y as f32, (x + y) as f32]);
        let (r, g, b) = (image.channel(0), image.channel(1), image.channel(2));
        assert_eq!(RgbImage::from_planes(&r, &g, &b).unwrap(), image);
        assert!(RgbImage::from_planes(&r, &g, &Plane::new(1, 1)).is_none());
        assert_eq!(image.as_slice()[..6], [0.0, 0.0, 0.0, 1.0, 0.0, 1.0]);
        assert_eq!(image.map_to_plane(|p| p[0] + p[1]), b);
    }

    #[test]
    fn pixels_can_be_edited_in_place() {
        let mut image = RgbImage::new(2, 2);
        image.set_pixel(1, 0, [1.0, 2.0, 3.0]);
        image.row_mut(1)[0] = [4.0, 5.0, 6.0];
        image.pixels_mut()[3][2] = 9.0;
        assert_eq!(
            image.into_pixels(),
            [[0.0; 3], [1.0, 2.0, 3.0], [4.0, 5.0, 6.0], [0.0, 0.0, 9.0]]
        );
    }

    #[test]
    fn an_empty_image_is_allowed_everywhere() {
        let mut image = RgbImage::new(4, 0);
        image.par_rows_mut(|_, _| panic!("no rows to visit"));
        assert!(image.is_empty());
        assert!(image.channel(1).is_empty());
        let mut narrow = RgbImage::new(0, 4);
        narrow.par_rows_mut(|_, _| panic!("no rows to visit"));
    }
}
