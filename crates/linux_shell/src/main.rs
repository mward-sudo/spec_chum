//! Native GTK4 Spec Chum shell (`spec_chum_linux`) — [#351](https://github.com/mward-sudo/spec_chum/issues/351).
//!
//! Thin adapter over [`host_api::HostSession`]: GTK4 window + menus, framebuffer
//! present, keyboard → matrix, cpal audio, optional Agent Debug HTTP.

#[cfg(target_os = "linux")]
mod gtk_ui;

#[cfg(target_os = "linux")]
fn main() -> anyhow::Result<()> {
    gtk_ui::run()
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!(
        "spec-chum-linux: the native GTK4 shell only runs on Linux.\n\
         On this host use the egui app: cargo run -p app --release\n\
         See docs/LINUX_NATIVE.md (#351)."
    );
    std::process::exit(1);
}
