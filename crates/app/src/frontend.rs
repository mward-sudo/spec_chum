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
use spec_chum_host::{MediaCategory, RomSetupJson, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH};

mod audio;
mod debugger;
mod dialogs;
mod menu;
mod navigation;
mod state;

use audio::{start_beeper, BeeperState};
use navigation::{should_suppress_guest_keyboard, FrontendView, GUEST_KEYBOARD_SUPPRESSED_ID};

const CPU_TSTATES_PER_SECOND: f64 = 3_500_000.0;

fn frame_repaint_delay(frame_tstates: Option<u32>) -> Duration {
    frame_tstates.map_or(Duration::from_millis(20), |tstates| {
        Duration::from_secs_f64(f64::from(tstates) / CPU_TSTATES_PER_SECOND)
    })
}

fn frame_is_due(next_frame_deadline: Option<Instant>, now: Instant) -> bool {
    next_frame_deadline.is_none_or(|deadline| now >= deadline)
}

fn next_frame_deadline(
    previous: Option<Instant>,
    frame_started: Instant,
    frame_delay: Duration,
) -> Instant {
    let base = previous
        .filter(|deadline| frame_started.saturating_duration_since(*deadline) < frame_delay)
        .unwrap_or(frame_started);
    base + frame_delay
}

fn should_step_key_script(advancing: bool, throttled: bool, due: bool) -> bool {
    advancing && (!throttled || due)
}

pub struct SpecChumApp {
    pub session: EmulatorSession,
    /// Present when embedded agent HTTP shares the session (#221).
    plane: Option<Arc<ControlPlane>>,
    _agent: Option<agent_server::embedded::EmbeddedServer>,
    /// Own-window capturer for `GET /v1/host/window` (#239); registered on `plane`.
    window_capturer: Option<Arc<control_plane::OwnWindowCapturer>>,
    texture: Option<egui::TextureHandle>,
    machine_images: std::collections::BTreeMap<&'static str, Option<egui::TextureHandle>>,
    beeper: Arc<std::sync::Mutex<BeeperState>>,
    _stream: Option<cpal::Stream>,
    next_frame_deadline: Option<Instant>,
    theme_applied: bool,
    /// Optional gamepad (USB/Bluetooth via gilrs). `None` if init failed.
    gilrs: Option<gilrs::Gilrs>,
    /// Host-local preferences (#186); written on change / exit.
    prefs: UiPreferences,
    library_search: String,
    library_category: Option<MediaCategory>,
    selected_media_path: Option<String>,
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
    /// Requested built-in model; the live machine stays active during ROM setup.
    pending_builtin_model: Option<Model>,
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
            theme::apply(ctx, self.prefs.appearance);
            self.theme_applied = true;
        }
        self.menu_bar(ctx);
        let route_changed = self.navigation_bar(ctx);
        let view = FrontendView::get(ctx);

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

        let paused = self.session.host_mut().paused();
        let advancing = self.session.host_mut().running() && !paused;
        let throttled = self.session.throttle && advancing;
        if !throttled {
            self.next_frame_deadline = None;
        }
        let frame_started = Instant::now();
        let due = frame_is_due(self.next_frame_deadline, frame_started);
        let repaint_delay = if !throttled || due {
            let script_consumed_input =
                should_step_key_script(advancing, throttled, due) && self.session.tick_key_script();
            if !script_consumed_input {
                let (mut keys_down, modifiers) = ctx.input(|i| (i.keys_down.clone(), i.modifiers));
                let previous_suppression = ctx.data(|data| {
                    data.get_temp::<bool>(egui::Id::new(GUEST_KEYBOARD_SUPPRESSED_ID))
                        .unwrap_or(false)
                });
                let suppress_guest_keys = should_suppress_guest_keyboard(
                    view,
                    route_changed,
                    previous_suppression,
                    !keys_down.is_empty(),
                );
                ctx.data_mut(|data| {
                    data.insert_temp(
                        egui::Id::new(GUEST_KEYBOARD_SUPPRESSED_ID),
                        suppress_guest_keys,
                    );
                });
                if suppress_guest_keys {
                    keys_down.clear();
                }
                let pad = self.poll_gamepad();
                self.session.sync_keyboard(&keys_down, modifiers, pad);
            }
            let audio = self.session.tick_frame();
            let frame_delay = frame_repaint_delay(audio.frame_tstates);
            if throttled {
                self.next_frame_deadline = Some(next_frame_deadline(
                    self.next_frame_deadline,
                    frame_started,
                    frame_delay,
                ));
            }
            if let Ok(mut b) = self.beeper.lock() {
                b.queue_frame(
                    &audio,
                    self.session.muted,
                    self.session.volume,
                    self.session.throttle,
                );
            }
            frame_delay
        } else {
            self.next_frame_deadline
                .map_or(Duration::from_millis(20), |deadline| {
                    deadline.saturating_duration_since(frame_started)
                })
        };

        egui::CentralPanel::default().show(ctx, |ui| {
            self.render_view(ui, ctx, view, route_changed);
        });

        if view != FrontendView::Debugger {
            self.render_debugger(ctx);
        }
        self.config_editor_window(ctx);
        self.rom_setup_window(ctx);

        if advancing {
            if self.session.throttle {
                ctx.request_repaint_after(repaint_delay);
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
    fn clear_color(&self, visuals: &egui::Visuals) -> [f32; 4] {
        theme::clear_color(visuals)
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

#[cfg(test)]
mod tests {
    use super::navigation::{should_suppress_guest_keyboard, FrontendView};
    use super::{frame_is_due, frame_repaint_delay, next_frame_deadline, should_step_key_script};
    use std::time::{Duration, Instant};

    #[test]
    fn builtin_rom_setup_preserves_active_profile_until_boot_succeeds() {
        use crate::{Machine, Model, SpecChumApp, UserMachineConfig};
        use spec_chum_host::{model_rom_path_key, PrefModel};

        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock after Unix epoch")
            .as_nanos();
        let unique = format!("spec-chum-model-selection-{}-{stamp}", std::process::id());
        let rom_path = std::env::temp_dir().join(format!("{unique}.rom"));
        let prefs_path = std::env::temp_dir().join(format!("{unique}.json"));
        let mut app = SpecChumApp::new_with_audio_prefs(false, prefs_path.clone());
        app.session
            .host_mut()
            .set_machine(Machine::new_48k(&vec![0; 16 * 1024]).expect("test ROM"));
        app.session
            .host_mut()
            .poke(0xc000, 0x5a)
            .expect("writable RAM");
        let profile = UserMachineConfig::new_named("Current custom", PrefModel::Spectrum48);
        app.prefs.custom_configs.push(profile.clone());
        app.prefs.active_config_id = Some(profile.id.clone());
        app.prefs.model_rom_paths.insert(
            model_rom_path_key(PrefModel::Spectrum16K, "main"),
            rom_path.display().to_string(),
        );

        app.on_builtin_model_selected(Model::Spectrum16K);
        assert!(app.show_rom_setup);
        assert_eq!(app.pending_builtin_model, Some(Model::Spectrum16K));
        assert_eq!(app.session.model(), Model::Spectrum48);
        assert_eq!(
            app.prefs.active_config_id.as_deref(),
            Some(profile.id.as_str())
        );
        app.finish_rom_setup();
        assert!(app.rom_setup_error.is_some());
        assert_eq!(
            app.session.host_mut().peek(0xc000).expect("old machine"),
            0x5a
        );
        app.close_rom_setup();
        assert_eq!(app.pending_builtin_model, None);
        assert_eq!(
            app.prefs.active_config_id.as_deref(),
            Some(profile.id.as_str())
        );

        std::fs::write(&rom_path, vec![0; 16 * 1024]).expect("test target ROM");
        app.on_builtin_model_selected(Model::Spectrum16K);
        assert_eq!(app.session.model(), Model::Spectrum16K);
        assert!(app.session.host_mut().has_machine());
        assert_eq!(app.prefs.active_config_id, None);
        assert!(app
            .prefs
            .custom_configs
            .iter()
            .any(|cfg| cfg.id == profile.id));
        assert!(!app.show_rom_setup);

        spec_chum_host::sync_model_rom_paths(std::collections::BTreeMap::default());
        let _ = std::fs::remove_file(rom_path);
        let _ = std::fs::remove_file(prefs_path);
    }

    #[test]
    fn repaint_delay_tracks_next_frame_clock_duration() {
        assert_eq!(
            frame_repaint_delay(Some(70_908)),
            Duration::from_secs_f64(70_908.0 / 3_500_000.0)
        );
        assert_eq!(frame_repaint_delay(None), Duration::from_millis(20));
    }

    #[test]
    fn throttled_frame_scheduler_skips_early_ui_repaints() {
        let now = Instant::now();
        let deadline = now + Duration::from_millis(20);
        assert!(frame_is_due(None, now));
        assert!(!frame_is_due(Some(deadline), now));
        assert!(frame_is_due(Some(deadline), deadline));
    }

    #[test]
    fn throttled_frame_deadline_preserves_cadence_and_resynchronizes_after_stalls() {
        let start = Instant::now();
        let delay = Duration::from_millis(20);
        let previous = start + delay;
        assert_eq!(
            next_frame_deadline(
                Some(previous),
                start + delay + Duration::from_millis(3),
                delay
            ),
            start + delay * 2
        );
        assert_eq!(
            next_frame_deadline(
                Some(previous),
                start + delay * 2 + Duration::from_millis(1),
                delay
            ),
            start + delay * 3 + Duration::from_millis(1)
        );
    }

    #[test]
    fn scripted_keys_only_advance_with_guest_frames() {
        assert!(should_step_key_script(true, false, false));
        assert!(should_step_key_script(true, true, true));
        assert!(!should_step_key_script(true, true, false));
        assert!(!should_step_key_script(false, false, true));
    }

    #[test]
    fn navigation_keys_do_not_reach_the_guest_and_suppression_clears_after_release() {
        assert!(should_suppress_guest_keyboard(
            FrontendView::Library,
            false,
            false,
            true,
        ));
        assert!(should_suppress_guest_keyboard(
            FrontendView::Play,
            true,
            false,
            true,
        ));
        assert!(should_suppress_guest_keyboard(
            FrontendView::Play,
            false,
            true,
            true,
        ));
        assert!(!should_suppress_guest_keyboard(
            FrontendView::Play,
            false,
            true,
            false,
        ));
        assert!(!should_suppress_guest_keyboard(
            FrontendView::Play,
            false,
            false,
            true,
        ));
    }
}
