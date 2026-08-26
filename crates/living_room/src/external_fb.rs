//! Spectrum RGBA injected from outside (Swift `host_api` owns the session).

use bevy::prelude::*;

use crate::crt::{CrtScreenTexture, SCREEN_H, SCREEN_W};
use crate::glow::FrameGlow;

/// Latest Spectrum framebuffer (RGBA8, `SCREEN_W`×`SCREEN_H`) from the Swift host.
#[derive(Resource, Debug, Clone)]
pub struct ExternalFramebuffer {
    pub rgba: Vec<u8>,
}

impl Default for ExternalFramebuffer {
    fn default() -> Self {
        Self {
            rgba: vec![0u8; (SCREEN_W * SCREEN_H * 4) as usize],
        }
    }
}

#[derive(Debug, Default)]
pub struct ExternalFramebufferPlugin;

impl Plugin for ExternalFramebufferPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ExternalFramebuffer>()
            .add_systems(Update, upload_external_framebuffer);
    }
}

fn upload_external_framebuffer(
    fb: Res<ExternalFramebuffer>,
    mut images: ResMut<Assets<Image>>,
    screen: Option<Res<CrtScreenTexture>>,
    mut glow: ResMut<FrameGlow>,
) {
    if !fb.is_changed() && !fb.is_added() {
        // Still refresh CRT each frame so first paint isn't stuck black.
    }
    glow.update_from_rgba(&fb.rgba, SCREEN_W, SCREEN_H);
    let Some(screen) = screen else {
        return;
    };
    let Some(mut image) = images.get_mut(&screen.0) else {
        return;
    };
    let Some(data) = image.data.as_mut() else {
        return;
    };
    let n = data.len().min(fb.rgba.len());
    data[..n].copy_from_slice(&fb.rgba[..n]);
}
