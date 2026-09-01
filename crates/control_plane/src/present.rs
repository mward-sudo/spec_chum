//! Software presentation of the guest framebuffer (egui-faithful NEAREST + letterbox).

use serde::Serialize;

use crate::error::{ApiError, ApiResult};

/// Upper bound for in-process host-display RGBA buffers (agent `/v1/host/display`).
const MAX_HOST_DISPLAY_RGBA_BYTES: usize = 64 * 1024 * 1024;

fn rgba_bytes_checked(w: usize, h: usize) -> ApiResult<usize> {
    w.checked_mul(h)
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| ApiError::BadRequest("rgba dimensions overflow".into()))
}

fn host_display_rgba_len_checked(w: usize, h: usize) -> ApiResult<usize> {
    let bytes = rgba_bytes_checked(w, h)?;
    if bytes > MAX_HOST_DISPLAY_RGBA_BYTES {
        return Err(ApiError::BadRequest(format!(
            "display canvas too large (max {} MiB rgba)",
            MAX_HOST_DISPLAY_RGBA_BYTES / (1024 * 1024)
        )));
    }
    Ok(bytes)
}

/// Largest size that fits `avail` while preserving `src` aspect ratio.
///
/// Matches [`app::display::fit_size`] (egui central panel).
#[must_use]
pub fn fit_size(src_w: f32, src_h: f32, avail_w: f32, avail_h: f32) -> (f32, f32) {
    if src_w <= 0.0 || src_h <= 0.0 || avail_w <= 0.0 || avail_h <= 0.0 {
        return (0.0, 0.0);
    }
    let scale = (avail_w / src_w).min(avail_h / src_h);
    (src_w * scale, src_h * scale)
}

/// How the output canvas size was chosen for `/v1/host/display`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PresentPanelSource {
    Explicit,
    Live,
    Scale,
    Default,
}

#[derive(Clone, Debug, Serialize)]
pub struct PresentMeta {
    pub width: usize,
    pub height: usize,
    pub source_width: usize,
    pub source_height: usize,
    pub filter: &'static str,
    pub panel: PresentPanelSource,
}

/// NEAREST-neighbour blit of `src` (RGBA8, `sw`×`sh`) into a black `dw`×`dh` canvas,
/// letterboxed to preserve aspect (egui display path).
pub fn compose_nearest_letterbox(
    src: &[u8],
    sw: usize,
    sh: usize,
    dw: usize,
    dh: usize,
) -> ApiResult<Vec<u8>> {
    if sw == 0 || sh == 0 || dw == 0 || dh == 0 {
        return Err(ApiError::BadRequest(
            "display present size must be non-zero".into(),
        ));
    }
    let src_bytes = host_display_rgba_len_checked(sw, sh)?;
    if src.len() != src_bytes {
        return Err(ApiError::Png(format!(
            "source rgba size mismatch: got {} want {}",
            src.len(),
            src_bytes
        )));
    }
    let (fw, fh) = fit_size(sw as f32, sh as f32, dw as f32, dh as f32);
    let fitted_w = fw.round().max(1.0) as usize;
    let fitted_h = fh.round().max(1.0) as usize;
    let fitted_w = fitted_w.min(dw);
    let fitted_h = fitted_h.min(dh);
    let ox = (dw - fitted_w) / 2;
    let oy = (dh - fitted_h) / 2;

    let out_bytes = host_display_rgba_len_checked(dw, dh)?;
    let mut out = vec![0u8; out_bytes];
    for dy in 0..fitted_h {
        let sy = (dy as u64 * sh as u64) / fitted_h as u64;
        for dx in 0..fitted_w {
            let sx = (dx as u64 * sw as u64) / fitted_w as u64;
            let si = (sy as usize * sw + sx as usize) * 4;
            let di = ((oy + dy) * dw + (ox + dx)) * 4;
            out[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    Ok(out)
}

pub fn encode_rgba_png(rgba: &[u8], w: usize, h: usize) -> ApiResult<Vec<u8>> {
    let expected = rgba_bytes_checked(w, h)?;
    if rgba.len() != expected {
        return Err(ApiError::Png(format!(
            "rgba size mismatch: got {} want {}",
            rgba.len(),
            expected
        )));
    }
    let mut out = Vec::new();
    {
        let mut encoder = png::Encoder::new(
            &mut out,
            u32::try_from(w).map_err(|_| ApiError::Png("width out of range".into()))?,
            u32::try_from(h).map_err(|_| ApiError::Png("height out of range".into()))?,
        );
        encoder.set_color(png::ColorType::Rgba);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| ApiError::Png(e.to_string()))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| ApiError::Png(e.to_string()))?;
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_letterboxes_wide() {
        let (w, h) = fit_size(352.0, 296.0, 800.0, 400.0);
        assert!((h - 400.0).abs() < 0.01);
        assert!(w < 800.0);
    }

    #[test]
    fn nearest_scale2_doubles() {
        // 2×2 red → 4×4
        let src = [
            255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255, 255, 0, 0, 255,
        ];
        let out = compose_nearest_letterbox(&src, 2, 2, 4, 4).unwrap();
        assert_eq!(out.len(), 4 * 4 * 4);
        assert_eq!(&out[0..4], &[255, 0, 0, 255]);
    }

    #[test]
    fn rejects_oversized_canvas() {
        let src = vec![0u8; 4];
        let err =
            compose_nearest_letterbox(&src, 1, 1, usize::MAX, 4).expect_err("overflow canvas");
        assert!(matches!(err, ApiError::BadRequest(_)));
    }
}
