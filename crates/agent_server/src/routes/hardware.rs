//! Hardware attach / MDR / Timex dock / ROM setup routes.

use axum::{
    extract::State,
    http::{header, HeaderMap},
    response::{IntoResponse, Response},
    Json,
};

use super::{
    api_error, auth_empty, auth_empty_blocking, auth_json, check_auth, AppState, PathBody,
};

pub(crate) async fn rom_setup(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match check_auth(&state, &headers) {
        Ok(()) => match state.plane.rom_setup() {
            Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
            Err(e) => api_error(&state.plane, e),
        },
        Err(e) => api_error(&state.plane, e),
    }
}

pub(crate) async fn insert_dck(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PathBody>,
) -> Response {
    let plane = state.plane.clone();
    let path = body.path;
    auth_empty_blocking(&state, &headers, move || plane.insert_dck(path.as_ref())).await
}

pub(crate) async fn eject_dck(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_empty(&state, &headers, || state.plane.eject_dck())
}

pub(crate) async fn hardware_status(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_json(&state, &headers, || state.plane.hardware_status())
}

pub(crate) async fn attach_multiface(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PathBody>,
) -> Response {
    let plane = state.plane.clone();
    let path = body.path;
    auth_empty_blocking(&state, &headers, move || {
        plane.attach_multiface(path.as_ref())
    })
    .await
}

pub(crate) async fn multiface_nmi(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_empty(&state, &headers, || state.plane.multiface_nmi())
}

pub(crate) async fn attach_interface1(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    let plane = state.plane.clone();
    auth_empty_blocking(&state, &headers, move || plane.attach_interface1()).await
}

pub(crate) async fn load_interface1_rom(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PathBody>,
) -> Response {
    let plane = state.plane.clone();
    let path = body.path;
    auth_empty_blocking(&state, &headers, move || {
        plane.load_interface1_rom(path.as_ref())
    })
    .await
}

pub(crate) async fn insert_mdr(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PathBody>,
) -> Response {
    let plane = state.plane.clone();
    let path = body.path;
    auth_empty_blocking(&state, &headers, move || plane.insert_mdr(path.as_ref())).await
}

pub(crate) async fn attach_divmmc(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_empty(&state, &headers, || state.plane.attach_divmmc())
}

pub(crate) async fn attach_beta(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_empty(&state, &headers, || state.plane.attach_beta())
}

pub(crate) async fn load_divmmc_sd(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PathBody>,
) -> Response {
    let plane = state.plane.clone();
    let path = body.path;
    auth_empty_blocking(&state, &headers, move || {
        plane.load_divmmc_sd(path.as_ref())
    })
    .await
}

pub(crate) async fn load_divmmc_eeprom(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PathBody>,
) -> Response {
    let plane = state.plane.clone();
    let path = body.path;
    auth_empty_blocking(&state, &headers, move || {
        plane.load_divmmc_eeprom(path.as_ref())
    })
    .await
}

pub(crate) async fn load_trdos_rom(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PathBody>,
) -> Response {
    let plane = state.plane.clone();
    let path = body.path;
    auth_empty_blocking(&state, &headers, move || {
        plane.load_trdos_rom(path.as_ref())
    })
    .await
}
