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
use photo_core::develop::{Crop, CropConstraint, CropRect, DevelopSettings};
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
    let wanted = match aspect {
        Some(aspect) => resized_locked(start, handle, delta, aspect, min),
        None => resized_free(start, handle, delta, min),
    };
    settled(start, wanted, fits)
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

/// A quarter turn or a mirror of the photo as it is seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Turn {
    Left,
    Right,
    FlipHorizontal,
    FlipVertical,
}

/// Turn or mirror the photo, the crop with it: the rectangle stays on the
/// content it was on, where the engine's own rotation leaves it at the same
/// place of a frame that has turned under it.
pub(crate) fn turn(settings: &mut DevelopSettings, turn: Turn) {
    let orientation = settings.orientation;
    settings.orientation = match turn {
        Turn::Left => orientation.rotated_ccw(),
        Turn::Right => orientation.rotated_cw(),
        Turn::FlipHorizontal => orientation.flipped_horizontal(),
        Turn::FlipVertical => orientation.flipped_vertical(),
    };
    let crop = &mut settings.crop;
    let quarter = matches!(turn, Turn::Left | Turn::Right);
    if let Some(rect) = &mut crop.rect {
        let CropRect {
            cx,
            cy,
            width,
            height,
        } = *rect;
        *rect = match turn {
            Turn::Right => CropRect {
                cx: 1.0 - cy,
                cy: cx,
                width: height,
                height: width,
            },
            Turn::Left => CropRect {
                cx: cy,
                cy: 1.0 - cx,
                width: height,
                height: width,
            },
            Turn::FlipHorizontal => CropRect {
                cx: 1.0 - cx,
                ..*rect
            },
            Turn::FlipVertical => CropRect {
                cy: 1.0 - cy,
                ..*rect
            },
        };
    }
    if quarter {
        if let CropConstraint::Ratio { width, height } = crop.constraint {
            crop.constraint = CropConstraint::Ratio {
                width: height,
                height: width,
            };
        }
    } else {
        // A mirror turns the other way.
        crop.angle = -crop.angle;
    }
}

/// An entry of the aspect menu.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Aspect {
    /// The photo's own shape, whichever way it is turned.
    Original,
    /// The shape the camera recorded, as a ratio that stays as it is.
    AsShot,
    /// A named ratio, short side first; it is laid the way the rectangle
    /// lies.
    Ratio(f32, f32),
    /// The ratio typed into the two fields.
    Custom,
}

/// The aspect menu, top to bottom.
pub(crate) const ASPECTS: [(&str, Aspect); 8] = [
    ("Original", Aspect::Original),
    ("As Shot", Aspect::AsShot),
    ("1 : 1", Aspect::Ratio(1.0, 1.0)),
    ("4 : 5", Aspect::Ratio(4.0, 5.0)),
    ("5 : 7", Aspect::Ratio(5.0, 7.0)),
    ("2 : 3", Aspect::Ratio(2.0, 3.0)),
    ("16 : 9", Aspect::Ratio(9.0, 16.0)),
    ("Custom", Aspect::Custom),
];

impl Aspect {
    /// What the menu calls it.
    pub(crate) fn label(self) -> &'static str {
        ASPECTS
            .iter()
            .find(|(_, aspect)| *aspect == self)
            .map_or("Custom", |(label, _)| label)
    }

    /// The constraint it sets, for a frame of `frame` pixels and a
    /// rectangle that lies `landscape` or not.
    pub(crate) fn constraint(self, frame: Vec2, landscape: bool, custom: (f32, f32)) -> CropConstraint {
        let (width, height) = match self {
            Self::Original => return CropConstraint::Original,
            Self::AsShot => (frame.x, frame.y),
            Self::Ratio(short, long) if landscape => (long, short),
            Self::Ratio(short, long) => (short, long),
            Self::Custom => custom,
        };
        CropConstraint::Ratio { width, height }.sanitized()
    }

    /// The menu entry a stored constraint is; `None` for no constraint.
    pub(crate) fn of(constraint: CropConstraint, frame: Vec2) -> Option<Self> {
        let (width, height) = match constraint {
            CropConstraint::Original => return Some(Self::Original),
            CropConstraint::Free => return None,
            CropConstraint::Ratio { width, height } => (width, height),
        };
        let same = |a: f32, b: f32| (a / b - width / height).abs() < 1.0e-3 * (width / height);
        let named = ASPECTS.iter().find_map(|(_, aspect)| match aspect {
            Self::Ratio(short, long) if same(*short, *long) || same(*long, *short) => Some(*aspect),
            _ => None,
        });
        Some(named.unwrap_or(if same(frame.x, frame.y) {
            Self::AsShot
        } else {
            Self::Custom
        }))
    }
}

/// The ratio (width over height) a constraint holds the rectangle to, in a
/// frame of `frame` pixels; `None` when it is free.
pub(crate) fn locked_ratio(constraint: CropConstraint, frame: Vec2) -> Option<f32> {
    match constraint.sanitized() {
        CropConstraint::Original => Some(frame.x / frame.y),
        CropConstraint::Free => None,
        CropConstraint::Ratio { width, height } => Some(width / height),
    }
}

#[cfg(test)]
mod tests {
    use photo_core::develop::Orientation;

    use super::*;
    use crate::app::develop::canvas::map::frame_geometry;

    const SOURCE: (u32, u32) = (1200, 800);
    const FRAME: Vec2 = vec2(1200.0, 800.0);
    const MIN: Vec2 = vec2(12.0, 8.0);

    fn whole() -> Upright {
        Upright {
            center: Vec2::ZERO,
            size: FRAME,
        }
    }

    fn fits_at(angle: f32) -> impl Fn(Upright) -> bool {
        let frame = frame_geometry(&DevelopSettings::default(), SOURCE);
        move |rect| shows_photo(rect, angle, &frame)
    }

    #[test]
    fn a_rectangle_reads_back_from_the_settings_as_it_was_written() {
        let crop = CropRect {
            cx: 0.4,
            cy: 0.55,
            width: 0.5,
            height: 0.3,
        };
        for angle in [0.0, 7.5, -30.0] {
            let upright = Upright::of(crop, angle, FRAME);
            assert_eq!(upright.size, vec2(600.0, 240.0));
            let back = upright.to_crop(angle, FRAME);
            for (was, is) in [
                (crop.cx, back.cx),
                (crop.cy, back.cy),
                (crop.width, back.width),
                (crop.height, back.height),
            ] {
                assert!((was - is).abs() < 1.0e-5, "{angle}: {crop:?} -> {back:?}");
            }
        }
    }

    #[test]
    fn the_whole_frame_shows_the_photo_and_a_turned_or_larger_one_does_not() {
        assert!(fits_at(0.0)(whole()));
        assert!(!fits_at(3.0)(whole()));
        let larger = Upright {
            size: FRAME + vec2(2.0, 0.0),
            ..whole()
        };
        assert!(!fits_at(0.0)(larger));
        let inscribed = Upright::of(CropRect::inscribed(3.0, 1.5, 1.5), 3.0, FRAME);
        assert!(fits_at(3.0)(inscribed));
    }

    #[test]
    fn a_corner_drag_keeps_the_opposite_corner_and_stops_at_the_photos_edge() {
        let start = Upright {
            center: vec2(0.0, 0.0),
            size: vec2(600.0, 400.0),
        };
        let corner = Handle { x: 1, y: 1 };
        let free = dragged(start, corner, vec2(60.0, -40.0), None, MIN, &fits_at(0.0));
        assert_eq!(free.rect().left_top(), start.rect().left_top());
        assert_eq!(free.size, vec2(660.0, 360.0));

        let far = dragged(start, corner, vec2(5000.0, 5000.0), None, MIN, &fits_at(0.0));
        assert!((far.rect().right() - 600.0).abs() < 0.5, "{far:?}");
        assert!((far.rect().bottom() - 400.0).abs() < 0.5, "{far:?}");

        let tiny = dragged(start, corner, vec2(-5000.0, -5000.0), None, MIN, &fits_at(0.0));
        assert_eq!(tiny.size, MIN, "no side under the least");
    }

    #[test]
    fn a_locked_drag_keeps_the_ratio_from_a_corner_and_from_an_edge() {
        let start = Upright {
            center: Vec2::ZERO,
            size: vec2(640.0, 360.0),
        };
        let ratio = 16.0 / 9.0;
        let fits = fits_at(0.0);
        let corner = dragged(start, Handle { x: -1, y: -1 }, vec2(-80.0, -10.0), Some(ratio), MIN, &fits);
        assert!((corner.aspect() - ratio).abs() < 1.0e-4);
        assert_eq!(corner.size.x, 720.0, "the larger pull is followed");
        assert_eq!(corner.rect().right_bottom(), start.rect().right_bottom());

        let edge = dragged(start, Handle { x: 1, y: 0 }, vec2(160.0, 0.0), Some(ratio), MIN, &fits);
        assert!((edge.aspect() - ratio).abs() < 1.0e-4);
        assert_eq!(edge.rect().left(), start.rect().left());
        assert_eq!(edge.center.y, 0.0, "it grows evenly up and down");

        let held = dragged(start, Handle { x: 0, y: 1 }, vec2(0.0, 5000.0), Some(ratio), MIN, &fits);
        assert!((held.aspect() - ratio).abs() < 1.0e-3);
        assert!(fits(held) && held.size.x > 1190.0, "{held:?}");
    }

    #[test]
    fn a_move_slides_along_the_edge_that_holds_it() {
        let start = Upright {
            center: Vec2::ZERO,
            size: vec2(400.0, 300.0),
        };
        let moved = dragged(start, Handle::MOVE, vec2(5000.0, 60.0), None, MIN, &fits_at(0.0));
        assert_eq!(moved.size, start.size);
        assert!((moved.rect().right() - 600.0).abs() < 0.5, "{moved:?}");
        assert_eq!(moved.center.y, 60.0);
    }

    #[test]
    fn straightening_shrinks_about_the_centre_and_grows_back() {
        let fits = fits_at(10.0);
        let turned = straightened(Vec2::ZERO, FRAME, 10.0, MIN, &fits);
        assert!(fits(turned));
        assert!((turned.aspect() - 1.5).abs() < 1.0e-3, "the shape is kept");
        assert!(turned.size.x < 1100.0 && turned.size.x > 900.0, "{turned:?}");
        let grown = Upright {
            size: turned.size * 1.01,
            ..turned
        };
        assert!(!fits(grown), "it is the largest that fits");
        let back = straightened(Vec2::ZERO, FRAME, 0.0, MIN, &fits_at(0.0));
        assert_eq!(back, whole());

        // A rectangle in a corner: kept where it is while it fits.
        let corner = vec2(-380.0, -240.0);
        let small = straightened(corner, vec2(200.0, 120.0), 2.0, MIN, &fits_at(2.0));
        assert_eq!(small.size, vec2(200.0, 120.0));
    }

    #[test]
    fn a_line_drawn_along_a_slope_gives_the_angle_that_levels_it() {
        let level = |dx: f32, dy: f32| leveling_angle(vec2(100.0, 100.0), vec2(100.0 + dx, 100.0 + dy));
        assert!((level(400.0, 400.0 * 5.0f32.to_radians().tan()).unwrap() - 5.0).abs() < 1.0e-3);
        assert!((level(-400.0, -400.0 * 5.0f32.to_radians().tan()).unwrap() - 5.0).abs() < 1.0e-3);
        assert!((level(400.0, -70.0).unwrap() + 9.926).abs() < 0.01);
        // Nearly upright: levelled as a vertical.
        assert!((level(20.0, 400.0).unwrap() + 2.862).abs() < 0.01);
        assert_eq!(level(1.0, 1.0), None);
    }

    #[test]
    fn the_whole_frame_is_stored_as_no_rectangle() {
        let crop = Crop::default();
        assert_eq!(with_rect(&crop, whole(), 0.0, FRAME).rect, None);
        assert!(with_rect(&crop, whole(), 4.0, FRAME).rect.is_some());
        let part = Upright {
            center: vec2(10.0, 0.0),
            size: vec2(600.0, 400.0),
        };
        let stored = with_rect(&crop, part, 0.0, FRAME);
        assert_eq!(upright_of(&stored, FRAME), part);
    }

    /// The source positions of the output's four corners.
    fn shown(settings: &DevelopSettings) -> Vec<Point> {
        let geometry = Geometry::from_settings(SOURCE.0, SOURCE.1, settings);
        let (width, height) = geometry.output_size();
        let (width, height) = (width as f32, height as f32);
        [(0.0, 0.0), (width, 0.0), (width, height), (0.0, height)]
            .iter()
            .map(|(x, y)| geometry.out_to_src(Point::new(*x, *y)))
            .collect()
    }

    #[test]
    fn a_turned_or_mirrored_crop_stays_on_its_content() {
        let mut settings = DevelopSettings::default();
        settings.orientation = Orientation::new(0, false);
        settings.crop = Crop {
            rect: Some(CropRect {
                cx: 0.3,
                cy: 0.6,
                width: 0.4,
                height: 0.25,
            }),
            angle: 8.0,
            constraint: CropConstraint::Ratio {
                width: 16.0,
                height: 9.0,
            },
        };
        let before = shown(&settings);
        for way in [Turn::Right, Turn::Left, Turn::FlipHorizontal, Turn::FlipVertical] {
            let mut turned = settings.clone();
            turn(&mut turned, way);
            let after = shown(&turned);
            for corner in &before {
                let nearest = after
                    .iter()
                    .map(|other| other.distance(*corner))
                    .fold(f32::MAX, f32::min);
                assert!(nearest < 1.5, "{way:?}: {corner:?} is {nearest} px from any corner");
            }
        }
        let mut quarter = settings.clone();
        turn(&mut quarter, Turn::Right);
        assert_eq!(
            quarter.crop.constraint,
            CropConstraint::Ratio {
                width: 9.0,
                height: 16.0
            }
        );
        assert_eq!(quarter.crop.angle, 8.0);
        let mut mirrored = settings.clone();
        turn(&mut mirrored, Turn::FlipHorizontal);
        assert_eq!(mirrored.crop.angle, -8.0);
    }

    #[test]
    fn the_menu_reads_a_stored_ratio_either_way_round() {
        assert_eq!(Aspect::of(CropConstraint::Original, FRAME), Some(Aspect::Original));
        assert_eq!(Aspect::of(CropConstraint::Free, FRAME), None);
        let ratio = |width, height| CropConstraint::Ratio { width, height };
        assert_eq!(Aspect::of(ratio(16.0, 9.0), FRAME), Some(Aspect::Ratio(9.0, 16.0)));
        assert_eq!(Aspect::of(ratio(4.0, 5.0), FRAME), Some(Aspect::Ratio(4.0, 5.0)));
        assert_eq!(Aspect::of(ratio(3.0, 2.0), FRAME), Some(Aspect::Ratio(2.0, 3.0)));
        assert_eq!(Aspect::of(ratio(1200.0, 700.0), vec2(1200.0, 700.0)), Some(Aspect::AsShot));
        assert_eq!(Aspect::of(ratio(2.2, 1.0), FRAME), Some(Aspect::Custom));

        let wide = Aspect::Ratio(9.0, 16.0).constraint(FRAME, true, (1.0, 1.0));
        assert_eq!(wide, ratio(16.0, 9.0));
        let tall = Aspect::Ratio(9.0, 16.0).constraint(FRAME, false, (1.0, 1.0));
        assert_eq!(tall, ratio(9.0, 16.0));
        assert_eq!(Aspect::Custom.constraint(FRAME, true, (7.0, 3.0)), ratio(7.0, 3.0));
        assert_eq!(locked_ratio(CropConstraint::Original, FRAME), Some(1.5));
        assert_eq!(locked_ratio(CropConstraint::Free, FRAME), None);
        assert_eq!(Aspect::Ratio(2.0, 3.0).label(), "2 : 3");
    }
}
