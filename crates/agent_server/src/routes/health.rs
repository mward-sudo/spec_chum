//! Health / status / last-error routes.

use axum::{
    extract::State,
    http::HeaderMap,
    response::{IntoResponse, Response},
    Json,
};
use serde::Serialize;

use super::{api_error, auth_json, check_auth, AppState};

pub(crate) async fn health(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match check_auth(&state, &headers) {
        Ok(()) => match state.plane.health() {
            Ok(body) => Json(body).into_response(),
            Err(e) => api_error(&state.plane, e),
        },
        Err(e) => api_error(&state.plane, e),
    }
}

pub(crate) async fn status(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_json(&state, &headers, || state.plane.status())
}
#[derive(Clone, Debug, Serialize)]
pub(crate) struct LastErrorResponse {
    last: Option<control_plane::LastErrorRecord>,
}

pub(crate) async fn last_error(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_json(&state, &headers, || {
        let record = state.plane.last_error();
        Ok(LastErrorResponse {
            last: if record.error.is_empty() {
                None
            } else {
                Some(record)
            },
        })
    })
}
