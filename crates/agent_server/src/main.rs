//! Standalone loopback agent debug server (`spec-chum-agent`).

use std::sync::Arc;

use agent_server::routes::serve;
use anyhow::Context;
use clap::Parser;
use control_plane::{ControlPlane, ServerConfig};
use spec_chum_host::ModelId;

#[derive(Parser, Debug)]
#[command(
    name = "spec-chum-agent",
    about = "Spec Chum loopback agent debug HTTP API"
)]
struct Cli {
    /// Spectrum model to autoload on startup.
    #[arg(long, default_value = "48k")]
    model: String,
    /// Render ULA border in framebuffer exports.
    #[arg(long)]
    border: bool,
    /// Listen port (overrides `SPEC_CHUM_AGENT_PORT`).
    #[arg(long)]
    port: Option<u16>,
    /// Bearer token (overrides `SPEC_CHUM_AGENT_TOKEN`).
    #[arg(long)]
    token: Option<String>,
}

fn parse_model(s: &str) -> anyhow::Result<ModelId> {
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    trace::init_from_env();
    let cli = Cli::parse();
    let mut config = ServerConfig::from_env();
    if let Some(port) = cli.port {
        config.port = port;
    }
    if let Some(token) = cli.token {
        config.token = Some(token);
    }
    let model = parse_model(&cli.model)?;
    let plane = Arc::new(ControlPlane::new(model, cli.border));
    plane
        .autoload_model(model)
        .context("autoload ROM for selected model")?;
    serve(config, plane).await
}
