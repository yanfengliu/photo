//! Generated stand-ins for photographs: smooth gradients with a glow, in a
//! few aspect ratios. The gallery has no catalog to draw from, and widgets
//! should be judged over something that behaves like a photo, not a flat fill.

use egui::{Color32, ColorImage, TextureHandle, TextureOptions};

/// One generated picture: three colours down the frame and a soft light.
struct Scene {
    name: &'static str,
    /// Width divided by height.
    aspect: f32,
    top: [u8; 3],
    middle: [u8; 3],
    bottom: [u8; 3],
    glow: [u8; 3],
    /// Where the glow is centred, as fractions of the frame.
    glow_at: [f32; 2],
}

const SCENES: [Scene; 8] = [
    Scene {
        name: "dusk",
        aspect: 1.5,
        top: [28, 44, 92],
        middle: [196, 112, 84],
        bottom: [34, 28, 48],
        glow: [255, 196, 120],
        glow_at: [0.68, 0.52],
    },
    Scene {
        name: "dune",
        aspect: 1.5,
        top: [142, 170, 196],
        middle: [214, 176, 128],
        bottom: [128, 84, 52],
        glow: [255, 236, 200],
        glow_at: [0.3, 0.2],
    },
    Scene {
        name: "sea",
        aspect: 16.0 / 9.0,
        top: [178, 206, 214],
        middle: [44, 120, 138],
        bottom: [14, 40, 66],
        glow: [220, 240, 236],
        glow_at: [0.5, 0.3],
    },
    Scene {
        name: "forest",
        aspect: 2.0 / 3.0,
        top: [168, 190, 150],
        middle: [58, 104, 62],
        bottom: [20, 38, 28],
        glow: [236, 232, 170],
        glow_at: [0.62, 0.22],
    },
    Scene {
        name: "rose",
        aspect: 1.0,
        top: [228, 170, 180],
        middle: [170, 92, 130],
        bottom: [60, 38, 84],
        glow: [255, 220, 214],
        glow_at: [0.4, 0.36],
    },
    Scene {
        name: "slate",
        aspect: 1.5,
        top: [198, 198, 198],
        middle: [112, 112, 112],
        bottom: [30, 30, 30],
        glow: [240, 240, 240],
        glow_at: [0.26, 0.3],
    },
    Scene {
        name: "glacier",
        aspect: 4.0 / 5.0,
        top: [226, 238, 244],
        middle: [120, 176, 204],
        bottom: [34, 66, 104],
        glow: [255, 255, 255],
        glow_at: [0.7, 0.24],
    },
    Scene {
        name: "ember",
        aspect: 2.4,
        top: [22, 16, 20],
        middle: [150, 50, 30],
        bottom: [26, 14, 12],
        glow: [255, 190, 92],
        glow_at: [0.5, 0.5],
    },
];

/// The longer edge of a generated picture, in pixels.
const LONG_EDGE: usize = 192;

/// A generated picture on the GPU, with the aspect ratio to show it at.
pub struct Picture {
    /// The texture.
    pub texture: TextureHandle,
    /// Width divided by height.
    pub aspect: f32,
}

/// Generate every scene and upload it. Call once.
pub fn generate(ctx: &egui::Context) -> Vec<Picture> {
    SCENES
        .iter()
        .map(|scene| Picture {
            texture: ctx.load_texture(
                format!("gallery-{}", scene.name),
                render(scene),
                TextureOptions::LINEAR,
            ),
            aspect: scene.aspect,
        })
        .collect()
}

fn render(scene: &Scene) -> ColorImage {
    let (width, height) = if scene.aspect >= 1.0 {
        (
            LONG_EDGE,
            (LONG_EDGE as f32 / scene.aspect).round() as usize,
        )
    } else {
        (
            (LONG_EDGE as f32 * scene.aspect).round() as usize,
            LONG_EDGE,
        )
    };
    let mut pixels = Vec::with_capacity(width * height);
    for y in 0..height {
        for x in 0..width {
            let u = (x as f32 + 0.5) / width as f32;
            let v = (y as f32 + 0.5) / height as f32;
            pixels.push(shade(scene, u, v));
        }
    }
    ColorImage::new([width, height], pixels)
}

/// The colour of `scene` at (`u`, `v`), both 0 to 1 from the top left.
fn shade(scene: &Scene, u: f32, v: f32) -> Color32 {
    // A gentle wave in the horizon keeps the bands from looking ruled.
    let v = (v + 0.035 * (u * 5.0).sin()).clamp(0.0, 1.0);
    let (from, to, t) = if v < 0.5 {
        (scene.top, scene.middle, v * 2.0)
    } else {
        (scene.middle, scene.bottom, (v - 0.5) * 2.0)
    };
    let t = t * t * (3.0 - 2.0 * t);
    let distance = ((u - scene.glow_at[0]) * scene.aspect).hypot(v - scene.glow_at[1]);
    let glow = 0.55 * (-distance * distance / 0.05).exp();
    let channel = |index: usize| {
        let base = f32::from(from[index]) + (f32::from(to[index]) - f32::from(from[index])) * t;
        let lit = base + (f32::from(scene.glow[index]) - base) * glow;
        lit.round().clamp(0.0, 255.0) as u8
    };
    Color32::from_rgb(channel(0), channel(1), channel(2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pictures_have_the_aspect_they_claim() {
        for scene in &SCENES {
            let image = render(scene);
            let aspect = image.width() as f32 / image.height() as f32;
            assert!(
                (aspect - scene.aspect).abs() < 0.02,
                "{}: {aspect}",
                scene.name
            );
            assert_eq!(image.width().max(image.height()), LONG_EDGE);
        }
    }

    #[test]
    fn a_picture_runs_from_its_top_colour_to_its_bottom_colour() {
        let scene = &SCENES[5];
        let top = shade(scene, 0.9, 0.0);
        let bottom = shade(scene, 0.9, 1.0);
        assert!(top.r() > 150 && bottom.r() < 60, "{top:?} {bottom:?}");
    }
}
