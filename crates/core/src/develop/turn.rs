//! Turning a developed photo: a quarter turn or a mirror of the photo as it
//! is seen, which takes the crop with it.
//!
//! The crop rectangle lives in the frame after orientation, so when only the
//! orientation changes the rectangle stays where it was in a frame that has
//! turned under it, and ends up on other content with its sides swapped.
//! [`DevelopSettings::turn`] is the one place a turn is applied, for the
//! Library's rotate commands and the crop tool alike.
//!
//! Masks and healing spots are stored in the source frame and need nothing.
//! The perspective [`Transform`](super::Transform) is left as it is: its
//! sliders are named for the frame as it is seen, so turning a photo that
//! has one changes what it corrects.

use super::{CropConstraint, CropRect, DevelopSettings, Orientation};
use crate::geom::Point;

/// A quarter turn or a mirror of the photo as it is seen.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Turn {
    /// A quarter turn counter-clockwise.
    Left,
    /// A quarter turn clockwise.
    Right,
    /// A mirror left to right.
    FlipHorizontal,
    /// A mirror top to bottom.
    FlipVertical,
}

impl Turn {
    /// All four, for code that must treat each.
    pub const ALL: [Self; 4] = [
        Self::Left,
        Self::Right,
        Self::FlipHorizontal,
        Self::FlipVertical,
    ];

    /// `orientation` followed by this turn.
    pub fn of(self, orientation: Orientation) -> Orientation {
        match self {
            Self::Left => orientation.rotated_ccw(),
            Self::Right => orientation.rotated_cw(),
            Self::FlipHorizontal => orientation.flipped_horizontal(),
            Self::FlipVertical => orientation.flipped_vertical(),
        }
    }

    /// Whether width and height trade places.
    pub fn is_quarter(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }

    /// Where a normalized point of the frame is after the frame is turned.
    pub fn point(self, p: Point) -> Point {
        match self {
            Self::Left => Point::new(p.y, 1.0 - p.x),
            Self::Right => Point::new(1.0 - p.y, p.x),
            Self::FlipHorizontal => Point::new(1.0 - p.x, p.y),
            Self::FlipVertical => Point::new(p.x, 1.0 - p.y),
        }
    }
}

impl DevelopSettings {
    /// Turn or mirror the photo as it is seen. The crop goes with it: the
    /// rectangle stays on the content it was on, a fixed ratio is laid the
    /// way the rectangle now lies, and a mirror reverses the straighten
    /// angle.
    pub fn turn(&mut self, turn: Turn) {
        self.orientation = turn.of(self.orientation);
        let crop = &mut self.crop;
        if let Some(rect) = &mut crop.rect {
            let centre = turn.point(Point::new(rect.cx, rect.cy));
            let (width, height) = if turn.is_quarter() {
                (rect.height, rect.width)
            } else {
                (rect.width, rect.height)
            };
            *rect = CropRect {
                cx: centre.x,
                cy: centre.y,
                width,
                height,
            };
        }
        if turn.is_quarter() {
            if let CropConstraint::Ratio { width, height } = crop.constraint {
                crop.constraint = CropConstraint::Ratio {
                    width: height,
                    height: width,
                };
            }
        } else {
            // Seen in a mirror, a tilt leans the other way.
            crop.angle = -crop.angle;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::develop::Crop;

    /// The source's width over its height in these tests.
    const SOURCE_ASPECT: f32 = 1.5;

    fn orientations() -> Vec<Orientation> {
        (0..4)
            .flat_map(|turns| {
                [
                    Orientation::new(turns, false),
                    Orientation::new(turns, true),
                ]
            })
            .collect()
    }

    fn cropped(orientation: Orientation) -> DevelopSettings {
        DevelopSettings {
            orientation,
            crop: Crop {
                rect: Some(CropRect {
                    cx: 0.3,
                    cy: 0.4,
                    width: 0.5,
                    height: 0.25,
                }),
                angle: 7.0,
                constraint: CropConstraint::Ratio {
                    width: 16.0,
                    height: 9.0,
                },
            },
            ..DevelopSettings::default()
        }
    }

    /// The crop's four corners as points of the source, whatever way the
    /// photo is turned: the content the crop is on.
    fn content(settings: &DevelopSettings) -> Vec<Point> {
        let (width, height) = settings.orientation.apply_to_size(SOURCE_ASPECT, 1.0);
        let rect = settings.crop.rect.expect("these tests crop");
        rect.corners(settings.crop.angle, width / height)
            .iter()
            .map(|corner| settings.orientation.inverse(*corner))
            .collect()
    }

    fn same_points(a: &[Point], b: &[Point]) -> bool {
        a.len() == b.len() && a.iter().all(|p| b.iter().any(|q| p.distance(*q) < 1e-5))
    }

    fn close(a: &DevelopSettings, b: &DevelopSettings) -> bool {
        let (ra, rb) = (a.crop.rect.unwrap(), b.crop.rect.unwrap());
        a.orientation == b.orientation
            && a.crop.constraint == b.crop.constraint
            && (a.crop.angle - b.crop.angle).abs() < 1e-6
            && [
                ra.cx - rb.cx,
                ra.cy - rb.cy,
                ra.width - rb.width,
                ra.height - rb.height,
            ]
            .iter()
            .all(|difference| difference.abs() < 1e-6)
    }

    #[test]
    fn the_crop_stays_on_its_content_through_every_turn_from_every_orientation() {
        for orientation in orientations() {
            for turn in Turn::ALL {
                let before = cropped(orientation);
                let mut after = before.clone();
                after.turn(turn);
                assert_eq!(after.orientation, turn.of(orientation));
                assert!(
                    same_points(&content(&before), &content(&after)),
                    "{turn:?} from {orientation:?} moved the crop: {:?} became {:?}",
                    content(&before),
                    content(&after)
                );
            }
        }
    }

    #[test]
    fn four_quarter_turns_two_mirrors_and_a_turn_there_and_back_change_nothing() {
        for orientation in orientations() {
            let start = cropped(orientation);
            for (turns, what) in [
                (vec![Turn::Right; 4], "four turns right"),
                (vec![Turn::Left; 4], "four turns left"),
                (vec![Turn::FlipHorizontal; 2], "two mirrors left to right"),
                (vec![Turn::FlipVertical; 2], "two mirrors top to bottom"),
                (vec![Turn::Left, Turn::Right], "left then right"),
            ] {
                let mut settings = start.clone();
                for turn in turns {
                    settings.turn(turn);
                }
                assert!(close(&settings, &start), "{what} from {orientation:?}");
            }
        }
    }

    #[test]
    fn a_fixed_ratio_is_laid_the_way_the_rectangle_lies_after_a_quarter_turn() {
        for turn in Turn::ALL {
            let mut settings = cropped(Orientation::IDENTITY);
            settings.turn(turn);
            let expected = if turn.is_quarter() {
                CropConstraint::Ratio {
                    width: 9.0,
                    height: 16.0,
                }
            } else {
                CropConstraint::Ratio {
                    width: 16.0,
                    height: 9.0,
                }
            };
            assert_eq!(settings.crop.constraint, expected, "{turn:?}");
        }
        for constraint in [CropConstraint::Original, CropConstraint::Free] {
            let mut settings = cropped(Orientation::IDENTITY);
            settings.crop.constraint = constraint;
            settings.turn(Turn::Right);
            assert_eq!(settings.crop.constraint, constraint);
        }
    }

    #[test]
    fn a_mirror_reverses_the_straighten_angle_and_a_quarter_turn_keeps_it() {
        for turn in Turn::ALL {
            let mut settings = cropped(Orientation::IDENTITY);
            settings.crop.rect = None;
            settings.turn(turn);
            let expected = if turn.is_quarter() { 7.0 } else { -7.0 };
            assert_eq!(settings.crop.angle, expected, "{turn:?}");
            assert_eq!(settings.crop.rect, None, "{turn:?} invented a rectangle");
        }
    }

    #[test]
    fn a_point_of_the_frame_goes_where_the_orientation_puts_it() {
        let p = Point::new(0.2, 0.7);
        for orientation in orientations() {
            for turn in Turn::ALL {
                let direct = turn.of(orientation).forward(p);
                let stepped = turn.point(orientation.forward(p));
                assert!(
                    direct.distance(stepped) < 1e-6,
                    "{turn:?} from {orientation:?}: {direct:?} against {stepped:?}"
                );
            }
        }
    }
}
