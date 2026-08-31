use serde::Serialize;

use spec_chum_host::{HostSession, ModelId};

use crate::error::{ApiError, ApiResult};

#[derive(Clone, Debug, Serialize)]
pub struct FramebufferMeta {
    pub width: usize,
    pub height: usize,
    pub border: bool,
    pub hires: bool,
    pub scld_mode: Option<u8>,
    pub model: String,
}

impl FramebufferMeta {
    #[must_use]
    pub fn from_session(session: &HostSession) -> Self {
        Self {
            width: session.width(),
            height: session.height(),
            border: session.with_border(),
            hires: session.framebuffer_hires(),
            scld_mode: session.timex_scld_mode(),
            model: model_slug(session.model()),
        }
    }
}

#[must_use]
pub fn model_slug(model: ModelId) -> String {
    match model {
        ModelId::Spectrum16K => "16k".into(),
        ModelId::Spectrum48 => "48k".into(),
        ModelId::Spectrum128 => "128k".into(),
        ModelId::SpectrumPlus2 => "plus2".into(),
        ModelId::SpectrumPlus2A => "plus2a".into(),
        ModelId::SpectrumPlus3 => "plus3".into(),
        ModelId::Pentagon128 => "pentagon128".into(),
        ModelId::TimexTC2048 => "timex_tc2048".into(),
        ModelId::TimexTS2068 => "timex_ts2068".into(),
    }
}

pub fn parse_model_slug(s: &str) -> ApiResult<ModelId> {
    Ok(match s.trim().to_ascii_lowercase().as_str() {
        "16" | "16k" => ModelId::Spectrum16K,
        "48" | "48k" => ModelId::Spectrum48,
        "128" | "128k" => ModelId::Spectrum128,
        "plus2" | "+2" => ModelId::SpectrumPlus2,
        "plus2a" | "+2a" => ModelId::SpectrumPlus2A,
        "plus3" | "+3" => ModelId::SpectrumPlus3,
        "pentagon" | "pentagon128" | "128p" => ModelId::Pentagon128,
        "timex" | "tc2048" | "timex2048" => ModelId::TimexTC2048,
        "ts2068" | "tc2068" | "timex2068" => ModelId::TimexTS2068,
        other => {
            return Err(ApiError::BadRequest(format!("unknown model {other}")));
        }
    })
}

pub fn encode_framebuffer_png(session: &HostSession) -> ApiResult<Vec<u8>> {
    let w = session.width();
    let h = session.height();
    let rgba = session.framebuffer();
    if rgba.len() != w * h * 4 {
        return Err(ApiError::Png(format!(
            "framebuffer size mismatch: got {} bytes, want {}",
            rgba.len(),
            w * h * 4
        )));
    }
    let mut rgba8 = Vec::with_capacity(w * h * 4);
    rgba8.extend_from_slice(rgba);
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
            .write_image_data(&rgba8)
            .map_err(|e| ApiError::Png(e.to_string()))?;
    }
    Ok(out)
}
