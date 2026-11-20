//! How each icon is drawn.
//!
//! Every icon is designed on a 16 x 16 unit square and scaled to the rect it
//! is painted in, so one design serves every size and pixel density.

use std::f32::consts::{PI, TAU};

use egui::epaint::{Mesh, Shape};
use egui::{Color32, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2, vec2};

use super::Icon;

/// A point on the 16-unit design grid.
type P = [f32; 2];

impl Icon {
    /// Paint the icon centred in `rect`, as large as fits, in `color`.
    pub fn paint(self, painter: &Painter, rect: Rect, color: Color32) {
        let pen = Pen::new(painter, rect, color);
        match self {
            Self::Grid => {
                for row in 0..3 {
                    for column in 0..3 {
                        let (x, y) = (1.0 + 5.0 * column as f32, 1.0 + 5.0 * row as f32);
                        pen.block(x, y, x + 4.0, y + 4.0);
                    }
                }
            }
            Self::Loupe => pen.frame(2.5, 3.5, 13.5, 12.5),
            Self::Compare => {
                pen.frame(1.5, 3.5, 7.5, 12.5);
                pen.frame(8.5, 3.5, 14.5, 12.5);
            }
            Self::Survey => {
                pen.frame(1.5, 2.5, 8.5, 13.5);
                pen.frame(10.5, 2.5, 14.5, 7.5);
                pen.frame(10.5, 8.5, 14.5, 13.5);
            }
            Self::FlagPick => {
                pen.line(&FLAG_POLE);
                pen.solid(&FLAG_CLOTH);
            }
            Self::FlagNone => {
                pen.line(&FLAG_POLE);
                pen.outline(&FLAG_CLOTH);
            }
            Self::FlagReject => {
                pen.line(&FLAG_POLE);
                pen.outline(&FLAG_CLOTH);
                pen.line(&[[9.5, 10.5], [13.5, 14.5]]);
                pen.line(&[[13.5, 10.5], [9.5, 14.5]]);
            }
            Self::StarFilled => pen.solid(&star()),
            Self::StarEmpty => pen.outline(&star()),
            Self::RotateCw => rotate_arrow(&pen, false),
            Self::RotateCcw => rotate_arrow(&pen, true),
            Self::Flip => {
                pen.line(&[[8.0, 2.0], [8.0, 14.0]]);
                pen.outline(&[[6.0, 4.5], [6.0, 11.5], [2.0, 11.5]]);
                pen.solid(&[[10.0, 4.5], [10.0, 11.5], [14.0, 11.5]]);
            }
            Self::Crop => {
                pen.line(&[[4.5, 1.5], [4.5, 11.5], [14.5, 11.5]]);
                pen.line(&[[1.5, 4.5], [11.5, 4.5], [11.5, 14.5]]);
            }
            Self::Heal => bandage(&pen),
            Self::LinearMask => {
                pen.frame(2.5, 2.5, 13.5, 13.5);
                pen.line(&[[2.5, 7.0], [13.5, 7.0]]);
                pen.block(2.5, 10.0, 13.5, 13.5);
            }
            Self::RadialMask => {
                pen.ring(8.0, 8.0, 5.5);
                pen.dot(8.0, 8.0, 1.6);
            }
            Self::Brush => {
                pen.line(&[[13.5, 2.5], [8.0, 8.0]]);
                pen.solid(&[[6.6, 7.2], [8.8, 9.4], [6.2, 12.6], [2.6, 13.4], [3.4, 9.8]]);
            }
            Self::Eye => {
                let lid = |sign: f32, step: usize| {
                    let t = step as f32 / 12.0;
                    [1.5 + 13.0 * t, 8.0 + sign * 4.2 * (PI * t).sin()]
                };
                let upper = (0..=12).map(|step| lid(-1.0, step));
                let lower = (1..12).rev().map(|step| lid(1.0, step));
                let outline: Vec<P> = upper.chain(lower).collect();
                pen.outline(&outline);
                pen.dot(8.0, 8.0, 2.0);
            }
            Self::Trash => {
                pen.line(&[[2.5, 4.5], [13.5, 4.5]]);
                pen.line(&[[6.5, 4.5], [6.5, 2.5], [9.5, 2.5], [9.5, 4.5]]);
                pen.line(&[[4.0, 4.5], [4.8, 13.5], [11.2, 13.5], [12.0, 4.5]]);
                pen.line(&[[6.5, 7.0], [6.5, 11.0]]);
                pen.line(&[[9.5, 7.0], [9.5, 11.0]]);
            }
            Self::Plus => {
                pen.line(&[[8.0, 3.0], [8.0, 13.0]]);
                pen.line(&[[3.0, 8.0], [13.0, 8.0]]);
            }
            Self::Minus => pen.line(&[[3.0, 8.0], [13.0, 8.0]]),
            Self::ChevronLeft => pen.line(&[[10.0, 3.5], [5.5, 8.0], [10.0, 12.5]]),
            Self::ChevronRight => pen.line(&[[6.0, 3.5], [10.5, 8.0], [6.0, 12.5]]),
            Self::ChevronUp => pen.line(&[[3.5, 10.0], [8.0, 5.5], [12.5, 10.0]]),
            Self::ChevronDown => pen.line(&[[3.5, 6.0], [8.0, 10.5], [12.5, 6.0]]),
            Self::ChevronsLeft => {
                pen.line(&[[8.0, 3.5], [3.5, 8.0], [8.0, 12.5]]);
                pen.line(&[[13.0, 3.5], [8.5, 8.0], [13.0, 12.5]]);
            }
            Self::ChevronsRight => {
                pen.line(&[[3.0, 3.5], [7.5, 8.0], [3.0, 12.5]]);
                pen.line(&[[8.0, 3.5], [12.5, 8.0], [8.0, 12.5]]);
            }
            Self::BeforeAfter => {
                pen.frame(2.5, 3.5, 13.5, 12.5);
                pen.block(8.0, 3.5, 13.5, 12.5);
                pen.line(&[[8.0, 1.5], [8.0, 14.5]]);
            }
            Self::Picker => {
                // An eyedropper at 45 degrees: the bulb, the collar, the
                // glass and the drop at its tip.
                pen.solid(&[[11.0, 1.8], [14.2, 5.0], [12.4, 6.8], [9.2, 3.6]]);
                pen.line(&[[8.2, 4.4], [11.6, 7.8]]);
                pen.line(&[[10.0, 6.2], [3.8, 12.4], [3.2, 12.8]]);
                pen.dot(2.6, 13.4, 1.1);
            }
            Self::Zoom => {
                magnifier(&pen);
                pen.line(&[[7.0, 5.0], [7.0, 9.0]]);
                pen.line(&[[5.0, 7.0], [9.0, 7.0]]);
            }
            Self::Search => magnifier(&pen),
            Self::Import => {
                pen.line(&[[8.0, 2.0], [8.0, 10.0]]);
                pen.line(&[[4.8, 6.8], [8.0, 10.0], [11.2, 6.8]]);
                pen.line(&TRAY);
            }
            Self::Export => {
                pen.line(&[[8.0, 10.5], [8.0, 2.5]]);
                pen.line(&[[4.8, 5.7], [8.0, 2.5], [11.2, 5.7]]);
                pen.line(&TRAY);
            }
            Self::Filter => {
                pen.outline(&[
                    [2.5, 3.0],
                    [13.5, 3.0],
                    [9.5, 8.0],
                    [9.5, 13.0],
                    [6.5, 11.5],
                    [6.5, 8.0],
                ]);
            }
            Self::Lock => {
                pen.frame(3.5, 7.5, 12.5, 13.5);
                let mut shackle = vec![[5.5, 7.5]];
                shackle.extend(arc([8.0, 5.5], 2.5, 180.0, 360.0));
                shackle.push([10.5, 7.5]);
                pen.line(&shackle);
            }
            Self::Unlock => {
                pen.frame(3.5, 7.5, 12.5, 13.5);
                let mut shackle = vec![[5.5, 7.5]];
                shackle.extend(arc([8.0, 5.5], 2.5, 180.0, 360.0));
                pen.line(&shackle);
            }
            Self::Warning => {
                pen.outline(&[[8.0, 2.3], [14.3, 13.5], [1.7, 13.5]]);
                pen.line(&[[8.0, 6.3], [8.0, 10.0]]);
                pen.dot(8.0, 11.8, 0.8);
            }
            Self::Folder => pen.outline(&[
                [1.5, 3.5],
                [6.0, 3.5],
                [7.5, 5.0],
                [14.5, 5.0],
                [14.5, 13.0],
                [1.5, 13.0],
            ]),
            Self::Collection => photo_stack(&pen),
            Self::SmartCollection => {
                photo_stack(&pen);
                pen.dot(9.5, 6.5, 1.7);
            }
            Self::Check => pen.line(&[[3.0, 8.5], [6.5, 12.0], [13.0, 4.5]]),
            Self::Close => {
                pen.line(&[[4.0, 4.0], [12.0, 12.0]]);
                pen.line(&[[12.0, 4.0], [4.0, 12.0]]);
            }
        }
    }
}

const FLAG_POLE: [P; 2] = [[4.5, 2.5], [4.5, 14.0]];
const FLAG_CLOTH: [P; 5] = [
    [4.5, 3.0],
    [13.0, 3.0],
    [10.5, 6.0],
    [13.0, 9.0],
    [4.5, 9.0],
];
const TRAY: [P; 4] = [[2.5, 10.5], [2.5, 13.5], [13.5, 13.5], [13.5, 10.5]];

/// A five-pointed star.
fn star() -> Vec<P> {
    (0..10)
        .map(|corner| {
            let radius = if corner % 2 == 0 { 6.6 } else { 2.9 };
            let angle = TAU * corner as f32 / 10.0 - TAU / 4.0;
            [8.0 + radius * angle.cos(), 8.7 + radius * angle.sin()]
        })
        .collect()
}

/// Points along a circular arc, by angle in degrees. Angles grow clockwise
/// on screen, from 0 at the right.
fn arc(center: P, radius: f32, from: f32, to: f32) -> impl Iterator<Item = P> {
    let steps = ((to - from).abs() / 15.0).ceil().max(1.0) as usize;
    (0..=steps).map(move |step| {
        let angle = (from + (to - from) * step as f32 / steps as f32).to_radians();
        [
            center[0] + radius * angle.cos(),
            center[1] + radius * angle.sin(),
        ]
    })
}

/// Three quarters of a circle ending in an arrow head. Clockwise unless
/// `mirrored`.
fn rotate_arrow(pen: &Pen<'_>, mirrored: bool) {
    let flip = |[x, y]: P| if mirrored { [16.0 - x, y] } else { [x, y] };
    let sweep: Vec<P> = arc([8.0, 8.5], 5.0, 150.0, 360.0).map(flip).collect();
    pen.line(&sweep);
    let head: Vec<P> = [[10.6, 6.6], [13.0, 9.4], [15.4, 6.6]].map(flip).to_vec();
    pen.line(&head);
}

/// A sticking plaster: a pill at 45 degrees with its pad marked.
fn bandage(pen: &Pen<'_>) {
    let turn = |[x, y]: P| {
        let (sin, cos) = (-TAU / 8.0).sin_cos();
        [8.0 + x * cos - y * sin, 8.0 + x * sin + y * cos]
    };
    let pill: Vec<P> = arc([3.6, 0.0], 2.8, -90.0, 90.0)
        .chain(arc([-3.6, 0.0], 2.8, 90.0, 270.0))
        .map(turn)
        .collect();
    pen.outline(&pill);
    for x in [-1.7, 1.7] {
        pen.line(&[turn([x, -2.8]), turn([x, 2.8])]);
    }
}

/// Two photos, one behind the other: a collection.
fn photo_stack(pen: &Pen<'_>) {
    pen.frame(4.5, 2.5, 14.5, 10.5);
    pen.line(&[[1.5, 5.5], [1.5, 13.5], [11.5, 13.5]]);
}

/// A lens and its handle.
fn magnifier(pen: &Pen<'_>) {
    pen.ring(7.0, 7.0, 4.5);
    pen.line(&[[10.4, 10.4], [14.0, 14.0]]);
}

/// Draws on the 16-unit design grid of one icon.
struct Pen<'a> {
    painter: &'a Painter,
    origin: Pos2,
    unit: f32,
    stroke: Stroke,
}

impl<'a> Pen<'a> {
    fn new(painter: &'a Painter, rect: Rect, color: Color32) -> Self {
        let side = rect.width().min(rect.height());
        let unit = side / 16.0;
        Self {
            painter,
            origin: rect.center() - Vec2::splat(side / 2.0),
            unit,
            stroke: Stroke::new((1.25 * unit).max(1.0), color),
        }
    }

    fn at(&self, [x, y]: P) -> Pos2 {
        self.origin + vec2(x, y) * self.unit
    }

    fn points(&self, points: &[P]) -> Vec<Pos2> {
        points.iter().map(|point| self.at(*point)).collect()
    }

    /// An open line through `points`.
    fn line(&self, points: &[P]) {
        let points = self.points(points);
        let right_angled = points
            .windows(2)
            .all(|pair| pair[0].x == pair[1].x || pair[0].y == pair[1].y);
        if right_angled {
            // Separate segments are snapped to the pixel grid, so they stay
            // crisp at any position; a path is not.
            for pair in points.windows(2) {
                self.painter.line_segment([pair[0], pair[1]], self.stroke);
            }
        } else {
            self.painter.add(Shape::line(points, self.stroke));
        }
    }

    /// A closed outline through `points`.
    fn outline(&self, points: &[P]) {
        self.painter
            .add(Shape::closed_line(self.points(points), self.stroke));
    }

    /// A filled shape. It may be concave as long as its centroid sees every
    /// corner (true of stars, flags and brush tips).
    fn solid(&self, points: &[P]) {
        let points = self.points(points);
        let sum = points
            .iter()
            .fold(Vec2::ZERO, |sum, point| sum + point.to_vec2());
        let center = (sum / points.len() as f32).to_pos2();
        let mut mesh = Mesh::default();
        mesh.colored_vertex(center, self.stroke.color);
        for point in &points {
            mesh.colored_vertex(*point, self.stroke.color);
        }
        let corners = points.len() as u32;
        for corner in 0..corners {
            mesh.add_triangle(0, 1 + corner, 1 + (corner + 1) % corners);
        }
        self.painter.add(Shape::mesh(mesh));
        // The mesh has hard edges; a hairline around it anti-aliases them.
        let hairline = Stroke::new(self.stroke.width * 0.6, self.stroke.color);
        self.painter.add(Shape::closed_line(points, hairline));
    }

    /// The outline of a rectangle with slightly rounded corners.
    fn frame(&self, left: f32, top: f32, right: f32, bottom: f32) {
        let rect = Rect::from_min_max(self.at([left, top]), self.at([right, bottom]));
        self.painter
            .rect_stroke(rect, self.unit, self.stroke, StrokeKind::Middle);
    }

    /// A filled rectangle.
    fn block(&self, left: f32, top: f32, right: f32, bottom: f32) {
        let rect = Rect::from_min_max(self.at([left, top]), self.at([right, bottom]));
        self.painter.rect_filled(rect, 0.0, self.stroke.color);
    }

    /// The outline of a circle.
    fn ring(&self, x: f32, y: f32, radius: f32) {
        self.painter
            .circle_stroke(self.at([x, y]), radius * self.unit, self.stroke);
    }

    /// A filled circle.
    fn dot(&self, x: f32, y: f32, radius: f32) {
        self.painter
            .circle_filled(self.at([x, y]), radius * self.unit, self.stroke.color);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_arc_starts_and_ends_where_asked() {
        let points: Vec<P> = arc([8.0, 8.0], 4.0, 0.0, 90.0).collect();
        let (first, last) = (points[0], points[points.len() - 1]);
        assert!((first[0] - 12.0).abs() < 1.0e-4 && (first[1] - 8.0).abs() < 1.0e-4);
        assert!((last[0] - 8.0).abs() < 1.0e-4 && (last[1] - 12.0).abs() < 1.0e-4);
        assert!(points.len() >= 7, "smooth enough to read as a curve");
    }

    #[test]
    fn the_star_has_ten_corners_inside_the_design_grid() {
        let star = star();
        assert_eq!(star.len(), 10);
        assert!(
            star.iter()
                .all(|[x, y]| (1.0..=15.0).contains(x) && (1.0..=15.5).contains(y))
        );
        // The first corner is the top point.
        assert!((star[0][0] - 8.0).abs() < 1.0e-4 && star[0][1] < 3.0);
    }

    #[test]
    fn every_icon_paints_something_and_stays_inside_its_rect() {
        let ctx = egui::Context::default();
        let rect = Rect::from_min_size(egui::pos2(100.0, 100.0), Vec2::splat(16.0));
        let output = ctx.run_ui(egui::RawInput::default(), |ui| {
            for icon in Icon::ALL {
                let layer = egui::LayerId::new(egui::Order::Middle, egui::Id::unique(icon));
                let painter = ui.ctx().layer_painter(layer);
                icon.paint(&painter, rect, Color32::WHITE);
                let mut shapes = 0;
                painter.for_each_shape(|clipped| {
                    shapes += 1;
                    let bounds = clipped.shape.visual_bounding_rect();
                    assert!(
                        rect.expand(1.0).contains_rect(bounds),
                        "{icon:?} paints outside its rect: {bounds:?}"
                    );
                });
                assert!(shapes > 0, "{icon:?} paints nothing");
            }
        });
        output.drop_without_applying_deltas();
    }
}
