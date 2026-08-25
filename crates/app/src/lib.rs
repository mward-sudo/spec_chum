//! Spec Chum — egui frontend library (testable without a display).

mod display;
mod keymap;
mod theme;

pub use keymap::MAPPING_DOC;

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use eframe::egui;
use machine::{AyStereoMode, JoystickMode, JoystickState, Machine, Model};

/// Session state shared by the GUI and headless tests.
#[derive(Debug)]
pub struct EmulatorSession {
    pub machine: Option<Machine>,
    pub framebuffer: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub with_border: bool,
    pub running: bool,
    pub throttle: bool,
    pub muted: bool,
    pub status: String,
    pub model: Model,
    pub debug_open: bool,
    pub debug_mem_addr: u16,
    pub debug_break_pc: u16,
    /// Host joystick presentation (Kempston / Sinclair / Cursor).
    pub joystick_mode: JoystickMode,
    /// When true, accumulate egui pointer delta into the Kempston mouse each frame.
    pub kempston_mouse: bool,
    /// Scripted matrix keys (e.g. auto-type `LOAD ""`).
    key_script: Option<KeyScript>,
}

/// Hold a chord for `frames` emulated frames, then advance.
#[derive(Clone, Debug)]
struct KeyScript {
    steps: Vec<(Vec<(usize, u8)>, u32)>,
    step_i: usize,
    frames_left: u32,
}

impl KeyScript {
    /// Frames a key must stay down / gaps between chords for 48K ROM debounce.
    /// Shorter (6/3) drops the second `"` so `LOAD ""` never reaches LD-BYTES (#85).
    const PRESS: u32 = 10;
    const GAP: u32 = 5;
    /// Idle frames after menu → 48 BASIC Enter so MAIN-EXEC settles (machine waits on PC).
    const MENU_TO_48_BASIC_WAIT: u32 = 120;

    /// 48K keyword mode: J (`LOAD`) then `""` then Enter (PROGRAM tapes).
    fn load_quotes_48k() -> Self {
        Self::load_quotes_48k_inner(false)
    }

    /// 48K: `LOAD "" CODE` Enter (CODE tapes such as `attr_mark.tap`).
    fn load_quotes_code_48k() -> Self {
        Self::load_quotes_48k_inner(true)
    }

    /// 128K / +3 boot menu → 48 BASIC, then keyword `LOAD ""` [CODE].
    ///
    /// +3's first item is disk **Loader**, not a tape loader — do not press Enter alone.
    fn load_quotes_128_or_plus3(with_code: bool) -> Self {
        let gap = Vec::new();
        let cursor_down = vec![keymap::CAPS, (4, 4)]; // Caps+6
        let enter = vec![(6, 0)];
        let mut steps = Vec::new();
        // Loader/Tape Loader → BASIC → Calculator → 48 BASIC (three downs).
        for _ in 0..3 {
            steps.push((cursor_down.clone(), Self::PRESS));
            steps.push((gap.clone(), Self::GAP));
        }
        steps.push((enter, Self::PRESS));
        steps.push((gap, Self::MENU_TO_48_BASIC_WAIT));
        let mut load = Self::load_quotes_48k_inner(with_code);
        steps.append(&mut load.steps);
        Self {
            steps,
            step_i: 0,
            frames_left: 0,
        }
    }

    fn load_quotes_48k_inner(with_code: bool) -> Self {
        let j = vec![(6, 3)];
        let quote = vec![keymap::SYM, (5, 0)];
        let extend = vec![keymap::CAPS, keymap::SYM];
        let code_i = vec![(5, 2)]; // I in E mode → CODE
        let enter = vec![(6, 0)];
        let gap = Vec::new();
        let mut steps = vec![
            (j, Self::PRESS),
            (gap.clone(), Self::GAP),
            (quote.clone(), Self::PRESS),
            (gap.clone(), Self::GAP),
            (quote, Self::PRESS),
            (gap.clone(), Self::GAP),
        ];
        if with_code {
            steps.push((extend, Self::PRESS));
            steps.push((gap.clone(), Self::GAP));
            steps.push((code_i, Self::PRESS));
            steps.push((gap.clone(), Self::GAP));
        }
        steps.push((enter, Self::PRESS));
        steps.push((gap, 15));
        Self {
            steps,
            step_i: 0,
            frames_left: 0,
        }
    }
}

impl EmulatorSession {
    #[must_use]
    pub fn new(model: Model, with_border: bool) -> Self {
        let (width, height) = if with_border { (352, 296) } else { (256, 192) };
        Self {
            machine: None,
            framebuffer: vec![0; width * height * 4],
            width,
            height,
            with_border,
            running: true,
            throttle: true,
            muted: false,
            status: "Load a ROM via Machine menu (or auto-detect roms/)".into(),
            model,
            debug_open: false,
            debug_mem_addr: 0,
            debug_break_pc: 0,
            joystick_mode: JoystickMode::Kempston,
            kempston_mouse: false,
            key_script: None,
        }
    }

    #[must_use]
    pub fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    pub fn try_autoload_rom(&mut self) {
        let root = Self::workspace_root();
        match self.model {
            Model::Spectrum48 => {
                let rom48 = root.join("roms/spec48.rom");
                if let Ok(data) = std::fs::read(&rom48) {
                    match Machine::new_48k(&data) {
                        Ok(m) => {
                            self.machine = Some(m);
                            self.status = format!("Loaded {}", rom48.display());
                        }
                        Err(e) => self.status = e,
                    }
                } else {
                    self.status =
                        format!("Missing {}. Run ./scripts/fetch_roms.sh", rom48.display());
                }
            }
            Model::Spectrum128 => {
                let rom128 = root.join("roms/128/spec128uk.rom");
                if let Ok(data) = std::fs::read(&rom128) {
                    match Machine::new_128k(&data) {
                        Ok(m) => {
                            self.machine = Some(m);
                            self.status = format!("Loaded {}", rom128.display());
                        }
                        Err(e) => self.status = e,
                    }
                } else {
                    self.status =
                        format!("Missing {}. Run ./scripts/fetch_roms.sh", rom128.display());
                }
            }
            Model::SpectrumPlus3 => {
                let rom = root.join("roms/plus3/plus3.rom");
                let rom_alt = root.join("roms/plus2a/plus2a.rom");
                let path = if rom.exists() { rom } else { rom_alt };
                if let Ok(data) = std::fs::read(&path) {
                    match Machine::new_plus3(&data) {
                        Ok(m) => {
                            self.machine = Some(m);
                            self.status = format!("Loaded {}", path.display());
                        }
                        Err(e) => self.status = e,
                    }
                } else {
                    self.status = "Missing +2A/+3 ROM. Run ./scripts/fetch_roms.sh".to_string();
                }
            }
        }
    }

    pub fn load_snapshot(&mut self, path: &Path) {
        if let Ok(snap) =
            formats::Snapshot128::load_sna(path).or_else(|_| formats::Snapshot128::load_z80(path))
        {
            let target = if snap.is_plus3() {
                Model::SpectrumPlus3
            } else {
                Model::Spectrum128
            };
            if self.machine.is_none() || self.model != target {
                self.model = target;
                self.machine = None;
                self.try_autoload_rom();
            }
            if let Some(m) = self.machine.as_mut() {
                m.apply_snapshot128(&snap);
                self.status = format!("Loaded 128K/+3 snapshot {}", path.display());
            }
            return;
        }
        match formats::Snapshot48::load_sna(path).or_else(|_| formats::Snapshot48::load_z80(path)) {
            Ok(snap) => {
                if self.machine.is_none() || self.model != Model::Spectrum48 {
                    self.model = Model::Spectrum48;
                    self.machine = None;
                    self.try_autoload_rom();
                }
                if let Some(m) = self.machine.as_mut() {
                    m.apply_snapshot48(&snap);
                    self.status = format!("Loaded snapshot {}", path.display());
                }
            }
            Err(e) => self.status = format!("Snapshot error: {e}"),
        }
    }

    pub fn load_tap(&mut self, path: &Path) {
        match tape::TapImage::load(path) {
            Ok(img) => {
                if let Some(m) = self.machine.as_mut() {
                    m.insert_tape(tape::TapPlayer::new(img));
                    self.status = format!(
                        "Inserted TAP {} (paused — Tape → Play, or Type LOAD \"\")",
                        path.display()
                    );
                } else {
                    self.status = "Load a machine ROM before inserting tape".into();
                }
            }
            Err(e) => self.status = format!("TAP error: {e}"),
        }
    }

    pub fn load_tzx(&mut self, path: &Path) {
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                self.status = format!("TZX error: {e}");
                return;
            }
        };
        let Some(m) = self.machine.as_mut() else {
            self.status = "Load a machine ROM before inserting tape".into();
            return;
        };
        // Standard-speed TZX → TAP deck so ROM/RAM LD-BYTES flash-load works (e.g. The Boggit).
        if tape::TzxPlayer::is_standard_speed_only(&data) {
            match tape::TzxPlayer::to_tap_player(&data) {
                Ok(player) if player.image.blocks.is_empty() => {}
                Ok(player) => {
                    let n = player.image.blocks.len();
                    m.insert_tape(player);
                    self.status = format!(
                        "Inserted TZX {} as TAP ({n} blocks, paused). Type LOAD \"\" (PROGRAM) or LOAD \"\" CODE, then Play. 128K/+3: Type LOAD enters 48 BASIC (not +3 disk Loader). Instant flash-load skips ROM gaps; custom loaders still use EAR.",
                        path.display()
                    );
                    return;
                }
                Err(e) => {
                    self.status = format!("TZX error: {e}");
                    return;
                }
            }
        }
        match tape::TzxPlayer::parse(&data) {
            Ok(player) => {
                m.insert_tzx(player);
                self.status = format!(
                    "Inserted TZX {} (pulse playback, paused — Play when loader is ready)",
                    path.display()
                );
            }
            Err(e) => self.status = format!("TZX error: {e}"),
        }
    }

    pub fn play_tape(&mut self) {
        if let Some(m) = self.machine.as_mut() {
            if m.has_tape() {
                m.set_tape_playing(true);
                self.status = "Tape playing".into();
            } else {
                self.status = "No tape inserted".into();
            }
        }
    }

    pub fn pause_tape(&mut self) {
        if let Some(m) = self.machine.as_mut() {
            m.set_tape_playing(false);
            self.status = "Tape paused".into();
        }
    }

    pub fn rewind_tape(&mut self) {
        if let Some(m) = self.machine.as_mut() {
            m.rewind_tape();
            self.status = "Tape rewound (paused)".into();
        }
    }

    /// Queue `LOAD ""` + Enter for 48K keyword mode; press Play separately when LD-BYTES waits.
    pub fn type_load_quotes(&mut self) {
        self.type_load_quotes_inner(false);
    }

    /// Queue `LOAD "" CODE` + Enter (48K) for CODE blocks.
    pub fn type_load_quotes_code(&mut self) {
        self.type_load_quotes_inner(true);
    }

    fn type_load_quotes_inner(&mut self, with_code: bool) {
        // +3 menu "Loader" is +3DOS disk — never Enter alone for tape (#144).
        // 128K/+3: 48 BASIC then keyword LOAD (matches Machine::type_load_quotes_*).
        self.key_script = Some(match self.model {
            Model::Spectrum48 => {
                if with_code {
                    KeyScript::load_quotes_code_48k()
                } else {
                    KeyScript::load_quotes_48k()
                }
            }
            Model::Spectrum128 | Model::SpectrumPlus3 => {
                KeyScript::load_quotes_128_or_plus3(with_code)
            }
        });
        self.status = match (self.model, with_code) {
            (Model::Spectrum48, true) => {
                "Typing LOAD \"\" CODE — press Tape → Play when border goes red/cyan".into()
            }
            (Model::Spectrum48, false) => {
                "Typing LOAD \"\" — press Tape → Play when the border goes red/cyan".into()
            }
            (_, true) => {
                "Typing 48 BASIC LOAD \"\" CODE — press Tape → Play when border goes red/cyan"
                    .into()
            }
            (_, false) => {
                "Typing 48 BASIC LOAD \"\" — press Tape → Play when the border goes red/cyan".into()
            }
        };
    }

    /// Advance scripted keys if any; returns true when a script consumed input this frame.
    pub fn tick_key_script(&mut self) -> bool {
        if self.machine.is_none() {
            return false;
        }
        let Some(script) = self.key_script.as_mut() else {
            return false;
        };
        if script.step_i >= script.steps.len() {
            self.key_script = None;
            return false;
        }
        if script.frames_left == 0 {
            script.frames_left = script.steps[script.step_i].1.max(1);
        }
        let keys = script.steps[script.step_i].0.clone();
        script.frames_left -= 1;
        if script.frames_left == 0 {
            script.step_i += 1;
        }
        if script.step_i >= script.steps.len() {
            self.key_script = None;
        }
        if let Some(machine) = self.machine.as_mut() {
            let kb = machine.keyboard_mut();
            kb.reset();
            for &(row, bit) in &keys {
                kb.set_key(row, bit, true);
            }
        }
        true
    }

    pub fn load_rzx(&mut self, path: &Path) {
        match formats::RzxRecording::load(path) {
            Ok(rec) => {
                if let Some(m) = self.machine.as_mut() {
                    m.insert_rzx(rec);
                    self.status = format!("Loaded RZX {}", path.display());
                } else {
                    self.status = "Load a machine ROM before RZX".into();
                }
            }
            Err(e) => self.status = format!("RZX error: {e}"),
        }
    }

    pub fn load_dsk(&mut self, path: &Path) {
        match formats::DskImage::load(path) {
            Ok(img) => {
                if let Some(m) = self.machine.as_mut() {
                    match m.insert_disk(img) {
                        Ok(()) => self.status = format!("Inserted DSK {}", path.display()),
                        Err(e) => self.status = e,
                    }
                } else {
                    self.status = "Load +2A/+3 ROM before inserting disk".into();
                }
            }
            Err(e) => self.status = format!("DSK error: {e}"),
        }
    }

    /// Rebuild the Spectrum matrix from currently held egui keys (macOS-friendly).
    ///
    /// `pad` is ORed with arrow/Tab keyboard stick. Joystick modes that touch the
    /// matrix run first; physical key chords are restored afterward so held digits
    /// (e.g. Num1–Num5 in Sinclair-left) are not cleared by joystick routing.
    pub fn sync_keyboard(
        &mut self,
        keys_down: &std::collections::HashSet<egui::Key>,
        modifiers: egui::Modifiers,
        pad: JoystickState,
    ) {
        let Some(machine) = self.machine.as_mut() else {
            return;
        };
        machine.keyboard_mut().reset();
        let mut stick = pad;
        stick.left |= keys_down.contains(&egui::Key::ArrowLeft);
        stick.right |= keys_down.contains(&egui::Key::ArrowRight);
        stick.up |= keys_down.contains(&egui::Key::ArrowUp);
        stick.down |= keys_down.contains(&egui::Key::ArrowDown);
        stick.fire |= keys_down.contains(&egui::Key::Tab);
        machine.apply_joystick_state(self.joystick_mode, stick);

        let kb = machine.keyboard_mut();
        let suppress_caps = keys_down
            .iter()
            .any(|k| keymap::suppresses_modifier_caps(*k));
        for (row, bit) in keymap::modifier_keys(modifiers, suppress_caps) {
            kb.set_key(row, bit, true);
        }
        for key in keys_down {
            // Arrow cursor chords are applied via joystick mode instead.
            if matches!(
                key,
                egui::Key::ArrowLeft
                    | egui::Key::ArrowRight
                    | egui::Key::ArrowUp
                    | egui::Key::ArrowDown
                    | egui::Key::Tab
            ) {
                continue;
            }
            if let Some(chord) = keymap::chord_for(*key, modifiers) {
                for (row, bit) in chord.keys {
                    kb.set_key(row, bit, true);
                }
            }
        }
    }

    /// Apply a single egui key edge (tests / scripted input).
    pub fn apply_key(&mut self, key: egui::Key, pressed: bool) {
        let modifiers = egui::Modifiers::default();
        let Some(machine) = self.machine.as_mut() else {
            return;
        };
        let kb = machine.keyboard_mut();
        if let Some(chord) = keymap::chord_for(key, modifiers) {
            for (row, bit) in chord.keys {
                kb.set_key(row, bit, pressed);
            }
        }
    }

    pub fn apply_modifiers(&mut self, modifiers: egui::Modifiers) {
        let Some(machine) = self.machine.as_mut() else {
            return;
        };
        let kb = machine.keyboard_mut();
        for (row, bit) in keymap::modifier_keys(modifiers, false) {
            kb.set_key(row, bit, true);
        }
        if !modifiers.shift {
            kb.set_key(keymap::CAPS.0, keymap::CAPS.1, false);
        }
        if !(modifiers.alt || modifiers.ctrl) {
            kb.set_key(keymap::SYM.0, keymap::SYM.1, false);
        }
    }

    /// Run one emulated frame into the RGBA framebuffer when `running`.
    pub fn tick_frame(&mut self) -> machine::FrameAudio {
        if !self.running {
            return machine::FrameAudio::default();
        }
        let Some(machine) = self.machine.as_mut() else {
            return machine::FrameAudio::default();
        };
        if machine.debugger().paused {
            machine.render_rgba(&mut self.framebuffer, self.with_border);
            return machine::FrameAudio::default();
        }
        let audio = machine.run_frame();
        machine.render_rgba(&mut self.framebuffer, self.with_border);
        audio
    }
}

pub struct SpecChumApp {
    pub session: EmulatorSession,
    texture: Option<egui::TextureHandle>,
    beeper: Arc<Mutex<BeeperState>>,
    _stream: Option<cpal::Stream>,
    theme_applied: bool,
    /// Optional gamepad (USB/Bluetooth via gilrs). `None` if init failed.
    gilrs: Option<gilrs::Gilrs>,
}

impl std::fmt::Debug for SpecChumApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpecChumApp")
            .field("session", &self.session)
            .field("has_texture", &self.texture.is_some())
            .field("has_audio", &self._stream.is_some())
            .finish()
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
        let beeper = Arc::new(Mutex::new(BeeperState {
            frame_t_per_sample: 69888.0 / 44100.0,
            ..BeeperState::default()
        }));
        let stream = if start_audio {
            start_beeper(Arc::clone(&beeper))
        } else {
            None
        };
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        let gilrs = match gilrs::Gilrs::new() {
            Ok(g) => Some(g),
            Err(gilrs::Error::NotImplemented(g)) => Some(g),
            Err(e) => {
                session.status = format!("{} (gamepad unavailable: {e})", session.status);
                None
            }
        };
        Self {
            session,
            texture: None,
            beeper,
            _stream: stream,
            theme_applied: false,
            gilrs,
        }
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
                    ui.menu_button("File", |ui| {
                        if ui.button("Open snapshot (SNA/Z80)…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("Snapshots", &["sna", "z80"])
                                .pick_file()
                            {
                                self.session.load_snapshot(&path);
                            }
                            ui.close_menu();
                        }
                        if ui.button("Open TAP…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("TAP", &["tap"])
                                .pick_file()
                            {
                                self.session.load_tap(&path);
                            }
                            ui.close_menu();
                        }
                        if ui.button("Open TZX…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("TZX", &["tzx"])
                                .pick_file()
                            {
                                self.session.load_tzx(&path);
                            }
                            ui.close_menu();
                        }
                        if ui.button("Open RZX…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("RZX", &["rzx"])
                                .pick_file()
                            {
                                self.session.load_rzx(&path);
                            }
                            ui.close_menu();
                        }
                        if ui.button("Open DSK…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("DSK", &["dsk"])
                                .pick_file()
                            {
                                self.session.load_dsk(&path);
                            }
                            ui.close_menu();
                        }
                        if ui.button("Quit").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });
                    ui.menu_button("Machine", |ui| {
                        if ui
                            .radio_value(&mut self.session.model, Model::Spectrum48, "Spectrum 48K")
                            .clicked()
                        {
                            self.session.try_autoload_rom();
                        }
                        if ui
                            .radio_value(
                                &mut self.session.model,
                                Model::Spectrum128,
                                "Spectrum 128K",
                            )
                            .clicked()
                        {
                            self.session.try_autoload_rom();
                        }
                        if ui
                            .radio_value(
                                &mut self.session.model,
                                Model::SpectrumPlus3,
                                "Spectrum +2A/+3",
                            )
                            .clicked()
                        {
                            self.session.try_autoload_rom();
                        }
                        if ui.button("Reset").clicked() {
                            if let Some(m) = self.session.machine.as_mut() {
                                m.reset();
                                self.session.status = "Reset".into();
                            }
                            ui.close_menu();
                        }
                        ui.checkbox(&mut self.session.running, "Running");
                        ui.checkbox(&mut self.session.throttle, "Throttle ~50Hz");
                        ui.separator();
                        ui.label("Joystick");
                        ui.radio_value(
                            &mut self.session.joystick_mode,
                            JoystickMode::Kempston,
                            "Kempston",
                        );
                        ui.radio_value(
                            &mut self.session.joystick_mode,
                            JoystickMode::SinclairLeft,
                            "Sinclair left (1–5)",
                        );
                        ui.radio_value(
                            &mut self.session.joystick_mode,
                            JoystickMode::SinclairRight,
                            "Sinclair right (6–0)",
                        );
                        ui.radio_value(
                            &mut self.session.joystick_mode,
                            JoystickMode::Cursor,
                            "Cursor",
                        );
                        ui.checkbox(&mut self.session.kempston_mouse, "Kempston mouse");
                        if matches!(
                            self.session.model,
                            Model::Spectrum128 | Model::SpectrumPlus3
                        ) {
                            ui.separator();
                            ui.label("AY stereo");
                            let mut mode = self
                                .session
                                .machine
                                .as_ref()
                                .map_or(AyStereoMode::Mono, Machine::ay_stereo_mode);
                            ui.radio_value(&mut mode, AyStereoMode::Mono, "Mono");
                            ui.radio_value(&mut mode, AyStereoMode::Acb, "ACB");
                            ui.radio_value(&mut mode, AyStereoMode::Abc, "ABC");
                            if let Some(m) = self.session.machine.as_mut() {
                                m.set_ay_stereo_mode(mode);
                            }
                        }
                        ui.checkbox(&mut self.session.muted, "Mute");
                    });
                    ui.menu_button("Tape", |ui| {
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
                        if ui.button("Type LOAD \"\"").clicked() {
                            self.session.type_load_quotes();
                            ui.close_menu();
                        }
                        if ui.button("Type LOAD \"\" CODE").clicked() {
                            self.session.type_load_quotes_code();
                            ui.close_menu();
                        }
                        ui.separator();
                        if let Some(m) = self.session.machine.as_mut() {
                            let mut opts = m.tape_load_options();
                            let mut instant = opts.flash_load;
                            if ui.checkbox(&mut instant, "Instant flash-load").changed() {
                                opts.flash_load = instant;
                                m.set_tape_load_options(opts);
                                self.session.status = if instant {
                                    "Tape: instant flash-load".into()
                                } else {
                                    format!("Tape: EAR load at {}x", opts.speed)
                                };
                            }
                            ui.label("EAR speed:");
                            for speed in [1u32, 2, 5, 10, 20] {
                                let selected = opts.speed == speed;
                                if ui.selectable_label(selected, format!("{speed}x")).clicked() {
                                    opts.speed = speed;
                                    m.set_tape_load_options(opts);
                                    self.session.status = if opts.flash_load {
                                        format!("Tape: instant flash-load, EAR speed {speed}x")
                                    } else {
                                        format!("Tape: EAR load at {speed}x")
                                    };
                                }
                            }
                            if ui
                                .button("Experience (~20s EAR)")
                                .on_hover_text(
                                    "EAR path at 16x (issue #82 interim; abbreviate tones later)",
                                )
                                .clicked()
                            {
                                opts.flash_load = false;
                                opts.speed = 16;
                                m.set_tape_load_options(opts);
                                self.session.status =
                                    "Tape: experience EAR load at 16x (~20s-class)".into();
                            }
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
                            self.session.status = format!(
                                "Trace categories=0x{:x} ({} events)",
                                c.bits(),
                                trace::len()
                            );
                        }
                        if ui.button("Clear ring").clicked() {
                            trace::clear();
                            self.session.status = "Trace cleared".into();
                            ui.close_menu();
                        }
                        if ui.button("Dump to stderr").clicked() {
                            trace::dump_to_stderr();
                            self.session.status =
                                format!("Dumped {} trace events to stderr", trace::len());
                            ui.close_menu();
                        }
                        if ui.button("Dump to file…").clicked() {
                            if let Some(path) = rfd::FileDialog::new()
                                .set_file_name("spec_chum_trace.txt")
                                .save_file()
                            {
                                match trace::dump_to_file(&path) {
                                    Ok(()) => {
                                        self.session.status =
                                            format!("Trace dump → {}", path.display());
                                    }
                                    Err(e) => {
                                        self.session.status = format!("Trace dump failed: {e}");
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
                    });
                    ui.separator();
                    if let Some(p) = self
                        .session
                        .machine
                        .as_ref()
                        .and_then(Machine::tape_progress)
                    {
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
                    }
                    if self
                        .session
                        .machine
                        .as_ref()
                        .is_some_and(Machine::tape_playing)
                    {
                        ui.strong("▶ tape");
                    }
                    ui.label(&self.session.status);
                });
            });

        if !self.session.tick_key_script() {
            let (keys_down, modifiers) = ctx.input(|i| (i.keys_down.clone(), i.modifiers));
            let pad = self.poll_gamepad();
            self.session.sync_keyboard(&keys_down, modifiers, pad);
        }

        if self.session.kempston_mouse {
            if let Some(machine) = self.session.machine.as_mut() {
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
        } else if let Some(machine) = self.session.machine.as_mut() {
            // Drop guest button state when host mouse input is toggled off mid-press.
            machine.mouse_mut().set_buttons(false, false, false);
        }

        let audio = self.session.tick_frame();
        if let Ok(mut b) = self.beeper.lock() {
            b.muted = self.session.muted;
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
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [self.session.width, self.session.height],
                &self.session.framebuffer,
            );
            let tex = self.texture.get_or_insert_with(|| {
                ctx.load_texture("screen", image.clone(), egui::TextureOptions::NEAREST)
            });
            tex.set(image, egui::TextureOptions::NEAREST);
            let src = egui::vec2(self.session.width as f32, self.session.height as f32);
            let avail = ui.available_size();
            let fitted = display::fit_size(src, avail);
            ui.centered_and_justified(|ui| {
                ui.image((tex.id(), fitted));
            });
        });

        self.debug_window(ctx);

        let paused = self
            .session
            .machine
            .as_ref()
            .is_some_and(|m| m.debugger().paused);
        let advancing = self.session.running && !paused;
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
                let Some(m) = self.session.machine.as_mut() else {
                    ui.label("No machine loaded");
                    return;
                };
                let paused = m.debugger().paused;
                ui.horizontal(|ui| {
                    if paused {
                        if ui.button("Run").clicked() {
                            let pc = m.cpu().regs.pc;
                            m.debugger_mut().continue_from_pc(pc);
                            self.session.running = true;
                        }
                    } else if ui.button("Pause").clicked() {
                        m.debugger_mut().paused = true;
                        self.session.status = "Paused".into();
                    }
                    if ui.button("Step").clicked() {
                        let pc = m.cpu().regs.pc;
                        if m.debugger().paused {
                            m.debugger_mut().continue_from_pc(pc);
                        }
                        m.debugger_mut().paused = false;
                        m.step_once();
                        m.debugger_mut().paused = true;
                        m.render_rgba(&mut self.session.framebuffer, self.session.with_border);
                    }
                });
                let ins = m.inspect();
                ui.separator();
                ui.monospace(format!("{ins}"));
                ui.separator();
                ui.label("Disassembly");
                ui.monospace(m.disasm_window(m.cpu().regs.pc, 12));
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
                        self.session.debug_mem_addr = m.cpu().regs.pc;
                    }
                    if ui.button("SP").clicked() {
                        self.session.debug_mem_addr = m.cpu().regs.sp;
                    }
                    if ui.button("HL").clicked() {
                        self.session.debug_mem_addr = m.cpu().regs.hl();
                    }
                });
                ui.monospace(m.hexdump(self.session.debug_mem_addr, 64));
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
                        self.session.debug_break_pc = m.cpu().regs.pc;
                    }
                    if ui.button("Add").clicked() {
                        m.debugger_mut().add_pc_break(self.session.debug_break_pc);
                    }
                    if ui.button("Clear breaks").clicked() {
                        m.debugger_mut().clear_breaks();
                    }
                });
                ui.label(format!("PC breaks: {:?}", m.debugger().pc_breaks));
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

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui(ctx);
    }
}

fn start_beeper(state: Arc<Mutex<BeeperState>>) -> Option<cpal::Stream> {
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
                    let left = (beep + ay_l).clamp(-1.0, 1.0);
                    let right = (beep + ay_r).clamp(-1.0, 1.0);
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_maps_to_symbol_p_on_session() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if session.machine.is_none() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        let mut keys = std::collections::HashSet::new();
        keys.insert(egui::Key::Quote);
        let mods = egui::Modifiers {
            shift: true,
            ..Default::default()
        };
        session.sync_keyboard(&keys, mods, JoystickState::empty());
        let kb = session.machine.as_mut().unwrap().keyboard_mut();
        // Symbol (row7 bit1) and P (row5 bit0) active-low
        assert_eq!(kb.rows[7] & (1 << 1), 0);
        assert_eq!(kb.rows[5] & (1 << 0), 0);
        // Caps must not be forced by Shift for punctuation
        assert_ne!(kb.rows[0] & (1 << 0), 0);
    }

    #[test]
    fn arrow_left_maps_joystick_kempston_and_cursor_mode() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if session.machine.is_none() {
            return;
        }
        let mut keys = std::collections::HashSet::new();
        keys.insert(egui::Key::ArrowLeft);
        session.joystick_mode = JoystickMode::Kempston;
        session.sync_keyboard(&keys, egui::Modifiers::default(), JoystickState::empty());
        assert!(session.machine.as_mut().unwrap().kempston_mut().left);

        session.joystick_mode = JoystickMode::Cursor;
        session.sync_keyboard(&keys, egui::Modifiers::default(), JoystickState::empty());
        let m = session.machine.as_mut().unwrap();
        let rows = m.keyboard_mut().rows;
        assert_eq!(rows[0] & 1, 0); // Caps
        assert_eq!(rows[3] & (1 << 4), 0); // 5
        assert!(!m.kempston_mut().left);
    }

    #[test]
    fn physical_num1_survives_sinclair_left_joystick_clear() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if session.machine.is_none() {
            return;
        }
        let mut keys = std::collections::HashSet::new();
        keys.insert(egui::Key::Num1);
        session.joystick_mode = JoystickMode::SinclairLeft;
        session.sync_keyboard(&keys, egui::Modifiers::default(), JoystickState::empty());
        let rows = session.machine.as_mut().unwrap().keyboard_mut().rows;
        // Num1 = row 3 bit 0 must stay pressed even though Sinclair clears that row first.
        assert_eq!(rows[3] & (1 << 0), 0);
    }

    #[test]
    fn physical_num5_survives_cursor_joystick_clear() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if session.machine.is_none() {
            return;
        }
        let mut keys = std::collections::HashSet::new();
        keys.insert(egui::Key::Num5);
        session.joystick_mode = JoystickMode::Cursor;
        session.sync_keyboard(&keys, egui::Modifiers::default(), JoystickState::empty());
        let rows = session.machine.as_mut().unwrap().keyboard_mut().rows;
        // Num5 = row 3 bit 4 (also used as Cursor left); physical hold must remain.
        assert_eq!(rows[3] & (1 << 4), 0);
    }

    fn synthetic_sna48_bytes() -> Vec<u8> {
        let mut data = vec![0u8; 49179];
        data[26] = 5;
        data[23] = 0x00;
        data[24] = 0x40;
        data[27] = 0x00;
        data[28] = 0x80;
        data[27 + 0x4000] = 0xaa;
        data
    }

    #[test]
    fn load_snapshot48_switches_from_128k() {
        let rom48 = EmulatorSession::workspace_root().join("roms/spec48.rom");
        if !rom48.is_file() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        let mut session = EmulatorSession::new(Model::Spectrum128, true);
        session.try_autoload_rom();
        if session.machine.is_none() {
            eprintln!("skip: 128K ROM missing");
            return;
        }
        let path = std::env::temp_dir().join("spec_chum_app_128_to_48.sna");
        std::fs::write(&path, synthetic_sna48_bytes()).expect("write sna");
        session.load_snapshot(&path);
        assert_eq!(session.model, Model::Spectrum48);
        assert_eq!(
            session.machine.as_ref().map(Machine::model),
            Some(Model::Spectrum48)
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_snapshot48_switches_from_plus3() {
        let rom48 = EmulatorSession::workspace_root().join("roms/spec48.rom");
        if !rom48.is_file() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        let mut session = EmulatorSession::new(Model::SpectrumPlus3, true);
        session.try_autoload_rom();
        if session.machine.is_none() {
            eprintln!("skip: +3 ROM missing");
            return;
        }
        let path = std::env::temp_dir().join("spec_chum_app_plus3_to_48.sna");
        std::fs::write(&path, synthetic_sna48_bytes()).expect("write sna");
        session.load_snapshot(&path);
        assert_eq!(session.model, Model::Spectrum48);
        assert_eq!(
            session.machine.as_ref().map(Machine::model),
            Some(Model::Spectrum48)
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tzx_standard_inserts_as_paused_tap() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if session.machine.is_none() {
            return;
        }
        let boggit = PathBuf::from("/Users/michael/Downloads/BoggitThe/The Boggit - Side 1.tzx");
        if !boggit.exists() {
            // Fall back to synthesizing via fixture TAP path only
            let tap =
                EmulatorSession::workspace_root().join("tests/fixtures/tape/minimal_code.tap");
            session.load_tap(&tap);
            assert!(!session.machine.as_ref().unwrap().tape_playing());
            session.play_tape();
            assert!(session.machine.as_ref().unwrap().tape_playing());
            return;
        }
        session.load_tzx(&boggit);
        assert!(
            session.status.contains("as TAP"),
            "status={}",
            session.status
        );
        assert!(!session.machine.as_ref().unwrap().tape_playing());
        assert!(session.machine.as_ref().unwrap().has_tape());
        session.type_load_quotes();
        assert!(session.key_script.is_some());
        // PRESS/GAP timings need more than 40 frames for the full script.
        for _ in 0..200 {
            if !session.tick_key_script() {
                break;
            }
        }
        assert!(session.key_script.is_none(), "script should finish");
    }

    #[test]
    fn type_load_quotes_code_script_finishes() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if session.machine.is_none() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        session.type_load_quotes_code();
        assert!(session.key_script.is_some());
        for _ in 0..200 {
            if !session.tick_key_script() {
                break;
            }
        }
        assert!(session.key_script.is_none(), "CODE script should finish");
    }

    /// +3 Type LOAD must queue keys (not a disk-Loader hint) and flash-load attr_mark (#144).
    #[test]
    fn plus3_type_load_code_flash_loads_attr_mark() {
        let mut session = EmulatorSession::new(Model::SpectrumPlus3, true);
        session.try_autoload_rom();
        if session.machine.is_none() {
            eprintln!("skip: plus3/plus2a ROM missing");
            return;
        }
        for _ in 0..250 {
            session.tick_frame();
        }
        let tap = EmulatorSession::workspace_root().join("tests/fixtures/tape/attr_mark.tap");
        session.load_tap(&tap);
        session.type_load_quotes_code();
        assert!(
            session.key_script.is_some(),
            "plus3 Type LOAD must queue a key script, not a status-only Loader hint"
        );
        assert!(
            !session.status.to_lowercase().contains("tape loader"),
            "must not point +3 users at disk Loader; status={}",
            session.status
        );
        // Menu → 48 BASIC wait (~120) + LOAD "" CODE (~100) needs >300 frames.
        for _ in 0..500 {
            let _ = session.tick_key_script();
            session.tick_frame();
            if session.key_script.is_none() {
                break;
            }
        }
        assert!(
            session.key_script.is_none(),
            "plus3 Type LOAD script should finish"
        );
        session.play_tape();
        let mut loaded = false;
        for _ in 0..400 {
            session.tick_frame();
            let m = session.machine.as_ref().unwrap();
            let code_ok = m.read_mem(0x8000) == 0x21
                && m.read_mem(0x8001) == 0x00
                && m.read_mem(0x8002) == 0x58
                && m.read_mem(0x8003) == 0x36
                && m.read_mem(0x8004) == 0xd7
                && m.read_mem(0x8005) == 0xc9;
            if code_ok {
                loaded = true;
                break;
            }
        }
        assert!(
            loaded,
            "plus3 egui Type LOAD CODE + Play should flash-load attr_mark at 0x8000 (PC={:04X} block={:?})",
            session.machine.as_ref().unwrap().cpu().regs.pc,
            session.machine.as_ref().unwrap().tape_block()
        );
    }

    #[test]
    fn play_tape_advances_ear_on_fixture() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if session.machine.is_none() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        let tap = EmulatorSession::workspace_root().join("tests/fixtures/tape/minimal_code.tap");
        session.load_tap(&tap);
        if let Some(m) = session.machine.as_mut() {
            let mut opts = m.tape_load_options();
            opts.flash_load = false;
            m.set_tape_load_options(opts);
        }
        assert!(!session.machine.as_ref().unwrap().tape_playing());
        for _ in 0..3 {
            session.tick_frame();
        }
        assert!(
            !session.machine.as_ref().unwrap().ear(),
            "paused tape must not drive EAR"
        );
        session.play_tape();
        assert!(session.machine.as_ref().unwrap().tape_playing());
        assert!(session.status.contains("playing"));
        let mut saw_high = false;
        for _ in 0..8 {
            session.tick_frame();
            if session.machine.as_ref().unwrap().ear() {
                saw_high = true;
                break;
            }
        }
        assert!(saw_high, "Tape → Play must raise EAR during pilot");
    }

    #[test]
    fn session_loads_tap_fixture_headless() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if session.machine.is_none() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        let tap = EmulatorSession::workspace_root().join("tests/fixtures/tape/minimal_code.tap");
        session.load_tap(&tap);
        assert!(
            session.status.contains("Inserted TAP"),
            "status={}",
            session.status
        );
        assert_eq!(
            session.machine.as_ref().and_then(Machine::tape_block),
            Some(0)
        );
        let before = session.framebuffer.clone();
        for _ in 0..5 {
            session.tick_frame();
        }
        assert_ne!(session.framebuffer, before, "framebuffer should update");
        assert!(session.machine.as_ref().unwrap().cpu().t > 0);
    }

    #[test]
    fn egui_menu_smoke_without_window() {
        let mut app = SpecChumApp::new_with_audio(false);
        let ctx = egui::Context::default();
        let raw = egui::RawInput::default();
        let mut saw_file = false;
        let mut saw_machine = false;
        let _ = ctx.run(raw, |ctx| {
            app.ui(ctx);
            saw_file = true;
            saw_machine = app.session.status.contains("Loaded")
                || app.session.status.contains("Missing")
                || app.session.status.contains("ROM");
        });
        assert!(saw_file);
        assert!(saw_machine, "status={}", app.session.status);
        // Second frame after menus exist — menu strip must reserve clickable height.
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.ui(ctx);
            assert!(
                theme::menu_bar_min_height() >= 28.0,
                "menu bar must be tall enough to click"
            );
        });
        assert_eq!(app.session.framebuffer.len(), 352 * 296 * 4);
        assert_eq!(ctx.style().visuals.panel_fill.a(), 255);
    }

    #[test]
    fn debug_window_smoke_headless() {
        let mut app = SpecChumApp::new_with_audio(false);
        app.session.debug_open = true;
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.ui(ctx);
        });
        assert!(app.session.debug_open);
        if let Some(m) = app.session.machine.as_mut() {
            let pc = m.cpu().regs.pc;
            m.step_once();
            assert_ne!(m.cpu().regs.pc, pc, "step_once should advance PC");
        }
    }
}
