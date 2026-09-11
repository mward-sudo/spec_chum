//! Native Win32 Spec Chum shell (`spec_chum_windows`) — [#351](https://github.com/mward-sudo/spec_chum/issues/351).
//!
//! Thin adapter over [`host_api::HostSession`]: classic Win32 window + menus,
//! framebuffer blit, keyboard → matrix, cpal audio, optional Agent Debug HTTP.

#[cfg(windows)]
mod win32;

#[cfg(windows)]
fn main() -> anyhow::Result<()> {
    win32::run()
}

#[cfg(not(windows))]
fn main() {
    eprintln!(
        "spec-chum-windows: the native Win32 shell only runs on Windows.\n\
         On this host use the egui app: cargo run -p app --release\n\
         See docs/WINDOWS_NATIVE.md (#351)."
    );
    std::process::exit(1);
}
