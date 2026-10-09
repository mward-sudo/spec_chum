//! Simple, readable egui chrome (no fake glass under the titlebar).

use eframe::egui::{self, Color32, CornerRadius, Shadow, Stroke, Vec2};
use spec_chum_host::AppearancePreference;

/// Apply both palette variants and select the persisted preference.
pub fn apply(ctx: &egui::Context, appearance: AppearancePreference) {
    ctx.style_mut_of(egui::Theme::Light, |style| apply_palette(style, false));
    ctx.style_mut_of(egui::Theme::Dark, |style| apply_palette(style, true));
    set_appearance(ctx, appearance);
}

pub fn set_appearance(ctx: &egui::Context, appearance: AppearancePreference) {
    ctx.set_theme(match appearance {
        AppearancePreference::System => egui::ThemePreference::System,
        AppearancePreference::Light => egui::ThemePreference::Light,
        AppearancePreference::Dark => egui::ThemePreference::Dark,
    });
}

#[derive(Clone, Copy)]
pub struct StatusColors {
    pub success: Color32,
    pub warning: Color32,
    pub error: Color32,
}

pub fn status_colors(visuals: &egui::Visuals) -> StatusColors {
    if visuals.dark_mode {
        StatusColors {
            success: Color32::from_rgb(128, 216, 165),
            warning: Color32::from_rgb(255, 204, 102),
            error: Color32::from_rgb(255, 138, 128),
        }
    } else {
        StatusColors {
            success: Color32::from_rgb(23, 107, 58),
            warning: Color32::from_rgb(129, 82, 0),
            error: Color32::from_rgb(180, 35, 24),
        }
    }
}

fn apply_palette(style: &mut egui::Style, dark: bool) {
    let mut visuals = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    if dark {
        visuals.panel_fill = Color32::from_rgb(31, 35, 40);
        visuals.window_fill = Color32::from_rgb(38, 43, 49);
        visuals.faint_bg_color = Color32::from_rgb(45, 50, 57);
        visuals.extreme_bg_color = Color32::from_rgb(20, 23, 27);
        visuals.window_stroke = Stroke::new(1.0_f32, Color32::from_rgb(70, 77, 86));
        visuals.widgets.noninteractive.bg_stroke =
            Stroke::new(1.0_f32, Color32::from_rgb(65, 71, 79));
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(43, 48, 55);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(57, 65, 75);
        visuals.widgets.active.bg_fill = Color32::from_rgb(68, 80, 97);
        visuals.selection.bg_fill = Color32::from_rgb(65, 112, 184);
        visuals.override_text_color = Some(Color32::from_rgb(230, 233, 238));
    } else {
        // Opaque panel fills — translucent “glass” under a native titlebar broke hit zones.
        visuals.panel_fill = Color32::from_rgb(246, 248, 250);
        visuals.window_fill = Color32::from_rgb(255, 255, 255);
        visuals.faint_bg_color = Color32::from_rgb(236, 239, 243);
        visuals.extreme_bg_color = Color32::from_rgb(232, 235, 240);
        visuals.window_stroke = Stroke::new(1.0_f32, Color32::from_rgb(200, 205, 212));
        visuals.widgets.noninteractive.bg_stroke =
            Stroke::new(1.0_f32, Color32::from_rgb(210, 214, 220));
        visuals.widgets.inactive.bg_fill = Color32::from_rgb(255, 255, 255);
        visuals.widgets.hovered.bg_fill = Color32::from_rgb(235, 240, 248);
        visuals.widgets.active.bg_fill = Color32::from_rgb(220, 228, 240);
        visuals.selection.bg_fill = Color32::from_rgb(90, 140, 220);
        visuals.override_text_color = Some(Color32::from_rgb(28, 32, 38));
    }
    visuals.window_corner_radius = CornerRadius::same(8);
    visuals.menu_corner_radius = CornerRadius::same(6);
    visuals.window_shadow = Shadow {
        offset: [0, 2],
        blur: 8,
        spread: 0,
        color: Color32::from_black_alpha(20),
    };
    style.spacing.item_spacing = Vec2::new(10.0, 8.0);
    style.spacing.button_padding = Vec2::new(12.0, 6.0);
    style.spacing.menu_margin = egui::Margin::same(8);
    style.spacing.window_margin = egui::Margin::same(10);
    style.interaction.tooltip_delay = 0.35;
    style.visuals = visuals;
}

/// Soft clear color behind panels (eframe `App::clear_color`).
#[must_use]
pub fn clear_color(visuals: &egui::Visuals) -> [f32; 4] {
    if visuals.dark_mode {
        [0.08, 0.09, 0.11, 1.0]
    } else {
        [0.94, 0.95, 0.97, 1.0]
    }
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
        apply(&ctx, AppearancePreference::Light);
        assert!(!ctx.style().visuals.dark_mode);
        assert!(menu_bar_min_height() >= 28.0);
        let c = clear_color(&ctx.style().visuals);
        assert!((0.0..=1.0).contains(&c[0]));
    }

    #[test]
    fn panel_fill_is_opaque() {
        let ctx = egui::Context::default();
        apply(&ctx, AppearancePreference::Light);
        let fill = ctx.style().visuals.panel_fill;
        assert_eq!(fill.a(), 255, "opaque panels keep hit-testing predictable");
    }

    #[test]
    fn applies_dark_and_light_preferences() {
        let ctx = egui::Context::default();
        apply(&ctx, AppearancePreference::Dark);
        assert!(ctx.style().visuals.dark_mode);
        assert_eq!(clear_color(&ctx.style().visuals), [0.08, 0.09, 0.11, 1.0]);

        set_appearance(&ctx, AppearancePreference::Light);
        assert!(!ctx.style().visuals.dark_mode);
    }

    #[test]
    fn status_colors_meet_wcag_aa_contrast_on_panels() {
        for dark in [false, true] {
            let mut visuals = if dark {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            };
            visuals.panel_fill = if dark {
                Color32::from_rgb(31, 35, 40)
            } else {
                Color32::from_rgb(246, 248, 250)
            };
            let colors = status_colors(&visuals);
            for color in [colors.success, colors.warning, colors.error] {
                assert!(contrast_ratio(color, visuals.panel_fill) >= 4.5);
            }
        }
    }

    fn contrast_ratio(foreground: Color32, background: Color32) -> f32 {
        let luminance = |color: Color32| {
            let channel = |value: u8| {
                let value = f32::from(value) / 255.0;
                if value <= 0.04045 {
                    value / 12.92
                } else {
                    ((value + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * channel(color.r()) + 0.7152 * channel(color.g()) + 0.0722 * channel(color.b())
        };
        let (lighter, darker) = {
            let foreground = luminance(foreground);
            let background = luminance(background);
            (foreground.max(background), foreground.min(background))
        };
        (lighter + 0.05) / (darker + 0.05)
    }
}
