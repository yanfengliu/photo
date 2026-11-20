//! Interpolation for Fujifilm X-Trans mosaics.
//!
//! rawler's own development uses a "bilinear" X-Trans interpolation that
//! replaces every channel of every pixel, measured or not, with the mean of
//! a 5 x 5 window: fine for a thumbnail, a visible blur in a developed
//! picture. Its better interpolator (Markesteijn) relies on wrapping
//! integer arithmetic and panics wherever overflow checks are on. So this
//! layout is interpolated here, in three passes:
//!
//! 1. **Green from green.** More than half the sites are green, and the
//!    pattern is built so that every red or blue site sits in the middle of
//!    a straight run "G G x G G". Green is interpolated along that run with
//!    the four-point cubic (weights -1/6, 2/3, 2/3, -1/6), which follows
//!    fine smooth detail closely.
//! 2. **Green again, with the site's own sample heard.** Four greens cannot
//!    tell an edge next to the site from a smooth slope, nor a real crest
//!    from the overshoot of a cubic. The red or blue sample measured at the
//!    site can: colour changes far more slowly than brightness, so that
//!    sample less the colour difference found at nearby sites of its colour
//!    is a second opinion on the green. The cubic may leave the range of its
//!    four greens only as far as the second opinion goes, and where the two
//!    still contradict each other by more than the noise the second one
//!    wins, unless the differences it rests on disagree among themselves.
//! 3. **Red and blue as green plus a colour difference**, the difference
//!    averaged from the sites of that colour within two pixels, those across
//!    an edge in green counting for little.
//!
//! Measured samples are passed through untouched; an interpolated value
//! never leaves the range of the samples it was made from by more than a
//! third of that range, which is as far as a cubic through them can reach;
//! and a flat field comes back flat.

use rawler::cfa::{CFA, CFA_COLOR_B, CFA_COLOR_G, CFA_COLOR_R};
use rayon::prelude::*;

use super::frame::Area;

/// How far, in pixels, a colour difference is gathered from.
const CHROMA_RADIUS: isize = 2;
/// Width of the Gaussian that weights colour differences by distance.
const CHROMA_SIGMA: f32 = 1.2;
/// Differences up to this fraction of the level are taken for sensor noise
/// rather than for structure in the picture.
const NOISE_RELATIVE: f32 = 0.03;
/// The same, as a share of full scale, for the deepest shadows.
const NOISE_FLOOR: f32 = 0.002;
/// Two values this many times the noise apart count half as kin.
const KINSHIP_NOISES: f32 = 3.0;

/// The difference sensor noise alone explains at a given level.
#[inline]
fn noise(level: f32) -> f32 {
    NOISE_FLOOR + NOISE_RELATIVE * level.abs()
}

/// How much a neighbour whose value is `other` belongs with a site whose
/// value is `own`: one for equal values, falling off once they differ by
/// more than a few times the noise, as they do across an edge.
#[inline]
fn kinship(own: f32, other: f32) -> f32 {
    let distance = (other - own) / (KINSHIP_NOISES * noise(own));
    1.0 / (1.0 + distance * distance)
}

/// One weighted neighbour of a site.
#[derive(Clone, Copy)]
struct Tap {
    dx: isize,
    dy: isize,
    weight: f32,
}

/// Green at a red or blue site as the greens around it suggest.
#[derive(Clone, Copy)]
struct Estimate {
    green: f32,
    /// How far the greens are from lying on a smooth curve through the
    /// site, in the units of the samples: zero for a parabola, the height
    /// of the step for an edge. The colour difference found at a doubtful
    /// site counts for less at its neighbours.
    doubt: f32,
    /// The range of the greens the estimate was made from.
    low: f32,
    high: f32,
}

impl Estimate {
    /// From the greens next to the site on a line through it and, if they
    /// are green too, the two beyond those.
    fn along((a, b): (f32, f32), far: Option<(f32, f32)>) -> Self {
        match far {
            Some((a2, b2)) => Self {
                green: (4.0 * (a + b) - (a2 + b2)) / 6.0,
                doubt: ((b2 - a2) - 2.0 * (b - a)).abs(),
                low: a.min(b).min(a2).min(b2),
                high: a.max(b).max(a2).max(b2),
            },
            None => Self {
                green: (a + b) / 2.0,
                doubt: (a - b).abs(),
                low: a.min(b),
                high: a.max(b),
            },
        }
    }

    /// The estimate held to the range of its greens.
    fn cautious(self) -> f32 {
        self.green.max(self.low).min(self.high)
    }
}

/// The mosaic within the active area, with its colour pattern.
struct Mosaic<'a> {
    samples: &'a [f32],
    /// Width of the whole sensor frame `samples` covers.
    frame_width: usize,
    area: Area,
    /// Colour of each site of one pattern period, as seen from the top-left
    /// of the active area.
    colors: Vec<usize>,
    period_x: usize,
    period_y: usize,
}

impl Mosaic<'_> {
    #[inline]
    fn sample(&self, x: usize, y: usize) -> f32 {
        self.samples[(self.area.y + y) * self.frame_width + self.area.x + x]
    }

    #[inline]
    fn phase(&self, x: usize, y: usize) -> usize {
        (y % self.period_y) * self.period_x + x % self.period_x
    }

    #[inline]
    fn color(&self, x: usize, y: usize) -> usize {
        self.colors[self.phase(x, y)]
    }

    /// The position `(x + dx, y + dy)` if it lies inside the active area.
    #[inline]
    fn offset(&self, x: usize, y: usize, dx: isize, dy: isize) -> Option<(usize, usize)> {
        let x = x.checked_add_signed(dx).filter(|&x| x < self.area.width)?;
        let y = y.checked_add_signed(dy).filter(|&y| y < self.area.height)?;
        Some((x, y))
    }

    /// The green sample at an offset, if there is a green site there.
    #[inline]
    fn green_at(&self, x: usize, y: usize, dx: isize, dy: isize) -> Option<f32> {
        let (x, y) = self.offset(x, y, dx, dy)?;
        (self.color(x, y) == CFA_COLOR_G).then(|| self.sample(x, y))
    }

    /// Green at a red or blue site, from the greens around it alone.
    fn green_from_greens(&self, x: usize, y: usize) -> Estimate {
        let along = |dx: isize, dy: isize| {
            let near = self
                .green_at(x, y, -dx, -dy)
                .zip(self.green_at(x, y, dx, dy))?;
            let far =
                self.green_at(x, y, -2 * dx, -2 * dy)
                    .zip(self.green_at(x, y, 2 * dx, 2 * dy));
            Some(Estimate::along(near, far))
        };
        match (along(1, 0), along(0, 1)) {
            (Some(row), None) => row,
            (None, Some(column)) => column,
            // Both axes offer a pair (not an X-Trans site, but no reason to
            // fail): take the one the picture is smoother along.
            (Some(row), Some(column)) => {
                if row.doubt <= column.doubt {
                    row
                } else {
                    column
                }
            }
            (None, None) => self.green_nearby(x, y),
        }
    }

    /// The mean of whatever green there is among the eight neighbours; the
    /// site's own sample when there is none at all.
    fn green_nearby(&self, x: usize, y: usize) -> Estimate {
        let (mut sum, mut count) = (0.0, 0u32);
        let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
        for dy in -1..=1 {
            for dx in -1..=1 {
                if let Some(green) = self.green_at(x, y, dx, dy) {
                    sum += green;
                    count += 1;
                    low = low.min(green);
                    high = high.max(green);
                }
            }
        }
        if count == 0 {
            let own = self.sample(x, y);
            return Estimate {
                green: own,
                doubt: 0.0,
                low: own,
                high: own,
            };
        }
        Estimate {
            green: sum / count as f32,
            doubt: high - low,
            low,
            high,
        }
    }

    /// Green at a red or blue site, with the site's own sample heard.
    ///
    /// `first` holds, for every site, the green its greens suggest and the
    /// doubt in it; `taps` are the sites of this one's colour around it.
    /// Their colour differences are averaged, counting for more the nearer
    /// they are, the closer their sample is to this site's, and the less
    /// their own green is in doubt. The site's sample less that average is
    /// the second opinion on its green.
    ///
    /// The first opinion is held to the range of its greens, stretched as
    /// far as the second opinion reaches. The result then moves from the
    /// first towards the second by the share their conflict has in the sum
    /// of the conflict, the disagreement among the differences, and the
    /// noise: all the way where a clear second opinion contradicts the
    /// first, not at all where they agree. Neither opinion is ever further
    /// outside the range of the greens than a third of that range.
    fn green_refined(&self, x: usize, y: usize, first: &[[f32; 2]], taps: &[Tap]) -> f32 {
        let estimate = self.green_from_greens(x, y);
        let own = self.sample(x, y);
        let (mut total, mut sum, mut squares) = (0.0f32, 0.0f32, 0.0f32);
        for tap in taps.iter().filter(|tap| (tap.dx, tap.dy) != (0, 0)) {
            if let Some((tx, ty)) = self.offset(x, y, tap.dx, tap.dy) {
                let [green, doubt] = first[ty * self.area.width + tx];
                let other = self.sample(tx, ty);
                let difference = other - green;
                let weight =
                    tap.weight * kinship(own, other) / (doubt * doubt + noise(green).powi(2));
                total += weight;
                sum += weight * difference;
                squares += weight * difference * difference;
            }
        }
        if total <= 0.0 {
            return estimate.cautious();
        }
        let mean = sum / total;
        let disagreement = (squares / total - mean * mean).max(0.0);
        let reach = (estimate.high - estimate.low) / 3.0;
        let second = (own - mean)
            .max(estimate.low - reach)
            .min(estimate.high + reach);
        let held = estimate
            .green
            .max(estimate.low.min(second))
            .min(estimate.high.max(second));
        let conflict = (second - held).powi(2);
        let say = conflict / (conflict + disagreement + noise(held).powi(2));
        held + say * (second - held)
    }

    /// For every site of the pattern, the neighbours of colour `color` whose
    /// colour difference is averaged there (the site itself among them when
    /// it has that colour).
    fn chroma_taps(&self, color: usize) -> Vec<Vec<Tap>> {
        let mut taps = Vec::with_capacity(self.colors.len());
        for y in 0..self.period_y {
            for x in 0..self.period_x {
                let mut site = Vec::new();
                for dy in -CHROMA_RADIUS..=CHROMA_RADIUS {
                    for dx in -CHROMA_RADIUS..=CHROMA_RADIUS {
                        let column = (x as isize + dx).rem_euclid(self.period_x as isize) as usize;
                        let row = (y as isize + dy).rem_euclid(self.period_y as isize) as usize;
                        if self.colors[row * self.period_x + column] == color {
                            let distance_squared = (dx * dx + dy * dy) as f32;
                            let weight =
                                (-distance_squared / (2.0 * CHROMA_SIGMA * CHROMA_SIGMA)).exp();
                            site.push(Tap { dx, dy, weight });
                        }
                    }
                }
                taps.push(site);
            }
        }
        taps
    }

    /// Red or blue at a site of another colour: the green there plus the
    /// colour difference of the sites in `taps`, those whose green is close
    /// to this one's counting for most. Held within a third of the range of
    /// the samples it was made from; where none is in reach, the pixel is
    /// left neutral.
    fn chroma(&self, x: usize, y: usize, green: &[f32], taps: &[Tap]) -> f32 {
        let here = green[y * self.area.width + x];
        let (mut difference, mut total) = (0.0, 0.0);
        let (mut low, mut high) = (f32::INFINITY, f32::NEG_INFINITY);
        for tap in taps {
            if let Some((tx, ty)) = self.offset(x, y, tap.dx, tap.dy) {
                let (sample, there) = (self.sample(tx, ty), green[ty * self.area.width + tx]);
                let weight = tap.weight * kinship(here, there);
                difference += weight * (sample - there);
                total += weight;
                low = low.min(sample);
                high = high.max(sample);
            }
        }
        if total <= 0.0 {
            return here;
        }
        let reach = (high - low) / 3.0;
        (here + difference / total)
            .max(low - reach)
            .min(high + reach)
    }
}

/// Interpolates the red/green/blue mosaic inside `area` to full colour.
///
/// `samples` covers the whole sensor frame, `frame_width` pixels wide, and
/// `cfa` gives the filter colour of each of its sites. The result has one
/// RGB pixel per site of `area`, in row order. Any pattern of red, green and
/// blue filters is accepted; the method is designed for X-Trans.
///
/// The caller guarantees that `area` lies inside the frame.
pub(crate) fn interpolate(
    samples: &[f32],
    frame_width: usize,
    area: Area,
    cfa: &CFA,
) -> Vec<[f32; 3]> {
    let (period_x, period_y) = (cfa.width.max(1), cfa.height.max(1));
    let sites = (0..period_y).flat_map(|y| (0..period_x).map(move |x| (x, y)));
    let mosaic = Mosaic {
        samples,
        frame_width,
        area,
        colors: sites
            .map(|(x, y)| cfa.color_at(area.y + y, area.x + x))
            .collect(),
        period_x,
        period_y,
    };
    let (width, height) = (area.width, area.height);
    if width == 0 || height == 0 {
        return Vec::new();
    }
    let taps = [CFA_COLOR_R, CFA_COLOR_B].map(|color| (color, mosaic.chroma_taps(color)));

    // Green everywhere as the greens alone suggest it, with the doubt.
    let mut first = vec![[0.0f32; 2]; width * height];
    first
        .par_chunks_mut(width)
        .enumerate()
        .for_each(|(y, row)| {
            for (x, value) in row.iter_mut().enumerate() {
                *value = if mosaic.color(x, y) == CFA_COLOR_G {
                    [mosaic.sample(x, y), 0.0]
                } else {
                    let estimate = mosaic.green_from_greens(x, y);
                    [estimate.green, estimate.doubt]
                };
            }
        });

    // Green again at the red and blue sites, now with their own samples.
    let mut green = vec![0.0f32; width * height];
    green
        .par_chunks_mut(width)
        .enumerate()
        .for_each(|(y, row)| {
            for (x, value) in row.iter_mut().enumerate() {
                let own_color = mosaic.color(x, y);
                let own_taps = taps.iter().find(|(color, _)| *color == own_color);
                *value = match own_taps {
                    Some((_, taps)) => {
                        mosaic.green_refined(x, y, &first, &taps[mosaic.phase(x, y)])
                    }
                    None => first[y * width + x][0],
                };
            }
        });
    drop(first);

    let mut out = vec![[0.0f32; 3]; width * height];
    out.par_chunks_mut(width).enumerate().for_each(|(y, row)| {
        for (x, pixel) in row.iter_mut().enumerate() {
            pixel[CFA_COLOR_G] = green[y * width + x];
            let own_color = mosaic.color(x, y);
            for (color, taps) in &taps {
                pixel[*color] = if own_color == *color {
                    mosaic.sample(x, y)
                } else {
                    mosaic.chroma(x, y, &green, &taps[mosaic.phase(x, y)])
                };
            }
        }
    });
    out
}

#[cfg(test)]
mod tests;
