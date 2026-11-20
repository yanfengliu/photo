//! Where the picture sits on the sensor and which way up it is: the active
//! area, the recommended crop, and the orientation, validated so that
//! nothing downstream can index outside the data.

use rawler::RawImage;
use rawler::formats::tiff::IFD;
use rawler::imgop::{Dim2, Point, Rect};
use rawler::tags::{DngTag, TiffCommonTag};

use crate::MAX_PIXELS;
use crate::error::DecodeError;
use crate::orient::Orientation;

/// A rectangle on the sensor, in pixels from its top-left corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Area {
    pub(crate) x: usize,
    pub(crate) y: usize,
    pub(crate) width: usize,
    pub(crate) height: usize,
}

impl Area {
    pub(crate) fn rect(self) -> Rect {
        Rect::new(
            Point::new(self.x, self.y),
            Dim2::new(self.width, self.height),
        )
    }

    /// The overlap of two areas, `None` when they do not overlap.
    fn intersect(self, other: Self) -> Option<Self> {
        let (x, y) = (self.x.max(other.x), self.y.max(other.y));
        let right = (self.x.saturating_add(self.width)).min(other.x.saturating_add(other.width));
        let bottom = (self.y.saturating_add(self.height)).min(other.y.saturating_add(other.height));
        (right > x && bottom > y).then(|| Self {
            x,
            y,
            width: right - x,
            height: bottom - y,
        })
    }
}

impl From<Rect> for Area {
    fn from(rect: Rect) -> Self {
        Self {
            x: rect.p.x,
            y: rect.p.y,
            width: rect.d.w,
            height: rect.d.h,
        }
    }
}

/// The geometry of one raw frame.
///
/// Guarantees: `active` lies inside the `width` x `height` sensor frame,
/// `crop` lies inside `active`, and neither is empty.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Frame {
    pub(crate) width: usize,
    pub(crate) height: usize,
    /// The part of the sensor that received light. Demosaicing works on
    /// exactly this, so masked border pixels never bleed into the picture.
    pub(crate) active: Area,
    /// The part of the active area the file recommends showing.
    pub(crate) crop: Area,
}

impl Frame {
    /// Checks and normalizes a frame description as found in a file.
    ///
    /// A missing active area means the whole frame; a missing crop means the
    /// whole active area. An active area that leaves the frame is trimmed to
    /// it; a crop that leaves the active area is trimmed likewise, and one
    /// that misses it entirely is ignored. A frame with no pixels, or more
    /// than [`MAX_PIXELS`], is an error.
    pub(crate) fn new(
        width: usize,
        height: usize,
        active: Option<Area>,
        crop: Option<Area>,
    ) -> Result<Self, DecodeError> {
        let pixels = (width as u64).saturating_mul(height as u64);
        if pixels == 0 {
            return Err(DecodeError::Corrupt("the raw frame has no pixels".into()));
        }
        if pixels > MAX_PIXELS {
            return Err(DecodeError::TooLarge(format!(
                "a {width} x {height} sensor is over the {} megapixel limit",
                MAX_PIXELS / 1_000_000
            )));
        }
        let whole = Area {
            x: 0,
            y: 0,
            width,
            height,
        };
        let active = match active {
            None => whole,
            Some(area) => area.intersect(whole).ok_or_else(|| {
                DecodeError::Corrupt("the active area lies outside the sensor".into())
            })?,
        };
        let crop = crop
            .and_then(|area| area.intersect(active))
            .unwrap_or(active);
        Ok(Self {
            width,
            height,
            active,
            crop,
        })
    }

    /// The frame of a decoded (or dummy-decoded) raw image.
    pub(crate) fn of(raw: &RawImage) -> Result<Self, DecodeError> {
        match raw.fuji_rotation_width {
            // The data of these sensors is stored turned by 45 degrees; the
            // crop is given for the picture after it has been turned back,
            // and there is no active area.
            Some(split) => {
                let (width, height) = fuji_upright_size(raw.width, split);
                Self::new(width, height, None, raw.crop_area.map(Area::from))
            }
            None => Self::new(
                raw.width,
                raw.height,
                raw.active_area.map(Area::from),
                raw.crop_area.map(Area::from),
            ),
        }
    }

    /// The frame described by the raw image directory of a DNG, read from
    /// its tags the way rawler's DNG decoder does, without decoding pixels.
    pub(crate) fn of_dng(raw_ifd: &IFD) -> Result<Self, DecodeError> {
        let values = |tag: u16, count: usize| -> Option<Vec<usize>> {
            let entry = raw_ifd.get_entry(tag)?;
            (0..count)
                .map(|index| entry.get_usize(index).ok().flatten())
                .collect()
        };
        let missing = || DecodeError::Corrupt("the DNG raw image has no dimensions".into());
        let width = values(TiffCommonTag::ImageWidth.into(), 1).ok_or_else(missing)?[0];
        let height = values(TiffCommonTag::ImageLength.into(), 1).ok_or_else(missing)?[0];

        // ActiveArea is top, left, bottom, right.
        let active = match values(DngTag::ActiveArea.into(), 4) {
            None => None,
            Some(edges) if edges[2] >= edges[0] && edges[3] >= edges[1] => Some(Area {
                x: edges[1],
                y: edges[0],
                width: edges[3] - edges[1],
                height: edges[2] - edges[0],
            }),
            Some(_) => {
                return Err(DecodeError::Corrupt(
                    "the DNG active area is inside out".into(),
                ));
            }
        };
        // The default crop is relative to the active area.
        let (origin_x, origin_y) = active.map_or((0, 0), |area| (area.x, area.y));
        let origin = values(DngTag::DefaultCropOrigin.into(), 2);
        let size = values(DngTag::DefaultCropSize.into(), 2);
        let crop = origin.zip(size).map(|(origin, size)| Area {
            x: origin[0].saturating_add(origin_x),
            y: origin[1].saturating_add(origin_y),
            width: size[0],
            height: size[1],
        });
        Self::new(width, height, active, crop)
    }
}

/// Size of a Fuji Super CCD picture once its 45 degree storage rotation is
/// undone, for stored data `width` pixels wide split at `split`. The same
/// arithmetic as rawler's `fuji_calc_dimension`, which is not public.
fn fuji_upright_size(width: usize, split: usize) -> (usize, usize) {
    let across = (split as f64 * std::f64::consts::SQRT_2).floor() as usize;
    let down = ((width as f64 - split as f64) * std::f64::consts::SQRT_2).floor() as usize;
    if across > down {
        (across, down)
    } else {
        (down, across)
    }
}

/// The orientation of a raw file from the two places rawler reports one.
///
/// Decoders that work it out themselves set it on the image; for the rest
/// the image says "normal" and the truth is the EXIF tag.
pub(crate) fn orientation(from_image: rawler::Orientation, exif: Option<u16>) -> Orientation {
    use rawler::Orientation::{Normal, Unknown};
    let code = match from_image {
        Normal | Unknown => exif.unwrap_or(1),
        decided => decided.to_u16(),
    };
    Orientation::from_exif(u32::from(code))
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn area(x: usize, y: usize, width: usize, height: usize) -> Area {
        Area {
            x,
            y,
            width,
            height,
        }
    }

    #[test]
    fn missing_areas_default_to_everything() {
        let frame = Frame::new(100, 80, None, None).unwrap();
        assert_eq!(frame.active, area(0, 0, 100, 80));
        assert_eq!(frame.crop, frame.active);
        let frame = Frame::new(100, 80, Some(area(4, 2, 90, 70)), None).unwrap();
        assert_eq!(frame.crop, area(4, 2, 90, 70));
    }

    #[test]
    fn the_crop_is_kept_inside_the_active_area_which_is_kept_inside_the_frame() {
        let frame = Frame::new(
            100,
            80,
            Some(area(4, 2, 200, 200)),
            Some(area(0, 0, 50, 500)),
        )
        .unwrap();
        assert_eq!(frame.active, area(4, 2, 96, 78));
        assert_eq!(frame.crop, area(4, 2, 46, 78));
        // A crop that misses the active area altogether is ignored.
        let frame = Frame::new(
            100,
            80,
            Some(area(0, 0, 10, 10)),
            Some(area(50, 50, 10, 10)),
        )
        .unwrap();
        assert_eq!(frame.crop, area(0, 0, 10, 10));
    }

    #[test]
    fn impossible_frames_are_errors() {
        assert!(matches!(
            Frame::new(0, 80, None, None),
            Err(DecodeError::Corrupt(_))
        ));
        assert!(matches!(
            Frame::new(80, 0, None, None),
            Err(DecodeError::Corrupt(_))
        ));
        let outside = Some(area(100, 0, 10, 10));
        assert!(matches!(
            Frame::new(100, 80, outside, None),
            Err(DecodeError::Corrupt(_))
        ));
        assert!(matches!(
            Frame::new(100_000, 100_000, None, None),
            Err(DecodeError::TooLarge(_))
        ));
        assert!(matches!(
            Frame::new(usize::MAX, usize::MAX, None, None),
            Err(DecodeError::TooLarge(_))
        ));
        // Offsets near the integer limit must not wrap around.
        let wild = Some(area(usize::MAX - 2, usize::MAX - 2, 10, 10));
        assert!(Frame::new(100, 80, wild, wild).is_err());
    }

    #[test]
    fn the_image_orientation_wins_unless_it_is_undecided() {
        use rawler::Orientation as Raw;
        assert_eq!(orientation(Raw::Normal, None), Orientation::UPRIGHT);
        assert_eq!(orientation(Raw::Normal, Some(6)), Orientation::from_exif(6));
        assert_eq!(
            orientation(Raw::Unknown, Some(8)),
            Orientation::from_exif(8)
        );
        assert_eq!(
            orientation(Raw::Rotate90, Some(3)),
            Orientation::from_exif(6)
        );
        assert_eq!(orientation(Raw::Rotate270, None), Orientation::from_exif(8));
        assert_eq!(orientation(Raw::Unknown, Some(77)), Orientation::UPRIGHT);
    }

    #[test]
    fn fuji_sizes_follow_the_diagonal_of_the_stored_grid() {
        // 1000 wide, split at 600: 600 * sqrt(2) by 400 * sqrt(2).
        assert_eq!(fuji_upright_size(1000, 600), (848, 565));
        // The longer side always comes first.
        assert_eq!(fuji_upright_size(1000, 400), (848, 565));
        // A split past the width leaves nothing, which `Frame::new` rejects.
        assert_eq!(fuji_upright_size(100, 150).1, 0);
    }
}
