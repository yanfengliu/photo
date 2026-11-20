//! Spike: prove eframe + headless egui_kittest rendering works here.

#[derive(Default)]
pub struct SpikeApp {
    pub exposure: f32,
    texture: Option<egui::TextureHandle>,
}

impl eframe::App for SpikeApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Panel::left("left")
            .default_size(200.0)
            .show(ui, |ui| {
                ui.heading("Navigator");
            });
        egui::Panel::right("right")
            .default_size(260.0)
            .show(ui, |ui| {
                ui.add(egui::Slider::new(&mut self.exposure, -5.0..=5.0).text("Exposure"));
            });
        egui::CentralPanel::default_margins().show(ui, |ui| {
            let texture = self.texture.get_or_insert_with(|| {
                let (w, h) = (256usize, 160usize);
                let mut pixels = Vec::with_capacity(w * h);
                for y in 0..h {
                    for x in 0..w {
                        pixels.push(egui::Color32::from_rgb(
                            (x * 255 / w) as u8,
                            (y * 255 / h) as u8,
                            128,
                        ));
                    }
                }
                ui.ctx().load_texture(
                    "spike",
                    egui::ColorImage::new([w, h], pixels),
                    egui::TextureOptions::LINEAR,
                )
            });
            ui.image((texture.id(), egui::vec2(512.0, 320.0)));
        });
    }
}

pub fn run() -> Result<(), Box<dyn std::error::Error>> {
    eframe::run_native(
        "Photo",
        eframe::NativeOptions::default(),
        Box::new(|_cc| Ok(Box::new(SpikeApp::default()))),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn spike_drives_and_renders_headless() {
        let mut harness = egui_kittest::Harness::builder()
            .with_size(egui::vec2(1280.0, 800.0))
            .wgpu()
            .build_eframe(|_cc| SpikeApp::default());
        harness.run();
        harness
            .get_by_role_and_label(egui::accesskit::Role::Slider, "Exposure")
            .set_value_accesskit(2.0);
        harness.run();
        assert!((harness.state().exposure - 2.0).abs() < 0.01);

        let image = harness.render().expect("headless wgpu render");
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/ui-snapshots");
        std::fs::create_dir_all(&dir).unwrap();
        image.save(dir.join("spike.png")).unwrap();
    }
}
