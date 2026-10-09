//! Classic Win32 message-loop host (#351 — Hardware / Settings / Debug deepen).

#![allow(unsafe_code)] // Win32 window-proc / GDI / USERDATA — SAFETY at each site.

use std::mem::{size_of, zeroed};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use machine::TapeLoadOptions;
use parking_lot::Mutex;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_SUCCESS, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_USE_IMMERSIVE_DARK_MODE};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, EndPaint, InvalidateRect, StretchDIBits, BITMAPINFO, BITMAPINFOHEADER, BI_RGB,
    DIB_RGB_COLORS, PAINTSTRUCT, SRCCOPY,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetKeyState, VK_CONTROL, VK_MENU, VK_SHIFT,
};
use windows::Win32::UI::WindowsAndMessaging::{
    CheckMenuRadioItem, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    GetClientRect, GetDlgItem, GetMenu, GetWindowLongPtrW, GetWindowTextLengthW, GetWindowTextW,
    LoadCursorW, MessageBoxW, MoveWindow, PeekMessageW, PostQuitMessage, RegisterClassExW,
    SendMessageW, SetWindowLongPtrW, SetWindowTextW, ShowWindow, TranslateMessage, CREATESTRUCTW,
    CS_HREDRAW, CS_OWNDC, CS_VREDRAW, CW_USEDEFAULT, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY,
    GWLP_USERDATA, HMENU, IDC_ARROW, IDOK, MB_ICONERROR, MB_OK, MB_OKCANCEL, MF_BYCOMMAND,
    MF_BYPOSITION, MSG, PM_REMOVE, SW_SHOW, WINDOW_EX_STYLE, WINDOW_STYLE, WM_ACTIVATE, WM_COMMAND,
    WM_CREATE, WM_DESTROY, WM_INITMENUPOPUP, WM_KEYDOWN, WM_KEYUP, WM_PAINT, WM_QUIT,
    WM_SETTINGCHANGE, WM_SIZE, WM_SYSKEYDOWN, WM_SYSKEYUP, WNDCLASSEXW, WS_BORDER, WS_CHILD,
    WS_OVERLAPPEDWINDOW, WS_VISIBLE, WS_VSCROLL,
};

use control_plane::ControlPlane;
use spec_chum_host::{
    acquire_next_assets, default_prefs_path, install_model_rom, load_prefs, query_recent_media,
    rom_setup_json, save_prefs, sync_model_rom_paths, AppearancePreference, HostError, HostSession,
    MediaCategory, MediaCompatibility, MediaEntry, MediaFormat, ModelId, PrefAyStereo, PrefModel,
    UiPreferences,
};

use native_shell_common::audio::{self, PcmRing};
use native_shell_common::commands::{
    ear_speed_from_menu_id, joystick_from_menu_id, menu_id_for_model, model_from_menu_id,
    model_menu_label,
};
use native_shell_common::{
    IDM_DBG_BREAK_PC, IDM_DBG_CLEAR_BREAKS, IDM_DBG_CONTINUE, IDM_DBG_PAUSE, IDM_DBG_REFRESH,
    IDM_DBG_STEP, IDM_DBG_TOGGLE, IDM_FILE_EXIT, IDM_FILE_OPEN_DSK, IDM_FILE_OPEN_RZX,
    IDM_FILE_OPEN_SNAPSHOT, IDM_FILE_OPEN_TAPE, IDM_FILE_OPEN_TRD, IDM_HW_ATTACH_BETA,
    IDM_HW_ATTACH_DIVMMC, IDM_HW_ATTACH_IF1, IDM_HW_ATTACH_MULTIFACE, IDM_HW_DIVMMC_EEPROM,
    IDM_HW_DIVMMC_SD, IDM_HW_DIVMMC_SD_SLOT1, IDM_HW_EJECT_DCK, IDM_HW_INSERT_DCK,
    IDM_HW_INSERT_MDR, IDM_HW_LOAD_TRDOS_ROM, IDM_HW_MULTIFACE_NMI, IDM_HW_OPEN_TRD,
    IDM_MACHINE_RESET, IDM_SET_APPEARANCE_DARK, IDM_SET_APPEARANCE_LIGHT,
    IDM_SET_APPEARANCE_SYSTEM, IDM_SET_AY_ABC, IDM_SET_AY_ACB, IDM_SET_AY_MONO, IDM_SET_JOY_CURSOR,
    IDM_SET_JOY_KEMPSTON, IDM_SET_JOY_SINCLAIR_L, IDM_SET_JOY_SINCLAIR_R, IDM_SET_KEMPSTON_MOUSE,
    IDM_SET_MUTE, IDM_SET_ONLINE_TITLES, IDM_SET_TAPE_EAR_1, IDM_SET_TAPE_EAR_10,
    IDM_SET_TAPE_EAR_2, IDM_SET_TAPE_EAR_20, IDM_SET_TAPE_EAR_5, IDM_SET_TAPE_EXPERIENCE,
    IDM_SET_TAPE_INSTANT, IDM_SET_THROTTLE, IDM_TAPE_PAUSE, IDM_TAPE_PLAY, IDM_TAPE_REWIND,
};
#[path = "win32/library.rs"]
mod library;

use windows_shell::keymap::{
    self, apply_chord, apply_modifiers, chord_for_vk, suppresses_modifier_caps,
};

const CLASS_NAME: &str = "SpecChumWindowsShell\0";
const DEBUG_CLASS: &str = "SpecChumWindowsDebug\0";
const WINDOW_TITLE: &str = "Spec Chum\0";
const ID_DBG_EDIT: i32 = 2001;
const IDM_MACHINE_ROM_SETUP: usize = 1289;
const IDM_LIBRARY_RECENT_BASE: usize = 1600;
const IDM_LIBRARY_RECENT_COUNT: usize = 12;
const IDM_LIBRARY_OPEN: usize = 1590;
const PERSONALIZE_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Themes\\Personalize\0";
const APPS_USE_LIGHT_THEME: &str = "AppsUseLightTheme\0";

/// Live host held either exclusively or shared with embedded Agent HTTP.
#[derive(Debug)]
enum HostSlot {
    Direct(Box<HostSession>),
    Shared(Arc<Mutex<HostSession>>),
}

#[derive(Clone, Copy, Debug)]
enum AfterNextAssets {
    Select(ModelId),
    ReloadCurrent(ModelId),
}

#[derive(Debug)]
struct NextAssetDownload {
    receiver: Receiver<Result<PathBuf, String>>,
    after: Option<AfterNextAssets>,
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
    _stream: Option<audio::OutputStream>,
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
    library_hwnd: Option<HWND>,
    library_search: Option<HWND>,
    library_category: Option<HWND>,
    library_list: Option<HWND>,
    library_details: Option<HWND>,
    library_entries: Vec<MediaEntry>,
    main_hwnd: Option<HWND>,
    /// Last time the debugger EDIT control was rewritten from `tick_frame`.
    last_debug_refresh: Instant,
    next_assets_download: Option<NextAssetDownload>,
    recent_menu: Option<HMENU>,
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
        let (selected_model, _preferred_error) =
            select_startup_model(model, |candidate| session.select_model(candidate)).map_err(
                |error| match error {
                    StartupModelError::PreferredAndFallback(preferred_error, fallback_error) => {
                        anyhow::anyhow!(
                            "could not initialize preferred model {model:?} ({preferred_error}); \
                     48K fallback also failed ({fallback_error}); run ./scripts/fetch_roms.sh"
                        )
                    }
                    StartupModelError::RequiredModel(error) => anyhow::anyhow!(
                "could not initialize required 48K model ({error}); run ./scripts/fetch_roms.sh"
            ),
                },
            )?;
        if selected_model != model {
            // A missing preferred ROM is an expected fallback case.
            prefs.select_builtin_model(PrefModel::Spectrum48);
        }

        prefs
            .apply_to_host_session(&mut session)
            .context("apply session prefs")?;

        let pcm = Arc::new(Mutex::new(PcmRing::new()));
        {
            let mut ring = pcm.lock();
            ring.volume = prefs.volume;
            ring.muted = prefs.muted;
        }
        let stream = audio::start_stream(Arc::clone(&pcm), audio::ShellPlatform::Windows);

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
            library_hwnd: None,
            library_search: None,
            library_category: None,
            library_list: None,
            library_details: None,
            library_entries: Vec::new(),
            main_hwnd: None,
            last_debug_refresh: Instant::now(),
            next_assets_download: None,
            recent_menu: None,
        })
    }

    fn persist_prefs(&self) {
        if let Err(e) = save_prefs(&self.prefs_path, &self.prefs) {
            eprintln!("spec-chum-windows: save prefs failed: {e}");
        }
    }

    fn apply_prefs_to_host(&mut self) -> Result<(), HostError> {
        let prefs = self.prefs.clone();
        self.host.with_mut(|s| prefs.apply_to_host_session(s))?;
        let mut ring = self.pcm.lock();
        ring.volume = prefs.volume;
        ring.muted = prefs.muted;
        Ok(())
    }

    fn tick_frame(&mut self, hwnd: HWND) {
        self.poll_next_assets_download();
        let min_dt = if self.prefs.throttle {
            self.host
                .with_mut(|session| Duration::from_secs_f64(session.frame_period_seconds()))
        } else {
            Duration::from_millis(0)
        };
        if min_dt > Duration::ZERO {
            let now = Instant::now();
            let elapsed = now.duration_since(self.last_frame);
            if elapsed < min_dt {
                return;
            }
            self.last_frame = if elapsed > min_dt * 2 {
                now
            } else {
                self.last_frame + min_dt
            };
        } else {
            self.last_frame = Instant::now();
        }

        let (pcm_snap, title, w, h, rgba) = self.host.with_mut(|s| {
            let _ = s.run_frame();
            let pcm = s.audio_pcm_stereo().to_vec();
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

        self.pcm.lock().push_stereo_frame(&pcm_snap);
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
        let (dst_pixels, _) = self.bgra.as_chunks_mut::<4>();
        let (src_pixels, _) = rgba.as_chunks::<4>();
        for (dst, src) in dst_pixels.iter_mut().zip(src_pixels) {
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

        let held = self.held.clone();
        let mut has_owned_chord = false;
        let mut has_plain_chord = false;
        for &vk in &held {
            if chord_for_vk(vk, shift).is_some() {
                if suppresses_modifier_caps(vk, shift) {
                    has_owned_chord = true;
                } else {
                    has_plain_chord = true;
                }
            }
        }
        let suppress = spec_chum_host::keymap::caps_modifier_suppressed(
            has_owned_chord,
            has_plain_chord,
            held.iter().copied().any(keymap::is_joystick_routing_key),
        );
        let _ = self
            .host
            .with_mut(|session| session.set_joystick(keymap::kempston_mask(&held)));
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
            &["sna", "z80"],
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

    fn open_recent_media(&mut self, index: usize) {
        let Some(path) = recent_media_paths(&self.prefs.recent_files)
            .get(index)
            .cloned()
        else {
            return;
        };
        match self.host.with_mut(|session| session.open_media_path(&path)) {
            Ok(()) => {
                self.prefs.push_recent(&path);
                self.persist_prefs();
            }
            Err(error) => self.report_err("Open recent media", &error),
        }
    }

    fn refresh_recent_menu_entries(&self) {
        if let Some(menu) = self.recent_menu {
            self.refresh_recent_menu(menu);
        }
    }

    fn refresh_recent_menu(&self, menu: HMENU) {
        use windows::Win32::UI::WindowsAndMessaging::{DeleteMenu, GetMenuItemCount};
        // SAFETY: menu is the Library popup created and owned by this window.
        unsafe {
            while GetMenuItemCount(Some(menu)) > 0 {
                if DeleteMenu(menu, 0, MF_BYPOSITION).is_err() {
                    break;
                }
            }
        }
        let paths = recent_media_paths(&self.prefs.recent_files);
        if paths.is_empty() {
            let label: Vec<u16> = "No recent media\0".encode_utf16().collect();
            // SAFETY: menu is our popup and label is NUL-terminated.
            unsafe {
                let _ = windows::Win32::UI::WindowsAndMessaging::AppendMenuW(
                    menu,
                    windows::Win32::UI::WindowsAndMessaging::MF_STRING
                        | windows::Win32::UI::WindowsAndMessaging::MF_GRAYED,
                    0,
                    PCWSTR(label.as_ptr()),
                );
            }
            return;
        }
        for (index, path) in paths.iter().enumerate() {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("Media");
            let parent = path.parent().and_then(Path::to_str).unwrap_or("");
            append_menu(
                menu,
                IDM_LIBRARY_RECENT_BASE + index,
                &format!("&{} {name} — {parent}", index + 1),
            );
        }
    }

    /// Offer the shared per-slot ROM setup flow. Returns false when the user
    /// cancels, validation fails, or a background download is pending.
    fn setup_roms_for_model(&mut self, model: ModelId, after: AfterNextAssets) -> bool {
        if model == ModelId::SpectrumNext && self.next_assets_download.is_some() {
            self.host.with_mut(|session| {
                session.set_status("Official System/Next assets are still downloading…".to_owned())
            });
            return false;
        }
        let mut paths = self.prefs.model_rom_paths.clone();
        let setup = rom_setup_json(model, &paths);
        if setup.complete {
            return true;
        }
        if model == ModelId::SpectrumNext
            && message_box_confirm(
                self.main_hwnd,
                "ZX Spectrum Next assets",
                "Get the pinned official System/Next 24.11 distribution and separate GPL boot code? Choose Cancel to select the verified archive from its companion asset folder. Source and license details: https://github.com/mward-sudo/spec_chum/blob/main/docs/ROMS.md. Spec Chum is unaffiliated with SpecNext Ltd.",
            )
        {
            if self.next_assets_download.is_none() {
                let (sender, receiver) = mpsc::channel();
                std::thread::spawn(move || {
                    let result = acquire_next_assets().map_err(|error| error.to_string());
                    let _ = sender.send(result);
                });
                self.next_assets_download = Some(NextAssetDownload {
                    receiver,
                    after: Some(after),
                });
                self.host.with_mut(|session| {
                    session.set_status("Acquiring official System/Next assets…".to_owned())
                });
            }
            return false;
        }
        for slot in setup.slots.iter().filter(|slot| slot.status != "found") {
            let prompt = format!(
                "{} ROM Setup\n{} — {}\nExpected size: {} bytes\n{}\n\nSelect OK to choose an image, or Cancel to keep the current model.",
                setup.model_title, slot.label, slot.status, slot.expected_bytes, slot.hint
            );
            if !message_box_confirm(self.main_hwnd, "ROM Setup", &prompt) {
                return false;
            }
            let extensions: &[&str] = if model == ModelId::SpectrumNext {
                &["zip"]
            } else {
                &["rom", "bin"]
            };
            let path = rfd::FileDialog::new()
                .add_filter(&slot.label, extensions)
                .set_title(format!("Choose {} ROM", slot.label))
                .pick_file();
            let Some(path) = path else { return false };
            match install_model_rom(model, &slot.id, &path, &mut paths) {
                Ok(_) => {
                    self.prefs.model_rom_paths = paths.clone();
                    self.persist_prefs();
                }
                Err(error) => {
                    self.report_err("ROM Setup", &HostError::Message(error.to_string()));
                    return false;
                }
            }
        }
        sync_model_rom_paths(paths.clone());
        self.prefs.model_rom_paths = paths;
        self.persist_prefs();
        true
    }

    fn poll_next_assets_download(&mut self) {
        let Some(download) = self.next_assets_download.as_ref() else {
            return;
        };
        let result = match download.receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("Official asset download stopped unexpectedly".into())
            }
        };
        let after = self
            .next_assets_download
            .take()
            .and_then(|download| download.after);
        match result {
            Ok(archive) => {
                self.prefs.model_rom_paths.insert(
                    spec_chum_host::model_rom_path_key(PrefModel::SpectrumNext, "next_assets"),
                    archive.display().to_string(),
                );
                sync_model_rom_paths(self.prefs.model_rom_paths.clone());
                self.persist_prefs();
                self.host.with_mut(|session| {
                    session.set_status(format!("Verified {}", archive.display()))
                });
                match after {
                    Some(AfterNextAssets::Select(model)) => self.select_model(model),
                    Some(AfterNextAssets::ReloadCurrent(model))
                        if self.prefs.model.to_model_id() == model =>
                    {
                        self.select_model(model);
                    }
                    _ => {}
                }
            }
            Err(error) => self.report_err("Next asset setup", &HostError::Message(error)),
        }
        if self.library_hwnd.is_some() {
            self.refresh_library();
        }
    }

    fn select_model(&mut self, model: ModelId) {
        if let Some(download) = self.next_assets_download.as_mut() {
            download.after =
                (model == ModelId::SpectrumNext).then_some(AfterNextAssets::Select(model));
        }
        let mut next = self.prefs.clone();
        next.select_builtin_model(PrefModel::from_model_id(model));
        sync_model_rom_paths(next.model_rom_paths.clone());
        let result = self.host.with_mut(|s| {
            s.select_model(model)?;
            next.apply_to_host_session(s)?;
            Ok::<(), HostError>(())
        });
        match result {
            Ok(()) => {
                self.prefs = next;
                self.persist_prefs();
            }
            Err(e) => {
                sync_model_rom_paths(self.prefs.model_rom_paths.clone());
                let setup = rom_setup_json(model, &self.prefs.model_rom_paths);
                let restore = self.prefs.model.to_model_id();
                if let Err(restore_error) = self.host.with_mut(|s| {
                    s.select_model(restore)?;
                    self.prefs.apply_to_host_session(s)
                }) {
                    self.report_err(
                        "Select model",
                        &HostError::Message(format!(
                            "selecting {model:?} failed ({e}); restoring {restore:?} also failed ({restore_error})"
                        )),
                    );
                    return;
                }
                if !setup.complete
                    && self.setup_roms_for_model(model, AfterNextAssets::Select(model))
                {
                    let mut retry = self.prefs.clone();
                    retry.select_builtin_model(PrefModel::from_model_id(model));
                    sync_model_rom_paths(retry.model_rom_paths.clone());
                    match self.host.with_mut(|s| {
                        s.select_model(model)?;
                        retry.apply_to_host_session(s)
                    }) {
                        Ok(()) => {
                            self.prefs = retry;
                            self.persist_prefs();
                        }
                        Err(retry_error) => self.report_err("Select model", &retry_error),
                    }
                } else if self.next_assets_download.is_none() {
                    self.report_err("Select model", &e);
                }
            }
        }
    }

    fn open_rom_setup(&mut self) {
        let model = self.prefs.model.to_model_id();
        let already_complete = rom_setup_json(model, &self.prefs.model_rom_paths).complete;
        if !self.setup_roms_for_model(model, AfterNextAssets::ReloadCurrent(model))
            || already_complete
        {
            return;
        }
        if let Err(e) = self.host.with_mut(|s| s.select_model(model)) {
            self.report_err("ROM Setup", &e);
            return;
        }
        if let Err(e) = self.apply_prefs_to_host() {
            self.report_err("ROM Setup", &e);
        }
    }

    fn reset_machine(&mut self) {
        if let Err(e) = self.host.with_mut(HostSession::reset) {
            self.report_err("Reset", &e);
        }
    }

    fn update_prefs(&mut self, title: &str, update: impl FnOnce(&mut UiPreferences)) -> bool {
        if let Err(e) = self
            .host
            .with_mut(|session| self.prefs.update_and_apply(session, update))
        {
            self.report_err(title, &e);
            return false;
        }
        self.persist_prefs();
        true
    }

    fn set_joystick(&mut self, joy: spec_chum_host::PrefJoystick) {
        self.update_prefs("Joystick", |prefs| prefs.set_joystick(joy.to_mode()));
    }

    fn set_tape_ear_speed(&mut self, speed: u32) {
        self.update_prefs("Tape EAR speed", |prefs| {
            prefs.tape_experience = false;
            prefs.tape_ear_speed = speed;
        });
    }

    fn set_tape_experience(&mut self) {
        self.update_prefs("Experience load", |prefs| prefs.tape_experience = true);
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

    fn set_appearance(&mut self, appearance: AppearancePreference) {
        self.prefs.appearance = appearance;
        self.persist_prefs();
        if let Some(hwnd) = self.main_hwnd {
            apply_window_appearance(hwnd, appearance);
        }
    }

    fn set_ay(&mut self, mode: PrefAyStereo) {
        self.update_prefs("AY stereo", |prefs| prefs.set_ay_stereo(mode.to_mode()));
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
        let text = self.host.with_mut(|session| session.debugger_text());
        // Win32 multiline EDIT expects CRLF line endings.
        let text = text.replace("\r\n", "\n").replace('\n', "\r\n");
        set_window_title(edit, &text);
        self.last_debug_refresh = Instant::now();
    }

    fn debug_pause(&mut self) {
        self.host.with_mut(HostSession::debug_pause);
        self.refresh_debug_text();
    }

    fn debug_continue(&mut self) {
        self.host_action("Continue", HostSession::debug_continue);
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

    fn drain_pending_command(&mut self, id: usize) {
        let affects_library = (IDM_LIBRARY_RECENT_BASE
            ..IDM_LIBRARY_RECENT_BASE + IDM_LIBRARY_RECENT_COUNT)
            .contains(&id)
            || model_from_menu_id(id).is_some()
            || matches!(
                id,
                IDM_FILE_OPEN_TAPE
                    | IDM_FILE_OPEN_SNAPSHOT
                    | IDM_FILE_OPEN_RZX
                    | IDM_FILE_OPEN_DSK
                    | IDM_FILE_OPEN_TRD
                    | IDM_MACHINE_ROM_SETUP
                    | IDM_HW_ATTACH_BETA
                    | IDM_HW_LOAD_TRDOS_ROM
                    | IDM_HW_OPEN_TRD
            );
        match id {
            id if (IDM_LIBRARY_RECENT_BASE..IDM_LIBRARY_RECENT_BASE + IDM_LIBRARY_RECENT_COUNT)
                .contains(&id) =>
            {
                self.open_recent_media(id - IDM_LIBRARY_RECENT_BASE)
            }
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
            IDM_MACHINE_ROM_SETUP => self.open_rom_setup(),
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
            IDM_SET_APPEARANCE_SYSTEM => self.set_appearance(AppearancePreference::System),
            IDM_SET_APPEARANCE_LIGHT => self.set_appearance(AppearancePreference::Light),
            IDM_SET_APPEARANCE_DARK => self.set_appearance(AppearancePreference::Dark),
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
        if affects_library && self.library_hwnd.is_some() {
            self.refresh_library();
        }
        let _ = &self.plane;
    }
}

/// Select the configured startup model, falling back to 48K after a failed
/// preferred ROM load. The preferred error is retained for diagnostics if the
/// required fallback fails too.
#[derive(Debug, PartialEq, Eq)]
enum StartupModelError<E> {
    PreferredAndFallback(E, E),
    RequiredModel(E),
}

fn select_startup_model<E>(
    preferred: ModelId,
    mut select: impl FnMut(ModelId) -> Result<(), E>,
) -> Result<(ModelId, Option<E>), StartupModelError<E>> {
    match select(preferred) {
        Ok(()) => Ok((preferred, None)),
        Err(error) if preferred == ModelId::Spectrum48 => {
            Err(StartupModelError::RequiredModel(error))
        }
        Err(preferred_error) => match select(ModelId::Spectrum48) {
            Ok(()) => Ok((ModelId::Spectrum48, Some(preferred_error))),
            Err(fallback_error) => Err(StartupModelError::PreferredAndFallback(
                preferred_error,
                fallback_error,
            )),
        },
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

fn message_box_confirm(owner: Option<HWND>, title: &str, body: &str) -> bool {
    let title: Vec<u16> = title.encode_utf16().chain(Some(0)).collect();
    let body: Vec<u16> = body.encode_utf16().chain(Some(0)).collect();
    // SAFETY: Both strings are NUL-terminated UTF-16, and owner is an optional
    // HWND supplied by this process' window state.
    unsafe {
        MessageBoxW(
            owner,
            PCWSTR(body.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_OKCANCEL,
        ) == IDOK
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

fn recent_media_paths(recent_files: &[String]) -> Vec<PathBuf> {
    recent_files
        .iter()
        .filter_map(|value| MediaFormat::from_path(Path::new(value)).map(|_| PathBuf::from(value)))
        .take(IDM_LIBRARY_RECENT_COUNT)
        .collect()
}

fn build_menu() -> Result<(HMENU, HMENU)> {
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

        let library = CreatePopupMenu()?;
        append_menu(library, IDM_LIBRARY_OPEN, "Open Library…");
        append_sep(library);
        let recent = CreatePopupMenu()?;
        append_popup(library, recent, "Open &Recent");
        append_popup(menubar, library, "&Library");

        let machine = CreatePopupMenu()?;
        for m in ModelId::ALL {
            append_menu(machine, menu_id_for_model(m), model_menu_label(m));
        }
        append_sep(machine);
        append_menu(machine, IDM_MACHINE_ROM_SETUP, "ROM &Setup…");
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
        let appearance = CreatePopupMenu()?;
        append_menu(appearance, IDM_SET_APPEARANCE_SYSTEM, "&System");
        append_menu(appearance, IDM_SET_APPEARANCE_LIGHT, "&Light");
        append_menu(appearance, IDM_SET_APPEARANCE_DARK, "&Dark");
        append_popup(settings, appearance, "&Appearance");
        append_sep(settings);
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

        Ok((menubar, recent))
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

fn system_appearance_is_dark() -> bool {
    let key: Vec<u16> = PERSONALIZE_KEY.encode_utf16().collect();
    let value_name: Vec<u16> = APPS_USE_LIGHT_THEME.encode_utf16().collect();
    let mut apps_use_light_theme = 1_u32;
    let mut value_size = size_of::<u32>() as u32;
    // SAFETY: both strings are NUL-terminated UTF-16 buffers that remain alive
    // for the call; the data and size pointers refer to live DWORD values.
    let result = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            PCWSTR(key.as_ptr()),
            PCWSTR(value_name.as_ptr()),
            RRF_RT_REG_DWORD,
            None,
            Some((&mut apps_use_light_theme as *mut u32).cast()),
            Some(&mut value_size),
        )
    };

    // Missing/unreadable personalization values use Windows' light default.
    result == ERROR_SUCCESS && apps_use_light_theme == 0
}

fn apply_window_appearance(hwnd: HWND, preference: AppearancePreference) {
    let dark = match preference {
        AppearancePreference::System => system_appearance_is_dark(),
        AppearancePreference::Light => false,
        AppearancePreference::Dark => true,
    };
    let dark_mode = i32::from(dark);
    // SAFETY: hwnd is the live top-level window owned by this shell and the
    // attribute receives a pointer to the correctly sized Win32 BOOL value (i32).
    let _ = unsafe {
        DwmSetWindowAttribute(
            hwnd,
            DWMWA_USE_IMMERSIVE_DARK_MODE,
            (&dark_mode as *const i32).cast(),
            size_of::<i32>() as u32,
        )
    };

    let selected_id = match preference {
        AppearancePreference::System => IDM_SET_APPEARANCE_SYSTEM,
        AppearancePreference::Light => IDM_SET_APPEARANCE_LIGHT,
        AppearancePreference::Dark => IDM_SET_APPEARANCE_DARK,
    };
    // SAFETY: hwnd is the live top-level window whose menu was created by this shell.
    let menu = unsafe { GetMenu(hwnd) };
    if !menu.0.is_null() {
        // SAFETY: the appearance commands are consecutive IDs in this menu.
        let _ = unsafe {
            CheckMenuRadioItem(
                menu,
                IDM_SET_APPEARANCE_SYSTEM as u32,
                IDM_SET_APPEARANCE_DARK as u32,
                selected_id as u32,
                MF_BYCOMMAND.0,
            )
        };
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
        WM_SETTINGCHANGE => {
            if !state_ptr.is_null() {
                // SAFETY: state_ptr is owned by this window until WM_DESTROY.
                let state = unsafe { &mut *state_ptr };
                if state.prefs.appearance == AppearancePreference::System {
                    apply_window_appearance(hwnd, AppearancePreference::System);
                }
            }
            // Keep the standard notification handling intact for USER32/DWM.
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
        }
        WM_COMMAND => {
            if !state_ptr.is_null() {
                let id = wparam.0 & 0xffff;
                let state = unsafe { &mut *state_ptr };
                state.queue_command(id);
            }
            LRESULT(0)
        }
        WM_INITMENUPOPUP => {
            if !state_ptr.is_null() {
                let state = unsafe { &mut *state_ptr };
                let menu = HMENU(wparam.0 as _);
                if state.recent_menu == Some(menu) {
                    state.refresh_recent_menu(menu);
                    return LRESULT(0);
                }
            }
            unsafe { DefWindowProcW(hwnd, msg, wparam, lparam) }
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
                // DestroyWindow synchronously calls the Library procedure, which
                // borrows AppState. End our borrow before destroying either HWND.
                unsafe {
                    let (dbg, library) = {
                        let state = &mut *state_ptr;
                        state.debug_edit = None;
                        (state.debug_hwnd.take(), state.library_hwnd.take())
                    };
                    if let Some(dbg) = dbg {
                        let _ = DestroyWindow(dbg);
                    }
                    if let Some(library) = library {
                        let _ = DestroyWindow(library);
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

        let (menu, recent_menu) = build_menu()?;
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
        (*app_ptr).recent_menu = Some(recent_menu);
        apply_window_appearance(hwnd, (*app_ptr).prefs.appearance);
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
            // The Library constructor enters its window proc synchronously.
            // Pop the command before calling it so no AppState borrow spans that call.
            let pending_cmd = (*state_ptr).pending_cmd.take();
            if let Some(id) = pending_cmd {
                if id == IDM_LIBRARY_OPEN {
                    library::open_library(state_ptr);
                } else {
                    (*state_ptr).drain_pending_command(id);
                }
            }
            (*state_ptr).tick_frame(hwnd);
            let throttle = (*state_ptr).prefs.throttle;
            if throttle {
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod startup_tests {
    use super::*;

    #[test]
    fn library_recent_paths_include_supported_media_only() {
        let recent = [
            "/games/next.rom",
            "/games/disk.img",
            "/games/PLAY.TAP",
            "/games/snapshot.szx",
            "/games/disk.trd",
            "/games/disk.dsk",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();

        let paths = recent_media_paths(&recent);

        assert_eq!(
            paths,
            vec![
                PathBuf::from("/games/PLAY.TAP"),
                PathBuf::from("/games/disk.trd"),
                PathBuf::from("/games/disk.dsk"),
            ]
        );
    }

    #[test]
    fn preferred_failure_uses_successful_48k_fallback() {
        let result = select_startup_model(ModelId::Spectrum128, |model| {
            if model == ModelId::Spectrum48 {
                Ok(())
            } else {
                Err("preferred missing")
            }
        });
        assert_eq!(result, Ok((ModelId::Spectrum48, Some("preferred missing"))));
    }

    #[test]
    fn preferred_and_fallback_failures_are_both_returned() {
        let result = select_startup_model(ModelId::Spectrum128, |model| {
            if model == ModelId::Spectrum48 {
                Err("fallback missing")
            } else {
                Err("preferred missing")
            }
        });
        assert!(matches!(
            result,
            Err(StartupModelError::PreferredAndFallback(
                "preferred missing",
                "fallback missing"
            ))
        ));
    }

    #[test]
    fn direct_48k_failure_is_returned_without_retry() {
        let mut attempts = 0;
        let result = select_startup_model(ModelId::Spectrum48, |_| {
            attempts += 1;
            Err::<(), _>("48K missing")
        });
        assert_eq!(attempts, 1);
        assert!(matches!(
            result,
            Err(StartupModelError::RequiredModel("48K missing"))
        ));
    }
}
