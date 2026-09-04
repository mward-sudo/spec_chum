//! ROM / snapshot / disk / tape / type-load routes.

use axum::{extract::State, http::HeaderMap, response::Response, Json};
use machine::TapeLoadOptions;
use serde::Deserialize;

use super::{auth_empty, auth_json_blocking, AppState, PathBody};

fn default_one() -> u32 {
    super::default_one()
}

pub(crate) async fn load_rom(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PathBody>,
) -> Response {
    auth_empty(&state, &headers, || {
        state.plane.load_rom_path(body.path.as_ref())
    })
}

pub(crate) async fn load_snapshot(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PathBody>,
) -> Response {
    auth_empty(&state, &headers, || {
        state.plane.load_snapshot(body.path.as_ref())
    })
}

pub(crate) async fn load_rzx(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PathBody>,
) -> Response {
    auth_empty(&state, &headers, || {
        state.plane.load_rzx(body.path.as_ref())
    })
}

pub(crate) async fn load_dsk(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PathBody>,
) -> Response {
    auth_empty(&state, &headers, || {
        state.plane.load_dsk(body.path.as_ref())
    })
}

pub(crate) async fn load_trd(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PathBody>,
) -> Response {
    auth_empty(&state, &headers, || {
        state.plane.load_trd(body.path.as_ref())
    })
}

pub(crate) async fn tape_open(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PathBody>,
) -> Response {
    auth_empty(&state, &headers, || {
        state.plane.tape_open(body.path.as_ref())
    })
}

pub(crate) async fn tape_play(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_empty(&state, &headers, || state.plane.tape_play())
}

pub(crate) async fn tape_pause(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_empty(&state, &headers, || state.plane.tape_pause())
}

pub(crate) async fn tape_rewind(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_empty(&state, &headers, || state.plane.tape_rewind())
}

pub(crate) async fn tape_eject(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_empty(&state, &headers, || state.plane.tape_eject())
}

#[derive(Debug, Deserialize)]
pub(crate) struct TapeLoadBody {
    #[serde(default = "default_flash")]
    flash_load: bool,
    #[serde(default)]
    ear_load: bool,
    #[serde(default)]
    experience_load: bool,
    #[serde(default = "default_one")]
    speed: u32,
}

pub(crate) fn default_flash() -> bool {
    true
}

pub(crate) async fn tape_load(
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
pub(crate) struct TypeLoadBody {
    #[serde(default)]
    code: bool,
    #[serde(default = "default_warmup")]
    warmup: u32,
    #[serde(default)]
    max: u32,
}

pub(crate) fn default_warmup() -> u32 {
    200
}

pub(crate) async fn type_load(
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
