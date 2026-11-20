//! A view: which part of the output to render, and at how many pixels.

use photo_core::geom::{Point, Rect};

use crate::geometry::Geometry;

/// A request to render an output-space rectangle `roi` into `width` x
/// `height` pixels (DESIGN §5.1). Fit-to-window, 1:1 tiles, thumbnails and
/// export strips are all just views.
///
/// `roi` is in full-resolution output pixels (see
/// [`Geometry::output_size`]) and may extend beyond the output. View pixel
/// `(x, y)` has its centre at output position
/// `roi.x0 + (x + 0.5) * roi.width() / width` (and likewise for `y`).
///
/// A view made by [`Self::tile`] is a window onto its parent's **pixel
/// grid**, not a new grid fitted to a rounded rectangle: it keeps the
/// parent's origin and pixel pitch in full precision and only remembers
/// where in that grid it starts. Its pixels therefore sit at exactly the
/// positions the parent's pixels do, and everything decided from a view's
/// scale (the pyramid level, the brush bitmap resolution) is decided
/// identically for every tile. That is what makes contract R1 hold exactly
/// rather than approximately.
///
/// Two views are equal when they are the same window of the same grid, so
/// a tile is not equal to a fresh view of the rectangle it happens to cover.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct View {
    /// Output position of the top-left corner of pixel `(0, 0)` of the grid.
    origin: (f64, f64),
    /// Output pixels per view pixel along x and y.
    pitch: (f64, f64),
    /// The grid pixel this view's top-left pixel is.
    first: (i64, i64),
    width: u32,
    height: u32,
}

impl View {
    /// The view that renders `roi` into `width` x `height` pixels.
    ///
    /// An empty or non-finite `roi` gives a view all of whose pixels sit at
    /// one finite position; it renders without failing.
    pub fn new(roi: Rect, width: u32, height: u32) -> Self {
        let finite_or_zero = |value: f32| {
            if value.is_finite() {
                f64::from(value)
            } else {
                0.0
            }
        };
        let pitch = |extent: f32, pixels: u32| {
            let pitch = f64::from(extent) / f64::from(pixels);
            if pixels > 0 && pitch.is_finite() && pitch > 0.0 {
                pitch
            } else {
                0.0
            }
        };
        Self {
            origin: (finite_or_zero(roi.x0), finite_or_zero(roi.y0)),
            pitch: (pitch(roi.width(), width), pitch(roi.height(), height)),
            first: (0, 0),
            width,
            height,
        }
    }

    /// The whole output at full resolution: one view pixel per output pixel.
    pub fn full(geometry: &Geometry) -> Self {
        let (width, height) = geometry.output_size();
        Self::new(Rect::from_size(width as f32, height as f32), width, height)
    }

    /// The whole output, reduced so that its longer side is at most
    /// `max_edge` pixels. Never enlarges; both sides are at least 1.
    pub fn fit(geometry: &Geometry, max_edge: u32) -> Self {
        let (width, height) = geometry.output_size();
        let long = width.max(height);
        if long <= max_edge {
            return Self::full(geometry);
        }
        let ratio = f64::from(max_edge.max(1)) / f64::from(long);
        let scaled = |side: u32| ((f64::from(side) * ratio).round() as u32).max(1);
        Self::new(
            Rect::from_size(width as f32, height as f32),
            scaled(width),
            scaled(height),
        )
    }

    /// Width of the rendered buffer, in pixels.
    pub fn width(&self) -> u32 {
        self.width
    }

    /// Height of the rendered buffer, in pixels.
    pub fn height(&self) -> u32 {
        self.height
    }

    /// The rectangle of the output this view covers, in full-resolution
    /// output pixels: its first pixel's corner plus its pixel count times
    /// its pitch, rounded to `f32`. For a view made with [`Self::new`] that
    /// is the `roi` it was given, up to `f32` rounding of the roi's extent;
    /// for a tile it is the tile's part of its parent's.
    pub fn roi(&self) -> Rect {
        let (x0, y0) = self.output(0.0, 0.0);
        let (x1, y1) = self.output(f64::from(self.width), f64::from(self.height));
        Rect::new(x0 as f32, y0 as f32, x1 as f32, y1 as f32)
    }

    /// View pixels per output pixel: `width / roi.width()`. 1 for a 1:1
    /// view, 0.5 for a half-size preview. Spatial parameters defined at full
    /// resolution are multiplied by this.
    ///
    /// Every tile of a view has exactly its parent's scale. A view with no
    /// pixels, or whose `roi` is empty, not finite, or so small that its
    /// scale is not a finite `f32`, has no scale; 1 is returned for it so
    /// that nothing downstream divides by zero or multiplies by infinity.
    pub fn scale(&self) -> f32 {
        let scale = (1.0 / self.pitch.0) as f32;
        if scale.is_finite() && scale > 0.0 {
            scale
        } else {
            1.0
        }
    }

    /// The window of this view's pixel grid covering pixels `x..x + width`,
    /// `y..y + height`: a tile. Rendering it gives the pixels that cropping
    /// a render of the whole view would (contract R1); for
    /// [`resample`](crate::resample()), [`rasterize_mask`](crate::rasterize_mask)
    /// and [`grain_plane`](crate::grain_plane) they are the same bits.
    ///
    /// The window may reach past the view's edges; those pixels are
    /// rendered as what they are, positions of the same grid outside it.
    pub fn tile(&self, x: u32, y: u32, width: u32, height: u32) -> Self {
        Self {
            first: (self.first.0 + i64::from(x), self.first.1 + i64::from(y)),
            width,
            height,
            ..*self
        }
    }

    /// Where this view's top-left pixel is in the pixel grid of the view it
    /// was cut from: `(0, 0)` unless it was made by [`Self::tile`].
    ///
    /// This is the anchor for anything that works on blocks of pixels
    /// shared between tiles; subtract the padding of the buffer in question
    /// to get the `origin` of
    /// [`fast_guided_filter`](crate::filter::fast_guided_filter).
    pub fn grid_position(&self) -> (i64, i64) {
        self.first
    }

    /// The output position of a point given in view pixels, where `(0.5,
    /// 0.5)` is the centre of the top-left view pixel. Positions outside the
    /// view are allowed.
    pub fn to_output(&self, x: f32, y: f32) -> Point {
        let (x, y) = self.output(f64::from(x), f64::from(y));
        Point::new(x as f32, y as f32)
    }

    fn output(&self, x: f64, y: f64) -> (f64, f64) {
        (
            self.origin.0 + (self.first.0 as f64 + x) * self.pitch.0,
            self.origin.1 + (self.first.1 as f64 + y) * self.pitch.1,
        )
    }

    /// The pixel grid of this view surrounded by `padding` extra pixels on
    /// every side: the layout every per-view buffer in this crate uses.
    pub(crate) fn grid(&self, padding: u32) -> PixelGrid {
        let padding = i64::from(padding);
        PixelGrid {
            origin: self.origin,
            pitch: self.pitch,
            first: (self.first.0 - padding, self.first.1 - padding),
            width: self.width as usize + 2 * padding as usize,
            height: self.height as usize + 2 * padding as usize,
        }
    }
}

/// Where the pixels of a padded view sit in output space. Buffer pixel
/// `(padding, padding)` is the view's top-left pixel.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PixelGrid {
    origin: (f64, f64),
    pitch: (f64, f64),
    /// The grid pixel that buffer pixel `(0, 0)` is.
    first: (i64, i64),
    /// Buffer width in pixels, padding included.
    pub(crate) width: usize,
    /// Buffer height in pixels, padding included.
    pub(crate) height: usize,
}

impl PixelGrid {
    /// Output x of the centre of buffer column `column`.
    ///
    /// The pixel's index in the shared grid is formed in integers first, so
    /// a tile and its parent compute the same position from the same
    /// operands and get the same bits.
    #[inline]
    pub(crate) fn x(&self, column: usize) -> f64 {
        self.origin.0 + ((self.first.0 + column as i64) as f64 + 0.5) * self.pitch.0
    }

    /// Output y of the centre of buffer row `row`.
    #[inline]
    pub(crate) fn y(&self, row: usize) -> f64 {
        self.origin.1 + ((self.first.1 + row as i64) as f64 + 0.5) * self.pitch.1
    }

    /// Output pixels per view pixel: the geometric mean of the two axes, so
    /// a view whose sides were rounded to whole pixels still has one scale.
    /// 1 for a degenerate view.
    pub(crate) fn pitch(&self) -> f64 {
        let pitch = (self.pitch.0 * self.pitch.1).sqrt();
        if pitch.is_finite() && pitch > 0.0 {
            pitch
        } else {
            1.0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn geometry() -> Geometry {
        Geometry::identity(6000, 4000)
    }

    fn centre(view: &View, padding: u32, column: usize, row: usize) -> (f64, f64) {
        let grid = view.grid(padding);
        (grid.x(column), grid.y(row))
    }

    #[test]
    fn the_full_view_is_one_to_one() {
        let view = View::full(&geometry());
        assert_eq!((view.width(), view.height()), (6000, 4000));
        assert_eq!(view.scale(), 1.0);
        assert_eq!(view.grid_position(), (0, 0));
        assert_eq!(centre(&view, 0, 0, 0), (0.5, 0.5));
        assert_eq!(centre(&view, 0, 5999, 3999), (5999.5, 3999.5));
        assert_eq!(view.grid(0).pitch(), 1.0);
    }

    #[test]
    fn padding_extends_the_grid_outward_without_moving_it() {
        let view = View::new(Rect::new(100.0, 50.0, 1100.0, 550.0), 500, 250);
        let grid = view.grid(3);
        assert_eq!((grid.width, grid.height), (506, 256));
        assert_eq!((grid.x(3), grid.y(3)), centre(&view, 0, 0, 0));
        assert_eq!((grid.x(0), grid.y(0)), (95.0, 45.0));
        assert_eq!(grid.pitch(), 2.0);
    }

    #[test]
    fn fit_keeps_the_aspect_ratio_and_never_enlarges() {
        let view = View::fit(&geometry(), 2048);
        assert_eq!((view.width(), view.height()), (2048, 1365));
        assert_eq!(view.roi(), Rect::from_size(6000.0, 4000.0));
        assert!((view.scale() - 2048.0 / 6000.0).abs() < 1e-6);
        assert_eq!(View::fit(&geometry(), 9000), View::full(&geometry()));
        let tiny = View::fit(&Geometry::identity(4000, 3), 10);
        assert_eq!((tiny.width(), tiny.height()), (10, 1));
    }

    #[test]
    fn a_tile_is_a_window_onto_its_parents_grid() {
        let view = View::new(Rect::new(100.0, 50.0, 1100.0, 550.0), 500, 250);
        let tile = view.tile(120, 30, 64, 32);
        assert_eq!((tile.width(), tile.height()), (64, 32));
        assert_eq!(tile.roi(), Rect::new(340.0, 110.0, 468.0, 174.0));
        assert_eq!(tile.grid_position(), (120, 30));
        assert_eq!(centre(&tile, 0, 0, 0), centre(&view, 0, 120, 30));
        assert_eq!(centre(&tile, 0, 63, 31), centre(&view, 0, 183, 61));
        // A padded tile reaches into its neighbours on the same grid.
        assert_eq!(centre(&tile, 5, 0, 0), centre(&view, 0, 115, 25));
        // A tile of a tile is still on that grid.
        let inner = tile.tile(4, 2, 8, 8);
        assert_eq!(inner.grid_position(), (124, 32));
        assert_eq!(centre(&inner, 0, 0, 0), centre(&view, 0, 124, 32));
    }

    #[test]
    fn tiles_share_their_parents_scale_and_positions_bit_for_bit() {
        // A pitch that no f32 rectangle of a tile could reproduce.
        let view = View::new(Rect::new(0.0, 0.0, 1998.6143, 1333.3157), 1000, 667);
        for (x, y, w, h) in [
            (333u32, 333u32, 100u32, 100u32),
            (7, 660, 13, 7),
            (999, 0, 1, 1),
        ] {
            let tile = view.tile(x, y, w, h);
            assert_eq!(tile.scale().to_bits(), view.scale().to_bits());
            assert_eq!(
                tile.grid(0).pitch().to_bits(),
                view.grid(0).pitch().to_bits()
            );
            for (column, row) in [(0usize, 0usize), (w as usize - 1, h as usize - 1)] {
                let own = centre(&tile, 0, column, row);
                let parents = centre(&view, 0, x as usize + column, y as usize + row);
                assert_eq!(
                    (own.0.to_bits(), own.1.to_bits()),
                    (parents.0.to_bits(), parents.1.to_bits())
                );
            }
        }
    }

    #[test]
    fn view_positions_convert_to_output_positions() {
        let view = View::new(Rect::new(100.0, 50.0, 1100.0, 550.0), 500, 250);
        assert_eq!(view.to_output(0.0, 0.0), Point::new(100.0, 50.0));
        assert_eq!(view.to_output(0.5, 0.5), Point::new(101.0, 51.0));
        assert_eq!(view.to_output(500.0, 250.0), Point::new(1100.0, 550.0));
        assert_eq!(
            view.tile(10, 20, 5, 5).to_output(0.0, 0.0),
            Point::new(120.0, 90.0)
        );
    }

    #[test]
    fn degenerate_views_have_finite_positions() {
        let empty = View::new(Rect::from_size(10.0, 10.0), 0, 0);
        assert_eq!(centre(&empty, 2, 3, 3), (0.0, 0.0));
        assert_eq!(empty.grid(2).pitch(), 1.0);
        assert_eq!(empty.scale(), 1.0);
        assert_eq!(View::new(Rect::default(), 8, 8).scale(), 1.0);
        // A pitch too small for its inverse to be a finite f32.
        let microscopic = View::new(Rect::new(0.0, 0.0, 1.0e-38, 1.0), 1_000_000, 1);
        assert_eq!(microscopic.scale(), 1.0);
        let broken = View::new(Rect::new(f32::NAN, 0.0, f32::INFINITY, 1.0), 4, 4);
        let (x, y) = centre(&broken, 0, 1, 1);
        assert!(x.is_finite() && y.is_finite());
        let backwards = View::new(Rect::new(10.0, 10.0, 0.0, 0.0), 4, 4);
        assert_eq!(centre(&backwards, 0, 3, 3), (10.0, 10.0));
    }
}
