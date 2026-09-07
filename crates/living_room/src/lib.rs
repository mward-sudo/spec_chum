//! Experimental Bevy living-room CRT host (see `docs/LIVING_ROOM.md`, issue #146).

// Bevy system signatures use elided lifetimes extensively; workspace rust_2018_idioms warns.
#![allow(elided_lifetimes_in_paths)]

pub mod agent_embed;
#[cfg(feature = "standalone")]
pub mod audio;
pub mod camera;
pub mod crt;
pub mod external_fb;
pub mod fb_scale;
pub mod ffi;
#[cfg(feature = "standalone")]
pub mod file_dialog;
pub mod glow;
pub mod headless;
#[cfg(feature = "standalone")]
pub mod host;
pub mod hybrid;
pub mod image_copy;
#[cfg(feature = "standalone")]
pub mod keymap;
pub mod perf;
pub mod present;
#[cfg(target_os = "macos")]
pub mod present_metal;
pub mod quality;
pub mod room;
#[cfg(feature = "standalone")]
pub mod ui_overlay;

pub use external_fb::ExternalFramebuffer;
pub use headless::{HeadlessRoom, HeadlessRoomError, DEFAULT_ROOM_H, DEFAULT_ROOM_W};
#[cfg(target_os = "macos")]
pub use present_metal::PresentIosurfaceError;

/// Keep `host_api` linked into the embed staticlib so SpecChumMac resolves `sc_*`
/// even when the `standalone` feature (and `host` module) is off.
#[doc(hidden)]
// Force-link host_api + agent FFI into the embed staticlib (#171).
#[allow(dead_code, unused_imports)]
mod ensure_host_api_linked {
    use spec_chum_host::{HostSession, ModelId};

    #[inline(never)]
    pub fn touch() {
        let _ = std::mem::size_of::<HostSession>();
        let _ = ModelId::Spectrum48;
        // Reference the FFI module so its `#[no_mangle]` symbols land in the .a.
        let _ = spec_chum_host::ffi::sc_create
            as unsafe extern "C" fn(u32, i32) -> *mut std::ffi::c_void;
        let _ = crate::agent_embed::sc_agent_embed_start
            as unsafe extern "C" fn(*mut std::ffi::c_void) -> i32;
    }

    #[used]
    static FORCE_LINK: fn() = touch;
}

use std::path::PathBuf;

use bevy::prelude::*;

#[cfg(feature = "standalone")]
mod standalone_app {
    use bevy::prelude::*;

    use crate::asset_plugin;
    use crate::audio::AudioPlugin;
    use crate::camera::CameraPlugin;
    use crate::crt::CrtPlugin;
    use crate::file_dialog::FileDialogPlugin;
    use crate::glow::GlowPlugin;
    use crate::host::HostPlugin;
    use crate::hybrid::HybridPlugin;
    use crate::room::RoomPlugin;
    use crate::ui_overlay::UiOverlayPlugin;

    /// Standalone winit living-room app (Bevy chrome / cpal / HostSession).
    pub fn living_room_app() -> App {
        let mut app = App::new();
        app.add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Spec Chum — Living Room (experimental)".into(),
                        resolution: (1280, 720).into(),
                        ..default()
                    }),
                    ..default()
                })
                .set(asset_plugin()),
        )
        .add_plugins((
            HostPlugin,
            AudioPlugin,
            CrtPlugin,
            RoomPlugin,
            CameraPlugin,
            GlowPlugin,
            HybridPlugin,
            FileDialogPlugin,
            UiOverlayPlugin,
        ));
        app
    }
}

#[cfg(feature = "standalone")]
pub use standalone_app::living_room_app;

/// Resolve the living-room asset root at runtime (relocatable `.app` + worktree).
///
/// Order:
/// 1. `SPEC_CHUM_LIVING_ROOM_ASSETS`
/// 2. `SPEC_CHUM_ROOT/crates/living_room/assets`
/// 3. `../Resources/living_room_assets` relative to the executable (bundled app)
/// 4. `CARGO_MANIFEST_DIR/assets` (dev `cargo run`)
pub fn resolve_asset_root() -> PathBuf {
    if let Ok(p) = std::env::var("SPEC_CHUM_LIVING_ROOM_ASSETS") {
        let path = PathBuf::from(p);
        if path.is_dir() {
            return path;
        }
    }
    if let Ok(root) = std::env::var("SPEC_CHUM_ROOT") {
        let path = PathBuf::from(root).join("crates/living_room/assets");
        if path.is_dir() {
            return path;
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        if let Some(macos_dir) = exe.parent() {
            // SpecChumMac.app/Contents/MacOS/binary → Contents/Resources/living_room_assets
            if let Some(contents) = macos_dir.parent() {
                let bundled = contents.join("Resources/living_room_assets");
                if bundled.is_dir() {
                    return bundled;
                }
            }
        }
    }
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("assets")
}

/// Asset root for Bevy (standalone + headless embed).
pub fn asset_plugin() -> AssetPlugin {
    AssetPlugin {
        file_path: resolve_asset_root().to_string_lossy().into_owned(),
        ..default()
    }
}
