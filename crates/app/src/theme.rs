//! Simple, readable egui chrome (no fake glass under the titlebar).

use eframe::egui::{self, Color32, CornerRadius, Shadow, Stroke, Vec2};

/// Apply once at startup. Safe for headless `Context::run` tests.
pub fn apply(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::light();
    visuals.dark_mode = false;
    // Opaque panel fills — translucent “glass” under a native titlebar broke hit zones.
    visuals.panel_fill = Color32::from_rgb(246, 248, 250);
    visuals.window_fill = Color32::from_rgb(255, 255, 255);
    visuals.faint_bg_color = Color32::from_rgb(236, 239, 243);
    visuals.extreme_bg_color = Color32::from_rgb(232, 235, 240);
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(6);
    visuals.window_shadow = Shadow {
        offset: [0, 2],
        blur: 8,
        spread: 0,
        color: Color32::from_black_alpha(20),
    };
    visuals.window_stroke = Stroke::new(1.0_f32, Color32::from_rgb(200, 205, 212));
    visuals.widgets.noninteractive.bg_stroke =
        Stroke::new(1.0_f32, Color32::from_rgb(210, 214, 220));
    visuals.widgets.inactive.bg_fill = Color32::from_rgb(255, 255, 255);
    visuals.widgets.hovered.bg_fill = Color32::from_rgb(235, 240, 248);
    visuals.widgets.active.bg_fill = Color32::from_rgb(220, 228, 240);
    visuals.selection.bg_fill = Color32::from_rgb(90, 140, 220);
    visuals.override_text_color = Some(Color32::from_rgb(28, 32, 38));
    ctx.set_visuals(visuals);

    let mut style = (*ctx.style()).clone();
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.window_margin = egui::Margin::same(10);
    style.interaction.tooltip_delay = 0.35;
    ctx.set_style(style);
}

/// Soft clear color behind panels (eframe `App::clear_color`).
#[must_use]
pub fn clear_color() -> [f32; 4] {
    [0.94, 0.95, 0.97, 1.0]
}

/// Minimum height for the top menu strip so buttons remain easy to click.
#[must_use]
pub fn menu_bar_min_height() -> f32 {
    36.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn apply_does_not_panic_on_default_context() {
        let ctx = egui::Context::default();
        apply(&ctx);
        assert!(!ctx.style().visuals.dark_mode);
        assert!(menu_bar_min_height() >= 28.0);
        let c = clear_color();
        assert!((0.0..=1.0).contains(&c[0]));
    }

    #[test]
    fn panel_fill_is_opaque() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let fill = ctx.style().visuals.panel_fill;
        assert_eq!(fill.a(), 255, "opaque panels keep hit-testing predictable");
    }
}
