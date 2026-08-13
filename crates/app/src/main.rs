//! Spec Chum — ZX Spectrum emulator frontend.

use app::SpecChumApp;
use eframe::egui;

fn main() -> eframe::Result {
    trace::init_from_env();
    // Use a normal titlebar (not fullsize content view). Drawing under the macOS
    // titlebar buried menu hit-tests in the traffic-light / drag region (#60).
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([780.0, 680.0])
        .with_min_inner_size([480.0, 400.0])
        .with_title("Spec Chum");

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
