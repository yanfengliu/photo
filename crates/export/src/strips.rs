//! Rendering the export in horizontal strips, so memory stays bounded
//! whatever the picture's size.
//!
//! The whole export is one view of the photo's output at the target size,
//! rendered directly through the pipeline (no full-size render shrunk
//! afterwards). Each strip is a [`View::tile`] of that view; contract R1
//! makes a tile the pixels of that region of a single render, within 1/255,
//! so the strips join without seams. (In practice they are bit for bit on
//! Apple silicon; on x86-64 Windows a value at a strip's edge has come out
//! one 16-bit level apart.) Output sharpening needs a few rows above and
//! below, so each strip is rendered with that many extra rows, which are
//! the rows a single render would have there.

use photo_core::colorimetry::RgbSpace;
use photo_core::develop::DevelopSettings;
use photo_core::geom::Rect;
use photo_develop::{CancelToken, Quality, RenderRequest, Renderer, Source};
use photo_imaging::View;

use crate::item::ExportError;
use crate::sharpen::Sharpener;
use crate::watermark::WatermarkLayer;

/// Rendered pixels per strip, about two megapixels: enough rows that the
/// padding the spatial stages need is a small overhead, few enough that the
/// pipeline's working buffers for a strip stay near 150 MB (measured: a
/// 24 MP JPEG export peaks 140 MB above the source it renders, at the same
/// speed as four-megapixel strips that peak 270 MB above it).
const STRIP_PIXELS: u32 = 1 << 21;

/// Rows of a strip are a multiple of this, so a JPEG encoder's 8-row block
/// rows never straddle two strips.
pub(crate) const ROW_GRANULE: u32 = 8;

/// The final pixels of an export, strip by strip: interleaved 16-bit RGB in
/// the export's colour space.
pub(crate) trait StripSource {
    /// Width and height of the picture.
    fn size(&self) -> (u32, u32);

    /// Rows per strip: a multiple of [`ROW_GRANULE`]. Strip `n` covers rows
    /// `n * strip_rows()` up to the next strip or the picture's end.
    fn strip_rows(&self) -> u32;

    /// Rows `first..first + count` of the picture.
    fn rows(&mut self, first: u32, count: u32) -> Result<Vec<u16>, ExportError>;
}

/// The export of one photo, rendered on demand.
pub(crate) struct Producer<'a> {
    source: &'a Source,
    settings: &'a DevelopSettings,
    space: RgbSpace,
    view: View,
    size: (u32, u32),
    strip_rows: u32,
    renderer: Renderer,
    sharpener: Option<Sharpener>,
    watermark: Option<WatermarkLayer>,
    cancel: &'a CancelToken,
}

impl<'a> Producer<'a> {
    /// The export of `source` with `settings` at `size`, encoded in
    /// `space`, finished with output sharpening and a watermark when given.
    pub(crate) fn new(
        source: &'a Source,
        settings: &'a DevelopSettings,
        space: RgbSpace,
        size: (u32, u32),
        sharpener: Option<Sharpener>,
        watermark: Option<WatermarkLayer>,
        cancel: &'a CancelToken,
    ) -> Self {
        let (output_width, output_height) = photo_develop::output_size(source, settings);
        let roi = Rect::from_size(output_width as f32, output_height as f32);
        let rows = (STRIP_PIXELS / size.0.max(1)) / ROW_GRANULE * ROW_GRANULE;
        Self {
            source,
            settings,
            space,
            view: View::new(roi, size.0, size.1),
            size,
            strip_rows: rows.max(ROW_GRANULE),
            renderer: Renderer::new(),
            sharpener,
            watermark,
            cancel,
        }
    }

    /// The same producer with `rows` rows per strip (rounded up to a
    /// multiple of [`ROW_GRANULE`]).
    #[cfg(test)]
    pub(crate) fn with_strip_rows(mut self, rows: u32) -> Self {
        self.strip_rows = rows.div_ceil(ROW_GRANULE).max(1) * ROW_GRANULE;
        self
    }

    /// Rows `first..last` of the view, rendered through the pipeline.
    fn render(&mut self, first: u32, last: u32) -> Result<Vec<u16>, ExportError> {
        let request = RenderRequest {
            source: self.source,
            settings: self.settings,
            view: self.view.tile(0, first, self.size.0, last - first),
            quality: Quality::Full,
            space: self.space,
        };
        self.renderer
            .render_rgb16(&request, self.cancel)
            .map(|image| image.pixels)
            .ok_or(ExportError::Cancelled)
    }
}

impl StripSource for Producer<'_> {
    fn size(&self) -> (u32, u32) {
        self.size
    }

    fn strip_rows(&self) -> u32 {
        self.strip_rows
    }

    fn rows(&mut self, first: u32, count: u32) -> Result<Vec<u16>, ExportError> {
        if self.cancel.is_cancelled() {
            return Err(ExportError::Cancelled);
        }
        let (width, height) = self.size;
        let end = (first + count).min(height);
        let mut rows = match self.sharpener {
            Some(sharpener) => {
                let reach = Sharpener::REACH;
                let top = first.saturating_sub(reach);
                let bottom = (end + reach).min(height);
                let band = self.render(top, bottom)?;
                sharpener.apply(&band, width, top, first, end)
            }
            None => self.render(first, end)?,
        };
        if let Some(watermark) = &self.watermark {
            watermark.draw(&mut rows, width, first);
        }
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use photo_core::image::{SourceColor, SourceImage};

    use super::*;
    use crate::settings::{OutputSharpening, Watermark};

    /// A small scene with texture, edges and colour.
    fn source(width: u32, height: u32) -> Source {
        let scene = photo_testkit::scene_linear(width, height);
        let pixels = scene
            .pixels
            .iter()
            .flat_map(|rgb| rgb.map(|v| (v.clamp(0.0, 1.0) * 65535.0).round() as u16))
            .collect();
        let image = SourceImage::new(width, height, pixels, SourceColor::Rgb(RgbSpace::Srgb));
        Source::new(image.unwrap())
    }

    fn settings() -> DevelopSettings {
        let mut settings = DevelopSettings::default();
        settings.tone.contrast = 25.0;
        settings.presence.clarity = 30.0;
        settings.presence.texture = 40.0;
        settings.detail.sharpening.amount = 50.0;
        settings.detail.noise_reduction.luminance = 20.0;
        settings.effects.vignette.amount = -30.0;
        settings.effects.grain.amount = 20.0;
        settings
    }

    fn all_rows(producer: &mut Producer) -> Vec<u16> {
        let (_, height) = producer.size();
        let step = producer.strip_rows();
        let mut out = Vec::new();
        for first in (0..height).step_by(step as usize) {
            out.extend(producer.rows(first, step).unwrap());
        }
        out
    }

    /// Contract R1 as the export uses it: strips of a reduced view, joined,
    /// are the single render of that view, bit for bit.
    #[test]
    fn strips_join_into_exactly_the_single_render() {
        let source = source(480, 330);
        let settings = settings();
        let cancel = CancelToken::new();
        let size = (301, 207);
        let producer = Producer::new(
            &source,
            &settings,
            RgbSpace::Srgb,
            size,
            None,
            None,
            &cancel,
        );
        let whole = Renderer::new()
            .render_rgb16(
                &RenderRequest {
                    source: &source,
                    settings: &settings,
                    view: producer.view,
                    quality: Quality::Full,
                    space: RgbSpace::Srgb,
                },
                &cancel,
            )
            .unwrap();
        let mut strips = producer.with_strip_rows(16);
        assert!(strips.strip_rows() < size.1 / 4, "several strips");
        assert_eq!(all_rows(&mut strips), whole.pixels);
    }

    /// Output sharpening and the watermark read neighbouring rows and
    /// absolute positions; strips must not change what they do.
    #[test]
    fn finishing_is_the_same_in_strips_and_in_one_piece() {
        let source = source(400, 300);
        let settings = settings();
        let cancel = CancelToken::new();
        let size = (400, 300);
        let watermark = Watermark {
            text: "Strips & seams".into(),
            anchor: crate::settings::Anchor::Left,
            size: 9.0,
            ..Watermark::default()
        };
        let make = |rows: u32| {
            Producer::new(
                &source,
                &settings,
                RgbSpace::DisplayP3,
                size,
                Sharpener::new(OutputSharpening::High),
                WatermarkLayer::new(&watermark, size, RgbSpace::DisplayP3),
                &cancel,
            )
            .with_strip_rows(rows)
        };
        let single = all_rows(&mut make(size.1));
        assert_eq!(all_rows(&mut make(8)), single);
        assert_eq!(all_rows(&mut make(24)), single);
    }

    #[test]
    fn strips_hold_about_two_megapixels_in_whole_block_rows() {
        let source = source(64, 48);
        let settings = DevelopSettings::default();
        let cancel = CancelToken::new();
        for width in [64u32, 6000, 9504, 200_000] {
            let producer = Producer::new(
                &source,
                &settings,
                RgbSpace::Srgb,
                (width, 64),
                None,
                None,
                &cancel,
            );
            let rows = producer.strip_rows();
            assert_eq!(rows % ROW_GRANULE, 0);
            assert!(
                rows * width <= STRIP_PIXELS || rows == ROW_GRANULE,
                "{width}"
            );
        }
    }

    #[test]
    fn a_cancelled_token_stops_the_next_strip() {
        let source = source(64, 48);
        let settings = DevelopSettings::default();
        let cancel = CancelToken::new();
        let mut producer = Producer::new(
            &source,
            &settings,
            RgbSpace::Srgb,
            (64, 48),
            None,
            None,
            &cancel,
        );
        assert!(producer.rows(0, 8).is_ok());
        cancel.cancel();
        assert!(matches!(producer.rows(8, 8), Err(ExportError::Cancelled)));
    }
}
