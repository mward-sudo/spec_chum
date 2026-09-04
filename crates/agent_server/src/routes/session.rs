//! Model / config / run / step / border routes.

use axum::{extract::State, http::HeaderMap, response::Response, Json};
use serde::Deserialize;
use spec_chum_host::UserMachineConfig;

use super::{auth_empty, auth_empty_blocking, auth_json, auth_json_blocking, AppState};

fn default_one() -> u32 {
    super::default_one()
}

pub(crate) async fn apply_config(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<UserMachineConfig>,
) -> Response {
    auth_empty(&state, &headers, || state.plane.apply_config(&body))
}
#[derive(Debug, Deserialize)]
pub(crate) struct ModelBody {
    model: String,
}

pub(crate) async fn set_model(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<ModelBody>,
) -> Response {
    auth_empty(&state, &headers, || state.plane.set_model(&body.model))
}

pub(crate) async fn reset(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_empty(&state, &headers, || state.plane.reset())
}

#[derive(Debug, Deserialize)]
pub(crate) struct RunningBody {
    running: bool,
}

pub(crate) async fn set_running(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RunningBody>,
) -> Response {
    auth_empty(&state, &headers, || state.plane.set_running(body.running))
}

#[derive(Debug, Deserialize)]
pub(crate) struct RunBody {
    #[serde(default = "default_one")]
    frames: u32,
}

pub(crate) async fn run(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<RunBody>,
) -> Response {
    let plane = state.plane.clone();
    auth_json_blocking(&state, &headers, move || plane.run_frames(body.frames)).await
}

#[derive(Debug, Deserialize)]
pub(crate) struct StepBody {
    #[serde(default = "default_one")]
    count: u32,
}

pub(crate) async fn step(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<StepBody>,
) -> Response {
    let plane = state.plane.clone();
    auth_empty_blocking(&state, &headers, move || plane.step(body.count)).await
}

pub(crate) async fn continue_execution(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    auth_json(&state, &headers, || state.plane.continue_execution())
}

#[derive(Debug, Deserialize)]
pub(crate) struct RunUntilBody {
    #[serde(default = "default_max_insn")]
    max_insns: u32,
}

pub(crate) fn default_max_insn() -> u32 {
    10_000_000
}

pub(crate) async fn run_until(
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
pub(crate) struct BorderBody {
    with_border: bool,
}

pub(crate) async fn set_border(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<BorderBody>,
) -> Response {
    auth_empty(&state, &headers, || {
        state.plane.set_border(body.with_border)
    })
}
