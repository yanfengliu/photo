//! The canvas while the crop tool is open: the whole photo, straightened by
//! the crop's angle, with the rectangle drawn over it. Drag a corner or an
//! edge to resize, the inside to move; with the straighten tool armed, a
//! drag draws the line to level.
//!
//! The engine is asked for the frame as a region of the output that reaches
//! past the crop on every side, so the settings it renders are the crop's
//! own draft and nothing is committed to see the rest of the photo.

use egui::epaint::Shape;
use egui::{Color32, CursorIcon, Painter, Pos2, Rect, Response, Stroke, Ui, Vec2, pos2, vec2};
use photo_core::geom::{self, Point};
use photo_engine::{CanvasView, DevelopSession};

use super::aspect;
use super::rect::{self, Handle, Upright};
use super::{CropTool, Drag, Frame, set_angle, set_rect};
use crate::app::Cx;
use crate::app::develop::canvas::map::{Straight, frame_geometry, turned_half_extent};
use crate::app::develop::right::Editing;
use crate::theme::palette;

/// How near a corner the pointer takes it, in points.
const CORNER_REACH: f32 = 14.0;
/// How near an edge the pointer takes it, in points.
const EDGE_REACH: f32 = 8.0;
/// Screen pixels asked of the engine beyond the frame on every side, so a
/// frame rendered a moment before the rectangle moved still covers it.
const PAD: f32 = 48.0;

/// What to ask the engine to render while cropping: the whole frame,
/// straightened, as a region of the session's own output at `scale` screen
/// pixels per pixel, or fewer where that would make a frame with a side
/// over `limit` pixels (the most a texture can have). `None` when the
/// frame's centre has no place in the output (a transform beyond its
/// horizon).
pub(crate) fn view(session: &DevelopSession, scale: f32, limit: f32) -> Option<CanvasView> {
    let geometry = session.geometry()?;
    let settings = session.settings();
    let frame = frame_geometry(settings, geometry.source_size());
    let (width, height) = frame.frame_size();
    let size = vec2(width as f32, height as f32);
    let center = frame.out_to_src(Point::new(size.x / 2.0, size.y / 2.0));
    let center = geometry.src_to_out(center)?;
    let half = turned_half_extent(size, settings.crop.angle) + Vec2::splat(PAD / scale);
    let roi = geom::Rect::new(
        (center.x - half.x).floor(),
        (center.y - half.y).floor(),
        (center.x + half.x).ceil(),
        (center.y + half.y).ceil(),
    );
    let scale = scale.min(limit / roi.width()).min(limit / roi.height());
    Some(CanvasView::Region { roi, scale })
}

/// Where `upright` is on screen, in points.
pub(crate) fn on_screen(straight: &Straight, upright: Upright, ppp: f32) -> Rect {
    let rect = upright.rect();
    let corner = |at: Pos2| (straight.upright_to_screen(at.to_vec2()).to_vec2() / ppp).to_pos2();
    Rect::from_min_max(corner(rect.min), corner(rect.max))
}

/// The handle of `rect` (points) at `at`: a corner first, then an edge,
/// then the inside.
pub(crate) fn hit(rect: Rect, at: Pos2) -> Option<Handle> {
    let corner = Handle::RESIZE
        .into_iter()
        .filter(|handle| handle.is_corner())
        .find(|handle| handle.on(rect).distance(at) <= CORNER_REACH);
    if corner.is_some() {
        return corner;
    }
    let reach = rect.expand(EDGE_REACH);
    if !reach.contains(at) {
        return None;
    }
    let near = |value: f32, edge: f32| (value - edge).abs() <= EDGE_REACH;
    let handle = if near(at.x, rect.left()) {
        Handle { x: -1, y: 0 }
    } else if near(at.x, rect.right()) {
        Handle { x: 1, y: 0 }
    } else if near(at.y, rect.top()) {
        Handle { x: 0, y: -1 }
    } else if near(at.y, rect.bottom()) {
        Handle { x: 0, y: 1 }
    } else {
        Handle::MOVE
    };
    Some(handle)
}

/// Take the canvas's input for the crop tool.
pub(crate) fn interact(
    ui: &Ui,
    cx: &mut Cx<'_>,
    editing: &Editing,
    frame: &Frame,
    straight: &Straight,
    response: &Response,
) {
    let ppp = ui.ctx().pixels_per_point();
    let crop = editing.settings.crop;
    let angle = crop.angle;
    let upright = rect::upright_of(&crop, frame.size);
    let shown = on_screen(straight, upright, ppp);
    let pointer = response
        .interact_pointer_pos()
        .or_else(|| response.hover_pos());
    let pressed = ui.input(|input| input.pointer.press_origin());

    if cx.ui.develop.tools.crop.leveling {
        if response.hovered() {
            ui.ctx().set_cursor_icon(CursorIcon::Crosshair);
        }
        if response.drag_started()
            && let Some(from) = pressed
        {
            cx.ui.develop.tools.crop.line = Some((from, pointer.unwrap_or(from)));
        }
        if response.dragged()
            && let (Some(to), Some(line)) = (pointer, cx.ui.develop.tools.crop.line.as_mut())
        {
            line.1 = to;
        }
        if response.drag_stopped() {
            let tool = &mut cx.ui.develop.tools.crop;
            tool.leveling = false;
            if let Some((from, to)) = tool.line.take() {
                let frame_at = |at: Pos2| straight.to_frame((at.to_vec2() * ppp).to_pos2());
                if let Some(level) = rect::leveling_angle(frame_at(from), frame_at(to)) {
                    cx.ui.develop.tools.crop.turning = None;
                    set_angle(cx, editing, frame, level, true);
                }
            }
        }
        return;
    }

    let over = cx
        .ui
        .develop
        .tools
        .crop
        .drag
        .map(|drag| drag.handle)
        .or_else(|| pointer.and_then(|at| hit(shown, at)));
    if let Some(handle) = over.filter(|_| response.hovered() || response.dragged()) {
        ui.ctx().set_cursor_icon(handle.cursor());
    }
    if response.drag_started()
        && let Some(from) = pressed
        && let Some(handle) = hit(shown, from)
    {
        let fits = frame.fits(angle);
        // A rectangle that reaches off the photo (a transform was changed
        // since it was drawn) is first brought back onto it.
        let start = if fits(upright) {
            upright
        } else {
            rect::largest(
                upright.center,
                upright.aspect(),
                frame.min(),
                upright.size,
                &fits,
            )
            .unwrap_or(upright)
        };
        cx.ui.develop.tools.crop.drag = Some(Drag {
            handle,
            start,
            pressed: from,
            at: from,
        });
    }
    // Only when the pointer has moved: a pointer held still asks for
    // nothing.
    if response.dragged()
        && let (Some(drag), Some(at)) = (cx.ui.develop.tools.crop.drag.as_mut(), pointer)
        && drag.at != at
    {
        drag.at = at;
        let drag = *drag;
        let delta = (at - drag.pressed) * ppp / straight.scale;
        let ratio = aspect::locked_ratio(crop.constraint, frame.size);
        let fits = frame.fits(angle);
        let moved = rect::dragged(drag.start, drag.handle, delta, ratio, frame.min(), &fits);
        set_rect(cx, editing, frame, moved, angle);
    }
    if response.drag_stopped() {
        cx.ui.develop.tools.crop.drag = None;
    }
}

/// Hide what lies outside the photo: beyond its edges the engine repeats
/// the edge pixels, which is not part of the picture.
pub(crate) fn cover_outside(painter: &Painter, corners: [Pos2; 4]) {
    let middle = corners
        .iter()
        .fold(Vec2::ZERO, |sum, corner| sum + corner.to_vec2())
        / 4.0;
    let far = painter.clip_rect().size().length() * 2.0 + 1.0;
    for side in 0..4 {
        let (from, to) = (corners[side], corners[(side + 1) % 4]);
        let along = (to - from).normalized();
        let mut out = along.rot90();
        if out.dot(from.to_vec2() - middle) < 0.0 {
            out = -out;
        }
        let quad = vec![
            from - along * far,
            to + along * far,
            to + along * far + out * far,
            from - along * far + out * far,
        ];
        painter.add(Shape::convex_polygon(
            quad,
            palette::WORK_AREA,
            Stroke::NONE,
        ));
    }
}

/// Paint the rectangle over the photo: what it leaves out darkened, its
/// frame and handles, the thirds while it is dragged, a grid while the
/// photo is straightened, and the line being drawn.
pub(crate) fn paint(painter: &Painter, tool: &CropTool, rect: Rect) {
    let area = painter.clip_rect();
    let veil = Color32::from_black_alpha(150);
    for part in [
        Rect::from_min_max(area.min, pos2(area.right(), rect.top())),
        Rect::from_min_max(pos2(area.left(), rect.bottom()), area.max),
        Rect::from_min_max(
            pos2(area.left(), rect.top()),
            pos2(rect.left(), rect.bottom()),
        ),
        Rect::from_min_max(
            pos2(rect.right(), rect.top()),
            pos2(area.right(), rect.bottom()),
        ),
    ] {
        if part.is_positive() {
            painter.rect_filled(part, 0.0, veil);
        }
    }

    let guide = Stroke::new(1.0, Color32::from_white_alpha(110));
    if tool.straightening() {
        let step = (rect.width().min(rect.height()) / 8.0).max(12.0);
        let center = rect.center();
        let mut offset = 0.0;
        while offset < rect.width().max(rect.height()) / 2.0 {
            for x in [center.x - offset, center.x + offset] {
                if rect.x_range().contains(x) {
                    painter.vline(x, rect.y_range(), guide);
                }
            }
            for y in [center.y - offset, center.y + offset] {
                if rect.y_range().contains(y) {
                    painter.hline(rect.x_range(), y, guide);
                }
            }
            offset += step;
        }
    } else if tool.drag.is_some() {
        for third in [1.0 / 3.0, 2.0 / 3.0] {
            painter.vline(rect.left() + rect.width() * third, rect.y_range(), guide);
            painter.hline(rect.x_range(), rect.top() + rect.height() * third, guide);
        }
    }

    let dark = Stroke::new(1.0, Color32::from_black_alpha(160));
    let light = Stroke::new(1.0, palette::TEXT_STRONG);
    painter.rect_stroke(rect.expand(1.0), 0.0, dark, egui::StrokeKind::Outside);
    painter.rect_stroke(rect, 0.0, light, egui::StrokeKind::Outside);
    let grip = Stroke::new(3.0, palette::TEXT_STRONG);
    for handle in Handle::RESIZE {
        let at = handle.on(rect);
        if handle.is_corner() {
            let arm = vec2(-f32::from(handle.x), -f32::from(handle.y)) * 16.0;
            painter.line_segment([at, at + vec2(arm.x, 0.0)], grip);
            painter.line_segment([at, at + vec2(0.0, arm.y)], grip);
        } else {
            let along = vec2(f32::from(handle.y).abs(), f32::from(handle.x).abs()) * 11.0;
            painter.line_segment([at - along, at + along], grip);
        }
    }

    if let Some((from, to)) = tool.line {
        painter.line_segment([from, to], Stroke::new(3.0, Color32::from_black_alpha(170)));
        painter.line_segment([from, to], Stroke::new(1.5, palette::TEXT_STRONG));
        for end in [from, to] {
            painter.circle_filled(end, 3.5, palette::TEXT_STRONG);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_corner_wins_over_an_edge_and_an_edge_over_the_inside() {
        let rect = Rect::from_min_max(pos2(100.0, 100.0), pos2(500.0, 400.0));
        assert_eq!(hit(rect, pos2(104.0, 97.0)), Some(Handle { x: -1, y: -1 }));
        assert_eq!(hit(rect, pos2(498.0, 405.0)), Some(Handle { x: 1, y: 1 }));
        assert_eq!(hit(rect, pos2(300.0, 103.0)), Some(Handle { x: 0, y: -1 }));
        assert_eq!(hit(rect, pos2(495.0, 250.0)), Some(Handle { x: 1, y: 0 }));
        assert_eq!(hit(rect, pos2(300.0, 250.0)), Some(Handle::MOVE));
        assert_eq!(hit(rect, pos2(60.0, 250.0)), None);
        assert_eq!(hit(rect, pos2(300.0, 430.0)), None);
    }
}
