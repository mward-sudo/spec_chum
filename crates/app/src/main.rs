//! Spec Chum — ZX Spectrum emulator frontend.

use app::SpecChumApp;
use eframe::egui;
use spec_chum_host::{default_prefs_path, load_prefs, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH};

fn main() -> eframe::Result {
    trace::init_from_env();
    // egui Debug uses HostSession (same type as control_plane). Parallel
    // SPEC_CHUM_AGENT=1 embed was removed (#221) — use standalone
    // `spec-chum-agent` / `spec-chum-debug --serve` for HTTP agents.
    if std::env::var("SPEC_CHUM_AGENT").ok().as_deref() == Some("1") {
        eprintln!(
            "SPEC_CHUM_AGENT=1 is no longer supported in egui (dual-session removed; #221). \
             Start standalone: cargo run -p agent_server --release"
        );
    }
    let prefs = load_prefs(&default_prefs_path());
    let width = prefs.window_width.max(MIN_WINDOW_WIDTH);
    let height = prefs.window_height.max(MIN_WINDOW_HEIGHT);
    // Use a normal titlebar (not fullsize content view). Drawing under the macOS
    // titlebar buried menu hit-tests in the traffic-light / drag region (#60).
    let viewport = egui::ViewportBuilder::default()
        .with_inner_size([width, height])
        .with_min_inner_size([MIN_WINDOW_WIDTH, MIN_WINDOW_HEIGHT])
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
