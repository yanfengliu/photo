//! Where the photo sits on the Develop canvas, and which part of it the
//! engine is asked to render.
//!
//! Everything here is in physical pixels: the canvas box, the photo's place
//! and the scale (screen pixels per full-resolution output pixel). The
//! photo's corner is kept on a whole screen pixel and, at the fixed zooms,
//! the part asked of the engine starts on the same grid, so a rendered frame
//! lands on the screen's pixels one for one.

use egui::{Pos2, Rect, Vec2, pos2, vec2};
use photo_core::geom;
use photo_engine::CanvasView;

/// How the canvas fits the photo.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum CanvasZoom {
    /// The whole photo, as large as fits.
    #[default]
    Fit,
    /// The canvas filled, the overflow cropped (drag to pan).
    Fill,
    /// One screen pixel per photo pixel.
    OneToOne,
    /// Two screen pixels per photo pixel.
    TwoToOne,
    /// Four screen pixels per photo pixel.
    FourToOne,
}

impl CanvasZoom {
    /// Every zoom, from the farthest to the closest.
    pub const ALL: [Self; 5] = [
        Self::Fit,
        Self::Fill,
        Self::OneToOne,
        Self::TwoToOne,
        Self::FourToOne,
    ];

    /// What the toolbar calls the zoom.
    pub fn label(self) -> &'static str {
        match self {
            Self::Fit => "Fit",
            Self::Fill => "Fill",
            Self::OneToOne => "1:1",
            Self::TwoToOne => "2:1",
            Self::FourToOne => "4:1",
        }
    }

    /// Screen pixels per photo pixel, for the zooms that fix it.
    pub fn fixed_scale(self) -> Option<f32> {
        match self {
            Self::Fit | Self::Fill => None,
            Self::OneToOne => Some(1.0),
            Self::TwoToOne => Some(2.0),
            Self::FourToOne => Some(4.0),
        }
    }
}

/// Room kept around a fitted photo, in points.
pub(crate) const FIT_MARGIN: f32 = 16.0;

/// The canvas's zoom and where it looks.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct CanvasState {
    pub(crate) zoom: CanvasZoom,
    /// The zoom Space and Z go back to from a closer one.
    pub(crate) fitted: CanvasZoom,
    /// The point of the photo at the centre of the canvas, as fractions of
    /// its width and height. Fractions, so the place survives a change of
    /// the photo's size (the embedded preview giving way to the decoded
    /// file, a crop) and the move to the next photo.
    pub(crate) center: Vec2,
}

impl Default for CanvasState {
    fn default() -> Self {
        Self {
            zoom: CanvasZoom::Fit,
            fitted: CanvasZoom::Fit,
            center: vec2(0.5, 0.5),
        }
    }
}

impl CanvasState {
    /// Choose a zoom from the toolbar, keeping the centre.
    pub(crate) fn set(&mut self, zoom: CanvasZoom) {
        if zoom.fixed_scale().is_none() {
            self.fitted = zoom;
        }
        self.zoom = zoom;
    }

    /// Space or Z, or a click: between the fitted zoom and 1:1, keeping the
    /// point under `pointer` (physical pixels) where it is. Without a
    /// pointer over the photo the centre stays.
    pub(crate) fn toggle_at(&mut self, placement: &Placement, pointer: Option<Pos2>) {
        let target = if self.zoom.fixed_scale().is_some() {
            self.fitted
        } else {
            CanvasZoom::OneToOne
        };
        self.zoom_to(target, placement, pointer);
    }

    /// Go to `zoom`, keeping the point under `pointer` where it is.
    pub(crate) fn zoom_to(
        &mut self,
        zoom: CanvasZoom,
        placement: &Placement,
        pointer: Option<Pos2>,
    ) {
        let anchor = pointer.filter(|pointer| placement.photo().contains(*pointer));
        let next = Placement::new(zoom, self.center, placement.output, placement.canvas);
        if let Some(pointer) = anchor {
            let held = placement.to_output(pointer);
            // Put `held` under the pointer at the new scale.
            let origin = pointer - held.to_vec2() * next.scale;
            let center = (next.canvas.center() - origin) / next.scale / next.output;
            self.center = center;
        }
        self.set(zoom);
        let settled = Placement::new(self.zoom, self.center, placement.output, placement.canvas);
        self.center = settled.center();
    }

    /// Move the photo by `delta` physical pixels (a drag or a scroll).
    pub(crate) fn pan(&mut self, placement: &Placement, delta: Vec2) {
        if delta == Vec2::ZERO {
            return;
        }
        let moved = self.center - delta / placement.scale / placement.output;
        let settled = Placement::new(self.zoom, moved, placement.output, placement.canvas);
        self.center = settled.center();
    }
}

/// Where the photo is on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct Placement {
    /// The canvas box, in physical pixels.
    pub(crate) canvas: Rect,
    /// The photo's full-resolution output size, in pixels.
    pub(crate) output: Vec2,
    /// Screen pixels per output pixel.
    pub(crate) scale: f32,
    /// Where output `(0, 0)` is on screen, on a whole pixel.
    pub(crate) origin: Pos2,
    /// Whether the photo is fitted, so the engine renders the whole of it.
    pub(crate) fitted: bool,
}

impl Placement {
    /// The photo of `output` pixels at `zoom` in `canvas` (physical
    /// pixels), the point `center` (fractions) at the middle where the
    /// photo is larger than the canvas; a smaller photo is centred.
    pub(crate) fn new(zoom: CanvasZoom, center: Vec2, output: Vec2, canvas: Rect) -> Self {
        let output = output.max(Vec2::splat(1.0));
        let room = canvas.size().max(Vec2::splat(1.0));
        let fitted_room = (room - Vec2::splat(2.0 * FIT_MARGIN)).max(room * 0.5);
        let scale = match zoom.fixed_scale() {
            Some(scale) => scale,
            None if zoom == CanvasZoom::Fill => (room.x / output.x).max(room.y / output.y),
            None => (fitted_room.x / output.x).min(fitted_room.y / output.y),
        };
        let size = output * scale;
        let wanted = canvas.center() - center * size;
        let axis = |wanted: f32, low: f32, high: f32, extent: f32| {
            if extent <= high - low {
                (low + high - extent) / 2.0
            } else {
                wanted.clamp(high - extent, low)
            }
        };
        let origin = pos2(
            axis(wanted.x, canvas.left(), canvas.right(), size.x).round(),
            axis(wanted.y, canvas.top(), canvas.bottom(), size.y).round(),
        );
        Self {
            canvas,
            output,
            scale,
            origin,
            fitted: zoom == CanvasZoom::Fit,
        }
    }

    /// The whole photo's rect on screen.
    pub(crate) fn photo(&self) -> Rect {
        Rect::from_min_size(self.origin, self.output * self.scale)
    }

    /// The output point under a screen position.
    pub(crate) fn to_output(&self, screen: Pos2) -> Pos2 {
        ((screen - self.origin) / self.scale).to_pos2()
    }

    /// Where an output rectangle is on screen.
    pub(crate) fn on_screen(&self, roi: geom::Rect) -> Rect {
        Rect::from_min_max(
            self.origin + vec2(roi.x0, roi.y0) * self.scale,
            self.origin + vec2(roi.x1, roi.y1) * self.scale,
        )
    }

    /// The point of the photo at the centre of the canvas, as fractions.
    pub(crate) fn center(&self) -> Vec2 {
        (self.canvas.center() - self.origin) / self.scale / self.output
    }

    /// The part of the output that is in view, in output pixels.
    pub(crate) fn visible(&self) -> geom::Rect {
        let seen = self.canvas.intersect(self.photo());
        let min = self.to_output(seen.min);
        let max = self.to_output(seen.max);
        geom::Rect::new(
            min.x.max(0.0),
            min.y.max(0.0),
            max.x.min(self.output.x),
            max.y.min(self.output.y),
        )
    }

    /// The part of the photo in view as fractions of its size; `None` when
    /// all of it is.
    pub(crate) fn seen_fraction(&self) -> Option<Rect> {
        let seen = self.visible();
        let whole = seen.x0 <= 0.5
            && seen.y0 <= 0.5
            && seen.x1 >= self.output.x - 0.5
            && seen.y1 >= self.output.y - 0.5;
        (!whole).then(|| {
            Rect::from_min_max(
                pos2(seen.x0 / self.output.x, seen.y0 / self.output.y),
                pos2(seen.x1 / self.output.x, seen.y1 / self.output.y),
            )
        })
    }

    /// What to ask the engine to render: the whole photo for a fitted
    /// canvas, otherwise the part in view at the canvas's scale.
    pub(crate) fn canvas_view(&self) -> CanvasView {
        if self.fitted {
            let fitted = self.photo().intersect(self.canvas);
            return CanvasView::Fit {
                width: fitted.width().round().max(1.0) as u32,
                height: fitted.height().round().max(1.0) as u32,
            };
        }
        CanvasView::Region {
            roi: self.visible(),
            scale: self.scale,
        }
    }
}

/// `rect` (points) in physical pixels, its edges on whole pixels.
pub(crate) fn to_pixels(rect: Rect, pixels_per_point: f32) -> Rect {
    Rect::from_min_max(
        (rect.min.to_vec2() * pixels_per_point).round().to_pos2(),
        (rect.max.to_vec2() * pixels_per_point).round().to_pos2(),
    )
}

/// `rect` (physical pixels) in points.
pub(crate) fn to_points(rect: Rect, pixels_per_point: f32) -> Rect {
    Rect::from_min_max(
        (rect.min.to_vec2() / pixels_per_point).to_pos2(),
        (rect.max.to_vec2() / pixels_per_point).to_pos2(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn canvas() -> Rect {
        Rect::from_min_size(pos2(100.0, 50.0), vec2(1000.0, 800.0))
    }

    const OUTPUT: Vec2 = vec2(4000.0, 2000.0);

    #[test]
    fn fit_shows_the_whole_photo_inside_a_margin_and_asks_for_that_box() {
        let placement = Placement::new(CanvasZoom::Fit, vec2(0.5, 0.5), OUTPUT, canvas());
        let photo = placement.photo();
        assert!(canvas().contains_rect(photo));
        assert!((photo.width() - (1000.0 - 2.0 * FIT_MARGIN)).abs() < 1.0);
        assert!((photo.center() - canvas().center()).length() < 1.0);
        assert_eq!(placement.seen_fraction(), None);
        match placement.canvas_view() {
            CanvasView::Fit { width, height } => {
                assert_eq!(width, photo.width().round() as u32);
                assert_eq!(height, photo.height().round() as u32);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn fill_covers_the_canvas_and_renders_only_what_is_in_view() {
        let placement = Placement::new(CanvasZoom::Fill, vec2(0.5, 0.5), OUTPUT, canvas());
        assert!(placement.photo().contains_rect(canvas()));
        let CanvasView::Region { roi, scale } = placement.canvas_view() else {
            panic!("a region");
        };
        assert_eq!(scale, 0.4);
        assert!((roi.width() * scale - 1000.0).abs() < 0.5);
        assert!((roi.height() - 2000.0).abs() < 0.5);
        assert!(roi.x0 > 0.0 && roi.x1 < 4000.0, "the middle of the width");
    }

    #[test]
    fn one_to_one_puts_whole_output_pixels_on_whole_screen_pixels() {
        let center = vec2(0.3137, 0.7213);
        for zoom in [CanvasZoom::OneToOne, CanvasZoom::TwoToOne, CanvasZoom::FourToOne] {
            let placement = Placement::new(zoom, center, OUTPUT, canvas());
            assert_eq!(placement.origin, placement.origin.round());
            let CanvasView::Region { roi, scale } = placement.canvas_view() else {
                panic!("a region");
            };
            assert_eq!(Some(scale), zoom.fixed_scale());
            let screen = placement.on_screen(roi);
            assert_eq!(screen.min, screen.min.round(), "{zoom:?}");
            assert_eq!(screen, canvas(), "the whole canvas, nothing outside it");
        }
    }

    #[test]
    fn a_photo_cannot_be_panned_off_the_canvas() {
        let mut state = CanvasState::default();
        state.set(CanvasZoom::OneToOne);
        let placement = Placement::new(state.zoom, state.center, OUTPUT, canvas());
        state.pan(&placement, vec2(1.0e6, -1.0e6));
        let moved = Placement::new(state.zoom, state.center, OUTPUT, canvas());
        assert_eq!(moved.photo().left(), canvas().left());
        assert_eq!(moved.photo().bottom(), canvas().bottom());
    }

    #[test]
    fn a_small_photo_stays_centred_however_it_is_panned() {
        let small = vec2(300.0, 200.0);
        let mut state = CanvasState::default();
        state.set(CanvasZoom::OneToOne);
        let placement = Placement::new(state.zoom, state.center, small, canvas());
        state.pan(&placement, vec2(250.0, 90.0));
        let moved = Placement::new(state.zoom, state.center, small, canvas());
        assert!((moved.photo().center() - canvas().center()).length() <= 1.0);
        let CanvasView::Region { roi, .. } = moved.canvas_view() else {
            panic!("a region");
        };
        assert_eq!((roi.width(), roi.height()), (300.0, 200.0), "only the photo");
    }

    #[test]
    fn toggling_keeps_the_point_under_the_pointer_and_comes_back_to_the_fitted_zoom() {
        let mut state = CanvasState::default();
        let fitted = Placement::new(state.zoom, state.center, OUTPUT, canvas());
        let pointer = pos2(400.0, 300.0);
        let under = fitted.to_output(pointer);
        state.toggle_at(&fitted, Some(pointer));
        assert_eq!(state.zoom, CanvasZoom::OneToOne);
        let zoomed = Placement::new(state.zoom, state.center, OUTPUT, canvas());
        let now = zoomed.to_output(pointer);
        assert!((now - under).length() < 1.5, "{under:?} -> {now:?}");

        state.toggle_at(&zoomed, Some(pointer));
        assert_eq!(state.zoom, CanvasZoom::Fit);
        state.set(CanvasZoom::Fill);
        state.set(CanvasZoom::TwoToOne);
        let deep = Placement::new(state.zoom, state.center, OUTPUT, canvas());
        state.toggle_at(&deep, None);
        assert_eq!(state.zoom, CanvasZoom::Fill, "back to the zoom it came from");
    }

    #[test]
    fn the_part_in_view_is_reported_as_fractions_only_when_part_is_hidden() {
        let placement = Placement::new(CanvasZoom::OneToOne, vec2(0.5, 0.5), OUTPUT, canvas());
        let seen = placement.seen_fraction().unwrap();
        assert!((seen.width() - 0.25).abs() < 1.0e-3, "{seen:?}");
        assert!((seen.center().x - 0.5).abs() < 1.0e-3);
    }

    #[test]
    fn points_and_pixels_convert_both_ways() {
        let rect = Rect::from_min_max(pos2(10.25, 20.0), pos2(110.0, 70.5));
        let pixels = to_pixels(rect, 2.0);
        assert_eq!(pixels, Rect::from_min_max(pos2(21.0, 40.0), pos2(220.0, 141.0)));
        assert_eq!(to_points(pixels, 2.0).min, pos2(10.5, 20.0));
    }
}
