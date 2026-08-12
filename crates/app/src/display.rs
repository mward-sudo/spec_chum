//! Framebuffer fit / letterbox helpers for the egui central panel.

/// Compute the largest size that fits `avail` while preserving `src` aspect ratio.
#[must_use]
pub fn fit_size(src: egui::Vec2, avail: egui::Vec2) -> egui::Vec2 {
    if src.x <= 0.0 || src.y <= 0.0 || avail.x <= 0.0 || avail.y <= 0.0 {
        return egui::Vec2::ZERO;
    }
    let scale = (avail.x / src.x).min(avail.y / src.y);
    egui::vec2(src.x * scale, src.y * scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn letterboxes_wide_window() {
        let src = egui::vec2(352.0, 296.0);
        let avail = egui::vec2(800.0, 400.0);
        let fitted = fit_size(src, avail);
        let eps = 0.01;
        assert!(
            (fitted.y - 400.0).abs() < eps,
            "height-limited {:?}",
            fitted
        );
        assert!(fitted.x < avail.x);
        assert!((fitted.x / fitted.y - src.x / src.y).abs() < 0.001);
    }

    #[test]
    fn pillarboxes_tall_window() {
        let src = egui::vec2(352.0, 296.0);
        let avail = egui::vec2(400.0, 800.0);
        let fitted = fit_size(src, avail);
        assert!((fitted.x - 400.0).abs() < 0.01);
        assert!(fitted.y < avail.y);
    }

    #[test]
    fn zero_avail_is_zero() {
        assert_eq!(
            fit_size(egui::vec2(10.0, 10.0), egui::Vec2::ZERO),
            egui::Vec2::ZERO
        );
    }
}
