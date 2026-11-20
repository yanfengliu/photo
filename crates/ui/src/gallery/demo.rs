//! Made-up content for the gallery: a roll of photos in every state a cell
//! can show, a plausible histogram, and a smooth curve through control points.
//!
//! None of this is application logic. The real app gets all three from the
//! engine; the gallery only needs something believable to put in the widgets.

use photo_core::develop::CurvePoint;
use photo_core::meta::{ColorLabel, Flag, Rating};

use crate::widgets::{Badges, HistogramData, SelectionTier};

/// One cell's worth of made-up photo.
#[derive(Clone, Debug)]
pub struct Photo {
    /// File name, which is also the cell's accessible name.
    pub name: String,
    /// Which generated picture it shows; `None` for a cell with no thumbnail.
    pub picture: Option<usize>,
    /// Selection tier.
    pub tier: SelectionTier,
    /// Star rating.
    pub rating: Rating,
    /// Pick or reject flag.
    pub flag: Flag,
    /// Colour label.
    pub label: ColorLabel,
    /// Badges.
    pub badges: Badges,
}

/// Twenty photos that between them show every tier, rating, flag, label and
/// badge, plus a cell whose thumbnail has not arrived and one whose file is
/// missing. `pictures` is how many generated pictures there are to share out.
pub fn roll(pictures: usize) -> Vec<Photo> {
    use ColorLabel::{Blue, Green, None as NoLabel, Purple, Red, Yellow};
    use Flag::{Picked, Rejected, Unflagged};
    use SelectionTier::{Active, Selected};

    let plain = SelectionTier::None;
    let badge = |edited, virtual_copy, missing| Badges {
        edited,
        virtual_copy,
        missing,
        quick_collection: false,
    };
    let none = badge(false, false, false);
    let edited = badge(true, false, false);
    let states = [
        (plain, 0, Unflagged, NoLabel, none),
        (plain, 3, Picked, NoLabel, edited),
        (Selected, 4, Picked, Red, edited),
        (Active, 5, Picked, NoLabel, edited),
        (Selected, 2, Unflagged, NoLabel, none),
        (plain, 1, Rejected, NoLabel, none),
        (plain, 0, Unflagged, Green, badge(true, true, false)),
        (plain, 4, Unflagged, NoLabel, none),
        (plain, 2, Unflagged, NoLabel, badge(false, false, true)),
        (plain, 5, Picked, Purple, edited),
        (plain, 0, Unflagged, NoLabel, none),
        (Selected, 3, Unflagged, NoLabel, badge(false, true, false)),
        (plain, 1, Unflagged, NoLabel, none),
        (plain, 0, Rejected, NoLabel, edited),
        (plain, 3, Picked, NoLabel, none),
        (plain, 2, Unflagged, Blue, edited),
        (plain, 4, Picked, Yellow, badge(true, true, false)),
        (plain, 0, Unflagged, NoLabel, none),
        (plain, 5, Unflagged, NoLabel, edited),
        (plain, 1, Unflagged, NoLabel, none),
    ];
    states
        .into_iter()
        .enumerate()
        .map(|(index, (tier, stars, flag, label, badges))| Photo {
            name: format!("DSC_{:04}.ARW", 4120 + index),
            // The eleventh cell stands for a thumbnail still being made.
            picture: (index != 10 && pictures > 0).then(|| (index * 3 + 1) % pictures),
            tier,
            rating: Rating::new(stars),
            flag,
            label,
            badges: Badges {
                // Two photos sit in the quick collection.
                quick_collection: index == 7 || index == 16,
                ..badges
            },
        })
        .collect()
}

/// A histogram shaped like a real photograph's: a few humps per channel
/// that only partly overlap, and some highlights blown in the red channel.
pub fn histogram() -> HistogramData {
    let hump = |bin: usize, center: f32, width: f32, height: f32| {
        let offset = (bin as f32 - center) / width;
        height * (-0.5 * offset * offset).exp()
    };
    let channel = |humps: &[(f32, f32, f32)]| {
        let mut counts = [0_u32; 256];
        for (bin, count) in counts.iter_mut().enumerate() {
            let level: f32 = humps
                .iter()
                .map(|(center, width, height)| hump(bin, *center, *width, *height))
                .sum();
            // A little texture, the same every run, so it does not look drawn.
            let grain = 1.0 + 0.025 * ((bin * 37 % 11) as f32 / 10.0 - 0.5);
            *count = (level * grain) as u32;
        }
        counts
    };
    let mut data = HistogramData {
        red: channel(&[
            (58.0, 22.0, 5200.0),
            (150.0, 34.0, 6600.0),
            (226.0, 12.0, 2600.0),
        ]),
        green: channel(&[
            (64.0, 24.0, 6200.0),
            (132.0, 30.0, 7000.0),
            (204.0, 16.0, 1500.0),
        ]),
        blue: channel(&[
            (46.0, 18.0, 7600.0),
            (112.0, 28.0, 5600.0),
            (182.0, 22.0, 1700.0),
        ]),
        luma: channel(&[
            (60.0, 22.0, 5600.0),
            (136.0, 32.0, 6800.0),
            (208.0, 16.0, 1700.0),
        ]),
    };
    data.red[255] = 2400;
    data
}

/// The value at `x` of a smooth curve through `points`, which must be sorted
/// by `x`. The curve never overshoots its control points (a monotone cubic,
/// after Fritsch and Carlson), like the pipeline's own point curves.
pub fn smooth_curve(points: &[CurvePoint], x: f32) -> f32 {
    let (Some(first), Some(last)) = (points.first(), points.last()) else {
        return x;
    };
    if x <= first.x || points.len() == 1 {
        return first.y;
    }
    if x >= last.x {
        return last.y;
    }
    let segment = points
        .partition_point(|point| point.x <= x)
        .clamp(1, points.len() - 1)
        - 1;
    let (a, b) = (points[segment], points[segment + 1]);
    let width = b.x - a.x;
    let t = (x - a.x) / width;
    let (ta, tb) = (tangent(points, segment), tangent(points, segment + 1));
    let (t2, t3) = (t * t, t * t * t);
    (2.0 * t3 - 3.0 * t2 + 1.0) * a.y
        + (t3 - 2.0 * t2 + t) * width * ta
        + (-2.0 * t3 + 3.0 * t2) * b.y
        + (t3 - t2) * width * tb
}

/// The slope of the monotone curve at control point `index`.
fn tangent(points: &[CurvePoint], index: usize) -> f32 {
    let slope = |from: usize| {
        let (a, b) = (points[from], points[from + 1]);
        (b.y - a.y) / (b.x - a.x)
    };
    let last = points.len() - 1;
    if index == 0 {
        return slope(0);
    }
    if index == last {
        return slope(last - 1);
    }
    let (before, after) = (slope(index - 1), slope(index));
    if before * after <= 0.0 {
        // A local extreme: flat, so the curve cannot overshoot it.
        return 0.0;
    }
    // The harmonic mean keeps the curve monotone between the two points.
    2.0 * before * after / (before + after)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_roll_shows_every_state_a_cell_can_be_in() {
        let roll = roll(8);
        assert_eq!(roll.len(), 20);
        assert_eq!(
            roll.iter()
                .filter(|photo| photo.tier == SelectionTier::Active)
                .count(),
            1
        );
        assert!(
            roll.iter()
                .any(|photo| photo.tier == SelectionTier::Selected)
        );
        for stars in 0..=5 {
            assert!(
                roll.iter().any(|photo| photo.rating.stars() == stars),
                "{stars} stars"
            );
        }
        for flag in [Flag::Picked, Flag::Rejected, Flag::Unflagged] {
            assert!(roll.iter().any(|photo| photo.flag == flag), "{flag:?}");
        }
        for label in ColorLabel::ALL {
            assert!(roll.iter().any(|photo| photo.label == label), "{label:?}");
        }
        assert!(roll.iter().any(|photo| photo.badges.edited));
        assert!(roll.iter().any(|photo| photo.badges.virtual_copy));
        assert!(roll.iter().any(|photo| photo.badges.missing));
        assert!(roll.iter().any(|photo| photo.badges.quick_collection));
        assert_eq!(
            roll.iter().filter(|photo| photo.picture.is_none()).count(),
            1
        );
        assert!(
            roll.iter()
                .filter_map(|photo| photo.picture)
                .all(|picture| picture < 8)
        );
    }

    #[test]
    fn the_histogram_clips_only_at_the_bright_end() {
        let data = histogram();
        assert!(data.highlights_clipped());
        assert!(!data.shadows_clipped());
    }

    #[test]
    fn the_smooth_curve_passes_through_its_points_and_never_overshoots() {
        let points = [
            CurvePoint::new(0.0, 0.0),
            CurvePoint::new(0.25, 0.1),
            CurvePoint::new(0.6, 0.8),
            CurvePoint::new(1.0, 1.0),
        ];
        for point in points {
            assert!((smooth_curve(&points, point.x) - point.y).abs() < 1.0e-5);
        }
        let samples: Vec<f32> = (0..=200)
            .map(|step| smooth_curve(&points, step as f32 / 200.0))
            .collect();
        assert!(samples.windows(2).all(|pair| pair[0] <= pair[1] + 1.0e-6));
        assert!(samples.iter().all(|value| (0.0..=1.0).contains(value)));
    }

    #[test]
    fn the_identity_curve_is_a_straight_line() {
        let identity = [CurvePoint::new(0.0, 0.0), CurvePoint::new(1.0, 1.0)];
        for step in 0..=10 {
            let x = step as f32 / 10.0;
            assert!((smooth_curve(&identity, x) - x).abs() < 1.0e-6);
        }
        assert_eq!(smooth_curve(&[], 0.3), 0.3);
    }
}
