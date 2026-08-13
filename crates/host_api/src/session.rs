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
}

impl HostSession {
    #[must_use]
    pub fn new(model: ModelId, with_border: bool) -> Self {
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

    /// Run one video frame into the RGBA framebuffer when `running`.
    pub fn run_frame(&mut self) {
        if !self.running {
            return;
        }
        let Some(m) = self.machine.as_mut() else {
            return;
        };
        let _audio = m.run_frame();
        m.render_rgba(&mut self.framebuffer, self.with_border);
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
    fn model_id_roundtrip() {
        assert_eq!(ModelId::from_u32(0), Some(ModelId::Spectrum48));
        assert_eq!(ModelId::from_u32(1), Some(ModelId::Spectrum128));
        assert_eq!(ModelId::from_u32(2), Some(ModelId::SpectrumPlus3));
        assert_eq!(ModelId::from_u32(9), None);
        assert_eq!(ModelId::Spectrum48.to_model(), Model::Spectrum48);
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
