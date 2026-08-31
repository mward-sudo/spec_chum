//! Standalone loopback agent debug server (`spec-chum-agent`).

use std::sync::Arc;

use agent_server::routes::serve;
use anyhow::Context;
use clap::Parser;
use control_plane::{parse_model_slug, ControlPlane, ServerConfig};
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
    /// Allow unauthenticated mutations without a token (dev only).
    #[arg(long)]
    insecure: bool,
}

fn parse_model(s: &str) -> anyhow::Result<ModelId> {
    parse_model_slug(s).map_err(|e| anyhow::anyhow!("{e}"))
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
    if cli.insecure {
        config.insecure = true;
    }
    let model = parse_model(&cli.model)?;
    let plane = Arc::new(ControlPlane::new(model, cli.border));
    plane
        .autoload_model(model)
        .context("autoload ROM for selected model")?;
    serve(config, plane, None).await
}
