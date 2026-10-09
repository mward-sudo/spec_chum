//! Application sections and their host-local navigation behavior.

use super::{display, SpecChumApp};
use eframe::egui;

const FRONTEND_VIEW_ID: &str = "frontend.view";
pub(super) const GUEST_KEYBOARD_SUPPRESSED_ID: &str = "frontend.guest_keyboard_suppressed";
const PLAY_FOCUS_REQUEST_ID: &str = "frontend.play_focus_request";

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
                                &["sna", "z80", "tap", "tzx", "rzx", "dsk"],
                            )
                            .pick_file()
                        {
                            self.open_recent_path(&path);
                            current = FrontendView::Play;
                            ctx.data_mut(|data| {
                                data.insert_temp(egui::Id::new(GUEST_KEYBOARD_SUPPRESSED_ID), true);
                                data.insert_temp(egui::Id::new(PLAY_FOCUS_REQUEST_ID), true);
                            });
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
                ui.label("Select a built-in model or configure your machine.");
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
        ui.label("Recently opened media");
        ui.separator();
        if self.prefs.recent_files.is_empty() {
            ui.label("No recent media yet. Use Open media… or File → Open.");
            return;
        }
        let recents = self.prefs.recent_files.clone();
        egui::ScrollArea::vertical().show(ui, |ui| {
            for path in recents {
                let label = std::path::Path::new(&path)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(&path);
                if ui.button(label).on_hover_text(&path).clicked() {
                    self.open_recent_path(std::path::Path::new(&path));
                    FrontendView::Play.set(ui.ctx());
                    ui.ctx().data_mut(|data| {
                        data.insert_temp(egui::Id::new(PLAY_FOCUS_REQUEST_ID), true);
                    });
                }
            }
        });
    }
}
