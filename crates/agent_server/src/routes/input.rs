//! Keyboard / joystick / mouse / prefs routes.

use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use control_plane::{ApiError, PrefsPatch};
use serde::Deserialize;

use super::{api_error, auth_empty, auth_json, check_auth, AppState};

#[derive(Debug, Deserialize)]
pub(crate) struct KeyAction {
    row: usize,
    bit: u8,
    #[serde(default = "default_pressed")]
    pressed: bool,
}

pub(crate) fn default_pressed() -> bool {
    true
}

#[derive(Debug, Deserialize)]
pub(crate) struct KeysBody {
    #[serde(default)]
    clear: bool,
    #[serde(default)]
    row: Option<usize>,
    #[serde(default)]
    bit: Option<u8>,
    #[serde(default)]
    pressed: Option<bool>,
    #[serde(default)]
    keys: Vec<KeyAction>,
}

#[derive(Debug, Deserialize)]
pub(crate) struct JoystickBody {
    #[serde(default)]
    clear: bool,
    #[serde(default)]
    mask: Option<u8>,
}

pub(crate) async fn set_joystick(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<JoystickBody>,
) -> Response {
    if body.clear {
        return auth_empty(&state, &headers, || state.plane.clear_joystick());
    }
    let Some(mask) = body.mask else {
        return api_error(
            &state.plane,
            ApiError::BadRequest("joystick body requires clear or mask".into()),
        );
    };
    auth_empty(&state, &headers, || state.plane.set_joystick(mask))
}

#[derive(Debug, Deserialize)]
pub(crate) struct MouseBody {
    #[serde(default)]
    clear: bool,
    #[serde(default)]
    dx: Option<i8>,
    #[serde(default)]
    dy: Option<i8>,
    #[serde(default)]
    left: Option<bool>,
    #[serde(default)]
    right: Option<bool>,
    #[serde(default)]
    middle: Option<bool>,
}

pub(crate) async fn set_mouse(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<MouseBody>,
) -> Response {
    if body.clear {
        return auth_empty(&state, &headers, || state.plane.clear_mouse());
    }
    let has_delta = body.dx.is_some() || body.dy.is_some();
    let has_buttons = body.left.is_some() || body.right.is_some() || body.middle.is_some();
    if !has_delta && !has_buttons {
        return api_error(
            &state.plane,
            ApiError::BadRequest(
                "mouse body requires clear, dx/dy, and/or left/right/middle".into(),
            ),
        );
    }
    auth_empty(&state, &headers, || {
        state
            .plane
            .set_mouse(body.dx, body.dy, body.left, body.right, body.middle)
    })
}

pub(crate) async fn get_prefs(State(state): State<AppState>, headers: HeaderMap) -> Response {
    auth_json(&state, &headers, || state.plane.prefs())
}

pub(crate) async fn patch_prefs(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<PrefsPatch>,
) -> Response {
    auth_json(&state, &headers, || state.plane.patch_prefs(body))
}

pub(crate) async fn set_keys(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(body): Json<KeysBody>,
) -> Response {
    if body.clear {
        return auth_empty(&state, &headers, || state.plane.clear_keys());
    }
    if !body.keys.is_empty() {
        for key in &body.keys {
            if key.row > 7 || key.bit > 4 {
                return api_error(
                    &state.plane,
                    ApiError::BadRequest("key row/bit out of range".into()),
                );
            }
        }
        match check_auth(&state, &headers) {
            Ok(()) => {
                for key in body.keys {
                    if let Err(e) = state.plane.set_key(key.row, key.bit, key.pressed) {
                        return api_error(&state.plane, e);
                    }
                }
                StatusCode::NO_CONTENT.into_response()
            }
            Err(e) => api_error(&state.plane, e),
        }
    } else {
        let (Some(row), Some(bit)) = (body.row, body.bit) else {
            return api_error(
                &state.plane,
                ApiError::BadRequest("keys body requires clear, keys[], or row+bit".into()),
            );
        };
        let pressed = body.pressed.unwrap_or(true);
        auth_empty(&state, &headers, || state.plane.set_key(row, bit, pressed))
    }
}
