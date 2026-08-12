//! Spec Chum — ZX Spectrum emulator frontend.

use app::SpecChumApp;
use eframe::egui;

fn main() -> eframe::Result {
    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([780.0, 680.0])
        .with_min_inner_size([480.0, 400.0])
        .with_title("Spec Chum");

    #[cfg(target_os = "macos")]
    {
        // Traffic-light friendly chrome: content draws under the titlebar.
        viewport = viewport
            .with_fullsize_content_view(true)
            .with_titlebar_shown(true)
            .with_title_shown(false)
            .with_titlebar_buttons_shown(true);
    }

    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };
    eframe::run_native(
        "Spec Chum",
        options,
        Box::new(|_cc| Ok(Box::new(SpecChumApp::new()))),
    )
}
