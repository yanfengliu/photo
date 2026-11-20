//! Masks seen through geometry and views: a mask stays attached to image
//! content, and its rasterization is region and resolution independent.

use photo_core::develop::{
    BrushStroke, Crop, CropRect, LensCorrections, Mask, MaskShape, Orientation, Transform,
};
use photo_core::geom::{Point, Rect};
use photo_imaging::{Geometry, Plane, View, rasterize_mask};

use crate::mask::{H, W, add, brush, linear, mask, radial, stroke, subtract};
use crate::util::{max_abs_diff, mean_abs_diff};

fn warped() -> Geometry {
    Geometry::new(
        W,
        H,
        &LensCorrections {
            distortion: 30.0,
            ..Default::default()
        },
        Orientation::new(3, true),
        &Transform {
            vertical: 25.0,
            rotate: -4.0,
            scale: 108.0,
            ..Default::default()
        },
        &Crop {
            rect: Some(CropRect {
                cx: 0.5,
                cy: 0.45,
                width: 0.7,
                height: 0.8,
            }),
            angle: 6.0,
            ..Default::default()
        },
    )
}

fn turned_and_cropped() -> Geometry {
    Geometry::new(
        W,
        H,
        &LensCorrections::default(),
        Orientation::new(1, true),
        &Transform::default(),
        &Crop {
            rect: Some(CropRect {
                cx: 0.5,
                cy: 0.5,
                width: 0.8,
                height: 0.8,
            }),
            ..Default::default()
        },
    )
}

#[test]
fn a_mask_stays_on_its_content_through_orientation_crop_and_scale() {
    // A small hard disc around source position (60, 60) px.
    let spot = MaskShape::Radial {
        center: Point::new(0.3, 0.6),
        radius_x: 0.04,
        radius_y: 0.04,
        angle: 0.0,
        feather: 0.0,
        invert: false,
    };
    let on_content = mask(vec![add(spot)]);
    let source_point = Point::new(60.0, 60.0);
    for geometry in [Geometry::identity(W, H), turned_and_cropped(), warped()] {
        let at = geometry.src_to_out(source_point).unwrap();
        let (ow, oh) = geometry.output_size();
        assert!(at.x > 12.0 && at.y > 12.0 && at.x < ow as f32 - 12.0 && at.y < oh as f32 - 12.0);
        for (width, height) in [(ow, oh), (ow / 2, oh / 2)] {
            let view = View::new(Rect::from_size(ow as f32, oh as f32), width, height);
            let alpha = rasterize_mask(&on_content, &geometry, &view, 0);
            let scale = view.scale();
            let (x, y) = ((at.x * scale) as usize, (at.y * scale) as usize);
            assert_eq!(alpha.get(x, y), 1.0, "at the content's output position");
            let away = ((at.x + 12.0) * scale) as usize;
            assert_eq!(alpha.get(away, y), 0.0, "12 px beside it");
            let covered: f32 = alpha.as_slice().iter().sum();
            // The disc is 8 px in radius at full resolution wherever it lands.
            let expected = std::f32::consts::PI * 64.0 * scale * scale;
            assert!(
                (covered / expected - 1.0).abs() < 0.35,
                "{covered} vs {expected}"
            );
        }
    }
}

fn everything() -> Mask {
    mask(vec![
        add(linear((0.1, 0.2), (0.7, 0.9))),
        add(brush(vec![
            stroke(
                &[(0.15, 0.3), (0.4, 0.35), (0.6, 0.7), (0.9, 0.6)],
                60.0,
                80.0,
                false,
            ),
            stroke(&[(0.3, 0.1), (0.35, 0.9)], 0.0, 100.0, true),
            stroke(&[(0.7, 0.2), (0.75, 0.25)], 30.0, 55.0, false),
        ])),
        subtract(radial(70.0, false, 25.0)),
    ])
}

#[test]
fn r1_a_padded_tile_of_a_mask_matches_the_whole() {
    let mask = everything();
    for geometry in [Geometry::identity(W, H), warped()] {
        let (ow, oh) = geometry.output_size();
        for scale in [1.0f32, 0.5] {
            let (vw, vh) = ((ow as f32 * scale) as u32, (oh as f32 * scale) as u32);
            let view = View::new(
                Rect::from_size(vw as f32 / scale, vh as f32 / scale),
                vw,
                vh,
            );
            let whole = rasterize_mask(&mask, &geometry, &view, 0);
            let (x, y, w, h) = (vw / 4, vh / 3, vw / 2, vh / 2);
            let tile = rasterize_mask(&mask, &geometry, &view.tile(x, y, w, h), 6);
            assert_eq!(
                (tile.width(), tile.height()),
                (w as usize + 12, h as usize + 12)
            );
            let inner = tile.crop(6, 6, w as usize, h as usize);
            let expected = whole.crop(x as usize, y as usize, w as usize, h as usize);
            // The same bits, not merely close: a tile stamps only the brush
            // cells it reads, but each cell comes out as it does for the whole.
            let bits = |plane: &Plane| {
                plane
                    .as_slice()
                    .iter()
                    .map(|v| v.to_bits())
                    .collect::<Vec<_>>()
            };
            assert!(
                bits(&inner) == bits(&expected),
                "scale {scale}: differs by {}",
                max_abs_diff(inner.as_slice(), expected.as_slice())
            );
        }
    }
}

#[test]
fn r2_a_half_scale_mask_approximates_the_full_one_downsampled() {
    let mask = everything();
    for geometry in [Geometry::identity(W, H), warped()] {
        let (ow, oh) = geometry.output_size();
        let (ow, oh) = (ow / 2 * 2, oh / 2 * 2);
        let roi = Rect::from_size(ow as f32, oh as f32);
        let full = rasterize_mask(&mask, &geometry, &View::new(roi, ow, oh), 0);
        let half = rasterize_mask(&mask, &geometry, &View::new(roi, ow / 2, oh / 2), 0);
        let reference = Plane::from_fn(half.width(), half.height(), |x, y| {
            let block = [(0, 0), (1, 0), (0, 1), (1, 1)];
            block
                .iter()
                .map(|(dx, dy)| full.get(2 * x + dx, 2 * y + dy))
                .sum::<f32>()
                / 4.0
        });
        let mean = mean_abs_diff(half.as_slice(), reference.as_slice());
        assert!(mean < 0.01, "{mean}");
    }
}

#[test]
fn outside_the_picture_a_mask_continues_its_edge_value() {
    // Shrink the picture inside the frame: output positions left and right
    // of it show the picture's replicated edge, and the mask must agree.
    let geometry = Geometry::new(
        W,
        H,
        &LensCorrections::default(),
        Orientation::IDENTITY,
        &Transform {
            scale: 50.0,
            ..Default::default()
        },
        &Crop::default(),
    );
    let gradient = mask(vec![add(linear((0.0, 0.5), (1.0, 0.5)))]);
    let alpha = rasterize_mask(&gradient, &geometry, &View::full(&geometry), 0);
    // The picture spans output x = 50..150.
    assert_eq!(alpha.get(10, 50), 1.0);
    assert_eq!(alpha.get(190, 50), 0.0);
    assert!((alpha.get(100, 50) - 0.5).abs() < 0.02);
    let disc = mask(vec![add(radial(0.0, false, 0.0))]);
    let alpha = rasterize_mask(&disc, &geometry, &View::full(&geometry), 0);
    assert_eq!(alpha.get(100, 50), 1.0);
    assert_eq!(alpha.get(10, 50), 0.0);
}

#[test]
fn padding_adds_real_coverage_around_the_view() {
    let gradient = mask(vec![add(linear((0.25, 0.5), (0.75, 0.5)))]);
    let geometry = Geometry::identity(W, H);
    let view = View::full(&geometry).tile(60, 20, 80, 40);
    let plain = rasterize_mask(&gradient, &geometry, &view, 0);
    let padded = rasterize_mask(&gradient, &geometry, &view, 9);
    assert_eq!((padded.width(), padded.height()), (98, 58));
    assert_eq!(padded.crop(9, 9, 80, 40), plain);
    // The padding column left of the tile is further toward the start.
    assert!(padded.get(0, 20) > plain.get(0, 20));
}

#[test]
fn a_distant_stroke_does_not_change_how_another_renders() {
    // Two hard dabs at opposite corners of a 24 MP source, seen at 1:1
    // around the first one. Adding the second must not touch a single bit
    // of the first: no shared resolution budget, no shared rounding.
    let (width, height) = (6_000u32, 4_000u32);
    let geometry = Geometry::identity(width, height);
    let dab = |x: f32, y: f32| BrushStroke {
        points: vec![Point::new(x, y)],
        radius: 0.005,
        feather: 0.0,
        flow: 100.0,
        erase: false,
    };
    let one = mask(vec![add(brush(vec![dab(0.05, 0.05)]))]);
    let two = mask(vec![add(brush(vec![dab(0.05, 0.05), dab(0.95, 0.95)]))]);
    // The first dab is 30 px in radius around (300, 200).
    let view = View::full(&geometry).tile(250, 150, 100, 100);
    let alone = rasterize_mask(&one, &geometry, &view, 2);
    let together = rasterize_mask(&two, &geometry, &view, 2);
    let bits = |plane: &Plane| {
        plane
            .as_slice()
            .iter()
            .map(|v| v.to_bits())
            .collect::<Vec<_>>()
    };
    assert!(bits(&alone) == bits(&together));
    // And the dab's edge is as sharp as a 1:1 view can show: from full to
    // empty within two pixels along a row through its centre.
    let row: Vec<f32> = (0..104).map(|x| alone.get(x, 52)).collect();
    let partial = row.iter().filter(|&&v| v > 0.0 && v < 1.0).count();
    assert!(
        partial <= 4,
        "{partial} partly covered pixels across two edges"
    );
    assert_eq!(row[52], 1.0);
}

#[test]
fn brush_detail_is_bounded_but_never_coarser_than_the_view_needs() {
    // A huge source with a hard brush across all of it, seen through a small
    // fit view and through a 1:1 tile. Both must show a crisp edge on their
    // own pixel grid and take a bounded amount of work.
    let (width, height) = (12_000u32, 8_000u32);
    let geometry = Geometry::identity(width, height);
    let across = BrushStroke {
        points: vec![Point::new(0.02, 0.5), Point::new(0.98, 0.5)],
        radius: 0.05,
        feather: 0.0,
        flow: 100.0,
        erase: false,
    };
    let mask = mask(vec![add(brush(vec![across]))]);
    // The stroke's lower edge is at y = 4000 + 600 = 4600 px.
    let fit = View::fit(&geometry, 600);
    let small = rasterize_mask(&mask, &geometry, &fit, 0);
    let edge = 4600.0 * fit.scale();
    assert_eq!(small.get(300, (edge - 3.0) as usize), 1.0);
    assert_eq!(small.get(300, (edge + 3.0) as usize), 0.0);

    let tile = View::new(Rect::new(6000.0, 4568.0, 6064.0, 4632.0), 64, 64);
    let close = rasterize_mask(&mask, &geometry, &tile, 0);
    assert_eq!(close.get(32, 20), 1.0);
    assert_eq!(close.get(32, 44), 0.0);
}
