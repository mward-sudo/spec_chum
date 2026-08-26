//! Deferred native file dialog for the standalone Bevy binary.
//!
//! Sync `rfd` during a UI `Interaction::Pressed` beachballs Bevy on macOS.
//! Setting [`OpenMediaDialog::request`] and opening on the next Update tick is enough
//! for the winit host. SpecChumMac uses `NSOpenPanel` instead (see SwiftUI embed).

use bevy::prelude::*;

use crate::host::EmulatorHost;

#[derive(Resource, Debug, Default)]
pub struct OpenMediaDialog {
    /// Set by chrome / ⌘O; consumed by [`run_open_dialog`].
    pub request: bool,
}

#[derive(Debug, Default)]
pub struct FileDialogPlugin;

impl Plugin for FileDialogPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<OpenMediaDialog>()
            .add_systems(Update, run_open_dialog);
    }
}

fn run_open_dialog(mut dialog: ResMut<OpenMediaDialog>, mut host: ResMut<EmulatorHost>) {
    if !dialog.request {
        return;
    }
    dialog.request = false;
    host.status = format!(
        "{} · Choose tape / snapshot…",
        crate::host::model_label(host.session.model())
    );
    if let Some(path) = rfd::FileDialog::new()
        .add_filter("Tape / snapshot", &["tap", "tzx", "sna", "z80"])
        .set_title("Open tape or snapshot")
        .pick_file()
    {
        host.open_media(path);
    } else {
        host.refresh_status_from_session();
    }
}
