//! App construction, preference state, model setup, and host input.

use super::{start_beeper, BeeperState, SpecChumApp};
use crate::{ControlPlane, EmulatorSession, JoystickState, Model};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use spec_chum_host::{default_prefs_path, load_prefs, save_prefs, sync_model_rom_paths, PrefModel};

impl SpecChumApp {
    #[must_use]
    pub fn new() -> Self {
        Self::new_with_audio(true)
    }

    /// Construct the app; tests pass `start_audio = false` to avoid cpal devices.
    #[must_use]
    pub fn new_with_audio(start_audio: bool) -> Self {
        Self::new_with_audio_prefs(start_audio, default_prefs_path())
    }

    /// Like [`Self::new_with_audio`], but loads/saves prefs from `prefs_path` (tests).
    #[must_use]
    pub fn new_with_audio_prefs(start_audio: bool, prefs_path: PathBuf) -> Self {
        let mut prefs = load_prefs(&prefs_path);
        sync_model_rom_paths(prefs.model_rom_paths.clone());
        let beeper = Arc::new(std::sync::Mutex::new(BeeperState::with_preferences(
            prefs.muted,
            prefs.volume,
        )));
        let stream = if start_audio {
            start_beeper(Arc::clone(&beeper))
        } else {
            None
        };
        let mut session = EmulatorSession::new(prefs.model.to_model(), true);
        session.throttle = prefs.throttle;
        session.muted = prefs.muted;
        session.volume = prefs.volume;
        session
            .host_mut()
            .set_joystick_mode(prefs.joystick_mode.to_mode());
        session
            .host_mut()
            .set_online_tape_titles(prefs.online_tape_titles);
        session.kempston_mouse = prefs.kempston_mouse;
        if let Some(cfg) = prefs.active_custom_config().cloned() {
            match session.apply_user_machine_config(&cfg) {
                Ok(()) => {
                    prefs.sync_machine_fields_from_config(&cfg);
                    session
                        .host_mut()
                        .set_joystick_mode(cfg.joystick_mode.to_mode());
                    session.kempston_mouse = cfg.kempston_mouse;
                }
                Err(e) => {
                    session.set_model(prefs.model.to_model());
                    session.try_autoload_rom();
                    let prior = session.host_mut().status().to_owned();
                    session
                        .host_mut()
                        .set_status(format!("Config “{}” failed: {e} — {prior}", cfg.name));
                }
            }
        } else {
            session.try_autoload_rom();
        }
        {
            let host = &mut *session.host_mut();
            let model = host.model().to_model();
            if let Some(m) = host.machine_mut() {
                m.set_tape_load_options(prefs.tape_load_options());
                if matches!(
                    model,
                    Model::Spectrum128
                        | Model::SpectrumPlus2
                        | Model::SpectrumPlus2A
                        | Model::SpectrumPlus3
                        | Model::SpectrumPlus3e
                        | Model::Pentagon128
                        | Model::ScorpionZs256
                        | Model::TimexTS2068
                ) {
                    m.set_ay_stereo_mode(prefs.effective_ay_stereo());
                }
            }
        }
        let gilrs = match gilrs::Gilrs::new() {
            Ok(g) => Some(g),
            Err(gilrs::Error::NotImplemented(g)) => Some(g),
            Err(e) => {
                let status = session.host_mut().status().to_owned();
                session
                    .host_mut()
                    .set_status(format!("{status} (gamepad unavailable: {e})"));
                None
            }
        };
        // Only share the HostSession Arc when embedding agent HTTP — keeps the
        // GUI path lock-free (HostSlot::Direct) for normal runs and tests (#221).
        let (plane, agent) = if std::env::var("SPEC_CHUM_AGENT").ok().as_deref() == Some("1") {
            let shared = session.share_host();
            let plane = Arc::new(ControlPlane::from_shared(shared));
            match agent_server::embedded::spawn_from_env_with_plane(Arc::clone(&plane)) {
                Ok(server) => {
                    if let Some(ref s) = server {
                        let status = session.host_mut().status().to_owned();
                        session
                            .host_mut()
                            .set_status(format!("{status} — agent http://{}", s.addr));
                    }
                    (Some(plane), server)
                }
                Err(e) => {
                    eprintln!("spec-chum: embedded agent failed: {e}");
                    let status = session.host_mut().status().to_owned();
                    session
                        .host_mut()
                        .set_status(format!("{status} — agent embed failed: {e}"));
                    (Some(plane), None)
                }
            }
        } else {
            (None, None)
        };
        let window_capturer = plane.as_ref().map(|p| {
            let cap = control_plane::OwnWindowCapturer::new();
            p.set_window_capture(Some(
                Arc::clone(&cap) as Arc<dyn control_plane::HostWindowCapture>
            ));
            cap
        });
        let mut app = Self {
            session,
            plane,
            _agent: agent,
            window_capturer,
            texture: None,
            beeper,
            _stream: stream,
            next_frame_deadline: None,
            theme_applied: false,
            gilrs,
            prefs,
            library_search: String::new(),
            library_category: None,
            selected_media_path: None,
            prefs_path,
            prefs_dirty: false,
            prefs_size_deadline: None,
            config_draft: None,
            config_editor_is_new: false,
            config_editor_error: None,
            show_rom_setup: false,
            rom_setup: None,
            rom_setup_error: None,
            next_assets_download: None,
        };
        app.refresh_rom_setup();
        app.maybe_auto_present_rom_setup();
        app
    }

    /// Shared [`ControlPlane`] when `SPEC_CHUM_AGENT=1` embedded the HTTP server.
    #[must_use]
    pub fn plane(&self) -> Option<&Arc<ControlPlane>> {
        self.plane.as_ref()
    }

    /// Built-in model picked from Machine menu — sync session, prefs, and auto-open ROM dialog when needed.
    pub(super) fn on_builtin_model_selected(&mut self, pick: Model) {
        self.prefs.select_builtin_model(PrefModel::from_model(pick));
        self.session.set_model(pick);
        sync_model_rom_paths(self.prefs.model_rom_paths.clone());
        self.session.try_autoload_rom();
        self.apply_restored_machine_options();
        self.mark_prefs_dirty();
        self.maybe_auto_present_rom_setup();
    }

    pub(super) fn mark_prefs_dirty(&mut self) {
        self.prefs_dirty = true;
    }

    fn sync_prefs_from_session(&mut self) {
        if self.prefs.active_config_id.is_none() {
            self.prefs.set_model_from_machine(self.session.model());
        }
        self.prefs.throttle = self.session.throttle;
        self.prefs.muted = self.session.muted;
        self.prefs.volume = self.session.volume.clamp(0.0, 1.0);
        self.prefs
            .set_joystick(self.session.host_mut().joystick_mode());
        self.prefs.kempston_mouse = self.session.kempston_mouse;
        self.prefs.online_tape_titles = self.session.host_mut().online_tape_titles();
        {
            let host = &mut *self.session.host_mut();
            if let Some(m) = host.machine() {
                self.prefs.set_tape_from_options(m.tape_load_options());
                self.prefs.set_ay_stereo(m.ay_stereo_mode());
            }
        }
    }

    pub(super) fn persist_prefs_if_dirty(&mut self) {
        if !self.prefs_dirty {
            return;
        }
        self.sync_prefs_from_session();
        if save_prefs(&self.prefs_path, &self.prefs).is_ok() {
            sync_model_rom_paths(self.prefs.model_rom_paths.clone());
            self.prefs_dirty = false;
        }
    }

    pub(super) fn note_recent_if_ok(&mut self, path: &Path) {
        let status = self.session.host_mut().status().to_owned();
        if status.starts_with("Inserted")
            || status.starts_with("Loaded")
            || status.starts_with("DSK inserted")
        {
            self.note_recent_file(path);
        }
    }

    fn note_recent_file(&mut self, path: &Path) {
        self.prefs.push_recent(path);
        self.mark_prefs_dirty();
    }

    /// Re-apply tape / AY options after a model ROM reload.
    pub(super) fn apply_restored_machine_options(&mut self) {
        {
            let host = &mut *self.session.host_mut();
            let model = host.model().to_model();
            if let Some(m) = host.machine_mut() {
                m.set_tape_load_options(self.prefs.tape_load_options());
                if matches!(
                    model,
                    Model::Spectrum128
                        | Model::SpectrumPlus2
                        | Model::SpectrumPlus2A
                        | Model::SpectrumPlus3
                        | Model::SpectrumPlus3e
                        | Model::Pentagon128
                        | Model::ScorpionZs256
                        | Model::TimexTS2068
                ) {
                    m.set_ay_stereo_mode(self.prefs.effective_ay_stereo());
                }
            }
        }
    }

    pub(super) fn open_recent_path(&mut self, path: &Path) -> bool {
        let result = self.session.host_mut().open_media_path(path);
        if let Err(error) = result {
            self.session
                .host_mut()
                .set_status(format!("Open failed: {error}"));
            if !path.is_file() {
                self.prefs.remove_recent(path);
                self.mark_prefs_dirty();
            }
            return false;
        }
        self.note_recent_file(path);
        // Snapshot may have switched model — keep prefs in sync.
        self.prefs.set_model_from_machine(self.session.model());
        self.mark_prefs_dirty();
        true
    }

    pub(super) fn poll_gamepad(&mut self) -> JoystickState {
        let Some(gilrs) = self.gilrs.as_mut() else {
            return JoystickState::empty();
        };
        while gilrs.next_event().is_some() {}
        let mut stick = JoystickState::empty();
        for (_, gamepad) in gilrs.gamepads() {
            const DEAD: f32 = 0.5;
            let axis = |a: gilrs::Axis| gamepad.value(a);
            let left = gamepad.is_pressed(gilrs::Button::DPadLeft)
                || axis(gilrs::Axis::LeftStickX) < -DEAD;
            let right = gamepad.is_pressed(gilrs::Button::DPadRight)
                || axis(gilrs::Axis::LeftStickX) > DEAD;
            let up =
                gamepad.is_pressed(gilrs::Button::DPadUp) || axis(gilrs::Axis::LeftStickY) > DEAD;
            let down = gamepad.is_pressed(gilrs::Button::DPadDown)
                || axis(gilrs::Axis::LeftStickY) < -DEAD;
            stick.left |= left;
            stick.right |= right;
            stick.up |= up;
            stick.down |= down;
            stick.fire |= gamepad.is_pressed(gilrs::Button::South)
                || gamepad.is_pressed(gilrs::Button::East)
                || gamepad.is_pressed(gilrs::Button::West)
                || gamepad.is_pressed(gilrs::Button::North);
        }
        stick
    }
}
