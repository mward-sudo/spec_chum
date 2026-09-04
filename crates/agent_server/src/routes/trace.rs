//! Trace category / dump / clear routes.

use axum::{
    extract::{Query, State},
    http::{header, HeaderMap},
    response::{IntoResponse, Response},
    Json,
};
use control_plane::{ApiError, TraceFormat};
use serde::Deserialize;

use super::{api_error, auth_empty, auth_json, check_auth, AppState};

pub(crate) async fn trace_categories(
    State(state): State<AppState>,
    headers: HeaderMap,
) -> Response {
    auth_json(&state, &headers, || state.plane.trace_categories())
}

#[derive(Debug, Deserialize)]
pub(crate) struct TraceCategoriesBody {
    categories: String,
}

pub(crate) async fn set_trace_categories(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<TraceCategoriesBody>,
) -> Response {
    auth_empty(&state, &headers, || {
        state.plane.set_trace_categories(&body.categories)
    })
}

#[derive(Debug, Deserialize)]
pub(crate) struct TraceQuery {
    #[serde(default)]
    format: Option<String>,
    last: Option<usize>,
}

pub(crate) async fn trace_dump(
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
            return api_error(
                &state.plane,
                ApiError::BadRequest(format!("unknown trace format: {other}")),
            )
        }
    };
    match check_auth(&state, &headers) {
        Ok(()) => match state.plane.trace_dump(format, q.last) {
            Ok(text) => {
                let ctype = match format {
                    TraceFormat::Json | TraceFormat::Ndjson => "application/json",
                    TraceFormat::Text => "text/plain; charset=utf-8",
                };
                ([(header::CONTENT_TYPE, ctype)], text).into_response()
            }
            Err(e) => api_error(&state.plane, e),
        },
        Err(e) => api_error(&state.plane, e),
    }
}

pub(crate) async fn trace_clear(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_empty(&state, &headers, || state.plane.trace_clear())
}
