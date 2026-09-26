//! GTK4 application host for Spec Chum on Linux (#351 deepen).

use std::cell::RefCell;
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use control_plane::ControlPlane;
use gdk4::prelude::*;
use glib::clone;
use glib::translate::IntoGlib;
use gtk4::prelude::*;
use gtk4::{
    gio, glib, Application, ApplicationWindow, Label, Orientation, Picture, ScrolledWindow,
    TextBuffer, TextView, Window,
};
use machine::TapeLoadOptions;
use parking_lot::Mutex;
use spec_chum_host::{
    default_prefs_path, load_prefs, save_prefs, sync_model_rom_paths, HostError, HostSession,
    ModelId, PrefAyStereo, PrefModel, UiPreferences,
};

use linux_shell::audio::{self, PcmRing};
use linux_shell::commands::model_menu_label;
use linux_shell::keymap::{
    apply_chord, apply_modifiers, chord_for_keyval, is_modifier_keyval,
    suppresses_modifier_caps_with_shift,
};

const APP_ID: &str = "org.specchum.SpecChumLinux";

enum HostSlot {
    Direct(Box<HostSession>),
    Shared(Arc<Mutex<HostSession>>),
}

impl HostSlot {
    fn with_mut<R>(&mut self, f: impl FnOnce(&mut HostSession) -> R) -> R {
        match self {
            Self::Direct(s) => f(s),
            Self::Shared(s) => f(&mut s.lock()),
        }
    }
}

struct AppState {
    host: HostSlot,
    pcm: Arc<Mutex<PcmRing>>,
    _stream: Option<cpal::Stream>,
    _agent: Option<agent_server::embedded::EmbeddedServer>,
    _plane: Option<Arc<ControlPlane>>,
    prefs: UiPreferences,
    prefs_path: std::path::PathBuf,
    held: Vec<u32>,
    shift_l: bool,
    shift_r: bool,
    ctrl_l: bool,
    ctrl_r: bool,
    alt_l: bool,
    alt_r: bool,
    last_frame: Instant,
    debug_window: Option<Window>,
    debug_buffer: Option<TextBuffer>,
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
        let selected = select_startup_model(model, |candidate| session.select_model(candidate))
            .context("select startup model")?;
        if selected != model {
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
        let stream = audio::start_stream(Arc::clone(&pcm));

        let (host, plane, agent) = if std::env::var("SPEC_CHUM_AGENT").ok().as_deref() == Some("1")
        {
            let shared = Arc::new(Mutex::new(session));
            let plane = Arc::new(ControlPlane::from_shared(Arc::clone(&shared)));
            let agent = match agent_server::embedded::spawn_from_env_with_plane(Arc::clone(&plane))
            {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("spec-chum-linux: embedded agent failed: {e}");
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
            _plane: plane,
            prefs,
            prefs_path,
            held: Vec::new(),
            shift_l: false,
            shift_r: false,
            ctrl_l: false,
            ctrl_r: false,
            alt_l: false,
            alt_r: false,
            last_frame: Instant::now(),
            debug_window: None,
            debug_buffer: None,
            last_debug_refresh: Instant::now(),
        })
    }

    fn persist_prefs(&self) {
        if let Err(e) = save_prefs(&self.prefs_path, &self.prefs) {
            eprintln!("spec-chum-linux: save prefs failed: {e}");
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

    fn shift(&self) -> bool {
        self.shift_l || self.shift_r
    }

    fn ctrl(&self) -> bool {
        self.ctrl_l || self.ctrl_r
    }

    fn alt(&self) -> bool {
        self.alt_l || self.alt_r
    }

    fn set_key_matrix(&mut self, row: usize, bit: u8, pressed: bool) {
        let _ = self.host.with_mut(|s| s.set_key(row, bit, pressed));
    }

    fn refresh_input(&mut self) {
        let _ = self.host.with_mut(HostSession::clear_keys);
        let shift = self.shift();
        let mut suppress = false;
        for &kv in &self.held {
            if suppresses_modifier_caps_with_shift(kv, shift) {
                suppress = true;
            }
        }
        let alt = self.alt();
        let ctrl = self.ctrl();
        {
            let mut apply = |row, bit, pressed| self.set_key_matrix(row, bit, pressed);
            apply_modifiers(&mut apply, shift, alt, ctrl, suppress);
        }
        let held = self.held.clone();
        for &kv in &held {
            if let Some(ch) = chord_for_keyval(kv, shift) {
                let mut apply = |row, bit, pressed| self.set_key_matrix(row, bit, pressed);
                apply_chord(&mut apply, &ch, true);
            }
        }
    }

    fn on_key(&mut self, keyval: u32, pressed: bool) {
        if is_modifier_keyval(keyval) {
            match keyval {
                linux_shell::keymap::key::SHIFT_L => self.shift_l = pressed,
                linux_shell::keymap::key::SHIFT_R => self.shift_r = pressed,
                linux_shell::keymap::key::CONTROL_L => self.ctrl_l = pressed,
                linux_shell::keymap::key::CONTROL_R => self.ctrl_r = pressed,
                linux_shell::keymap::key::ALT_L => self.alt_l = pressed,
                linux_shell::keymap::key::ALT_R => self.alt_r = pressed,
                _ => {}
            }
            self.refresh_input();
            return;
        }
        if pressed {
            if !self.held.contains(&keyval) {
                self.held.push(keyval);
            }
        } else {
            self.held.retain(|&k| k != keyval);
        }
        self.refresh_input();
    }

    fn clear_all_input(&mut self) {
        self.held.clear();
        self.shift_l = false;
        self.shift_r = false;
        self.ctrl_l = false;
        self.ctrl_r = false;
        self.alt_l = false;
        self.alt_r = false;
        self.refresh_input();
    }

    fn report_err(&mut self, title: &str, err: &HostError) {
        let msg = format!("{title}: {err}");
        eprintln!("spec-chum-linux: {msg}");
        self.host.with_mut(|s| s.set_status(msg));
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
        let Some(path) = path else {
            return;
        };
        match self.host.with_mut(|s| load(s, &path)) {
            Ok(()) => {
                self.prefs.push_recent(&path);
                self.persist_prefs();
            }
            Err(e) => self.report_err(title, &e),
        }
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

    fn select_model(&mut self, model: ModelId) {
        let previous = self.prefs.clone();
        let mut next = previous.clone();
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
                // `select_model` may leave the host with no machine; restore prior.
                sync_model_rom_paths(previous.model_rom_paths.clone());
                let restore = previous.model.to_model_id();
                if let Err(restore_error) = self.host.with_mut(|s| {
                    s.select_model(restore)?;
                    previous.apply_to_host_session(s)
                }) {
                    let combined = HostError::Message(format!(
                        "selecting {model:?} failed ({e}); restoring {restore:?} also failed ({restore_error})"
                    ));
                    self.report_err("Select model", &combined);
                } else {
                    self.report_err("Select model", &e);
                }
            }
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

    fn toggle_debug_window(&mut self, parent: &ApplicationWindow, state: &Rc<RefCell<AppState>>) {
        if let Some(win) = self.debug_window.take() {
            self.debug_buffer = None;
            if win.is_visible() {
                win.destroy();
            }
            return;
        }
        let buffer = TextBuffer::new(None::<&gtk4::TextTagTable>);
        let view = TextView::with_buffer(&buffer);
        view.set_editable(false);
        view.set_monospace(true);
        view.set_wrap_mode(gtk4::WrapMode::WordChar);
        let scroll = ScrolledWindow::builder()
            .child(&view)
            .hexpand(true)
            .vexpand(true)
            .build();
        let win = Window::builder()
            .transient_for(parent)
            .title("Spec Chum — Debugger")
            .default_width(640)
            .default_height(520)
            .child(&scroll)
            .build();
        win.connect_close_request(clone!(
            #[strong]
            state,
            move |_| {
                let mut s = state.borrow_mut();
                s.debug_window = None;
                s.debug_buffer = None;
                glib::Propagation::Proceed
            }
        ));
        win.present();
        self.debug_buffer = Some(buffer);
        self.debug_window = Some(win);
        self.refresh_debug_text();
    }

    fn maybe_refresh_debug_text(&mut self) {
        if self.debug_buffer.is_none() {
            return;
        }
        const MIN_INTERVAL: Duration = Duration::from_millis(250);
        if self.last_debug_refresh.elapsed() < MIN_INTERVAL {
            return;
        }
        self.refresh_debug_text();
    }

    fn refresh_debug_text(&mut self) {
        let Some(buffer) = self.debug_buffer.as_ref() else {
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
        buffer.set_text(&text);
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

    fn tick_frame(&mut self, picture: &Picture, status_label: &Label, window: &ApplicationWindow) {
        let min_dt = if self.prefs.throttle {
            Duration::from_millis(20)
        } else {
            Duration::from_millis(0)
        };
        if min_dt > Duration::ZERO && self.last_frame.elapsed() < min_dt {
            return;
        }
        self.last_frame = Instant::now();

        let (pcm_snap, window_title, status_text, w, h, rgba) = self.host.with_mut(|s| {
            let _ = s.run_frame();
            let pcm = s.audio_pcm().to_vec();
            let status = s.status().to_owned();
            let window_title = match s.media_title() {
                Some(t) => format!("Spec Chum — {t}"),
                None => format!("Spec Chum — {status}"),
            };
            let w = s.width();
            let h = s.height();
            let rgba = s.framebuffer().to_vec();
            (pcm, window_title, status, w, h, rgba)
        });

        self.pcm.lock().push_frame(&pcm_snap);
        status_label.set_text(&status_text);
        window.set_title(Some(&window_title));
        self.maybe_refresh_debug_text();

        if w > 0 && h > 0 && rgba.len() >= w * h * 4 {
            let bytes = glib::Bytes::from_owned(rgba);
            let texture = gdk4::MemoryTexture::new(
                i32::try_from(w).unwrap_or(1),
                i32::try_from(h).unwrap_or(1),
                gdk4::MemoryFormat::R8g8b8a8,
                &bytes,
                w.saturating_mul(4),
            );
            picture.set_paintable(Some(&texture));
        }
    }
}

/// Load the requested startup model, falling back to 48K only when another
/// model's ROM is unavailable. Failure of the fallback retains both causes.
fn select_startup_model(
    preferred: ModelId,
    mut select: impl FnMut(ModelId) -> Result<(), HostError>,
) -> Result<ModelId, HostError> {
    match select(preferred) {
        Ok(()) => Ok(preferred),
        Err(preferred_error) if preferred == ModelId::Spectrum48 => Err(preferred_error),
        Err(preferred_error) => select(ModelId::Spectrum48)
            .map(|()| ModelId::Spectrum48)
            .map_err(|fallback_error| {
                HostError::Message(format!(
                    "could not load preferred model {preferred:?} ({preferred_error}); 48K fallback also failed ({fallback_error})"
                ))
            }),
    }
}

/// Run the native GTK4 shell until the window closes.
pub fn run() -> Result<()> {
    let state = Rc::new(RefCell::new(AppState::new().context("init host session")?));

    let app = Application::builder().application_id(APP_ID).build();

    app.connect_activate(clone!(
        #[strong]
        state,
        move |app| {
            if let Some(window) = app.active_window() {
                window.present();
                return;
            }
            build_ui(app, Rc::clone(&state));
        }
    ));

    let code = app.run();
    if code != glib::ExitCode::SUCCESS {
        anyhow::bail!("GTK application exited with {code:?}");
    }
    Ok(())
}

fn build_ui(app: &Application, state: Rc<RefCell<AppState>>) {
    let window = ApplicationWindow::builder()
        .application(app)
        .title("Spec Chum")
        .default_width(800)
        .default_height(700)
        .show_menubar(true)
        .build();

    let picture = Picture::new();
    picture.set_keep_aspect_ratio(true);
    picture.set_hexpand(true);
    picture.set_vexpand(true);
    picture.set_can_focus(true);

    let status_label = Label::new(Some("Ready"));
    status_label.set_halign(gtk4::Align::Start);
    status_label.set_margin_start(8);
    status_label.set_margin_end(8);
    status_label.set_margin_top(4);
    status_label.set_margin_bottom(4);

    let vbox = gtk4::Box::new(Orientation::Vertical, 0);
    vbox.append(&picture);
    vbox.append(&status_label);
    window.set_child(Some(&vbox));

    install_actions(app, &window, Rc::clone(&state));
    install_menubar(app);
    install_keys(&window, &picture, Rc::clone(&state));

    let period = if state.borrow().prefs.throttle {
        Duration::from_millis(20)
    } else {
        Duration::from_millis(16)
    };
    glib::timeout_add_local(
        period,
        clone!(
            #[strong]
            state,
            #[strong]
            picture,
            #[strong]
            status_label,
            #[strong]
            window,
            move || {
                state
                    .borrow_mut()
                    .tick_frame(&picture, &status_label, &window);
                glib::ControlFlow::Continue
            }
        ),
    );

    window.present();
    let _ = picture.grab_focus();
}

fn install_menubar(app: &Application) {
    let menubar = gio::Menu::new();

    let file = gio::Menu::new();
    file.append(Some("Open Tape…"), Some("app.open_tape"));
    file.append(Some("Open Snapshot…"), Some("app.open_snapshot"));
    file.append(Some("Open RZX…"), Some("app.open_rzx"));
    file.append(Some("Open Disk (DSK)…"), Some("app.open_dsk"));
    file.append(Some("Open TRD…"), Some("app.open_trd"));
    file.append(Some("Quit"), Some("app.quit"));
    menubar.append_submenu(Some("_File"), &file);

    let machine = gio::Menu::new();
    for (i, m) in ModelId::ALL.iter().enumerate() {
        machine.append(
            Some(model_menu_label(*m)),
            Some(&format!("app.select_model_{i}")),
        );
    }
    machine.append(Some("Reset"), Some("app.machine_reset"));
    menubar.append_submenu(Some("_Machine"), &machine);

    let tape = gio::Menu::new();
    tape.append(Some("Play"), Some("app.tape_play"));
    tape.append(Some("Pause"), Some("app.tape_pause"));
    tape.append(Some("Rewind"), Some("app.tape_rewind"));
    menubar.append_submenu(Some("_Tape"), &tape);

    let hw = gio::Menu::new();
    hw.append(
        Some("Attach Multiface ROM…"),
        Some("app.hw_attach_multiface"),
    );
    hw.append(Some("Multiface NMI"), Some("app.hw_multiface_nmi"));
    hw.append(Some("Attach DivMMC"), Some("app.hw_attach_divmmc"));
    hw.append(Some("Open DivMMC SD image…"), Some("app.hw_divmmc_sd"));
    hw.append(
        Some("Open DivMMC SD (slot 1)…"),
        Some("app.hw_divmmc_sd_slot1"),
    );
    hw.append(Some("Open DivMMC EEPROM…"), Some("app.hw_divmmc_eeprom"));
    hw.append(Some("Attach Interface 1"), Some("app.hw_attach_if1"));
    hw.append(Some("Open Microdrive MDR…"), Some("app.hw_insert_mdr"));
    hw.append(Some("Attach Beta Disk"), Some("app.hw_attach_beta"));
    hw.append(Some("Load TR-DOS ROM…"), Some("app.hw_load_trdos_rom"));
    hw.append(Some("Open TRD…"), Some("app.hw_open_trd"));
    hw.append(Some("Insert Timex Dock…"), Some("app.hw_insert_dck"));
    hw.append(Some("Eject Timex Dock"), Some("app.hw_eject_dck"));
    menubar.append_submenu(Some("_Hardware"), &hw);

    let settings = gio::Menu::new();
    let joy = gio::Menu::new();
    joy.append(Some("Kempston"), Some("app.set_joy_kempston"));
    joy.append(Some("Sinclair Left"), Some("app.set_joy_sinclair_l"));
    joy.append(Some("Sinclair Right"), Some("app.set_joy_sinclair_r"));
    joy.append(Some("Cursor"), Some("app.set_joy_cursor"));
    settings.append_submenu(Some("Joystick"), &joy);
    let tape_mode = gio::Menu::new();
    tape_mode.append(Some("EAR 1×"), Some("app.set_tape_ear_1"));
    tape_mode.append(Some("EAR 2×"), Some("app.set_tape_ear_2"));
    tape_mode.append(Some("EAR 5×"), Some("app.set_tape_ear_5"));
    tape_mode.append(Some("EAR 10×"), Some("app.set_tape_ear_10"));
    tape_mode.append(Some("EAR 20×"), Some("app.set_tape_ear_20"));
    tape_mode.append(Some("Experience"), Some("app.set_tape_experience"));
    tape_mode.append(Some("Instant (session)"), Some("app.set_tape_instant"));
    settings.append_submenu(Some("Tape load"), &tape_mode);
    let ay = gio::Menu::new();
    ay.append(Some("Mono"), Some("app.set_ay_mono"));
    ay.append(Some("ACB"), Some("app.set_ay_acb"));
    ay.append(Some("ABC"), Some("app.set_ay_abc"));
    settings.append_submenu(Some("AY stereo"), &ay);
    settings.append(Some("Toggle Mute"), Some("app.toggle_mute"));
    settings.append(
        Some("Toggle Throttle (~50 Hz)"),
        Some("app.toggle_throttle"),
    );
    settings.append(
        Some("Toggle Online tape titles"),
        Some("app.toggle_online_titles"),
    );
    settings.append(
        Some("Toggle Kempston Mouse pref"),
        Some("app.toggle_kempston_mouse"),
    );
    menubar.append_submenu(Some("_Settings"), &settings);

    let debug = gio::Menu::new();
    debug.append(Some("Inspector…"), Some("app.dbg_toggle"));
    debug.append(Some("Pause"), Some("app.dbg_pause"));
    debug.append(Some("Continue"), Some("app.dbg_continue"));
    debug.append(Some("Step"), Some("app.dbg_step"));
    debug.append(Some("Breakpoint at PC"), Some("app.dbg_break_pc"));
    debug.append(Some("Clear breakpoints"), Some("app.dbg_clear_breaks"));
    debug.append(Some("Refresh"), Some("app.dbg_refresh"));
    menubar.append_submenu(Some("_Debug"), &debug);

    app.set_menubar(Some(&menubar));
}

fn install_actions(app: &Application, window: &ApplicationWindow, state: Rc<RefCell<AppState>>) {
    fn add_action(
        app: &Application,
        name: &str,
        state: Rc<RefCell<AppState>>,
        f: impl Fn(&mut AppState) + 'static,
    ) {
        let action = gio::SimpleAction::new(name, None);
        action.connect_activate(move |_, _| {
            f(&mut state.borrow_mut());
        });
        app.add_action(&action);
    }

    add_action(app, "open_tape", Rc::clone(&state), |s| {
        s.open_path("Open Tape", "Tape", &["tap", "tzx"], HostSession::open_tape);
    });
    add_action(app, "open_snapshot", Rc::clone(&state), |s| {
        s.open_path(
            "Open Snapshot",
            "Snapshot",
            &["sna", "z80"],
            HostSession::load_snapshot,
        );
    });
    add_action(app, "open_rzx", Rc::clone(&state), |s| {
        s.open_path("Open RZX", "RZX", &["rzx"], HostSession::load_rzx);
    });
    add_action(app, "open_dsk", Rc::clone(&state), |s| {
        s.open_path("Open Disk (DSK)", "Disk", &["dsk"], HostSession::load_dsk);
    });
    add_action(app, "open_trd", Rc::clone(&state), |s| {
        s.open_path("Open TRD", "TRD", &["trd"], HostSession::load_trd);
    });
    add_action(app, "tape_play", Rc::clone(&state), |s| {
        s.host_action("Play tape", HostSession::play_tape);
    });
    add_action(app, "tape_pause", Rc::clone(&state), |s| {
        s.host_action("Pause tape", HostSession::pause_tape);
    });
    add_action(app, "tape_rewind", Rc::clone(&state), |s| {
        s.host_action("Rewind tape", HostSession::rewind_tape);
    });

    for (i, m) in ModelId::ALL.iter().enumerate() {
        let model = *m;
        add_action(
            app,
            &format!("select_model_{i}"),
            Rc::clone(&state),
            move |s| {
                s.select_model(model);
            },
        );
    }
    add_action(app, "machine_reset", Rc::clone(&state), |s| {
        s.reset_machine();
    });

    add_action(app, "hw_attach_multiface", Rc::clone(&state), |s| {
        s.open_path(
            "Attach Multiface ROM",
            "Multiface ROM",
            &["rom", "bin"],
            HostSession::attach_multiface,
        );
    });
    add_action(app, "hw_multiface_nmi", Rc::clone(&state), |s| {
        s.host_action("Multiface NMI", HostSession::multiface_nmi);
    });
    add_action(app, "hw_attach_divmmc", Rc::clone(&state), |s| {
        s.host_action("Attach DivMMC", HostSession::attach_divmmc);
    });
    add_action(app, "hw_divmmc_sd", Rc::clone(&state), |s| {
        s.open_path(
            "Open DivMMC SD",
            "SD image",
            &["img", "bin", "mmc", "sd"],
            HostSession::load_divmmc_sd,
        );
    });
    add_action(app, "hw_divmmc_sd_slot1", Rc::clone(&state), |s| {
        s.open_path(
            "Open DivMMC SD (slot 1)",
            "SD image",
            &["img", "bin", "mmc", "sd"],
            |sess, p| sess.load_divmmc_sd_slot(p, 1),
        );
    });
    add_action(app, "hw_divmmc_eeprom", Rc::clone(&state), |s| {
        s.open_path(
            "Open DivMMC EEPROM",
            "EEPROM / ESXDOS",
            &["rom", "bin", "eeprom"],
            HostSession::load_divmmc_eeprom,
        );
    });
    add_action(app, "hw_attach_if1", Rc::clone(&state), |s| {
        s.host_action("Attach Interface 1", HostSession::attach_interface1);
    });
    add_action(app, "hw_insert_mdr", Rc::clone(&state), |s| {
        s.open_path(
            "Open Microdrive MDR",
            "MDR",
            &["mdr"],
            HostSession::insert_mdr,
        );
    });
    add_action(app, "hw_attach_beta", Rc::clone(&state), |s| {
        s.host_action("Attach Beta Disk", HostSession::attach_beta);
    });
    add_action(app, "hw_load_trdos_rom", Rc::clone(&state), |s| {
        s.open_path(
            "Load TR-DOS ROM",
            "TR-DOS ROM",
            &["rom", "bin"],
            HostSession::load_trdos_rom,
        );
    });
    add_action(app, "hw_open_trd", Rc::clone(&state), |s| {
        s.open_path("Open TRD", "TRD", &["trd"], HostSession::load_trd);
    });
    add_action(app, "hw_insert_dck", Rc::clone(&state), |s| {
        s.open_path(
            "Insert Timex Dock",
            "Timex dock",
            &["dck"],
            HostSession::insert_dck,
        );
    });
    add_action(app, "hw_eject_dck", Rc::clone(&state), |s| {
        s.host_action("Eject Timex Dock", HostSession::eject_dck);
    });

    add_action(app, "set_joy_kempston", Rc::clone(&state), |s| {
        s.set_joystick(spec_chum_host::PrefJoystick::Kempston);
    });
    add_action(app, "set_joy_sinclair_l", Rc::clone(&state), |s| {
        s.set_joystick(spec_chum_host::PrefJoystick::SinclairLeft);
    });
    add_action(app, "set_joy_sinclair_r", Rc::clone(&state), |s| {
        s.set_joystick(spec_chum_host::PrefJoystick::SinclairRight);
    });
    add_action(app, "set_joy_cursor", Rc::clone(&state), |s| {
        s.set_joystick(spec_chum_host::PrefJoystick::Cursor);
    });
    add_action(app, "set_tape_ear_1", Rc::clone(&state), |s| {
        s.set_tape_ear_speed(1);
    });
    add_action(app, "set_tape_ear_2", Rc::clone(&state), |s| {
        s.set_tape_ear_speed(2);
    });
    add_action(app, "set_tape_ear_5", Rc::clone(&state), |s| {
        s.set_tape_ear_speed(5);
    });
    add_action(app, "set_tape_ear_10", Rc::clone(&state), |s| {
        s.set_tape_ear_speed(10);
    });
    add_action(app, "set_tape_ear_20", Rc::clone(&state), |s| {
        s.set_tape_ear_speed(20);
    });
    add_action(app, "set_tape_experience", Rc::clone(&state), |s| {
        s.set_tape_experience();
    });
    add_action(app, "set_tape_instant", Rc::clone(&state), |s| {
        s.set_tape_instant();
    });
    add_action(app, "set_ay_mono", Rc::clone(&state), |s| {
        s.set_ay(PrefAyStereo::Mono);
    });
    add_action(app, "set_ay_acb", Rc::clone(&state), |s| {
        s.set_ay(PrefAyStereo::Acb);
    });
    add_action(app, "set_ay_abc", Rc::clone(&state), |s| {
        s.set_ay(PrefAyStereo::Abc);
    });
    add_action(app, "toggle_mute", Rc::clone(&state), |s| {
        s.toggle_mute();
    });
    add_action(app, "toggle_throttle", Rc::clone(&state), |s| {
        s.toggle_throttle();
    });
    add_action(app, "toggle_online_titles", Rc::clone(&state), |s| {
        s.toggle_online_titles();
    });
    add_action(app, "toggle_kempston_mouse", Rc::clone(&state), |s| {
        s.toggle_kempston_mouse();
    });

    {
        let state = Rc::clone(&state);
        let window = window.clone();
        let action = gio::SimpleAction::new("dbg_toggle", None);
        action.connect_activate(move |_, _| {
            let state2 = Rc::clone(&state);
            state.borrow_mut().toggle_debug_window(&window, &state2);
        });
        app.add_action(&action);
    }
    add_action(app, "dbg_pause", Rc::clone(&state), |s| {
        s.debug_pause();
    });
    add_action(app, "dbg_continue", Rc::clone(&state), |s| {
        s.debug_continue();
    });
    add_action(app, "dbg_step", Rc::clone(&state), |s| {
        s.debug_step();
    });
    add_action(app, "dbg_break_pc", Rc::clone(&state), |s| {
        s.debug_break_at_pc();
    });
    add_action(app, "dbg_clear_breaks", Rc::clone(&state), |s| {
        s.debug_clear_breaks();
    });
    add_action(app, "dbg_refresh", Rc::clone(&state), |s| {
        s.refresh_debug_text();
    });

    let quit = gio::SimpleAction::new("quit", None);
    quit.connect_activate(clone!(
        #[strong]
        window,
        move |_, _| {
            window.close();
        }
    ));
    app.add_action(&quit);

    app.set_accels_for_action("app.open_tape", &["<Control>o"]);
    app.set_accels_for_action("app.quit", &["<Control>q"]);
}

fn keyval_from_gdk(key: gdk4::Key) -> u32 {
    key.into_glib()
}

fn install_keys(window: &ApplicationWindow, picture: &Picture, state: Rc<RefCell<AppState>>) {
    let controller = gtk4::EventControllerKey::new();
    controller.set_propagation_phase(gtk4::PropagationPhase::Capture);

    controller.connect_key_pressed(clone!(
        #[strong]
        state,
        move |_, key, _keycode, _mods| {
            state.borrow_mut().on_key(keyval_from_gdk(key), true);
            glib::Propagation::Proceed
        }
    ));

    controller.connect_key_released(clone!(
        #[strong]
        state,
        move |_, key, _keycode, _mods| {
            state.borrow_mut().on_key(keyval_from_gdk(key), false);
        }
    ));

    window.add_controller(controller);

    let focus = gtk4::EventControllerFocus::new();
    focus.connect_leave(clone!(
        #[strong]
        state,
        move |_| {
            state.borrow_mut().clear_all_input();
        }
    ));
    window.add_controller(focus);

    let pic_controller = gtk4::EventControllerKey::new();
    pic_controller.connect_key_pressed(clone!(
        #[strong]
        state,
        move |_, key, _keycode, _mods| {
            state.borrow_mut().on_key(keyval_from_gdk(key), true);
            glib::Propagation::Stop
        }
    ));
    pic_controller.connect_key_released(clone!(
        #[strong]
        state,
        move |_, key, _keycode, _mods| {
            state.borrow_mut().on_key(keyval_from_gdk(key), false);
        }
    ));
    picture.add_controller(pic_controller);
}

#[cfg(test)]
mod startup_model_tests {
    use super::*;

    #[test]
    fn falls_back_to_48k_when_preferred_model_fails() {
        let mut attempted = Vec::new();
        let selected = select_startup_model(ModelId::Spectrum128, |model| {
            attempted.push(model);
            if model == ModelId::Spectrum128 {
                Err(HostError::Message("128K ROM missing".into()))
            } else {
                Ok(())
            }
        });

        assert!(matches!(selected, Ok(ModelId::Spectrum48)));
        assert_eq!(attempted, [ModelId::Spectrum128, ModelId::Spectrum48]);
    }

    #[test]
    fn reports_both_preferred_and_fallback_failures() {
        let selected = select_startup_model(ModelId::Spectrum128, |model| {
            Err(HostError::Message(format!("{model:?} ROM missing")))
        });

        let Err(HostError::Message(message)) = selected else {
            panic!("expected a combined startup model error");
        };
        assert!(message.contains("Spectrum128 ROM missing"));
        assert!(message.contains("Spectrum48 ROM missing"));
    }

    #[test]
    fn does_not_retry_48k_selection_after_failure() {
        let mut attempts = 0;
        let selected = select_startup_model(ModelId::Spectrum48, |_| {
            attempts += 1;
            Err(HostError::Message("48K ROM missing".into()))
        });

        assert!(selected.is_err());
        assert_eq!(attempts, 1);
    }
}
