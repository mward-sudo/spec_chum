//! Loopback HTTP routes for the Spec Chum agent debug API.

mod debug;
mod hardware;
mod health;
mod input;
mod media;
mod session;
mod trace;
mod video;

use std::sync::Arc;

use axum::{
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use control_plane::{ApiError, ErrorBody, ServerConfig};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tower_http::trace::TraceLayer;

pub type SharedPlane = Arc<control_plane::ControlPlane>;

#[derive(Clone, Debug)]
pub struct AppState {
    pub plane: SharedPlane,
    pub token: Option<String>,
    pub insecure: bool,
}

#[derive(Debug, Deserialize)]
pub(crate) struct PathBody {
    pub(crate) path: String,
}

pub(crate) fn default_one() -> u32 {
    1
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(health::health))
        .route("/v1/status", get(health::status))
        .route("/v1/inspect", get(video::inspect))
        .route("/v1/video", get(video::video))
        .route("/v1/memory/regions", get(video::memory_regions))
        .route("/v1/errors/last", get(health::last_error))
        .route("/v1/framebuffer", get(video::framebuffer))
        .route("/v1/host/display", get(video::host_display))
        .route("/v1/host/window", get(video::host_window))
        .route("/v1/model", post(session::set_model))
        .route("/v1/config", post(session::apply_config))
        .route("/v1/reset", post(session::reset))
        .route("/v1/keys", post(input::set_keys))
        .route("/v1/joystick", post(input::set_joystick))
        .route("/v1/mouse", post(input::set_mouse))
        .route("/v1/prefs", get(input::get_prefs).patch(input::patch_prefs))
        .route("/v1/running", post(session::set_running))
        .route("/v1/run", post(session::run))
        .route("/v1/step", post(session::step))
        .route("/v1/continue", post(session::continue_execution))
        .route("/v1/run-until", post(session::run_until))
        .route("/v1/peek", get(debug::peek))
        .route("/v1/poke", post(debug::poke))
        .route("/v1/regs", post(debug::patch_regs))
        .route("/v1/disasm", get(debug::disasm))
        .route("/v1/rom", post(media::load_rom))
        .route("/v1/snapshot", post(media::load_snapshot))
        .route("/v1/rzx", post(media::load_rzx))
        .route("/v1/dsk", post(media::load_dsk))
        .route("/v1/trd", post(media::load_trd))
        .route("/v1/tape/open", post(media::tape_open))
        .route("/v1/tape/play", post(media::tape_play))
        .route("/v1/tape/pause", post(media::tape_pause))
        .route("/v1/tape/rewind", post(media::tape_rewind))
        .route("/v1/tape/eject", post(media::tape_eject))
        .route("/v1/tape/load", post(media::tape_load))
        .route("/v1/type-load", post(media::type_load))
        .route("/v1/border", post(session::set_border))
        .route(
            "/v1/debug/breakpoints",
            get(debug::list_breakpoints).post(debug::add_breakpoint),
        )
        .route("/v1/debug/last-break", get(debug::last_break))
        .route(
            "/v1/debug/breakpoints/{pc}",
            delete(debug::remove_breakpoint),
        )
        .route(
            "/v1/debug/watches",
            get(debug::list_watches).post(debug::add_watch),
        )
        .route("/v1/debug/watches/{addr}", delete(debug::remove_mem_watch))
        .route(
            "/v1/debug/port-watches",
            get(debug::list_port_watches).post(debug::add_port_watch),
        )
        .route(
            "/v1/debug/port-watches/{addr}",
            delete(debug::remove_port_watch),
        )
        .route(
            "/v1/trace/categories",
            get(trace::trace_categories).put(trace::set_trace_categories),
        )
        .route("/v1/trace", get(trace::trace_dump))
        .route("/v1/trace/clear", post(trace::trace_clear))
        .route("/v1/rom/setup", get(hardware::rom_setup))
        .route(
            "/v1/timex/dock",
            post(hardware::insert_dck).delete(hardware::eject_dck),
        )
        .route("/v1/hardware", get(hardware::hardware_status))
        .route("/v1/hardware/multiface", post(hardware::attach_multiface))
        .route("/v1/hardware/multiface/nmi", post(hardware::multiface_nmi))
        .route("/v1/hardware/interface1", post(hardware::attach_interface1))
        .route(
            "/v1/hardware/interface1/rom",
            post(hardware::load_interface1_rom),
        )
        .route("/v1/hardware/mdr", post(hardware::insert_mdr))
        .route("/v1/hardware/divmmc", post(hardware::attach_divmmc))
        .route("/v1/hardware/divmmc/sd", post(hardware::load_divmmc_sd))
        .route(
            "/v1/hardware/divmmc/eeprom",
            post(hardware::load_divmmc_eeprom),
        )
        .route("/v1/hardware/beta", post(hardware::attach_beta))
        .route("/v1/hardware/trdos/rom", post(hardware::load_trdos_rom))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Failures signaled on the embedded-server ready channel before listen succeeds.
#[derive(Debug, Error)]
pub enum ReadyError {
    #[error("tokio runtime failed: {0}")]
    Runtime(#[source] std::io::Error),
    #[error("bind failed: {0}")]
    Bind(#[source] std::io::Error),
}

/// Optional one-shot channel signaled after the listen socket binds successfully.
pub type ReadySender = std::sync::mpsc::SyncSender<Result<(), ReadyError>>;

pub async fn serve(
    config: ServerConfig,
    plane: SharedPlane,
    ready: Option<ReadySender>,
) -> anyhow::Result<()> {
    control_plane::ServerConfig::validate_bind_host(&config.host)?;
    config
        .validate_auth_config()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    let addr = (config.host.as_str(), config.port);
    let state = AppState {
        plane,
        token: config.token.clone(),
        insecure: config.insecure,
    };
    let app = router(state);
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(e) => {
            if let Some(tx) = ready {
                let _ = tx.send(Err(ReadyError::Bind(std::io::Error::new(
                    e.kind(),
                    e.to_string(),
                ))));
            }
            return Err(e.into());
        }
    };
    if let Some(tx) = ready {
        let _ = tx.send(Ok(()));
    }
    eprintln!(
        "spec-chum-agent listening on http://{}:{}",
        config.host, config.port
    );
    axum::serve(listener, app).await?;
    Ok(())
}

pub(crate) fn check_auth(state: &AppState, headers: &HeaderMap) -> Result<(), ApiError> {
    let Some(expected) = &state.token else {
        return if state.insecure {
            Ok(())
        } else {
            Err(ApiError::Unauthorized)
        };
    };
    let auth = headers
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    let provided = auth.strip_prefix("Bearer ").unwrap_or(auth);
    if provided == expected {
        Ok(())
    } else {
        Err(ApiError::Unauthorized)
    }
}

pub(crate) fn api_error(plane: &SharedPlane, err: ApiError) -> Response {
    plane.record_error(&err);
    let status =
        StatusCode::from_u16(err.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(ErrorBody::from(err))).into_response()
}

pub(crate) fn auth_empty(
    state: &AppState,
    headers: &HeaderMap,
    f: impl FnOnce() -> Result<(), ApiError>,
) -> Response {
    match check_auth(state, headers) {
        Ok(()) => match f() {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => api_error(&state.plane, e),
        },
        Err(e) => api_error(&state.plane, e),
    }
}

pub(crate) fn auth_json<T: Serialize>(
    state: &AppState,
    headers: &HeaderMap,
    f: impl FnOnce() -> Result<T, ApiError>,
) -> Response {
    match check_auth(state, headers) {
        Ok(()) => match f() {
            Ok(body) => Json(body).into_response(),
            Err(e) => api_error(&state.plane, e),
        },
        Err(e) => api_error(&state.plane, e),
    }
}

pub(crate) async fn auth_json_blocking<T, F>(
    state: &AppState,
    headers: &HeaderMap,
    f: F,
) -> Response
where
    T: Serialize + Send + 'static,
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
{
    match check_auth(state, headers) {
        Ok(()) => match tokio::task::spawn_blocking(f).await {
            Ok(Ok(body)) => Json(body).into_response(),
            Ok(Err(e)) => api_error(&state.plane, e),
            Err(e) => api_error(&state.plane, ApiError::Message(format!("task join: {e}"))),
        },
        Err(e) => api_error(&state.plane, e),
    }
}

pub(crate) async fn auth_empty_blocking<F>(state: &AppState, headers: &HeaderMap, f: F) -> Response
where
    F: FnOnce() -> Result<(), ApiError> + Send + 'static,
{
    match check_auth(state, headers) {
        Ok(()) => match tokio::task::spawn_blocking(f).await {
            Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
            Ok(Err(e)) => api_error(&state.plane, e),
            Err(e) => api_error(&state.plane, ApiError::Message(format!("task join: {e}"))),
        },
        Err(e) => api_error(&state.plane, e),
    }
}

pub(crate) fn parse_addr(s: &str) -> Result<u16, ApiError> {
    let t = s.trim().trim_start_matches('$');
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return u16::from_str_radix(hex, 16)
            .map_err(|e| ApiError::BadRequest(format!("bad address: {e}")));
    }
    let looks_hex = t.len() == 4
        || t.bytes()
            .any(|b| b.is_ascii_hexdigit() && !b.is_ascii_digit());
    if looks_hex {
        u16::from_str_radix(t, 16)
            .or_else(|_| t.parse())
            .map_err(|e| ApiError::BadRequest(format!("bad address: {e}")))
    } else {
        t.parse()
            .map_err(|e| ApiError::BadRequest(format!("bad address: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use axum::body::Body;
    use axum::http::{header, Request, StatusCode};
    use control_plane::ControlPlane;
    use spec_chum_host::ModelId;
    use tower::ServiceExt;

    use super::*;

    #[test]
    fn ready_error_display_keeps_prefixes() {
        let bind = ReadyError::Bind(std::io::Error::new(
            std::io::ErrorKind::AddrInUse,
            "address already in use",
        ));
        assert!(bind.to_string().starts_with("bind failed:"), "got {bind}");
        let runtime = ReadyError::Runtime(std::io::Error::other("runtime boom"));
        assert!(
            runtime.to_string().starts_with("tokio runtime failed:"),
            "got {runtime}"
        );
    }

    fn test_app() -> Router {
        let plane = Arc::new(ControlPlane::new(ModelId::Spectrum48, false));
        router(AppState {
            plane,
            token: None,
            insecure: true,
        })
    }

    #[tokio::test]
    async fn health_endpoint_ok() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/v1/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn model_post_parses_json() {
        let app = test_app();
        let resp = app
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/model")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"model":"48k"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        // Without ROM on CI path, select may 500; endpoint must accept JSON.
        assert!(resp.status() == StatusCode::NO_CONTENT || resp.status().is_server_error());
    }
}
