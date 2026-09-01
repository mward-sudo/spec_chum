//! Spec Chum — egui frontend library (testable without a display).

mod display;
mod keymap;
mod theme;
mod window_capture;

pub use window_capture::OwnWindowCapturer;

pub use keymap::MAPPING_DOC;

use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use parking_lot::{Mutex as ParkingMutex, MutexGuard};
use std::time::{Duration, Instant};

use control_plane::ControlPlane;
use eframe::egui;
use machine::{AyStereoMode, JoystickMode, JoystickState, Machine, Model, TapeLoadOptions};
use spec_chum_host::{
    apply_user_config, default_prefs_path, hardware_compat, install_model_rom, load_prefs,
    model_requires_user_rom, model_rom_available, rom_setup_json, save_prefs,
    slot_rom_overrides_for_model, sync_model_rom_paths, HostSession, ModelId, PrefAyStereo,
    PrefJoystick, PrefModel, RomSetupJson, UiPreferences, UserMachineConfig, MIN_WINDOW_HEIGHT,
    MIN_WINDOW_WIDTH,
};

/// Live machine storage: direct for GUI-only, shared `Arc` when agent HTTP is on (#221).
#[derive(Debug)]
enum HostSlot {
    Direct(Box<HostSession>),
    Shared(Arc<ParkingMutex<HostSession>>),
}

/// Mutable access to the live [`HostSession`] (exclusive or mutex-guarded).
#[derive(Debug)]
pub enum HostAccess<'a> {
    Exclusive(&'a mut HostSession),
    Locked(MutexGuard<'a, HostSession>),
}

impl Deref for HostAccess<'_> {
    type Target = HostSession;
    fn deref(&self) -> &HostSession {
        match self {
            Self::Exclusive(s) => s,
            Self::Locked(g) => g,
        }
    }
}

impl DerefMut for HostAccess<'_> {
    fn deref_mut(&mut self) -> &mut HostSession {
        match self {
            Self::Exclusive(s) => s,
            Self::Locked(g) => g,
        }
    }
}

/// Session state shared by the GUI and headless tests.
///
/// Live machine state lives in [`HostSession`] — shared via `Arc` when
/// `SPEC_CHUM_AGENT=1` embeds HTTP on the same plane as Debug (#221).
#[derive(Debug)]
pub struct EmulatorSession {
    host: HostSlot,
    pub throttle: bool,
    pub muted: bool,
    /// Host PCM gain 0…1 (what the user hears). Does not affect EAR / flash-load.
    pub volume: f32,
    pub debug_open: bool,
    pub debug_mem_addr: u16,
    pub debug_break_pc: u16,
    /// When true, accumulate egui pointer delta into the Kempston mouse each frame.
    pub kempston_mouse: bool,
    /// Scripted matrix keys (e.g. auto-type `LOAD ""`).
    key_script: Option<KeyScript>,
    /// After Instant Type LOAD finishes, auto-Play with flash-load on.
    pending_instant_play: bool,
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

    /// +2A: menu **Loader** is tape — Enter alone for PROGRAM; CODE via 48 BASIC.
    fn load_quotes_plus2a(with_code: bool) -> Self {
        if with_code {
            return Self::load_quotes_128_or_plus3(true);
        }
        let enter = vec![(6, 0)];
        let gap = Vec::new();
        Self {
            steps: vec![(enter, Self::PRESS), (gap, Self::MENU_TO_48_BASIC_WAIT)],
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
        let mut host = HostSession::new(ModelId::from_model(model), with_border);
        host.set_status("Load a ROM via Machine menu (or auto-detect from roms/)");
        Self {
            host: HostSlot::Direct(Box::new(host)),
            throttle: true,
            muted: false,
            volume: 1.0,
            debug_open: false,
            debug_mem_addr: 0,
            debug_break_pc: 0,
            kempston_mouse: false,
            key_script: None,
            pending_instant_play: false,
        }
    }

    /// Mutable access to the live session (no lock when agent HTTP is off).
    pub fn host_mut(&mut self) -> HostAccess<'_> {
        match &mut self.host {
            HostSlot::Direct(s) => HostAccess::Exclusive(s.as_mut()),
            HostSlot::Shared(a) => HostAccess::Locked(a.lock()),
        }
    }

    /// Promote to a shared `Arc` for [`ControlPlane::from_shared`] / embedded HTTP.
    pub fn share_host(&mut self) -> Arc<ParkingMutex<HostSession>> {
        match &self.host {
            HostSlot::Shared(a) => return Arc::clone(a),
            HostSlot::Direct(_) => {}
        }
        let placeholder = HostSession::new(ModelId::Spectrum48, true);
        let HostSlot::Direct(session) =
            std::mem::replace(&mut self.host, HostSlot::Direct(Box::new(placeholder)))
        else {
            unreachable!("just checked Direct");
        };
        let arc = Arc::new(ParkingMutex::new(*session));
        self.host = HostSlot::Shared(Arc::clone(&arc));
        arc
    }

    #[must_use]
    pub fn model(&self) -> Model {
        match &self.host {
            HostSlot::Direct(s) => s.model().to_model(),
            HostSlot::Shared(a) => a.lock().model().to_model(),
        }
    }

    pub fn set_model(&mut self, model: Model) {
        self.host_mut().set_model(ModelId::from_model(model));
    }

    #[must_use]
    pub fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    pub fn try_autoload_rom(&mut self) {
        let root = Self::workspace_root();
        let model = self.model();
        let overrides = slot_rom_overrides_for_model(PrefModel::from_model(model).to_model_id());
        if let Some(path) = machine::resolve_rom_path_in_with_overrides(
            model,
            std::slice::from_ref(&root),
            &overrides,
        ) {
            if let Ok(data) = std::fs::read(&path) {
                let built = Self::build_machine(model, &data, &overrides);
                match built {
                    Ok(m) => {
                        self.host_mut().set_machine(m);
                        self.host_mut()
                            .set_status(format!("Loaded {}", path.display()));
                    }
                    Err(e) => self.host_mut().set_status(e),
                }
                return;
            }
        }
        self.host_mut().set_status(format!(
            "Missing ROM for {} — {}",
            machine::model_title(model),
            machine::unavailable_reason(model)
        ));
    }

    fn build_machine(
        model: Model,
        data: &[u8],
        overrides: &std::collections::BTreeMap<String, std::path::PathBuf>,
    ) -> Result<Machine, String> {
        match model {
            Model::Spectrum16K => Machine::new_16k(data),
            Model::Spectrum48 => Machine::new_48k(data),
            Model::SpectrumPlus2 => Machine::new_plus2(data),
            Model::SpectrumPlus2A => Machine::new_plus2a(data),
            Model::SpectrumPlus3 => Machine::new_plus3(data),
            Model::Spectrum128 => Machine::new_128k(data),
            Model::Pentagon128 => {
                let trdos = machine::read_trdos_rom_with_overrides(Model::Pentagon128, overrides)?;
                Machine::new_pentagon128(data, &trdos)
            }
            Model::TimexTC2048 => Machine::new_timex_tc2048(data).map_err(|e| e.to_string()),
            Model::TimexTS2068 => {
                let exrom = machine::read_exrom_with_overrides(Model::TimexTS2068, overrides)?;
                Machine::new_timex_ts2068(data, &exrom).map_err(|e| e.to_string())
            }
        }
    }

    /// Boot from a saved user profile (#187).
    pub fn apply_user_machine_config(
        &mut self,
        config: &UserMachineConfig,
    ) -> Result<(), spec_chum_host::MachineConfigError> {
        let roots = vec![Self::workspace_root()];
        let applied = apply_user_config(config, &roots)?;
        self.set_model(applied.model);
        self.host_mut().set_joystick_mode(applied.joystick_mode);
        self.kempston_mouse = applied.kempston_mouse;
        self.host_mut().set_machine(applied.machine);
        self.host_mut().set_status(applied.status);
        Ok(())
    }

    pub fn load_snapshot(&mut self, path: &Path) {
        if let Ok(snap) =
            formats::Snapshot128::load_sna(path).or_else(|_| formats::Snapshot128::load_z80(path))
        {
            let target = match snap.model {
                formats::Snapshot128Model::SpectrumPlus3 => Model::SpectrumPlus3,
                formats::Snapshot128Model::SpectrumPlus2A => Model::SpectrumPlus2A,
                formats::Snapshot128Model::Spectrum128 => Model::Spectrum128,
            };
            if !self.host_mut().has_machine() || self.model() != target {
                self.set_model(target);
                self.host_mut().clear_machine();
                self.try_autoload_rom();
            }
            {
                let host = &mut *self.host_mut();
                if let Some(m) = host.machine_mut() {
                    m.apply_snapshot128(&snap);
                    host.set_status(format!("Loaded 128K/+2A/+3 snapshot {}", path.display()));
                }
            }
            return;
        }
        match formats::Snapshot48::load_sna(path).or_else(|_| formats::Snapshot48::load_z80(path)) {
            Ok(snap) => {
                if !self.host_mut().has_machine() || self.model() != Model::Spectrum48 {
                    self.set_model(Model::Spectrum48);
                    self.host_mut().clear_machine();
                    self.try_autoload_rom();
                }
                {
                    let host = &mut *self.host_mut();
                    if let Some(m) = host.machine_mut() {
                        m.apply_snapshot48(&snap);
                        host.set_status(format!("Loaded snapshot {}", path.display()));
                    }
                }
            }
            Err(e) => self.host_mut().set_status(format!("Snapshot error: {e}")),
        }
    }

    pub fn load_tap(&mut self, path: &Path) {
        match tape::TapImage::load(path) {
            Ok(img) => {
                let host = &mut *self.host_mut();
                if let Some(m) = host.machine_mut() {
                    m.insert_tape(tape::TapPlayer::new(img));
                    let mut opts = m.tape_load_options();
                    if opts.flash_load || opts.experience_load {
                        opts.flash_load = false;
                        m.set_tape_load_options(opts);
                    }
                    host.set_status(format!(
                        "Inserted TAP {} (paused — Tape → Play for EAR, or Instant)",
                        path.display()
                    ));
                } else {
                    host.set_status("Load a machine ROM before inserting tape");
                }
            }
            Err(e) => self.host_mut().set_status(format!("TAP error: {e}")),
        }
    }

    pub fn load_tzx(&mut self, path: &Path) {
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                self.host_mut().set_status(format!("TZX error: {e}"));
                return;
            }
        };
        if !self.host_mut().has_machine() {
            self.host_mut()
                .set_status("Load a machine ROM before inserting tape");
            return;
        }
        // Standard-speed TZX → TAP deck so ROM/RAM LD-BYTES flash-load works (e.g. The Boggit).
        if tape::TzxPlayer::is_standard_speed_only(&data) {
            match tape::TzxPlayer::to_tap_player(&data) {
                Ok(player) if player.image.blocks.is_empty() => {}
                Ok(player) => {
                    let n = player.image.blocks.len();
                    {
                        let host = &mut *self.host_mut();
                        if let Some(m) = host.machine_mut() {
                            m.insert_tape(player);
                        }
                    }
                    self.force_flash_load(false);
                    self.host_mut().set_status(format!(
                        "Inserted TZX {} as TAP ({n} blocks, paused). Type LOAD \"\" then Play (EAR), or Instant. 128K/+3: Type LOAD enters 48 BASIC (+3 disk Loader is not tape). +2A: Type LOAD uses menu Loader (tape).",
                        path.display()
                    ));
                    return;
                }
                Err(e) => {
                    self.host_mut().set_status(format!("TZX error: {e}"));
                    return;
                }
            }
        }
        match tape::TzxPlayer::parse(&data) {
            Ok(player) => {
                {
                    let host = &mut *self.host_mut();
                    if let Some(m) = host.machine_mut() {
                        m.insert_tzx(player);
                    }
                }
                self.force_flash_load(false);
                self.host_mut().set_status(format!(
                    "Inserted TZX {} (pulse playback, paused — Play when loader is ready)",
                    path.display()
                ));
            }
            Err(e) => self.host_mut().set_status(format!("TZX error: {e}")),
        }
    }

    /// Play always uses the EAR path (flash-load off), respecting the EAR speed multiplier.
    pub fn play_tape(&mut self) {
        self.force_flash_load(false);
        self.play_tape_keeping_options();
    }

    /// Start the deck without changing flash-load (used by Instant while flash is temporarily on).
    fn play_tape_keeping_options(&mut self) {
        {
            let host = &mut *self.host_mut();
            if let Some(m) = host.machine_mut() {
                if m.has_tape() {
                    m.set_tape_playing(true);
                    host.set_status("Tape playing");
                } else {
                    host.set_status("No tape inserted");
                }
            }
        }
    }

    pub fn pause_tape(&mut self) {
        let host = &mut *self.host_mut();
        if let Some(m) = host.machine_mut() {
            m.set_tape_playing(false);
            let mut opts = m.tape_load_options();
            if opts.flash_load || opts.experience_load {
                opts.flash_load = false;
                m.set_tape_load_options(opts);
            }
            host.set_status("Tape paused");
        }
    }

    pub fn rewind_tape(&mut self) {
        let host = &mut *self.host_mut();
        if let Some(m) = host.machine_mut() {
            m.rewind_tape();
            let mut opts = m.tape_load_options();
            if opts.flash_load || opts.experience_load {
                opts.flash_load = false;
                m.set_tape_load_options(opts);
            }
            host.set_status("Tape rewound (paused)");
        }
    }

    fn force_flash_load(&mut self, on: bool) {
        {
            let host = &mut *self.host_mut();
            if let Some(m) = host.machine_mut() {
                let mut opts = m.tape_load_options();
                if opts.flash_load == on && (!on || !opts.experience_load) {
                    return;
                }
                opts.flash_load = on;
                if on {
                    opts.experience_load = false;
                }
                m.set_tape_load_options(opts);
            }
        }
    }

    /// Queue `LOAD ""` + Enter for 48K keyword mode; press Play separately when LD-BYTES waits.
    pub fn type_load_quotes(&mut self) {
        self.type_load_quotes_inner(false, false);
    }

    /// Queue `LOAD "" CODE` + Enter (48K) for CODE blocks.
    pub fn type_load_quotes_code(&mut self) {
        self.type_load_quotes_inner(true, false);
    }

    /// Instant load: open a tape image, enable flash-load, Type LOAD "" (PROGRAM), then Play.
    /// UI always prompts for a path first (`instant_load_path`). If already at LD-BYTES, Play immediately.
    pub fn instant_load_tape(&mut self) {
        let check = {
            let host = &*self.host_mut();
            match host.machine() {
                None => None,
                Some(m) if !m.has_tape() => Some(None),
                Some(m) => Some(Some(m.cpu().regs.pc == tape::LD_BYTES_TRAP_PC)),
            }
        };
        let at_ld_bytes = match check {
            None => {
                self.host_mut().set_status("Instant: no machine");
                return;
            }
            Some(None) => {
                self.host_mut().set_status("Instant: insert a tape first");
                return;
            }
            Some(Some(at)) => at,
        };
        self.force_flash_load(true);
        if at_ld_bytes {
            self.pending_instant_play = false;
            self.play_tape_keeping_options();
            self.host_mut()
                .set_status("Instant: flash-loading at LD-BYTES");
            return;
        }

        self.type_load_quotes_inner(false, true);
        self.host_mut()
            .set_status("Instant: typing LOAD \"\" then flash-load Play");
    }

    /// Always-prompt Instant: insert the chosen image, then flash + Type LOAD + Play.
    /// Does not reuse a previously inserted tape without selecting a path.
    pub fn instant_load_path(&mut self, path: &Path) {
        let ext = path
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "dsk" => {
                self.load_dsk(path);
            }
            "tzx" => {
                self.load_tzx(path);
                if self.host_mut().machine().is_some_and(Machine::has_tape) {
                    self.instant_load_tape();
                }
            }
            _ => {
                self.load_tap(path);
                if self.host_mut().machine().is_some_and(Machine::has_tape) {
                    self.instant_load_tape();
                }
            }
        }
    }

    fn type_load_quotes_inner(&mut self, with_code: bool, pending_play: bool) {
        // +3 menu "Loader" is +3DOS disk — never Enter alone for tape (#144).
        // +2A menu Loader is tape — Enter alone for PROGRAM (#145).
        // 128K/+3: 48 BASIC then keyword LOAD (matches Machine::type_load_quotes_*).
        self.pending_instant_play = pending_play;
        self.key_script = Some(match self.model() {
            Model::Spectrum16K | Model::Spectrum48 | Model::TimexTC2048 | Model::TimexTS2068 => {
                if with_code {
                    KeyScript::load_quotes_code_48k()
                } else {
                    KeyScript::load_quotes_48k()
                }
            }
            Model::SpectrumPlus2A => KeyScript::load_quotes_plus2a(with_code),
            Model::Spectrum128
            | Model::SpectrumPlus2
            | Model::SpectrumPlus3
            | Model::Pentagon128 => KeyScript::load_quotes_128_or_plus3(with_code),
        });
        if pending_play {
            return;
        }
        let msg = match (self.model(), with_code) {
            (
                Model::Spectrum16K | Model::Spectrum48 | Model::TimexTC2048 | Model::TimexTS2068,
                true,
            ) => "Typing LOAD \"\" CODE — press Tape → Play when border goes red/cyan",
            (
                Model::Spectrum16K | Model::Spectrum48 | Model::TimexTC2048 | Model::TimexTS2068,
                false,
            ) => "Typing LOAD \"\" — press Tape → Play when the border goes red/cyan",
            (Model::SpectrumPlus2A, false) => {
                "Selecting +2A tape Loader — press Tape → Play when border goes red/cyan"
            }
            (_, true) => {
                "Typing 48 BASIC LOAD \"\" CODE — press Tape → Play when border goes red/cyan"
            }
            (_, false) => {
                "Typing 48 BASIC LOAD \"\" — press Tape → Play when the border goes red/cyan"
            }
        };
        self.host_mut().set_status(msg);
    }

    /// Advance scripted keys if any; returns true when a script consumed input this frame.
    pub fn tick_key_script(&mut self) -> bool {
        if !self.host_mut().has_machine() {
            return false;
        }
        let Some(script) = self.key_script.as_mut() else {
            return false;
        };
        if script.step_i >= script.steps.len() {
            self.key_script = None;
            self.finish_instant_play_if_pending();
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
            self.finish_instant_play_if_pending();
        }
        {
            let host = &mut *self.host_mut();
            if let Some(machine) = host.machine_mut() {
                let kb = machine.keyboard_mut();
                kb.reset();
                for &(row, bit) in &keys {
                    kb.set_key(row, bit, true);
                }
            }
        }
        true
    }

    fn finish_instant_play_if_pending(&mut self) {
        if !self.pending_instant_play {
            return;
        }
        self.pending_instant_play = false;
        self.play_tape_keeping_options();
        self.host_mut()
            .set_status("Instant: flash-loading after LOAD \"\"");
    }

    pub fn load_rzx(&mut self, path: &Path) {
        match formats::RzxRecording::load(path) {
            Ok(rec) => {
                let host = &mut *self.host_mut();
                if let Some(m) = host.machine_mut() {
                    m.insert_rzx(rec);
                    host.set_status(format!("Loaded RZX {}", path.display()));
                } else {
                    host.set_status("Load a machine ROM before RZX");
                }
            }
            Err(e) => self.host_mut().set_status(format!("RZX error: {e}")),
        }
    }

    pub fn load_dsk(&mut self, path: &Path) {
        match formats::DskImage::load(path) {
            Ok(img) => {
                let host = &mut *self.host_mut();
                if let Some(m) = host.machine_mut() {
                    match m.insert_disk(img) {
                        Ok(()) => {
                            host.set_status(format!(
                                "DSK inserted ({}) — use +3 Loader / +3DOS",
                                path.display()
                            ));
                        }
                        Err(e) => host.set_status(e),
                    }
                } else {
                    host.set_status("Load +3 ROM before inserting disk");
                }
            }
            Err(e) => self.host_mut().set_status(format!("DSK error: {e}")),
        }
    }

    pub fn attach_multiface(&mut self, path: &Path) {
        match std::fs::read(path) {
            Ok(data) => {
                let host = &mut *self.host_mut();
                if let Some(m) = host.machine_mut() {
                    match m.attach_multiface(&data) {
                        Ok(()) => {
                            host.set_status(format!("Attached Multiface 1 {}", path.display()))
                        }
                        Err(e) => host.set_status(e),
                    }
                } else {
                    host.set_status("Load a 48K ROM before Multiface");
                }
            }
            Err(e) => self
                .host_mut()
                .set_status(format!("Multiface ROM error: {e}")),
        }
    }

    pub fn multiface_nmi(&mut self) {
        {
            let host = &mut *self.host_mut();
            if let Some(m) = host.machine_mut() {
                if m.multiface_nmi().is_some() {
                    host.set_status("Multiface NMI");
                } else {
                    host.set_status("Multiface not attached");
                }
            }
        }
    }

    pub fn attach_divmmc_stub(&mut self) {
        {
            let host = &mut *self.host_mut();
            if let Some(m) = host.machine_mut() {
                match m.attach_divmmc() {
                    Ok(_) => host.set_status("DivMMC attached"),
                    Err(e) => host.set_status(e),
                }
            } else {
                host.set_status("Load a machine ROM first");
            }
        }
    }

    pub fn attach_divmmc_sd(&mut self, path: &Path) {
        match std::fs::read(path) {
            Ok(data) => {
                let host = &mut *self.host_mut();
                if let Some(m) = host.machine_mut() {
                    match m.attach_divmmc() {
                        Ok(div) => {
                            div.attach_sd(data);
                            host.set_status(format!("DivMMC SD {}", path.display()));
                        }
                        Err(e) => host.set_status(e),
                    }
                } else {
                    host.set_status("Load a machine ROM first");
                }
            }
            Err(e) => self.host_mut().set_status(format!("SD image error: {e}")),
        }
    }

    pub fn attach_divmmc_eeprom(&mut self, path: &Path) {
        match std::fs::read(path) {
            Ok(data) => {
                let host = &mut *self.host_mut();
                if let Some(m) = host.machine_mut() {
                    match m.attach_divmmc_eeprom(&data) {
                        Ok(()) => {
                            host.set_status(format!("DivMMC EEPROM {}", path.display()));
                        }
                        Err(e) => host.set_status(e),
                    }
                } else {
                    host.set_status("Load a machine ROM first");
                }
            }
            Err(e) => self.host_mut().set_status(format!("EEPROM error: {e}")),
        }
    }

    pub fn attach_interface1_stub(&mut self) {
        {
            let host = &mut *self.host_mut();
            if let Some(m) = host.machine_mut() {
                match m.attach_interface1() {
                    Ok(if1) => {
                        // Optional IF1 ROM — not shipped; try common local paths.
                        let mut loaded = if1.rom_loaded;
                        if !loaded {
                            let roots = [
                                Self::workspace_root(),
                                std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
                            ];
                            for cand in ["roms/if1.rom", "roms/if1-2.rom", "roms/interface1.rom"] {
                                for root in &roots {
                                    let path = root.join(cand);
                                    if let Ok(data) = std::fs::read(&path) {
                                        match if1.load_rom(&data) {
                                            Ok(()) => {
                                                loaded = true;
                                                break;
                                            }
                                            Err(e) => {
                                                host.set_status(e.to_string());
                                                return;
                                            }
                                        }
                                    }
                                }
                                if loaded {
                                    break;
                                }
                            }
                        }
                        host.set_status(if loaded {
                            "Interface 1 attached (ROM loaded)"
                        } else {
                            "Interface 1 attached (no roms/if1.rom — paging hooks ready)"
                        });
                    }
                    Err(e) => host.set_status(e.to_string()),
                }
            } else {
                host.set_status("Load a machine ROM first");
            }
        }
    }

    pub fn insert_mdr(&mut self, path: &Path) {
        match std::fs::read(path) {
            Ok(data) => match formats::MdrImage::parse(&data) {
                Ok(cart) => {
                    let host = &mut *self.host_mut();
                    if let Some(m) = host.machine_mut() {
                        match m.attach_interface1() {
                            Ok(if1) => {
                                if1.insert_mdr(cart);
                                host.set_status(format!("Inserted MDR {}", path.display()));
                            }
                            Err(e) => host.set_status(e.to_string()),
                        }
                    } else {
                        host.set_status("Load a machine ROM first");
                    }
                }
                Err(e) => self.host_mut().set_status(format!("MDR error: {e}")),
            },
            Err(e) => self.host_mut().set_status(format!("MDR error: {e}")),
        }
    }

    pub fn insert_dck(&mut self, path: &Path) {
        match std::fs::read(path) {
            Ok(data) => match formats::DckImage::parse(&data) {
                Ok(image) => {
                    let host = &mut *self.host_mut();
                    if let Some(m) = host.machine_mut() {
                        match m.insert_timex_dock(&image) {
                            Ok(()) => {
                                host.set_status(format!("Inserted DCK {}", path.display()));
                            }
                            Err(e) => host.set_status(e.to_string()),
                        }
                    } else {
                        host.set_status("Load a machine ROM first");
                    }
                }
                Err(e) => self.host_mut().set_status(format!("DCK error: {e}")),
            },
            Err(e) => self.host_mut().set_status(format!("DCK error: {e}")),
        }
    }

    pub fn eject_dck(&mut self) {
        {
            let host = &mut *self.host_mut();
            if let Some(m) = host.machine_mut() {
                match m.eject_timex_dock() {
                    Ok(()) => host.set_status("Ejected Timex dock"),
                    Err(e) => host.set_status(e.to_string()),
                }
            } else {
                host.set_status("Load a machine ROM first");
            }
        }
    }

    pub fn attach_beta_stub(&mut self) {
        {
            let host = &mut *self.host_mut();
            if let Some(m) = host.machine_mut() {
                match m.attach_beta() {
                    Ok(_) => host.set_status("Beta Disk attached"),
                    Err(e) => host.set_status(e),
                }
            } else {
                host.set_status("Load a machine ROM first");
            }
        }
    }

    pub fn load_trdos_rom(&mut self, path: &Path) {
        match std::fs::read(path) {
            Ok(data) => {
                let host = &mut *self.host_mut();
                if let Some(m) = host.machine_mut() {
                    match m.load_trdos_rom(&data) {
                        Ok(()) => {
                            host.set_status(format!("Loaded TR-DOS ROM {}", path.display()));
                        }
                        Err(e) => host.set_status(e),
                    }
                } else {
                    host.set_status("Load a machine ROM first");
                }
            }
            Err(e) => self.host_mut().set_status(format!("TR-DOS ROM error: {e}")),
        }
    }

    pub fn insert_trd(&mut self, path: &Path) {
        match formats::TrdImage::load(path) {
            Ok(img) => {
                let host = &mut *self.host_mut();
                if let Some(m) = host.machine_mut() {
                    match m.attach_beta() {
                        Ok(beta) => {
                            beta.insert(img);
                            host.set_status(format!("Inserted TRD {}", path.display()));
                        }
                        Err(e) => host.set_status(e),
                    }
                } else {
                    host.set_status("Load a machine ROM first");
                }
            }
            Err(e) => self.host_mut().set_status(format!("TRD error: {e}")),
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
        let host = &mut *self.host_mut();
        let joystick_mode = host.joystick_mode();
        let Some(machine) = host.machine_mut() else {
            return;
        };
        machine.keyboard_mut().reset();
        let mut stick = pad;
        stick.left |= keys_down.contains(&egui::Key::ArrowLeft);
        stick.right |= keys_down.contains(&egui::Key::ArrowRight);
        stick.up |= keys_down.contains(&egui::Key::ArrowUp);
        stick.down |= keys_down.contains(&egui::Key::ArrowDown);
        stick.fire |= keys_down.contains(&egui::Key::Tab);
        machine.apply_joystick_state(joystick_mode, stick);

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
        let host = &mut *self.host_mut();
        let Some(machine) = host.machine_mut() else {
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
        let host = &mut *self.host_mut();
        let Some(machine) = host.machine_mut() else {
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
        self.host_mut().run_frame()
    }

    /// Grow/shrink the RGBA buffer when Timex SCLD switches between 256 and 512.
    pub fn sync_framebuffer_dims(&mut self) {
        self.host_mut().refresh_framebuffer();
    }
}

pub struct SpecChumApp {
    pub session: EmulatorSession,
    /// Present when embedded agent HTTP shares the session (#221).
    plane: Option<Arc<ControlPlane>>,
    _agent: Option<agent_server::embedded::EmbeddedServer>,
    /// Own-window capturer for `GET /v1/host/window` (#239); registered on `plane`.
    window_capturer: Option<Arc<window_capture::OwnWindowCapturer>>,
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
        f.debug_struct("SpecChumApp")
            .field("session", &self.session)
            .field("agent", &self._agent.as_ref().map(|s| s.addr.as_str()))
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
                        | Model::Pentagon128
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
            let cap = window_capture::OwnWindowCapturer::new();
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
                        | Model::Pentagon128
                        | Model::TimexTS2068
                ) {
                    m.set_ay_stereo_mode(self.prefs.effective_ay_stereo());
                }
            }
        }
    }

    fn open_recent_path(&mut self, path: PathBuf) {
        if !path.is_file() {
            self.session
                .host_mut()
                .set_status(format!("Recent file missing: {}", path.display()));
            self.prefs
                .recent_files
                .retain(|p| Path::new(p) != path.as_path());
            self.mark_prefs_dirty();
            return;
        }
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        match ext.as_str() {
            "sna" | "z80" => self.session.load_snapshot(&path),
            "tap" => self.session.load_tap(&path),
            "tzx" => self.session.load_tzx(&path),
            "rzx" => self.session.load_rzx(&path),
            "dsk" => self.session.load_dsk(&path),
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
            self.note_recent_if_ok(&path);
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
                                    Err(e) => self.rom_setup_error = Some(e),
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
                        let available = model_rom_available(
                            pref.to_model_id(),
                            &self.prefs.model_rom_paths,
                        ) || draft.custom_rom_path.is_some();
                        let title = machine::model_title(pick);
                        let mut selected = draft.base == pref;
                        if ui
                            .add_enabled_ui(available, |ui| {
                                ui.radio_value(&mut selected, true, title)
                            })
                            .inner
                            .clicked()
                        {
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
                                .checkbox(&mut draft.attach_multiface, "Multiface 1")
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
                                        self.open_recent_path(PathBuf::from(path_str));
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
                                | Model::Pentagon128
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

                        if matches!(model, Model::Spectrum16K | Model::Spectrum48 | Model::TimexTC2048 | Model::TimexTS2068) {
                            if ui.button("Attach Multiface 1 ROM…").clicked() {
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
                            ui.label("Multiface 1: 48K / 16K only");
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

                        if model == Model::SpectrumPlus3 {
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
                            .on_hover_text(
                                "Always asks for a TAP/TZX, then flash-loads (Type LOAD \"\" + Play). Play alone stays EAR-only. Use File → Open DSK for disks.",
                            )
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
                                            "Abbreviated pauses on the EAR path at 16× (~20s-class; issue #82)",
                                        )
                                        .clicked()
                                    {
                                        m.set_tape_load_options(TapeLoadOptions::experience());
                                        tape_status =
                                            Some("Tape: experience load (~20s EAR)".into());
                                        tape_prefs_changed = true;
                                    }
                                    ui.label("EAR speed:");
                                    for speed in [1u32, 2, 5, 10, 20] {
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
                    if let Some(p) = self
                        .session
                        .host_mut().machine()
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
                            let _ = self.session.host_mut().continue_execution();
                            self.session.host_mut().set_running(true);
                        }
                    } else if ui.button("Pause").clicked() {
                        self.session.host_mut().set_paused(true);
                        self.session.host_mut().set_status("Paused");
                    }
                    if ui.button("Step").clicked() {
                        let _ = self.session.host_mut().debug_step();
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
                    .map(|r| (r.pc, r.sp, r.hl))
                    .unwrap_or((0, 0, 0));
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
                        let _ = self.session.host_mut().add_breakpoint(bp);
                    }
                    if ui.button("Clear breaks").clicked() {
                        let _ = self.session.host_mut().clear_breakpoints();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quote_maps_to_symbol_p_on_session() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if !session.host_mut().has_machine() {
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
        let host = &mut *session.host_mut();
        let kb = host.machine_mut().unwrap().keyboard_mut();
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
        if !session.host_mut().has_machine() {
            return;
        }
        let mut keys = std::collections::HashSet::new();
        keys.insert(egui::Key::ArrowLeft);
        session.host_mut().set_joystick_mode(JoystickMode::Kempston);
        session.sync_keyboard(&keys, egui::Modifiers::default(), JoystickState::empty());
        assert!(
            session
                .host_mut()
                .machine_mut()
                .unwrap()
                .kempston_mut()
                .left
        );

        session.host_mut().set_joystick_mode(JoystickMode::Cursor);
        session.sync_keyboard(&keys, egui::Modifiers::default(), JoystickState::empty());
        let host = &mut *session.host_mut();
        let m = host.machine_mut().unwrap();
        let rows = m.keyboard_mut().rows;
        assert_eq!(rows[0] & 1, 0); // Caps
        assert_eq!(rows[3] & (1 << 4), 0); // 5
        assert!(!m.kempston_mut().left);
    }

    #[test]
    fn physical_num1_survives_sinclair_left_joystick_clear() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if !session.host_mut().has_machine() {
            return;
        }
        let mut keys = std::collections::HashSet::new();
        keys.insert(egui::Key::Num1);
        session
            .host_mut()
            .set_joystick_mode(JoystickMode::SinclairLeft);
        session.sync_keyboard(&keys, egui::Modifiers::default(), JoystickState::empty());
        let host = &mut *session.host_mut();
        let rows = host.machine_mut().unwrap().keyboard_mut().rows;
        // Num1 = row 3 bit 0 must stay pressed even though Sinclair clears that row first.
        assert_eq!(rows[3] & (1 << 0), 0);
    }

    #[test]
    fn physical_num5_survives_cursor_joystick_clear() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if !session.host_mut().has_machine() {
            return;
        }
        let mut keys = std::collections::HashSet::new();
        keys.insert(egui::Key::Num5);
        session.host_mut().set_joystick_mode(JoystickMode::Cursor);
        session.sync_keyboard(&keys, egui::Modifiers::default(), JoystickState::empty());
        let host = &mut *session.host_mut();
        let rows = host.machine_mut().unwrap().keyboard_mut().rows;
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
        if !session.host_mut().has_machine() {
            eprintln!("skip: 128K ROM missing");
            return;
        }
        let path = std::env::temp_dir().join("spec_chum_app_128_to_48.sna");
        std::fs::write(&path, synthetic_sna48_bytes()).expect("write sna");
        session.load_snapshot(&path);
        assert_eq!(session.model(), Model::Spectrum48);
        assert_eq!(
            session.host_mut().machine().map(Machine::model),
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
        if !session.host_mut().has_machine() {
            eprintln!("skip: +3 ROM missing");
            return;
        }
        let path = std::env::temp_dir().join("spec_chum_app_plus3_to_48.sna");
        std::fs::write(&path, synthetic_sna48_bytes()).expect("write sna");
        session.load_snapshot(&path);
        assert_eq!(session.model(), Model::Spectrum48);
        assert_eq!(
            session.host_mut().machine().map(Machine::model),
            Some(Model::Spectrum48)
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn tzx_standard_inserts_as_paused_tap() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if !session.host_mut().has_machine() {
            return;
        }
        let boggit = PathBuf::from("/Users/michael/Downloads/BoggitThe/The Boggit - Side 1.tzx");
        if !boggit.exists() {
            // Fall back to synthesizing via fixture TAP path only
            let tap =
                EmulatorSession::workspace_root().join("tests/fixtures/tape/minimal_code.tap");
            session.load_tap(&tap);
            assert!(!session.host_mut().machine().unwrap().tape_playing());
            session.play_tape();
            assert!(session.host_mut().machine().unwrap().tape_playing());
            return;
        }
        session.load_tzx(&boggit);
        assert!(
            session.host_mut().status().contains("as TAP"),
            "status={}",
            session.host_mut().status()
        );
        assert!(!session.host_mut().machine().unwrap().tape_playing());
        assert!(session.host_mut().machine().unwrap().has_tape());
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
        if !session.host_mut().has_machine() {
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
        if !session.host_mut().has_machine() {
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
            !session
                .host_mut()
                .status()
                .to_lowercase()
                .contains("tape loader"),
            "must not point +3 users at disk Loader; status={}",
            session.host_mut().status()
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
        // Instant-style flash for this regression (Play alone is EAR-only).
        if let Some(m) = session.host_mut().machine_mut() {
            let mut opts = m.tape_load_options();
            opts.flash_load = true;
            m.set_tape_load_options(opts);
            m.set_tape_playing(true);
        }
        let mut loaded = false;
        for _ in 0..400 {
            session.tick_frame();
            let host = &*session.host_mut();
            let m = host.machine().unwrap();
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
        {
            let host = &*session.host_mut();
            let m = host.machine().unwrap();
            assert!(
                loaded,
                "plus3 egui Type LOAD CODE + flash Play should load attr_mark at 0x8000 (PC={:04X} block={:?})",
                m.cpu().regs.pc,
                m.tape_block()
            );
        }
    }

    #[test]
    fn play_tape_advances_ear_on_fixture() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if !session.host_mut().has_machine() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        let tap = EmulatorSession::workspace_root().join("tests/fixtures/tape/minimal_code.tap");
        session.load_tap(&tap);
        if let Some(m) = session.host_mut().machine_mut() {
            let mut opts = m.tape_load_options();
            opts.flash_load = false;
            m.set_tape_load_options(opts);
        }
        assert!(!session.host_mut().machine().unwrap().tape_playing());
        for _ in 0..3 {
            session.tick_frame();
        }
        assert!(
            !session.host_mut().machine().unwrap().ear(),
            "paused tape must not drive EAR"
        );
        session.play_tape();
        assert!(session.host_mut().machine().unwrap().tape_playing());
        assert!(session.host_mut().status().contains("playing"));
        let mut saw_high = false;
        for _ in 0..8 {
            session.tick_frame();
            if session.host_mut().machine().unwrap().ear() {
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
        if !session.host_mut().has_machine() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        let tap = EmulatorSession::workspace_root().join("tests/fixtures/tape/minimal_code.tap");
        session.load_tap(&tap);
        assert!(
            session.host_mut().status().contains("Inserted TAP"),
            "status={}",
            session.host_mut().status()
        );
        assert_eq!(
            session.host_mut().machine().and_then(Machine::tape_block),
            Some(0)
        );
        let before = session.host_mut().framebuffer().to_vec();
        for _ in 0..5 {
            session.tick_frame();
        }
        assert_ne!(
            session.host_mut().framebuffer(),
            before.as_slice(),
            "framebuffer should update"
        );
        assert!(session.host_mut().machine().unwrap().cpu().t > 0);
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
            saw_machine = app.session.host_mut().status().contains("Loaded")
                || app.session.host_mut().status().contains("Missing")
                || app.session.host_mut().status().contains("ROM");
        });
        assert!(saw_file);
        assert!(saw_machine, "status={}", app.session.host_mut().status());
        // Second frame after menus exist — menu strip must reserve clickable height.
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.ui(ctx);
            assert!(
                theme::menu_bar_min_height() >= 28.0,
                "menu bar must be tall enough to click"
            );
        });
        assert_eq!(app.session.host_mut().framebuffer().len(), 352 * 296 * 4);
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
        {
            let host = &mut *app.session.host_mut();
            if let Some(m) = host.machine_mut() {
                let pc = m.cpu().regs.pc;
                m.step_once();
                assert_ne!(m.cpu().regs.pc, pc, "step_once should advance PC");
            }
        }
    }

    #[test]
    fn prefs_restore_model_tape_volume_on_launch() {
        use spec_chum_host::{save_prefs, PrefJoystick, PrefModel, UiPreferences};
        use std::time::{SystemTime, UNIX_EPOCH};

        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        let path = std::env::temp_dir().join(format!("spec-chum-app-prefs-{nanos}.json"));
        let prefs = UiPreferences {
            model: PrefModel::Spectrum128,
            tape_experience: true,
            volume: 0.55,
            muted: true,
            throttle: false,
            kempston_mouse: true,
            joystick_mode: PrefJoystick::Cursor,
            ..UiPreferences::default()
        };
        save_prefs(&path, &prefs).expect("save prefs");

        let mut app = SpecChumApp::new_with_audio_prefs(false, path.clone());
        let _ = std::fs::remove_file(&path);

        assert_eq!(app.session.model(), Model::Spectrum128);
        assert!((app.session.volume - 0.55).abs() < f32::EPSILON);
        assert!(app.session.muted);
        assert!(!app.session.throttle);
        assert!(app.session.kempston_mouse);
        assert_eq!(app.session.host_mut().joystick_mode(), JoystickMode::Cursor);
        {
            let host = &*app.session.host_mut();
            if let Some(m) = host.machine() {
                let opts = m.tape_load_options();
                assert!(opts.experience_load);
                assert!(!opts.flash_load);
            }
        }
    }

    #[test]
    fn emulator_session_uses_host_session() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        assert_eq!(session.host_mut().model(), ModelId::Spectrum48);
        assert!(session.host_mut().running());
        let _ = session.host_mut().inspect_text(); // HostSession debug API available
    }

    #[test]
    fn gui_and_control_plane_share_live_session() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if !session.host_mut().has_machine() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        let plane = ControlPlane::from_shared(session.share_host());
        session.host_mut().set_running(false);
        let after = {
            let host = &mut *session.host_mut();
            let m = host.machine_mut().expect("machine");
            let before = m.cpu().regs.pc;
            m.step_once();
            let after = m.cpu().regs.pc;
            assert_ne!(after, before, "GUI step should advance PC");
            after
        };
        let inspect = plane.inspect_json().expect("inspect");
        assert!(
            inspect.contains(&format!("\"pc\":{after}"))
                || inspect.contains(&format!("\"pc\": {after}")),
            "control_plane inspect must see GUI PC={after}: {inspect}"
        );
        plane.with_host_mut(|host| {
            if let Some(m) = host.machine_mut() {
                m.step_once();
            }
        });
        let gui_pc = {
            let host = &*session.host_mut();
            host.machine().map(|m| m.cpu().regs.pc).expect("machine")
        };
        let plane_pc =
            plane.with_host_ref(|host| host.machine().map(|m| m.cpu().regs.pc).expect("machine"));
        assert_eq!(
            gui_pc, plane_pc,
            "GUI and ControlPlane must share one machine"
        );
    }
}
