//! Breakpoints / watches / peek / poke / disasm / regs routes.

use axum::{
    extract::{Query, State},
    http::{header, HeaderMap},
    response::{IntoResponse, Response},
    Json,
};
use control_plane::ApiError;
use machine::Watch;
use serde::Deserialize;

use super::{api_error, auth_empty, auth_json, check_auth, parse_addr, AppState};

pub(crate) async fn last_break(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_json(&state, &headers, || state.plane.last_break())
}
#[derive(Debug, Deserialize)]
pub(crate) struct PeekQuery {
    addr: String,
    #[serde(default = "default_peek_len")]
    len: u16,
}

pub(crate) fn default_peek_len() -> u16 {
    64
}

pub(crate) async fn peek(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<PeekQuery>,
) -> Response {
    let addr = match parse_addr(&q.addr) {
        Ok(a) => a,
        Err(e) => return api_error(&state.plane, e),
    };
    match check_auth(&state, &headers) {
        Ok(()) => match state.plane.peek(addr, q.len) {
            Ok(text) => {
                ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], text).into_response()
            }
            Err(e) => api_error(&state.plane, e),
        },
        Err(e) => api_error(&state.plane, e),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct PokeBody {
    addr: String,
    value: u8,
}

pub(crate) async fn poke(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PokeBody>,
) -> Response {
    let addr = match parse_addr(&body.addr) {
        Ok(a) => a,
        Err(e) => return api_error(&state.plane, e),
    };
    auth_empty(&state, &headers, || state.plane.poke(addr, body.value))
}

#[derive(Debug, Deserialize)]
pub(crate) struct RegsBody {
    #[serde(default)]
    pc: Option<String>,
    #[serde(default)]
    sp: Option<String>,
    #[serde(default)]
    af: Option<String>,
}

pub(crate) async fn patch_regs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RegsBody>,
) -> Response {
    let pc = match body.pc.as_deref().map(parse_addr).transpose() {
        Ok(v) => v,
        Err(e) => return api_error(&state.plane, e),
    };
    let sp = match body.sp.as_deref().map(parse_addr).transpose() {
        Ok(v) => v,
        Err(e) => return api_error(&state.plane, e),
    };
    let af = match body.af.as_deref().map(parse_addr).transpose() {
        Ok(v) => v,
        Err(e) => return api_error(&state.plane, e),
    };
    let patch = spec_chum_host::RegsPatch { pc, sp, af };
    auth_json(&state, &headers, || state.plane.patch_regs(patch))
}

#[derive(Debug, Deserialize)]
pub(crate) struct DisasmQuery {
    addr: Option<String>,
    #[serde(default = "default_disasm_count")]
    count: usize,
}

pub(crate) fn default_disasm_count() -> usize {
    16
}

pub(crate) async fn disasm(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<DisasmQuery>,
) -> Response {
    let addr = match q.addr.as_deref().map(parse_addr).transpose() {
        Ok(a) => a,
        Err(e) => return api_error(&state.plane, e),
    };
    match check_auth(&state, &headers) {
        Ok(()) => match state.plane.disasm(addr, q.count) {
            Ok(text) => {
                ([(header::CONTENT_TYPE, "text/plain; charset=utf-8")], text).into_response()
            }
            Err(e) => api_error(&state.plane, e),
        },
        Err(e) => api_error(&state.plane, e),
    }
}
pub(crate) async fn list_breakpoints(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    auth_json(&state, &headers, || state.plane.list_breakpoints())
}

#[derive(Debug, Deserialize)]
pub(crate) struct BreakpointBody {
    pc: String,
}

pub(crate) async fn add_breakpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BreakpointBody>,
) -> Response {
    let pc = match parse_addr(&body.pc) {
        Ok(a) => a,
        Err(e) => return api_error(&state.plane, e),
    };
    auth_empty(&state, &headers, || state.plane.add_breakpoint(pc))
}

pub(crate) async fn remove_breakpoint(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(pc): axum::extract::Path<String>,
) -> Response {
    let pc = match parse_addr(&pc) {
        Ok(a) => a,
        Err(e) => return api_error(&state.plane, e),
    };
    auth_empty(&state, &headers, || state.plane.remove_breakpoint(pc))
}

pub(crate) async fn list_watches(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_json(&state, &headers, || state.plane.list_watches())
}

#[derive(Debug, Deserialize)]
pub(crate) struct WatchBody {
    addr: String,
    #[serde(default)]
    read: bool,
    #[serde(default)]
    write: bool,
}

pub(crate) async fn add_watch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<WatchBody>,
) -> Response {
    let addr = match parse_addr(&body.addr) {
        Ok(a) => a,
        Err(e) => return api_error(&state.plane, e),
    };
    if !body.read && !body.write {
        return api_error(
            &state.plane,
            ApiError::BadRequest("watch must enable read and/or write".into()),
        );
    }
    let watch = Watch {
        addr,
        read: body.read,
        write: body.write,
    };
    auth_empty(&state, &headers, || state.plane.add_mem_watch(watch))
}

pub(crate) async fn remove_mem_watch(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(addr): axum::extract::Path<String>,
) -> Response {
    let addr = match parse_addr(&addr) {
        Ok(a) => a,
        Err(e) => return api_error(&state.plane, e),
    };
    auth_empty(&state, &headers, || state.plane.remove_mem_watch(addr))
}

pub(crate) async fn list_port_watches(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    auth_json(&state, &headers, || state.plane.list_port_watches())
}

pub(crate) async fn add_port_watch(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<WatchBody>,
) -> Response {
    let addr = match parse_addr(&body.addr) {
        Ok(a) => a,
        Err(e) => return api_error(&state.plane, e),
    };
    if !body.read && !body.write {
        return api_error(
            &state.plane,
            ApiError::BadRequest("watch must enable read and/or write".into()),
        );
    }
    let watch = Watch {
        addr,
        read: body.read,
        write: body.write,
    };
    auth_empty(&state, &headers, || state.plane.add_port_watch(watch))
}

pub(crate) async fn remove_port_watch(
    State(state): State<AppState>,
    headers: HeaderMap,
    axum::extract::Path(addr): axum::extract::Path<String>,
) -> Response {
    let addr = match parse_addr(&addr) {
        Ok(a) => a,
        Err(e) => return api_error(&state.plane, e),
    };
    auth_empty(&state, &headers, || state.plane.remove_port_watch(addr))
}
