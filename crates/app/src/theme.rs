//! Tasteful macOS-oriented egui visuals (frosted panels, SF-like spacing).

use eframe::egui::{self, Color32, CornerRadius, Shadow, Stroke, Vec2};

/// Apply once at startup. Safe for headless `Context::run` tests.
pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::light();
    visuals.dark_mode = false;
    // Frosted glass–ish fills (alpha where egui composites over the clear color).
    visuals.panel_fill = Color32::from_rgba_unmultiplied(246, 248, 250, 235);
    visuals.window_fill = Color32::from_rgba_unmultiplied(255, 255, 255, 230);
    visuals.faint_bg_color = Color32::from_rgba_unmultiplied(236, 239, 243, 200);
    visuals.extreme_bg_color = Color32::from_rgb(232, 235, 240);
    visuals.window_corner_radius = CornerRadius::same(12);
    visuals.menu_corner_radius = CornerRadius::same(10);
    visuals.window_shadow = Shadow {
        offset: [0, 4],
        blur: 16,
        spread: 0,
        color: Color32::from_black_alpha(28),
    };
    visuals.window_stroke =
        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(180, 186, 196, 90));
    visuals.widgets.noninteractive.bg_stroke =
        Stroke::new(1.0_f32, Color32::from_rgba_unmultiplied(180, 186, 196, 60));
    visuals.widgets.inactive.bg_fill = Color32::from_rgba_unmultiplied(255, 255, 255, 180);
    visuals.widgets.hovered.bg_fill = Color32::from_rgba_unmultiplied(235, 240, 248, 220);
    visuals.widgets.active.bg_fill = Color32::from_rgba_unmultiplied(220, 228, 240, 230);
    visuals.selection.bg_fill = Color32::from_rgba_unmultiplied(90, 140, 220, 120);
    visuals.override_text_color = Some(Color32::from_rgb(28, 32, 38));
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    style.spacing.button_padding = Vec2::new(11.0, 5.0);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.window_margin = egui::Margin::same(10);
    style.interaction.tooltip_delay = 0.35;
    ctx.set_style(style);
}

/// Leading inset so the menu clears macOS traffic lights under a fullsize content view.
#[must_use]
pub fn macos_traffic_light_inset() -> f32 {
    if cfg!(target_os = "macos") {
        72.0
    } else {
        0.0
    }
}

/// Soft clear color behind translucent panels (eframe `App::clear_color`).
#[must_use]
pub fn clear_color() -> [f32; 4] {
    [0.94, 0.95, 0.97, 1.0]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_does_not_panic_on_default_context() {
        let ctx = egui::Context::default();
        apply(&ctx);
        assert!(!ctx.style().visuals.dark_mode);
        assert!(macos_traffic_light_inset() >= 0.0);
        let c = clear_color();
        assert!((0.0..=1.0).contains(&c[0]));
    }
}
