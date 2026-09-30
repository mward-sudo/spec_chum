//! Application menu bar and top status area.

use super::{
    theme, AyStereoMode, JoystickMode, Machine, Model, SpecChumApp, TapeLoadOptions,
    UserMachineConfig, MAPPING_DOC,
};
use eframe::egui;
use spec_chum_host::{model_rom_available, PrefAyStereo, PrefJoystick, PrefModel};
use std::path::Path;

impl SpecChumApp {
    pub(super) fn menu_bar(&mut self, ctx: &egui::Context) {
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
                                    self.open_recent_path(Path::new(&path_str));
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
                                            self.prefs.sync_machine_fields_from_config(&active);
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
                            | Model::SpectrumPlus3e
                            | Model::Pentagon128
                            | Model::ScorpionZs256
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

                    if matches!(
                        model,
                        Model::Spectrum16K
                            | Model::Spectrum48
                            | Model::TimexTC2048
                            | Model::TimexTS2068
                            | Model::Spectrum128
                            | Model::SpectrumPlus2
                            | Model::Pentagon128
                            | Model::ScorpionZs256
                    ) {
                        let mf_label = if matches!(
                            model,
                            Model::Spectrum128 | Model::SpectrumPlus2 | Model::Pentagon128 | Model::ScorpionZs256
                        ) {
                            "Attach Multiface 128 ROM…"
                        } else {
                            "Attach Multiface 1 ROM…"
                        };
                        if ui.button(mf_label).clicked() {
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
                        ui.label("Multiface: 48K-class (MF1) or 128K/+2 (MF128); not +2A/+3");
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
                            | Model::ScorpionZs256
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
                                egui::Button::new("Open DivMMC SD image (slot 1)…"),
                            )
                            .clicked()
                        {
                            if let Some(path) = rfd::FileDialog::new()
                                .add_filter("SD image", &["img", "bin", "mmc", "sd"])
                                .pick_file()
                            {
                                self.session.attach_divmmc_sd_slot(&path, 1);
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

                    if model == Model::SpectrumPlus3 || model == Model::SpectrumPlus3e {
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
                        .on_hover_text(format!(
                            "Always asks for a TAP/TZX, then flash-loads (Type LOAD \"\" + Play). Decks with a custom loader (pulse TZX) have no flash trap and load off EAR at {}× instead — never at the EAR speed below. Play alone stays EAR-only. Use File → Open DSK for disks.",
                            machine::INSTANT_EAR_FALLBACK_SPEED,
                        ))
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
                                        "Hybrid flash + cosmetic abbreviated pilots (~20s-class; #82 / #167). Pulse-only decks fall back to EAR at 16×.",
                                    )
                                    .clicked()
                                {
                                    m.set_tape_load_options(TapeLoadOptions::experience());
                                    tape_status =
                                        Some("Tape: experience load (~20s EAR)".into());
                                    tape_prefs_changed = true;
                                }
                                ui.label("EAR speed:").on_hover_text(format!(
                                    "Play (EAR) loading only — the loaded program always runs at 1×. Instant ignores this: flashable decks poke bytes at LD-BYTES, custom-loader decks load off EAR at {}×.",
                                    machine::INSTANT_EAR_FALLBACK_SPEED,
                                ));
                                for speed in [1u32, 2, 5, 10, 20, 64] {
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
                    ui.separator();
                    if ui
                        .checkbox(
                            &mut self.prefs.online_tape_titles,
                            "Look up tape titles online (ZXInfo)",
                        )
                        .on_hover_text(
                            "Opt-in (default off). Sends a SHA-512 of the opened tape file to api.zxinfo.dk — not the path or bytes. Failures keep the filename.",
                        )
                        .changed()
                    {
                        self.session
                            .host_mut()
                            .set_online_tape_titles(self.prefs.online_tape_titles);
                        self.mark_prefs_dirty();
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
                let (tape_title, has_tape, tape_progress) = {
                    let mut host = self.session.host_mut();
                    (
                        host.media_title().map(str::to_owned),
                        host.machine().is_some_and(Machine::has_tape),
                        host.machine().and_then(Machine::tape_progress),
                    )
                };
                if let Some(p) = tape_progress {
                    if let Some(ref title) = tape_title {
                        ui.label(title);
                    }
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
                    // The rate actually being run, not the EAR speed setting:
                    // turbo stops when the deck finishes (#390).
                    let effective = self
                        .session
                        .host_mut()
                        .machine()
                        .map_or(1, Machine::effective_speed_multiplier);
                    if effective > 1 {
                        ui.strong(format!("{effective}×"))
                            .on_hover_text("Spectrum frames per host tick while the tape plays");
                    }
                } else if has_tape {
                    if let Some(ref title) = tape_title {
                        ui.label(title);
                    }
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
    }
}
