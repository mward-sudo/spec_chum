//! Spec Chum — ZX Spectrum emulator frontend.

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([640.0, 480.0])
            .with_title("Spec Chum"),
        ..Default::default()
    };
    eframe::run_native(
        "Spec Chum",
        options,
        Box::new(|_cc| Ok(Box::new(SpecChumApp::default()))),
    )
}

struct SpecChumApp {
    framebuffer: Vec<egui::Color32>,
    width: usize,
    height: usize,
}

impl Default for SpecChumApp {
    fn default() -> Self {
        let width = 320;
        let height = 240;
        Self {
            framebuffer: vec![egui::Color32::from_rgb(0x00, 0x00, 0x80); width * height],
            width,
            height,
        }
    }
}

impl eframe::App for SpecChumApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Help", |ui| {
                    ui.label("Spec Chum — from-scratch ZX Spectrum emulator");
                });
            });
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Spec Chum");
            ui.label("Hardware-accurate ZX Spectrum emulator (Rust + egui)");
            let image = egui::ColorImage {
                size: [self.width, self.height],
                pixels: self.framebuffer.clone(),
            };
            let tex = ctx.load_texture("screen", image, egui::TextureOptions::NEAREST);
            ui.image((
                tex.id(),
                egui::vec2(self.width as f32 * 2.0, self.height as f32 * 2.0),
            ));
        });
    }
}
