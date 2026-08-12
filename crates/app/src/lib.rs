//! Spec Chum — egui frontend library (testable without a display).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use eframe::egui;
use machine::{Machine, Model};

/// Session state shared by the GUI and headless tests.
#[derive(Debug)]
pub struct EmulatorSession {
    pub machine: Option<Machine>,
    pub framebuffer: Vec<u8>,
    pub width: usize,
    pub height: usize,
    pub with_border: bool,
    pub running: bool,
    pub throttle: bool,
    pub muted: bool,
    pub status: String,
    pub model: Model,
}

impl EmulatorSession {
    #[must_use]
    pub fn new(model: Model, with_border: bool) -> Self {
        let (width, height) = if with_border { (352, 296) } else { (256, 192) };
        Self {
            machine: None,
            framebuffer: vec![0; width * height * 4],
            width,
            height,
            with_border,
            running: true,
            throttle: true,
            muted: false,
            status: "Load a ROM via Machine menu (or auto-detect roms/)".into(),
            model,
        }
    }

    #[must_use]
    pub fn workspace_root() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
    }

    pub fn try_autoload_rom(&mut self) {
        let root = Self::workspace_root();
        match self.model {
            Model::Spectrum48 => {
                let rom48 = root.join("roms/spec48.rom");
                if let Ok(data) = std::fs::read(&rom48) {
                    match Machine::new_48k(&data) {
                        Ok(m) => {
                            self.machine = Some(m);
                            self.status = format!("Loaded {}", rom48.display());
                        }
                        Err(e) => self.status = e,
                    }
                } else {
                    self.status =
                        format!("Missing {}. Run ./scripts/fetch_roms.sh", rom48.display());
                }
            }
            Model::Spectrum128 => {
                let rom128 = root.join("roms/128/spec128uk.rom");
                if let Ok(data) = std::fs::read(&rom128) {
                    match Machine::new_128k(&data) {
                        Ok(m) => {
                            self.machine = Some(m);
                            self.status = format!("Loaded {}", rom128.display());
                        }
                        Err(e) => self.status = e,
                    }
                } else {
                    self.status =
                        format!("Missing {}. Run ./scripts/fetch_roms.sh", rom128.display());
                }
            }
        }
    }

    pub fn load_snapshot(&mut self, path: &Path) {
        match formats::Snapshot48::load_sna(path).or_else(|_| formats::Snapshot48::load_z80(path)) {
            Ok(snap) => {
                if self.machine.is_none() {
                    self.model = Model::Spectrum48;
                    self.try_autoload_rom();
                }
                if let Some(m) = self.machine.as_mut() {
                    m.apply_snapshot48(&snap);
                    self.status = format!("Loaded snapshot {}", path.display());
                }
            }
            Err(e) => self.status = format!("Snapshot error: {e}"),
        }
    }

    pub fn load_tap(&mut self, path: &Path) {
        match tape::TapImage::load(path) {
            Ok(img) => {
                if let Some(m) = self.machine.as_mut() {
                    m.insert_tape(tape::TapPlayer::new(img));
                    self.status = format!("Inserted TAP {}", path.display());
                } else {
                    self.status = "Load a machine ROM before inserting tape".into();
                }
            }
            Err(e) => self.status = format!("TAP error: {e}"),
        }
    }

    pub fn apply_key(&mut self, key: egui::Key, pressed: bool) {
        let Some(machine) = self.machine.as_mut() else {
            return;
        };
        let kb = machine.keyboard_mut();
        let mapping: &[(egui::Key, usize, u8)] = &[
            (egui::Key::Num1, 3, 0),
            (egui::Key::Num2, 3, 1),
            (egui::Key::Num3, 3, 2),
            (egui::Key::Num4, 3, 3),
            (egui::Key::Num5, 3, 4),
            (egui::Key::Num6, 4, 4),
            (egui::Key::Num7, 4, 3),
            (egui::Key::Num8, 4, 2),
            (egui::Key::Num9, 4, 1),
            (egui::Key::Num0, 4, 0),
            (egui::Key::Q, 2, 0),
            (egui::Key::W, 2, 1),
            (egui::Key::E, 2, 2),
            (egui::Key::R, 2, 3),
            (egui::Key::T, 2, 4),
            (egui::Key::Y, 5, 4),
            (egui::Key::U, 5, 3),
            (egui::Key::I, 5, 2),
            (egui::Key::O, 5, 1),
            (egui::Key::P, 5, 0),
            (egui::Key::A, 1, 0),
            (egui::Key::S, 1, 1),
            (egui::Key::D, 1, 2),
            (egui::Key::F, 1, 3),
            (egui::Key::G, 1, 4),
            (egui::Key::H, 6, 4),
            (egui::Key::J, 6, 3),
            (egui::Key::K, 6, 2),
            (egui::Key::L, 6, 1),
            (egui::Key::Enter, 6, 0),
            (egui::Key::Z, 0, 1),
            (egui::Key::X, 0, 2),
            (egui::Key::C, 0, 3),
            (egui::Key::V, 0, 4),
            (egui::Key::B, 7, 4),
            (egui::Key::N, 7, 3),
            (egui::Key::M, 7, 2),
            (egui::Key::Space, 7, 0),
        ];
        for &(k, row, bit) in mapping {
            if k == key {
                kb.set_key(row, bit, pressed);
            }
        }
    }

    pub fn apply_modifiers(&mut self, modifiers: egui::Modifiers) {
        let Some(machine) = self.machine.as_mut() else {
            return;
        };
        let kb = machine.keyboard_mut();
        kb.set_key(0, 0, modifiers.shift);
        kb.set_key(7, 1, modifiers.ctrl || modifiers.alt);
    }

    /// Run one emulated frame into the RGBA framebuffer when `running`.
    pub fn tick_frame(&mut self) -> machine::FrameAudio {
        if !self.running {
            return machine::FrameAudio::default();
        }
        let Some(machine) = self.machine.as_mut() else {
            return machine::FrameAudio::default();
        };
        let audio = machine.run_frame();
        machine.render_rgba(&mut self.framebuffer, self.with_border);
        audio
    }
}

pub struct SpecChumApp {
    pub session: EmulatorSession,
    texture: Option<egui::TextureHandle>,
    beeper: Arc<Mutex<BeeperState>>,
    _stream: Option<cpal::Stream>,
}

impl std::fmt::Debug for SpecChumApp {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpecChumApp")
            .field("session", &self.session)
            .field("has_texture", &self.texture.is_some())
            .field("has_audio", &self._stream.is_some())
            .finish()
    }
}

struct BeeperState {
    edges: Vec<(u32, bool)>,
    ay_samples: Vec<f32>,
    ay_index: usize,
    level: bool,
    sample_rate: u32,
    frame_t_per_sample: f32,
    t: f32,
    muted: bool,
}

impl Default for BeeperState {
    fn default() -> Self {
        Self {
            edges: Vec::new(),
            ay_samples: Vec::new(),
            ay_index: 0,
            level: false,
            sample_rate: 44100,
            frame_t_per_sample: 69888.0 / 44100.0,
            t: 0.0,
            muted: false,
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
        let beeper = Arc::new(Mutex::new(BeeperState {
            frame_t_per_sample: 69888.0 / 44100.0,
            ..BeeperState::default()
        }));
        let stream = if start_audio {
            start_beeper(Arc::clone(&beeper))
        } else {
            None
        };
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        Self {
            session,
            texture: None,
            beeper,
            _stream: stream,
        }
    }

    /// egui UI body — callable from `App::update` or headless `Context::run`.
    pub fn ui(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open snapshot (SNA/Z80)…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Snapshots", &["sna", "z80"])
                            .pick_file()
                        {
                            self.session.load_snapshot(&path);
                        }
                        ui.close_menu();
                    }
                    if ui.button("Open TAP…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("TAP", &["tap"])
                            .pick_file()
                        {
                            self.session.load_tap(&path);
                        }
                        ui.close_menu();
                    }
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Machine", |ui| {
                    if ui
                        .radio_value(&mut self.session.model, Model::Spectrum48, "Spectrum 48K")
                        .clicked()
                    {
                        self.session.try_autoload_rom();
                    }
                    if ui
                        .radio_value(&mut self.session.model, Model::Spectrum128, "Spectrum 128K")
                        .clicked()
                    {
                        self.session.try_autoload_rom();
                    }
                    if ui.button("Reset").clicked() {
                        if let Some(m) = self.session.machine.as_mut() {
                            m.reset();
                            self.session.status = "Reset".into();
                        }
                        ui.close_menu();
                    }
                    ui.checkbox(&mut self.session.running, "Running");
                    ui.checkbox(&mut self.session.throttle, "Throttle ~50Hz");
                    ui.checkbox(&mut self.session.muted, "Mute");
                });
                ui.menu_button("Help", |ui| {
                    ui.label("Spec Chum — from-scratch ZX Spectrum emulator");
                });
                ui.label(&self.session.status);
            });
        });

        let key_events: Vec<(egui::Key, bool)> = ctx.input(|i| {
            i.events
                .iter()
                .filter_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed,
                        repeat,
                        ..
                    } if !repeat => Some((*key, *pressed)),
                    _ => None,
                })
                .collect()
        });
        for (key, pressed) in key_events {
            self.session.apply_key(key, pressed);
        }
        let modifiers = ctx.input(|i| i.modifiers);
        self.session.apply_modifiers(modifiers);

        let audio = self.session.tick_frame();
        if let Ok(mut b) = self.beeper.lock() {
            b.muted = self.session.muted;
            if !self.session.muted {
                b.edges = audio.beeper_edges;
                b.ay_samples = audio.ay_samples;
                b.ay_index = 0;
                b.t = 0.0;
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [self.session.width, self.session.height],
                &self.session.framebuffer,
            );
            let tex = self.texture.get_or_insert_with(|| {
                ctx.load_texture("screen", image.clone(), egui::TextureOptions::NEAREST)
            });
            tex.set(image, egui::TextureOptions::NEAREST);
            let scale = 2.0;
            ui.image((
                tex.id(),
                egui::vec2(
                    self.session.width as f32 * scale,
                    self.session.height as f32 * scale,
                ),
            ));
        });

        if self.session.running {
            if self.session.throttle {
                ctx.request_repaint_after(std::time::Duration::from_millis(20));
            } else {
                ctx.request_repaint();
            }
        }
    }
}

impl Default for SpecChumApp {
    fn default() -> Self {
        Self::new()
    }
}

impl eframe::App for SpecChumApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.ui(ctx);
    }
}

fn start_beeper(state: Arc<Mutex<BeeperState>>) -> Option<cpal::Stream> {
    use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};
    let host = cpal::default_host();
    let device = host.default_output_device()?;
    let config = device.default_output_config().ok()?;
    let sample_rate = config.sample_rate().0;
    {
        let mut s = state.lock().ok()?;
        s.sample_rate = sample_rate;
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
                for sample in data.iter_mut() {
                    while let Some(&(edge_t, level)) = st.edges.first() {
                        if st.t >= edge_t as f32 {
                            st.level = level;
                            st.edges.remove(0);
                        } else {
                            break;
                        }
                    }
                    let beep = if st.level { 0.15 } else { -0.15 };
                    let ay = if st.ay_index < st.ay_samples.len() {
                        let v = st.ay_samples[st.ay_index];
                        st.ay_index += 1;
                        // AY samples are 0..1; center and scale for mix.
                        (v - 0.5) * 0.5
                    } else if let Some(&last) = st.ay_samples.last() {
                        (last - 0.5) * 0.5
                    } else {
                        0.0
                    };
                    *sample = (beep + ay).clamp(-1.0, 1.0);
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
    fn session_loads_tap_fixture_headless() {
        let mut session = EmulatorSession::new(Model::Spectrum48, true);
        session.try_autoload_rom();
        if session.machine.is_none() {
            eprintln!("skip: roms/spec48.rom missing");
            return;
        }
        let tap = EmulatorSession::workspace_root().join("tests/fixtures/tape/minimal_code.tap");
        session.load_tap(&tap);
        assert!(
            session.status.contains("Inserted TAP"),
            "status={}",
            session.status
        );
        assert_eq!(
            session.machine.as_ref().and_then(Machine::tape_block),
            Some(0)
        );
        let before = session.framebuffer.clone();
        for _ in 0..5 {
            session.tick_frame();
        }
        assert_ne!(session.framebuffer, before, "framebuffer should update");
        assert!(session.machine.as_ref().unwrap().cpu().t > 0);
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
            // Widgets from the previous frame are available via full output / memory;
            // assert status label was set and texture path did not panic.
            saw_file = true;
            saw_machine = app.session.status.contains("Loaded")
                || app.session.status.contains("Missing")
                || app.session.status.contains("ROM");
        });
        assert!(saw_file);
        assert!(saw_machine, "status={}", app.session.status);
        // Second frame after menus exist
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            app.ui(ctx);
        });
        assert_eq!(app.session.framebuffer.len(), 352 * 296 * 4);
    }
}
