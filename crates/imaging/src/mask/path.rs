//! Simplifying a brush stroke's path before it is stamped.
//!
//! A pointer reports a position every few pixels, so a stroke's path has
//! many more points than its shape needs, and every segment costs time for
//! every cell it reaches. Points are dropped only where the path drawn
//! without them stays within a tolerance of the path painted, so a stroke
//! that doubles back keeps its turning point.

/// How far, in cells, the path a stroke is drawn along may stray from the
/// path it was painted along, at least: a quarter of a cell, which is a
/// quarter of a view pixel or less at 1:1 and smaller scales, and a quarter
/// of a source pixel when the view magnifies.
pub(super) const SIMPLIFY_TOLERANCE: f64 = 0.25;
/// Most original path segments one drawn segment may replace. Bounds the
/// work of simplifying a path to a constant per point.
const MAX_RUN: usize = 64;

/// Distance from `p` to the segment `a`-`b`.
fn distance_to_segment(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let length_squared = dx * dx + dy * dy;
    let along = if length_squared > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / length_squared).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (a.0 + along * dx - p.0).hypot(a.1 + along * dy - p.1)
}

/// The points of `path` that a polyline within `tolerance` of it needs:
/// the first, the last, and wherever the path bends by more than that.
///
/// From each kept point the longest run of following points (up to
/// [`MAX_RUN`]) is replaced by one segment, provided every point it skips
/// lies within `tolerance` of that segment. A segment's `tolerance`
/// neighbourhood is convex, so the whole skipped stretch of path lies
/// within `tolerance` of the segment; and the segment lies within
/// `tolerance` of the stretch, which runs from one of its ends to the
/// other. A turning point is never skipped unless the turn is that small.
pub(super) fn simplify(path: &[(f64, f64)], tolerance: f64) -> Vec<(f64, f64)> {
    let Some(&first) = path.first() else {
        return Vec::new();
    };
    let mut kept = vec![first];
    let mut anchor = 0;
    while anchor + 1 < path.len() {
        let mut reach = anchor + 1;
        for end in anchor + 2..=(anchor + MAX_RUN).min(path.len() - 1) {
            let (a, b) = (path[anchor], path[end]);
            let fits = path[anchor + 1..end]
                .iter()
                .all(|&p| distance_to_segment(p, a, b) <= tolerance);
            if !fits {
                break;
            }
            reach = end;
        }
        kept.push(path[reach]);
        anchor = reach;
    }
    kept
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_that_turns_back_keeps_its_turning_point() {
        let there_and_back = [(500.0, 500.0), (524.0, 500.0), (500.0, 500.0)];
        assert_eq!(
            simplify(&there_and_back, SIMPLIFY_TOLERANCE),
            there_and_back
        );
        // A wiggle smaller than the tolerance is not a turn.
        let wiggle = [(500.0, 500.0), (500.1, 500.1), (500.0, 500.0)];
        assert_eq!(
            simplify(&wiggle, SIMPLIFY_TOLERANCE),
            [(500.0, 500.0), (500.0, 500.0)]
        );
        assert!(simplify(&[], SIMPLIFY_TOLERANCE).is_empty());
        assert_eq!(simplify(&[(1.0, 2.0)], SIMPLIFY_TOLERANCE), [(1.0, 2.0)]);
    }

    #[test]
    fn a_long_straight_run_is_drawn_in_bounded_pieces() {
        let line: Vec<(f64, f64)> = (0..=2000).map(|i| (i as f64 * 0.25, 7.0)).collect();
        let kept = simplify(&line, SIMPLIFY_TOLERANCE);
        assert_eq!(kept.len(), 2000 / MAX_RUN + 2);
        assert_eq!((kept[0], kept[kept.len() - 1]), (line[0], line[2000]));
    }

    #[test]
    fn a_simplified_curve_stays_within_the_tolerance_of_the_original() {
        // A dense spiral: every original point near the drawn path, and
        // every drawn point near the original path.
        let spiral: Vec<(f64, f64)> = (0..3000)
            .map(|i| {
                let t = i as f64 * 0.01;
                (
                    500.0 + (20.0 + 5.0 * t) * t.cos(),
                    500.0 + (20.0 + 5.0 * t) * t.sin(),
                )
            })
            .collect();
        let kept = simplify(&spiral, SIMPLIFY_TOLERANCE);
        assert!(
            kept.len() < spiral.len() / 3,
            "{} of {}",
            kept.len(),
            spiral.len()
        );
        let near = |p: (f64, f64), path: &[(f64, f64)]| {
            path.windows(2)
                .map(|pair| distance_to_segment(p, pair[0], pair[1]))
                .fold(f64::INFINITY, f64::min)
        };
        for &p in &spiral {
            assert!(near(p, &kept) <= SIMPLIFY_TOLERANCE + 1e-9, "{p:?}");
        }
        for pair in kept.windows(2) {
            let middle = ((pair[0].0 + pair[1].0) / 2.0, (pair[0].1 + pair[1].1) / 2.0);
            assert!(
                near(middle, &spiral) <= SIMPLIFY_TOLERANCE + 1e-9,
                "{middle:?}"
            );
        }
    }
}
