//! egui/eframe desktop frontend lifecycle and frame presentation.

use super::{
    display, theme, window_capture, AyStereoMode, ControlPlane, EmulatorSession, JoystickMode,
    Machine, Model, TapeLoadOptions, UiPreferences, UserMachineConfig, MAPPING_DOC,
};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use spec_chum_host::{RomSetupJson, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH};

mod audio;
mod debugger;
mod dialogs;
mod menu;
mod state;

use audio::{start_beeper, BeeperState};

pub struct SpecChumApp {
    pub session: EmulatorSession,
    /// Present when embedded agent HTTP shares the session (#221).
    plane: Option<Arc<ControlPlane>>,
    _agent: Option<agent_server::embedded::EmbeddedServer>,
    /// Own-window capturer for `GET /v1/host/window` (#239); registered on `plane`.
    window_capturer: Option<Arc<control_plane::OwnWindowCapturer>>,
    texture: Option<egui::TextureHandle>,
    beeper: Arc<std::sync::Mutex<BeeperState>>,
    _stream: Option<cpal::Stream>,
    theme_applied: bool,
    /// Optional gamepad (USB/Bluetooth via gilrs). `None` if init failed.
    gilrs: Option<gilrs::Gilrs>,
    /// Host-local preferences (#186); written on change / exit.
    prefs: UiPreferences,
    prefs_path: PathBuf,
    prefs_dirty: bool,
    /// Debounce window-size writes so continuous resize does not save every frame.
    prefs_size_deadline: Option<Instant>,
    /// Create/edit dialog for custom machine profiles (#187).
    config_draft: Option<UserMachineConfig>,
    config_editor_is_new: bool,
    config_editor_error: Option<String>,
    /// Built-in ROM picker when files under `roms/` are missing (#188).
    show_rom_setup: bool,
    rom_setup: Option<RomSetupJson>,
    rom_setup_error: Option<String>,
    next_assets_download: Option<mpsc::Receiver<Result<PathBuf, String>>>,
}

impl std::fmt::Debug for SpecChumApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // Keep drop-order holders (_agent/_stream) out of Debug; prefs/UI fields omitted.
        f.debug_struct("SpecChumApp")
            .field("session", &self.session)
            .field("has_texture", &self.texture.is_some())
            .finish_non_exhaustive()
    }
}

impl SpecChumApp {
    /// egui UI body — callable from `App::update` or headless `Context::run`.
    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.theme_applied {
            theme::apply(ctx);
            self.theme_applied = true;
        }
        self.menu_bar(ctx);

        if !self.session.tick_key_script() {
            let (keys_down, modifiers) = ctx.input(|i| (i.keys_down.clone(), i.modifiers));
            let pad = self.poll_gamepad();
            self.session.sync_keyboard(&keys_down, modifiers, pad);
        }

        if self.session.kempston_mouse {
            {
                let host = &mut *self.session.host_mut();
                if let Some(machine) = host.machine_mut() {
                    let (dx, dy, primary, secondary, middle) = ctx.input(|i| {
                        let d = i.pointer.delta();
                        (
                            d.x.round() as i32,
                            d.y.round() as i32,
                            i.pointer.primary_down(),
                            i.pointer.secondary_down(),
                            i.pointer.middle_down(),
                        )
                    });
                    let mouse = machine.mouse_mut();
                    // Clamp per-frame motion into i8 range for the 8-bit counters.
                    let dx = dx.clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8;
                    let dy = dy.clamp(i32::from(i8::MIN), i32::from(i8::MAX)) as i8;
                    if dx != 0 || dy != 0 {
                        mouse.set_delta(dx, dy);
                    }
                    // Host primary = left; Kempston D0=right, D1=left.
                    mouse.set_buttons(primary, secondary, middle);
                }
            }
        } else {
            let host = &mut *self.session.host_mut();
            if let Some(machine) = host.machine_mut() {
                // Drop guest button state when host mouse input is toggled off mid-press.
                machine.mouse_mut().set_buttons(false, false, false);
            }
        }

        let audio = self.session.tick_frame();
        if let Ok(mut b) = self.beeper.lock() {
            b.queue_frame(audio, self.session.muted, self.session.volume);
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let (image, src) = {
                let host = &*self.session.host_mut();
                let image = egui::ColorImage::from_rgba_unmultiplied(
                    [host.width(), host.height()],
                    host.framebuffer(),
                );
                let src = egui::vec2(host.width() as f32, host.height() as f32);
                (image, src)
            };
            let tex = self.texture.get_or_insert_with(|| {
                ctx.load_texture("screen", image.clone(), egui::TextureOptions::NEAREST)
            });
            tex.set(image, egui::TextureOptions::NEAREST);
            let avail = ui.available_size();
            let fitted = display::fit_size(src, avail);
            if let Some(plane) = self.plane.as_ref() {
                let pw = avail.x.round().max(1.0) as u32;
                let ph = avail.y.round().max(1.0) as u32;
                plane.set_display_panel_size(pw, ph);
            }
            ui.centered_and_justified(|ui| {
                ui.image((tex.id(), fitted));
            });
        });

        self.render_debugger(ctx);
        self.config_editor_window(ctx);
        self.rom_setup_window(ctx);

        let paused = self
            .session
            .host_mut()
            .machine()
            .is_some_and(|m| m.debugger().paused);
        let advancing = self.session.host_mut().running() && !paused;
        if advancing {
            if self.session.throttle {
                ctx.request_repaint_after(std::time::Duration::from_millis(20));
            } else {
                ctx.request_repaint();
            }
        } else if self.session.debug_open || paused {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }
}

impl Default for SpecChumApp {
    fn default() -> Self {
        Self::new()
    }
}

impl eframe::App for SpecChumApp {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        theme::clear_color()
    }

    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.poll_next_assets_download();
        if self.next_assets_download.is_some() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
        if let Some(cap) = self.window_capturer.as_ref() {
            window_capture::refresh_window_id_from_frame(cap, frame);
        }
        let size = ctx.input(|i| i.viewport().inner_rect.map(|r| r.size()));
        if let Some(size) = size {
            let w = size.x.max(MIN_WINDOW_WIDTH);
            let h = size.y.max(MIN_WINDOW_HEIGHT);
            if (self.prefs.window_width - w).abs() > 0.5
                || (self.prefs.window_height - h).abs() > 0.5
            {
                self.prefs.window_width = w;
                self.prefs.window_height = h;
                self.prefs_size_deadline = Some(Instant::now() + Duration::from_millis(750));
            }
        }
        if self
            .prefs_size_deadline
            .is_some_and(|t| Instant::now() >= t)
        {
            self.prefs_size_deadline = None;
            self.mark_prefs_dirty();
        }
        self.ui(ctx);
        self.persist_prefs_if_dirty();
    }

    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.prefs_size_deadline = None;
        self.prefs_dirty = true;
        self.persist_prefs_if_dirty();
    }
}
