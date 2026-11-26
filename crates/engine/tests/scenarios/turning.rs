//! Turning and mirroring photos from the Library: the picture afterwards is
//! the picture before, turned, whatever crop the photo has.

use photo_core::develop::{Crop, CropConstraint, CropRect, DevelopSettings};
use photo_core::image::Rgba8Image;
use photo_core::meta::PhotoId;
use photo_engine::{Engine, StillSettings, Target};
use tempfile::TempDir;

use crate::common::Studio;

/// The Library's four commands that turn a photo.
#[derive(Clone, Copy, Debug)]
enum Turning {
    Left,
    Right,
    Horizontal,
    Vertical,
}

const ALL: [Turning; 4] = [
    Turning::Left,
    Turning::Right,
    Turning::Horizontal,
    Turning::Vertical,
];

impl Turning {
    fn command(self, engine: &mut Engine, photo: PhotoId) {
        let target = Target::Photos(vec![photo]);
        match self {
            Self::Left => engine.rotate_left(target),
            Self::Right => engine.rotate_right(target),
            Self::Horizontal => engine.flip_horizontal(target),
            Self::Vertical => engine.flip_vertical(target),
        }
        .unwrap();
    }

    fn is_quarter(self) -> bool {
        matches!(self, Self::Left | Self::Right)
    }

    /// `picture` as someone who turned a print of it this way would see it.
    fn of(self, picture: &Rgba8Image) -> Rgba8Image {
        let (width, height) = (picture.width, picture.height);
        let (turned_width, turned_height) = if self.is_quarter() {
            (height, width)
        } else {
            (width, height)
        };
        let mut pixels = Vec::with_capacity(picture.pixels.len());
        for y in 0..turned_height {
            for x in 0..turned_width {
                // Where this pixel of the turned picture was before.
                let (from_x, from_y) = match self {
                    Self::Left => (width - 1 - y, x),
                    Self::Right => (y, height - 1 - x),
                    Self::Horizontal => (width - 1 - x, y),
                    Self::Vertical => (x, height - 1 - y),
                };
                pixels.extend(picture.pixel(from_x, from_y));
            }
        }
        Rgba8Image {
            width: turned_width,
            height: turned_height,
            pixels,
            space: picture.space,
        }
    }
}

fn picture(studio: &Studio, photo: PhotoId) -> Rgba8Image {
    studio
        .engine
        .render_photo_rgba8(photo, StillSettings::Current, None)
        .unwrap()
}

/// The largest difference between two pictures in any channel of any
/// pixel, and the mean difference over all of them.
fn difference(a: &Rgba8Image, b: &Rgba8Image) -> (u8, f64) {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let differences = a.pixels.iter().zip(&b.pixels).map(|(a, b)| a.abs_diff(*b));
    let largest = differences.clone().max().unwrap_or(0);
    let sum: f64 = differences.map(f64::from).sum();
    (largest, sum / a.pixels.len() as f64)
}

/// A photo of the fixture scene (sky, sun, ground, colour patches: no two
/// corners alike) with a crop off its centre, tilted and wider than tall.
fn cropped_photo() -> (TempDir, Studio, PhotoId, DevelopSettings) {
    let dir = tempfile::tempdir().unwrap();
    let file = photo_testkit::write_png8(dir.path(), "scene.png", 480, 320);
    let mut studio = Studio::new();
    studio.import(vec![file]);
    let photo = studio.ids()[0];
    studio.engine.open_develop(photo).unwrap();
    let mut settings = studio.engine.develop().unwrap().settings().clone();
    settings.crop = Crop {
        rect: Some(CropRect {
            cx: 0.6,
            cy: 0.42,
            width: 0.45,
            height: 0.5,
        }),
        angle: 7.0,
        constraint: CropConstraint::Ratio {
            width: 27.0,
            height: 20.0,
        },
    };
    studio.engine.apply_settings(settings).unwrap();
    studio.engine.close_develop();
    studio.settle();
    let cropped = studio.engine.develop_settings(photo).unwrap();
    assert!(cropped.crop.rect.is_some() && cropped.crop.angle == 7.0);
    (dir, studio, photo, cropped)
}

/// Turn the photo each of the four ways from where it is now, undoing in
/// between. Each time the picture must be the one before, turned, and one
/// undo must bring back the settings and the picture there were.
fn turn_every_way(studio: &mut Studio, photo: PhotoId) {
    let settings = studio.engine.develop_settings(photo).unwrap();
    let before = picture(studio, photo);
    assert_ne!(before.width, before.height, "the crop is not square");
    for turning in ALL {
        turning.command(&mut studio.engine, photo);
        let after = picture(studio, photo);
        let expected = turning.of(&before);
        assert_eq!(
            (after.width, after.height),
            (expected.width, expected.height),
            "{turning:?}: the size of the picture"
        );
        let (largest, mean) = difference(&after, &expected);
        assert!(
            largest <= 2 && mean < 0.1,
            "{turning:?}: the picture is not the one before, turned: it differs by up to {largest} of 255, {mean:.3} on average"
        );

        assert!(studio.engine.undo().unwrap());
        assert_eq!(
            studio.engine.develop_settings(photo).unwrap(),
            settings,
            "{turning:?}: one undo brings back the orientation and the crop"
        );
        assert_eq!(picture(studio, photo).pixels, before.pixels, "{turning:?}");
    }
}

#[test]
fn a_cropped_photo_turned_from_the_library_shows_the_same_picture_turned() {
    let (_dir, mut studio, photo, _) = cropped_photo();
    turn_every_way(&mut studio, photo);
}

#[test]
fn a_cropped_photo_that_is_already_turned_and_mirrored_turns_the_same_way() {
    let (_dir, mut studio, photo, cropped) = cropped_photo();
    Turning::Right.command(&mut studio.engine, photo);
    Turning::Horizontal.command(&mut studio.engine, photo);
    let turned = studio.engine.develop_settings(photo).unwrap();
    assert_ne!(turned.orientation, cropped.orientation);
    turn_every_way(&mut studio, photo);
}
