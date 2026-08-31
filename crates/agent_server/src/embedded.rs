//! Background loopback agent server for hosts (#210 Phase B).

use std::sync::Arc;
use std::thread::{self, JoinHandle};

use anyhow::{Context, Result};
use control_plane::{ControlPlane, ServerConfig};
use spec_chum_host::ModelId;

use crate::routes::serve;

/// Keeps the embedded HTTP server thread alive for the host process lifetime.
#[derive(Debug)]
pub struct EmbeddedServer {
    _thread: JoinHandle<()>,
    pub addr: String,
}

/// Spawn a loopback agent server on a background thread (non-blocking for UI hosts).
pub fn spawn(config: ServerConfig, plane: Arc<ControlPlane>) -> Result<EmbeddedServer> {
    control_plane::ServerConfig::validate_bind_host(&config.host)?;
    let addr = config.socket_addr();
    let thread = thread::Builder::new()
        .name("spec-chum-agent".into())
        .spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    eprintln!("spec-chum-agent embedded: tokio runtime failed: {e}");
                    return;
                }
            };
            if let Err(e) = rt.block_on(serve(config, plane)) {
                eprintln!("spec-chum-agent embedded server error: {e}");
            }
        })
        .context("spawn embedded agent thread")?;
    eprintln!("spec-chum-agent embedded listening on http://{addr}");
    Ok(EmbeddedServer {
        _thread: thread,
        addr,
    })
}

fn parse_model(s: &str) -> Result<ModelId> {
    Ok(match s.to_ascii_lowercase().as_str() {
        "16" | "16k" => ModelId::Spectrum16K,
        "48" | "48k" => ModelId::Spectrum48,
        "128" | "128k" => ModelId::Spectrum128,
        "plus2" | "+2" => ModelId::SpectrumPlus2,
        "plus2a" | "+2a" => ModelId::SpectrumPlus2A,
        "plus3" | "+3" => ModelId::SpectrumPlus3,
        "pentagon" | "pentagon128" | "128p" => ModelId::Pentagon128,
        "timex" | "tc2048" | "timex2048" => ModelId::TimexTC2048,
        "ts2068" | "tc2068" | "timex2068" => ModelId::TimexTS2068,
        other => anyhow::bail!("unknown model {other}"),
    })
}

/// When `SPEC_CHUM_AGENT=1`, start an embedded loopback server with its own session.
///
/// This is a **parallel debug session** (not yet wired to egui/macOS machine state).
/// Agents connect via `http://127.0.0.1:17384` (or `SPEC_CHUM_AGENT_PORT`).
pub fn spawn_from_env() -> Result<Option<EmbeddedServer>> {
    if std::env::var("SPEC_CHUM_AGENT").ok().as_deref() != Some("1") {
        return Ok(None);
    }
    let model_slug = std::env::var("SPEC_CHUM_AGENT_MODEL").unwrap_or_else(|_| "48k".into());
    let with_border = std::env::var("SPEC_CHUM_AGENT_BORDER")
        .ok()
        .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    let model = parse_model(&model_slug)?;
    let config = ServerConfig::from_env();
    let plane = Arc::new(ControlPlane::new(model, with_border));
    plane
        .autoload_model(model)
        .context("autoload ROM for embedded agent server")?;
    spawn(config, plane).map(Some)
}
