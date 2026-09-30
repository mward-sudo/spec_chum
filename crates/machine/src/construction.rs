use bus::{Bus128, Bus48, BusPlus3};
use ula::{Ula48, FRAME_TSTATES_PENTAGON};
use z80::Cpu;

use super::{
    read_exrom_with_overrides, read_trdos_rom_with_overrides, Debugger, Machine, MachineBuildError,
    Model, TapeLoadOptions,
};

impl Machine {
    fn new_spec48(bus: Bus48, model_id: u8) -> Self {
        trace::emit(trace::EventKind::MachineModel { model: model_id });
        Self::Spec48 {
            cpu: Cpu::new(),
            bus: Box::new(bus),
            ula: Ula48::new(),
            tape: None,
            tape_opts: TapeLoadOptions::default(),
            rzx: None,
            debugger: Debugger::default(),
        }
    }

    fn new_spec128(
        bus: Bus128,
        model_id: u8,
        plus2_rom: bool,
        pentagon: bool,
        scorpion: bool,
    ) -> Self {
        trace::emit(trace::EventKind::MachineModel { model: model_id });
        Self::Spec128 {
            cpu: Cpu::new(),
            bus: Box::new(bus),
            ula: Ula48::new(),
            tape: None,
            tape_opts: TapeLoadOptions::default(),
            rzx: None,
            debugger: Debugger::default(),
            plus2_rom,
            pentagon,
            scorpion,
        }
    }

    pub fn new_48k(rom: &[u8]) -> Result<Self, MachineBuildError> {
        let mut bus = Bus48::new();
        bus.load_rom(rom)?;
        Ok(Self::new_spec48(bus, 0))
    }

    /// Spectrum 16K: 48K ULA / bus timing, 16 KiB RAM only (#188).
    pub fn new_16k(rom: &[u8]) -> Result<Self, MachineBuildError> {
        let mut bus = Bus48::new();
        bus.ram16k = true;
        bus.load_rom(rom)?;
        Ok(Self::new_spec48(bus, 5))
    }

    /// Timex TC2048: 48K hardware + SCLD ports (#192 Phase 1).
    pub fn new_timex_tc2048(rom: &[u8]) -> Result<Self, MachineBuildError> {
        let mut bus = Bus48::new();
        bus.timex = true;
        bus.load_rom(rom)?;
        Ok(Self::new_spec48(bus, 7))
    }

    /// Timex TS2068 / TC2068: home ROM + EX-ROM, horizontal MMU, AY (#192 Phase 2a).
    pub fn new_timex_ts2068(home_rom: &[u8], exrom: &[u8]) -> Result<Self, MachineBuildError> {
        let mut bus = Bus48::new();
        bus.timex = true;
        bus.timex_2068 = true;
        bus.load_rom(home_rom)?;
        bus.load_timex_exrom(exrom)?;
        Ok(Self::new_spec48(bus, 8))
    }

    pub fn new_128k(rom: &[u8]) -> Result<Self, MachineBuildError> {
        let mut bus = Bus128::new();
        bus.load_rom128(rom)?;
        Ok(Self::new_spec128(bus, 1, false, false, false))
    }

    /// Amstrad grey +2: 128K hardware with `roms/plus2/` ROM (#188).
    pub fn new_plus2(rom: &[u8]) -> Result<Self, MachineBuildError> {
        let mut bus = Bus128::new();
        bus.load_rom128(rom)?;
        Ok(Self::new_spec128(bus, 4, true, false, false))
    }

    /// Pentagon 128: 128K banking, user main ROM + TR-DOS (#188 Phase B / #193).
    pub fn new_pentagon128(main_rom: &[u8], trdos_rom: &[u8]) -> Result<Self, MachineBuildError> {
        let mut bus = Bus128::new();
        bus.frame_tstates = FRAME_TSTATES_PENTAGON;
        bus.load_rom128(main_rom)?;
        let mut m = Self::new_spec128(bus, 6, false, true, false);
        m.attach_beta()
            .map_err(|e| MachineBuildError::Message(e.to_string()))?
            .load_rom(trdos_rom)?;
        Ok(m)
    }

    /// Scorpion ZS-256: 256K banking + `#1FFD`, user main ROM + TR-DOS (#193).
    pub fn new_scorpion_zs256(
        main_rom: &[u8],
        trdos_rom: &[u8],
    ) -> Result<Self, MachineBuildError> {
        let mut bus = Bus128::new();
        bus.scorpion = true;
        bus.frame_tstates = FRAME_TSTATES_PENTAGON;
        bus.load_rom_scorpion(main_rom)?;
        let mut m = Self::new_spec128(bus, 10, false, false, true);
        m.attach_beta()
            .map_err(|e| MachineBuildError::Message(e.to_string()))?
            .load_rom(trdos_rom)?;
        Ok(m)
    }

    pub fn new_plus3(rom: &[u8]) -> Result<Self, MachineBuildError> {
        Self::new_amstrad_plus(rom, true, false)
    }

    /// Spectrum +3e: same +3 hardware with Garry Lancaster enhanced ROMs (#194).
    pub fn new_plus3e(rom: &[u8]) -> Result<Self, MachineBuildError> {
        Self::new_amstrad_plus(rom, true, true)
    }

    /// Spectrum +2A: same gate array as +3 but FDC ports float (`disk_interface = false`).
    pub fn new_plus2a(rom: &[u8]) -> Result<Self, MachineBuildError> {
        Self::new_amstrad_plus(rom, false, false)
    }

    /// Construct a model using its main ROM and any required peripheral ROMs.
    pub fn from_rom_with_overrides(
        model: Model,
        rom: &[u8],
        overrides: &std::collections::BTreeMap<String, std::path::PathBuf>,
    ) -> Result<Self, MachineBuildError> {
        match model {
            Model::Spectrum16K => Self::new_16k(rom),
            Model::Spectrum48 => Self::new_48k(rom),
            Model::Spectrum128 => Self::new_128k(rom),
            Model::SpectrumPlus2 => Self::new_plus2(rom),
            Model::SpectrumPlus2A => Self::new_plus2a(rom),
            Model::SpectrumPlus3 => Self::new_plus3(rom),
            Model::SpectrumPlus3e => Self::new_plus3e(rom),
            Model::ScorpionZs256 | Model::Pentagon128 => {
                let trdos = read_trdos_rom_with_overrides(model, overrides)?;
                if model == Model::ScorpionZs256 {
                    Self::new_scorpion_zs256(rom, &trdos)
                } else {
                    Self::new_pentagon128(rom, &trdos)
                }
            }
            Model::TimexTC2048 => Self::new_timex_tc2048(rom),
            Model::TimexTS2068 => {
                let exrom = read_exrom_with_overrides(model, overrides)?;
                Self::new_timex_ts2068(rom, &exrom)
            }
        }
    }

    fn new_amstrad_plus(
        rom: &[u8],
        disk_interface: bool,
        plus3e: bool,
    ) -> Result<Self, MachineBuildError> {
        let mut bus = BusPlus3::new_with_disk(disk_interface);
        bus.load_rom64(rom)?;
        let model_id = if plus3e {
            9
        } else if disk_interface {
            2
        } else {
            3
        };
        trace::emit(trace::EventKind::MachineModel { model: model_id });
        Ok(Self::SpecPlus3 {
            cpu: Cpu::new(),
            bus: Box::new(bus),
            ula: Ula48::new(),
            tape: None,
            tape_opts: TapeLoadOptions::default(),
            rzx: None,
            debugger: Debugger::default(),
            plus3e,
        })
    }
}
