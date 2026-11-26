//! Where the photo's pixels are on the canvas: one mapping between the
//! screen and the decoded source, shared by the frames, the overlays and the
//! tools.
//!
//! The canvas shows the photo in one of two ways. Normally it is the output
//! (the photo as cropped and turned), placed as the zoom says
//! ([`Placement`]). While the crop tool is open it is the whole frame (the
//! photo turned and transformed but not cropped), straightened by the crop's
//! angle so that what will be level is level ([`Straight`]).
//!
//! Either way a source position has one place on screen, found through the
//! engine's [`Geometry`] and never through arithmetic of this crate's own.
//! Spots and masks, which live in source coordinates, are drawn through it;
//! and a frame rendered for an older version of the settings (a drag outruns
//! the renderer) is drawn through that version's geometry, so its pixels
//! land on the content they show.

use egui::epaint::{Mesh, Vertex};
use egui::{Color32, Painter, Pos2, Rect, Shape, TextureId, Vec2, pos2, vec2};
use photo_core::develop::{Crop, DevelopSettings};
use photo_core::geom::{self, Point};
use photo_engine::Geometry;

use super::mapping::{FIT_MARGIN, Placement, to_points};

/// Cells per side of the mesh a frame is warped with.
const MESH: usize = 8;

/// The whole texture.
const UV: Rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0));

/// `v` turned by `degrees`: positive takes x towards y, which on screen
/// (y down) is clockwise.
pub(crate) fn turned(v: Vec2, degrees: f32) -> Vec2 {
    let (sin, cos) = degrees.to_radians().sin_cos();
    vec2(v.x * cos - v.y * sin, v.x * sin + v.y * cos)
}

/// The geometry of `settings` without their crop, for a source of `source`
/// pixels: its output is the frame a crop rectangle is cut from.
pub(crate) fn frame_geometry(settings: &DevelopSettings, source: (u32, u32)) -> Geometry {
    Geometry::new(
        source.0,
        source.1,
        &settings.lens,
        settings.orientation,
        &settings.transform,
        &Crop::default(),
    )
}

/// Half the box a frame of `size` takes when turned by `angle` degrees.
pub(crate) fn turned_half_extent(size: Vec2, angle: f32) -> Vec2 {
    let (sin, cos) = angle.to_radians().sin_cos();
    let (sin, cos) = (sin.abs(), cos.abs());
    vec2(size.x * cos + size.y * sin, size.x * sin + size.y * cos) / 2.0
}

/// The straightened frame the crop tool works on: the frame turned about its
/// centre against the crop's angle, so the crop rectangle is upright.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Straight {
    /// Lens, orientation and transform without the crop.
    pub(crate) frame: Geometry,
    /// The straighten angle shown, in degrees.
    pub(crate) angle: f32,
    /// Screen pixels per frame pixel.
    pub(crate) scale: f32,
    /// Where the frame's centre is on screen, in physical pixels.
    pub(crate) center: Pos2,
}

impl Straight {
    /// The frame of `settings` for a source of `source` pixels, straightened
    /// by `angle` and fitted into `canvas` (physical pixels).
    pub(crate) fn new(
        settings: &DevelopSettings,
        source: (u32, u32),
        angle: f32,
        canvas: Rect,
    ) -> Self {
        let frame = frame_geometry(settings, source);
        let (width, height) = frame.frame_size();
        let half = turned_half_extent(vec2(width as f32, height as f32), angle);
        let room = canvas.size().max(Vec2::splat(1.0));
        let fitted = (room - Vec2::splat(2.0 * FIT_MARGIN)).max(room * 0.5);
        let scale = (fitted.x / (2.0 * half.x)).min(fitted.y / (2.0 * half.y));
        Self {
            frame,
            angle,
            scale,
            center: canvas.center().round(),
        }
    }

    /// The frame's size in pixels.
    pub(crate) fn size(&self) -> Vec2 {
        let (width, height) = self.frame.frame_size();
        vec2(width as f32, height as f32)
    }

    /// Where a frame position is on screen, in physical pixels.
    pub(crate) fn to_screen(&self, frame: Vec2) -> Pos2 {
        self.center + turned(frame - self.size() / 2.0, -self.angle) * self.scale
    }

    /// The frame position under a screen position (physical pixels).
    pub(crate) fn to_frame(&self, screen: Pos2) -> Vec2 {
        self.size() / 2.0 + turned((screen - self.center) / self.scale, self.angle)
    }

    /// Where a position of the straightened frame (frame pixels from the
    /// frame's centre, the crop rectangle upright) is on screen.
    pub(crate) fn upright_to_screen(&self, upright: Vec2) -> Pos2 {
        self.center + upright * self.scale
    }

    /// The box the turned frame takes on screen, in physical pixels.
    pub(crate) fn bounds(&self) -> Rect {
        let half = turned_half_extent(self.size(), self.angle) * self.scale;
        Rect::from_center_size(self.center, 2.0 * half)
    }
}

/// What the canvas shows the photo as.
#[derive(Clone, Debug)]
pub(crate) enum Display {
    /// The output, placed as the zoom says.
    Output(Placement),
    /// The crop tool's straightened frame.
    Straight(Straight),
}

/// The mapping between the screen and the open photo's source.
#[derive(Clone, Debug)]
pub(crate) struct CanvasMap {
    /// Physical pixels per point.
    pub(crate) ppp: f32,
    /// The geometry of the session's settings.
    pub(crate) geometry: Geometry,
    pub(crate) display: Display,
}

impl CanvasMap {
    /// The source's size in pixels.
    pub(crate) fn source_size(&self) -> Vec2 {
        let (width, height) = self.geometry.source_size();
        vec2(width as f32, height as f32)
    }

    /// The source's longer side in pixels: what a spot's or a brush's radius
    /// is a fraction of.
    pub(crate) fn long_edge(&self) -> f32 {
        self.source_size().max_elem()
    }

    /// Where a source position (pixels) is on screen, in points. `None`
    /// beyond the horizon of a perspective transform.
    pub(crate) fn source_to_screen(&self, source: Point) -> Option<Pos2> {
        let pixels = match &self.display {
            Display::Output(placement) => {
                let out = self.geometry.src_to_out(source)?;
                placement.origin + vec2(out.x, out.y) * placement.scale
            }
            Display::Straight(straight) => {
                let frame = straight.frame.src_to_out(source)?;
                straight.to_screen(vec2(frame.x, frame.y))
            }
        };
        let points = pixels.to_vec2() / self.ppp;
        (points.x.is_finite() && points.y.is_finite()).then(|| points.to_pos2())
    }

    /// The source position (pixels) under a screen position (points).
    pub(crate) fn screen_to_source(&self, screen: Pos2) -> Point {
        let pixels = (screen.to_vec2() * self.ppp).to_pos2();
        match &self.display {
            Display::Output(placement) => {
                let out = placement.to_output(pixels);
                self.geometry.out_to_src(Point::new(out.x, out.y))
            }
            Display::Straight(straight) => {
                let frame = straight.to_frame(pixels);
                straight.frame.out_to_src(Point::new(frame.x, frame.y))
            }
        }
    }

    /// How many source pixels one point of the screen spans at `at`.
    pub(crate) fn source_per_point(&self, at: Pos2) -> f32 {
        let here = self.screen_to_source(at);
        here.distance(self.screen_to_source(at + vec2(1.0, 0.0)))
    }

    /// A normalized source position (0 to 1 across the source) in pixels.
    pub(crate) fn pixels(&self, normalized: Point) -> Point {
        let size = self.source_size();
        Point::new(normalized.x * size.x, normalized.y * size.y)
    }

    /// A source position in pixels as a normalized one.
    pub(crate) fn normalized(&self, pixels: Point) -> Point {
        let size = self.source_size();
        Point::new(pixels.x / size.x, pixels.y / size.y)
    }

    /// Where a normalized source position is on screen, in points.
    pub(crate) fn to_screen(&self, normalized: Point) -> Option<Pos2> {
        self.source_to_screen(self.pixels(normalized))
    }

    /// The normalized source position under a screen position (points).
    pub(crate) fn to_normalized(&self, screen: Pos2) -> Point {
        self.normalized(self.screen_to_source(screen))
    }

    /// The box the photo takes on screen, in points: the output, or in the
    /// crop tool the turned frame.
    pub(crate) fn photo_rect(&self) -> Rect {
        let pixels = match &self.display {
            Display::Output(placement) => placement.photo(),
            Display::Straight(straight) => straight.bounds(),
        };
        to_points(pixels, self.ppp)
    }

    /// The four corners of the source on screen, in points, in order around
    /// it.
    pub(crate) fn source_corners(&self) -> Option<[Pos2; 4]> {
        let size = self.source_size();
        let corner = |x: f32, y: f32| self.source_to_screen(Point::new(x, y));
        Some([
            corner(0.0, 0.0)?,
            corner(size.x, 0.0)?,
            corner(size.x, size.y)?,
            corner(0.0, size.y)?,
        ])
    }

    /// Paint a texture whose pixels are the output rectangle `roi` as
    /// `rendered` lays the output out (`None`: as the settings do now).
    ///
    /// With the layout unchanged and the output on screen, that is a plain
    /// rectangle, one texture pixel per screen pixel at the fixed zooms.
    /// Otherwise the texture is warped: each point of it goes to where the
    /// content it shows is now.
    pub(crate) fn paint(
        &self,
        painter: &Painter,
        texture: TextureId,
        roi: geom::Rect,
        rendered: Option<&Geometry>,
    ) {
        let same = rendered.is_none_or(|geometry| *geometry == self.geometry);
        if let (Display::Output(placement), true) = (&self.display, same) {
            let rect = to_points(placement.on_screen(roi), self.ppp);
            painter.image(texture, rect, UV, Color32::WHITE);
            return;
        }
        let geometry = rendered.unwrap_or(&self.geometry);
        let side = MESH + 1;
        let mut mesh = Mesh::with_texture(texture);
        let mut placed = Vec::with_capacity(side * side);
        for row in 0..side {
            for column in 0..side {
                let (u, v) = (column as f32 / MESH as f32, row as f32 / MESH as f32);
                let out = Point::new(roi.x0 + u * roi.width(), roi.y0 + v * roi.height());
                let at = self.source_to_screen(geometry.out_to_src(out));
                placed.push(at.is_some());
                mesh.vertices.push(Vertex {
                    pos: at.unwrap_or_default(),
                    uv: pos2(u, v),
                    color: Color32::WHITE,
                });
            }
        }
        for row in 0..MESH {
            for column in 0..MESH {
                let a = row * side + column;
                let (b, c, d) = (a + 1, a + side, a + side + 1);
                if [a, b, c, d].iter().all(|index| placed[*index]) {
                    mesh.add_triangle(a as u32, b as u32, c as u32);
                    mesh.add_triangle(b as u32, d as u32, c as u32);
                }
            }
        }
        painter.add(Shape::mesh(mesh));
    }

    /// A circle of `radius` source pixels about a source position, as the
    /// points it has on screen: an ellipse where the photo is transformed.
    pub(crate) fn circle(&self, center: Point, radius: f32) -> Vec<Pos2> {
        self.ellipse(center, vec2(radius, radius), 0.0)
    }

    /// An ellipse about a source position with `radii` in source pixels, its
    /// own x axis turned by `angle` degrees in the source, as its points on
    /// screen.
    pub(crate) fn ellipse(&self, center: Point, radii: Vec2, angle: f32) -> Vec<Pos2> {
        const SEGMENTS: usize = 64;
        (0..SEGMENTS)
            .filter_map(|step| {
                let around = step as f32 / SEGMENTS as f32 * std::f32::consts::TAU;
                let own = vec2(radii.x * around.cos(), radii.y * around.sin());
                let offset = turned(own, angle);
                self.source_to_screen(Point::new(center.x + offset.x, center.y + offset.y))
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use photo_core::develop::{CropRect, Orientation};

    use super::*;
    use crate::app::develop::canvas::mapping::CanvasZoom;

    const SOURCE: (u32, u32) = (1200, 800);

    fn canvas() -> Rect {
        Rect::from_min_size(pos2(100.0, 60.0), vec2(1000.0, 800.0))
    }

    fn output_map(settings: &DevelopSettings) -> CanvasMap {
        let geometry = Geometry::from_settings(SOURCE.0, SOURCE.1, settings);
        let (width, height) = geometry.output_size();
        let output = vec2(width as f32, height as f32);
        let placement = Placement::new(CanvasZoom::Fit, vec2(0.5, 0.5), output, canvas());
        CanvasMap {
            ppp: 2.0,
            geometry,
            display: Display::Output(placement),
        }
    }

    fn straight_map(settings: &DevelopSettings) -> CanvasMap {
        let straight = Straight::new(settings, SOURCE, settings.crop.angle, canvas());
        CanvasMap {
            ppp: 2.0,
            geometry: Geometry::from_settings(SOURCE.0, SOURCE.1, settings),
            display: Display::Straight(straight),
        }
    }

    fn cropped() -> DevelopSettings {
        let mut settings = DevelopSettings::default();
        settings.orientation = Orientation::new(1, false);
        settings.crop.angle = 12.0;
        settings.crop.rect = Some(CropRect {
            cx: 0.45,
            cy: 0.55,
            width: 0.4,
            height: 0.3,
        });
        settings
    }

    #[test]
    fn a_source_position_goes_to_the_screen_and_back_in_both_displays() {
        let settings = cropped();
        for map in [output_map(&settings), straight_map(&settings)] {
            for source in [Point::new(600.0, 400.0), Point::new(130.5, 702.25)] {
                let screen = map.source_to_screen(source).unwrap();
                let back = map.screen_to_source(screen);
                assert!(
                    (back.x - source.x).abs() < 0.05 && (back.y - source.y).abs() < 0.05,
                    "{source:?} -> {screen:?} -> {back:?}"
                );
            }
            let normalized = Point::new(0.25, 0.75);
            let pixels = map.pixels(normalized);
            assert_eq!((pixels.x, pixels.y), (300.0, 600.0));
            assert_eq!(map.normalized(pixels), normalized);
        }
    }

    #[test]
    fn the_straightened_frame_is_whole_on_the_canvas_with_the_crop_upright() {
        let settings = cropped();
        let map = straight_map(&settings);
        let Display::Straight(straight) = &map.display else {
            panic!("straight");
        };
        assert!(canvas().contains_rect(straight.bounds()));
        // The frame is the source turned a quarter: 800 wide, 1200 high.
        assert_eq!(straight.size(), vec2(800.0, 1200.0));
        // The crop rectangle's corners, as the settings give them, make an
        // upright rectangle on screen.
        let corners = settings.crop.rect.unwrap().corners(12.0, 800.0 / 1200.0);
        let on_screen: Vec<Pos2> = corners
            .iter()
            .map(|corner| straight.to_screen(vec2(corner.x * 800.0, corner.y * 1200.0)))
            .collect();
        assert!((on_screen[0].y - on_screen[1].y).abs() < 0.01, "top is level");
        assert!((on_screen[1].x - on_screen[2].x).abs() < 0.01, "right is upright");
        let back = straight.to_frame(on_screen[2]);
        assert!((back - vec2(corners[2].x * 800.0, corners[2].y * 1200.0)).length() < 0.01);
    }

    #[test]
    fn every_corner_of_the_source_is_on_the_canvas_while_cropping() {
        let map = straight_map(&cropped());
        let corners = map.source_corners().unwrap();
        let on_canvas = to_points(canvas(), 2.0).expand(0.5);
        assert!(corners.iter().all(|corner| on_canvas.contains(*corner)));
        assert!(map.photo_rect().expand(0.5).contains(corners[0]));
    }

    #[test]
    fn a_circle_in_the_source_is_a_circle_on_screen_when_the_photo_is_only_turned() {
        let map = output_map(&cropped());
        let center = Point::new(500.0, 420.0);
        let at = map.source_to_screen(center).unwrap();
        let ring = map.circle(center, 40.0);
        assert_eq!(ring.len(), 64);
        let Display::Output(placement) = &map.display else {
            panic!("output");
        };
        let radius = 40.0 * placement.scale / map.ppp;
        for point in ring {
            assert!(((point - at).length() - radius).abs() < 0.05);
        }
    }
}
