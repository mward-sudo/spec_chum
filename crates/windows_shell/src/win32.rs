//! Classic Win32 message-loop host (#351 — Hardware / Settings / Debug deepen).

#![allow(unsafe_code)] // Win32 window-proc / GDI / USERDATA — SAFETY at each site.

use std::mem::{size_of, zeroed};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use machine::TapeLoadOptions;
use parking_lot::Mutex;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, InvalidateRect, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT};
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DispatchMessageW, GetClientRect, GetWindowLongPtrW,
    LoadCursorW, MessageBoxW, PeekMessageW, PostQuitMessage, RegisterClassExW, SetWindowLongPtrW,
    SetWindowTextW, ShowWindow, TranslateMessage, CREATESTRUCTW, CS_HREDRAW, CS_OWNDC, CS_VREDRAW,
    CW_USEDEFAULT, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, GWLP_USERDATA, HMENU, IDC_ARROW,
    MB_ICONERROR, MB_OK, MSG, PM_REMOVE, SW_SHOW, WINDOW_EX_STYLE, WM_COMMAND, WM_CREATE,
    WM_DESTROY, WM_KEYDOWN, WM_KEYUP, WM_PAINT, WM_QUIT, WM_SIZE, WM_SYSKEYDOWN, WM_SYSKEYUP,
    WNDCLASSEXW, WS_BORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_VISIBLE, WS_VSCROLL,
};

use control_plane::ControlPlane;
use spec_chum_host::{
    default_prefs_path, load_prefs, save_prefs, sync_model_rom_paths, HostError, HostSession,
    ModelId, PrefAyStereo, PrefModel, UiPreferences,
};

use windows_shell::audio::{self, PcmRing};
use windows_shell::commands::{
    ear_speed_from_menu_id, joystick_from_menu_id, menu_id_for_model, model_from_menu_id,
    model_menu_label, IDM_DBG_BREAK_PC, IDM_DBG_CLEAR_BREAKS, IDM_DBG_CONTINUE, IDM_DBG_PAUSE,
    IDM_DBG_REFRESH, IDM_DBG_STEP, IDM_DBG_TOGGLE, IDM_FILE_EXIT, IDM_FILE_OPEN_DSK,
    IDM_FILE_OPEN_RZX, IDM_FILE_OPEN_SNAPSHOT, IDM_FILE_OPEN_TAPE, IDM_FILE_OPEN_TRD,
    IDM_HW_ATTACH_BETA, IDM_HW_ATTACH_DIVMMC, IDM_HW_ATTACH_IF1, IDM_HW_ATTACH_MULTIFACE,
    IDM_HW_DIVMMC_EEPROM, IDM_HW_DIVMMC_SD, IDM_HW_DIVMMC_SD_SLOT1, IDM_HW_EJECT_DCK,
    IDM_HW_INSERT_DCK, IDM_HW_INSERT_MDR, IDM_HW_LOAD_TRDOS_ROM, IDM_HW_MULTIFACE_NMI,
    IDM_HW_OPEN_TRD, IDM_MACHINE_RESET, IDM_SET_AY_ABC, IDM_SET_AY_ACB, IDM_SET_AY_MONO,
    IDM_SET_JOY_CURSOR, IDM_SET_JOY_KEMPSTON, IDM_SET_JOY_SINCLAIR_L, IDM_SET_JOY_SINCLAIR_R,
    IDM_SET_KEMPSTON_MOUSE, IDM_SET_MUTE, IDM_SET_ONLINE_TITLES, IDM_SET_TAPE_EAR_1,
    IDM_SET_TAPE_EAR_10, IDM_SET_TAPE_EAR_2, IDM_SET_TAPE_EAR_20, IDM_SET_TAPE_EAR_5,
    IDM_SET_TAPE_EXPERIENCE, IDM_SET_TAPE_INSTANT, IDM_SET_THROTTLE, IDM_TAPE_PAUSE, IDM_TAPE_PLAY,
    IDM_TAPE_REWIND,
};
use windows_shell::keymap::{
    self, apply_chord, apply_modifiers, chord_for_vk, suppresses_modifier_caps,
};

const CLASS_NAME: &str = "SpecChumWindowsShell\0";
const DEBUG_CLASS: &str = "SpecChumWindowsDebug\0";
const WINDOW_TITLE: &str = "Spec Chum\0";
const ID_DBG_EDIT: i32 = 2001;

/// Live host held either exclusively or shared with embedded Agent HTTP.
#[derive(Debug)]
enum HostSlot {
    Direct(Box<HostSession>),
    Shared(Arc<Mutex<HostSession>>),
}

impl HostSlot {
    fn with_mut<R>(&mut self, f: impl FnOnce(&mut HostSession) -> R) -> R {
        match self {
            Self::Direct(s) => f(s),
            Self::Shared(a) => f(&mut a.lock()),
        }
    }
}

struct AppState {
    host: HostSlot,
    pcm: Arc<Mutex<PcmRing>>,
    _stream: Option<cpal::Stream>,
    _agent: Option<agent_server::embedded::EmbeddedServer>,
    plane: Option<Arc<ControlPlane>>,
    prefs: UiPreferences,
    prefs_path: PathBuf,
    /// Last key VKs we injected (for release / modifier refresh).
    held: Vec<u16>,
    bgra: Vec<u8>,
    status: String,
    last_frame: Instant,
    /// Menu / Ctrl+O actions queued from `wnd_proc` so modal `rfd` dialogs
    /// never run while `&mut AppState` is borrowed inside the window procedure.
    pending_cmd: Option<usize>,
    debug_hwnd: Option<HWND>,
    debug_edit: Option<HWND>,
    main_hwnd: Option<HWND>,
    /// Last time the debugger EDIT control was rewritten from `tick_frame`.
    last_debug_refresh: Instant,
}

impl AppState {
    fn new() -> Result<Self> {
        let prefs_path = default_prefs_path();
        let mut prefs = load_prefs(&prefs_path);
        sync_model_rom_paths(prefs.model_rom_paths.clone());

        let model = prefs.model.to_model_id();
        let mut session = HostSession::new(model, true);
        // `set_model` only switches the preferred model; `select_model` also
        // searches `SPEC_CHUM_ROOT` / cwd / exe-dir for `roms/spec48.rom` etc.
        let _ = session.select_model(model);
        if !session.has_machine() {
            // Fall back to 48K if preferred model ROM is missing.
            let _ = session.select_model(ModelId::Spectrum48);
            prefs.select_builtin_model(PrefModel::Spectrum48);
        }
        if !session.has_machine() {
            anyhow::bail!("48K ROM not found — run ./scripts/fetch_roms.sh from the repo root");
        }

        apply_session_prefs(&mut session, &prefs).context("apply session prefs")?;

        let pcm = Arc::new(Mutex::new(PcmRing::new()));
        {
            let mut ring = pcm.lock();
            ring.volume = prefs.volume;
            ring.muted = prefs.muted;
        }
        let stream = audio::start_stream(Arc::clone(&pcm));

        let (host, plane, agent) = if std::env::var("SPEC_CHUM_AGENT").ok().as_deref() == Some("1")
        {
            let shared = Arc::new(Mutex::new(session));
            let plane = Arc::new(ControlPlane::from_shared(Arc::clone(&shared)));
            let agent = match agent_server::embedded::spawn_from_env_with_plane(Arc::clone(&plane))
            {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("spec-chum-windows: embedded agent failed: {e}");
                    None
                }
            };
            (HostSlot::Shared(shared), Some(plane), agent)
        } else {
            (HostSlot::Direct(Box::new(session)), None, None)
        };

        Ok(Self {
            host,
            pcm,
            _stream: stream,
            _agent: agent,
            plane,
            prefs,
            prefs_path,
            held: Vec::new(),
            bgra: Vec::new(),
            status: "Ready".into(),
            last_frame: Instant::now(),
            pending_cmd: None,
            debug_hwnd: None,
            debug_edit: None,
            main_hwnd: None,
            last_debug_refresh: Instant::now(),
        })
    }

    fn persist_prefs(&self) {
        if let Err(e) = save_prefs(&self.prefs_path, &self.prefs) {
            eprintln!("spec-chum-windows: save prefs failed: {e}");
        }
    }

    fn apply_prefs_to_host(&mut self) -> Result<(), HostError> {
        let prefs = self.prefs.clone();
        self.host.with_mut(|s| apply_session_prefs(s, &prefs))?;
        let mut ring = self.pcm.lock();
        ring.volume = prefs.volume;
        ring.muted = prefs.muted;
        Ok(())
    }

    fn tick_frame(&mut self, hwnd: HWND) {
        let min_dt = if self.prefs.throttle {
            Duration::from_millis(20)
        } else {
            Duration::from_millis(0)
        };
        if min_dt > Duration::ZERO && self.last_frame.elapsed() < min_dt {
            return;
        }
        self.last_frame = Instant::now();

        let (pcm_snap, title, w, h, rgba) = self.host.with_mut(|s| {
            let _ = s.run_frame();
            let pcm = s.audio_pcm().to_vec();
            let status = s.status().to_owned();
            let title = match s.media_title() {
                Some(t) => format!("Spec Chum — {t}"),
                None => format!("Spec Chum — {status}"),
            };
            let w = s.width();
            let h = s.height();
            let rgba = s.framebuffer().to_vec();
            (pcm, title, w, h, rgba)
        });

        self.pcm.lock().push_frame(&pcm_snap);
        self.status = title.clone();
        self.sync_bgra(&rgba, w, h);
        set_window_title(hwnd, &title);
        // SAFETY: hwnd is our window; invalidate full client for next paint.
        unsafe {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        self.maybe_refresh_debug_text();
    }

    fn sync_bgra(&mut self, rgba: &[u8], w: usize, h: usize) {
        let need = w.saturating_mul(h).saturating_mul(4);
        if self.bgra.len() != need {
            self.bgra.resize(need, 0);
        }
        for (dst, src) in self.bgra.chunks_exact_mut(4).zip(rgba.chunks_exact(4)) {
            dst[0] = src[2]; // B
            dst[1] = src[1]; // G
            dst[2] = src[0]; // R
            dst[3] = 0;
        }
    }

    fn paint(&mut self, hwnd: HWND) {
        // SAFETY: always pair BeginPaint/EndPaint for WM_PAINT, even when we skip blit.
        unsafe {
            let mut ps = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut ps);
            let (w, h) = self.host.with_mut(|s| (s.width(), s.height()));
            if w > 0 && h > 0 && self.bgra.len() >= w * h * 4 {
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let cw = (client.right - client.left).max(1);
                let ch = (client.bottom - client.top).max(1);

                let info = BITMAPINFO {
                    bmiHeader: BITMAPINFOHEADER {
                        biSize: size_of::<BITMAPINFOHEADER>() as u32,
                        biWidth: w as i32,
                        biHeight: -(h as i32), // top-down
                        biPlanes: 1,
                        biBitCount: 32,
                        biCompression: BI_RGB.0,
                        ..zeroed()
                    },
                    ..zeroed()
                };

                let _ = StretchDIBits(
                    hdc,
                    0,
                    0,
                    cw,
                    ch,
                    0,
                    0,
                    w as i32,
                    h as i32,
                    Some(self.bgra.as_ptr().cast()),
                    &info,
                    DIB_RGB_COLORS,
                    SRCCOPY,
                );
            }
            let _ = EndPaint(hwnd, &ps);
        }
    }

    fn set_key_matrix(&mut self, row: usize, bit: u8, pressed: bool) {
        let _ = self.host.with_mut(|s| s.set_key(row, bit, pressed));
    }

    fn refresh_input(&mut self) {
        let shift = key_down(VK_SHIFT.0);
        let ctrl = key_down(VK_CONTROL.0);
        let alt = key_down(VK_MENU.0);

        let _ = self.host.with_mut(HostSession::clear_keys);

        let mut suppress = false;
        let held = self.held.clone();
        for &vk in &held {
            if suppresses_modifier_caps(vk) {
                suppress = true;
            }
        }
        {
            let mut apply = |row, bit, pressed| self.set_key_matrix(row, bit, pressed);
            apply_modifiers(&mut apply, shift, alt, ctrl, suppress);
        }
        for &vk in &held {
            if let Some(ch) = chord_for_vk(vk, shift) {
                let mut apply = |row, bit, pressed| self.set_key_matrix(row, bit, pressed);
                apply_chord(&mut apply, &ch, true);
            }
        }
    }

    fn on_key(&mut self, vk: u16, pressed: bool) {
        if matches!(
            vk,
            keymap::vk::SHIFT | keymap::vk::CONTROL | keymap::vk::MENU
        ) {
            self.refresh_input();
            return;
        }
        if pressed {
            if !self.held.contains(&vk) {
                self.held.push(vk);
            }
        } else {
            self.held.retain(|&k| k != vk);
        }
        self.refresh_input();
    }

    fn report_err(&mut self, title: &str, err: &HostError) {
        let msg = format!("{title}: {err}");
        self.host.with_mut(|s| s.set_status(msg.clone()));
        self.status = msg.clone();
        message_box(self.main_hwnd, title, &msg);
    }

    fn open_path(
        &mut self,
        title: &str,
        filter_name: &str,
        exts: &[&str],
        load: impl FnOnce(&mut HostSession, &Path) -> Result<(), HostError>,
    ) {
        let path = rfd::FileDialog::new()
            .add_filter(filter_name, exts)
            .set_title(title)
            .pick_file();
        let Some(path) = path else { return };
        match self.host.with_mut(|s| load(s, &path)) {
            Ok(()) => {
                self.prefs.push_recent(&path);
                self.persist_prefs();
            }
            Err(e) => self.report_err(title, &e),
        }
    }

    fn open_tape(&mut self) {
        self.open_path("Open Tape", "Tape", &["tap", "tzx"], HostSession::open_tape);
    }

    fn open_snapshot(&mut self) {
        self.open_path(
            "Open Snapshot",
            "Snapshot",
            &["sna", "z80", "szx"],
            HostSession::load_snapshot,
        );
    }

    fn open_rzx(&mut self) {
        self.open_path("Open RZX", "RZX", &["rzx"], HostSession::load_rzx);
    }

    fn open_dsk(&mut self) {
        self.open_path("Open Disk (DSK)", "Disk", &["dsk"], HostSession::load_dsk);
    }

    fn open_trd(&mut self) {
        self.open_path(
            "Open TR-DOS disk (TRD)",
            "TRD",
            &["trd"],
            HostSession::load_trd,
        );
    }

    fn select_model(&mut self, model: ModelId) {
        let mut next = self.prefs.clone();
        next.select_builtin_model(PrefModel::from_model_id(model));
        sync_model_rom_paths(next.model_rom_paths.clone());
        let result = self.host.with_mut(|s| {
            s.select_model(model)?;
            apply_session_prefs(s, &next)?;
            Ok::<(), HostError>(())
        });
        match result {
            Ok(()) => {
                self.prefs = next;
                self.persist_prefs();
            }
            Err(e) => self.report_err("Select model", &e),
        }
    }

    fn reset_machine(&mut self) {
        if let Err(e) = self.host.with_mut(HostSession::reset) {
            self.report_err("Reset", &e);
        }
    }

    fn set_joystick(&mut self, joy: spec_chum_host::PrefJoystick) {
        let previous = self.prefs.clone();
        self.prefs.set_joystick(joy.to_mode());
        if let Err(e) = self.apply_prefs_to_host() {
            self.prefs = previous;
            self.report_err("Joystick", &e);
            return;
        }
        self.persist_prefs();
    }

    fn set_tape_ear_speed(&mut self, speed: u32) {
        let previous = self.prefs.clone();
        self.prefs.tape_experience = false;
        self.prefs.tape_ear_speed = speed;
        if let Err(e) = self.apply_prefs_to_host() {
            self.prefs = previous;
            self.report_err("Tape EAR speed", &e);
            return;
        }
        self.persist_prefs();
    }

    fn set_tape_experience(&mut self) {
        let previous = self.prefs.clone();
        self.prefs.tape_experience = true;
        if let Err(e) = self.apply_prefs_to_host() {
            self.prefs = previous;
            self.report_err("Experience load", &e);
            return;
        }
        self.persist_prefs();
    }

    fn set_tape_instant(&mut self) {
        // Instant is session-only (never sticky in prefs — same as egui).
        let opts = TapeLoadOptions {
            flash_load: true,
            ..TapeLoadOptions::default()
        };
        if let Err(e) = self.host.with_mut(|s| s.set_tape_load_options(opts)) {
            self.report_err("Instant load", &e);
        }
    }

    fn toggle_mute(&mut self) {
        self.prefs.muted = !self.prefs.muted;
        self.pcm.lock().muted = self.prefs.muted;
        self.persist_prefs();
    }

    fn toggle_throttle(&mut self) {
        self.prefs.throttle = !self.prefs.throttle;
        self.persist_prefs();
    }

    fn toggle_online_titles(&mut self) {
        self.prefs.online_tape_titles = !self.prefs.online_tape_titles;
        let on = self.prefs.online_tape_titles;
        self.host.with_mut(|s| s.set_online_tape_titles(on));
        self.persist_prefs();
    }

    fn toggle_kempston_mouse(&mut self) {
        self.prefs.kempston_mouse = !self.prefs.kempston_mouse;
        // Mouse attach is driven by host input; persist preference for parity.
        self.persist_prefs();
    }

    fn set_ay(&mut self, mode: PrefAyStereo) {
        let previous = self.prefs.clone();
        self.prefs.set_ay_stereo(mode.to_mode());
        if let Err(e) = self.apply_prefs_to_host() {
            self.prefs = previous;
            self.report_err("AY stereo", &e);
            return;
        }
        self.persist_prefs();
    }

    fn host_action(
        &mut self,
        title: &str,
        f: impl FnOnce(&mut HostSession) -> Result<(), HostError>,
    ) {
        if let Err(e) = self.host.with_mut(f) {
            self.report_err(title, &e);
        }
    }

    fn toggle_debug_window(&mut self) {
        if let Some(hwnd) = self.debug_hwnd.take() {
            self.debug_edit = None;
            // SAFETY: hwnd is the debug window we created; DestroyWindow posts WM_DESTROY.
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd);
            }
            return;
        }
        let self_ptr = self as *mut AppState;
        match create_debug_window(self.main_hwnd, self_ptr) {
            Ok((hwnd, edit)) => {
                self.debug_hwnd = Some(hwnd);
                self.debug_edit = Some(edit);
                self.refresh_debug_text();
            }
            Err(e) => {
                eprintln!("spec-chum-windows: debug window failed: {e}");
            }
        }
    }

    fn maybe_refresh_debug_text(&mut self) {
        if self.debug_edit.is_none() {
            return;
        }
        // Avoid rewriting the EDIT control every Spectrum frame.
        const MIN_INTERVAL: Duration = Duration::from_millis(250);
        if self.last_debug_refresh.elapsed() < MIN_INTERVAL {
            return;
        }
        self.refresh_debug_text();
    }

    fn refresh_debug_text(&mut self) {
        let Some(edit) = self.debug_edit else {
            return;
        };
        let text = self.host.with_mut(|s| {
            let inspect = s.inspect_text().unwrap_or_else(|e| format!("inspect: {e}"));
            let disasm = s.disasm(None, 16).unwrap_or_default();
            let breaks = s
                .list_pc_breakpoints()
                .unwrap_or_default()
                .into_iter()
                .map(|pc| format!("${pc:04X}"))
                .collect::<Vec<_>>()
                .join(" ");
            let paused = if s.paused() { "paused" } else { "running" };
            format!("{inspect}\n\n--- disasm ({paused}) ---\n{disasm}\n\nbreakpoints: {breaks}\n")
        });
        // Win32 multiline EDIT expects CRLF line endings.
        let text = text.replace("\r\n", "\n").replace('\n', "\r\n");
        set_window_title(edit, &text);
        self.last_debug_refresh = Instant::now();
    }

    fn debug_pause(&mut self) {
        self.host.with_mut(|s| {
            s.set_paused(true);
            s.set_status("Paused");
        });
        self.refresh_debug_text();
    }

    fn debug_continue(&mut self) {
        self.host_action("Continue", |s| {
            s.continue_execution()?;
            s.set_running(true);
            Ok(())
        });
        self.refresh_debug_text();
    }

    fn debug_step(&mut self) {
        self.host_action("Step", HostSession::debug_step);
        self.refresh_debug_text();
    }

    fn debug_break_at_pc(&mut self) {
        let result = self.host.with_mut(|s| {
            let pc = s.regs()?.pc;
            s.add_breakpoint(pc)?;
            Ok::<u16, HostError>(pc)
        });
        match result {
            Ok(pc) => {
                self.host
                    .with_mut(|s| s.set_status(format!("Breakpoint at ${pc:04X}")));
                self.refresh_debug_text();
            }
            Err(e) => self.report_err("Breakpoint", &e),
        }
    }

    fn debug_clear_breaks(&mut self) {
        self.host_action("Clear breakpoints", HostSession::clear_breakpoints);
        self.refresh_debug_text();
    }

    fn queue_command(&mut self, id: usize) {
        if id == IDM_FILE_EXIT {
            unsafe {
                PostQuitMessage(0);
            }
            return;
        }
        self.pending_cmd = Some(id);
    }

    fn drain_pending_command(&mut self) {
        let Some(id) = self.pending_cmd.take() else {
            return;
        };
        match id {
            IDM_FILE_OPEN_TAPE => self.open_tape(),
            IDM_FILE_OPEN_SNAPSHOT => self.open_snapshot(),
            IDM_FILE_OPEN_RZX => self.open_rzx(),
            IDM_FILE_OPEN_DSK => self.open_dsk(),
            IDM_FILE_OPEN_TRD => self.open_trd(),
            IDM_TAPE_PLAY => self.host_action("Play tape", HostSession::play_tape),
            IDM_TAPE_PAUSE => self.host_action("Pause tape", HostSession::pause_tape),
            IDM_TAPE_REWIND => self.host_action("Rewind tape", HostSession::rewind_tape),
            IDM_MACHINE_RESET => self.reset_machine(),
            id if model_from_menu_id(id).is_some() => {
                if let Some(m) = model_from_menu_id(id) {
                    self.select_model(m);
                }
            }
            IDM_HW_ATTACH_MULTIFACE => self.open_path(
                "Attach Multiface ROM",
                "Multiface ROM",
                &["rom", "bin"],
                HostSession::attach_multiface,
            ),
            IDM_HW_MULTIFACE_NMI => self.host_action("Multiface NMI", HostSession::multiface_nmi),
            IDM_HW_ATTACH_DIVMMC => self.host_action("Attach DivMMC", HostSession::attach_divmmc),
            IDM_HW_DIVMMC_SD => self.open_path(
                "Open DivMMC SD",
                "SD image",
                &["img", "bin", "mmc", "sd"],
                HostSession::load_divmmc_sd,
            ),
            IDM_HW_DIVMMC_SD_SLOT1 => self.open_path(
                "Open DivMMC SD (slot 1)",
                "SD image",
                &["img", "bin", "mmc", "sd"],
                |s, p| s.load_divmmc_sd_slot(p, 1),
            ),
            IDM_HW_DIVMMC_EEPROM => self.open_path(
                "Open DivMMC EEPROM",
                "EEPROM / ESXDOS",
                &["rom", "bin", "eeprom"],
                HostSession::load_divmmc_eeprom,
            ),
            IDM_HW_ATTACH_IF1 => {
                self.host_action("Attach Interface 1", HostSession::attach_interface1)
            }
            IDM_HW_INSERT_MDR => self.open_path(
                "Open Microdrive MDR",
                "MDR",
                &["mdr"],
                HostSession::insert_mdr,
            ),
            IDM_HW_ATTACH_BETA => self.host_action("Attach Beta Disk", HostSession::attach_beta),
            IDM_HW_LOAD_TRDOS_ROM => self.open_path(
                "Load TR-DOS ROM",
                "TR-DOS ROM",
                &["rom", "bin"],
                HostSession::load_trdos_rom,
            ),
            IDM_HW_OPEN_TRD => self.open_trd(),
            IDM_HW_INSERT_DCK => self.open_path(
                "Insert Timex Dock",
                "Timex dock",
                &["dck"],
                HostSession::insert_dck,
            ),
            IDM_HW_EJECT_DCK => self.host_action("Eject Timex Dock", HostSession::eject_dck),
            id if joystick_from_menu_id(id).is_some() => {
                if let Some(j) = joystick_from_menu_id(id) {
                    self.set_joystick(j);
                }
            }
            id if ear_speed_from_menu_id(id).is_some() => {
                if let Some(sp) = ear_speed_from_menu_id(id) {
                    self.set_tape_ear_speed(sp);
                }
            }
            IDM_SET_TAPE_EXPERIENCE => self.set_tape_experience(),
            IDM_SET_TAPE_INSTANT => self.set_tape_instant(),
            IDM_SET_MUTE => self.toggle_mute(),
            IDM_SET_THROTTLE => self.toggle_throttle(),
            IDM_SET_ONLINE_TITLES => self.toggle_online_titles(),
            IDM_SET_KEMPSTON_MOUSE => self.toggle_kempston_mouse(),
            IDM_SET_AY_MONO => self.set_ay(PrefAyStereo::Mono),
            IDM_SET_AY_ACB => self.set_ay(PrefAyStereo::Acb),
            IDM_SET_AY_ABC => self.set_ay(PrefAyStereo::Abc),
            IDM_DBG_TOGGLE => self.toggle_debug_window(),
            IDM_DBG_PAUSE => self.debug_pause(),
            IDM_DBG_CONTINUE => self.debug_continue(),
            IDM_DBG_STEP => self.debug_step(),
            IDM_DBG_BREAK_PC => self.debug_break_at_pc(),
            IDM_DBG_CLEAR_BREAKS => self.debug_clear_breaks(),
            IDM_DBG_REFRESH => self.refresh_debug_text(),
            _ => {}
        }
        let _ = &self.plane;
    }
}

fn apply_session_prefs(session: &mut HostSession, prefs: &UiPreferences) -> Result<(), HostError> {
    session.set_joystick_mode(prefs.joystick_mode.to_mode());
    session.set_online_tape_titles(prefs.online_tape_titles);
    session.set_tape_load_options(prefs.tape_load_options())?;
    if let Some(m) = session.machine_mut() {
        m.set_ay_stereo_mode(prefs.effective_ay_stereo());
    }
    Ok(())
}

fn key_down(vk: u16) -> bool {
    // SAFETY: GetKeyState is a pure input query.
    unsafe { GetKeyState(i32::from(vk)) < 0 }
}

fn set_window_title(hwnd: HWND, title: &str) {
    let wide: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: wide is NUL-terminated; hwnd is our window.
    unsafe {
        let _ = SetWindowTextW(hwnd, PCWSTR(wide.as_ptr()));
    }
}

fn message_box(owner: Option<HWND>, title: &str, body: &str) {
    let t: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let b: Vec<u16> = body.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: NUL-terminated UTF-16; owner may be null.
    unsafe {
        let _ = MessageBoxW(
            owner,
            PCWSTR(b.as_ptr()),
            PCWSTR(t.as_ptr()),
            MB_OK | MB_ICONERROR,
        );
    }
}

fn append_menu(menu: HMENU, id: usize, text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{AppendMenuW, MF_STRING};
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: menu created by us; text NUL-terminated.
    unsafe {
        let _ = AppendMenuW(menu, MF_STRING, id, PCWSTR(wide.as_ptr()));
    }
}

fn append_popup(parent: HMENU, popup: HMENU, label: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{AppendMenuW, MF_POPUP};
    let wide: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: owned menus; label NUL-terminated.
    unsafe {
        let _ = AppendMenuW(parent, MF_POPUP, popup.0 as usize, PCWSTR(wide.as_ptr()));
    }
}

fn append_sep(menu: HMENU) {
    use windows::Win32::UI::WindowsAndMessaging::{AppendMenuW, MF_SEPARATOR};
    // SAFETY: owned menu.
    unsafe {
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
    }
}

fn build_menu() -> Result<HMENU> {
    use windows::Win32::UI::WindowsAndMessaging::{CreateMenu, CreatePopupMenu};
    // SAFETY: CreateMenu/CreatePopupMenu return owned menus we attach to the window.
    unsafe {
        let menubar = CreateMenu()?;

        let file = CreatePopupMenu()?;
        append_menu(file, IDM_FILE_OPEN_TAPE, "Open &Tape…\tCtrl+O");
        append_menu(file, IDM_FILE_OPEN_SNAPSHOT, "Open &Snapshot…");
        append_menu(file, IDM_FILE_OPEN_RZX, "Open &RZX…");
        append_menu(file, IDM_FILE_OPEN_DSK, "Open &DSK…");
        append_menu(file, IDM_FILE_OPEN_TRD, "Open T&RD…");
        append_sep(file);
        append_menu(file, IDM_FILE_EXIT, "E&xit");
        append_popup(menubar, file, "&File");

        let machine = CreatePopupMenu()?;
        for m in ModelId::ALL {
            append_menu(machine, menu_id_for_model(m), model_menu_label(m));
        }
        append_sep(machine);
        append_menu(machine, IDM_MACHINE_RESET, "&Reset");
        append_popup(menubar, machine, "&Machine");

        let tape = CreatePopupMenu()?;
        append_menu(tape, IDM_TAPE_PLAY, "&Play");
        append_menu(tape, IDM_TAPE_PAUSE, "P&ause");
        append_menu(tape, IDM_TAPE_REWIND, "&Rewind");
        append_popup(menubar, tape, "&Tape");

        let hw = CreatePopupMenu()?;
        append_menu(hw, IDM_HW_ATTACH_MULTIFACE, "Attach &Multiface ROM…");
        append_menu(hw, IDM_HW_MULTIFACE_NMI, "Multiface &NMI");
        append_sep(hw);
        append_menu(hw, IDM_HW_ATTACH_DIVMMC, "Attach &DivMMC");
        append_menu(hw, IDM_HW_DIVMMC_SD, "Open DivMMC &SD image…");
        append_menu(hw, IDM_HW_DIVMMC_SD_SLOT1, "Open DivMMC SD (slot &1)…");
        append_menu(hw, IDM_HW_DIVMMC_EEPROM, "Open DivMMC &EEPROM…");
        append_sep(hw);
        append_menu(hw, IDM_HW_ATTACH_IF1, "Attach &Interface 1");
        append_menu(hw, IDM_HW_INSERT_MDR, "Open Microdrive &MDR…");
        append_sep(hw);
        append_menu(hw, IDM_HW_ATTACH_BETA, "Attach &Beta Disk");
        append_menu(hw, IDM_HW_LOAD_TRDOS_ROM, "Load T&R-DOS ROM…");
        append_menu(hw, IDM_HW_OPEN_TRD, "Open T&RD…");
        append_sep(hw);
        append_menu(hw, IDM_HW_INSERT_DCK, "Insert Timex &Dock…");
        append_menu(hw, IDM_HW_EJECT_DCK, "&Eject Timex Dock");
        append_popup(menubar, hw, "&Hardware");

        let settings = CreatePopupMenu()?;
        let joy = CreatePopupMenu()?;
        append_menu(joy, IDM_SET_JOY_KEMPSTON, "&Kempston");
        append_menu(joy, IDM_SET_JOY_SINCLAIR_L, "Sinclair &Left");
        append_menu(joy, IDM_SET_JOY_SINCLAIR_R, "Sinclair &Right");
        append_menu(joy, IDM_SET_JOY_CURSOR, "&Cursor");
        append_popup(settings, joy, "&Joystick");
        let tape_mode = CreatePopupMenu()?;
        append_menu(tape_mode, IDM_SET_TAPE_EAR_1, "EAR &1×");
        append_menu(tape_mode, IDM_SET_TAPE_EAR_2, "EAR &2×");
        append_menu(tape_mode, IDM_SET_TAPE_EAR_5, "EAR &5×");
        append_menu(tape_mode, IDM_SET_TAPE_EAR_10, "EAR 1&0×");
        append_menu(tape_mode, IDM_SET_TAPE_EAR_20, "EAR 2&0×");
        append_sep(tape_mode);
        append_menu(tape_mode, IDM_SET_TAPE_EXPERIENCE, "&Experience");
        append_menu(tape_mode, IDM_SET_TAPE_INSTANT, "&Instant (session)");
        append_popup(settings, tape_mode, "&Tape load");
        let ay = CreatePopupMenu()?;
        append_menu(ay, IDM_SET_AY_MONO, "&Mono");
        append_menu(ay, IDM_SET_AY_ACB, "&ACB");
        append_menu(ay, IDM_SET_AY_ABC, "A&BC");
        append_popup(settings, ay, "&AY stereo");
        append_sep(settings);
        append_menu(settings, IDM_SET_MUTE, "Toggle &Mute");
        append_menu(settings, IDM_SET_THROTTLE, "Toggle &Throttle (~50 Hz)");
        append_menu(
            settings,
            IDM_SET_ONLINE_TITLES,
            "Toggle &Online tape titles",
        );
        append_menu(
            settings,
            IDM_SET_KEMPSTON_MOUSE,
            "Toggle Kempston &Mouse pref",
        );
        append_popup(menubar, settings, "&Settings");

        let debug = CreatePopupMenu()?;
        append_menu(debug, IDM_DBG_TOGGLE, "&Inspector…");
        append_sep(debug);
        append_menu(debug, IDM_DBG_PAUSE, "&Pause");
        append_menu(debug, IDM_DBG_CONTINUE, "&Continue");
        append_menu(debug, IDM_DBG_STEP, "&Step");
        append_sep(debug);
        append_menu(debug, IDM_DBG_BREAK_PC, "Breakpoint at &PC");
        append_menu(debug, IDM_DBG_CLEAR_BREAKS, "C&lear breakpoints");
        append_menu(debug, IDM_DBG_REFRESH, "&Refresh");
        append_popup(menubar, debug, "&Debug");

        Ok(menubar)
    }
}

fn create_debug_window(owner: Option<HWND>, app: *mut AppState) -> Result<(HWND, HWND)> {
    // SAFETY: register a one-shot debug class and create an owned top-level window.
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let class_name: Vec<u16> = DEBUG_CLASS.encode_utf16().collect();
        let cursor = LoadCursorW(None, IDC_ARROW)?;
        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(debug_wnd_proc),
            hInstance: hinstance.into(),
            hCursor: cursor,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..zeroed()
        };
        let _ = RegisterClassExW(&wc);

        let title: Vec<u16> = "Spec Chum — Debugger\0".encode_utf16().collect();
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            640,
            520,
            owner,
            None,
            Some(hinstance.into()),
            Some(app.cast()),
        )?;

        let edit_class: Vec<u16> = "EDIT\0".encode_utf16().collect();
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let edit = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(edit_class.as_ptr()),
            PCWSTR::null(),
            WS_CHILD
                | WS_VISIBLE
                | WS_VSCROLL
                | WS_BORDER
                | windows::Win32::UI::WindowsAndMessaging::WINDOW_STYLE(
                    (ES_MULTILINE | ES_READONLY | ES_AUTOVSCROLL) as u32,
                ),
            0,
            0,
            client.right - client.left,
            client.bottom - client.top,
            Some(hwnd),
            Some(HMENU(ID_DBG_EDIT as isize as *mut core::ffi::c_void)),
            Some(hinstance.into()),
            None,
        )?;

        let _ = ShowWindow(hwnd, SW_SHOW);
        Ok((hwnd, edit))
    }
}

extern "system" fn debug_wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    match msg {
        WM_CREATE => {
            // SAFETY: lpCreateParams is the AppState pointer from create_debug_window.
            let cs = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            let app = cs.lpCreateParams as *mut AppState;
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, app as isize);
            }
            LRESULT(0)
        }
        WM_SIZE => {
            // SAFETY: resize the single EDIT child to the client area.
            unsafe {
                use windows::Win32::UI::WindowsAndMessaging::{GetDlgItem, MoveWindow};
                if let Ok(edit) = GetDlgItem(Some(hwnd), ID_DBG_EDIT) {
                    let mut client = RECT::default();
                    let _ = GetClientRect(hwnd, &mut client);
                    let _ = MoveWindow(
                        edit,
                        0,
                        0,
                        client.right - client.left,
                        client.bottom - client.top,
                        true,
                    );
                }
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            // SAFETY: clear AppState debug handles when the X button closes the window.
            let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut AppState;
            if !state_ptr.is_null() {
                let state = unsafe { &mut *state_ptr };
                state.debug_hwnd = None;
                state.debug_edit = None;
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

extern "system" fn wnd_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // SAFETY: USERDATA is either null or a Box<AppState> we installed in WM_CREATE.
    let state_ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut AppState;

    match msg {
        WM_CREATE => {
            // SAFETY: lpCreateParams is the Box raw pointer we passed to CreateWindowExW.
            let cs = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            let app = cs.lpCreateParams as *mut AppState;
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, app as isize);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            if !state_ptr.is_null() {
                // SAFETY: state_ptr owned by this window until WM_DESTROY.
                let state = unsafe { &mut *state_ptr };
                state.paint(hwnd);
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            if !state_ptr.is_null() {
                let id = (wparam.0 as usize) & 0xffff;
                let state = unsafe { &mut *state_ptr };
                state.queue_command(id);
            }
            LRESULT(0)
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if !state_ptr.is_null() {
                let vk = (wparam.0 & 0xffff) as u16;
                let state = unsafe { &mut *state_ptr };
                // Windows HIG: Ctrl+O opens tape (Cmd+O on macOS SpecChumMac).
                if vk == 0x4F && key_down(keymap::vk::CONTROL) {
                    state.queue_command(IDM_FILE_OPEN_TAPE);
                } else {
                    state.on_key(vk, true);
                }
            }
            LRESULT(0)
        }
        WM_KEYUP | WM_SYSKEYUP => {
            if !state_ptr.is_null() {
                let vk = (wparam.0 & 0xffff) as u16;
                let state = unsafe { &mut *state_ptr };
                state.on_key(vk, false);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            if !state_ptr.is_null() {
                // SAFETY: tear down debug HWND while AppState is still alive, then free it.
                unsafe {
                    let state = &mut *state_ptr;
                    if let Some(dbg) = state.debug_hwnd.take() {
                        state.debug_edit = None;
                        let _ = windows::Win32::UI::WindowsAndMessaging::DestroyWindow(dbg);
                    }
                    SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
                    drop(Box::from_raw(state_ptr));
                }
            }
            unsafe {
                PostQuitMessage(0);
            }
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) },
    }
}

/// Run the native Win32 shell until the window closes.
pub fn run() -> Result<()> {
    let app = Box::new(AppState::new().context("init host session")?);
    let app_ptr = Box::into_raw(app);

    // SAFETY: Win32 class registration / window creation for our process.
    unsafe {
        let hinstance = GetModuleHandleW(None)?;
        let class_name: Vec<u16> = CLASS_NAME.encode_utf16().collect();
        let cursor = LoadCursorW(None, IDC_ARROW)?;

        let wc = WNDCLASSEXW {
            cbSize: size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW | CS_OWNDC,
            lpfnWndProc: Some(wnd_proc),
            hInstance: hinstance.into(),
            hCursor: cursor,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..zeroed()
        };
        let atom = RegisterClassExW(&wc);
        if atom == 0 {
            let _ = Box::from_raw(app_ptr);
            anyhow::bail!("RegisterClassExW failed");
        }

        let menu = build_menu()?;
        let title: Vec<u16> = WINDOW_TITLE.encode_utf16().collect();
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE::default(),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            800,
            700,
            None,
            Some(menu),
            Some(hinstance.into()),
            Some(app_ptr.cast()),
        )?;

        (*app_ptr).main_hwnd = Some(hwnd);
        let _ = ShowWindow(hwnd, SW_SHOW);

        let mut msg = MSG::default();
        loop {
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT {
                    return Ok(());
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }

            let state_ptr = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AppState;
            if state_ptr.is_null() {
                break;
            }
            (*state_ptr).drain_pending_command();
            (*state_ptr).tick_frame(hwnd);
            let throttle = (*state_ptr).prefs.throttle;
            if throttle {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
    Ok(())
}
