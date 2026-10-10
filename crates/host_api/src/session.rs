//! Safe session wrapper around [`machine::Machine`] for host frontends.

mod media_title;
use media_title::PendingMediaTitle;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use formats::MediaTitleSource;
use machine::{JoystickMode, JoystickState, Machine, Model, NextMachine, Watch};
use parking_lot::Mutex;
use thiserror::Error;

pub const KEYBOARD_ROWS: usize = 8;
pub const KEYBOARD_BITS_PER_ROW: u8 = 5;
pub const KEYBOARD_BIT_MAX: u8 = KEYBOARD_BITS_PER_ROW - 1;
pub const KEYBOARD_KEY_RANGE_ERROR: &str = "key row/bit out of range";

/// Model identifiers for the C ABI (stable numeric values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ModelId {
    Spectrum48 = 0,
    Spectrum128 = 1,
    SpectrumPlus3 = 2,
    /// Amstrad +2A (no disk interface). Added after Plus3; keep numeric id stable.
    SpectrumPlus2A = 3,
    /// Amstrad grey +2 (#188). Added after +2A.
    SpectrumPlus2 = 4,
    /// 16 KiB RAM Spectrum (#188).
    Spectrum16K = 5,
    /// Pentagon 128 clone (#188 Phase B).
    Pentagon128 = 6,
    /// Timex TC2048 (#192 Phase 1).
    TimexTC2048 = 7,
    /// Timex TS2068 / TC2068 (#192 Phase 2a).
    TimexTS2068 = 8,
    /// Spectrum +3e enhanced firmware (#194).
    SpectrumPlus3e = 9,
    /// Scorpion ZS-256 clone (#193).
    ScorpionZs256 = 10,
    SpectrumNext = 11,
}

impl ModelId {
    #[must_use]
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::Spectrum48),
            1 => Some(Self::Spectrum128),
            2 => Some(Self::SpectrumPlus3),
            3 => Some(Self::SpectrumPlus2A),
            4 => Some(Self::SpectrumPlus2),
            5 => Some(Self::Spectrum16K),
            6 => Some(Self::Pentagon128),
            7 => Some(Self::TimexTC2048),
            8 => Some(Self::TimexTS2068),
            9 => Some(Self::SpectrumPlus3e),
            10 => Some(Self::ScorpionZs256),
            11 => Some(Self::SpectrumNext),
            _ => None,
        }
    }

    #[must_use]
    pub fn to_model(self) -> Model {
        match self {
            Self::Spectrum16K => Model::Spectrum16K,
            Self::Spectrum48 => Model::Spectrum48,
            Self::Spectrum128 => Model::Spectrum128,
            Self::SpectrumPlus2 => Model::SpectrumPlus2,
            Self::SpectrumPlus3 => Model::SpectrumPlus3,
            Self::SpectrumPlus3e => Model::SpectrumPlus3e,
            Self::SpectrumPlus2A => Model::SpectrumPlus2A,
            Self::Pentagon128 => Model::Pentagon128,
            Self::ScorpionZs256 => Model::ScorpionZs256,
            Self::TimexTC2048 => Model::TimexTC2048,
            Self::TimexTS2068 => Model::TimexTS2068,
            Self::SpectrumNext => Model::SpectrumNext,
        }
    }

    #[must_use]
    pub fn from_model(m: Model) -> Self {
        match m {
            Model::Spectrum16K => Self::Spectrum16K,
            Model::Spectrum48 => Self::Spectrum48,
            Model::Spectrum128 => Self::Spectrum128,
            Model::SpectrumPlus2 => Self::SpectrumPlus2,
            Model::SpectrumPlus3 => Self::SpectrumPlus3,
            Model::SpectrumPlus3e => Self::SpectrumPlus3e,
            Model::SpectrumPlus2A => Self::SpectrumPlus2A,
            Model::Pentagon128 => Self::Pentagon128,
            Model::ScorpionZs256 => Self::ScorpionZs256,
            Model::TimexTC2048 => Self::TimexTC2048,
            Model::TimexTS2068 => Self::TimexTS2068,
            Model::SpectrumNext => Self::SpectrumNext,
        }
    }

    /// All models in canonical UI order (matches [`machine::ALL_MODELS`]).
    pub const ALL: [Self; 12] = [
        Self::Spectrum16K,
        Self::Spectrum48,
        Self::Spectrum128,
        Self::SpectrumPlus2,
        Self::SpectrumPlus2A,
        Self::SpectrumPlus3,
        Self::SpectrumPlus3e,
        Self::Pentagon128,
        Self::ScorpionZs256,
        Self::TimexTC2048,
        Self::TimexTS2068,
        Self::SpectrumNext,
    ];

    /// Stable numeric ABI id. Do not derive this from [`Self::ALL`] order.
    #[must_use]
    pub const fn numeric_id(self) -> u32 {
        self as u32
    }

    #[must_use]
    pub fn rom_available(self) -> bool {
        crate::rom_setup::model_rom_available(self, &crate::rom_setup::model_rom_paths_snapshot())
    }

    #[must_use]
    pub fn unavailable_reason(self) -> &'static str {
        machine::unavailable_reason(self.to_model())
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
    #[error("{0} is unavailable on ZX Spectrum Next")]
    UnsupportedNext(&'static str),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Exactly one active hardware runtime belongs to a host session.
#[derive(Debug)]
pub(crate) enum HostRuntime {
    Classic(Box<Machine>),
    Next(Box<NextMachine>),
}

impl HostRuntime {
    fn classic(&self) -> Option<&Machine> {
        match self {
            Self::Classic(machine) => Some(machine),
            Self::Next(_) => None,
        }
    }

    fn classic_mut(&mut self) -> Option<&mut Machine> {
        match self {
            Self::Classic(machine) => Some(machine),
            Self::Next(_) => None,
        }
    }

    fn framebuffer_dims(&self, with_border: bool) -> (usize, usize) {
        match self {
            Self::Classic(machine) => machine.framebuffer_dims(with_border),
            Self::Next(_) => NextMachine::framebuffer_dims(with_border),
        }
    }

    fn render_rgba(&self, out: &mut [u8], with_border: bool) {
        match self {
            Self::Classic(machine) => machine.render_rgba(out, with_border),
            Self::Next(machine) => machine.render_rgba(out, with_border),
        }
    }
}

/// Core registers exposed through `sc_regs`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize)]
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

/// Optional register poke for agent TR-DOS / debug entry (`POST /v1/regs`, #261).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RegsPatch {
    pub pc: Option<u16>,
    pub sp: Option<u16>,
    pub af: Option<u16>,
}

impl RegsPatch {
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.pc.is_none() && self.sp.is_none() && self.af.is_none()
    }
}

/// Bounded PC-breakpoint history and the currently paused stop.
#[derive(Clone, Debug)]
pub struct PcBreakpointObservation {
    pub recent: Vec<machine::PcBreakpointHit>,
    pub active: Option<machine::PcBreakpointHit>,
    pub latest_id: u64,
}

/// Host-owned emulator session: machine + RGBA framebuffer + status.
///
/// Fields are private; in-process hosts (egui / `living_room`) use accessors
/// (`machine` / `machine_mut`, `framebuffer`, `set_status`, `set_border`, …).
#[derive(Debug)]
pub struct HostSession {
    machine: Option<HostRuntime>,
    model: ModelId,
    with_border: bool,
    framebuffer: Vec<u8>,
    width: usize,
    height: usize,
    running: bool,
    status: String,
    /// Display title for the last inserted tape (catalogue hit or filename).
    media_title: Option<String>,
    /// SHA-512 (lowercase hex) of the last inserted tape file, when known.
    media_sha512: Option<String>,
    /// Where [`Self::media_title`] came from (for debug / honesty).
    media_title_source: Option<MediaTitleSource>,
    /// Original path associated with the current tape identity, for Library metadata matching.
    media_path: Option<PathBuf>,
    /// Opt-in `ZXInfo` online title lookup (#373). Default off.
    online_tape_titles: bool,
    /// Bumped on each identity set/clear so stale background hits are ignored.
    media_title_generation: u64,
    /// Background enrichment inbox (cache / `ZXInfo` → title).
    pending_media_title: Arc<Mutex<Option<PendingMediaTitle>>>,
    /// Mono PCM for the last frame (~882 samples @ 44100 Hz / 50 fps).
    audio_pcm: Vec<f32>,
    /// Interleaved stereo PCM for the last frame (left, right per sample).
    audio_pcm_stereo: Vec<f32>,
    /// Mixed speaker level carried across frame boundaries (beeper edges reset each frame).
    last_speaker_level: bool,
    /// Host joystick presentation mode.
    joystick_mode: JoystickMode,
    /// Last applied host joystick mask state.
    joystick_state: JoystickState,
    /// Host-held Spectrum matrix keys so Sinclair/Cursor joystick clears do not drop them.
    host_keys: [[bool; 5]; 8],
}

fn require_machine(machine: Option<&HostRuntime>) -> Result<&Machine, HostError> {
    match machine {
        Some(HostRuntime::Classic(machine)) => Ok(machine),
        Some(HostRuntime::Next(_)) => Err(HostError::UnsupportedNext("classic machine operation")),
        None => Err(HostError::NoMachine),
    }
}

fn require_machine_mut(machine: &mut Option<HostRuntime>) -> Result<&mut Machine, HostError> {
    match machine.as_mut() {
        Some(HostRuntime::Classic(machine)) => Ok(machine),
        Some(HostRuntime::Next(_)) => Err(HostError::UnsupportedNext("classic machine operation")),
        None => Err(HostError::NoMachine),
    }
}

fn classic_frame_tstates(model: machine::Model) -> u32 {
    match model {
        machine::Model::Spectrum16K
        | machine::Model::Spectrum48
        | machine::Model::TimexTC2048
        | machine::Model::TimexTS2068 => 69_888,
        machine::Model::Spectrum128
        | machine::Model::SpectrumPlus2
        | machine::Model::SpectrumPlus2A
        | machine::Model::SpectrumPlus3
        | machine::Model::SpectrumPlus3e => 70_908,
        machine::Model::Pentagon128 | machine::Model::ScorpionZs256 => 71_680,
        machine::Model::SpectrumNext => 0,
    }
}

fn z80_flags_text(flags: u8) -> String {
    format!(
        "Flags: S={} Z={} 5={} H={} 3={} P/V={} N={} C={}",
        u8::from(flags & 0x80 != 0),
        u8::from(flags & 0x40 != 0),
        u8::from(flags & 0x20 != 0),
        u8::from(flags & 0x10 != 0),
        u8::from(flags & 0x08 != 0),
        u8::from(flags & 0x04 != 0),
        u8::from(flags & 0x02 != 0),
        u8::from(flags & 0x01 != 0),
    )
}

impl HostSession {
    /// Create an unloaded session for `model` with the selected border mode.
    ///
    /// ROM loading is a separate operation; this initializes status and buffers only.
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
            media_title: None,
            media_sha512: None,
            media_title_source: None,
            media_path: None,
            online_tape_titles: false,
            media_title_generation: 0,
            pending_media_title: Arc::new(Mutex::new(None)),
            audio_pcm: Vec::new(),
            audio_pcm_stereo: Vec::new(),
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

    /// Allow or suppress frame advancement. A stopped session keeps its machine and pixels.
    pub fn set_running(&mut self, running: bool) {
        self.running = running;
    }

    /// Change framebuffer dimensions and immediately redraw the current machine, if loaded.
    pub fn set_border(&mut self, with_border: bool) {
        if self.with_border == with_border {
            return;
        }
        self.with_border = with_border;
        self.sync_framebuffer_dims();
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

    /// Borrow the live machine (egui / in-process hosts).
    #[must_use]
    pub fn machine(&self) -> Option<&Machine> {
        self.machine.as_ref().and_then(HostRuntime::classic)
    }

    /// Mutable borrow of the live machine (egui / in-process hosts).
    pub fn machine_mut(&mut self) -> Option<&mut Machine> {
        self.machine.as_mut().and_then(HostRuntime::classic_mut)
    }

    /// Install a booted machine into this session.
    pub fn set_machine(&mut self, machine: Machine) {
        self.model = ModelId::from_model(machine.model());
        self.machine = Some(HostRuntime::Classic(Box::new(machine)));
        self.reapply_host_keys();
        self.last_speaker_level = false;
        if !self.has_tape() {
            self.clear_media_identity();
        }
    }

    /// Drop the live machine (model selection retained).
    pub fn clear_machine(&mut self) {
        self.machine = None;
        self.audio_pcm.clear();
        self.audio_pcm_stereo.clear();
        self.last_speaker_level = false;
        self.clear_media_identity();
    }

    /// Replace the host status string (UI / debug surfaces).
    pub fn set_status(&mut self, status: impl Into<String>) {
        self.status = status.into();
    }

    #[must_use]
    pub fn tape_playing(&self) -> bool {
        self.machine
            .as_ref()
            .and_then(HostRuntime::classic)
            .is_some_and(Machine::tape_playing)
    }

    #[must_use]
    pub fn has_tape(&self) -> bool {
        self.machine
            .as_ref()
            .and_then(HostRuntime::classic)
            .is_some_and(Machine::has_tape)
    }

    /// Tape progress for UI, if a deck is inserted.
    #[must_use]
    pub fn tape_progress(&self) -> Option<machine::TapeProgress> {
        self.machine
            .as_ref()
            .and_then(HostRuntime::classic)
            .and_then(Machine::tape_progress)
    }

    /// Mono PCM samples from the last [`Self::run_frame`] (empty if no machine).
    #[must_use]
    pub fn audio_pcm(&self) -> &[f32] {
        &self.audio_pcm
    }

    /// Interleaved stereo PCM samples from the last frame (empty if no frame ran).
    #[must_use]
    pub fn audio_pcm_stereo(&self) -> &[f32] {
        &self.audio_pcm_stereo
    }

    /// CPU T-states in the selected model's video frame.
    #[must_use]
    pub fn frame_tstates(&self) -> u32 {
        match self.machine.as_ref() {
            Some(HostRuntime::Next(machine)) => machine.frame_tstates(),
            Some(HostRuntime::Classic(machine)) => classic_frame_tstates(machine.model()),
            None => 0,
        }
    }

    /// Wall-clock period hosts should use for throttled frame advancement.
    /// Classic hosts retain their established 50 Hz pacing; Next follows the
    /// selected raster timing in 3.5 MHz CPU T-states.
    #[must_use]
    pub fn frame_period_seconds(&self) -> f64 {
        if self
            .machine
            .as_ref()
            .is_some_and(|machine| matches!(machine, HostRuntime::Next(_)))
        {
            f64::from(self.frame_tstates()) / 3_500_000.0
        } else {
            0.020
        }
    }

    /// Select a model and unload the current machine without attempting ROM discovery.
    /// Use [`Self::select_model`] to also try loading the model ROM.
    pub fn set_model(&mut self, model: ModelId) {
        self.model = model;
        self.machine = None;
        self.status = format!("Model set to {model:?}; load a ROM");
    }

    /// Switch the selected model and autoload its ROM from `roms/`.
    ///
    /// After this returns, [`Self::model`] always matches `model`. On success the
    /// loaded [`Machine`] matches too; on ROM miss the machine stays unloaded.
    pub fn select_model(&mut self, model: ModelId) -> Result<(), HostError> {
        self.set_model(model);
        if model == ModelId::SpectrumNext {
            return self.boot_next();
        }
        self.try_autoload_rom();
        if self.has_machine() {
            Ok(())
        } else {
            Err(HostError::Message(format!(
                "ROM for {model:?} not found; run ./scripts/fetch_roms.sh"
            )))
        }
    }

    /// Boot a built-in model before replacing the running machine.
    ///
    /// ROM discovery and Next asset preparation can fail. In that case the
    /// current model, machine, media, framebuffer and host settings remain live.
    pub fn activate_model(&mut self, model: ModelId) -> Result<(), HostError> {
        self.activate_model_with(model, |candidate| candidate.select_model(model))
    }

    fn activate_model_with(
        &mut self,
        model: ModelId,
        boot: impl FnOnce(&mut HostSession) -> Result<(), HostError>,
    ) -> Result<(), HostError> {
        let mut candidate = Self::new(model, self.with_border);
        boot(&mut candidate)?;
        let Some(machine) = candidate.machine.take() else {
            return Err(HostError::NoMachine);
        };
        self.model = model;
        self.machine = Some(machine);
        self.reapply_host_keys();
        self.audio_pcm.clear();
        self.audio_pcm_stereo.clear();
        self.last_speaker_level = false;
        self.clear_media_identity();
        self.status = candidate.status;
        self.refresh_framebuffer();
        Ok(())
    }

    #[cfg(test)]
    fn test_activate_model_transaction() {
        let mut session = Self::new(ModelId::Spectrum48, true);
        session.set_machine(Machine::new_48k(&vec![0; 16 * 1024]).expect("test ROM"));
        session.poke(0xc000, 0x5a).expect("writable RAM");
        session.set_status("running");
        session.set_running(false);
        let pixels = session.framebuffer().to_vec();

        let failure = session.activate_model_with(ModelId::Spectrum16K, |_| {
            Err(HostError::Message("ROM setup failed".into()))
        });
        assert!(failure.is_err());
        assert_eq!(session.model(), ModelId::Spectrum48);
        assert_eq!(
            session.peek(0xc000).expect("old machine remains live"),
            0x5a
        );
        assert_eq!(session.status(), "running");
        assert_eq!(session.framebuffer(), pixels);
        assert!(!session.running());

        session
            .activate_model_with(ModelId::Spectrum16K, |candidate| {
                candidate.set_machine(Machine::new_16k(&vec![0; 16 * 1024]).expect("test ROM"));
                candidate.set_status("new machine booted");
                Ok(())
            })
            .expect("activation");
        assert_eq!(session.model(), ModelId::Spectrum16K);
        assert_eq!(
            session.machine().expect("new machine").model(),
            Model::Spectrum16K
        );
        assert_eq!(session.status(), "new machine booted");
        assert!(!session.running());
    }

    /// Build and install the selected model using already-read ROM bytes.
    ///
    /// Returns a host error if the ROM or any required model-specific ROM is invalid or missing.
    pub fn load_rom_bytes(&mut self, rom: &[u8]) -> Result<(), HostError> {
        if self.model == ModelId::SpectrumNext {
            return Err(HostError::Message(
                "ZX Spectrum Next boots only from verified System/Next assets".into(),
            ));
        }
        let overrides = crate::rom_setup::slot_rom_overrides_for_model(self.model);
        self.load_rom_bytes_with_overrides(rom, &overrides)
    }

    /// Read ROM bytes from `path`, load them for the selected model, and update status.
    ///
    /// File I/O and machine-construction failures are returned as [`HostError`].
    pub fn load_rom_path(&mut self, path: &Path) -> Result<(), HostError> {
        if self.model == ModelId::SpectrumNext {
            return Err(HostError::UnsupportedNext("direct ROM loading"));
        }
        let data = std::fs::read(path)?;
        self.load_rom_bytes(&data)?;
        self.status = format!("Loaded {}", path.display());
        Ok(())
    }

    /// Boot from a saved user profile (#187), applying its model and joystick mode.
    ///
    /// The configuration's ROM paths are resolved before replacing the current machine;
    /// invalid configuration or unavailable ROMs are returned as `MachineConfigError`.
    pub fn apply_user_config(
        &mut self,
        config: &crate::machine_config::UserMachineConfig,
    ) -> Result<(), crate::machine_config::MachineConfigError> {
        let roots = rom_search_roots();
        let applied = crate::machine_config::apply_user_config(config, &roots)?;
        self.model = ModelId::from_model(applied.model);
        self.joystick_mode = applied.joystick_mode;
        self.machine = Some(HostRuntime::Classic(Box::new(applied.machine)));
        self.reapply_host_keys();
        self.last_speaker_level = false;
        self.status = applied.status;
        Ok(())
    }

    /// Reset the loaded machine while preserving host joystick and held-key state.
    ///
    /// An inserted tape remains present and paused. Returns [`HostError::NoMachine`] if unloaded.
    pub fn reset(&mut self) -> Result<(), HostError> {
        if let Some(HostRuntime::Next(next)) = self.machine.as_mut() {
            next.reset();
            self.reapply_host_keys();
            self.status = "Reset ZX Spectrum Next".into();
            return Ok(());
        }
        let m = require_machine_mut(&mut self.machine)?;
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

    /// Read and insert a TAP or TZX file, initially paused, and update its media identity.
    ///
    /// Requires a loaded machine. Unsupported extensions, file errors, and parse errors are
    /// returned without reporting a successful insertion; formats are selected by file extension.
    pub fn open_tape(&mut self, path: &Path) -> Result<(), HostError> {
        if self.machine.is_none() {
            return Err(HostError::NoMachine);
        }
        if matches!(self.machine, Some(HostRuntime::Next(_))) {
            return Err(HostError::UnsupportedNext("tape loading"));
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "tap" => {
                let data = std::fs::read(path)?;
                let img =
                    tape::TapImage::parse(&data).map_err(|e| HostError::Message(e.to_string()))?;
                if let Some(m) = self.machine.as_mut().and_then(HostRuntime::classic_mut) {
                    Self::clear_instant_tape_mode(m);
                    m.insert_tape(tape::TapPlayer::new(img));
                }
                self.set_media_identity_from_bytes(&data, path);
                let title = self
                    .media_title
                    .clone()
                    .unwrap_or_else(|| path.display().to_string());
                self.status = format!("Inserted TAP {title} (paused — Play when loader is ready)");
            }
            "tzx" => {
                let data = std::fs::read(path)?;
                if tape::TzxPlayer::is_standard_speed_only(&data) {
                    match tape::TzxPlayer::to_tap_player(&data) {
                        Ok(player) if player.image.blocks.is_empty() => {}
                        Ok(player) => {
                            let n = player.image.blocks.len();
                            if let Some(m) =
                                self.machine.as_mut().and_then(HostRuntime::classic_mut)
                            {
                                Self::clear_instant_tape_mode(m);
                                m.insert_tape(player);
                            }
                            self.set_media_identity_from_bytes(&data, path);
                            let title = self
                                .media_title
                                .clone()
                                .unwrap_or_else(|| path.display().to_string());
                            self.status =
                                format!("Inserted TZX {title} as TAP ({n} blocks, paused)");
                            return Ok(());
                        }
                        Err(e) => return Err(HostError::Message(e.to_string())),
                    }
                }
                let player =
                    tape::TzxPlayer::parse(&data).map_err(|e| HostError::Message(e.to_string()))?;
                if let Some(m) = self.machine.as_mut().and_then(HostRuntime::classic_mut) {
                    Self::clear_instant_tape_mode(m);
                    m.insert_tzx(player);
                }
                self.set_media_identity_from_bytes(&data, path);
                let title = self
                    .media_title
                    .clone()
                    .unwrap_or_else(|| path.display().to_string());
                self.status = format!("Inserted TZX {title} (paused)");
            }
            _ => {
                return Err(HostError::Message(format!(
                    "unsupported tape extension: {ext}"
                )));
            }
        }
        Ok(())
    }

    /// Instant is an ephemeral action for the currently inserted tape. A valid new tape
    /// starts on the user's selected EAR/Experience mode, preserving speed and preference.
    fn clear_instant_tape_mode(machine: &mut Machine) {
        let mut options = machine.tape_load_options();
        if options.flash_load {
            options.flash_load = false;
            machine.set_tape_load_options(options);
        }
    }

    /// Load a SNA/Z80 snapshot from `path` (128K/+3 first, then 48K), switching model when needed.
    ///
    /// Mirrors egui `SpecChumApp::load_snapshot`: selects the matching model for 128K/+3
    /// (including 128K↔Plus3) and for 48K snapshots loaded onto a non-48K machine; requires
    /// ROM autoload before apply.
    pub fn load_snapshot(&mut self, path: &Path) -> Result<(), HostError> {
        let data = std::fs::read(path)?;
        let model = self.apply_snapshot_bytes(&data, None)?;
        self.status = if model == ModelId::Spectrum48 {
            format!("Loaded snapshot {}", path.display())
        } else {
            format!("Loaded {model:?} snapshot {}", path.display())
        };
        Ok(())
    }

    fn apply_snapshot_bytes(
        &mut self,
        data: &[u8],
        extension: Option<&str>,
    ) -> Result<ModelId, HostError> {
        let snapshot128 = match extension {
            Some("SNA") => formats::Snapshot128::parse_sna(data),
            Some("Z80") => formats::Snapshot128::parse_z80(data),
            Some(other) => {
                return Err(HostError::Message(format!(
                    "unsupported snapshot format '{other}'"
                )));
            }
            None => formats::Snapshot128::parse_sna(data)
                .or_else(|_| formats::Snapshot128::parse_z80(data)),
        };
        if let Ok(snapshot) = snapshot128 {
            let required = match snapshot.model {
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
            let machine = require_machine_mut(&mut self.machine)?;
            machine.apply_snapshot128(&snapshot);
            machine.apply_joystick_state(self.joystick_mode, self.joystick_state);
            self.reapply_host_keys();
            return Ok(required);
        }

        let snapshot48 = match extension {
            Some("SNA") => formats::Snapshot48::parse_sna(data),
            Some("Z80") => formats::Snapshot48::parse_z80(data),
            Some(other) => {
                return Err(HostError::Message(format!(
                    "unsupported snapshot format '{other}'"
                )));
            }
            None => formats::Snapshot48::parse_sna(data)
                .or_else(|_| formats::Snapshot48::parse_z80(data)),
        }
        .map_err(|error| HostError::Message(error.to_string()))?;
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
        let machine = require_machine_mut(&mut self.machine)?;
        machine.apply_snapshot48(&snapshot48);
        machine.apply_joystick_state(self.joystick_mode, self.joystick_state);
        self.reapply_host_keys();
        Ok(ModelId::Spectrum48)
    }

    /// Read an RZX recording from `path`, apply an initial embedded snapshot when present, and
    /// attach its input frames to the resulting machine for replay.
    ///
    /// This path accepts a file path (unlike machine APIs that take parsed recordings).
    /// A recording without an embedded snapshot requires a loaded machine. I/O and format errors
    /// are returned as [`HostError`].
    pub fn load_rzx(&mut self, path: &Path) -> Result<(), HostError> {
        if self.machine.is_none() && !path.exists() {
            return Err(HostError::NoMachine);
        }
        let mut rec =
            formats::RzxRecording::load(path).map_err(|e| HostError::Message(e.to_string()))?;
        if let Some(snapshot) = rec.snapshot.take() {
            self.apply_snapshot_bytes(&snapshot.data, Some(&snapshot.extension))?;
        }
        require_machine_mut(&mut self.machine)?.insert_rzx(rec);
        self.status = format!("Loaded RZX {}", path.display());
        Ok(())
    }

    /// Read a DSK image from `path` and insert it into the loaded +3 machine.
    ///
    /// Missing-machine, I/O, unsupported-model, and format errors are returned as [`HostError`].
    pub fn load_dsk(&mut self, path: &Path) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        let img = formats::DskImage::load(path).map_err(|e| HostError::Message(e.to_string()))?;
        m.insert_disk(img)
            .map_err(|e| HostError::Message(e.to_string()))?;
        self.status = format!("Inserted DSK {}", path.display());
        Ok(())
    }

    /// Read a TRD image from `path` and insert it into the loaded machine's Beta Disk interface.
    ///
    /// Missing-machine, I/O, unsupported-model, and format errors are returned as [`HostError`].
    pub fn load_trd(&mut self, path: &Path) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        let img = formats::TrdImage::load(path).map_err(|e| HostError::Message(e.to_string()))?;
        m.insert_trd(img)
            .map_err(|e| HostError::Message(e.to_string()))?;
        self.status = format!("Inserted TRD {}", path.display());
        Ok(())
    }

    /// Load a 16 KiB TR-DOS ROM (attaches Beta on 48K/128K).
    pub fn load_trdos_rom(&mut self, path: &Path) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        let data = std::fs::read(path)?;
        m.load_trdos_rom(&data)
            .map_err(|e| HostError::Message(e.to_string()))?;
        self.status = format!("Loaded TR-DOS ROM {}", path.display());
        Ok(())
    }

    /// Attach Beta Disk / TR-DOS with no media (48K/128K).
    pub fn attach_beta(&mut self) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        m.attach_beta()
            .map_err(|e| HostError::Message(e.to_string()))?;
        self.status = "Beta Disk attached".into();
        Ok(())
    }

    #[must_use]
    pub fn has_beta(&self) -> bool {
        self.machine
            .as_ref()
            .and_then(HostRuntime::classic)
            .is_some_and(Machine::has_beta)
    }

    /// Best-effort ROM load for the current model (persisted paths, then workspace search).
    fn try_autoload_rom(&mut self) {
        if self.model == ModelId::SpectrumNext {
            if let Err(error) = self.boot_next() {
                self.status = error.to_string();
            }
            return;
        }
        let model = self.model.to_model();
        let roots = rom_search_roots();
        let slot_overrides = crate::rom_setup::slot_rom_overrides_for_model(self.model);
        if let Some(path) =
            machine::resolve_rom_path_in_with_overrides(model, &roots, &slot_overrides)
        {
            if let Ok(data) = std::fs::read(&path) {
                if self
                    .load_rom_bytes_with_overrides(&data, &slot_overrides)
                    .is_ok()
                {
                    self.status = format!("Loaded {}", path.display());
                }
            }
        }
    }

    fn boot_next(&mut self) -> Result<(), HostError> {
        let assets = crate::next_assets::NextAssets::discover()
            .map_err(|error| HostError::Message(error.to_string()))?;
        let card = assets
            .prepare_card()
            .map_err(|error| HostError::Message(error.to_string()))?;
        let mut next = NextMachine::new(&vec![0xff; 0x10000])
            .map_err(|error| HostError::Message(error.to_string()))?;
        next.install_ipl(&std::fs::read(&assets.ipl)?)
            .map_err(|error| HostError::Message(error.to_string()))?;
        next.attach_sd_image(card)
            .map_err(|error| HostError::Message(error.to_string()))?;
        self.machine = Some(HostRuntime::Next(Box::new(next)));
        self.reapply_host_keys();
        self.last_speaker_level = false;
        self.clear_media_identity();
        self.status = "Booting ZX Spectrum Next from verified System/Next 24.11".into();
        Ok(())
    }

    fn load_rom_bytes_with_overrides(
        &mut self,
        rom: &[u8],
        overrides: &std::collections::BTreeMap<String, PathBuf>,
    ) -> Result<(), HostError> {
        let machine = match self.model {
            ModelId::Spectrum16K => Machine::new_16k(rom),
            ModelId::Spectrum48 => Machine::new_48k(rom),
            ModelId::Spectrum128 => Machine::new_128k(rom),
            ModelId::SpectrumPlus2 => Machine::new_plus2(rom),
            ModelId::SpectrumPlus3 => Machine::new_plus3(rom),
            ModelId::SpectrumPlus3e => Machine::new_plus3e(rom),
            ModelId::SpectrumPlus2A => Machine::new_plus2a(rom),
            ModelId::ScorpionZs256 => {
                let trdos = machine::read_trdos_rom_with_overrides(Model::ScorpionZs256, overrides)
                    .map_err(|e| HostError::Message(e.to_string()))?;
                Machine::new_scorpion_zs256(rom, &trdos)
            }
            ModelId::Pentagon128 => {
                let trdos = machine::read_trdos_rom_with_overrides(Model::Pentagon128, overrides)
                    .map_err(|e| HostError::Message(e.to_string()))?;
                Machine::new_pentagon128(rom, &trdos)
            }
            ModelId::TimexTC2048 => Machine::new_timex_tc2048(rom),
            ModelId::TimexTS2068 => {
                let exrom = machine::read_exrom_with_overrides(Model::TimexTS2068, overrides)
                    .map_err(|e| HostError::Message(e.to_string()))?;
                Machine::new_timex_ts2068(rom, &exrom)
            }
            ModelId::SpectrumNext => {
                return Err(HostError::Message(
                    "ZX Spectrum Next requires the verified SD boot path".into(),
                ))
            }
        }
        .map_err(|e| HostError::Message(e.to_string()))?;
        self.machine = Some(HostRuntime::Classic(Box::new(machine)));
        self.reapply_host_keys();
        self.last_speaker_level = false;
        self.status = "ROM loaded".into();
        Ok(())
    }

    /// Start the inserted tape deck. Returns an error when no machine or tape is present.
    pub fn play_tape(&mut self) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        if !m.has_tape() {
            self.status = "No tape inserted".into();
            return Err(HostError::Message("no tape".into()));
        }
        m.set_tape_playing(true);
        self.status = "Tape playing".into();
        Ok(())
    }

    /// Pause the tape deck; requires a loaded machine but succeeds if no tape is inserted.
    pub fn pause_tape(&mut self) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        m.set_tape_playing(false);
        self.status = "Tape paused".into();
        Ok(())
    }

    /// Rewind the tape deck and leave it paused; requires a loaded machine.
    pub fn rewind_tape(&mut self) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        m.rewind_tape();
        self.status = "Tape rewound (paused)".into();
        Ok(())
    }

    /// Clear the inserted tape deck (no-op-ish success when empty).
    pub fn eject_tape(&mut self) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        m.eject_tape();
        self.clear_media_identity();
        self.status = "Tape ejected".into();
        Ok(())
    }

    #[must_use]
    pub fn tape_load_options(&self) -> Option<machine::TapeLoadOptions> {
        self.machine
            .as_ref()
            .and_then(HostRuntime::classic)
            .map(Machine::tape_load_options)
    }

    /// Set tape loading behavior and update status with the effective mode and speed.
    ///
    /// Requires a loaded machine; the options apply to the machine's current tape path.
    pub fn set_tape_load_options(
        &mut self,
        opts: machine::TapeLoadOptions,
    ) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        m.set_tape_load_options(opts);
        let effective = m.tape_load_options();
        let mode = if effective.flash_load {
            "instant"
        } else if effective.experience_load {
            "experience"
        } else {
            "EAR"
        };
        self.status = format!("Tape load: {mode}, speed {}x", effective.speed);
        Ok(())
    }

    /// Set one Spectrum matrix key (`row` 0..7, `bit` 0..4).
    pub fn set_key(&mut self, row: usize, bit: u8, pressed: bool) -> Result<(), HostError> {
        if row >= KEYBOARD_ROWS || bit > KEYBOARD_BIT_MAX {
            return Err(HostError::Message(KEYBOARD_KEY_RANGE_ERROR.into()));
        }
        if self.machine.is_none() {
            return Err(HostError::NoMachine);
        }
        self.host_keys[row][bit as usize] = pressed;
        self.reapply_host_keys();
        Ok(())
    }

    /// Release all host-held Spectrum matrix keys while preserving joystick input.
    pub fn clear_keys(&mut self) -> Result<(), HostError> {
        if self.machine.is_none() {
            return Err(HostError::NoMachine);
        }
        self.host_keys = [[false; 5]; 8];
        self.reapply_host_keys();
        Ok(())
    }

    /// Set joystick presentation mode. Recomposes input when a machine is loaded;
    /// otherwise stores the preference for the next boot (egui prefs / pre-ROM).
    pub fn set_joystick_mode(&mut self, mode: JoystickMode) {
        self.joystick_mode = mode;
        self.reapply_host_keys();
    }

    pub fn set_joystick(&mut self, mask: u8) -> Result<(), HostError> {
        if self.machine.is_none() {
            return Err(HostError::NoMachine);
        }
        self.joystick_state = JoystickState::from_mask(mask);
        self.reapply_host_keys();
        Ok(())
    }

    /// Clear host joystick buttons and recompose the effective keyboard input.
    pub fn clear_joystick(&mut self) -> Result<(), HostError> {
        if self.machine.is_none() {
            return Err(HostError::NoMachine);
        }
        self.joystick_state = JoystickState::empty();
        self.reapply_host_keys();
        Ok(())
    }

    /// Accumulate host pointer motion into the Kempston mouse (positive `dy` = down).
    pub fn set_mouse_delta(&mut self, dx: i8, dy: i8) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
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
        let m = require_machine_mut(&mut self.machine)?;
        m.mouse_mut().set_buttons(left, right, middle);
        Ok(())
    }

    /// Reset Kempston mouse axes and buttons to defaults.
    pub fn clear_mouse(&mut self) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        m.mouse_mut().reset();
        Ok(())
    }

    fn reapply_host_keys(&mut self) {
        let Some(runtime) = self.machine.as_mut() else {
            return;
        };
        match runtime {
            HostRuntime::Classic(machine) => Self::recompose_input(
                machine,
                self.joystick_mode,
                self.joystick_state,
                &self.host_keys,
            ),
            HostRuntime::Next(machine) => {
                machine.apply_joystick_state(self.joystick_mode, self.joystick_state);
                for (row, bits) in self.host_keys.iter().enumerate() {
                    for (bit, pressed) in bits.iter().enumerate() {
                        if *pressed {
                            machine.bus.keyboard.set_key(row, bit as u8, true);
                        }
                    }
                }
            }
        }
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

    /// Attach Multiface 1 (48K-class) or Multiface 128 (128K/+2) from an 8 KiB ROM path.
    pub fn attach_multiface(&mut self, path: &Path) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        let data = std::fs::read(path)?;
        m.attach_multiface(&data)
            .map_err(|e| HostError::Message(e.to_string()))?;
        let label = match m.model() {
            machine::Model::Spectrum128
            | machine::Model::SpectrumPlus2
            | machine::Model::Pentagon128
            | machine::Model::ScorpionZs256 => "Multiface 128",
            _ => "Multiface 1",
        };
        self.status = format!("Attached {label} from {}", path.display());
        Ok(())
    }

    /// Raise Multiface NMI if attached.
    pub fn multiface_nmi(&mut self) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        match m.multiface_nmi() {
            Some(_) => {
                self.status = "Multiface NMI".into();
                Ok(())
            }
            None => Err(HostError::Message(
                "Multiface not attached (48K → MF1, 128K/+2 → MF128; 8 KiB ROM)".into(),
            )),
        }
    }

    #[must_use]
    pub fn has_multiface(&self) -> bool {
        self.machine
            .as_ref()
            .and_then(HostRuntime::classic)
            .is_some_and(Machine::has_multiface)
    }

    /// Attach Interface 1 on 48K/128K (optionally load `roms/if1.rom` if present).
    pub fn attach_interface1(&mut self) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        let if1 = m
            .attach_interface1()
            .map_err(|e| HostError::Message(e.to_string()))?;
        if !if1.rom_loaded {
            for cand in ["roms/if1.rom", "roms/if1-2.rom", "roms/interface1.rom"] {
                for root in rom_search_roots() {
                    let path = root.join(cand);
                    if path.is_file() {
                        let data = std::fs::read(&path)?;
                        if1.load_rom(&data)
                            .map_err(|e| HostError::Message(e.to_string()))?;
                        break;
                    }
                }
                if if1.rom_loaded {
                    break;
                }
            }
        }
        self.status = if if1.rom_loaded {
            "Interface 1 attached (ROM loaded)".into()
        } else {
            "Interface 1 attached (no IF1 ROM on disk)".into()
        };
        Ok(())
    }

    /// Load an 8 KiB Interface 1 ROM image from `path`.
    pub fn load_interface1_rom(&mut self, path: &Path) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        let data = std::fs::read(path)?;
        m.load_interface1_rom(&data)
            .map_err(|e| HostError::Message(e.to_string()))?;
        self.status = format!("Loaded IF1 ROM {}", path.display());
        Ok(())
    }

    /// Insert a Microdrive `.mdr` cartridge (attaches IF1 if needed).
    pub fn insert_mdr(&mut self, path: &Path) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        let data = std::fs::read(path)?;
        let cart =
            formats::MdrImage::parse(&data).map_err(|e| HostError::Message(e.to_string()))?;
        let if1 = m
            .attach_interface1()
            .map_err(|e| HostError::Message(e.to_string()))?;
        if1.insert_mdr(cart);
        self.status = format!("Inserted MDR {}", path.display());
        Ok(())
    }

    /// Insert a Timex `.dck` dock cartridge (TS2068 only). Soft-resets the machine.
    pub fn insert_dck(&mut self, path: &Path) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        let data = std::fs::read(path)?;
        let image =
            formats::DckImage::parse(&data).map_err(|e| HostError::Message(e.to_string()))?;
        m.insert_timex_dock(&image)
            .map_err(|e| HostError::Message(e.to_string()))?;
        self.status = format!("Inserted DCK {}", path.display());
        Ok(())
    }

    /// Eject Timex dock cartridge (TS2068 only). Soft-resets the machine.
    pub fn eject_dck(&mut self) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        m.eject_timex_dock()
            .map_err(|e| HostError::Message(e.to_string()))?;
        self.status = "Ejected Timex dock".into();
        Ok(())
    }

    #[must_use]
    pub fn has_timex_dock(&self) -> bool {
        self.machine
            .as_ref()
            .and_then(HostRuntime::classic)
            .is_some_and(Machine::has_timex_dock)
    }

    #[must_use]
    pub fn has_interface1(&self) -> bool {
        self.machine
            .as_ref()
            .and_then(HostRuntime::classic)
            .is_some_and(Machine::has_interface1)
    }

    /// Attach `DivMMC` on 48K/128K (no media).
    pub fn attach_divmmc(&mut self) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        m.attach_divmmc()
            .map_err(|e| HostError::Message(e.to_string()))?;
        self.status = "DivMMC attached".into();
        Ok(())
    }

    /// Attach `DivMMC` and load a flat SD/MMC image into slot 0.
    pub fn load_divmmc_sd(&mut self, path: &Path) -> Result<(), HostError> {
        self.load_divmmc_sd_slot(path, 0)
    }

    /// Attach `DivMMC` and load a flat SD/MMC image into slot `0` or `1`.
    pub fn load_divmmc_sd_slot(&mut self, path: &Path, slot: u8) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        let data = std::fs::read(path)?;
        m.attach_divmmc_sd_slot(slot, data)
            .map_err(|e| HostError::Message(e.to_string()))?;
        self.status = format!("DivMMC SD slot {slot} {}", path.display());
        Ok(())
    }

    /// Attach `DivMMC` and load an ESXDOS EEPROM (8 KiB or larger prefix).
    pub fn load_divmmc_eeprom(&mut self, path: &Path) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        let data = std::fs::read(path)?;
        m.attach_divmmc_eeprom(&data)
            .map_err(|e| HostError::Message(e.to_string()))?;
        self.status = format!("DivMMC EEPROM {}", path.display());
        Ok(())
    }

    #[must_use]
    pub fn has_divmmc(&self) -> bool {
        self.machine
            .as_ref()
            .and_then(HostRuntime::classic)
            .is_some_and(Machine::has_divmmc)
    }

    #[must_use]
    pub fn joystick_mode(&self) -> JoystickMode {
        self.joystick_mode
    }

    /// Peek one byte of machine memory.
    pub fn peek(&self, addr: u16) -> Result<u8, HostError> {
        let m = require_machine(self.machine.as_ref())?;
        Ok(m.read_mem(addr))
    }

    /// Poke one byte of machine memory.
    pub fn poke(&mut self, addr: u16, value: u8) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        m.write_mem(addr, value);
        Ok(())
    }

    /// JSON inspect snapshot (`Inspect::to_json`).
    pub fn inspect_json(&self) -> Result<String, HostError> {
        let m = require_machine(self.machine.as_ref())?;
        Ok(m.inspect().to_json())
    }

    /// Human-readable inspect snapshot (`Inspect` display).
    pub fn inspect_text(&self) -> Result<String, HostError> {
        let m = require_machine(self.machine.as_ref())?;
        Ok(format!("{}", m.inspect()))
    }

    /// Core registers for the C `sc_regs` ABI.
    pub fn regs(&self) -> Result<HostRegs, HostError> {
        let m = require_machine(self.machine.as_ref())?;
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

    /// Human-readable Z80 flag bits for debugger workspaces.
    pub fn debugger_flags_text(&self) -> Result<String, HostError> {
        Ok(z80_flags_text(self.regs()?.af as u8))
    }

    /// Patch PC / SP / AF for headless entry (e.g. TR-DOS `USR 15616` → `PC=0x3D00`).
    pub fn patch_regs(&mut self, patch: RegsPatch) -> Result<HostRegs, HostError> {
        if patch.is_empty() {
            return Err(HostError::Message(
                "regs patch requires at least one of pc, sp, af".into(),
            ));
        }
        let m = require_machine_mut(&mut self.machine)?;
        let r = &mut m.cpu_mut().regs;
        if let Some(pc) = patch.pc {
            r.pc = pc;
        }
        if let Some(sp) = patch.sp {
            r.sp = sp;
        }
        if let Some(af) = patch.af {
            r.set_af(af);
        }
        let out = HostRegs {
            pc: r.pc,
            sp: r.sp,
            af: r.af(),
            bc: r.bc(),
            de: r.de(),
            hl: r.hl(),
            ix: r.ix(),
            iy: r.iy(),
        };
        Ok(out)
    }

    /// One CPU/machine instruction (`step_once`).
    pub fn step(&mut self) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        m.step_once();
        Ok(())
    }

    /// Debugger UI step: clear a PC re-hit, execute one instruction, leave paused, refresh pixels.
    pub fn debug_step(&mut self) -> Result<(), HostError> {
        {
            let m = require_machine_mut(&mut self.machine)?;
            let pc = m.cpu().regs.pc;
            if m.debugger().paused {
                m.debugger_mut().continue_from_pc(pc);
            }
            m.debugger_mut().paused = false;
            m.step_once();
            m.debugger_mut().paused = true;
        }
        self.refresh_framebuffer();
        Ok(())
    }

    /// Set the debugger paused flag (no-op without a machine).
    pub fn set_paused(&mut self, paused: bool) {
        match self.machine.as_mut() {
            Some(HostRuntime::Classic(machine)) => machine.debugger_mut().paused = paused,
            Some(HostRuntime::Next(machine)) => machine.set_paused(paused),
            None => {}
        }
    }

    pub fn debug_pause(&mut self) {
        self.set_paused(true);
        self.set_status("Paused");
    }

    #[must_use]
    pub fn paused(&self) -> bool {
        match self.machine.as_ref() {
            Some(HostRuntime::Classic(machine)) => machine.debugger().paused,
            Some(HostRuntime::Next(machine)) => machine.paused(),
            None => false,
        }
    }

    /// Resume after a debugger stop, allowing the breakpoint at the current PC to be passed once.
    ///
    /// This clears the debugger pause state but does not change the host `running` flag.
    pub fn continue_execution(&mut self) -> Result<(), HostError> {
        match self.machine.as_mut() {
            Some(HostRuntime::Classic(machine)) => {
                let pc = machine.cpu().regs.pc;
                machine.debugger_mut().continue_from_pc(pc);
            }
            Some(HostRuntime::Next(machine)) => machine.set_paused(false),
            None => return Err(HostError::NoMachine),
        }
        Ok(())
    }

    /// Resume debugger execution and enable host frame advancement.
    pub fn debug_continue(&mut self) -> Result<(), HostError> {
        self.continue_execution()?;
        self.set_running(true);
        Ok(())
    }

    /// Add a PC breakpoint.
    pub fn add_breakpoint(&mut self, pc: u16) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        m.debugger_mut().add_pc_break(pc);
        Ok(())
    }

    /// Add a memory access watch (read and/or write).
    pub fn add_mem_watch(&mut self, watch: Watch) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        m.debugger_mut().add_mem_watch(watch);
        Ok(())
    }

    /// Add an I/O port access watch (read and/or write).
    pub fn add_port_watch(&mut self, watch: Watch) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        m.debugger_mut().add_port_watch(watch);
        Ok(())
    }

    /// Remove a memory watch at `addr`. Returns `false` when none matched.
    pub fn remove_mem_watch(&mut self, addr: u16) -> Result<bool, HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        Ok(m.debugger_mut().remove_mem_watch(addr))
    }

    /// Remove a port watch at `addr`. Returns `false` when none matched.
    pub fn remove_port_watch(&mut self, addr: u16) -> Result<bool, HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        Ok(m.debugger_mut().remove_port_watch(addr))
    }

    /// Execute up to `max_insns` instructions, stopping on a breakpoint, halt, or exhausted budget.
    ///
    /// Requires a loaded machine; the returned reason also reports an exhausted budget.
    pub fn run_until_break(&mut self, max_insns: u32) -> Result<machine::BreakReason, HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        Ok(m.run_until_break(u64::from(max_insns)))
    }

    /// Last debugger stop reason (`BreakReason::None` when nothing has stopped yet).
    pub fn last_break_reason(&self) -> Result<machine::BreakReason, HostError> {
        let m = require_machine(self.machine.as_ref())?;
        Ok(m.debugger().last_hit)
    }

    /// Recent PC stops and the current paused stop, read from one machine state.
    pub fn pc_breakpoint_observation(
        &self,
        since: u64,
    ) -> Result<PcBreakpointObservation, HostError> {
        let Some(m) = self.machine.as_ref().and_then(HostRuntime::classic) else {
            return Err(HostError::NoMachine);
        };
        let debugger = m.debugger();
        Ok(PcBreakpointObservation {
            recent: debugger.pc_hits_since(since),
            active: debugger.current_pc_hit(),
            latest_id: debugger.latest_pc_hit_id(),
        })
    }

    /// Run one video frame into the RGBA framebuffer when `running`.
    ///
    /// Skips CPU/ULA advance while the debugger is paused, but still refreshes
    /// pixels so hosts can show the current screen. Returns the raw
    /// [`machine::FrameAudio`] (empty when not advancing) for hosts that mix
    /// beeper edges themselves (egui); mono PCM is always updated in
    /// [`Self::audio_pcm`] when a frame advances.
    pub fn run_frame(&mut self) -> machine::FrameAudio {
        self.apply_pending_media_title();
        if !self.running || self.machine.is_none() {
            return machine::FrameAudio::default();
        }
        if let Some(HostRuntime::Next(next)) = self.machine.as_mut() {
            let audio = next.run_frame();
            self.last_speaker_level = render_frame_pcm_with_count(
                &audio,
                next.frame_tstates(),
                self.last_speaker_level,
                audio.ay_samples.len(),
                &mut self.audio_pcm,
            );
            render_frame_stereo_pcm(&audio, &self.audio_pcm, &mut self.audio_pcm_stereo);
            self.sync_framebuffer_dims();
            self.refresh_framebuffer();
            return audio;
        }
        if self
            .machine
            .as_ref()
            .and_then(HostRuntime::classic)
            .is_some_and(|m| m.debugger().paused)
        {
            self.refresh_framebuffer();
            return machine::FrameAudio::default();
        }
        let (audio, frame_t, w, h) = {
            let m = self
                .machine
                .as_mut()
                .and_then(HostRuntime::classic_mut)
                .expect("machine checked above");
            let audio = m.run_frame();
            let frame_t = classic_frame_tstates(m.model());
            let (w, h) = m.framebuffer_dims(self.with_border);
            (audio, frame_t, w, h)
        };
        self.last_speaker_level = render_frame_pcm(
            &audio,
            frame_t,
            self.last_speaker_level,
            &mut self.audio_pcm,
        );
        render_frame_stereo_pcm(&audio, &self.audio_pcm, &mut self.audio_pcm_stereo);
        if self.width != w || self.height != h || self.framebuffer.len() != w * h * 4 {
            self.width = w;
            self.height = h;
            self.framebuffer.resize(w * h * 4, 0);
        }
        if let Some(m) = self.machine.as_ref() {
            m.render_rgba(&mut self.framebuffer, self.with_border);
        }
        audio
    }

    /// Re-render the RGBA framebuffer from the current machine (e.g. after step).
    pub fn refresh_framebuffer(&mut self) {
        self.sync_framebuffer_dims();
        if let Some(m) = self.machine.as_ref() {
            m.render_rgba(&mut self.framebuffer, self.with_border);
        }
    }

    /// Grow/shrink the RGBA buffer when Timex SCLD switches between 256 and 512.
    fn sync_framebuffer_dims(&mut self) {
        let (w, h) = if let Some(m) = self.machine.as_ref() {
            m.framebuffer_dims(self.with_border)
        } else {
            dims(self.with_border)
        };
        if self.width == w && self.height == h && self.framebuffer.len() == w * h * 4 {
            return;
        }
        self.width = w;
        self.height = h;
        self.framebuffer.resize(w * h * 4, 0);
    }

    /// Disassemble `count` instructions at `addr` (defaults to current PC).
    pub fn disasm(&self, addr: Option<u16>, count: usize) -> Result<String, HostError> {
        let m = require_machine(self.machine.as_ref())?;
        let addr = addr.unwrap_or_else(|| m.cpu().regs.pc);
        Ok(m.disasm_window(addr, count))
    }

    /// Combined debugger workspace snapshot for native windows.
    ///
    /// Keep this read-only and bounded: native shells refresh it on a timer so
    /// debugger UI work never runs in the guest frame or audio path.
    pub fn debugger_text(&self) -> String {
        let pc = self.regs().map_or(0, |regs| regs.pc);
        self.debugger_text_at(pc)
    }

    /// Combined debugger workspace snapshot focused on a selected memory address.
    pub fn debugger_text_at(&self, address: u16) -> String {
        let inspect = self
            .inspect_text()
            .unwrap_or_else(|e| format!("inspect: {e}"));
        let flags = self
            .debugger_flags_text()
            .unwrap_or_else(|e| format!("flags: {e}"));
        let disasm = self.disasm(None, 16).unwrap_or_default();
        let memory = self.hexdump(address, 64).unwrap_or_default();
        let breaks = self
            .list_pc_breakpoints()
            .unwrap_or_default()
            .into_iter()
            .map(|pc| format!("${pc:04X}"))
            .collect::<Vec<_>>()
            .join(" ");
        let paused = if !self.has_machine() {
            "no machine loaded"
        } else if self.paused() {
            "paused"
        } else {
            "running"
        };
        let trace_count = trace::len();
        let trace_events = trace::snapshot_recent(16);
        let trace = trace_events
            .into_iter()
            .rev()
            .take(16)
            .map(|event| event.to_string())
            .collect::<Vec<_>>()
            .join("\n");
        let trace = if trace.is_empty() {
            "(no trace events; enable tracing from the Debug menu)".to_owned()
        } else {
            trace
        };
        format!(
            "Execution: {paused}\n\n{inspect}\n{flags}\n\n--- disassembly at PC ---\n{disasm}\n\n--- memory at ${address:04X} ---\n{memory}\n\nPC breakpoints: {breaks}\n\n--- recent trace ({trace_count} buffered, latest 16) ---\n{trace}\n"
        )
    }

    /// Enable the shared low-volume trace categories exposed by native Debug menus.
    pub fn debug_enable_default_trace(&mut self) {
        trace::enable(trace::Category::DEFAULT);
    }

    /// Clear buffered trace events without changing the enabled categories.
    pub fn debug_clear_trace(&mut self) {
        trace::clear();
    }

    /// Hexdump `len` bytes starting at `addr`.
    pub fn hexdump(&self, addr: u16, len: u16) -> Result<String, HostError> {
        let m = require_machine(self.machine.as_ref())?;
        Ok(m.hexdump(addr, len))
    }

    /// Advance up to `frames` video frames, respecting the running flag and stopping when paused.
    ///
    /// Requires a loaded machine. Returns the last debugger break reason observed.
    pub fn run_frames(&mut self, frames: u32) -> Result<machine::BreakReason, HostError> {
        if self.machine.is_none() {
            return Err(HostError::NoMachine);
        }
        let mut last = machine::BreakReason::None;
        for _ in 0..frames {
            if self.paused() {
                last = self
                    .machine
                    .as_ref()
                    .and_then(HostRuntime::classic)
                    .map_or(machine::BreakReason::None, |m| m.debugger().last_hit);
                break;
            }
            self.run_frame();
            if self.paused() {
                last = self
                    .machine
                    .as_ref()
                    .and_then(HostRuntime::classic)
                    .map_or(machine::BreakReason::None, |m| m.debugger().last_hit);
                break;
            }
        }
        Ok(last)
    }

    /// Type `LOAD ""` (and optionally `CODE`) from an inserted tape, then run until the load marker or frame budget.
    ///
    /// Requires an inserted tape. `warmup` frames run before typing; `max == 0` selects a
    /// default budget based on the active tape-load mode. The result reports whether the
    /// expected BASIC/code marker appeared; machine absence and missing tape are errors.
    pub fn type_load(
        &mut self,
        with_code: bool,
        warmup: u32,
        max: u32,
    ) -> Result<TypeLoadResult, HostError> {
        {
            let m = require_machine_mut(&mut self.machine)?;
            if !m.has_tape() {
                return Err(HostError::Message("no tape inserted".into()));
            }
            m.set_tape_playing(false);
        }
        for _ in 0..warmup {
            self.run_frame();
        }
        {
            let m = require_machine_mut(&mut self.machine)?;
            m.type_load_quotes(with_code);
            m.set_tape_playing(true);
        }
        let ear = self
            .machine
            .as_ref()
            .and_then(HostRuntime::classic)
            .is_some_and(|m| !m.tape_load_options().flash_load);
        let limit = if max > 0 {
            max
        } else if ear {
            200_000
        } else {
            200
        };
        let mut loaded = false;
        for _ in 0..limit {
            self.run_frame();
            if let Some(m) = self.machine.as_ref().and_then(HostRuntime::classic) {
                loaded = if with_code {
                    Self::attr_mark_code_loaded(m)
                } else {
                    Self::print_ok_loaded(m)
                };
                if loaded {
                    break;
                }
            }
        }
        let m = self
            .machine
            .as_ref()
            .and_then(HostRuntime::classic)
            .expect("machine");
        Ok(TypeLoadResult {
            load_ok: loaded,
            attr_mark: if with_code {
                Some(m.read_mem(0x5800))
            } else {
                None
            },
        })
    }

    pub fn clear_breakpoints(&mut self) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        m.debugger_mut().clear_breaks();
        Ok(())
    }

    pub fn list_pc_breakpoints(&self) -> Result<Vec<u16>, HostError> {
        let m = require_machine(self.machine.as_ref())?;
        Ok(m.debugger().pc_breaks.clone())
    }

    pub fn list_watches(&self) -> Result<(Vec<Watch>, Vec<Watch>), HostError> {
        let m = require_machine(self.machine.as_ref())?;
        let dbg = m.debugger();
        Ok((dbg.mem_watches.clone(), dbg.port_watches.clone()))
    }

    pub fn remove_breakpoint(&mut self, pc: u16) -> Result<(), HostError> {
        let m = require_machine_mut(&mut self.machine)?;
        m.debugger_mut().remove_pc_break(pc);
        Ok(())
    }

    #[must_use]
    pub fn framebuffer_hires(&self) -> bool {
        self.machine
            .as_ref()
            .and_then(HostRuntime::classic)
            .is_some_and(machine::Machine::framebuffer_hires)
    }

    #[must_use]
    pub fn timex_scld_mode(&self) -> Option<u8> {
        self.machine
            .as_ref()
            .and_then(HostRuntime::classic)
            .and_then(machine::Machine::timex_scld_mode)
    }

    fn attr_mark_code_loaded(m: &Machine) -> bool {
        m.read_mem(0x8000) == 0x21
            && m.read_mem(0x8001) == 0x00
            && m.read_mem(0x8002) == 0x58
            && m.read_mem(0x8003) == 0x36
            && m.read_mem(0x8004) == 0xd7
            && m.read_mem(0x8005) == 0xc9
    }

    fn print_ok_loaded(m: &Machine) -> bool {
        let prog = u16::from_le_bytes([m.read_mem(0x5C53), m.read_mem(0x5C54)]);
        let eline = u16::from_le_bytes([m.read_mem(0x5C59), m.read_mem(0x5C5A)]);
        for a in prog..eline {
            if m.read_mem(a) == b'O' && m.read_mem(a.wrapping_add(1)) == b'K' {
                return true;
            }
        }
        false
    }
}

/// Result of a scripted type-load.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TypeLoadResult {
    pub load_ok: bool,
    pub attr_mark: Option<u8>,
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
    render_frame_pcm_with_count(
        audio,
        frame_tstates,
        initial_level,
        AUDIO_SAMPLES_PER_FRAME,
        out,
    )
}

fn render_frame_pcm_with_count(
    audio: &machine::FrameAudio,
    frame_tstates: u32,
    initial_level: bool,
    sample_count: usize,
    out: &mut Vec<f32>,
) -> bool {
    out.clear();
    out.resize(sample_count, 0.0);
    let t_per = frame_tstates as f32 / sample_count as f32;
    let mut edge_i = 0usize;
    let mut level = initial_level;
    let mut t = 0.0f32;
    let mut ay_i = 0usize;
    for (sample_index, sample) in out.iter_mut().enumerate() {
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
        let (dac_left, dac_right) = audio.dac_stereo_sample(sample_index);
        let dac = f32::midpoint(dac_left, dac_right);
        *sample = (beep + ay + dac).clamp(-1.0, 1.0);
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
    for (index, sample) in out.iter_mut().enumerate() {
        if audio
            .audio_muted_samples
            .get(index)
            .copied()
            .unwrap_or(false)
        {
            *sample = 0.0;
        }
    }
    level
}

fn render_frame_stereo_pcm(audio: &machine::FrameAudio, mono: &[f32], out: &mut Vec<f32>) {
    out.clear();
    out.reserve(mono.len() * 2);
    for (index, &sample) in mono.iter().enumerate() {
        if audio
            .audio_muted_samples
            .get(index)
            .copied()
            .unwrap_or(false)
        {
            out.extend_from_slice(&[0.0, 0.0]);
            continue;
        }
        let mono_ay = audio
            .ay_samples
            .get(index)
            .or_else(|| audio.ay_samples.last())
            .map_or(0.0, |value| (value - 0.5) * 0.5);
        let left_ay = audio
            .ay_left
            .get(index)
            .or_else(|| audio.ay_left.last())
            .map_or(mono_ay, |value| (value - 0.5) * 0.5);
        let right_ay = audio
            .ay_right
            .get(index)
            .or_else(|| audio.ay_right.last())
            .map_or(mono_ay, |value| (value - 0.5) * 0.5);
        let (dac_left, dac_right) = audio.dac_stereo_sample(index);
        let dac = [dac_left, dac_right];
        let mono_dac = f32::midpoint(dac[0], dac[1]);
        out.extend_from_slice(&[
            (sample - mono_ay - mono_dac + left_ay + dac[0]).clamp(-1.0, 1.0),
            (sample - mono_ay - mono_dac + right_ay + dac[1]).clamp(-1.0, 1.0),
        ]);
    }
}

fn rom_search_roots() -> Vec<std::path::PathBuf> {
    machine::search_roots()
}

fn dims(with_border: bool) -> (usize, usize) {
    if with_border {
        (352, 296)
    } else {
        (256, 192)
    }
}

#[cfg(test)]
mod activation_tests {
    use super::{HostRuntime, HostSession, ModelId};
    use crate::prefs::{PrefJoystick, UiPreferences};
    use machine::{JoystickMode, NextMachine};

    #[test]
    fn failed_boot_preserves_live_machine_and_success_commits() {
        HostSession::test_activate_model_transaction();
    }

    #[test]
    fn native_preferences_apply_on_next_without_classic_tape_options() {
        let mut session = HostSession::new(ModelId::SpectrumNext, true);
        let next = NextMachine::new(&vec![0xff; 0x10000]).expect("Next ROM image is 64 KiB");
        session.model = ModelId::SpectrumNext;
        session.machine = Some(HostRuntime::Next(Box::new(next)));
        let prefs = UiPreferences {
            joystick_mode: PrefJoystick::Cursor,
            online_tape_titles: true,
            tape_ear_speed: 5,
            ..UiPreferences::default()
        };

        prefs
            .apply_to_host_session(&mut session)
            .expect("Next has no classic tape deck to configure");

        assert_eq!(session.joystick_mode(), JoystickMode::Cursor);
        assert!(session.online_tape_titles());
        assert!(session.tape_load_options().is_none());
    }
}

#[cfg(test)]
#[path = "session_tests.rs"]
mod tests;
