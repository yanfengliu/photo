//! Contact sheets: labelled renders in a grid, saved as PNG.

use std::path::Path;

use photo_core::image::Rgba8Image;

use super::font::{ADVANCE, LINE, draw};

const SCALE: u32 = 2;
const LABEL: u32 = LINE * SCALE + 8;
const GAP: u32 = 6;
const BACKGROUND: [u8; 3] = [40, 40, 40];
const INK: [u8; 3] = [235, 235, 235];

/// A titled grid of labelled images, filled row by row.
pub struct Sheet {
    title: String,
    columns: usize,
    cells: Vec<(String, Rgba8Image)>,
}

impl Sheet {
    pub fn new(title: &str, columns: usize) -> Self {
        Self {
            title: title.to_string(),
            columns,
            cells: Vec::new(),
        }
    }

    pub fn add(&mut self, label: impl Into<String>, image: Rgba8Image) {
        self.cells.push((label.into(), image));
    }

    /// Start the next row even if this one is not full.
    pub fn end_row(&mut self) {
        while !self.cells.len().is_multiple_of(self.columns) {
            self.cells.push((
                String::new(),
                Rgba8Image {
                    width: 0,
                    height: 0,
                    pixels: Vec::new(),
                    space: Default::default(),
                },
            ));
        }
    }

    /// Compose and write `<dir>/<name>.png`.
    pub fn save(&self, dir: &Path, name: &str) {
        let cell_w = self.cells.iter().map(|(_, i)| i.width).max().unwrap_or(1);
        let cell_h = self.cells.iter().map(|(_, i)| i.height).max().unwrap_or(1);
        let rows = self.cells.len().div_ceil(self.columns) as u32;
        let columns = self.columns as u32;
        let width = GAP + columns * (cell_w + GAP);
        let title = LABEL + GAP;
        let height = title + rows * (cell_h + LABEL + GAP);
        let mut pixels = BACKGROUND.repeat((width * height) as usize);
        draw(&mut pixels, width, GAP, GAP, SCALE, &self.title, INK);
        for (index, (label, image)) in self.cells.iter().enumerate() {
            let (column, row) = (index as u32 % columns, index as u32 / columns);
            let x0 = GAP + column * (cell_w + GAP);
            let y0 = title + row * (cell_h + LABEL + GAP);
            for y in 0..image.height {
                for x in 0..image.width {
                    let [r, g, b, _] = image.pixel(x, y);
                    let at = (((y0 + y) * width + x0 + x) * 3) as usize;
                    pixels[at..at + 3].copy_from_slice(&[r, g, b]);
                }
            }
            let fits = (cell_w / (ADVANCE * SCALE)) as usize;
            let text: String = label.chars().take(fits).collect();
            draw(&mut pixels, width, x0, y0 + cell_h + 4, SCALE, &text, INK);
        }
        std::fs::create_dir_all(dir).unwrap();
        image::RgbImage::from_raw(width, height, pixels)
            .unwrap()
            .save(dir.join(format!("{name}.png")))
            .unwrap();
    }
}
