//! The crop rectangle's arithmetic: dragging its handles, holding it to an
//! aspect ratio, keeping it on the photo, straightening, and carrying it
//! through quarter turns and mirrors.
//!
//! Everything is worked out in the straightened frame: frame pixels measured
//! from the frame's centre, turned against the crop's angle, where the crop
//! rectangle is upright ([`Upright`]). Whether a rectangle shows only the
//! photo is asked of the engine's geometry ([`shows_photo`]), so a
//! perspective transform or a lens correction that leaves part of the frame
//! empty holds the rectangle back just as the frame's edges do.

use egui::{CursorIcon, Pos2, Rect, Vec2, pos2, vec2};
use photo_core::develop::{Crop, CropRect};
use photo_core::geom::Point;
use photo_engine::Geometry;

use crate::app::develop::canvas::map::turned;

/// A crop rectangle in the straightened frame: upright, in frame pixels
/// measured from the frame's centre.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Upright {
    pub(crate) center: Vec2,
    pub(crate) size: Vec2,
}

impl Upright {
    /// The rectangle `crop` describes, at `angle`, in a frame of `frame`
    /// pixels.
    pub(crate) fn of(crop: CropRect, angle: f32, frame: Vec2) -> Self {
        let center = vec2(crop.cx * frame.x, crop.cy * frame.y);
        Self {
            center: turned(center - frame / 2.0, -angle),
            size: vec2(crop.width * frame.x, crop.height * frame.y),
        }
    }

    /// The rectangle as the settings store it.
    pub(crate) fn to_crop(self, angle: f32, frame: Vec2) -> CropRect {
        let center = frame / 2.0 + turned(self.center, angle);
        CropRect {
            cx: center.x / frame.x,
            cy: center.y / frame.y,
            width: self.size.x / frame.x,
            height: self.size.y / frame.y,
        }
    }

    /// The rectangle as a rect of the straightened frame.
    pub(crate) fn rect(self) -> Rect {
        Rect::from_center_size(self.center.to_pos2(), self.size)
    }

    fn from_rect(rect: Rect) -> Self {
        Self {
            center: rect.center().to_vec2(),
            size: rect.size(),
        }
    }

    /// Width divided by height.
    pub(crate) fn aspect(self) -> f32 {
        self.size.x / self.size.y.max(f32::EPSILON)
    }

    /// The corners, in order around the rectangle.
    fn corners(self) -> [Vec2; 4] {
        let rect = self.rect();
        [
            rect.left_top(),
            rect.right_top(),
            rect.right_bottom(),
            rect.left_bottom(),
        ]
        .map(Pos2::to_vec2)
    }

    /// The rectangle part of the way from this one to `to`.
    fn toward(self, to: Self, t: f32) -> Self {
        Self {
            center: self.center + (to.center - self.center) * t,
            size: self.size + (to.size - self.size) * t,
        }
    }
}

/// Whether every point of the rectangle's edge shows the photo, when the
/// frame is straightened by `angle`: the edge is walked and each point asked
/// of `frame` (the geometry without the crop) where in the source it comes
/// from.
pub(crate) fn shows_photo(rect: Upright, angle: f32, frame: &Geometry) -> bool {
    const SAMPLES: usize = 12;
    const SLACK: f32 = 0.05;
    let (frame_width, frame_height) = frame.frame_size();
    let half_frame = vec2(frame_width as f32, frame_height as f32) / 2.0;
    let (width, height) = frame.source_size();
    let corners = rect.corners();
    (0..4).all(|side| {
        let (from, to) = (corners[side], corners[(side + 1) % 4]);
        (0..SAMPLES).all(|step| {
            let upright = from + (to - from) * (step as f32 / SAMPLES as f32);
            let at = half_frame + turned(upright, angle);
            let source = frame.out_to_src(Point::new(at.x, at.y));
            (-SLACK..=width as f32 + SLACK).contains(&source.x)
                && (-SLACK..=height as f32 + SLACK).contains(&source.y)
        })
    })
}

/// What of the rectangle a drag holds: a corner, an edge, or all of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) struct Handle {
    /// -1 the left edge, 1 the right, 0 neither.
    pub(crate) x: i8,
    /// -1 the top edge, 1 the bottom, 0 neither.
    pub(crate) y: i8,
}

impl Handle {
    /// The inside of the rectangle: a drag moves it.
    pub(crate) const MOVE: Self = Self { x: 0, y: 0 };

    /// The four corners and the four edges.
    pub(crate) const RESIZE: [Self; 8] = [
        Self { x: -1, y: -1 },
        Self { x: 1, y: -1 },
        Self { x: 1, y: 1 },
        Self { x: -1, y: 1 },
        Self { x: 0, y: -1 },
        Self { x: 1, y: 0 },
        Self { x: 0, y: 1 },
        Self { x: -1, y: 0 },
    ];

    pub(crate) fn is_move(self) -> bool {
        self == Self::MOVE
    }

    pub(crate) fn is_corner(self) -> bool {
        self.x != 0 && self.y != 0
    }

    /// Where the handle sits on `rect`: a corner, or the middle of an edge.
    pub(crate) fn on(self, rect: Rect) -> Pos2 {
        let along = |sign: i8, low: f32, high: f32| match sign {
            -1 => low,
            1 => high,
            _ => (low + high) / 2.0,
        };
        pos2(
            along(self.x, rect.left(), rect.right()),
            along(self.y, rect.top(), rect.bottom()),
        )
    }

    /// The pointer over the handle.
    pub(crate) fn cursor(self) -> CursorIcon {
        match (self.x, self.y) {
            (0, 0) => CursorIcon::Grab,
            (0, _) => CursorIcon::ResizeVertical,
            (_, 0) => CursorIcon::ResizeHorizontal,
            (x, y) if x == y => CursorIcon::ResizeNwSe,
            _ => CursorIcon::ResizeNeSw,
        }
    }
}

/// The rectangle `start` after `handle` was dragged by `delta` (frame
/// pixels), held to `aspect` (width over height) when one is locked, no side
/// under `min`, and stopped where `fits` says it would leave the photo.
pub(crate) fn dragged(
    start: Upright,
    handle: Handle,
    delta: Vec2,
    aspect: Option<f32>,
    min: Vec2,
    fits: &dyn Fn(Upright) -> bool,
) -> Upright {
    if handle.is_move() {
        // Along x, then along y: a rectangle held back by one edge of the
        // photo still slides along it.
        let across = Upright {
            center: start.center + vec2(delta.x, 0.0),
            ..start
        };
        let across = settled(start, across, fits);
        let down = Upright {
            center: across.center + vec2(0.0, delta.y),
            ..across
        };
        return settled(across, down, fits);
    }
    match aspect {
        Some(aspect) => {
            let wanted = resized_locked(start, handle, delta, aspect, min);
            settled(start, wanted, fits)
        }
        None => {
            // Each way on its own: a corner held back by one edge of the
            // photo still follows the pointer along it.
            let sideways = Handle { y: 0, ..handle };
            let across = resized_free(start, sideways, delta, min);
            let across = settled(start, across, fits);
            let upright = Handle { x: 0, ..handle };
            let down = resized_free(across, upright, delta, min);
            settled(across, down, fits)
        }
    }
}

/// `to`, or as far towards it from `from` as still fits.
fn settled(from: Upright, to: Upright, fits: &dyn Fn(Upright) -> bool) -> Upright {
    if fits(to) {
        return to;
    }
    if !fits(from) {
        return from;
    }
    let (mut low, mut high) = (0.0f32, 1.0f32);
    for _ in 0..24 {
        let middle = (low + high) / 2.0;
        if fits(from.toward(to, middle)) {
            low = middle;
        } else {
            high = middle;
        }
    }
    from.toward(to, low)
}

fn resized_free(start: Upright, handle: Handle, delta: Vec2, min: Vec2) -> Upright {
    let rect = start.rect();
    let (mut left, mut right) = (rect.left(), rect.right());
    let (mut top, mut bottom) = (rect.top(), rect.bottom());
    match handle.x {
        -1 => left = (left + delta.x).min(right - min.x),
        1 => right = (right + delta.x).max(left + min.x),
        _ => {}
    }
    match handle.y {
        -1 => top = (top + delta.y).min(bottom - min.y),
        1 => bottom = (bottom + delta.y).max(top + min.y),
        _ => {}
    }
    Upright::from_rect(Rect::from_min_max(pos2(left, top), pos2(right, bottom)))
}

/// A resize that keeps `aspect`: a corner keeps the opposite corner where
/// it is, an edge the opposite edge, growing evenly to both sides across.
fn resized_locked(start: Upright, handle: Handle, delta: Vec2, aspect: f32, min: Vec2) -> Upright {
    let pull = vec2(f32::from(handle.x) * delta.x, f32::from(handle.y) * delta.y);
    let height = match (handle.x != 0, handle.y != 0) {
        (true, true) => {
            let grown = (start.size + pull) / start.size;
            start.size.y * grown.x.max(grown.y)
        }
        (true, false) => (start.size.x + pull.x) / aspect,
        _ => start.size.y + pull.y,
    };
    let height = height.max(min.y.max(min.x / aspect));
    let size = vec2(height * aspect, height);
    let grown = size - start.size;
    let shift = vec2(f32::from(handle.x) * grown.x, f32::from(handle.y) * grown.y) / 2.0;
    Upright {
        center: start.center + shift,
        size,
    }
}

/// The largest rectangle of `aspect` about `center` that fits, no larger
/// than `cap` and no side under `min`. `None` when even the smallest does
/// not fit there.
pub(crate) fn largest(
    center: Vec2,
    aspect: f32,
    min: Vec2,
    cap: Vec2,
    fits: &dyn Fn(Upright) -> bool,
) -> Option<Upright> {
    let of_height = |height: f32| Upright {
        center,
        size: vec2(height * aspect, height),
    };
    let floor = min.y.max(min.x / aspect);
    let ceiling = cap.y.min(cap.x / aspect).max(floor);
    if !fits(of_height(floor)) {
        return None;
    }
    if fits(of_height(ceiling)) {
        return Some(of_height(ceiling));
    }
    let (mut low, mut high) = (floor, ceiling);
    for _ in 0..28 {
        let middle = (low + high) / 2.0;
        if fits(of_height(middle)) {
            low = middle;
        } else {
            high = middle;
        }
    }
    Some(of_height(low))
}

/// The rectangle that was `size` about `center` (frame pixels from the
/// frame's centre, the frame not turned) once the frame is straightened by
/// `angle`: as it was if that still shows only the photo, otherwise shrunk
/// about its centre; and when its centre itself no longer shows the photo,
/// the largest of its shape about the frame's centre. `fits` tests a
/// rectangle at that angle.
pub(crate) fn straightened(
    center: Vec2,
    size: Vec2,
    angle: f32,
    min: Vec2,
    fits: &dyn Fn(Upright) -> bool,
) -> Upright {
    let kept = Upright {
        center: turned(center, -angle),
        size,
    };
    if fits(kept) {
        return kept;
    }
    let aspect = kept.aspect();
    largest(kept.center, aspect, min, size, fits)
        .or_else(|| largest(Vec2::ZERO, aspect, min, size, fits))
        .unwrap_or(kept)
}

/// The crop angle that makes the line from `from` to `to` (frame pixels,
/// the frame not turned) level, or upright if it is nearer to that; `None`
/// for a line too short to mean anything.
pub(crate) fn leveling_angle(from: Vec2, to: Vec2) -> Option<f32> {
    let along = to - from;
    if along.length() < 4.0 {
        return None;
    }
    let mut angle = along.y.atan2(along.x).to_degrees();
    // A line has no direction.
    if angle > 90.0 {
        angle -= 180.0;
    } else if angle <= -90.0 {
        angle += 180.0;
    }
    // Nearer upright than level: it is something that should be vertical.
    if angle > 45.0 {
        angle -= 90.0;
    } else if angle < -45.0 {
        angle += 90.0;
    }
    Some(angle.clamp(-45.0, 45.0))
}

/// The rectangle `crop` cuts from a frame of `frame` pixels, upright in the
/// frame straightened by the crop's angle.
pub(crate) fn upright_of(crop: &Crop, frame: Vec2) -> Upright {
    Upright::of(crop.resolved(frame.x / frame.y), crop.angle, frame)
}

/// `crop` with the rectangle `rect` at `angle`. The whole frame, not
/// turned, is stored as no rectangle at all, which is what an uncropped
/// photo has.
pub(crate) fn with_rect(crop: &Crop, rect: Upright, angle: f32, frame: Vec2) -> Crop {
    let cut = rect.to_crop(angle, frame);
    let near = |value: f32, target: f32| (value - target).abs() < 1.0e-4;
    let whole = angle == 0.0
        && near(cut.cx, 0.5)
        && near(cut.cy, 0.5)
        && near(cut.width, 1.0)
        && near(cut.height, 1.0);
    Crop {
        rect: (!whole).then_some(cut),
        angle,
        constraint: crop.constraint,
    }
}

#[cfg(test)]
mod tests;
