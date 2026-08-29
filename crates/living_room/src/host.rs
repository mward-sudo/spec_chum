//! HostSession resource + 50 Hz tick + input after camera lock.

use std::path::PathBuf;
use std::time::Duration;

use bevy::prelude::*;
use machine::TapeLoadOptions;
use spec_chum_host::{HostSession, ModelId};

use crate::audio::AudioOut;
use crate::camera::CameraLocked;
use crate::crt::{CrtScreenTexture, SCREEN_H, SCREEN_W};
use crate::glow::FrameGlow;
use crate::keymap;

const FRAME_PERIOD: Duration = Duration::from_millis(20);

/// Short label for overlay / status (matches egui Machine menu naming).
#[must_use]
pub fn model_label(model: ModelId) -> &'static str {
    match model {
        ModelId::Spectrum48 => "48K",
        ModelId::Spectrum128 => "128K",
        ModelId::SpectrumPlus2A => "+2A",
        ModelId::SpectrumPlus3 => "+3",
    }
}

#[derive(Resource)]
pub struct EmulatorHost {
    pub session: HostSession,
    pub accumulator: Duration,
    pub status: String,
    pub paused_overlay: bool,
}

impl std::fmt::Debug for EmulatorHost {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmulatorHost")
            .field("status", &self.status)
            .field("paused_overlay", &self.paused_overlay)
            .field("accumulator", &self.accumulator)
            .field("model", &self.session.model())
            .finish_non_exhaustive()
    }
}

#[derive(Debug, Default)]
pub struct HostPlugin;

impl Plugin for HostPlugin {
    fn build(&self, app: &mut App) {
        app.insert_resource(EmulatorHost::boot())
            .add_systems(Update, (host_hotkeys, tick_emulator).chain());
    }
}

impl EmulatorHost {
    fn boot() -> Self {
        let prefs = spec_chum_host::load_prefs(&spec_chum_host::default_prefs_path());
        let model = prefs.model.to_model_id();
        let mut host = Self {
            session: HostSession::new(model, true),
            accumulator: Duration::ZERO,
            status: String::new(),
            paused_overlay: false,
        };
        // Single path: selection always goes through select_model + ROM autoload.
        host.select_model(model);
        let _ = host
            .session
            .set_tape_load_options(prefs.tape_load_options());
        host
    }

    /// Switch emulated model; always updates [`HostSession::model`] then autoloads ROM.
    pub fn select_model(&mut self, model: ModelId) {
        let label = model_label(model);
        self.status = match self.session.select_model(model) {
            Ok(()) => format!("{label} · {}", self.session.status()),
            Err(e) => format!("{label} · {e}"),
        };
        debug_assert_eq!(self.session.model(), model);
        if self.paused_overlay {
            self.session.set_paused(true);
        }
    }

    pub fn refresh_status_from_session(&mut self) {
        let label = model_label(self.session.model());
        self.status = format!("{label} · {}", self.session.status());
    }

    /// Soft reset: keep inserted tape / disk / model (unlike [`Self::select_model`]).
    pub fn reset(&mut self) {
        match self.session.reset() {
            Ok(()) => self.refresh_status_from_session(),
            Err(e) => {
                self.status = format!("{} · {e}", model_label(self.session.model()));
            }
        }
    }

    pub fn open_media(&mut self, path: PathBuf) {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        let result = match ext.as_str() {
            "tap" | "tzx" => self.session.open_tape(&path),
            // Snapshot path forces Spectrum48 inside host_api and reloads ROM.
            "sna" | "z80" => self.session.load_snapshot(&path),
            _ => Err(spec_chum_host::HostError::Message(format!(
                "unsupported extension: {ext}"
            ))),
        };
        match result {
            Ok(()) => self.refresh_status_from_session(),
            Err(e) => {
                self.status = format!("{} · {e}", model_label(self.session.model()));
            }
        }
    }

    pub fn set_play_ear(&mut self) {
        let opts = TapeLoadOptions {
            flash_load: false,
            speed: 1,
            ..Default::default()
        };
        let _ = self.session.set_tape_load_options(opts);
        match self.session.play_tape() {
            Ok(()) => self.refresh_status_from_session(),
            Err(e) => {
                self.status = format!("{} · {e}", model_label(self.session.model()));
            }
        }
    }

    pub fn set_instant(&mut self) {
        let opts = TapeLoadOptions {
            flash_load: true,
            speed: 1,
            ..Default::default()
        };
        match self.session.set_tape_load_options(opts) {
            Ok(()) => {
                let _ = self.session.play_tape();
                self.refresh_status_from_session();
            }
            Err(e) => {
                self.status = format!("{} · {e}", model_label(self.session.model()));
            }
        }
    }
}

fn host_hotkeys(
    keys: Res<ButtonInput<KeyCode>>,
    locked: Option<Res<CameraLocked>>,
    mut host: ResMut<EmulatorHost>,
) {
    // Bare letters are Spectrum matrix — host actions live on toolbar / ⌘ shortcuts
    // (see `ui_overlay`). F1–F3 never hit the matrix.
    if keys.just_pressed(KeyCode::F1) {
        host.select_model(ModelId::Spectrum48);
    } else if keys.just_pressed(KeyCode::F2) {
        host.select_model(ModelId::Spectrum128);
    } else if keys.just_pressed(KeyCode::F3) {
        host.select_model(ModelId::SpectrumPlus3);
    }

    // Intro skip still uses Esc before lock; after lock Esc pauses (not a Spectrum key).
    if locked.is_none() {
        return;
    }
    if keys.just_pressed(KeyCode::Escape) {
        host.paused_overlay = !host.paused_overlay;
        let paused = host.paused_overlay;
        host.session.set_paused(paused);
        let label = model_label(host.session.model());
        host.status = if paused {
            format!("{label} · Paused")
        } else {
            format!("{label} · Running")
        };
    }
}

fn tick_emulator(
    time: Res<Time>,
    locked: Option<Res<CameraLocked>>,
    mut host: ResMut<EmulatorHost>,
    mut images: ResMut<Assets<Image>>,
    screen: Option<Res<CrtScreenTexture>>,
    mut glow: ResMut<FrameGlow>,
    audio: Option<Res<AudioOut>>,
    keys: Res<ButtonInput<KeyCode>>,
) {
    if host.paused_overlay {
        return;
    }

    host.accumulator += time.delta();
    let mut frames = 0u32;
    while host.accumulator >= FRAME_PERIOD && frames < 2 {
        host.accumulator -= FRAME_PERIOD;
        frames += 1;

        if locked.is_some() {
            // While ⌘/Ctrl is held, keys are host shortcuts — don't poke the matrix.
            let host_mods = keys.pressed(KeyCode::SuperLeft)
                || keys.pressed(KeyCode::SuperRight)
                || keys.pressed(KeyCode::ControlLeft)
                || keys.pressed(KeyCode::ControlRight);
            let _ = host.session.clear_keys();
            if !host_mods {
                for (row, bit) in keymap::matrix_from_bevy(&keys) {
                    let _ = host.session.set_key(row, bit, true);
                }
            }
        }

        host.session.run_frame();

        // Drop PCM during intro (AudioOut is also gated until CameraLocked).
        if locked.is_some() {
            if let Some(audio) = audio.as_ref() {
                audio.push_pcm(host.session.audio_pcm());
            }
        }

        glow.update_from_rgba(host.session.framebuffer(), SCREEN_W, SCREEN_H);

        if let Some(screen) = screen.as_ref() {
            if let Some(mut image) = images.get_mut(&screen.0) {
                if let Some(data) = image.data.as_mut() {
                    let src = host.session.framebuffer();
                    let n = data.len().min(src.len());
                    data[..n].copy_from_slice(&src[..n]);
                }
            }
        }
    }
    // Cap leftover after max catch-up so a hitch doesn't spiral unbounded debt.
    if frames >= 2 {
        host.accumulator = host.accumulator.min(FRAME_PERIOD);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn select_model_keeps_session_model_matching() {
        let mut host = EmulatorHost {
            session: HostSession::new(ModelId::Spectrum48, true),
            accumulator: Duration::ZERO,
            status: String::new(),
            paused_overlay: false,
        };
        for model in [
            ModelId::Spectrum48,
            ModelId::Spectrum128,
            ModelId::SpectrumPlus3,
        ] {
            host.select_model(model);
            assert_eq!(host.session.model(), model);
            assert!(
                host.status.starts_with(model_label(model)),
                "status should lead with model label: {}",
                host.status
            );
        }
    }

    #[test]
    fn model_labels_match_menu_names() {
        assert_eq!(model_label(ModelId::Spectrum48), "48K");
        assert_eq!(model_label(ModelId::Spectrum128), "128K");
        assert_eq!(model_label(ModelId::SpectrumPlus2A), "+2A");
        assert_eq!(model_label(ModelId::SpectrumPlus3), "+3");
    }
}
