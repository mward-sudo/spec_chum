//! Spec Chum — ZX Spectrum emulator frontend.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use eframe::egui;
use machine::{Machine, Model};

fn main() -> eframe::Result {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([720.0, 620.0])
            .with_title("Spec Chum"),
        ..Default::default()
    };
    eframe::run_native(
        "Spec Chum",
        options,
        Box::new(|_cc| Ok(Box::new(SpecChumApp::new()))),
    )
}

struct SpecChumApp {
    machine: Option<Machine>,
    framebuffer: Vec<u8>,
    texture: Option<egui::TextureHandle>,
    width: usize,
    height: usize,
    with_border: bool,
    running: bool,
    throttle: bool,
    muted: bool,
    status: String,
    model: Model,
    beeper: Arc<Mutex<BeeperState>>,
    _stream: Option<cpal::Stream>,
}

struct BeeperState {
    edges: Vec<(u32, bool)>,
    level: bool,
    sample_rate: u32,
    frame_t_per_sample: f32,
    t: f32,
}

impl Default for BeeperState {
    fn default() -> Self {
        Self {
            edges: Vec::new(),
            level: false,
            sample_rate: 44100,
            frame_t_per_sample: 69888.0 / 44100.0,
            t: 0.0,
        }
    }
}

impl SpecChumApp {
    fn new() -> Self {
        let with_border = true;
        let (width, height) = if with_border { (352, 296) } else { (256, 192) };
        let beeper = Arc::new(Mutex::new(BeeperState {
            frame_t_per_sample: 69888.0 / 44100.0,
            ..BeeperState::default()
        }));
        let stream = start_beeper(Arc::clone(&beeper));
        let mut app = Self {
            machine: None,
            framebuffer: vec![0; width * height * 4],
            texture: None,
            width,
            height,
            with_border,
            running: true,
            throttle: true,
            muted: false,
            status: "Load a ROM via Machine menu (or auto-detect roms/)".into(),
            model: Model::Spectrum48,
            beeper,
            _stream: stream,
        };
        app.try_autoload_rom();
        app
    }

    fn try_autoload_rom(&mut self) {
        let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..");
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

    fn apply_key(&mut self, key: egui::Key, pressed: bool) {
        let Some(machine) = self.machine.as_mut() else {
            return;
        };
        let kb = machine.keyboard_mut();
        // Spectrum matrix mapping (common PC layout)
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

    fn apply_modifiers(&mut self, modifiers: egui::Modifiers) {
        let Some(machine) = self.machine.as_mut() else {
            return;
        };
        let kb = machine.keyboard_mut();
        kb.set_key(0, 0, modifiers.shift); // Caps Shift
        kb.set_key(7, 1, modifiers.ctrl || modifiers.alt); // Symbol Shift
    }

    fn load_snapshot(&mut self, path: &std::path::Path) {
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

    fn load_tap(&mut self, path: &std::path::Path) {
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
}

impl eframe::App for SpecChumApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::TopBottomPanel::top("menu").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open snapshot (SNA/Z80)…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("Snapshots", &["sna", "z80"])
                            .pick_file()
                        {
                            self.load_snapshot(&path);
                        }
                        ui.close_menu();
                    }
                    if ui.button("Open TAP…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("TAP", &["tap"])
                            .pick_file()
                        {
                            self.load_tap(&path);
                        }
                        ui.close_menu();
                    }
                    if ui.button("Quit").clicked() {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button("Machine", |ui| {
                    if ui
                        .radio_value(&mut self.model, Model::Spectrum48, "Spectrum 48K")
                        .clicked()
                    {
                        self.try_autoload_rom();
                    }
                    if ui
                        .radio_value(&mut self.model, Model::Spectrum128, "Spectrum 128K")
                        .clicked()
                    {
                        self.try_autoload_rom();
                    }
                    if ui.button("Reset").clicked() {
                        if let Some(m) = self.machine.as_mut() {
                            m.reset();
                            self.status = "Reset".into();
                        }
                        ui.close_menu();
                    }
                    ui.checkbox(&mut self.running, "Running");
                    ui.checkbox(&mut self.throttle, "Throttle ~50Hz");
                    ui.checkbox(&mut self.muted, "Mute");
                });
                ui.menu_button("Help", |ui| {
                    ui.label("Spec Chum — from-scratch ZX Spectrum emulator");
                });
                ui.label(&self.status);
            });
        });

        // Keyboard
        ctx.input(|i| {
            for event in &i.events {
                if let egui::Event::Key {
                    key,
                    pressed,
                    repeat,
                    ..
                } = event
                {
                    if !repeat {
                        // need mutable self — collect then apply
                        let _ = (key, pressed);
                    }
                }
            }
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
            self.apply_key(key, pressed);
        }
        let modifiers = ctx.input(|i| i.modifiers);
        self.apply_modifiers(modifiers);

        if self.running {
            if let Some(machine) = self.machine.as_mut() {
                let edges = machine.run_frame();
                if !self.muted {
                    if let Ok(mut b) = self.beeper.lock() {
                        b.edges = edges;
                        b.t = 0.0;
                    }
                }
                machine.render_rgba(&mut self.framebuffer, self.with_border);
            }
        }

        egui::CentralPanel::default().show(ctx, |ui| {
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [self.width, self.height],
                &self.framebuffer,
            );
            let tex = self.texture.get_or_insert_with(|| {
                ctx.load_texture("screen", image.clone(), egui::TextureOptions::NEAREST)
            });
            tex.set(image, egui::TextureOptions::NEAREST);
            let scale = 2.0;
            ui.image((
                tex.id(),
                egui::vec2(self.width as f32 * scale, self.height as f32 * scale),
            ));
        });

        if self.running {
            if self.throttle {
                ctx.request_repaint_after(std::time::Duration::from_millis(20));
            } else {
                ctx.request_repaint();
            }
        }
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
        s.frame_t_per_sample = 69888.0 * 50.0 / f32::from(sample_rate as u16);
        // Prefer: samples per frame ≈ sample_rate/50, T per sample = 69888 / that
        s.frame_t_per_sample = 69888.0 / (sample_rate as f32 / 50.0);
    }
    let stream = device
        .build_output_stream(
            &config.config(),
            move |data: &mut [f32], _| {
                let Ok(mut st) = state.lock() else {
                    return;
                };
                for sample in data.iter_mut() {
                    // Advance through edges
                    while let Some(&(edge_t, level)) = st.edges.first() {
                        if st.t >= edge_t as f32 {
                            st.level = level;
                            st.edges.remove(0);
                        } else {
                            break;
                        }
                    }
                    let amp = if st.level { 0.2 } else { -0.2 };
                    *sample = amp;
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
