//! Background loopback agent server for hosts (#210 Phase B / #221).

use std::sync::Arc;
use std::thread::{self, JoinHandle};
use std::time::Duration;

use anyhow::{Context, Result};
use control_plane::{ControlPlane, ServerConfig};

use crate::routes::{serve, ReadyError};

/// Keeps the embedded HTTP server thread alive for the host process lifetime.
#[derive(Debug)]
pub struct EmbeddedServer {
    _thread: JoinHandle<()>,
    pub addr: String,
}

const READY_TIMEOUT: Duration = Duration::from_secs(5);

/// Spawn a loopback agent server on a background thread (non-blocking for UI hosts).
pub fn spawn(config: ServerConfig, plane: Arc<ControlPlane>) -> Result<EmbeddedServer> {
    control_plane::ServerConfig::validate_bind_host(&config.host)?;
    config.validate_auth_config()?;
    let addr = config.socket_addr();
    let (ready_tx, ready_rx) = std::sync::mpsc::sync_channel(1);
    let thread = thread::Builder::new()
        .name("spec-chum-agent".into())
        .spawn(move || {
            let rt = match tokio::runtime::Runtime::new() {
                Ok(rt) => rt,
                Err(e) => {
                    let _ = ready_tx.send(Err(ReadyError::Runtime(e)));
                    return;
                }
            };
            if let Err(e) = rt.block_on(serve(config, plane, Some(ready_tx))) {
                eprintln!("spec-chum-agent embedded server error: {e}");
            }
        })
        .context("spawn embedded agent thread")?;
    match ready_rx.recv_timeout(READY_TIMEOUT) {
        Ok(Ok(())) => {
            eprintln!("spec-chum-agent embedded listening on http://{addr}");
            Ok(EmbeddedServer {
                _thread: thread,
                addr,
            })
        }
        Ok(Err(err)) => Err(err.into()),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => Err(anyhow::anyhow!(
            "embedded agent server did not become ready in time"
        )),
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => Err(anyhow::anyhow!(
            "embedded agent server exited before becoming ready"
        )),
    }
}

/// When `SPEC_CHUM_AGENT=1`, start an embedded loopback server on the **shared**
/// live [`ControlPlane`] (same machine as the host — #221).
///
/// egui builds `ControlPlane::from_shared(session.shared_host())` then calls this
/// so HTTP and the Debug panel share one live session.
pub fn spawn_from_env_with_plane(plane: Arc<ControlPlane>) -> Result<Option<EmbeddedServer>> {
    if std::env::var("SPEC_CHUM_AGENT").ok().as_deref() != Some("1") {
        return Ok(None);
    }
    let config = ServerConfig::from_env();
    spawn(config, plane).map(Some)
}

/// Backward-compatible helper: builds a **fresh** plane (not GUI-backed).
///
/// Prefer [`spawn_from_env_with_plane`] from egui so HTTP and Debug share state.
#[deprecated(note = "use spawn_from_env_with_plane with the GUI ControlPlane (#221)")]
pub fn spawn_from_env() -> Result<Option<EmbeddedServer>> {
    use control_plane::parse_model_slug;

    if std::env::var("SPEC_CHUM_AGENT").ok().as_deref() != Some("1") {
        return Ok(None);
    }
    let model_slug = std::env::var("SPEC_CHUM_AGENT_MODEL").unwrap_or_else(|_| "48k".into());
    let with_border = std::env::var("SPEC_CHUM_AGENT_BORDER")
        .is_ok_and(|v| v == "1" || v.eq_ignore_ascii_case("true"));
    let model = parse_model_slug(&model_slug).map_err(|e| anyhow::anyhow!("{e}"))?;
    let config = ServerConfig::from_env();
    let plane = Arc::new(ControlPlane::new(model, with_border));
    plane
        .autoload_model(model)
        .context("autoload ROM for embedded agent server")?;
    spawn(config, plane).map(Some)
}

#[cfg(test)]
mod tests {
    use std::net::TcpListener;
    use std::sync::Arc;

    use control_plane::{parse_model_slug, ControlPlane, ServerConfig};
    use spec_chum_host::ModelId;

    use super::*;

    #[test]
    fn parse_model_accepts_timex_ts2068_alias() {
        assert_eq!(
            parse_model_slug("timex_ts2068").expect("alias"),
            ModelId::TimexTS2068
        );
    }

    #[test]
    fn spawn_fails_when_port_in_use() {
        let listener = TcpListener::bind("127.0.0.1:0").expect("ephemeral bind");
        let port = listener.local_addr().expect("addr").port();
        let config = ServerConfig {
            host: "127.0.0.1".into(),
            port,
            token: None,
            insecure: true,
        };
        let plane = Arc::new(ControlPlane::new(ModelId::Spectrum48, false));
        let result = spawn(config, plane);
        assert!(
            result.is_err(),
            "spawn should fail when the listen port is already taken"
        );
    }

    #[test]
    fn spawn_from_env_with_plane_skips_when_unset() {
        // Ensure env is not forcing a listen in unit tests.
        std::env::remove_var("SPEC_CHUM_AGENT");
        let plane = Arc::new(ControlPlane::new(ModelId::Spectrum48, false));
        let server = spawn_from_env_with_plane(plane).expect("spawn");
        assert!(server.is_none());
    }
}
