//! Safe session wrapper around [`machine::Machine`] for host frontends.

use std::path::Path;

use machine::{JoystickMode, JoystickState, Machine, Model};
use thiserror::Error;

/// Model identifiers for the C ABI (stable numeric values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ModelId {
    Spectrum48 = 0,
    Spectrum128 = 1,
    SpectrumPlus3 = 2,
    /// Amstrad +2A (no disk interface). Added after Plus3; keep numeric id stable.
    SpectrumPlus2A = 3,
}

impl ModelId {
    #[must_use]
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::Spectrum48),
            1 => Some(Self::Spectrum128),
            2 => Some(Self::SpectrumPlus3),
            3 => Some(Self::SpectrumPlus2A),
            _ => None,
        }
    }

    #[must_use]
    pub fn to_model(self) -> Model {
        match self {
            Self::Spectrum48 => Model::Spectrum48,
            Self::Spectrum128 => Model::Spectrum128,
            Self::SpectrumPlus3 => Model::SpectrumPlus3,
            Self::SpectrumPlus2A => Model::SpectrumPlus2A,
        }
    }
}

#[derive(Debug, Error)]
pub enum HostError {
    #[error("{0}")]
    Message(String),
    #[error("no machine loaded")]
    NoMachine,
    #[error("invalid model id")]
    BadModel,
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Core registers exposed through `sc_regs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HostRegs {
    pub pc: u16,
    pub sp: u16,
    pub af: u16,
    pub bc: u16,
    pub de: u16,
    pub hl: u16,
    pub ix: u16,
    pub iy: u16,
}

/// Host-owned emulator session: machine + RGBA framebuffer + status.
#[derive(Debug)]
pub struct HostSession {
    machine: Option<Machine>,
    model: ModelId,
    with_border: bool,
    framebuffer: Vec<u8>,
    width: usize,
    height: usize,
    running: bool,
    status: String,
    /// Mono PCM for the last frame (~882 samples @ 44100 Hz / 50 fps).
    audio_pcm: Vec<f32>,
    /// Mixed speaker level carried across frame boundaries (beeper edges reset each frame).
    last_speaker_level: bool,
    /// Host joystick presentation mode.
    joystick_mode: JoystickMode,
    /// Last applied host joystick mask state.
    joystick_state: JoystickState,
    /// Host-held Spectrum matrix keys so Sinclair/Cursor joystick clears do not drop them.
    host_keys: [[bool; 5]; 8],
}

impl HostSession {
    #[must_use]
    pub fn new(model: ModelId, with_border: bool) -> Self {
        trace::init_from_env();
        let (width, height) = dims(with_border);
        Self {
            machine: None,
            model,
            with_border,
            framebuffer: vec![0; width * height * 4],
            width,
            height,
            running: true,
            status: "No ROM loaded".into(),
            audio_pcm: Vec::new(),
            last_speaker_level: false,
            joystick_mode: JoystickMode::Kempston,
            joystick_state: JoystickState::empty(),
            host_keys: [[false; 5]; 8],
        }
    }

    #[must_use]
    pub fn model(&self) -> ModelId {
        self.model
    }

    #[must_use]
    pub fn width(&self) -> usize {
        self.width
    }

    #[must_use]
    pub fn height(&self) -> usize {
        self.height
    }

    #[must_use]
    pub fn framebuffer(&self) -> &[u8] {
        &self.framebuffer
    }

    #[must_use]
    pub fn status(&self) -> &str {
        &self.status
    }

    #[must_use]
    pub fn running(&self) -> bool {
        self.running
    }

    pub fn set_running(&mut self, running: bool) {
        self.running = running;
    }

    pub fn set_border(&mut self, with_border: bool) {
        if self.with_border == with_border {
            return;
        }
        self.with_border = with_border;
        let (w, h) = dims(with_border);
        self.width = w;
        self.height = h;
        self.framebuffer.resize(w * h * 4, 0);
        if let Some(m) = self.machine.as_ref() {
            m.render_rgba(&mut self.framebuffer, self.with_border);
        }
    }

    #[must_use]
    pub fn with_border(&self) -> bool {
        self.with_border
    }

    #[must_use]
    pub fn has_machine(&self) -> bool {
        self.machine.is_some()
    }

    #[must_use]
    pub fn tape_playing(&self) -> bool {
        self.machine.as_ref().is_some_and(Machine::tape_playing)
    }

    #[must_use]
    pub fn has_tape(&self) -> bool {
        self.machine.as_ref().is_some_and(Machine::has_tape)
    }

    /// Tape progress for UI, if a deck is inserted.
    #[must_use]
    pub fn tape_progress(&self) -> Option<machine::TapeProgress> {
        self.machine.as_ref().and_then(Machine::tape_progress)
    }

    /// Mono PCM samples from the last [`Self::run_frame`] (empty if no machine).
    #[must_use]
    pub fn audio_pcm(&self) -> &[f32] {
        &self.audio_pcm
    }

    pub fn set_model(&mut self, model: ModelId) {
        self.model = model;
        self.machine = None;
        self.status = format!("Model set to {:?}; load a ROM", model);
    }

    /// Switch the selected model and autoload its ROM from `roms/`.
    ///
    /// After this returns, [`Self::model`] always matches `model`. On success the
    /// loaded [`Machine`] matches too; on ROM miss the machine stays unloaded.
    pub fn select_model(&mut self, model: ModelId) -> Result<(), HostError> {
        self.set_model(model);
        self.try_autoload_rom();
        if self.has_machine() {
            Ok(())
        } else {
            Err(HostError::Message(format!(
                "ROM for {model:?} not found; run ./scripts/fetch_roms.sh"
            )))
        }
    }

    /// Load ROM bytes for the current model.
    pub fn load_rom_bytes(&mut self, rom: &[u8]) -> Result<(), HostError> {
        let machine = match self.model {
            ModelId::Spectrum48 => Machine::new_48k(rom),
            ModelId::Spectrum128 => Machine::new_128k(rom),
            ModelId::SpectrumPlus3 => Machine::new_plus3(rom),
            ModelId::SpectrumPlus2A => Machine::new_plus2a(rom),
        }
        .map_err(HostError::Message)?;
        self.machine = Some(machine);
        self.reapply_host_keys();
        self.last_speaker_level = false;
        self.status = "ROM loaded".into();
        Ok(())
    }

    pub fn load_rom_path(&mut self, path: &Path) -> Result<(), HostError> {
        let data = std::fs::read(path)?;
        self.load_rom_bytes(&data)?;
        self.status = format!("Loaded {}", path.display());
        Ok(())
    }

    pub fn reset(&mut self) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        let mode = self.joystick_mode;
        let state = self.joystick_state;
        let keys = self.host_keys;
        m.reset();
        Self::recompose_input(m, mode, state, &keys);
        self.status = if m.has_tape() {
            "Reset (tape still inserted, paused)".into()
        } else {
            "Reset".into()
        };
        Ok(())
    }

    pub fn open_tape(&mut self, path: &Path) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "tap" => {
                let img =
                    tape::TapImage::load(path).map_err(|e| HostError::Message(e.to_string()))?;
                m.insert_tape(tape::TapPlayer::new(img));
                self.status = format!(
                    "Inserted TAP {} (paused — Play when loader is ready)",
                    path.display()
                );
            }
            "tzx" => {
                let data = std::fs::read(path)?;
                if tape::TzxPlayer::is_standard_speed_only(&data) {
                    match tape::TzxPlayer::to_tap_player(&data) {
                        Ok(player) if player.image.blocks.is_empty() => {}
                        Ok(player) => {
                            let n = player.image.blocks.len();
                            m.insert_tape(player);
                            self.status = format!(
                                "Inserted TZX {} as TAP ({n} blocks, paused)",
                                path.display()
                            );
                            return Ok(());
                        }
                        Err(e) => return Err(HostError::Message(e.to_string())),
                    }
                }
                let player =
                    tape::TzxPlayer::parse(&data).map_err(|e| HostError::Message(e.to_string()))?;
                m.insert_tzx(player);
                self.status = format!("Inserted TZX {} (paused)", path.display());
            }
            _ => {
                return Err(HostError::Message(format!(
                    "unsupported tape extension: {ext}"
                )));
            }
        }
        Ok(())
    }

    /// Load a SNA/Z80 snapshot (128K/+3 first, then 48K), switching model when needed.
    ///
    /// Mirrors egui `SpecChumApp::load_snapshot`: selects the matching model for 128K/+3
    /// (including 128K↔Plus3) and for 48K snapshots loaded onto a non-48K machine; requires
    /// ROM autoload before apply.
    pub fn load_snapshot(&mut self, path: &Path) -> Result<(), HostError> {
        if let Ok(snap) =
            formats::Snapshot128::load_sna(path).or_else(|_| formats::Snapshot128::load_z80(path))
        {
            let required = match snap.model {
                formats::Snapshot128Model::SpectrumPlus3 => ModelId::SpectrumPlus3,
                formats::Snapshot128Model::SpectrumPlus2A => ModelId::SpectrumPlus2A,
                formats::Snapshot128Model::Spectrum128 => ModelId::Spectrum128,
            };
            if self.machine.is_none() || self.model != required {
                self.model = required;
                self.machine = None;
                self.try_autoload_rom();
                if self.machine.is_none() {
                    return Err(HostError::Message(format!(
                        "ROM required for {required:?} snapshot not found; run ./scripts/fetch_roms.sh"
                    )));
                }
            }
            let Some(m) = self.machine.as_mut() else {
                return Err(HostError::NoMachine);
            };
            m.apply_snapshot128(&snap);
            // Model switch builds a fresh Machine — restore retained host joystick.
            m.apply_joystick_state(self.joystick_mode, self.joystick_state);
            self.reapply_host_keys();
            self.status = format!("Loaded {required:?} snapshot {}", path.display());
            return Ok(());
        }
        let snap = formats::Snapshot48::load_sna(path)
            .or_else(|_| formats::Snapshot48::load_z80(path))
            .map_err(|e| HostError::Message(e.to_string()))?;
        if self.machine.is_none() || self.model != ModelId::Spectrum48 {
            self.model = ModelId::Spectrum48;
            self.machine = None;
            self.try_autoload_rom();
            if self.machine.is_none() {
                return Err(HostError::Message(
                    "ROM required for Spectrum48 snapshot not found; run ./scripts/fetch_roms.sh"
                        .into(),
                ));
            }
        }
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        m.apply_snapshot48(&snap);
        m.apply_joystick_state(self.joystick_mode, self.joystick_state);
        self.reapply_host_keys();
        self.status = format!("Loaded snapshot {}", path.display());
        Ok(())
    }

    /// Load an RZX recording into the current machine.
    pub fn load_rzx(&mut self, path: &Path) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        let rec =
            formats::RzxRecording::load(path).map_err(|e| HostError::Message(e.to_string()))?;
        m.insert_rzx(rec);
        self.status = format!("Loaded RZX {}", path.display());
        Ok(())
    }

    /// Insert a +3 DSK image (requires a Plus3 machine).
    pub fn load_dsk(&mut self, path: &Path) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        let img = formats::DskImage::load(path).map_err(|e| HostError::Message(e.to_string()))?;
        m.insert_disk(img).map_err(HostError::Message)?;
        self.status = format!("Inserted DSK {}", path.display());
        Ok(())
    }

    /// Attach Beta Disk / TR-DOS and insert a `.trd` (48K/128K).
    pub fn load_trd(&mut self, path: &Path) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        let img = formats::TrdImage::load(path).map_err(|e| HostError::Message(e.to_string()))?;
        m.insert_trd(img).map_err(HostError::Message)?;
        self.status = format!("Inserted TRD {}", path.display());
        Ok(())
    }

    /// Load a 16 KiB TR-DOS ROM (attaches Beta on 48K/128K).
    pub fn load_trdos_rom(&mut self, path: &Path) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        let data = std::fs::read(path)?;
        m.load_trdos_rom(&data).map_err(HostError::Message)?;
        self.status = format!("Loaded TR-DOS ROM {}", path.display());
        Ok(())
    }

    /// Best-effort ROM load for the current model (workspace / `SPEC_CHUM_ROOT` / cwd).
    fn try_autoload_rom(&mut self) {
        let candidates: &[&str] = match self.model {
            ModelId::Spectrum48 => &["roms/spec48.rom"],
            ModelId::Spectrum128 => &["roms/128/spec128uk.rom"],
            ModelId::SpectrumPlus3 => &["roms/plus3/plus3.rom"],
            ModelId::SpectrumPlus2A => &["roms/plus2a/plus2a.rom", "roms/plus3/plus3.rom"],
        };
        for root in rom_search_roots() {
            for rel in candidates {
                let path = root.join(rel);
                if path.is_file() {
                    if let Ok(data) = std::fs::read(&path) {
                        if self.load_rom_bytes(&data).is_ok() {
                            self.status = format!("Loaded {}", path.display());
                            return;
                        }
                    }
                }
            }
        }
    }

    pub fn play_tape(&mut self) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        if !m.has_tape() {
            self.status = "No tape inserted".into();
            return Err(HostError::Message("no tape".into()));
        }
        m.set_tape_playing(true);
        self.status = "Tape playing".into();
        Ok(())
    }

    pub fn pause_tape(&mut self) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        m.set_tape_playing(false);
        self.status = "Tape paused".into();
        Ok(())
    }

    pub fn rewind_tape(&mut self) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        m.rewind_tape();
        self.status = "Tape rewound (paused)".into();
        Ok(())
    }

    #[must_use]
    pub fn tape_load_options(&self) -> Option<machine::TapeLoadOptions> {
        self.machine.as_ref().map(Machine::tape_load_options)
    }

    pub fn set_tape_load_options(
        &mut self,
        opts: machine::TapeLoadOptions,
    ) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        m.set_tape_load_options(opts);
        let effective = m.tape_load_options();
        let mode = if effective.flash_load {
            "instant"
        } else {
            "EAR"
        };
        self.status = format!("Tape load: {mode}, speed {}x", effective.speed);
        Ok(())
    }

    /// Set one Spectrum matrix key (`row` 0..7, `bit` 0..4).
    pub fn set_key(&mut self, row: usize, bit: u8, pressed: bool) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        if row > 7 || bit > 4 {
            return Err(HostError::Message("key row/bit out of range".into()));
        }
        self.host_keys[row][bit as usize] = pressed;
        Self::recompose_input(m, self.joystick_mode, self.joystick_state, &self.host_keys);
        Ok(())
    }

    pub fn clear_keys(&mut self) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        self.host_keys = [[false; 5]; 8];
        Self::recompose_input(m, self.joystick_mode, self.joystick_state, &self.host_keys);
        Ok(())
    }

    pub fn set_joystick_mode(&mut self, mode: JoystickMode) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        self.joystick_mode = mode;
        Self::recompose_input(m, self.joystick_mode, self.joystick_state, &self.host_keys);
        Ok(())
    }

    pub fn set_joystick(&mut self, mask: u8) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        self.joystick_state = JoystickState::from_mask(mask);
        Self::recompose_input(m, self.joystick_mode, self.joystick_state, &self.host_keys);
        Ok(())
    }

    pub fn clear_joystick(&mut self) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        self.joystick_state = JoystickState::empty();
        Self::recompose_input(m, self.joystick_mode, self.joystick_state, &self.host_keys);
        Ok(())
    }

    /// Accumulate host pointer motion into the Kempston mouse (positive `dy` = down).
    pub fn set_mouse_delta(&mut self, dx: i8, dy: i8) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        if dx != 0 || dy != 0 {
            m.mouse_mut().set_delta(dx, dy);
        }
        Ok(())
    }

    /// Set Kempston mouse button state (host primary = left).
    pub fn set_mouse_buttons(
        &mut self,
        left: bool,
        right: bool,
        middle: bool,
    ) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        m.mouse_mut().set_buttons(left, right, middle);
        Ok(())
    }

    fn reapply_host_keys(&mut self) {
        let Some(m) = self.machine.as_mut() else {
            return;
        };
        Self::recompose_input(m, self.joystick_mode, self.joystick_state, &self.host_keys);
    }

    /// Reset matrix, apply joystick routing, then overlay retained host keys.
    fn recompose_input(
        m: &mut Machine,
        mode: JoystickMode,
        state: JoystickState,
        host_keys: &[[bool; 5]; 8],
    ) {
        m.keyboard_mut().reset();
        m.apply_joystick_state(mode, state);
        let kb = m.keyboard_mut();
        for (row, bits) in host_keys.iter().enumerate() {
            for (bit, pressed) in bits.iter().enumerate() {
                if *pressed {
                    kb.set_key(row, bit as u8, true);
                }
            }
        }
    }

    /// Attach Multiface 1 (48K only) from an 8 KiB ROM image path.
    pub fn attach_multiface(&mut self, path: &Path) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        let data = std::fs::read(path)?;
        m.attach_multiface(&data).map_err(HostError::Message)?;
        self.status = format!("Attached Multiface 1 from {}", path.display());
        Ok(())
    }

    /// Raise Multiface NMI if attached.
    pub fn multiface_nmi(&mut self) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        match m.multiface_nmi() {
            Some(_) => {
                self.status = "Multiface NMI".into();
                Ok(())
            }
            None => Err(HostError::Message(
                "Multiface not attached (48K + 8K MF ROM)".into(),
            )),
        }
    }

    #[must_use]
    pub fn has_multiface(&self) -> bool {
        self.machine.as_ref().is_some_and(Machine::has_multiface)
    }

    /// Attach DivMMC on 48K/128K (no media).
    pub fn attach_divmmc(&mut self) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        m.attach_divmmc().map_err(HostError::Message)?;
        self.status = "DivMMC attached".into();
        Ok(())
    }

    /// Attach DivMMC and load a flat SD/MMC image.
    pub fn load_divmmc_sd(&mut self, path: &Path) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        let data = std::fs::read(path)?;
        let div = m.attach_divmmc().map_err(HostError::Message)?;
        div.attach_sd(data);
        self.status = format!("DivMMC SD {}", path.display());
        Ok(())
    }

    /// Attach DivMMC and load an ESXDOS EEPROM (8 KiB or larger prefix).
    pub fn load_divmmc_eeprom(&mut self, path: &Path) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        let data = std::fs::read(path)?;
        m.attach_divmmc_eeprom(&data).map_err(HostError::Message)?;
        self.status = format!("DivMMC EEPROM {}", path.display());
        Ok(())
    }

    #[must_use]
    pub fn has_divmmc(&self) -> bool {
        self.machine.as_ref().is_some_and(Machine::has_divmmc)
    }

    #[must_use]
    pub fn joystick_mode(&self) -> JoystickMode {
        self.joystick_mode
    }

    /// Peek one byte of machine memory.
    pub fn peek(&self, addr: u16) -> Result<u8, HostError> {
        let Some(m) = self.machine.as_ref() else {
            return Err(HostError::NoMachine);
        };
        Ok(m.read_mem(addr))
    }

    /// Poke one byte of machine memory.
    pub fn poke(&mut self, addr: u16, value: u8) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        m.write_mem(addr, value);
        Ok(())
    }

    /// JSON inspect snapshot (`Inspect::to_json`).
    pub fn inspect_json(&self) -> Result<String, HostError> {
        let Some(m) = self.machine.as_ref() else {
            return Err(HostError::NoMachine);
        };
        Ok(m.inspect().to_json())
    }

    /// Core registers for the C `sc_regs` ABI.
    pub fn regs(&self) -> Result<HostRegs, HostError> {
        let Some(m) = self.machine.as_ref() else {
            return Err(HostError::NoMachine);
        };
        let r = &m.cpu().regs;
        Ok(HostRegs {
            pc: r.pc,
            sp: r.sp,
            af: r.af(),
            bc: r.bc(),
            de: r.de(),
            hl: r.hl(),
            ix: r.ix(),
            iy: r.iy(),
        })
    }

    /// One CPU/machine instruction (`step_once`).
    pub fn step(&mut self) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        m.step_once();
        Ok(())
    }

    /// Set the debugger paused flag (no-op without a machine).
    pub fn set_paused(&mut self, paused: bool) {
        if let Some(m) = self.machine.as_mut() {
            m.debugger_mut().paused = paused;
        }
    }

    #[must_use]
    pub fn paused(&self) -> bool {
        self.machine.as_ref().is_some_and(|m| m.debugger().paused)
    }

    /// Add a PC breakpoint.
    pub fn add_breakpoint(&mut self, pc: u16) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        m.debugger_mut().add_pc_break(pc);
        Ok(())
    }

    /// Run until breakpoint, halt, or instruction budget.
    pub fn run_until_break(&mut self, max_insns: u32) -> Result<machine::BreakReason, HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        Ok(m.run_until_break(u64::from(max_insns)))
    }

    /// Run one video frame into the RGBA framebuffer when `running`.
    /// Skips advancing while the debugger is paused.
    pub fn run_frame(&mut self) {
        if !self.running {
            return;
        }
        let Some(m) = self.machine.as_mut() else {
            return;
        };
        if m.debugger().paused {
            return;
        }
        let audio = m.run_frame();
        let frame_t = match m.model() {
            machine::Model::Spectrum48 => 69_888,
            machine::Model::Spectrum128
            | machine::Model::SpectrumPlus2A
            | machine::Model::SpectrumPlus3 => 70_908,
        };
        self.last_speaker_level = render_frame_pcm(
            &audio,
            frame_t,
            self.last_speaker_level,
            &mut self.audio_pcm,
        );
        m.render_rgba(&mut self.framebuffer, self.with_border);
    }
}

/// Host audio sample rate (matches egui cpal default path).
pub const AUDIO_SAMPLE_RATE: u32 = 44_100;
/// Samples rendered per 50 Hz frame.
pub const AUDIO_SAMPLES_PER_FRAME: usize = (AUDIO_SAMPLE_RATE as usize) / 50;

fn render_frame_pcm(
    audio: &machine::FrameAudio,
    frame_tstates: u32,
    initial_level: bool,
    out: &mut Vec<f32>,
) -> bool {
    out.clear();
    out.resize(AUDIO_SAMPLES_PER_FRAME, 0.0);
    let t_per = frame_tstates as f32 / AUDIO_SAMPLES_PER_FRAME as f32;
    let mut edge_i = 0usize;
    let mut level = initial_level;
    let mut t = 0.0f32;
    let mut ay_i = 0usize;
    for sample in out.iter_mut() {
        while edge_i < audio.beeper_edges.len() {
            let (edge_t, edge_level) = audio.beeper_edges[edge_i];
            if t >= edge_t as f32 {
                level = edge_level;
                edge_i += 1;
            } else {
                break;
            }
        }
        let beep = if level { 0.15 } else { -0.15 };
        let ay = if ay_i < audio.ay_samples.len() {
            let v = audio.ay_samples[ay_i];
            ay_i += 1;
            (v - 0.5) * 0.5
        } else if let Some(&last) = audio.ay_samples.last() {
            (last - 0.5) * 0.5
        } else {
            0.0
        };
        *sample = (beep + ay).clamp(-1.0, 1.0);
        t += t_per;
    }
    // Edges in the final sample interval (after the last sample instant) must
    // still update the returned level so the next frame starts correctly.
    while edge_i < audio.beeper_edges.len() {
        let (edge_t, edge_level) = audio.beeper_edges[edge_i];
        if edge_t < frame_tstates {
            level = edge_level;
            edge_i += 1;
        } else {
            break;
        }
    }
    level
}

fn rom_search_roots() -> Vec<std::path::PathBuf> {
    let mut roots = Vec::new();
    if let Ok(cwd) = std::env::current_dir() {
        roots.push(cwd);
    }
    if let Ok(env) = std::env::var("SPEC_CHUM_ROOT") {
        roots.push(std::path::PathBuf::from(env));
    }
    // Dev / `cargo test`: crates/host_api → workspace root.
    roots.push(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../.."));
    roots
}

fn dims(with_border: bool) -> (usize, usize) {
    if with_border {
        (352, 296)
    } else {
        (256, 192)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    fn rom48() -> Option<Vec<u8>> {
        let path = workspace_root().join("roms/spec48.rom");
        std::fs::read(path).ok()
    }

    #[test]
    fn new_session_has_empty_framebuffer_dims() {
        let s = HostSession::new(ModelId::Spectrum48, true);
        assert_eq!(s.width(), 352);
        assert_eq!(s.height(), 296);
        assert_eq!(s.framebuffer().len(), 352 * 296 * 4);
        assert!(!s.has_machine());
    }

    #[test]
    fn border_toggle_resizes_framebuffer() {
        let mut s = HostSession::new(ModelId::Spectrum48, true);
        s.set_border(false);
        assert_eq!(s.width(), 256);
        assert_eq!(s.height(), 192);
        assert_eq!(s.framebuffer().len(), 256 * 192 * 4);
    }

    #[test]
    fn load_rom_and_run_frame_writes_pixels() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut s = HostSession::new(ModelId::Spectrum48, true);
        s.load_rom_bytes(&rom).expect("rom");
        assert!(s.has_machine());
        // Advance past cold boot snow a bit.
        for _ in 0..50 {
            s.run_frame();
        }
        let nonzero = s.framebuffer().iter().any(|&b| b != 0);
        assert!(nonzero, "expected rendered pixels after boot frames");
    }

    #[test]
    fn tape_play_requires_inserted_tape() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut s = HostSession::new(ModelId::Spectrum48, true);
        s.load_rom_bytes(&rom).expect("rom");
        assert!(s.play_tape().is_err());
        assert!(!s.tape_playing());
    }

    #[test]
    fn set_key_out_of_range_errors() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut s = HostSession::new(ModelId::Spectrum48, false);
        s.load_rom_bytes(&rom).expect("rom");
        assert!(s.set_key(8, 0, true).is_err());
        assert!(s.set_key(0, 5, true).is_err());
        s.set_key(0, 0, true).expect("caps");
    }

    #[test]
    fn set_key_injects_and_clear_resets_matrix() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut s = HostSession::new(ModelId::Spectrum48, false);
        s.load_rom_bytes(&rom).expect("rom");

        // J = row 6 bit 3 (LOAD keyword); matrix bits are active-low.
        s.set_key(6, 3, true).expect("J down");
        {
            let rows = s.machine.as_mut().expect("machine").keyboard_mut().rows;
            assert_eq!(rows[6] & (1 << 3), 0, "J bit should be pressed (cleared)");
        }

        s.set_key(7, 1, true).expect("Symbol Shift");
        {
            let rows = s.machine.as_mut().expect("machine").keyboard_mut().rows;
            assert_eq!(rows[7] & (1 << 1), 0, "Sym bit pressed");
        }

        s.clear_keys().expect("clear");
        {
            let rows = s.machine.as_mut().expect("machine").keyboard_mut().rows;
            assert!(rows.iter().all(|&r| r == 0x1f), "all rows idle after clear");
        }
    }

    #[test]
    fn set_key_holds_across_run_frames() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut s = HostSession::new(ModelId::Spectrum48, false);
        s.load_rom_bytes(&rom).expect("rom");

        // J must stay pressed for the whole hold — turbo hosts + flicker would
        // otherwise look like multiple keyword presses to 48K BASIC.
        s.set_key(6, 3, true).expect("J down");
        for i in 0..16 {
            s.run_frame();
            let rows = s.machine.as_mut().expect("machine").keyboard_mut().rows;
            assert_eq!(
                rows[6] & (1 << 3),
                0,
                "J must remain pressed after frame {i}"
            );
        }
        s.set_key(6, 3, false).expect("J up");
        {
            let rows = s.machine.as_mut().expect("machine").keyboard_mut().rows;
            assert_ne!(rows[6] & (1 << 3), 0, "J released");
        }
    }

    #[test]
    fn kempston_mouse_ports_after_synthetic_deltas() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut s = HostSession::new(ModelId::Spectrum48, false);
        s.load_rom_bytes(&rom).expect("rom");
        s.set_mouse_delta(20, -4).unwrap();
        s.set_mouse_buttons(true, true, false).unwrap();
        let mouse = s.machine.as_mut().unwrap().mouse_mut();
        assert_eq!(mouse.x, 20);
        assert_eq!(mouse.y, 4);
        assert_eq!(mouse.buttons_byte(), 0xfc); // D0+D1 clear
        s.set_mouse_buttons(false, false, false).unwrap();
        assert_eq!(s.machine.as_mut().unwrap().mouse_mut().buttons_byte(), 0xff);
    }

    #[test]
    fn joystick_kempston_mask_reaches_port() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut s = HostSession::new(ModelId::Spectrum48, false);
        s.load_rom_bytes(&rom).expect("rom");
        s.set_joystick_mode(JoystickMode::Kempston).unwrap();
        s.set_joystick(0x11).unwrap();
        assert_eq!(s.machine.as_mut().unwrap().kempston_mut().read(), 0x11);
        s.clear_joystick().unwrap();
        assert_eq!(s.machine.as_mut().unwrap().kempston_mut().read(), 0);
    }

    #[test]
    fn physical_num1_survives_sinclair_left_joystick_update() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut s = HostSession::new(ModelId::Spectrum48, false);
        s.load_rom_bytes(&rom).expect("rom");
        s.set_joystick_mode(JoystickMode::SinclairLeft).unwrap();
        // Num1 = row 3 bit 0 (also Sinclair-left left).
        s.set_key(3, 0, true).unwrap();
        s.set_joystick(0).unwrap(); // would clear Sinclair matrix without reapply
        let rows = s.machine.as_mut().unwrap().keyboard_mut().rows;
        assert_eq!(rows[3] & (1 << 0), 0, "Num1 must stay pressed");
    }

    #[test]
    fn physical_num5_survives_cursor_joystick_update() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut s = HostSession::new(ModelId::Spectrum48, false);
        s.load_rom_bytes(&rom).expect("rom");
        s.set_joystick_mode(JoystickMode::Cursor).unwrap();
        // Num5 = row 3 bit 4 (also Cursor left).
        s.set_key(3, 4, true).unwrap();
        s.set_joystick(0).unwrap();
        let rows = s.machine.as_mut().unwrap().keyboard_mut().rows;
        assert_eq!(rows[3] & (1 << 4), 0, "Num5 must stay pressed");
    }

    #[test]
    fn open_fixture_tap_progress_and_audio_pcm() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut s = HostSession::new(ModelId::Spectrum48, true);
        s.load_rom_bytes(&rom).expect("rom");
        let tap = workspace_root().join("tests/fixtures/tape/minimal_code.tap");
        s.open_tape(&tap).expect("tap");
        s.set_tape_load_options(machine::TapeLoadOptions {
            flash_load: false,
            speed: 1,
        })
        .expect("opts");
        let p0 = s.tape_progress().expect("progress");
        assert_eq!(p0.block_index, 0);
        assert_eq!(p0.block_count, 2);
        s.play_tape().expect("play");
        s.run_frame();
        let p1 = s.tape_progress().expect("progress after play");
        assert!(
            p1.pulse_index > p0.pulse_index || p1.fraction() > p0.fraction(),
            "tape progress should advance after a playing frame (before={p0:?} after={p1:?})"
        );
        assert_eq!(s.audio_pcm().len(), AUDIO_SAMPLES_PER_FRAME);
        let (min, max) = s
            .audio_pcm()
            .iter()
            .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), &x| {
                (lo.min(x), hi.max(x))
            });
        assert!(
            max - min > 0.05,
            "playing tape should produce a non-trivial PCM range, got min={min} max={max}"
        );
    }

    #[test]
    fn open_local_boggit_tzx_as_tap_when_present() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let boggit = PathBuf::from("/Users/michael/Downloads/BoggitThe/The Boggit - Side 1.tzx");
        if !boggit.is_file() {
            eprintln!("skip: local Boggit TZX not present");
            return;
        }
        let mut s = HostSession::new(ModelId::Spectrum48, true);
        s.load_rom_bytes(&rom).expect("rom");
        s.open_tape(&boggit).expect("boggit tzx");
        let p = s.tape_progress().expect("progress");
        assert!(p.block_count >= 2, "expected TAP conversion with blocks");
        assert!(s.status().contains("TAP") || s.status().contains("TZX"));
    }

    #[test]
    fn model_id_roundtrip() {
        assert_eq!(ModelId::from_u32(0), Some(ModelId::Spectrum48));
        assert_eq!(ModelId::from_u32(1), Some(ModelId::Spectrum128));
        assert_eq!(ModelId::from_u32(2), Some(ModelId::SpectrumPlus3));
        assert_eq!(ModelId::from_u32(3), Some(ModelId::SpectrumPlus2A));
        assert_eq!(ModelId::from_u32(9), None);
        assert_eq!(ModelId::Spectrum48.to_model(), Model::Spectrum48);
        assert_eq!(ModelId::SpectrumPlus2A.to_model(), Model::SpectrumPlus2A);
    }

    #[test]
    fn select_model_keeps_session_model_in_sync() {
        let mut s = HostSession::new(ModelId::Spectrum48, true);
        for model in [
            ModelId::Spectrum48,
            ModelId::Spectrum128,
            ModelId::SpectrumPlus3,
            ModelId::SpectrumPlus2A,
            ModelId::Spectrum48,
        ] {
            let result = s.select_model(model);
            assert_eq!(
                s.model(),
                model,
                "session.model() must match selection even if ROM load fails"
            );
            match result {
                Ok(()) => {
                    assert!(s.has_machine(), "autoload should install a machine");
                    assert_eq!(
                        s.machine.as_ref().expect("machine").model(),
                        model.to_model(),
                        "loaded Machine must match selected ModelId"
                    );
                }
                Err(_) => {
                    assert!(
                        !s.has_machine(),
                        "failed autoload must leave the machine unloaded"
                    );
                }
            }
        }
    }

    #[test]
    fn peek_poke_and_inspect_json() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut s = HostSession::new(ModelId::Spectrum48, false);
        s.load_rom_bytes(&rom).expect("rom");
        let pc0 = s.regs().expect("regs").pc;
        s.poke(0xC000, 0xA5).expect("poke");
        assert_eq!(s.peek(0xC000).expect("peek ram"), 0xA5);
        let json = s.inspect_json().expect("json");
        assert!(
            json.contains("\"pc\":"),
            "inspect json should include pc: {json}"
        );
        s.step().expect("step");
        assert_ne!(
            s.regs().expect("regs").pc,
            pc0,
            "step should advance PC from ROM"
        );
        s.add_breakpoint(0x1234).expect("break");
        s.set_paused(true);
        assert!(s.paused());
    }

    #[test]
    fn run_frame_skips_when_debugger_paused() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut s = HostSession::new(ModelId::Spectrum48, false);
        s.load_rom_bytes(&rom).expect("rom");
        s.run_frame();
        let t0 = s.machine.as_ref().expect("machine").cpu().t;
        s.set_paused(true);
        assert!(s.paused());
        s.run_frame();
        let t1 = s.machine.as_ref().expect("machine").cpu().t;
        assert_eq!(t0, t1, "paused debugger must not advance the machine");
        s.set_paused(false);
        s.run_frame();
        let t2 = s.machine.as_ref().expect("machine").cpu().t;
        assert!(t2 > t1, "unpaused run_frame should advance T-states");
    }

    #[test]
    fn peek_without_machine_errors() {
        let s = HostSession::new(ModelId::Spectrum48, true);
        assert!(matches!(s.peek(0), Err(HostError::NoMachine)));
        assert!(matches!(s.inspect_json(), Err(HostError::NoMachine)));
        assert!(matches!(s.regs(), Err(HostError::NoMachine)));
    }

    #[test]
    fn open_missing_tape_errors() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut s = HostSession::new(ModelId::Spectrum48, true);
        s.load_rom_bytes(&rom).expect("rom");
        let err = s
            .open_tape(Path::new("/tmp/spec_chum_definitely_missing.tap"))
            .expect_err("missing");
        assert!(matches!(err, HostError::Io(_) | HostError::Message(_)));
    }

    /// Minimal 48K SNA (49179 bytes) with PC=0x8000 via SP pop and one RAM marker.
    fn synthetic_sna48_bytes() -> Vec<u8> {
        let mut data = vec![0u8; 49179];
        data[26] = 5; // border
        data[23] = 0x00;
        data[24] = 0x40; // SP = 0x4000 → pop PC from RAM[0x4000]
        data[27] = 0x00;
        data[28] = 0x80; // PC = 0x8000
        data[27 + 0x4000] = 0xaa; // byte at 0x8000
        data
    }

    /// Minimal uncompressed Z80 v1 (48K).
    fn synthetic_z80_v1_bytes() -> Vec<u8> {
        let mut data = vec![0u8; 30 + 49152];
        data[0] = 0x11; // A
        data[1] = 0x22; // F
        data[6] = 0x00;
        data[7] = 0x81; // PC = 0x8100
        data[8] = 0x00;
        data[9] = 0x70; // SP = 0x7000
        data[12] = (6 << 1) & 0x0e; // border 6, uncompressed
        data[30] = 0xbe;
        data[31] = 0xef;
        data[30 + 0x1000] = 0x42;
        data
    }

    #[test]
    fn load_snapshot_sna48_sets_pc_and_ram() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let dir = std::env::temp_dir();
        let path = dir.join("spec_chum_host_api_test.sna");
        std::fs::write(&path, synthetic_sna48_bytes()).expect("write sna");
        let mut s = HostSession::new(ModelId::Spectrum48, false);
        s.load_rom_bytes(&rom).expect("rom");
        s.load_snapshot(&path).expect("sna");
        let regs = s.regs().expect("regs");
        assert_eq!(regs.pc, 0x8000);
        assert_eq!(s.peek(0x8000).expect("peek"), 0xaa);
        assert!(s.status().contains("snapshot"));
        let _ = std::fs::remove_file(&path);
    }

    fn load_128_or_plus3_rom(s: &mut HostSession, model: ModelId) -> bool {
        s.set_model(model);
        s.try_autoload_rom();
        s.has_machine()
    }

    #[test]
    fn load_snapshot48_switches_from_128k() {
        if rom48().is_none() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        let mut s = HostSession::new(ModelId::Spectrum128, false);
        if !load_128_or_plus3_rom(&mut s, ModelId::Spectrum128) {
            eprintln!("skip: 128K ROM missing");
            return;
        }
        assert_eq!(s.model(), ModelId::Spectrum128);
        s.set_joystick_mode(JoystickMode::Kempston).unwrap();
        s.set_joystick(0x11).unwrap();
        let path = std::env::temp_dir().join("spec_chum_host_api_128_to_48.sna");
        std::fs::write(&path, synthetic_sna48_bytes()).expect("write sna");
        s.load_snapshot(&path).expect("48k sna on 128");
        assert_eq!(s.model(), ModelId::Spectrum48);
        assert_eq!(
            s.machine.as_ref().map(machine::Machine::model),
            Some(Model::Spectrum48)
        );
        assert_eq!(s.regs().expect("regs").pc, 0x8000);
        assert_eq!(
            s.machine.as_mut().unwrap().kempston_mut().read(),
            0x11,
            "held joystick must survive model-switching snapshot load"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_snapshot48_switches_from_plus3() {
        if rom48().is_none() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        let mut s = HostSession::new(ModelId::SpectrumPlus3, false);
        if !load_128_or_plus3_rom(&mut s, ModelId::SpectrumPlus3) {
            eprintln!("skip: +3 ROM missing");
            return;
        }
        assert_eq!(s.model(), ModelId::SpectrumPlus3);
        let path = std::env::temp_dir().join("spec_chum_host_api_plus3_to_48.sna");
        std::fs::write(&path, synthetic_sna48_bytes()).expect("write sna");
        s.load_snapshot(&path).expect("48k sna on +3");
        assert_eq!(s.model(), ModelId::Spectrum48);
        assert_eq!(
            s.machine.as_ref().map(machine::Machine::model),
            Some(Model::Spectrum48)
        );
        assert_eq!(s.regs().expect("regs").pc, 0x8000);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_snapshot_z80_sets_pc_and_ram() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let dir = std::env::temp_dir();
        let path = dir.join("spec_chum_host_api_test.z80");
        std::fs::write(&path, synthetic_z80_v1_bytes()).expect("write z80");
        let mut s = HostSession::new(ModelId::Spectrum48, false);
        s.load_rom_bytes(&rom).expect("rom");
        s.load_snapshot(&path).expect("z80");
        let regs = s.regs().expect("regs");
        assert_eq!(regs.pc, 0x8100);
        assert_eq!(regs.sp, 0x7000);
        assert_eq!(s.peek(0x4000).expect("peek"), 0xbe);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn load_snapshot_without_machine_autoloads_48k_rom() {
        let fixture = workspace_root().join("tests/fixtures/snapshots/minimal48.sna");
        let (path, cleanup) = if fixture.is_file() {
            (fixture, false)
        } else {
            let path = std::env::temp_dir().join("spec_chum_host_api_autoload.sna");
            std::fs::write(&path, synthetic_sna48_bytes()).expect("write sna");
            (path, true)
        };
        let mut s = HostSession::new(ModelId::Spectrum48, false);
        match s.load_snapshot(&path) {
            Ok(()) => {
                assert!(s.has_machine());
                assert_eq!(s.regs().expect("regs").pc, 0x8000);
            }
            Err(HostError::NoMachine) => {
                eprintln!("skip: could not autoload ROM for snapshot");
            }
            Err(HostError::Message(msg))
                if msg.contains("ROM required") || msg.contains("fetch_roms") =>
            {
                eprintln!("skip: {msg}");
            }
            Err(e) => panic!("unexpected error: {e}"),
        }
        if cleanup {
            let _ = std::fs::remove_file(&path);
        }
    }

    #[test]
    fn load_rzx_and_dsk_require_machine() {
        let mut s = HostSession::new(ModelId::SpectrumPlus3, false);
        assert!(matches!(
            s.load_rzx(Path::new("/tmp/missing.rzx")),
            Err(HostError::NoMachine)
        ));
        assert!(matches!(
            s.load_dsk(Path::new("/tmp/missing.dsk")),
            Err(HostError::NoMachine)
        ));
        assert!(matches!(
            s.load_trd(Path::new("/tmp/missing.trd")),
            Err(HostError::NoMachine)
        ));
    }

    #[test]
    fn load_dsk_rejects_non_plus3() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut s = HostSession::new(ModelId::Spectrum48, false);
        s.load_rom_bytes(&rom).expect("rom");

        // Minimal parseable MV-CPC DSK (one empty track header) so load reaches insert_disk.
        let mut dsk = vec![0u8; 0x100];
        dsk[0..8].copy_from_slice(b"MV - CPC");
        dsk[0x30] = 1;
        dsk[0x31] = 1;
        let track_size: u16 = 0x100;
        dsk[0x32..0x34].copy_from_slice(&track_size.to_le_bytes());
        let mut track = vec![0u8; track_size as usize];
        track[0..12].copy_from_slice(b"Track-Info\r\n");
        dsk.extend_from_slice(&track);

        let dir = std::env::temp_dir().join("spec_chum_host_api_dsk_reject");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("reject.dsk");
        std::fs::write(&path, &dsk).expect("write dsk");

        let err = s.load_dsk(&path).expect_err("48K must reject DSK");
        match err {
            HostError::Message(msg) => {
                assert!(
                    msg.contains("+3") || msg.contains("Plus3") || msg.contains("plus3"),
                    "expected model-rejection message, got {msg}"
                );
            }
            other => panic!("expected Message rejection, got {other:?}"),
        }
    }

    #[test]
    fn load_trd_rejects_plus3() {
        let p = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../roms/plus3/plus3.rom");
        let Ok(rom) = std::fs::read(&p) else {
            eprintln!("skip: roms/plus3/plus3.rom missing");
            return;
        };
        let mut s = HostSession::new(ModelId::SpectrumPlus3, false);
        s.load_rom_bytes(&rom).expect("rom");

        let mut raw = vec![0u8; formats::TRD_SECTOR_SIZE * formats::TRD_SECTORS_PER_TRACK];
        raw[0xe3] = 0;
        let dir = std::env::temp_dir().join("spec_chum_host_api_trd_reject");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("reject.trd");
        std::fs::write(&path, &raw).expect("write trd");

        let err = s.load_trd(&path).expect_err("+3 must reject TRD");
        match err {
            HostError::Message(msg) => {
                assert!(
                    msg.contains("Beta") || msg.contains("+2A") || msg.contains("+3"),
                    "expected Beta rejection, got {msg}"
                );
            }
            other => panic!("expected Message rejection, got {other:?}"),
        }
    }

    #[test]
    fn render_frame_pcm_carries_late_edge_into_next_level() {
        let frame_tstates = 69_888u32;
        let audio = machine::FrameAudio {
            beeper_edges: vec![(frame_tstates - 1, true)],
            ay_samples: Vec::new(),
            ay_left: Vec::new(),
            ay_right: Vec::new(),
        };
        let mut out = Vec::new();
        let level = render_frame_pcm(&audio, frame_tstates, false, &mut out);
        assert!(
            level,
            "edge near end of frame must update returned speaker level"
        );
        assert_eq!(out.len(), AUDIO_SAMPLES_PER_FRAME);
    }
}
