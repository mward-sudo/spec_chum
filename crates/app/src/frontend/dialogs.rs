//! ROM setup and saved machine configuration dialogs.

use super::SpecChumApp;
use eframe::egui;
use spec_chum_host::{
    acquire_next_assets, hardware_compat, install_model_rom, model_requires_user_rom,
    model_rom_available, model_rom_path_key, rom_setup_json, sync_model_rom_paths, PrefAyStereo,
    PrefJoystick, PrefModel,
};
use std::sync::mpsc::{self, TryRecvError};

impl SpecChumApp {
    pub(super) fn poll_next_assets_download(&mut self) {
        let Some(receiver) = self.next_assets_download.as_ref() else {
            return;
        };
        let result = match receiver.try_recv() {
            Ok(result) => result,
            Err(TryRecvError::Empty) => return,
            Err(TryRecvError::Disconnected) => {
                Err("Official asset download stopped unexpectedly".into())
            }
        };
        self.next_assets_download = None;
        match result {
            Ok(archive) => {
                self.prefs.model_rom_paths.insert(
                    model_rom_path_key(PrefModel::SpectrumNext, "next_assets"),
                    archive.display().to_string(),
                );
                sync_model_rom_paths(self.prefs.model_rom_paths.clone());
                self.mark_prefs_dirty();
                self.session
                    .host_mut()
                    .set_status(format!("Verified {}", archive.display()));
                self.refresh_rom_setup();
            }
            Err(error) => self.rom_setup_error = Some(error),
        }
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
    pub(super) fn show_roms_toolbar_button(&self) -> bool {
        if self.prefs.active_config_id.is_some() {
            return false;
        }
        self.needs_rom_setup()
            || model_requires_user_rom(PrefModel::from_model(self.session.model()).to_model_id())
    }

    pub(super) fn refresh_rom_setup(&mut self) {
        self.rom_setup_error = None;
        self.rom_setup = Some(rom_setup_json(
            PrefModel::from_model(self.session.model()).to_model_id(),
            &self.prefs.model_rom_paths,
        ));
    }

    pub(super) fn maybe_auto_present_rom_setup(&mut self) {
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

    fn path_field(ui: &mut egui::Ui, label: &str, path: &mut Option<String>, filter: &str) {
        ui.horizontal(|ui| {
            ui.label(label);
            let display = path.as_deref().unwrap_or("(default / none)");
            ui.label(display);
            if ui.button("Browse…").clicked() {
                if let Some(picked) = rfd::FileDialog::new()
                    .add_filter(filter, &["rom", "bin", "img", "eeprom"])
                    // DiagROM and similar dumps often ship without an extension.
                    .add_filter("All files", &["*"])
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

    pub(super) fn rom_setup_window(&mut self, ctx: &egui::Context) {
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
                if self.session.model() == machine::Model::SpectrumNext {
                    ui.weak("Get the pinned official System/Next 24.11 distribution and separate GPL boot code, or select the verified archive from its companion asset folder below. Source and license details: https://github.com/mward-sudo/spec_chum/blob/main/docs/ROMS.md. Spec Chum is unaffiliated with SpecNext Ltd.");
                    if ui.add_enabled(self.next_assets_download.is_none(), egui::Button::new("Get official System/Next assets…")).clicked() {
                        self.rom_setup_error = None;
                        self.session.host_mut().set_status("Acquiring official System/Next assets…".to_owned());
                        let (sender, receiver) = mpsc::channel();
                        std::thread::spawn(move || {
                            let result = acquire_next_assets().map_err(|error| error.to_string());
                            let _ = sender.send(result);
                        });
                        self.next_assets_download = Some(receiver);
                    }
                    if self.next_assets_download.is_some() {
                        ui.spinner();
                        ui.label("Downloading and verifying official assets…");
                    }
                } else if doc.fetchable {
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
                            let filter = if self.session.model() == machine::Model::SpectrumNext {
                                ("Official System/Next archive", &["zip"][..])
                            } else {
                                ("ROM", &["rom", "bin"][..])
                            };
                            if let Some(picked) = rfd::FileDialog::new()
                                .add_filter(filter.0, filter.1)
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
                                    Err(e) => self.rom_setup_error = Some(e.to_string()),
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

    pub(super) fn config_editor_window(&mut self, ctx: &egui::Context) {
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
                        if pick == machine::Model::SpectrumNext {
                            continue;
                        }
                        let pref = PrefModel::from_model(pick);
                        let title = machine::model_title(pick);
                        let mut selected = draft.base == pref;
                        if ui.radio_value(&mut selected, true, title).clicked() {
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
                                .checkbox(&mut draft.attach_multiface, "Multiface (1 / 128)")
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
                                self.prefs.sync_machine_fields_from_config(&to_save);
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
}
