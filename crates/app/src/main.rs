//! Spec Chum — ZX Spectrum emulator frontend.
//!
//! With no headless args this opens the egui GUI. Headless debugger / agent HTTP
//! routes through the same binary ([#231](https://github.com/mward-sudo/spec_chum/issues/231)):
//!
//! - `spec_chum --serve …`
//! - `spec_chum debug <debug-cli-args…>`

use std::ffi::OsString;
use std::process::ExitCode;

use app::SpecChumApp;
use eframe::egui;
use spec_chum_host::{default_prefs_path, load_prefs, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH};

fn main() -> ExitCode {
    if let Some(args) = rewrite_headless_args(std::env::args_os().collect()) {
        return match debug_cli::run_from_args(args) {
            Ok(()) => ExitCode::SUCCESS,
            Err(e) => {
                eprintln!("{e:#}");
                ExitCode::FAILURE
            }
        };
    }
    match run_gui() {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::FAILURE
        }
    }
}

/// Detect headless mode and rewrite argv for [`debug_cli`].
///
/// - `spec_chum debug …` → strip the `debug` token, then parse as debug CLI
/// - `spec_chum --serve …` → pass through (top-level agent HTTP server)
fn rewrite_headless_args(mut args: Vec<OsString>) -> Option<Vec<OsString>> {
    if args.len() < 2 {
        return None;
    }
    if args[1].as_os_str() == "debug" {
        args.remove(1);
        return Some(args);
    }
    if args.iter().skip(1).any(|a| a.as_os_str() == "--serve") {
        return Some(args);
    }
    None
}

fn run_gui() -> eframe::Result {
    trace::init_from_env();
    // When SPEC_CHUM_AGENT=1, SpecChumApp embeds loopback HTTP on the same
    // Arc<ControlPlane> / HostSession as the GUI Debug panel (#221).
    // Requires SPEC_CHUM_AGENT_TOKEN or SPEC_CHUM_AGENT_INSECURE=1.
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

#[cfg(test)]
mod tests {
    use super::rewrite_headless_args;
    use std::ffi::OsString;

    fn os(args: &[&str]) -> Vec<OsString> {
        args.iter().map(OsString::from).collect()
    }

    fn as_strings(args: Vec<OsString>) -> Vec<String> {
        args.into_iter()
            .map(|a| a.to_string_lossy().into_owned())
            .collect()
    }

    #[test]
    fn debug_subcommand_strips_token() {
        let out =
            rewrite_headless_args(os(&["spec_chum", "debug", "dump-state"])).expect("headless");
        assert_eq!(as_strings(out), vec!["spec_chum", "dump-state"]);
    }

    #[test]
    fn serve_flag_routes_headless() {
        let out = rewrite_headless_args(os(&["spec_chum", "--serve", "--model", "48k"]))
            .expect("headless");
        assert_eq!(
            as_strings(out),
            vec!["spec_chum", "--serve", "--model", "48k"]
        );
    }

    #[test]
    fn bare_gui_launch_stays_gui() {
        assert!(rewrite_headless_args(os(&["spec_chum"])).is_none());
        assert!(rewrite_headless_args(os(&["spec_chum", "--something-gui-only"])).is_none());
    }
}
