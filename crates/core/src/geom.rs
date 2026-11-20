//! Minimal 2D geometry shared by settings, the pipeline and the UI.

use serde::{Deserialize, Serialize};

/// A 2D point or vector. Units depend on context and are always documented at
/// the use site (normalized image coordinates, source pixels, output pixels).
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn distance(self, other: Self) -> f32 {
        (self.x - other.x).hypot(self.y - other.y)
    }

    pub fn lerp(self, other: Self, t: f32) -> Self {
        Self::new(
            self.x + (other.x - self.x) * t,
            self.y + (other.y - self.y) * t,
        )
    }
}

/// An axis-aligned rectangle given by its minimum and maximum corners.
/// `x1 >= x0` and `y1 >= y0` for every rectangle produced by this crate.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct Rect {
    pub x0: f32,
    pub y0: f32,
    pub x1: f32,
    pub y1: f32,
}

impl Rect {
    pub const fn new(x0: f32, y0: f32, x1: f32, y1: f32) -> Self {
        Self { x0, y0, x1, y1 }
    }

    pub fn from_size(width: f32, height: f32) -> Self {
        Self::new(0.0, 0.0, width, height)
    }

    pub fn width(&self) -> f32 {
        self.x1 - self.x0
    }

    pub fn height(&self) -> f32 {
        self.y1 - self.y0
    }

    pub fn center(&self) -> Point {
        Point::new((self.x0 + self.x1) * 0.5, (self.y0 + self.y1) * 0.5)
    }

    pub fn is_empty(&self) -> bool {
        self.width() <= 0.0 || self.height() <= 0.0
    }

    pub fn contains(&self, p: Point) -> bool {
        p.x >= self.x0 && p.x <= self.x1 && p.y >= self.y0 && p.y <= self.y1
    }

    /// The overlap of two rectangles, or an empty rectangle anchored at the
    /// would-be overlap corner when they do not intersect.
    pub fn intersect(&self, other: &Self) -> Self {
        let x0 = self.x0.max(other.x0);
        let y0 = self.y0.max(other.y0);
        Self::new(
            x0,
            y0,
            self.x1.min(other.x1).max(x0),
            self.y1.min(other.y1).max(y0),
        )
    }

    /// Grow (or shrink, for negative `amount`) by `amount` on every side.
    pub fn expand(&self, amount: f32) -> Self {
        Self::new(
            self.x0 - amount,
            self.y0 - amount,
            self.x1 + amount,
            self.y1 + amount,
        )
    }

    pub fn translate(&self, dx: f32, dy: f32) -> Self {
        Self::new(self.x0 + dx, self.y0 + dy, self.x1 + dx, self.y1 + dy)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn disjoint_rectangles_intersect_to_empty() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(20.0, 20.0, 30.0, 30.0);
        assert!(a.intersect(&b).is_empty());
    }

    #[test]
    fn overlapping_rectangles_intersect_to_their_overlap() {
        let a = Rect::new(0.0, 0.0, 10.0, 10.0);
        let b = Rect::new(5.0, 2.0, 30.0, 8.0);
        assert_eq!(a.intersect(&b), Rect::new(5.0, 2.0, 10.0, 8.0));
    }

    #[test]
    fn expand_then_shrink_is_identity() {
        let r = Rect::new(1.0, 2.0, 3.0, 4.0);
        assert_eq!(r.expand(2.5).expand(-2.5), r);
    }
}
