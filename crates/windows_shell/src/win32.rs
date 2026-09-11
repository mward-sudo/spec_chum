//! Classic Win32 message-loop host (#351 vertical slice).

#![allow(unsafe_code)] // Win32 window-proc / GDI / USERDATA — SAFETY at each site.

use std::mem::{size_of, zeroed};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
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
    LoadCursorW, PeekMessageW, PostQuitMessage, RegisterClassExW, SetWindowLongPtrW,
    SetWindowTextW, ShowWindow, TranslateMessage, CREATESTRUCTW, CS_HREDRAW, CS_OWNDC, CS_VREDRAW,
    CW_USEDEFAULT, GWLP_USERDATA, HMENU, IDC_ARROW, MSG, PM_REMOVE, SW_SHOW, WINDOW_EX_STYLE,
    WM_COMMAND, WM_CREATE, WM_DESTROY, WM_KEYDOWN, WM_KEYUP, WM_PAINT, WM_QUIT, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WNDCLASSEXW, WS_OVERLAPPEDWINDOW,
};

use control_plane::ControlPlane;
use spec_chum_host::{HostError, HostSession, ModelId};

use windows_shell::audio::{self, PcmRing};
use windows_shell::keymap::{
    self, apply_chord, apply_modifiers, chord_for_vk, suppresses_modifier_caps,
};

const CLASS_NAME: &str = "SpecChumWindowsShell\0";
const WINDOW_TITLE: &str = "Spec Chum\0";

const IDM_FILE_OPEN_TAPE: usize = 1001;
const IDM_FILE_OPEN_SNAPSHOT: usize = 1002;
const IDM_FILE_OPEN_RZX: usize = 1003;
const IDM_FILE_OPEN_DSK: usize = 1004;
const IDM_FILE_OPEN_TRD: usize = 1005;
const IDM_FILE_EXIT: usize = 1009;
const IDM_TAPE_PLAY: usize = 1101;
const IDM_TAPE_PAUSE: usize = 1102;
const IDM_TAPE_REWIND: usize = 1103;

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
    /// Last key VKs we injected (for release / modifier refresh).
    held: Vec<u16>,
    bgra: Vec<u8>,
    status: String,
    last_frame: Instant,
    /// Menu / Ctrl+O actions queued from `wnd_proc` so modal `rfd` dialogs
    /// never run while `&mut AppState` is borrowed inside the window procedure.
    pending_cmd: Option<usize>,
}

impl AppState {
    fn new() -> Result<Self> {
        let mut session = HostSession::new(ModelId::Spectrum48, true);
        session.set_model(ModelId::Spectrum48);
        if !session.has_machine() {
            anyhow::bail!("48K ROM not found — run ./scripts/fetch_roms.sh from the repo root");
        }

        let pcm = Arc::new(Mutex::new(PcmRing::new()));
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
            held: Vec::new(),
            bgra: Vec::new(),
            status: "Ready".into(),
            last_frame: Instant::now(),
            pending_cmd: None,
        })
    }

    fn tick_frame(&mut self, hwnd: HWND) {
        // Pace toward ~50 Hz Spectrum frames (20 ms).
        let min_dt = Duration::from_millis(20);
        if self.last_frame.elapsed() < min_dt {
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
        // Modifiers first, then chords (arrows/punct may also hold Caps/Sym).
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

    fn open_path(
        &mut self,
        title: &str,
        filter_name: &str,
        exts: &[&str],
        load: impl FnOnce(&mut HostSession, &std::path::Path) -> Result<(), HostError>,
    ) {
        let path = rfd::FileDialog::new()
            .add_filter(filter_name, exts)
            .set_title(title)
            .pick_file();
        let Some(path) = path else { return };
        match self.host.with_mut(|s| load(s, &path)) {
            Ok(()) => {}
            Err(e) => {
                let msg = format!("{title} failed: {e}");
                self.host.with_mut(|s| s.set_status(msg.clone()));
                self.status = msg;
            }
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

    fn queue_command(&mut self, id: usize) {
        // Exit can run immediately (no modal dialog).
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
            IDM_TAPE_PLAY => {
                let _ = self.host.with_mut(HostSession::play_tape);
            }
            IDM_TAPE_PAUSE => {
                let _ = self.host.with_mut(HostSession::pause_tape);
            }
            IDM_TAPE_REWIND => {
                let _ = self.host.with_mut(HostSession::rewind_tape);
            }
            _ => {}
        }
        let _ = &self.plane;
    }
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

fn append_menu(menu: windows::Win32::UI::WindowsAndMessaging::HMENU, id: usize, text: &str) {
    use windows::Win32::UI::WindowsAndMessaging::{AppendMenuW, MF_STRING};
    let wide: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: menu created by us; text NUL-terminated.
    unsafe {
        let _ = AppendMenuW(menu, MF_STRING, id, PCWSTR(wide.as_ptr()));
    }
}

fn build_menu() -> Result<HMENU> {
    use windows::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreateMenu, CreatePopupMenu, MF_POPUP, MF_SEPARATOR, MF_STRING,
    };
    // SAFETY: CreateMenu/CreatePopupMenu return owned menus we attach to the window.
    unsafe {
        let menubar = CreateMenu()?;
        let file = CreatePopupMenu()?;
        append_menu(file, IDM_FILE_OPEN_TAPE, "Open &Tape…\tCtrl+O");
        append_menu(file, IDM_FILE_OPEN_SNAPSHOT, "Open &Snapshot…");
        append_menu(file, IDM_FILE_OPEN_RZX, "Open &RZX…");
        append_menu(file, IDM_FILE_OPEN_DSK, "Open &DSK…");
        append_menu(file, IDM_FILE_OPEN_TRD, "Open T&RD…");
        let _ = AppendMenuW(file, MF_SEPARATOR, 0, PCWSTR::null());
        append_menu(file, IDM_FILE_EXIT, "E&xit");

        let tape = CreatePopupMenu()?;
        append_menu(tape, IDM_TAPE_PLAY, "&Play");
        append_menu(tape, IDM_TAPE_PAUSE, "P&ause");
        append_menu(tape, IDM_TAPE_REWIND, "&Rewind");

        let file_label: Vec<u16> = "&File\0".encode_utf16().collect();
        let tape_label: Vec<u16> = "&Tape\0".encode_utf16().collect();
        let _ = AppendMenuW(
            menubar,
            MF_POPUP,
            file.0 as usize,
            PCWSTR(file_label.as_ptr()),
        );
        let _ = AppendMenuW(
            menubar,
            MF_POPUP,
            tape.0 as usize,
            PCWSTR(tape_label.as_ptr()),
        );
        let _ = MF_STRING;
        Ok(menubar)
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
                unsafe {
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
    let app = AppState::new().context("init host session")?;
    let app = Box::new(app);
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
            // Drain modal file opens outside wnd_proc (no re-entrant borrow).
            (*state_ptr).drain_pending_command();
            (*state_ptr).tick_frame(hwnd);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    Ok(())
}
