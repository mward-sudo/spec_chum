//! Safe session wrapper around [`machine::Machine`] for host frontends.

use std::path::Path;

use machine::{Machine, Model};
use thiserror::Error;

/// Model identifiers for the C ABI (stable numeric values).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u32)]
pub enum ModelId {
    Spectrum48 = 0,
    Spectrum128 = 1,
    SpectrumPlus3 = 2,
}

impl ModelId {
    #[must_use]
    pub fn from_u32(v: u32) -> Option<Self> {
        match v {
            0 => Some(Self::Spectrum48),
            1 => Some(Self::Spectrum128),
            2 => Some(Self::SpectrumPlus3),
            _ => None,
        }
    }

    #[must_use]
    pub fn to_model(self) -> Model {
        match self {
            Self::Spectrum48 => Model::Spectrum48,
            Self::Spectrum128 => Model::Spectrum128,
            Self::SpectrumPlus3 => Model::SpectrumPlus3,
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

    /// Load ROM bytes for the current model.
    pub fn load_rom_bytes(&mut self, rom: &[u8]) -> Result<(), HostError> {
        let machine = match self.model {
            ModelId::Spectrum48 => Machine::new_48k(rom),
            ModelId::Spectrum128 => Machine::new_128k(rom),
            ModelId::SpectrumPlus3 => Machine::new_plus3(rom),
        }
        .map_err(HostError::Message)?;
        self.machine = Some(machine);
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
        m.reset();
        self.status = "Reset".into();
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
                    match tape::TzxPlayer::to_tap_image(&data) {
                        Ok(tap) if !tap.blocks.is_empty() => {
                            let n = tap.blocks.len();
                            m.insert_tape(tape::TapPlayer::new(tap));
                            self.status = format!(
                                "Inserted TZX {} as TAP ({n} blocks, paused)",
                                path.display()
                            );
                            return Ok(());
                        }
                        Ok(_) => {}
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
        m.keyboard_mut().set_key(row, bit, pressed);
        Ok(())
    }

    pub fn clear_keys(&mut self) -> Result<(), HostError> {
        let Some(m) = self.machine.as_mut() else {
            return Err(HostError::NoMachine);
        };
        m.keyboard_mut().reset();
        Ok(())
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
            machine::Model::Spectrum128 | machine::Model::SpectrumPlus3 => 70_908,
        };
        render_frame_pcm(&audio, frame_t, &mut self.audio_pcm);
        m.render_rgba(&mut self.framebuffer, self.with_border);
    }
}

/// Host audio sample rate (matches egui cpal default path).
pub const AUDIO_SAMPLE_RATE: u32 = 44_100;
/// Samples rendered per 50 Hz frame.
pub const AUDIO_SAMPLES_PER_FRAME: usize = (AUDIO_SAMPLE_RATE as usize) / 50;

fn render_frame_pcm(audio: &machine::FrameAudio, frame_tstates: u32, out: &mut Vec<f32>) {
    out.clear();
    out.resize(AUDIO_SAMPLES_PER_FRAME, 0.0);
    let t_per = frame_tstates as f32 / AUDIO_SAMPLES_PER_FRAME as f32;
    let mut edge_i = 0usize;
    let mut level = false;
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
    fn open_fixture_tap_progress_and_audio_pcm() {
        let Some(rom) = rom48() else {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        };
        let mut s = HostSession::new(ModelId::Spectrum48, true);
        s.load_rom_bytes(&rom).expect("rom");
        let tap = workspace_root().join("tests/fixtures/tape/minimal_code.tap");
        s.open_tape(&tap).expect("tap");
        let p = s.tape_progress().expect("progress");
        assert_eq!(p.block_index, 0);
        assert_eq!(p.block_count, 2);
        s.play_tape().expect("play");
        s.run_frame();
        assert_eq!(s.audio_pcm().len(), AUDIO_SAMPLES_PER_FRAME);
        let energy: f32 = s.audio_pcm().iter().map(|x| x * x).sum();
        assert!(
            energy > 0.01,
            "playing tape should produce audible energy, got {energy}"
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
        assert_eq!(ModelId::from_u32(9), None);
        assert_eq!(ModelId::Spectrum48.to_model(), Model::Spectrum48);
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
}
