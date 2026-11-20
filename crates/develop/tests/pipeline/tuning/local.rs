//! Sheets for geometry, masks and healing.

use std::path::Path;

use photo_core::develop::{
    BrushStroke, Crop, CropRect, DevelopSettings, Mask, MaskComponent, MaskShape, Orientation,
    Spot, SpotMode, Transform,
};
use photo_core::geom::Point;
use photo_core::image::Rgba8Image;
use photo_develop::{Renderer, fit_view, mask_alpha};

use super::{CELL, Scenes, Sheet, actual, edit, thumb};

pub fn sheets(scenes: &Scenes, dir: &Path) {
    geometry(scenes, dir);
    masks(scenes, dir);
    healing(scenes, dir);
}

fn geometry(scenes: &Scenes, dir: &Path) {
    let mut renderer = Renderer::new();
    let mut sheet = Sheet::new("GEOMETRY", 5);
    let cases = [
        ("CROP ANGLE +10", edit(|s| s.crop.angle = 10.0)),
        (
            "CROP RECT ANGLE -15",
            edit(|s| {
                s.crop = Crop {
                    rect: Some(CropRect {
                        cx: 0.6,
                        cy: 0.45,
                        width: 0.5,
                        height: 0.6,
                    }),
                    angle: -15.0,
                    ..Default::default()
                };
            }),
        ),
        (
            "TURN CW",
            edit(|s| s.orientation = Orientation::new(1, false)),
        ),
        ("FLIP", edit(|s| s.orientation = Orientation::new(0, true))),
        (
            "TURN 180 + CROP",
            edit(|s| {
                s.orientation = Orientation::new(2, false);
                s.crop.rect = Some(CropRect {
                    cx: 0.5,
                    cy: 0.5,
                    width: 0.7,
                    height: 0.7,
                });
            }),
        ),
        ("VERTICAL -40", edit(|s| s.transform.vertical = -40.0)),
        ("HORIZONTAL +30", edit(|s| s.transform.horizontal = 30.0)),
        (
            "ROTATE 5 SCALE 120",
            edit(|s| {
                s.transform = Transform {
                    rotate: 5.0,
                    scale: 120.0,
                    ..Default::default()
                }
            }),
        ),
        ("DISTORTION +100", edit(|s| s.lens.distortion = 100.0)),
        ("DISTORTION -100", edit(|s| s.lens.distortion = -100.0)),
        ("LENS VIGNETTING +100", edit(|s| s.lens.vignetting = 100.0)),
        ("LENS VIGNETTING -100", edit(|s| s.lens.vignetting = -100.0)),
        ("ASPECT +100", edit(|s| s.transform.aspect = 100.0)),
        (
            "OFFSET X+30 Y-20",
            edit(|s| {
                s.transform.x_offset = 30.0;
                s.transform.y_offset = -20.0;
            }),
        ),
        (
            "ALL WITH CLARITY",
            edit(|s| {
                s.crop.angle = 6.0;
                s.transform.vertical = -25.0;
                s.lens.distortion = 40.0;
                s.presence.clarity = 60.0;
                s.tone.highlights = -60.0;
            }),
        ),
    ];
    for (label, settings) in &cases {
        sheet.add(*label, thumb(&mut renderer, &scenes.raster, settings));
    }
    sheet.save(dir, "12_geometry");
}

fn component(shape: MaskShape, subtract: bool) -> MaskComponent {
    MaskComponent { shape, subtract }
}

fn radial(cx: f32, cy: f32, rx: f32, ry: f32, feather: f32, invert: bool) -> MaskShape {
    MaskShape::Radial {
        center: Point::new(cx, cy),
        radius_x: rx,
        radius_y: ry,
        angle: 0.0,
        feather,
        invert,
    }
}

/// The red overlay a UI would draw: the render with the mask's alpha
/// blended toward red.
fn overlay(image: &Rgba8Image, alpha: &photo_imaging::Plane) -> Rgba8Image {
    let mut out = image.clone();
    for y in 0..image.height {
        for x in 0..image.width {
            let a = alpha.get(x as usize, y as usize) * 0.6;
            let at = ((y * image.width + x) * 4) as usize;
            let mix = |value: u8, target: f32| (f32::from(value) * (1.0 - a) + target * a) as u8;
            out.pixels[at] = mix(image.pixels[at], 255.0);
            out.pixels[at + 1] = mix(image.pixels[at + 1], 0.0);
            out.pixels[at + 2] = mix(image.pixels[at + 2], 0.0);
        }
    }
    out
}

fn masks(scenes: &Scenes, dir: &Path) {
    let mut renderer = Renderer::new();
    let mut sheet = Sheet::new("MASKS: RENDER AND RED OVERLAY", 4);
    let mut sky = Mask::from_shape(
        MaskShape::Linear {
            start: Point::new(0.5, 0.0),
            end: Point::new(0.5, 0.5),
        },
        1,
    );
    sky.adjust.exposure = -1.5;
    sky.adjust.dehaze = 40.0;
    let mut spot = Mask::from_shape(radial(0.3, 0.75, 0.15, 0.08, 50.0, false), 2);
    spot.adjust.exposure = 1.0;
    spot.adjust.clarity = 60.0;
    spot.adjust.saturation = 40.0;
    let mut brush = Mask::from_shape(
        MaskShape::Brush {
            strokes: vec![BrushStroke {
                points: (0..40)
                    .map(|i| {
                        let t = i as f32 / 39.0;
                        Point::new(0.1 + 0.8 * t, 0.72 + 0.04 * (t * 9.0).sin())
                    })
                    .collect(),
                radius: 0.03,
                feather: 60.0,
                flow: 100.0,
                erase: false,
            }],
        },
        3,
    );
    brush.adjust.saturation = -100.0;
    brush.adjust.exposure = 0.5;
    let mut vignette = Mask::from_shape(radial(0.5, 0.5, 0.45, 0.35, 80.0, true), 4);
    vignette.adjust.exposure = -1.0;
    vignette.adjust.temp = -40.0;
    vignette
        .components
        .push(component(radial(0.76, 0.2, 0.08, 0.08, 30.0, false), true));
    for mask in [sky, spot, brush, vignette] {
        let label = mask.name.clone();
        let settings = edit(|s| s.masks.push(mask));
        let image = thumb(&mut renderer, &scenes.raster, &settings);
        let view = fit_view(&scenes.raster, &settings, CELL.0, CELL.1);
        let alpha = mask_alpha(&scenes.raster, &settings, 0, view).unwrap();
        sheet.add(label.to_uppercase(), image.clone());
        sheet.add(
            "OVERLAY",
            overlay(
                &thumb(&mut renderer, &scenes.raster, &DevelopSettings::default()),
                &alpha,
            ),
        );
    }
    sheet.save(dir, "13_masks");
}

fn healing(scenes: &Scenes, dir: &Path) {
    let mut renderer = Renderer::new();
    let mut sheet = Sheet::new("HEALING: THE SUN REMOVED WITH SKY FROM ITS LEFT", 3);
    let spot = |mode| Spot {
        mode,
        target: Point::new(0.76, 0.2),
        source: Point::new(0.5, 0.2),
        radius: 0.075,
        feather: 40.0,
        opacity: 100.0,
    };
    let cases = [
        ("BEFORE", DevelopSettings::default()),
        ("HEAL", edit(|s| s.spots.push(spot(SpotMode::Heal)))),
        ("CLONE", edit(|s| s.spots.push(spot(SpotMode::Clone)))),
    ];
    let (width, height) = super::SCENE;
    let (x, y) = (0.76 * width as f32 - 180.0, 0.2 * height as f32 - 120.0);
    for (label, settings) in &cases {
        sheet.add(
            format!("1:1 {label}"),
            actual(&mut renderer, &scenes.raster, settings, x, y),
        );
    }
    for (label, settings) in &cases {
        sheet.add(
            format!("FIT {label}"),
            thumb(&mut renderer, &scenes.raster, settings),
        );
    }
    sheet.save(dir, "14_healing");
}
