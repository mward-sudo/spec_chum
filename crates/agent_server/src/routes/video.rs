//! Inspect / video / framebuffer / host capture routes.

use axum::{
    body::Body,
    extract::{Query, State},
    http::{header, HeaderMap, HeaderValue},
    response::{IntoResponse, Response},
};
use control_plane::{ApiError, FramebufferMeta, PresentMeta};
use serde::Deserialize;

use super::{api_error, auth_json, check_auth, AppState};

pub(crate) async fn inspect(State(state): State<AppState>, headers: HeaderMap) -> Response {
    match check_auth(&state, &headers) {
        Ok(()) => match state.plane.inspect_json() {
            Ok(json) => ([(header::CONTENT_TYPE, "application/json")], json).into_response(),
            Err(e) => api_error(&state.plane, e),
        },
        Err(e) => api_error(&state.plane, e),
    }
}

pub(crate) async fn video(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_json(&state, &headers, || state.plane.video_meta())
}

pub(crate) async fn memory_regions(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_json(&state, &headers, || state.plane.memory_map())
}
#[derive(Debug, Deserialize)]
pub(crate) struct FramebufferQuery {
    #[serde(default)]
    border: Option<bool>,
    #[serde(default = "default_png")]
    format: String,
}

pub(crate) fn default_png() -> String {
    "png".into()
}

pub(crate) async fn framebuffer(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<FramebufferQuery>,
) -> Response {
    if let Err(e) = check_auth(&state, &headers) {
        return api_error(&state.plane, e);
    }
    let restore_border = if let Some(border) = q.border {
        let prev = state.plane.framebuffer_meta().ok().map(|m| m.border);
        if let Err(e) = state.plane.set_border(border) {
            return api_error(&state.plane, e);
        }
        if let Err(e) = state.plane.run_frames(1) {
            return api_error(&state.plane, e);
        }
        prev.filter(|p| *p != border)
    } else {
        None
    };
    let meta = match state.plane.framebuffer_meta() {
        Ok(m) => m,
        Err(e) => return api_error(&state.plane, e),
    };
    let response = match q.format.to_ascii_lowercase().as_str() {
        "rgba" => rgba_response(&state, meta),
        "png" | "" => png_response(&state, meta),
        other => api_error(
            &state.plane,
            ApiError::BadRequest(format!("unknown framebuffer format: {other}")),
        ),
    };
    if let Some(prev) = restore_border {
        let _ = state.plane.set_border(prev);
    }
    response
}

pub(crate) fn rgba_response(state: &AppState, meta: FramebufferMeta) -> Response {
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
        Err(e) => api_error(&state.plane, e),
    }
}

pub(crate) fn png_response(state: &AppState, meta: FramebufferMeta) -> Response {
    match state.plane.framebuffer_png() {
        Ok(bytes) => {
            let mut resp = Response::new(Body::from(bytes));
            let h = resp.headers_mut();
            h.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
            insert_meta_headers(h, &meta);
            resp
        }
        Err(e) => api_error(&state.plane, e),
    }
}

#[derive(Debug, Deserialize)]
pub(crate) struct HostDisplayQuery {
    #[serde(default)]
    width: Option<u32>,
    #[serde(default)]
    height: Option<u32>,
    #[serde(default)]
    scale: Option<u32>,
    #[serde(default = "default_png")]
    format: String,
}

pub(crate) async fn host_display(
    State(state): State<AppState>,
    headers: HeaderMap,
    Query(q): Query<HostDisplayQuery>,
) -> Response {
    if let Err(e) = check_auth(&state, &headers) {
        return api_error(&state.plane, e);
    }
    match q.format.to_ascii_lowercase().as_str() {
        "rgba" => match state
            .plane
            .host_display_presented_rgba(q.width, q.height, q.scale)
        {
            Ok((bytes, meta)) => {
                let mut resp = Response::new(Body::from(bytes));
                let h = resp.headers_mut();
                h.insert(
                    header::CONTENT_TYPE,
                    HeaderValue::from_static("application/octet-stream"),
                );
                insert_present_headers(h, &meta);
                resp
            }
            Err(e) => api_error(&state.plane, e),
        },
        "png" | "" => match state
            .plane
            .host_display_presented(q.width, q.height, q.scale)
        {
            Ok((bytes, meta)) => {
                let mut resp = Response::new(Body::from(bytes));
                let h = resp.headers_mut();
                h.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
                insert_present_headers(h, &meta);
                resp
            }
            Err(e) => api_error(&state.plane, e),
        },
        other => api_error(
            &state.plane,
            ApiError::BadRequest(format!("unknown host display format: {other}")),
        ),
    }
}

pub(crate) async fn host_window(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if let Err(e) = check_auth(&state, &headers) {
        return api_error(&state.plane, e);
    }
    match state.plane.host_window_png() {
        Ok(bytes) => {
            let mut resp = Response::new(Body::from(bytes));
            let h = resp.headers_mut();
            h.insert(header::CONTENT_TYPE, HeaderValue::from_static("image/png"));
            h.insert(
                "x-specchum-source",
                HeaderValue::from_static("os-window-own-id"),
            );
            resp
        }
        Err(e) => api_error(&state.plane, e),
    }
}

pub(crate) fn insert_present_headers(h: &mut header::HeaderMap, meta: &PresentMeta) {
    if let Ok(v) = HeaderValue::from_str(&meta.width.to_string()) {
        h.insert("x-specchum-width", v);
    }
    if let Ok(v) = HeaderValue::from_str(&meta.height.to_string()) {
        h.insert("x-specchum-height", v);
    }
    if let Ok(v) = HeaderValue::from_str(&meta.source_width.to_string()) {
        h.insert("x-specchum-source-width", v);
    }
    if let Ok(v) = HeaderValue::from_str(&meta.source_height.to_string()) {
        h.insert("x-specchum-source-height", v);
    }
    h.insert("x-specchum-filter", HeaderValue::from_static(meta.filter));
    let panel = match meta.panel {
        control_plane::PresentPanelSource::Explicit => "explicit",
        control_plane::PresentPanelSource::Live => "live",
        control_plane::PresentPanelSource::Scale => "scale",
        control_plane::PresentPanelSource::Default => "default",
    };
    h.insert("x-specchum-panel", HeaderValue::from_static(panel));
}

pub(crate) fn insert_meta_headers(headers: &mut HeaderMap, meta: &FramebufferMeta) {
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
