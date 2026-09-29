//! egui/eframe desktop frontend and host audio output.

use super::{
    display, theme, window_capture, AyStereoMode, ControlPlane, EmulatorSession, JoystickMode,
    JoystickState, Machine, Model, TapeLoadOptions, UiPreferences, UserMachineConfig, MAPPING_DOC,
};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eframe::egui;
use spec_chum_host::{
    default_prefs_path, hardware_compat, install_model_rom, load_prefs, model_requires_user_rom,
    model_rom_available, rom_setup_json, save_prefs, sync_model_rom_paths, PrefAyStereo,
    PrefJoystick, PrefModel, RomSetupJson, MIN_WINDOW_HEIGHT, MIN_WINDOW_WIDTH,
};

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

struct BeeperState {
    edges: Vec<(u32, bool)>,
    ay_samples: Vec<f32>,
    ay_left: Vec<f32>,
    ay_right: Vec<f32>,
    ay_index: usize,
    level: bool,
    sample_rate: u32,
    channels: u16,
    frame_t_per_sample: f32,
    t: f32,
    muted: bool,
    /// Linear host output gain 0…1.
    volume: f32,
}

impl Default for BeeperState {
    fn default() -> Self {
        Self {
            edges: Vec::new(),
            ay_samples: Vec::new(),
            ay_left: Vec::new(),
            ay_right: Vec::new(),
            ay_index: 0,
            level: false,
            sample_rate: 44100,
            channels: 2,
            frame_t_per_sample: 69888.0 / 44100.0,
            t: 0.0,
            muted: false,
            volume: 1.0,
        }
    }
}

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
        let beeper = Arc::new(std::sync::Mutex::new(BeeperState {
            frame_t_per_sample: 69888.0 / 44100.0,
            muted: prefs.muted,
            volume: prefs.volume,
            ..BeeperState::default()
        }));
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
            theme_applied: false,
            gilrs,
            prefs,
            prefs_path,
            prefs_dirty: false,
            prefs_size_deadline: None,
            config_draft: None,
            config_editor_is_new: false,
            config_editor_error: None,
            show_rom_setup: false,
            rom_setup: None,
            rom_setup_error: None,
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

    fn needs_rom_setup(&self) -> bool {
        if self.prefs.active_config_id.is_some() {
            return false;
        }
        if self.rom_setup.as_ref().is_some_and(|doc| !doc.complete) {
            return true;
        }
        !model_rom_available(
            PrefModel::from_model(self.session.model()).to_model_id(),
            &self.prefs.model_rom_paths,
        )
    }

    /// Top-bar ROMs affordance for user-ROM models or when built-in ROM files are missing/invalid.
    fn show_roms_toolbar_button(&self) -> bool {
        if self.prefs.active_config_id.is_some() {
            return false;
        }
        self.needs_rom_setup()
            || model_requires_user_rom(PrefModel::from_model(self.session.model()).to_model_id())
    }

    fn refresh_rom_setup(&mut self) {
        self.rom_setup_error = None;
        self.rom_setup = Some(rom_setup_json(
            PrefModel::from_model(self.session.model()).to_model_id(),
            &self.prefs.model_rom_paths,
        ));
    }

    /// Built-in model picked from Machine menu — sync session, prefs, and auto-open ROM dialog when needed.
    fn on_builtin_model_selected(&mut self, pick: Model) {
        self.prefs.select_builtin_model(PrefModel::from_model(pick));
        self.session.set_model(pick);
        sync_model_rom_paths(self.prefs.model_rom_paths.clone());
        self.session.try_autoload_rom();
        self.apply_restored_machine_options();
        self.mark_prefs_dirty();
        self.maybe_auto_present_rom_setup();
    }

    /// Auto-open ROM setup when the active built-in model still lacks valid ROM files.
    fn maybe_auto_present_rom_setup(&mut self) {
        self.refresh_rom_setup();
        self.show_rom_setup = self.needs_rom_setup();
        if !self.show_rom_setup {
            return;
        }
        if let Some(doc) = &self.rom_setup {
            if !doc.complete {
                self.session
                    .host_mut()
                    .set_status(format!("ROMs required for {}", doc.model_title));
            }
        }
    }

    fn finish_rom_setup(&mut self) {
        self.session.try_autoload_rom();
        self.apply_restored_machine_options();
        if self.session.host_mut().has_machine() {
            self.show_rom_setup = false;
            self.rom_setup_error = None;
        }
        self.refresh_rom_setup();
    }

    fn mark_prefs_dirty(&mut self) {
        self.prefs_dirty = true;
    }

    fn sync_prefs_from_custom_config(&mut self, cfg: &UserMachineConfig) {
        self.prefs.sync_machine_fields_from_config(cfg);
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

    fn persist_prefs_if_dirty(&mut self) {
        if !self.prefs_dirty {
            return;
        }
        self.sync_prefs_from_session();
        if save_prefs(&self.prefs_path, &self.prefs).is_ok() {
            sync_model_rom_paths(self.prefs.model_rom_paths.clone());
            self.prefs_dirty = false;
        }
    }

    fn note_recent_if_ok(&mut self, path: &Path) {
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
    fn apply_restored_machine_options(&mut self) {
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

    fn open_recent_path(&mut self, path: &Path) {
        if !path.is_file() {
            self.session
                .host_mut()
                .set_status(format!("Recent file missing: {}", path.display()));
            self.prefs.recent_files.retain(|p| Path::new(p) != path);
            self.mark_prefs_dirty();
            return;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "sna" | "z80" => self.session.load_snapshot(path),
            "tap" => self.session.load_tap(path),
            "tzx" => self.session.load_tzx(path),
            "rzx" => self.session.load_rzx(path),
            "dsk" => self.session.load_dsk(path),
            _ => {
                self.session
                    .host_mut()
                    .set_status(format!("Unknown recent type: {}", path.display()));
                return;
            }
        }
        if !self
            .session
            .host_mut()
            .status()
            .to_ascii_lowercase()
            .contains("error")
            && !self.session.host_mut().status().contains("before")
            && !self.session.host_mut().status().contains("Missing")
        {
            self.note_recent_if_ok(path);
        }
        // Snapshot may have switched model — keep prefs in sync.
        self.prefs.set_model_from_machine(self.session.model());
        self.mark_prefs_dirty();
    }

    fn poll_gamepad(&mut self) -> JoystickState {
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

    fn path_field(ui: &mut egui::Ui, label: &str, path: &mut Option<String>, filter: &str) {
        ui.horizontal(|ui| {
            ui.label(label);
            let display = path.as_deref().unwrap_or("(default / none)");
            ui.label(display);
            if ui.button("Browse…").clicked() {
                if let Some(picked) = rfd::FileDialog::new()
                    .add_filter(filter, &["rom", "bin", "img", "eeprom"])
                    // DiagROM and similar dumps often ship without an extension.
                    .add_filter("All files", &["*"])
                    .pick_file()
                {
                    *path = picked.to_str().map(str::to_owned);
                }
            }
            if path.is_some() && ui.button("Clear").clicked() {
                *path = None;
            }
        });
    }

    fn rom_setup_window(&mut self, ctx: &egui::Context) {
        let mut open = self.show_rom_setup;
        let mut load_machine = false;
        let mut close = false;
        egui::Window::new("Required ROMs")
            .open(&mut open)
            .default_width(520.0)
            .default_height(360.0)
            .show(ctx, |ui| {
                let Some(doc) = self.rom_setup.clone() else {
                    ui.label("Could not load ROM requirements.");
                    return;
                };
                ui.label(&doc.model_title);
                ui.separator();
                if doc.fetchable {
                    ui.weak(
                        "System ROMs are not shipped — run ./scripts/fetch_roms.sh or choose files below (path remembered across restarts).",
                    );
                } else {
                    ui.weak(
                        "User-provided ROM dumps — choose each file below (path remembered across restarts).",
                    );
                }
                ui.separator();
                for slot in &doc.slots {
                    ui.group(|ui| {
                        ui.horizontal(|ui| {
                            ui.strong(&slot.label);
                            let (label, color) = match slot.status.as_str() {
                                "found" => ("Found", egui::Color32::GREEN),
                                "wrong_size" => ("Wrong size", egui::Color32::YELLOW),
                                _ => ("Missing", egui::Color32::RED),
                            };
                            ui.colored_label(color, label);
                        });
                        ui.monospace(format!(
                            "→ {} ({} KiB)",
                            slot.install_path,
                            slot.expected_bytes / 1024
                        ));
                        if let Some(path) = &slot.resolved_path {
                            ui.weak(path);
                        }
                        ui.weak(&slot.hint);
                        if ui.button(format!("Choose {}…", slot.label)).clicked() {
                            if let Some(picked) = rfd::FileDialog::new()
                                .add_filter("ROM", &["rom", "bin"])
                                .pick_file()
                            {
                                let model =
                                    PrefModel::from_model(self.session.model()).to_model_id();
                                match install_model_rom(
                                    model,
                                    &slot.id,
                                    &picked,
                                    &mut self.prefs.model_rom_paths,
                                ) {
                                    Ok(dest) => {
                                        sync_model_rom_paths(self.prefs.model_rom_paths.clone());
                                        self.mark_prefs_dirty();
                                        self.session.host_mut().set_status(format!(
                                            "Installed {} → {}",
                                            picked.display(),
                                            dest.display()
                                        ));
                                        self.refresh_rom_setup();
                                    }
                                    Err(e) => self.rom_setup_error = Some(e.to_string()),
                                }
                            }
                        }
                    });
                    ui.add_space(6.0);
                }
                if doc.complete {
                    ui.colored_label(egui::Color32::GREEN, "All required ROMs are present.");
                }
                if let Some(err) = &self.rom_setup_error {
                    ui.colored_label(egui::Color32::RED, err);
                }
                ui.separator();
                ui.horizontal(|ui| {
                    if doc.complete && ui.button("Load machine").clicked() {
                        load_machine = true;
                    }
                    if ui.button("Close").clicked() {
                        close = true;
                    }
                });
            });
        self.show_rom_setup = open && !close;
        if load_machine {
            self.finish_rom_setup();
        }
    }

    fn config_editor_window(&mut self, ctx: &egui::Context) {
        let Some(draft) = self.config_draft.as_mut() else {
            return;
        };
        let mut open = true;
        let mut save = false;
        let mut cancel = false;
        let title = if self.config_editor_is_new {
            "New configuration"
        } else {
            "Edit configuration"
        };
        egui::Window::new(title)
            .open(&mut open)
            .default_width(440.0)
            .default_height(560.0)
            .show(ctx, |ui| {
                ui.weak(
                    "Saved profile: base model, optional main ROM override, and hardware to attach on load.",
                );
                ui.separator();
                egui::ScrollArea::vertical().max_height(480.0).show(ui, |ui| {
                    ui.label("Name");
                    ui.text_edit_singleline(&mut draft.name);
                    ui.separator();
                    ui.label("Base model");
                    for pick in machine::ALL_MODELS {
                        let pref = PrefModel::from_model(pick);
                        let title = machine::model_title(pick);
                        let mut selected = draft.base == pref;
                        if ui.radio_value(&mut selected, true, title).clicked() {
                            draft.base = pref;
                            *draft = draft.clone().sanitized();
                        }
                    }
                    ui.separator();
                    Self::path_field(ui, "Main ROM", &mut draft.custom_rom_path, "ROM");
                    ui.label("Leave empty to use the default ROM for the base model.");
                    ui.separator();
                    ui.label("Input");
                    ui.radio_value(&mut draft.joystick_mode, PrefJoystick::Kempston, "Kempston");
                    ui.radio_value(
                        &mut draft.joystick_mode,
                        PrefJoystick::SinclairLeft,
                        "Sinclair left",
                    );
                    ui.radio_value(
                        &mut draft.joystick_mode,
                        PrefJoystick::SinclairRight,
                        "Sinclair right",
                    );
                    ui.radio_value(&mut draft.joystick_mode, PrefJoystick::Cursor, "Cursor");
                    ui.checkbox(&mut draft.kempston_mouse, "Kempston mouse");
                    let compat = hardware_compat(draft.base);
                    if compat.ay_stereo {
                        ui.separator();
                        ui.label("AY stereo");
                        ui.radio_value(&mut draft.ay_stereo, PrefAyStereo::Mono, "Mono");
                        ui.radio_value(&mut draft.ay_stereo, PrefAyStereo::Acb, "ACB");
                        ui.radio_value(&mut draft.ay_stereo, PrefAyStereo::Abc, "ABC");
                    }
                    ui.separator();
                    ui.label("Attach peripherals (saved with profile)");
                    if compat.multiface || compat.divmmc || compat.interface1 || compat.beta {
                        if compat.multiface {
                            if ui
                                .checkbox(&mut draft.attach_multiface, "Multiface (1 / 128)")
                                .changed()
                                && !draft.attach_multiface
                            {
                                draft.multiface_rom_path = None;
                            }
                            if draft.attach_multiface {
                                Self::path_field(
                                    ui,
                                    "Multiface ROM",
                                    &mut draft.multiface_rom_path,
                                    "Multiface",
                                );
                            }
                        }
                        if compat.divmmc {
                            if ui.checkbox(&mut draft.attach_divmmc, "DivMMC").changed()
                                && !draft.attach_divmmc
                            {
                                draft.divmmc_eeprom_path = None;
                            }
                            if draft.attach_divmmc {
                                Self::path_field(
                                    ui,
                                    "ESXDOS EEPROM",
                                    &mut draft.divmmc_eeprom_path,
                                    "EEPROM",
                                );
                            }
                        }
                        if compat.interface1 {
                            ui.checkbox(&mut draft.attach_interface1, "Interface 1 (stub)");
                            if draft.attach_interface1 {
                                Self::path_field(
                                    ui,
                                    "IF1 ROM",
                                    &mut draft.interface1_rom_path,
                                    "IF1",
                                );
                            }
                        }
                        if compat.beta {
                            if ui.checkbox(&mut draft.attach_beta, "Beta Disk").changed()
                                && !draft.attach_beta
                            {
                                draft.trdos_rom_path = None;
                            }
                            if draft.attach_beta {
                                Self::path_field(
                                    ui,
                                    "TR-DOS ROM",
                                    &mut draft.trdos_rom_path,
                                    "TR-DOS",
                                );
                            }
                        }
                    } else {
                        ui.weak("No optional peripheral hardware on this base model.");
                    }
                    if let Some(err) = &self.config_editor_error {
                        ui.colored_label(egui::Color32::RED, err);
                    }
                });
                ui.separator();
                ui.horizontal(|ui| {
                    let save_label = if self.config_editor_is_new {
                        "Create"
                    } else {
                        "Save"
                    };
                    if ui.button(save_label).clicked() {
                        save = true;
                    }
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                });
            });
        if !open || cancel {
            self.config_draft = None;
            self.config_editor_error = None;
            return;
        }
        if save {
            let to_save = draft.clone().sanitized();
            match to_save.validate() {
                Ok(()) => {
                    let is_new = !self.prefs.custom_configs.iter().any(|c| c.id == to_save.id);
                    if is_new
                        && self.prefs.custom_configs.len() >= spec_chum_host::MAX_CUSTOM_CONFIGS
                    {
                        self.config_editor_error = Some(format!(
                            "Cannot save more than {} configurations",
                            spec_chum_host::MAX_CUSTOM_CONFIGS
                        ));
                    } else {
                        match self.session.apply_user_machine_config(&to_save) {
                            Ok(()) => {
                                self.prefs.upsert_custom_config(to_save.clone());
                                self.prefs.select_custom_config(&to_save.id);
                                self.sync_prefs_from_custom_config(&to_save);
                                self.session
                                    .host_mut()
                                    .set_joystick_mode(to_save.joystick_mode.to_mode());
                                self.session.kempston_mouse = to_save.kempston_mouse;
                                self.apply_restored_machine_options();
                                self.mark_prefs_dirty();
                                self.config_draft = None;
                                self.config_editor_error = None;
                            }
                            Err(e) => self.config_editor_error = Some(e.to_string()),
                        }
                    }
                }
                Err(e) => self.config_editor_error = Some(e.to_string()),
            }
        }
    }

    /// egui UI body — callable from `App::update` or headless `Context::run`.
    pub fn ui(&mut self, ctx: &egui::Context) {
        if !self.theme_applied {
            theme::apply(ctx);
            self.theme_applied = true;
        }
        egui::TopBottomPanel::top("menu")
            .exact_height(theme::menu_bar_min_height())
            .frame(
                egui::Frame::new()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(egui::Margin::symmetric(10, 4))
                    .stroke(egui::Stroke::NONE),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if self.show_roms_toolbar_button() {
                        if ui.button("ROMs…").clicked() {
                            self.show_rom_setup = true;
                            self.refresh_rom_setup();
                        }
                        ui.separator();
                    }
                    ui.menu_button("File", |ui| {
                        if ui.button("Open snapshot (SNA/Z80)…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Snapshots", &["sna", "z80"])
                                .pick_file()
                            {
                                self.session.load_snapshot(&path);
                                self.note_recent_if_ok(&path);
                                self.prefs.set_model_from_machine(self.session.model());
                                self.mark_prefs_dirty();
                            }
                            ui.close_menu();
                        }
                        if ui.button("Open TAP…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("TAP", &["tap"])
                                .pick_file()
                            {
                                self.session.load_tap(&path);
                                self.note_recent_if_ok(&path);
                            }
                            ui.close_menu();
                        }
                        if ui.button("Open TZX…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("TZX", &["tzx"])
                                .pick_file()
                            {
                                self.session.load_tzx(&path);
                                self.note_recent_if_ok(&path);
                            }
                            ui.close_menu();
                        }
                        if ui.button("Open RZX…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("RZX", &["rzx"])
                                .pick_file()
                            {
                                self.session.load_rzx(&path);
                                self.note_recent_if_ok(&path);
                            }
                            ui.close_menu();
                        }
                        if ui.button("Open DSK…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("DSK", &["dsk"])
                                .pick_file()
                            {
                                self.session.load_dsk(&path);
                                self.note_recent_if_ok(&path);
                            }
                            ui.close_menu();
                        }
                        if !self.prefs.recent_files.is_empty() {
                            ui.separator();
                            ui.menu_button("Open recent", |ui| {
                                let recents = self.prefs.recent_files.clone();
                                for path_str in recents {
                                    let label = Path::new(&path_str)
                                        .file_name()
                                        .and_then(|n| n.to_str())
                                        .unwrap_or(path_str.as_str());
                                    if ui.button(label).clicked() {
                                        self.open_recent_path(Path::new(&path_str));
                                        ui.close_menu();
                                    }
                                }
                            });
                        }
                        if ui.button("Quit").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                    ui.menu_button("Machine", |ui| {
                        if self.prefs.active_config_id.is_none() {
                            if ui.button("ROMs…").clicked() {
                                self.show_rom_setup = true;
                                self.refresh_rom_setup();
                                ui.close_menu();
                            }
                            ui.separator();
                        }
                        ui.label("Built-in models");
                        ui.weak("Select only — default ROMs. Session hardware via Hardware menu.");
                        ui.weak(
                            "Timex TC2048 / TS2068: SCLD alt file, hi-colour, and 512×192 hi-res — docs/TIMEX.md.",
                        );
                        for pick in machine::ALL_MODELS {
                            let pref = PrefModel::from_model(pick);
                            let available = model_rom_available(
                                pref.to_model_id(),
                                &self.prefs.model_rom_paths,
                            );
                            let title = machine::model_title(pick);
                            let label = if available {
                                title.to_string()
                            } else {
                                format!("{title} (ROMs required)")
                            };
                            let mut selected = self.prefs.active_config_id.is_none()
                                && self.session.model() == pick;
                            let response = ui.radio_value(&mut selected, true, label);
                            if !available {
                                response.clone().on_hover_text(format!(
                                    "{} — {}",
                                    title,
                                    machine::unavailable_reason(pick)
                                ));
                            } else if pick == Model::TimexTC2048 || pick == Model::TimexTS2068 {
                                response.clone().on_hover_text(
                                    "Timex: home/EX-ROM + SCLD MMU (TS2068) / latches (TC2048); \
                                     alt file, hi-colour, and 512×192 hi-res (docs/TIMEX.md)",
                                );
                            }
                            if response.clicked() {
                                self.on_builtin_model_selected(pick);
                            }
                        }
                        ui.separator();
                        ui.label("My configurations");
                        if ui.button("+ New configuration…").clicked() {
                            let base = if let Some(cfg) = self.prefs.active_custom_config() {
                                cfg.base
                            } else {
                                PrefModel::from_model(self.session.model())
                            };
                            let mut draft = UserMachineConfig::new_named("My Spectrum", base);
                            draft.joystick_mode =
                                PrefJoystick::from_mode(self.session.host_mut().joystick_mode());
                            draft.kempston_mouse = self.session.kempston_mouse;
                            {
                                let host = &mut *self.session.host_mut();
                                if let Some(m) = host.machine() {
                                    draft.ay_stereo = PrefAyStereo::from_mode(m.ay_stereo_mode());
                                }
                            }
                            self.config_draft = Some(draft);
                            self.config_editor_is_new = true;
                            self.config_editor_error = None;
                            ui.close_menu();
                        }
                        if self.prefs.custom_configs.is_empty() {
                            ui.weak("(none saved yet)");
                        }
                        let configs: Vec<UserMachineConfig> =
                            self.prefs.custom_configs.clone();
                        for cfg in &configs {
                            ui.horizontal(|ui| {
                                let mut selected = self.prefs.active_config_id.as_deref()
                                    == Some(cfg.id.as_str());
                                if ui.radio_value(&mut selected, true, &cfg.name).clicked() {
                                    self.show_rom_setup = false;
                                    self.prefs.select_custom_config(&cfg.id);
                                    if let Some(active) = self.prefs.active_custom_config().cloned()
                                    {
                                        match self.session.apply_user_machine_config(&active) {
                                            Ok(()) => {
                                                self.sync_prefs_from_custom_config(&active);
                                                self.session.host_mut().set_joystick_mode(active.joystick_mode.to_mode());
                                                self.session.kempston_mouse = active.kempston_mouse;
                                                self.apply_restored_machine_options();
                                                self.mark_prefs_dirty();
                                            }
                                            Err(e) => self.session.host_mut().set_status(e.to_string()),
                                        }
                                    }
                                }
                                if ui.small_button("Edit…").clicked() {
                                    self.config_draft = Some(cfg.clone());
                                    self.config_editor_is_new = false;
                                    self.config_editor_error = None;
                                    ui.close_menu();
                                }
                                if ui.small_button("Delete").clicked() {
                                    let was_active =
                                        self.prefs.active_config_id.as_deref() == Some(cfg.id.as_str());
                                    self.prefs.delete_custom_config(&cfg.id);
                                    if was_active {
                                        self.prefs.model = self.prefs.last_builtin_model;
                                        self.on_builtin_model_selected(
                                            self.prefs.last_builtin_model.to_model(),
                                        );
                                    }
                                    self.mark_prefs_dirty();
                                    ui.close_menu();
                                }
                            });
                        }
                        let has_active = self.prefs.is_custom_config_active();
                        if ui
                            .add_enabled(has_active, egui::Button::new("Edit configuration…"))
                            .clicked()
                        {
                            if let Some(id) = self.prefs.active_config_id.clone() {
                                if let Some(cfg) =
                                    self.prefs.custom_configs.iter().find(|c| c.id == id)
                                {
                                    self.config_draft = Some(cfg.clone());
                                    self.config_editor_is_new = false;
                                    self.config_editor_error = None;
                                }
                            }
                            ui.close_menu();
                        }
                        if ui
                            .add_enabled(has_active, egui::Button::new("Delete configuration"))
                            .clicked()
                        {
                            if let Some(id) = self.prefs.active_config_id.clone() {
                                self.prefs.delete_custom_config(&id);
                                self.on_builtin_model_selected(
                                    self.prefs.last_builtin_model.to_model(),
                                );
                            }
                            ui.close_menu();
                        }
                        ui.separator();
                        ui.label("Session");
                        if ui
                            .add_enabled(
                                self.session.host_mut().has_machine(),
                                egui::Button::new("Reset"),
                            )
                            .clicked()
                        {
                            let reset_err = self.session.host_mut().reset().err();
                            if let Some(e) = reset_err {
                                self.session.host_mut().set_status(e.to_string());
                            }
                            ui.close_menu();
                        }
                        {
                            let mut running = self.session.host_mut().running();
                            if ui.checkbox(&mut running, "Running").changed() {
                                self.session.host_mut().set_running(running);
                            }
                        }
                        if ui
                            .checkbox(&mut self.session.throttle, "Throttle ~50Hz")
                            .changed()
                        {
                            self.mark_prefs_dirty();
                        }
                        ui.separator();
                        ui.label("Joystick");
                        {
                            let mut joy = self.session.host_mut().joystick_mode();
                            let mut joy_changed = false;
                            joy_changed |= ui
                                .radio_value(&mut joy, JoystickMode::Kempston, "Kempston")
                                .changed();
                            joy_changed |= ui
                                .radio_value(
                                    &mut joy,
                                    JoystickMode::SinclairLeft,
                                    "Sinclair left (1–5)",
                                )
                                .changed();
                            joy_changed |= ui
                                .radio_value(
                                    &mut joy,
                                    JoystickMode::SinclairRight,
                                    "Sinclair right (6–0)",
                                )
                                .changed();
                            joy_changed |= ui
                                .radio_value(&mut joy, JoystickMode::Cursor, "Cursor")
                                .changed();
                            if joy_changed {
                                self.session.host_mut().set_joystick_mode(joy);
                                self.mark_prefs_dirty();
                            }
                        }
                        if ui
                            .checkbox(&mut self.session.kempston_mouse, "Kempston mouse")
                            .changed()
                        {
                            self.mark_prefs_dirty();
                        }
                        if matches!(
                            self.session.model(),
                            Model::Spectrum128
                                | Model::SpectrumPlus2
                                | Model::SpectrumPlus2A
                                | Model::SpectrumPlus3
                                | Model::SpectrumPlus3e
                                | Model::Pentagon128
                                | Model::ScorpionZs256
                                | Model::TimexTS2068
                        ) {
                            ui.separator();
                            ui.label("AY stereo");
                            let mut mode = self
                                .session
                                .host_mut().machine()
                                .map_or(AyStereoMode::Mono, Machine::ay_stereo_mode);
                            let before = mode;
                            ui.radio_value(&mut mode, AyStereoMode::Mono, "Mono");
                            ui.radio_value(&mut mode, AyStereoMode::Acb, "ACB");
                            ui.radio_value(&mut mode, AyStereoMode::Abc, "ABC");
                            if mode != before {
                                {
                                    let host = &mut *self.session.host_mut();
                                    if let Some(m) = host.machine_mut() {
                                        m.set_ay_stereo_mode(mode);
                                    }
                                }
                                self.prefs.set_ay_stereo(mode);
                                self.mark_prefs_dirty();
                            }
                        }
                        if ui.checkbox(&mut self.session.muted, "Mute").changed() {
                            self.mark_prefs_dirty();
                        }
                        if ui
                            .add_enabled(
                                !self.session.muted,
                                egui::Slider::new(&mut self.session.volume, 0.0..=1.0)
                                    .text("Volume"),
                            )
                            .changed()
                        {
                            self.mark_prefs_dirty();
                        }
                    });
                    ui.menu_button("Hardware", |ui| {
                        let model = self.session.model();
                        let has_mf = self
                            .session
                            .host_mut().machine()
                            .is_some_and(Machine::has_multiface);
                        let has_div = self
                            .session
                            .host_mut().machine()
                            .is_some_and(Machine::has_divmmc);
                        let has_if1 = self
                            .session
                            .host_mut().machine()
                            .is_some_and(Machine::has_interface1);
                        let has_beta = self.session.host_mut().machine().is_some_and(Machine::has_beta);
                        let has_dock = self
                            .session
                            .host_mut().machine()
                            .is_some_and(Machine::has_timex_dock);

                        ui.label("Peripherals (partial where noted)");
                        ui.separator();

                        if matches!(
                            model,
                            Model::Spectrum16K
                                | Model::Spectrum48
                                | Model::TimexTC2048
                                | Model::TimexTS2068
                                | Model::Spectrum128
                                | Model::SpectrumPlus2
                                | Model::Pentagon128
                                | Model::ScorpionZs256
                        ) {
                            let mf_label = if matches!(
                                model,
                                Model::Spectrum128 | Model::SpectrumPlus2 | Model::Pentagon128 | Model::ScorpionZs256
                            ) {
                                "Attach Multiface 128 ROM…"
                            } else {
                                "Attach Multiface 1 ROM…"
                            };
                            if ui.button(mf_label).clicked() {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("Multiface ROM", &["rom", "bin"])
                                    .pick_file()
                                {
                                    self.session.attach_multiface(&path);
                                }
                                ui.close_menu();
                            }
                            if ui
                                .add_enabled(has_mf, egui::Button::new("Multiface NMI"))
                                .clicked()
                            {
                                self.session.multiface_nmi();
                                ui.close_menu();
                            }
                            if has_mf {
                                ui.label("Multiface: attached");
                            }
                        } else {
                            ui.label("Multiface: 48K-class (MF1) or 128K/+2 (MF128); not +2A/+3");
                        }

                        if model == Model::TimexTS2068 {
                            ui.separator();
                            if ui.button("Insert Timex Dock DCK…").clicked() {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("Timex dock", &["dck"])
                                    .pick_file()
                                {
                                    self.session.insert_dck(&path);
                                }
                                ui.close_menu();
                            }
                            if ui
                                .add_enabled(has_dock, egui::Button::new("Eject Timex Dock"))
                                .clicked()
                            {
                                self.session.eject_dck();
                                ui.close_menu();
                            }
                            ui.label(if has_dock {
                                "Dock: cartridge inserted (HOME/DOCK/EX-ROM banks from .dck)"
                            } else {
                                "Dock: empty (reads 0xFF when paged)"
                            });
                        }

                        ui.separator();
                        if matches!(
                            model,
                            Model::Spectrum16K
                                | Model::Spectrum48
                                | Model::TimexTC2048
                                | Model::TimexTS2068
                                | Model::Spectrum128
                                | Model::SpectrumPlus2
                                | Model::Pentagon128
                                | Model::ScorpionZs256
                        ) {
                            if ui.button("Attach DivMMC").clicked() {
                                self.session.attach_divmmc_stub();
                                ui.close_menu();
                            }
                            if ui
                                .add_enabled(has_div, egui::Button::new("Open DivMMC SD image…"))
                                .clicked()
                            {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("SD image", &["img", "bin", "mmc", "sd"])
                                    .pick_file()
                                {
                                    self.session.attach_divmmc_sd(&path);
                                }
                                ui.close_menu();
                            }
                            if ui
                                .add_enabled(
                                    has_div,
                                    egui::Button::new("Open DivMMC SD image (slot 1)…"),
                                )
                                .clicked()
                            {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("SD image", &["img", "bin", "mmc", "sd"])
                                    .pick_file()
                                {
                                    self.session.attach_divmmc_sd_slot(&path, 1);
                                }
                                ui.close_menu();
                            }
                            if ui
                                .add_enabled(
                                    has_div,
                                    egui::Button::new("Open DivMMC EEPROM (ESXDOS)…"),
                                )
                                .clicked()
                            {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("EEPROM / ESXDOS", &["rom", "bin", "eeprom"])
                                    .pick_file()
                                {
                                    self.session.attach_divmmc_eeprom(&path);
                                }
                                ui.close_menu();
                            }
                            ui.label(if has_div {
                                "DivMMC: attached (SPI sector I/O + automap; ESXDOS boot needs EEPROM)"
                            } else {
                                "DivMMC: not attached"
                            });

                            ui.separator();
                            if ui.button("Attach Interface 1 (stub)").clicked() {
                                self.session.attach_interface1_stub();
                                ui.close_menu();
                            }
                            if ui
                                .add_enabled(has_if1, egui::Button::new("Open Microdrive MDR…"))
                                .clicked()
                            {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("MDR", &["mdr"])
                                    .pick_file()
                                {
                                    self.session.insert_mdr(&path);
                                }
                                ui.close_menu();
                            }
                            ui.label(if has_if1 {
                                "IF1: attached (Microdrive I/O + ROM paging)"
                            } else {
                                "IF1: not attached"
                            });

                            ui.separator();
                            if ui.button("Attach Beta Disk").clicked() {
                                self.session.attach_beta_stub();
                                ui.close_menu();
                            }
                            if ui.button("Load TR-DOS ROM…").clicked() {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("TR-DOS ROM", &["rom", "bin"])
                                    .pick_file()
                                {
                                    self.session.load_trdos_rom(&path);
                                }
                                ui.close_menu();
                            }
                            if ui.button("Open TRD…").clicked() {
                                if let Some(path) = rfd::FileDialog::new()
                                    .add_filter("TRD", &["trd"])
                                    .pick_file()
                                {
                                    self.session.insert_trd(&path);
                                }
                                ui.close_menu();
                            }
                            ui.label(if has_beta {
                                "Beta: VG93 + TR-DOS paging (need 16K ROM for USR 15616)"
                            } else {
                                "Beta: not attached"
                            });
                        } else {
                            ui.label("DivMMC / IF1 / Beta: not on +2A/+3");
                        }

                        if model == Model::SpectrumPlus3 || model == Model::SpectrumPlus3e {
                            ui.separator();
                            ui.label("+3 disk: File → Open DSK…");
                        } else if model == Model::SpectrumPlus2A {
                            ui.separator();
                            ui.label("+2A: no floppy (tape Loader)");
                        }
                    });
                    ui.menu_button("Tape", |ui| {
                        let has_tape = self
                            .session
                            .host_mut().machine()
                            .is_some_and(Machine::has_tape);
                        if has_tape {
                            if ui.button("Play tape").clicked() {
                                self.session.play_tape();
                                ui.close_menu();
                            }
                            if ui.button("Pause tape").clicked() {
                                self.session.pause_tape();
                                ui.close_menu();
                            }
                            if ui.button("Rewind tape").clicked() {
                                self.session.rewind_tape();
                                ui.close_menu();
                            }
                            ui.separator();
                        }
                        if ui.button("Type LOAD \"\"").clicked() {
                            self.session.type_load_quotes();
                            ui.close_menu();
                        }
                        if ui.button("Type LOAD \"\" CODE").clicked() {
                            self.session.type_load_quotes_code();
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui
                            .button("Instant…")
                            .on_hover_text(format!(
                                "Always asks for a TAP/TZX, then flash-loads (Type LOAD \"\" + Play). Decks with a custom loader (pulse TZX) have no flash trap and load off EAR at {}× instead — never at the EAR speed below. Play alone stays EAR-only. Use File → Open DSK for disks.",
                                machine::INSTANT_EAR_FALLBACK_SPEED,
                            ))
                            .clicked()
                        {
                            // Tape-only: Instant never fakes Type LOAD for DSK.
                            let dialog =
                                rfd::FileDialog::new().add_filter("Tape", &["tap", "tzx"]);
                            if let Some(path) = dialog.pick_file() {
                                self.session.instant_load_path(&path);
                            }
                            ui.close_menu();
                        }
                        if has_tape {
                            let mut tape_prefs_changed = false;
                            let mut tape_status: Option<String> = None;
                            {
                                let host = &mut *self.session.host_mut();
                                if let Some(m) = host.machine_mut() {
                                    let mut opts = m.tape_load_options();
                                    ui.label("Load mode:");
                                    if ui
                                        .selectable_label(opts.experience_load, "Experience (~20s)")
                                        .on_hover_text(
                                            "Hybrid flash + cosmetic abbreviated pilots (~20s-class; #82 / #167). Pulse-only decks fall back to EAR at 16×.",
                                        )
                                        .clicked()
                                    {
                                        m.set_tape_load_options(TapeLoadOptions::experience());
                                        tape_status =
                                            Some("Tape: experience load (~20s EAR)".into());
                                        tape_prefs_changed = true;
                                    }
                                    ui.label("EAR speed:").on_hover_text(format!(
                                        "Play (EAR) loading only — the loaded program always runs at 1×. Instant ignores this: flashable decks poke bytes at LD-BYTES, custom-loader decks load off EAR at {}×.",
                                        machine::INSTANT_EAR_FALLBACK_SPEED,
                                    ));
                                    for speed in [1u32, 2, 5, 10, 20, 64] {
                                        let selected =
                                            !opts.experience_load && opts.speed == speed;
                                        if ui
                                            .selectable_label(selected, format!("{speed}x"))
                                            .clicked()
                                        {
                                            opts.experience_load = false;
                                            opts.flash_load = false;
                                            opts.speed = speed;
                                            m.set_tape_load_options(opts);
                                            tape_status =
                                                Some(format!("Tape: EAR speed {speed}x"));
                                            tape_prefs_changed = true;
                                        }
                                    }
                                }
                                if let Some(s) = tape_status {
                                    host.set_status(s);
                                }
                            }
                            if tape_prefs_changed {
                                {
                                    let host = &mut *self.session.host_mut();
                                    if let Some(m) = host.machine() {
                                        self.prefs.set_tape_from_options(m.tape_load_options());
                                    }
                                }
                                self.mark_prefs_dirty();
                            }
                        }
                        ui.separator();
                        if ui
                            .checkbox(
                                &mut self.prefs.online_tape_titles,
                                "Look up tape titles online (ZXInfo)",
                            )
                            .on_hover_text(
                                "Opt-in (default off). Sends a SHA-512 of the opened tape file to api.zxinfo.dk — not the path or bytes. Failures keep the filename.",
                            )
                            .changed()
                        {
                            self.session
                                .host_mut()
                                .set_online_tape_titles(self.prefs.online_tape_titles);
                            self.mark_prefs_dirty();
                        }
                    });
                    ui.menu_button("Debug", |ui| {
                        if ui.button("Debugger window").clicked() {
                            self.session.debug_open = true;
                            ui.close_menu();
                        }
                        ui.separator();
                        let cats = trace::categories();
                        let mut tape = cats.contains(trace::Category::TAPE);
                        let mut cpu = cats.contains(trace::Category::CPU);
                        let mut bus = cats.contains(trace::Category::BUS);
                        let mut ula = cats.contains(trace::Category::ULA);
                        let mut machine = cats.contains(trace::Category::MACHINE);
                        let mut ay = cats.contains(trace::Category::AY);
                        let mut changed = false;
                        changed |= ui.checkbox(&mut tape, "Trace tape").changed();
                        changed |= ui.checkbox(&mut cpu, "Trace CPU").changed();
                        changed |= ui.checkbox(&mut bus, "Trace bus").changed();
                        changed |= ui.checkbox(&mut ula, "Trace ULA").changed();
                        changed |= ui.checkbox(&mut machine, "Trace machine").changed();
                        changed |= ui.checkbox(&mut ay, "Trace AY").changed();
                        if changed {
                            let mut c = trace::Category::NONE;
                            if tape {
                                c |= trace::Category::TAPE;
                            }
                            if cpu {
                                c |= trace::Category::CPU;
                            }
                            if bus {
                                c |= trace::Category::BUS;
                            }
                            if ula {
                                c |= trace::Category::ULA;
                            }
                            if machine {
                                c |= trace::Category::MACHINE;
                            }
                            if ay {
                                c |= trace::Category::AY;
                            }
                            trace::enable(c);
                            self.session.host_mut().set_status(format!(
                                "Trace categories=0x{:x} ({} events)",
                                c.bits(),
                                trace::len()
                            ));
                        }
                        if ui.button("Clear ring").clicked() {
                            trace::clear();
                            self.session.host_mut().set_status("Trace cleared");
                            ui.close_menu();
                        }
                        if ui.button("Dump to stderr").clicked() {
                            trace::dump_to_stderr();
                            self.session.host_mut().set_status(format!("Dumped {} trace events to stderr", trace::len()));
                            ui.close_menu();
                        }
                        if ui.button("Dump to file…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .set_file_name("spec_chum_trace.txt")
                                .save_file()
                            {
                                match trace::dump_to_file(&path) {
                                    Ok(()) => {
                                        self.session.host_mut().set_status(format!("Trace dump → {}", path.display()));
                                    }
                                    Err(e) => {
                                        self.session.host_mut().set_status(format!("Trace dump failed: {e}"));
                                    }
                                }
                            }
                            ui.close_menu();
                        }
                    });
                    ui.menu_button("Help", |ui| {
                        ui.label("Spec Chum — from-scratch ZX Spectrum emulator");
                        ui.separator();
                        ui.label(MAPPING_DOC);
                        ui.separator();
                        ui.label(
                            "Amstrad have kindly given their permission for the redistribution \
of their copyrighted material but retain that copyright.",
                        );
                        ui.label(
                            "System ROMs are not shipped with Spec Chum — fetch with ./scripts/fetch_roms.sh \
(see docs/ROMS.md).",
                        );
                    });
                    ui.separator();
                    let (tape_title, has_tape, tape_progress) = {
                        let mut host = self.session.host_mut();
                        (
                            host.media_title().map(str::to_owned),
                            host.machine().is_some_and(Machine::has_tape),
                            host.machine().and_then(Machine::tape_progress),
                        )
                    };
                    if let Some(p) = tape_progress {
                        if let Some(ref title) = tape_title {
                            ui.label(title);
                        }
                        ui.add(
                            egui::ProgressBar::new(p.fraction())
                                .desired_width(120.0)
                                .show_percentage(),
                        );
                        ui.label(format!(
                            "tape {}/{}",
                            if p.block_count == 0 {
                                0
                            } else {
                                p.block_index.saturating_add(1).min(p.block_count)
                            },
                            p.block_count
                        ));
                        // The rate actually being run, not the EAR speed setting:
                        // turbo stops when the deck finishes (#390).
                        let effective = self
                            .session
                            .host_mut()
                            .machine()
                            .map_or(1, Machine::effective_speed_multiplier);
                        if effective > 1 {
                            ui.strong(format!("{effective}×"))
                                .on_hover_text("Spectrum frames per host tick while the tape plays");
                        }
                    } else if has_tape {
                        if let Some(ref title) = tape_title {
                            ui.label(title);
                        }
                    }
                    if self
                        .session
                        .host_mut().machine()
                        .is_some_and(Machine::tape_playing)
                    {
                        ui.strong("▶ tape");
                    }
                    ui.label(self.session.host_mut().status());
                });
            });

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
            b.muted = self.session.muted;
            b.volume = self.session.volume.clamp(0.0, 1.0);
            if !self.session.muted {
                b.edges = audio.beeper_edges;
                b.ay_samples = audio.ay_samples;
                b.ay_left = audio.ay_left;
                b.ay_right = audio.ay_right;
                b.ay_index = 0;
                b.t = 0.0;
            }
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

        self.debug_window(ctx);
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

    fn debug_window(&mut self, ctx: &egui::Context) {
        let mut open = self.session.debug_open;
        egui::Window::new("Debugger")
            .open(&mut open)
            .default_size([520.0, 480.0])
            .show(ctx, |ui| {
                if !self.session.host_mut().has_machine() {
                    ui.label("No machine loaded");
                    return;
                }
                let paused = self.session.host_mut().paused();
                ui.horizontal(|ui| {
                    if paused {
                        if ui.button("Run").clicked() {
                            let mut host = self.session.host_mut();
                            if let Err(error) = host.debug_continue() {
                                host.set_status(error.to_string());
                            }
                        }
                    } else if ui.button("Pause").clicked() {
                        self.session.host_mut().debug_pause();
                    }
                    if ui.button("Step").clicked() {
                        let mut host = self.session.host_mut();
                        if let Err(error) = host.debug_step() {
                            host.set_status(error.to_string());
                        }
                    }
                });
                let inspect = self.session.host_mut().inspect_text().unwrap_or_default();
                let disasm = self.session.host_mut().disasm(None, 12).unwrap_or_default();
                let mem_addr = self.session.debug_mem_addr;
                let hex = self
                    .session
                    .host_mut()
                    .hexdump(mem_addr, 64)
                    .unwrap_or_default();
                let breaks = self
                    .session
                    .host_mut()
                    .list_pc_breakpoints()
                    .unwrap_or_default();
                let (pc, sp, hl) = self
                    .session
                    .host_mut()
                    .regs()
                    .map_or((0, 0, 0), |r| (r.pc, r.sp, r.hl));
                ui.separator();
                ui.monospace(inspect);
                ui.separator();
                ui.label("Disassembly");
                ui.monospace(disasm);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Hex at");
                    let mut addr = self.session.debug_mem_addr;
                    if ui
                        .add(egui::DragValue::new(&mut addr).hexadecimal(4, false, false))
                        .changed()
                    {
                        self.session.debug_mem_addr = addr;
                    }
                    if ui.button("PC").clicked() {
                        self.session.debug_mem_addr = pc;
                    }
                    if ui.button("SP").clicked() {
                        self.session.debug_mem_addr = sp;
                    }
                    if ui.button("HL").clicked() {
                        self.session.debug_mem_addr = hl;
                    }
                });
                ui.monospace(hex);
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label("Break PC");
                    let mut bp = self.session.debug_break_pc;
                    if ui
                        .add(egui::DragValue::new(&mut bp).hexadecimal(4, false, false))
                        .changed()
                    {
                        self.session.debug_break_pc = bp;
                    }
                    if ui.button("PC").clicked() {
                        self.session.debug_break_pc = pc;
                    }
                    if ui.button("Add").clicked() {
                        let bp = self.session.debug_break_pc;
                        let mut host = self.session.host_mut();
                        if let Err(error) = host.add_breakpoint(bp) {
                            host.set_status(error.to_string());
                        }
                    }
                    if ui.button("Clear breaks").clicked() {
                        let mut host = self.session.host_mut();
                        if let Err(error) = host.clear_breakpoints() {
                            host.set_status(error.to_string());
                        }
                    }
                });
                ui.label(format!("PC breaks: {breaks:?}"));
                ui.separator();
                ui.label(format!("Trace ({} events)", trace::len()));
                for ev in trace::snapshot().iter().rev().take(16) {
                    ui.monospace(ev.to_string());
                }
            });
        self.session.debug_open = open;
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

fn start_beeper(state: Arc<std::sync::Mutex<BeeperState>>) -> Option<cpal::Stream> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let device = host.default_output_device()?;
    let config = device.default_output_config().ok()?;
    let sample_rate = config.sample_rate().0;
    let channels = config.channels();
    {
        let mut s = state.lock().ok()?;
        s.sample_rate = sample_rate;
        s.channels = channels;
        s.frame_t_per_sample = 69888.0 / (sample_rate as f32 / 50.0);
    }
    let stream = device
        .build_output_stream(
            &config.config(),
            move |data: &mut [f32], _| {
                let Ok(mut st) = state.lock() else {
                    return;
                };
                if st.muted {
                    for sample in data.iter_mut() {
                        *sample = 0.0;
                    }
                    return;
                }
                let ch = usize::from(st.channels.max(1));
                for frame in data.chunks_mut(ch) {
                    while let Some(&(edge_t, level)) = st.edges.first() {
                        if st.t >= edge_t as f32 {
                            st.level = level;
                            st.edges.remove(0);
                        } else {
                            break;
                        }
                    }
                    let beep = if st.level { 0.15 } else { -0.15 };
                    let (ay_l, ay_r) = if st.ay_index < st.ay_left.len()
                        && st.ay_index < st.ay_right.len()
                    {
                        let l = st.ay_left[st.ay_index];
                        let r = st.ay_right[st.ay_index];
                        st.ay_index += 1;
                        ((l - 0.5) * 0.5, (r - 0.5) * 0.5)
                    } else if st.ay_index < st.ay_samples.len() {
                        let v = st.ay_samples[st.ay_index];
                        st.ay_index += 1;
                        let m = (v - 0.5) * 0.5;
                        (m, m)
                    } else if let (Some(&l), Some(&r)) = (st.ay_left.last(), st.ay_right.last()) {
                        ((l - 0.5) * 0.5, (r - 0.5) * 0.5)
                    } else if let Some(&last) = st.ay_samples.last() {
                        let m = (last - 0.5) * 0.5;
                        (m, m)
                    } else {
                        (0.0, 0.0)
                    };
                    let gain = st.volume.clamp(0.0, 1.0);
                    let left = ((beep + ay_l) * gain).clamp(-1.0, 1.0);
                    let right = ((beep + ay_r) * gain).clamp(-1.0, 1.0);
                    frame[0] = left;
                    if ch > 1 {
                        frame[1] = right;
                    }
                    for s in frame.iter_mut().skip(2) {
                        *s = 0.0;
                    }
                    st.t += st.frame_t_per_sample;
                }
            },
            |err| eprintln!("audio error: {err}"),
            None,
        )
        .ok()?;
    stream.play().ok()?;
    Some(stream)
}
