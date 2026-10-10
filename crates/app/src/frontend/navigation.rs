//! Application sections and their host-local navigation behavior.

use super::{display, SpecChumApp};
use eframe::egui;
use spec_chum_host::{query_recent_media, MediaCategory, MediaCompatibility, MediaEntry};

const FRONTEND_VIEW_ID: &str = "frontend.view";
pub(super) const GUEST_KEYBOARD_SUPPRESSED_ID: &str = "frontend.guest_keyboard_suppressed";
const PLAY_FOCUS_REQUEST_ID: &str = "frontend.play_focus_request";
/// Longest side uploaded for a machine photo.
///
/// `egui_glow` aborts when a texture exceeds `GL_MAX_TEXTURE_SIZE`. Several
/// bundled photos are wider than 4096, the limit on common GLES devices.
/// The picker draws them at about 220×112, so this cap stays inside that
/// limit and inside egui's reported maximum when the GPU is smaller.
const MAX_MACHINE_PHOTO_SIDE: usize = 1024;

fn machine_photo_bytes(key: &str) -> Option<&'static [u8]> {
    match key {
        "spectrum48" => Some(include_bytes!("../../../../assets/machines/spectrum48.jpg")),
        "spectrum128" => Some(include_bytes!(
            "../../../../assets/machines/spectrum128.jpg"
        )),
        "plus2" => Some(include_bytes!("../../../../assets/machines/plus2.jpg")),
        "plus2a_black" => Some(include_bytes!(
            "../../../../assets/machines/plus2a_black.jpg"
        )),
        "plus3" => Some(include_bytes!("../../../../assets/machines/plus3.jpg")),
        "tc2048" => Some(include_bytes!("../../../../assets/machines/tc2048.jpg")),
        "ts2068" => Some(include_bytes!("../../../../assets/machines/ts2068.jpg")),
        "pentagon128_candidate" => Some(include_bytes!(
            "../../../../assets/machines/pentagon128_candidate.png"
        )),
        "scorpion_zs256_candidate" => Some(include_bytes!(
            "../../../../assets/machines/scorpion_zs256_candidate.png"
        )),
        "spectrum_next_candidate" => Some(include_bytes!(
            "../../../../assets/machines/spectrum_next_candidate.png"
        )),
        _ => None,
    }
}

fn fitted_machine_photo(bytes: &[u8], max_side: u32) -> Option<image::RgbaImage> {
    use image::GenericImageView;

    let max_side = max_side.max(1);
    let decoded = image::load_from_memory(bytes).ok()?;
    let (width, height) = decoded.dimensions();
    let fitted = if width > max_side || height > max_side {
        decoded.resize(max_side, max_side, image::imageops::FilterType::Triangle)
    } else {
        decoded
    };
    Some(fitted.to_rgba8())
}

fn load_machine_photo(ctx: &egui::Context, key: &str) -> Option<egui::TextureHandle> {
    let bytes = machine_photo_bytes(key)?;
    let max_side = ctx
        .input(|input| input.max_texture_side)
        .clamp(1, MAX_MACHINE_PHOTO_SIDE);
    let rgba = fitted_machine_photo(bytes, max_side as u32)?;
    let size = [rgba.width() as usize, rgba.height() as usize];
    let pixels = egui::ColorImage::from_rgba_unmultiplied(size, rgba.as_raw());
    Some(ctx.load_texture(
        format!("machine-{key}-photo"),
        pixels,
        egui::TextureOptions::LINEAR,
    ))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum FrontendView {
    Library,
    #[default]
    Play,
    Machines,
    Debugger,
    Settings,
}

impl FrontendView {
    const ALL: [(Self, &'static str); 5] = [
        (Self::Library, "Library"),
        (Self::Play, "Play"),
        (Self::Machines, "Machines"),
        (Self::Debugger, "Debugger"),
        (Self::Settings, "Settings"),
    ];

    pub(super) fn get(ctx: &egui::Context) -> Self {
        ctx.data(|data| data.get_temp(egui::Id::new(FRONTEND_VIEW_ID)))
            .unwrap_or_default()
    }

    fn set(self, ctx: &egui::Context) {
        ctx.data_mut(|data| data.insert_temp(egui::Id::new(FRONTEND_VIEW_ID), self));
    }
}

pub(super) fn should_suppress_guest_keyboard(
    view: FrontendView,
    route_changed: bool,
    was_suppressed: bool,
    any_keys_down: bool,
) -> bool {
    view != FrontendView::Play || ((route_changed || was_suppressed) && any_keys_down)
}

impl SpecChumApp {
    pub(super) fn navigation_bar(&mut self, ctx: &egui::Context) -> bool {
        let mut current = FrontendView::get(ctx);
        let previous = current;
        egui::TopBottomPanel::top("navigation")
            .exact_height(42.0)
            .frame(
                egui::Frame::new()
                    .fill(ctx.style().visuals.panel_fill)
                    .inner_margin(egui::Margin::symmetric(12, 4))
                    .stroke(egui::Stroke::NONE),
            )
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    for (view, label) in FrontendView::ALL {
                        if ui.selectable_label(current == view, label).clicked() {
                            current = view;
                        }
                    }
                    if ui.button("Open media…").clicked() {
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter(
                                "Spectrum media",
                                &["sna", "z80", "tap", "tzx", "rzx", "dsk", "trd"],
                            )
                            .pick_file()
                        {
                            if self.open_recent_path(&path) {
                                current = FrontendView::Play;
                                ctx.data_mut(|data| {
                                    data.insert_temp(
                                        egui::Id::new(GUEST_KEYBOARD_SUPPRESSED_ID),
                                        true,
                                    );
                                    data.insert_temp(egui::Id::new(PLAY_FOCUS_REQUEST_ID), true);
                                });
                            }
                        }
                    }
                });
            });
        if current != previous {
            current.set(ctx);
            if current == FrontendView::Debugger {
                self.session.debug_open = true;
            }
            if current == FrontendView::Play {
                ctx.data_mut(|data| {
                    data.insert_temp(egui::Id::new(GUEST_KEYBOARD_SUPPRESSED_ID), true);
                    data.insert_temp(egui::Id::new(PLAY_FOCUS_REQUEST_ID), true);
                });
            }
        }
        current != previous
    }

    pub(super) fn render_view(
        &mut self,
        ui: &mut egui::Ui,
        ctx: &egui::Context,
        view: FrontendView,
        route_changed: bool,
    ) {
        match view {
            FrontendView::Play => self.render_play(ui, ctx, route_changed),
            FrontendView::Library => self.render_library(ui),
            FrontendView::Machines => {
                ui.heading("Machines");
                ui.label("Choose a built-in machine or open a saved configuration.");
                ui.separator();
                self.render_machine_choices(ui, ctx);
                ui.separator();
                egui::ScrollArea::vertical().show(ui, |ui| self.machine_menu_contents(ui));
            }
            FrontendView::Debugger => {
                ui.heading("Debugger");
                ui.label("The debugger window is open with the live machine controls.");
                if ui.button("Show debugger window").clicked() {
                    self.session.debug_open = true;
                }
            }
            FrontendView::Settings => {
                ui.heading("Settings");
                ui.separator();
                self.settings_contents(ui, ctx);
            }
        }
    }

    fn render_machine_choices(&mut self, ui: &mut egui::Ui, ctx: &egui::Context) {
        use spec_chum_host::{host_model_catalog, ModelId};

        let catalog = host_model_catalog();
        for descriptor in &catalog {
            if let Some(key) = descriptor.image_key {
                self.machine_images
                    .entry(key)
                    .or_insert_with(|| load_machine_photo(ctx, key));
            }
        }
        egui::ScrollArea::horizontal().show(ui, |ui| {
            ui.horizontal(|ui| {
                for descriptor in catalog {
                    let Some(id) = ModelId::from_u32(descriptor.id) else {
                        continue;
                    };
                    let model = id.to_model();
                    let photo = descriptor
                        .image_key
                        .and_then(|key| self.machine_images.get(key))
                        .and_then(Option::as_ref)
                        .map(egui::TextureHandle::id);
                    ui.group(|ui| {
                        ui.set_min_width(238.0);
                        if let Some(texture_id) = photo {
                            ui.add(
                                egui::Image::new((texture_id, egui::vec2(220.0, 112.0))).alt_text(
                                    descriptor.image_description.unwrap_or(descriptor.title),
                                ),
                            );
                        } else {
                            ui.add_space(8.0);
                            ui.label(format!("Photo unavailable · {}", descriptor.title));
                            ui.add_space(8.0);
                        }
                        ui.strong(descriptor.title);
                        if descriptor.image_description.is_some_and(|description| {
                            description.starts_with("Generated illustrative candidate")
                        }) {
                            ui.weak("Illustration candidate");
                        }
                        if let Some(summary) = descriptor.memory_sound_summary {
                            ui.label(summary);
                        }
                        ui.label(if descriptor.available {
                            "ROMs ready"
                        } else {
                            "ROM setup required"
                        });
                        let compat = descriptor.compatible_peripherals;
                        let mut peripherals = Vec::new();
                        if compat.multiface {
                            peripherals.push("Multiface");
                        }
                        if compat.divmmc {
                            peripherals.push("DivMMC");
                        }
                        if compat.interface1 {
                            peripherals.push("Interface 1");
                        }
                        if compat.beta {
                            peripherals.push("Beta Disk");
                        }
                        if compat.timex_dock {
                            peripherals.push("Timex Dock");
                        }
                        if peripherals.is_empty() {
                            ui.label("Compatible peripherals: none listed");
                        } else {
                            ui.label(format!("Compatible: {}", peripherals.join(" · ")));
                        }
                        let active =
                            self.prefs.active_config_id.is_none() && self.session.model() == model;
                        if ui
                            .button(if active { "Selected" } else { "Select" })
                            .clicked()
                        {
                            self.on_builtin_model_selected(model);
                        }
                    });
                }
            });
        });
    }

    fn render_play(&mut self, ui: &mut egui::Ui, ctx: &egui::Context, route_changed: bool) {
        let (image, src) = {
            let host = &*self.session.host_mut();
            let image = egui::ColorImage::from_rgba_unmultiplied(
                [host.width(), host.height()],
                host.framebuffer(),
            );
            let src = egui::vec2(host.width() as f32, host.height() as f32);
            (image, src)
        };
        let tex = self.texture.get_or_insert_with(|| {
            ctx.load_texture("screen", image.clone(), egui::TextureOptions::NEAREST)
        });
        tex.set(image, egui::TextureOptions::NEAREST);
        let avail = ui.available_size();
        let fitted = display::fit_size(src, avail);
        if let Some(plane) = self.plane.as_ref() {
            plane.set_display_panel_size(
                avail.x.round().max(1.0) as u32,
                avail.y.round().max(1.0) as u32,
            );
        }
        let focus_requested = ctx.data_mut(|data| {
            let id = egui::Id::new(PLAY_FOCUS_REQUEST_ID);
            let requested = data.get_temp::<bool>(id).unwrap_or(false);
            data.insert_temp(id, false);
            requested
        });
        let should_focus = route_changed || focus_requested;
        ui.centered_and_justified(|ui| {
            let response = ui.add(egui::Image::new((tex.id(), fitted)).sense(egui::Sense::click()));
            if should_focus {
                response.request_focus();
            }
        });
    }

    fn render_library(&mut self, ui: &mut egui::Ui) {
        ui.heading("Library");
        ui.label("Recent media on this device");
        ui.separator();
        ui.horizontal(|ui| {
            ui.label("Search");
            ui.add(
                egui::TextEdit::singleline(&mut self.library_search)
                    .hint_text("Name or path")
                    .desired_width(220.0),
            );
            egui::ComboBox::from_id_salt("library-category")
                .selected_text(category_label(self.library_category))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut self.library_category, None, "All types");
                    for category in [
                        MediaCategory::Tape,
                        MediaCategory::Snapshot,
                        MediaCategory::Recording,
                        MediaCategory::Disk,
                    ] {
                        ui.selectable_value(
                            &mut self.library_category,
                            Some(category),
                            category_label(Some(category)),
                        );
                    }
                });
        });

        let entries = query_recent_media(
            &self.prefs.recent_files,
            &self.library_search,
            self.library_category,
            &mut self.session.host_mut(),
        );
        if self
            .selected_media_path
            .as_ref()
            .is_none_or(|selected| !entries.iter().any(|entry| &entry.path == selected))
        {
            self.selected_media_path = entries.first().map(|entry| entry.path.clone());
        }
        if entries.is_empty() {
            ui.add_space(12.0);
            ui.label(if self.prefs.recent_files.is_empty() {
                "No recent media yet. Open a supported media file to add it here."
            } else {
                "No supported recent media matches these filters."
            });
            return;
        }

        let mut open_path = None;
        let mut remove_path = None;
        ui.columns(2, |columns| {
            egui::ScrollArea::vertical()
                .id_salt("library-items")
                .show(&mut columns[0], |ui| {
                    for entry in &entries {
                        let selected = self.selected_media_path.as_deref() == Some(&entry.path);
                        let display_title = entry.title.as_deref().unwrap_or(&entry.name);
                        ui.horizontal(|ui| {
                            if ui.selectable_label(selected, display_title).clicked() {
                                self.selected_media_path = Some(entry.path.clone());
                            }
                            ui.small(format_label(entry));
                        });
                        ui.small(format!(
                            "{} · {}",
                            if entry.available {
                                "Available"
                            } else {
                                "Missing"
                            },
                            compatibility_label(entry.compatibility)
                        ));
                        ui.separator();
                    }
                });

            if let Some(entry) = self
                .selected_media_path
                .as_deref()
                .and_then(|selected| entries.iter().find(|entry| entry.path == selected))
            {
                columns[1].heading(&entry.name);
                if let Some(title) = &entry.title {
                    columns[1].label(format!("Known title: {title}"));
                    if let Some(source) = entry.title_source {
                        columns[1].small(format!("Title source: {source}"));
                    }
                }
                columns[1].label(format!(
                    "{} · {}",
                    format_label(entry),
                    category_label(Some(entry.category))
                ));
                columns[1].label(if entry.available {
                    "Available on this device"
                } else {
                    "File is unavailable"
                });
                columns[1].label(compatibility_label(entry.compatibility));
                columns[1].collapsing("Path", |ui| {
                    ui.code(&entry.path);
                });
                columns[1].horizontal(|ui| {
                    if ui
                        .add_enabled(
                            entry.available && entry.compatibility.can_open(),
                            egui::Button::new("Open"),
                        )
                        .clicked()
                    {
                        open_path = Some(entry.path.clone());
                    }
                    if ui.button("Remove from Library").clicked() {
                        remove_path = Some(entry.path.clone());
                    }
                });
            }
        });

        if let Some(path) = remove_path {
            self.prefs.remove_recent(std::path::Path::new(&path));
            self.mark_prefs_dirty();
            if self.selected_media_path.as_deref() == Some(&path) {
                self.selected_media_path = None;
            }
        } else if let Some(path) = open_path {
            if self.open_recent_path(std::path::Path::new(&path)) {
                FrontendView::Play.set(ui.ctx());
                ui.ctx().data_mut(|data| {
                    data.insert_temp(egui::Id::new(PLAY_FOCUS_REQUEST_ID), true);
                });
            }
        }
    }
}

fn category_label(category: Option<MediaCategory>) -> &'static str {
    match category {
        None => "All types",
        Some(MediaCategory::Tape) => "Tape",
        Some(MediaCategory::Snapshot) => "Snapshot",
        Some(MediaCategory::Recording) => "Recording",
        Some(MediaCategory::Disk) => "Disk",
    }
}

fn format_label(entry: &MediaEntry) -> &'static str {
    match entry.format {
        spec_chum_host::MediaFormat::Tap => "TAP",
        spec_chum_host::MediaFormat::Tzx => "TZX",
        spec_chum_host::MediaFormat::Sna => "SNA",
        spec_chum_host::MediaFormat::Z80 => "Z80",
        spec_chum_host::MediaFormat::Rzx => "RZX",
        spec_chum_host::MediaFormat::Dsk => "DSK",
        spec_chum_host::MediaFormat::Trd => "TRD",
    }
}

fn compatibility_label(compatibility: MediaCompatibility) -> &'static str {
    match compatibility {
        MediaCompatibility::Ready => "Compatible with current machine",
        MediaCompatibility::MayRequireMachine => "May need a machine",
        MediaCompatibility::MaySelectMachine => "Snapshot selects model; ROM may be required",
        MediaCompatibility::RequiresMachine => "Select a machine before opening",
        MediaCompatibility::RequiresPlus3 => "Requires a +3 or +3e",
        MediaCompatibility::RequiresBetaModel => "Requires a Beta-compatible model",
        MediaCompatibility::RequiresBeta => "Requires Beta Disk hardware",
        MediaCompatibility::RequiresTrdosRom => "Requires a TR-DOS ROM",
        MediaCompatibility::UnsupportedOnNext => "Not supported on Spectrum Next",
    }
}

#[cfg(test)]
mod machine_photo_tests {
    use super::*;
    use std::collections::BTreeSet;

    #[test]
    fn catalog_photo_keys_have_decodable_standalone_assets() {
        let catalog = spec_chum_host::host_model_catalog();
        let mut keys = BTreeSet::new();
        for descriptor in catalog {
            let Some(key) = descriptor.image_key else {
                assert_ne!(descriptor.title, "");
                continue;
            };
            keys.insert(key);
            let bytes = machine_photo_bytes(key).expect("catalog photo key has an embedded asset");
            assert_ne!(bytes.len(), 0);
            assert!(image::load_from_memory(bytes).is_ok());
            let fitted = fitted_machine_photo(bytes, MAX_MACHINE_PHOTO_SIDE as u32)
                .expect("catalog photo decodes within the texture cap");
            assert!(fitted.width() <= MAX_MACHINE_PHOTO_SIDE as u32);
            assert!(fitted.height() <= MAX_MACHINE_PHOTO_SIDE as u32);
            assert!(descriptor.image_description.is_some());
        }
        assert_eq!(
            keys,
            BTreeSet::from([
                "spectrum48",
                "spectrum128",
                "plus2",
                "plus2a_black",
                "plus3",
                "tc2048",
                "ts2068",
                "pentagon128_candidate",
                "scorpion_zs256_candidate",
                "spectrum_next_candidate"
            ])
        );
    }

    #[test]
    fn physically_shared_variants_use_the_same_catalog_photo() {
        let catalog = spec_chum_host::host_model_catalog();
        let image_key = |model: spec_chum_host::ModelId| {
            catalog
                .iter()
                .find(|descriptor| descriptor.id == model.numeric_id())
                .and_then(|descriptor| descriptor.image_key)
        };
        assert_eq!(
            image_key(spec_chum_host::ModelId::Spectrum16K),
            image_key(spec_chum_host::ModelId::Spectrum48)
        );
        assert_eq!(
            image_key(spec_chum_host::ModelId::SpectrumPlus3),
            image_key(spec_chum_host::ModelId::SpectrumPlus3e)
        );
    }
}
