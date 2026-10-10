//! Debugger workspace and controls.

use super::SpecChumApp;
use eframe::egui;

impl SpecChumApp {
    pub(super) fn render_debugger(&mut self, ctx: &egui::Context) {
        let mut open = self.session.debug_open;
        egui::Window::new("Debugger")
            .open(&mut open)
            .default_size([760.0, 620.0])
            .show(ctx, |ui| self.render_debugger_contents(ui));
        self.session.debug_open = open;
    }

    pub(super) fn render_debugger_contents(&mut self, ui: &mut egui::Ui) {
        if !self.session.host_mut().has_machine() {
            ui.label("No machine loaded");
            return;
        }

        let paused = self.session.host_mut().paused();
        ui.horizontal(|ui| {
            ui.strong(if paused {
                "Execution: paused"
            } else {
                "Execution: running"
            });
            if paused {
                if ui.button("Continue").clicked() {
                    let mut host = self.session.host_mut();
                    if let Err(error) = host.debug_continue() {
                        host.set_status(error.to_string());
                    }
                }
            } else if ui.button("Pause").clicked() {
                self.session.host_mut().debug_pause();
            }
            if ui.button("Step").clicked() {
                let mut host = self.session.host_mut();
                if let Err(error) = host.debug_step() {
                    host.set_status(error.to_string());
                }
            }
        });

        let inspect = self.session.host_mut().inspect_text().unwrap_or_default();
        let disasm = self.session.host_mut().disasm(None, 12).unwrap_or_default();
        let mem_addr = self.session.debug_mem_addr;
        let hex = self
            .session
            .host_mut()
            .hexdump(mem_addr, 64)
            .unwrap_or_default();
        let breaks = self
            .session
            .host_mut()
            .list_pc_breakpoints()
            .unwrap_or_default();
        let (pc, sp, hl) = self
            .session
            .host_mut()
            .regs()
            .map_or((0, 0, 0), |r| (r.pc, r.sp, r.hl));
        let flags = self
            .session
            .host_mut()
            .debugger_flags_text()
            .unwrap_or_else(|error| format!("Flags: {error}"));

        ui.separator();
        ui.label(flags);
        egui::CollapsingHeader::new("Registers")
            .default_open(true)
            .show(ui, |ui| ui.monospace(inspect));
        egui::CollapsingHeader::new("Disassembly at PC")
            .default_open(true)
            .show(ui, |ui| ui.monospace(disasm));
        egui::CollapsingHeader::new("Memory").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Address");
                let mut addr = self.session.debug_mem_addr;
                if ui
                    .add(egui::DragValue::new(&mut addr).hexadecimal(4, false, false))
                    .changed()
                {
                    self.session.debug_mem_addr = addr;
                }
                if ui.button("PC").clicked() {
                    self.session.debug_mem_addr = pc;
                }
                if ui.button("SP").clicked() {
                    self.session.debug_mem_addr = sp;
                }
                if ui.button("HL").clicked() {
                    self.session.debug_mem_addr = hl;
                }
            });
            ui.monospace(hex);
        });
        egui::CollapsingHeader::new("PC breakpoints")
            .default_open(true)
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label("Address");
                    let mut bp = self.session.debug_break_pc;
                    if ui
                        .add(egui::DragValue::new(&mut bp).hexadecimal(4, false, false))
                        .changed()
                    {
                        self.session.debug_break_pc = bp;
                    }
                    if ui.button("PC").clicked() {
                        self.session.debug_break_pc = pc;
                    }
                    if ui.button("Add breakpoint").clicked() {
                        let bp = self.session.debug_break_pc;
                        let mut host = self.session.host_mut();
                        if let Err(error) = host.add_breakpoint(bp) {
                            host.set_status(error.to_string());
                        }
                    }
                    if ui.button("Clear breakpoints").clicked() {
                        let mut host = self.session.host_mut();
                        if let Err(error) = host.clear_breakpoints() {
                            host.set_status(error.to_string());
                        }
                    }
                });
                ui.label(format!("Active PC breakpoints: {breaks:?}"));
            });
        egui::CollapsingHeader::new(format!("Trace ({} events)", trace::len())).show(ui, |ui| {
            if trace::len() == 0 {
                ui.label("No trace events. Enable tracing in the Debug menu.");
            }
            for event in trace::snapshot().iter().rev().take(16) {
                ui.monospace(event.to_string());
            }
        });
    }
}
