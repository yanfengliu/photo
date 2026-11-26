//! The brush: strokes painted onto a mask, or with Alt held, rubbed out of
//! it. A stroke is the path of the brush's centre with the size, feather
//! and flow the panel had when it began; the outline of the brush follows
//! the pointer.

use egui::epaint::Shape;
use egui::{Color32, Painter, Pos2, Stroke};
use photo_core::develop::{AdjSpec, BrushStroke, MaskShape, SettingsGroup, Spot};
use photo_core::geom::Point;

use crate::app::develop::canvas::map::CanvasMap;
use crate::theme::palette;

/// The brush's radius: a percentage of the photo's longer side.
pub(crate) const SIZE: AdjSpec = AdjSpec {
    key: "brush_size",
    label: "Size",
    min: 0.1,
    max: 25.0,
    step: 0.1,
    decimals: 1,
    group: SettingsGroup::Masking,
};
pub(crate) const FEATHER: AdjSpec = AdjSpec {
    key: "brush_feather",
    label: "Feather",
    min: 0.0,
    max: 100.0,
    step: 1.0,
    decimals: 0,
    group: SettingsGroup::Masking,
};
pub(crate) const FLOW: AdjSpec = AdjSpec {
    key: "brush_flow",
    label: "Flow",
    min: 1.0,
    max: 100.0,
    step: 1.0,
    decimals: 0,
    group: SettingsGroup::Masking,
};

/// How far the brush moves between two points of a stroke, as a fraction
/// of its radius.
const SPACING: f32 = 0.2;

/// What the next stroke is painted with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) struct BrushOptions {
    /// The radius as a percentage of the photo's longer side.
    pub(crate) size: f32,
    pub(crate) feather: f32,
    pub(crate) flow: f32,
}

impl Default for BrushOptions {
    fn default() -> Self {
        Self {
            size: 3.0,
            feather: 50.0,
            flow: 100.0,
        }
    }
}

impl BrushOptions {
    /// The radius as the settings store it: a fraction of the longer side.
    pub(crate) fn radius(&self) -> f32 {
        (self.size / 100.0).clamp(Spot::MIN_RADIUS, Spot::MAX_RADIUS)
    }
}

/// Begin a stroke at `at` (normalized source) on a brush shape: painting,
/// or erasing. Returns false for a shape that is not a brush.
pub(crate) fn begin(shape: &mut MaskShape, options: &BrushOptions, at: Point, erase: bool) -> bool {
    let MaskShape::Brush { strokes } = shape else {
        return false;
    };
    strokes.push(BrushStroke {
        points: vec![at],
        radius: options.radius(),
        feather: options.feather,
        flow: options.flow,
        erase,
    });
    true
}

/// Carry the stroke being painted on to `at` (normalized source). Returns
/// whether a point was added: the brush has to have moved a part of its own
/// radius.
pub(crate) fn extend(shape: &mut MaskShape, map: &CanvasMap, at: Point) -> bool {
    let MaskShape::Brush { strokes } = shape else {
        return false;
    };
    let Some(stroke) = strokes.last_mut() else {
        return false;
    };
    let step = stroke.radius * map.long_edge() * SPACING;
    let moved = stroke
        .points
        .last()
        .is_none_or(|last| map.pixels(*last).distance(map.pixels(at)) >= step);
    if moved {
        stroke.points.push(at);
    }
    moved
}

/// The brush's outline under the pointer: its edge, and inside it where the
/// feather begins.
pub(crate) fn paint_cursor(painter: &Painter, map: &CanvasMap, at: Pos2, options: &BrushOptions) {
    let center = map.screen_to_source(at);
    let radius = options.radius() * map.long_edge();
    let edge = map.circle(center, radius);
    if edge.len() < 3 {
        return;
    }
    let shadow = Stroke::new(3.0, Color32::from_black_alpha(130));
    painter.add(Shape::closed_line(edge.clone(), shadow));
    painter.add(Shape::closed_line(edge, Stroke::new(1.0, palette::TEXT_STRONG)));
    let solid = 1.0 - options.feather / 100.0;
    if solid > 0.05 && solid < 0.95 {
        let core = map.circle(center, radius * solid);
        painter.add(Shape::closed_line(
            core,
            Stroke::new(1.0, Color32::from_white_alpha(110)),
        ));
    }
    painter.circle_filled(at, 1.5, palette::TEXT_STRONG);
}

#[cfg(test)]
mod tests {
    use egui::{pos2, vec2};
    use photo_core::develop::DevelopSettings;
    use photo_engine::Geometry;

    use super::*;
    use crate::app::develop::canvas::map::Display;
    use crate::app::develop::canvas::mapping::{CanvasZoom, Placement};

    fn map() -> CanvasMap {
        let geometry = Geometry::from_settings(1000, 500, &DevelopSettings::default());
        let canvas = egui::Rect::from_min_size(pos2(0.0, 0.0), vec2(1032.0, 532.0));
        let placement = Placement::new(CanvasZoom::Fit, vec2(0.5, 0.5), vec2(1000.0, 500.0), canvas);
        CanvasMap {
            ppp: 1.0,
            geometry,
            display: Display::Output(placement),
        }
    }

    #[test]
    fn a_stroke_takes_the_panels_brush_and_gains_points_as_the_brush_moves() {
        let map = map();
        let options = BrushOptions {
            size: 5.0,
            feather: 30.0,
            flow: 80.0,
        };
        let mut shape = MaskShape::Brush {
            strokes: Vec::new(),
        };
        assert!(begin(&mut shape, &options, Point::new(0.2, 0.5), true));
        // The radius is 50 pixels: a point every 10.
        assert!(!extend(&mut shape, &map, Point::new(0.205, 0.5)), "5 px on");
        assert!(extend(&mut shape, &map, Point::new(0.212, 0.5)), "12 px on");
        let MaskShape::Brush { strokes } = &shape else {
            panic!("brush");
        };
        let stroke = &strokes[0];
        assert_eq!(stroke.points.len(), 2);
        assert_eq!(
            (stroke.radius, stroke.feather, stroke.flow, stroke.erase),
            (0.05, 30.0, 80.0, true)
        );
        let mut gradient = MaskShape::Linear {
            start: Point::new(0.0, 0.0),
            end: Point::new(1.0, 1.0),
        };
        assert!(!begin(&mut gradient, &options, Point::new(0.5, 0.5), false));
    }

    #[test]
    fn the_radius_is_held_to_what_a_stroke_can_have() {
        let big = BrushOptions {
            size: 90.0,
            ..BrushOptions::default()
        };
        assert_eq!(big.radius(), Spot::MAX_RADIUS);
        assert_eq!(BrushOptions::default().radius(), 0.03);
    }
}
