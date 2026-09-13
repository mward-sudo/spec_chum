//! GTK4 application host for Spec Chum on Linux (#351 vertical slice).

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
use gtk4::{gio, glib, Application, ApplicationWindow, Label, Orientation, Picture};
use parking_lot::Mutex;
use spec_chum_host::{
    default_prefs_path, load_prefs, save_prefs, sync_model_rom_paths, HostError, HostSession,
    ModelId, PrefModel, UiPreferences,
};

use linux_shell::audio::{self, PcmRing};
use linux_shell::keymap::{
    apply_chord, apply_modifiers, chord_for_keyval, is_modifier_keyval, suppresses_modifier_caps,
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
    status: String,
}

impl AppState {
    fn new() -> Result<Self> {
        let prefs_path = default_prefs_path();
        let mut prefs = load_prefs(&prefs_path);
        sync_model_rom_paths(prefs.model_rom_paths.clone());

        let model = prefs.model.to_model_id();
        let mut session = HostSession::new(model, true);
        session.set_model(model);
        if !session.has_machine() {
            session.set_model(ModelId::Spectrum48);
            prefs.select_builtin_model(PrefModel::Spectrum48);
        }
        if !session.has_machine() {
            anyhow::bail!("48K ROM not found — run ./scripts/fetch_roms.sh from the repo root");
        }

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
            status: "Ready".into(),
        })
    }

    fn persist_prefs(&self) {
        if let Err(e) = save_prefs(&self.prefs_path, &self.prefs) {
            eprintln!("spec-chum-linux: save prefs failed: {e}");
        }
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
        let mut suppress = false;
        for &kv in &self.held {
            if suppresses_modifier_caps(kv) {
                suppress = true;
            }
        }
        let shift = self.shift();
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
        self.host.with_mut(|s| s.set_status(msg.clone()));
        self.status = msg;
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
        self.status = status_text.clone();
        status_label.set_text(&status_text);
        window.set_title(Some(&window_title));

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

    // ~50 Hz frame pump when unthrottled; 20 ms when prefs.throttle is on.
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

    let tape = gio::Menu::new();
    tape.append(Some("Play"), Some("app.tape_play"));
    tape.append(Some("Pause"), Some("app.tape_pause"));
    tape.append(Some("Rewind"), Some("app.tape_rewind"));
    menubar.append_submenu(Some("_Tape"), &tape);

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
    // Keymap normalizes A–Z; modifiers keep their GDK keyvals.
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
            // Keep focus for continuous gameplay keys; let menu accelerators still fire.
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

    // Also attach to the picture so key focus works after clicking the display.
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
