//! Output sharpening for the screen: an unsharp mask on luma at the export
//! size, applied after sizing so its radius is in the pixels the viewer
//! sees.
//!
//! The detail is the encoded luma (Rec. 709 weights on the encoded values)
//! minus its Gaussian blur; scaled by the strength and compressed so that a
//! hard edge cannot ring, the same amount is added to all three channels,
//! so edges gain contrast without colour fringes. Working on encoded values
//! spreads the effect evenly from shadows to highlights, as the eye sees
//! it.

use rayon::prelude::*;

use crate::settings::OutputSharpening;

/// The blur's sigma in output pixels: a sub-pixel radius, the usual choice
/// for pictures viewed at 100% on a screen.
const SIGMA: f32 = 0.6;

/// Detail added per unit of detail found, for Low, Standard and High.
const AMOUNTS: [f32; 3] = [0.35, 0.7, 1.2];

/// Halo limit, in encoded units: the boost `b` is compressed to
/// `b · h / (h + |b|)`, so fine texture gets nearly all of it while a hard
/// edge gains at most `h` and does not ring.
const HALO: f32 = 0.05;

/// Luma weights on encoded values (ITU-R BT.709).
const LUMA: [f32; 3] = [0.2126, 0.7152, 0.0722];

/// Output sharpening at one strength.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Sharpener {
    amount: f32,
    kernel: [f32; KERNEL],
}

/// Taps of the blur along each axis.
const KERNEL: usize = 2 * Sharpener::REACH as usize + 1;

impl Sharpener {
    /// Rows (and columns) on each side a sharpened pixel depends on.
    pub(crate) const REACH: u32 = 2;

    /// The sharpener for `level`, or `None` when it is off.
    pub(crate) fn new(level: OutputSharpening) -> Option<Self> {
        let amount = match level {
            OutputSharpening::Off => return None,
            OutputSharpening::Low => AMOUNTS[0],
            OutputSharpening::Standard => AMOUNTS[1],
            OutputSharpening::High => AMOUNTS[2],
        };
        let reach = Self::REACH as i32;
        let mut kernel: [f32; KERNEL] = std::array::from_fn(|tap| {
            let offset = (tap as i32 - reach) as f32;
            (-offset * offset / (2.0 * SIGMA * SIGMA)).exp()
        });
        let total: f32 = kernel.iter().sum();
        kernel.iter_mut().for_each(|weight| *weight /= total);
        Some(Self { amount, kernel })
    }

    /// Rows `first..end` of a picture `width` pixels wide, sharpened.
    ///
    /// `band` holds rows `top..` of the picture (16-bit RGB); it must
    /// include every row within [`Self::REACH`] of the rows asked for that
    /// exists, and it is taken to end where the picture does. Beyond the
    /// picture's edges the edge pixels repeat, so a band cut from a larger
    /// picture gives the same result as the whole picture would.
    pub(crate) fn apply(
        &self,
        band: &[u16],
        width: u32,
        top: u32,
        first: u32,
        end: u32,
    ) -> Vec<u16> {
        let width = width as usize;
        let rows = band.len() / (3 * width.max(1));
        if width == 0 || rows == 0 || end <= first {
            return Vec::new();
        }
        let luma: Vec<f32> = band
            .chunks_exact(3)
            .map(|rgb| {
                (LUMA[0] * f32::from(rgb[0])
                    + LUMA[1] * f32::from(rgb[1])
                    + LUMA[2] * f32::from(rgb[2]))
                    / 65535.0
            })
            .collect();
        let reach = Self::REACH as isize;
        let across: Vec<f32> = luma
            .par_chunks_exact(width)
            .flat_map_iter(|row| {
                (0..width as isize).map(move |x| {
                    self.kernel
                        .iter()
                        .enumerate()
                        .map(|(tap, weight)| {
                            let at = (x + tap as isize - reach).clamp(0, width as isize - 1);
                            weight * row[at as usize]
                        })
                        .sum::<f32>()
                })
            })
            .collect();
        let (offset, count) = ((first - top) as usize, (end - first) as usize);
        let mut out = vec![0u16; count * width * 3];
        out.par_chunks_exact_mut(width * 3)
            .enumerate()
            .for_each(|(index, row_out)| {
                let row = offset + index;
                for x in 0..width {
                    let blurred: f32 = self
                        .kernel
                        .iter()
                        .enumerate()
                        .map(|(tap, weight)| {
                            let at =
                                (row as isize + tap as isize - reach).clamp(0, rows as isize - 1);
                            weight * across[at as usize * width + x]
                        })
                        .sum();
                    let boost = self.amount * (luma[row * width + x] - blurred);
                    let boost = boost * HALO / (HALO + boost.abs()) * 65535.0;
                    let pixel = (row * width + x) * 3;
                    for channel in 0..3 {
                        let value = f32::from(band[pixel + channel]) + boost;
                        row_out[x * 3 + channel] = value.round().clamp(0.0, 65535.0) as u16;
                    }
                }
            });
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(width: usize, rows: usize, value: u16) -> Vec<u16> {
        vec![value; width * rows * 3]
    }

    #[test]
    fn off_is_no_sharpener_and_the_kernel_is_normalized() {
        assert!(Sharpener::new(OutputSharpening::Off).is_none());
        let sharpener = Sharpener::new(OutputSharpening::Standard).unwrap();
        assert!((sharpener.kernel.iter().sum::<f32>() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn a_flat_picture_is_left_exactly_as_it_was() {
        let band = flat(9, 7, 31_000);
        for level in [OutputSharpening::Low, OutputSharpening::High] {
            let sharpener = Sharpener::new(level).unwrap();
            assert_eq!(sharpener.apply(&band, 9, 0, 0, 7), band);
        }
    }

    #[test]
    fn an_edge_gains_contrast_more_with_each_level() {
        // A vertical edge from dark to light.
        let (width, rows) = (12usize, 5usize);
        let band: Vec<u16> = (0..rows * width)
            .flat_map(|i| [if i % width < 6 { 20_000 } else { 40_000 }; 3])
            .collect();
        let mut previous = 0.0f32;
        for level in [
            OutputSharpening::Low,
            OutputSharpening::Standard,
            OutputSharpening::High,
        ] {
            let out = Sharpener::new(level)
                .unwrap()
                .apply(&band, width as u32, 0, 0, rows as u32);
            let (dark, light) = (out[(2 * width + 5) * 3], out[(2 * width + 6) * 3]);
            let step = f32::from(light) - f32::from(dark);
            assert!(dark < 20_000 && light > 40_000, "{level:?}: {dark} {light}");
            assert!(step > previous, "{level:?}");
            previous = step;
            // Far from the edge nothing changes.
            assert_eq!(out[(2 * width) * 3], 20_000);
        }
    }
}
