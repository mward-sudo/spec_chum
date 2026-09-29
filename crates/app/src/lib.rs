//! Spec Chum — egui frontend library (testable without a display).

mod display;
mod frontend;
mod keymap;
mod theme;
mod window_capture;

pub use frontend::SpecChumApp;
pub use keymap::MAPPING_DOC;

use std::ops::{Deref, DerefMut};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use control_plane::ControlPlane;
use eframe::egui;
use machine::{AyStereoMode, JoystickMode, JoystickState, Machine, Model, TapeLoadOptions};
use parking_lot::{Mutex as ParkingMutex, MutexGuard};
use spec_chum_host::{
    apply_user_config, slot_rom_overrides_for_model, HostSession, ModelId, PrefModel,
    UiPreferences, UserMachineConfig,
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
                let built = Machine::from_rom_with_overrides(model, &data, &overrides);
                match built {
                    Ok(m) => {
                        self.host_mut().set_machine(m);
                        self.host_mut()
                            .set_status(format!("Loaded {}", path.display()));
                    }
                    Err(e) => self.host_mut().set_status(e.to_string()),
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

    fn ensure_snapshot_model(&mut self, model: Model) {
        if !self.host_mut().has_machine() || self.model() != model {
            self.set_model(model);
            self.host_mut().clear_machine();
            self.try_autoload_rom();
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
            self.ensure_snapshot_model(target);
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
                self.ensure_snapshot_model(Model::Spectrum48);
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
        let data = match std::fs::read(path) {
            Ok(d) => d,
            Err(e) => {
                self.host_mut().set_status(format!("TAP error: {e}"));
                return;
            }
        };
        match tape::TapImage::parse(&data) {
            Ok(img) => {
                let host = &mut *self.host_mut();
                if !host.has_machine() {
                    host.set_status("Load a machine ROM before inserting tape");
                    return;
                }
                if let Some(m) = host.machine_mut() {
                    m.insert_tape(tape::TapPlayer::new(img));
                    Self::clear_flash_load_if_active(m);
                }
                host.set_media_identity_from_bytes(&data, path);
                let title = host
                    .media_title()
                    .unwrap_or_else(|| path.to_str().unwrap_or("tape"))
                    .to_owned();
                host.set_status(format!(
                    "Inserted TAP {title} (paused — Tape → Play for EAR, or Instant)"
                ));
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
                        host.set_media_identity_from_bytes(&data, path);
                    }
                    self.force_flash_load(false);
                    let title = self
                        .host_mut()
                        .media_title()
                        .unwrap_or_else(|| path.to_str().unwrap_or("tape"))
                        .to_owned();
                    self.host_mut().set_status(format!(
                        "Inserted TZX {title} as TAP ({n} blocks, paused). Type LOAD \"\" then Play (EAR), or Instant. 128K/+3: Type LOAD enters 48 BASIC (+3 disk Loader is not tape). +2A: Type LOAD uses menu Loader (tape)."
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
                    host.set_media_identity_from_bytes(&data, path);
                }
                self.force_flash_load(false);
                let title = self
                    .host_mut()
                    .media_title()
                    .unwrap_or_else(|| path.to_str().unwrap_or("tape"))
                    .to_owned();
                self.host_mut().set_status(format!(
                    "Inserted TZX {title} (pulse playback, paused — Play when loader is ready)"
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
            Self::clear_flash_load_if_active(m);
            host.set_status("Tape paused");
        }
    }

    pub fn rewind_tape(&mut self) {
        let host = &mut *self.host_mut();
        if let Some(m) = host.machine_mut() {
            m.rewind_tape();
            Self::clear_flash_load_if_active(m);
            host.set_status("Tape rewound (paused)");
        }
    }

    fn clear_flash_load_if_active(machine: &mut Machine) {
        let mut options = machine.tape_load_options();
        if options.flash_load || options.experience_load {
            options.flash_load = false;
            machine.set_tape_load_options(options);
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
    ///
    /// Decks with no LD-BYTES trap to poke (pulse TZX) load off EAR at
    /// [`machine::INSTANT_EAR_FALLBACK_SPEED`] — still fast, and the status says
    /// so rather than claiming a flash-load that cannot happen (#390).
    pub fn instant_load_tape(&mut self) {
        let check = {
            let host = &*self.host_mut();
            match host.machine() {
                None => None,
                Some(m) if !m.has_tape() => Some(None),
                Some(m) => Some(Some((
                    m.cpu().regs.pc == tape::LD_BYTES_TRAP_PC,
                    m.tape_supports_flash_load(),
                ))),
            }
        };
        let (at_ld_bytes, can_flash) = match check {
            None => {
                self.host_mut().set_status("Instant: no machine");
                return;
            }
            Some(None) => {
                self.host_mut().set_status("Instant: insert a tape first");
                return;
            }
            Some(Some(state)) => state,
        };
        self.force_flash_load(true);
        if at_ld_bytes {
            self.pending_instant_play = false;
            self.play_tape_keeping_options();
            self.host_mut().set_status(if can_flash {
                "Instant: flash-loading at LD-BYTES".to_owned()
            } else {
                Self::instant_ear_fallback_status()
            });
            return;
        }

        self.type_load_quotes_inner(false, true);
        self.host_mut().set_status(if can_flash {
            "Instant: typing LOAD \"\" then flash-load Play".to_owned()
        } else {
            format!(
                "Instant: typing LOAD \"\" — {}",
                Self::instant_ear_fallback_status()
            )
        });
    }

    /// Chrome for Instant on a deck the LD-BYTES trap cannot serve.
    fn instant_ear_fallback_status() -> String {
        format!(
            "custom loader (no flash trap) — EAR at {}×",
            machine::INSTANT_EAR_FALLBACK_SPEED
        )
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
        let model = self.model();
        let is_48k_class = model.is_48k_class();
        self.key_script = Some(if is_48k_class {
            KeyScript::load_quotes_48k_inner(with_code)
        } else if model == Model::SpectrumPlus2A {
            KeyScript::load_quotes_plus2a(with_code)
        } else {
            KeyScript::load_quotes_128_or_plus3(with_code)
        });
        if pending_play {
            return;
        }
        let msg = match (is_48k_class, model, with_code) {
            (true, _, true) => {
                "Typing LOAD \"\" CODE — press Tape → Play when border goes red/cyan"
            }
            (true, _, false) => {
                "Typing LOAD \"\" — press Tape → Play when the border goes red/cyan"
            }
            (false, Model::SpectrumPlus2A, false) => {
                "Selecting +2A tape Loader — press Tape → Play when border goes red/cyan"
            }
            (false, _, true) => {
                "Typing 48 BASIC LOAD \"\" CODE — press Tape → Play when border goes red/cyan"
            }
            (false, _, false) => {
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
        let can_flash = self
            .host_mut()
            .machine()
            .is_some_and(Machine::tape_supports_flash_load);
        self.host_mut().set_status(if can_flash {
            "Instant: flash-loading after LOAD \"\"".to_owned()
        } else {
            format!("Instant: {}", Self::instant_ear_fallback_status())
        });
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
                        Err(e) => host.set_status(e.to_string()),
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
                    let label = match m.model() {
                        Model::Spectrum128
                        | Model::SpectrumPlus2
                        | Model::Pentagon128
                        | Model::ScorpionZs256 => "Multiface 128",
                        _ => "Multiface 1",
                    };
                    match m.attach_multiface(&data) {
                        Ok(()) => {
                            host.set_status(format!("Attached {label} {}", path.display()));
                        }
                        Err(e) => host.set_status(e.to_string()),
                    }
                } else {
                    host.set_status("Load a ROM before Multiface");
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
                    Err(e) => host.set_status(e.to_string()),
                }
            } else {
                host.set_status("Load a machine ROM first");
            }
        }
    }

    pub fn attach_divmmc_sd(&mut self, path: &Path) {
        self.attach_divmmc_sd_slot(path, 0);
    }

    pub fn attach_divmmc_sd_slot(&mut self, path: &Path, slot: u8) {
        match std::fs::read(path) {
            Ok(data) => {
                let host = &mut *self.host_mut();
                if let Some(m) = host.machine_mut() {
                    match m.attach_divmmc_sd_slot(slot, data) {
                        Ok(()) => {
                            host.set_status(format!("DivMMC SD slot {slot} {}", path.display()));
                        }
                        Err(e) => host.set_status(e.to_string()),
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
                        Err(e) => host.set_status(e.to_string()),
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
                    Err(e) => host.set_status(e.to_string()),
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
                        Err(e) => host.set_status(e.to_string()),
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
                        Err(e) => host.set_status(e.to_string()),
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
    /// `pad` is combined with arrow/`Tab` keyboard stick. Joystick modes that touch the
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn machine_build_error_from_rom_read_displays() {
        let err = machine::MachineBuildError::from(machine::RomReadError::TrdosNotApplicable {
            model: "48K".into(),
        });
        assert!(
            err.to_string().contains("does not use a TR-DOS ROM"),
            "got {err}"
        );
    }

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

    /// +3 Type LOAD must queue keys (not a disk-Loader hint) and flash-load `attr_mark` (#144).
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
            .map_or(0, |d| d.as_nanos());
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
