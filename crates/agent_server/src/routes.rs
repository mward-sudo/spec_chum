//! Loopback HTTP routes for the Spec Chum agent debug API.

use std::sync::Arc;

use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, HeaderMap, HeaderValue, StatusCode},
    response::{IntoResponse, Response},
    routing::{delete, get, post},
    Json, Router,
};
use control_plane::{
    ApiError, ControlPlane, ErrorBody, FramebufferMeta, ServerConfig, TraceFormat,
};
use machine::{TapeLoadOptions, Watch};
use serde::{Deserialize, Serialize};
use tower_http::trace::TraceLayer;

pub type SharedPlane = Arc<ControlPlane>;

#[derive(Clone, Debug)]
pub struct AppState {
    pub plane: SharedPlane,
    pub token: Option<String>,
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/v1/health", get(health))
        .route("/v1/status", get(status))
        .route("/v1/inspect", get(inspect))
        .route("/v1/framebuffer", get(framebuffer))
        .route("/v1/model", post(set_model))
        .route("/v1/reset", post(reset))
        .route("/v1/running", post(set_running))
        .route("/v1/run", post(run))
        .route("/v1/step", post(step))
        .route("/v1/run-until", post(run_until))
        .route("/v1/peek", get(peek))
        .route("/v1/poke", post(poke))
        .route("/v1/disasm", get(disasm))
        .route("/v1/tape/open", post(tape_open))
        .route("/v1/tape/play", post(tape_play))
        .route("/v1/tape/pause", post(tape_pause))
        .route("/v1/tape/rewind", post(tape_rewind))
        .route("/v1/tape/load", post(tape_load))
        .route("/v1/type-load", post(type_load))
        .route("/v1/border", post(set_border))
        .route(
            "/v1/debug/breakpoints",
            get(list_breakpoints).post(add_breakpoint),
        )
        .route("/v1/debug/breakpoints/{pc}", delete(remove_breakpoint))
        .route("/v1/debug/watches", get(list_watches).post(add_watch))
        .route(
            "/v1/trace/categories",
            get(trace_categories).put(set_trace_categories),
        )
        .route("/v1/trace", get(trace_dump))
        .route("/v1/trace/clear", post(trace_clear))
        .route("/v1/rom/setup", get(rom_setup))
        .route("/v1/timex/dock", post(insert_dck).delete(eject_dck))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Optional one-shot channel signaled after the listen socket binds successfully.
pub type ReadySender = std::sync::mpsc::SyncSender<Result<(), String>>;

pub async fn serve(
    config: ServerConfig,
    plane: SharedPlane,
    ready: Option<ReadySender>,
) -> anyhow::Result<()> {
    control_plane::ServerConfig::validate_bind_host(&config.host)?;
    let addr = (config.host.as_str(), config.port);
    let state = AppState {
        plane,
        token: config.token.clone(),
    };
    let app = router(state);
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(e) => {
            if let Some(tx) = ready {
                let _ = tx.send(Err(format!("bind failed: {e}")));
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

fn check_auth(headers: &HeaderMap, token: &Option<String>) -> Result<(), ApiError> {
    let Some(expected) = token else {
        return Ok(());
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

fn api_error(err: ApiError) -> Response {
    let status =
        StatusCode::from_u16(err.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
    (status, Json(ErrorBody::from(err))).into_response()
}

async fn health(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match check_auth(&headers, &state.token) {
        Ok(()) => match state.plane.health() {
            Ok(body) => Json(body).into_response(),
            Err(e) => api_error(e),
        },
        Err(e) => api_error(e),
    }
}

async fn status(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_json(&state, &headers, || state.plane.status())
}

async fn inspect(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match check_auth(&headers, &state.token) {
        Ok(()) => match state.plane.inspect_json() {
            Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
            Err(e) => api_error(e),
        },
        Err(e) => api_error(e),
    }
}

#[derive(Debug, Deserialize)]
struct FramebufferQuery {
    #[serde(default)]
    border: Option<bool>,
    #[serde(default = "default_png")]
    format: String,
}

fn default_png() -> String {
    "png".into()
}

async fn framebuffer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<FramebufferQuery>,
) -> Response {
    if let Err(e) = check_auth(&headers, &state.token) {
        return api_error(e);
    }
    let restore_border = if let Some(border) = q.border {
        let prev = state.plane.framebuffer_meta().ok().map(|m| m.border);
        if let Err(e) = state.plane.set_border(border) {
            return api_error(e);
        }
        if let Err(e) = state.plane.run_frames(1) {
            return api_error(e);
        }
        prev.filter(|p| *p != border)
    } else {
        None
    };
    let meta = match state.plane.framebuffer_meta() {
        Ok(m) => m,
        Err(e) => return api_error(e),
    };
    let response = match q.format.to_ascii_lowercase().as_str() {
        "rgba" => rgba_response(&state, meta),
        "png" | "" => png_response(&state, meta),
        other => api_error(ApiError::BadRequest(format!(
            "unknown framebuffer format: {other}"
        ))),
    };
    if let Some(prev) = restore_border {
        let _ = state.plane.set_border(prev);
    }
    response
}

fn rgba_response(state: &AppState, meta: FramebufferMeta) -> Response {
    match state.plane.framebuffer_rgba() {
        Ok(bytes) => {
            let mut resp = Response::new(Body::from(bytes));
            let h = resp.headers_mut();
            h.insert(
                header::CONTENT_TYPE,
                HeaderValue::from_static("application/octet-stream"),
            );
            insert_meta_headers(h, &meta);
            resp
        }
        Err(e) => api_error(e),
    }
}

fn png_response(state: &AppState, meta: FramebufferMeta) -> Response {
    match state.plane.framebuffer_png() {
        Ok(bytes) => {
            let mut resp = Response::new(Body::from(bytes));
            let h = resp.headers_mut();
            h.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
            insert_meta_headers(h, &meta);
            resp
        }
        Err(e) => api_error(e),
    }
}

fn insert_meta_headers(headers: &mut HeaderMap, meta: &FramebufferMeta) {
    let _ = headers.insert(
        "X-SpecChum-Width",
        HeaderValue::from_str(&meta.width.to_string()).unwrap_or(HeaderValue::from_static("0")),
    );
    let _ = headers.insert(
        "X-SpecChum-Height",
        HeaderValue::from_str(&meta.height.to_string()).unwrap_or(HeaderValue::from_static("0")),
    );
    let _ = headers.insert(
        "X-SpecChum-Border",
        HeaderValue::from_static(if meta.border { "true" } else { "false" }),
    );
    let _ = headers.insert(
        "X-SpecChum-Hires",
        HeaderValue::from_static(if meta.hires { "true" } else { "false" }),
    );
    let _ = headers.insert(
        "X-SpecChum-Model",
        HeaderValue::from_str(&meta.model).unwrap_or(HeaderValue::from_static("unknown")),
    );
    if let Some(mode) = meta.scld_mode {
        let _ = headers.insert(
            "X-SpecChum-Scld-Mode",
            HeaderValue::from_str(&mode.to_string()).unwrap_or(HeaderValue::from_static("0")),
        );
    }
}

#[derive(Debug, Deserialize)]
struct ModelBody {
    model: String,
}

async fn set_model(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ModelBody>,
) -> Response {
    auth_empty(&state, &headers, || state.plane.set_model(&body.model))
}

async fn reset(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_empty(&state, &headers, || state.plane.reset())
}

#[derive(Debug, Deserialize)]
struct RunningBody {
    running: bool,
}

async fn set_running(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RunningBody>,
) -> Response {
    auth_empty(&state, &headers, || state.plane.set_running(body.running))
}

#[derive(Debug, Deserialize)]
struct RunBody {
    #[serde(default = "default_one")]
    frames: u32,
}

fn default_one() -> u32 {
    1
}

async fn run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RunBody>,
) -> Response {
    let plane = state.plane.clone();
    auth_json_blocking(&state, &headers, move || plane.run_frames(body.frames)).await
}

#[derive(Debug, Deserialize)]
struct StepBody {
    #[serde(default = "default_one")]
    count: u32,
}

async fn step(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<StepBody>,
) -> Response {
    let plane = state.plane.clone();
    auth_empty_blocking(&state, &headers, move || plane.step(body.count)).await
}

#[derive(Debug, Deserialize)]
struct RunUntilBody {
    #[serde(default = "default_max_insn")]
    max_insns: u32,
}

fn default_max_insn() -> u32 {
    10_000_000
}

async fn run_until(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RunUntilBody>,
) -> Response {
    let plane = state.plane.clone();
    auth_json_blocking(&state, &headers, move || {
        plane.run_until_break(body.max_insns)
    })
    .await
}

#[derive(Debug, Deserialize)]
struct PeekQuery {
    addr: String,
    #[serde(default = "default_peek_len")]
    len: u16,
}

fn default_peek_len() -> u16 {
    64
}

async fn peek(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PeekQuery>,
) -> Response {
    let addr = match parse_addr(&q.addr) {
        Ok(a) => a,
        Err(e) => return api_error(e),
    };
    match check_auth(&headers, &state.token) {
        Ok(()) => match state.plane.peek(addr, q.len) {
            Ok(text) => {
                ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], text).into_response()
            }
            Err(e) => api_error(e),
        },
        Err(e) => api_error(e),
    }
}

#[derive(Debug, Deserialize)]
struct PokeBody {
    addr: String,
    value: u8,
}

async fn poke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PokeBody>,
) -> Response {
    let addr = match parse_addr(&body.addr) {
        Ok(a) => a,
        Err(e) => return api_error(e),
    };
    auth_empty(&state, &headers, || state.plane.poke(addr, body.value))
}

#[derive(Debug, Deserialize)]
struct DisasmQuery {
    addr: Option<String>,
    #[serde(default = "default_disasm_count")]
    count: usize,
}

fn default_disasm_count() -> usize {
    16
}

async fn disasm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<DisasmQuery>,
) -> Response {
    let addr = match q.addr.as_deref().map(parse_addr).transpose() {
        Ok(a) => a,
        Err(e) => return api_error(e),
    };
    match check_auth(&headers, &state.token) {
        Ok(()) => match state.plane.disasm(addr, q.count) {
            Ok(text) => {
                ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], text).into_response()
            }
            Err(e) => api_error(e),
        },
        Err(e) => api_error(e),
    }
}

#[derive(Debug, Deserialize)]
struct PathBody {
    path: String,
}

async fn tape_open(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PathBody>,
) -> Response {
    auth_empty(&state, &headers, || {
        state.plane.tape_open(body.path.as_ref())
    })
}

async fn tape_play(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_empty(&state, &headers, || state.plane.tape_play())
}

async fn tape_pause(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_empty(&state, &headers, || state.plane.tape_pause())
}

async fn tape_rewind(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_empty(&state, &headers, || state.plane.tape_rewind())
}

#[derive(Debug, Deserialize)]
struct TapeLoadBody {
    #[serde(default = "default_flash")]
    flash_load: bool,
    #[serde(default)]
    ear_load: bool,
    #[serde(default)]
    experience_load: bool,
    #[serde(default = "default_one")]
    speed: u32,
}

fn default_flash() -> bool {
    true
}

async fn tape_load(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<TapeLoadBody>,
) -> Response {
    let flash_load = if body.ear_load {
        false
    } else {
        body.flash_load
    };
    auth_empty(&state, &headers, || {
        state.plane.tape_load_options(TapeLoadOptions {
            flash_load,
            experience_load: body.experience_load,
            speed: body.speed,
        })
    })
}

#[derive(Debug, Deserialize)]
struct TypeLoadBody {
    #[serde(default)]
    code: bool,
    #[serde(default = "default_warmup")]
    warmup: u32,
    #[serde(default)]
    max: u32,
}

fn default_warmup() -> u32 {
    200
}

async fn type_load(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<TypeLoadBody>,
) -> Response {
    let plane = state.plane.clone();
    auth_json_blocking(&state, &headers, move || {
        plane.type_load(body.code, body.warmup, body.max)
    })
    .await
}

#[derive(Debug, Deserialize)]
struct BorderBody {
    with_border: bool,
}

async fn set_border(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BorderBody>,
) -> Response {
    auth_empty(&state, &headers, || {
        state.plane.set_border(body.with_border)
    })
}

async fn list_breakpoints(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_json(&state, &headers, || state.plane.list_breakpoints())
}

#[derive(Debug, Deserialize)]
struct BreakpointBody {
    pc: String,
}

async fn add_breakpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BreakpointBody>,
) -> Response {
    let pc = match parse_addr(&body.pc) {
        Ok(a) => a,
        Err(e) => return api_error(e),
    };
    auth_empty(&state, &headers, || state.plane.add_breakpoint(pc))
}

async fn remove_breakpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(pc): axum::extract::Path<String>,
) -> Response {
    let pc = match parse_addr(&pc) {
        Ok(a) => a,
        Err(e) => return api_error(e),
    };
    auth_empty(&state, &headers, || state.plane.remove_breakpoint(pc))
}

async fn list_watches(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_json(&state, &headers, || state.plane.list_watches())
}

#[derive(Debug, Deserialize)]
struct WatchBody {
    addr: String,
    #[serde(default)]
    read: bool,
    #[serde(default)]
    write: bool,
}

async fn add_watch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<WatchBody>,
) -> Response {
    let addr = match parse_addr(&body.addr) {
        Ok(a) => a,
        Err(e) => return api_error(e),
    };
    if !body.read && !body.write {
        return api_error(ApiError::BadRequest(
            "watch must enable read and/or write".into(),
        ));
    }
    let watch = Watch {
        addr,
        read: body.read,
        write: body.write,
    };
    auth_empty(&state, &headers, || state.plane.add_mem_watch(watch))
}

async fn trace_categories(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_json(&state, &headers, || state.plane.trace_categories())
}

#[derive(Debug, Deserialize)]
struct TraceCategoriesBody {
    categories: String,
}

async fn set_trace_categories(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<TraceCategoriesBody>,
) -> Response {
    auth_empty(&state, &headers, || {
        state.plane.set_trace_categories(&body.categories)
    })
}

#[derive(Debug, Deserialize)]
struct TraceQuery {
    #[serde(default)]
    format: Option<String>,
    last: Option<usize>,
}

async fn trace_dump(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<TraceQuery>,
) -> Response {
    let format = match q
        .format
        .as_deref()
        .unwrap_or("text")
        .to_ascii_lowercase()
        .as_str()
    {
        "json" => TraceFormat::Json,
        "ndjson" => TraceFormat::Ndjson,
        "text" | "" => TraceFormat::Text,
        other => {
            return api_error(ApiError::BadRequest(format!(
                "unknown trace format: {other}"
            )))
        }
    };
    match check_auth(&headers, &state.token) {
        Ok(()) => match state.plane.trace_dump(format, q.last) {
            Ok(text) => {
                let ctype = match format {
                    TraceFormat::Json | TraceFormat::Ndjson => "application/json",
                    TraceFormat::Text => "text/plain; charset=utf-8",
                };
                ([(header::CONTENT_TYPE, ctype)], text).into_response()
            }
            Err(e) => api_error(e),
        },
        Err(e) => api_error(e),
    }
}

async fn trace_clear(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_empty(&state, &headers, || state.plane.trace_clear())
}

async fn rom_setup(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match check_auth(&headers, &state.token) {
        Ok(()) => match state.plane.rom_setup() {
            Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
            Err(e) => api_error(e),
        },
        Err(e) => api_error(e),
    }
}

async fn insert_dck(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PathBody>,
) -> Response {
    auth_empty(&state, &headers, || {
        state.plane.insert_dck(body.path.as_ref())
    })
}

async fn eject_dck(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_empty(&state, &headers, || state.plane.eject_dck())
}

fn auth_empty(
    state: &AppState,
    headers: &HeaderMap,
    f: impl FnOnce() -> Result<(), ApiError>,
) -> Response {
    match check_auth(headers, &state.token) {
        Ok(()) => match f() {
            Ok(()) => StatusCode::NO_CONTENT.into_response(),
            Err(e) => api_error(e),
        },
        Err(e) => api_error(e),
    }
}

fn auth_json<T: Serialize>(
    state: &AppState,
    headers: &HeaderMap,
    f: impl FnOnce() -> Result<T, ApiError>,
) -> Response {
    match check_auth(headers, &state.token) {
        Ok(()) => match f() {
            Ok(body) => Json(body).into_response(),
            Err(e) => api_error(e),
        },
        Err(e) => api_error(e),
    }
}

async fn auth_json_blocking<T, F>(state: &AppState, headers: &HeaderMap, f: F) -> Response
where
    T: Serialize + Send + 'static,
    F: FnOnce() -> Result<T, ApiError> + Send + 'static,
{
    match check_auth(headers, &state.token) {
        Ok(()) => match tokio::task::spawn_blocking(f).await {
            Ok(Ok(body)) => Json(body).into_response(),
            Ok(Err(e)) => api_error(e),
            Err(e) => api_error(ApiError::Message(format!("task join: {e}"))),
        },
        Err(e) => api_error(e),
    }
}

async fn auth_empty_blocking<F>(state: &AppState, headers: &HeaderMap, f: F) -> Response
where
    F: FnOnce() -> Result<(), ApiError> + Send + 'static,
{
    match check_auth(headers, &state.token) {
        Ok(()) => match tokio::task::spawn_blocking(f).await {
            Ok(Ok(())) => StatusCode::NO_CONTENT.into_response(),
            Ok(Err(e)) => api_error(e),
            Err(e) => api_error(ApiError::Message(format!("task join: {e}"))),
        },
        Err(e) => api_error(e),
    }
}

fn parse_addr(s: &str) -> Result<u16, ApiError> {
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
    use axum::http::Request;
    use control_plane::ControlPlane;
    use spec_chum_host::ModelId;
    use tower::ServiceExt;

    use super::*;

    fn test_app() -> Router {
        let plane = Arc::new(ControlPlane::new(ModelId::Spectrum48, false));
        router(AppState { plane, token: None })
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
