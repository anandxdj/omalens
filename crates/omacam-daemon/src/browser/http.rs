//! HTTPS routes, role authentication, JSON validation, and bounded polling.

use std::sync::atomic::Ordering;
use std::time::Instant;

use axum::body::Bytes;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use serde::Deserialize;
use serde_json::{Value, json};

use super::session::{
    AppState, BrowserOffer, CameraCommand, CommandError, HeartbeatError, PhoneHeartbeat, Role,
    authorized, binding_for_token,
};
use super::{APP_CSS, DESKTOP_JS, PAGE, PHONE_JS, SHARED_CONTROL_JS};

const MAX_HTTP_BODY_BYTES: usize = 64 * 1024;
const MAX_PREVIEW_BYTES: usize = 256 * 1024;

pub(super) fn router(state: AppState) -> Router {
    Router::new()
        .route("/", get(index))
        .route("/desktop", get(index))
        .route("/assets/app.css", get(app_css))
        .route("/assets/shared-control.js", get(shared_control_js))
        .route("/assets/phone.js", get(phone_js))
        .route("/assets/desktop.js", get(desktop_js))
        .route("/api/offer", post(offer))
        .route("/api/state", get(camera_state))
        .route("/api/command", post(camera_command))
        .route("/api/heartbeat", post(heartbeat))
        .route("/api/stop", post(stop))
        .route("/api/preview", get(preview))
        .layer(DefaultBodyLimit::max(MAX_HTTP_BODY_BYTES))
        .with_state(state)
}

async fn index() -> Html<&'static str> {
    Html(PAGE)
}

macro_rules! asset_handler {
    ($name:ident, $value:ident, $content_type:literal) => {
        async fn $name() -> Response {
            (
                [
                    (header::CONTENT_TYPE, $content_type),
                    (header::CACHE_CONTROL, "no-store"),
                    (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
                ],
                $value,
            )
                .into_response()
        }
    };
}

asset_handler!(app_css, APP_CSS, "text/css; charset=utf-8");
asset_handler!(
    shared_control_js,
    SHARED_CONTROL_JS,
    "text/javascript; charset=utf-8"
);
asset_handler!(phone_js, PHONE_JS, "text/javascript; charset=utf-8");
asset_handler!(desktop_js, DESKTOP_JS, "text/javascript; charset=utf-8");

async fn camera_state(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !authorized(&headers, &state, Role::Desktop) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !state.allow_poll(false, Instant::now()) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let mut session = state.session.lock().unwrap();
    session.expire_pending_if_needed(Instant::now());
    Json(session.refresh_snapshot()).into_response()
}

async fn camera_command(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(command): Json<CameraCommand>,
) -> Response {
    if !authorized(&headers, &state, Role::Desktop) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut session = state.session.lock().unwrap();
    session.expire_pending_if_needed(Instant::now());
    match session.enqueue_command(command, Instant::now()) {
        Ok(()) => Json(session.refresh_snapshot()).into_response(),
        Err(CommandError::NotLive) => (StatusCode::CONFLICT, "session is not live").into_response(),
        Err(CommandError::StaleRevision) => (
            StatusCode::CONFLICT,
            "session or revision changed; refresh state",
        )
            .into_response(),
        Err(CommandError::Pending) => {
            (StatusCode::CONFLICT, "a camera command is already pending").into_response()
        }
        Err(CommandError::InvalidId) => {
            (StatusCode::BAD_REQUEST, "invalid commandId").into_response()
        }
        Err(CommandError::ReusedId) => {
            (StatusCode::CONFLICT, "commandId was already used").into_response()
        }
        Err(CommandError::InvalidControls(error)) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({ "error": error })),
        )
            .into_response(),
    }
}

async fn heartbeat(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(update): Json<PhoneHeartbeat>,
) -> Response {
    if !authorized(&headers, &state, Role::Phone) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut session = state.session.lock().unwrap();
    match session.accept_heartbeat(update, Instant::now()) {
        Ok(()) => Json(session.refresh_snapshot()).into_response(),
        Err(
            HeartbeatError::WrongSession
            | HeartbeatError::StaleAcknowledgement
            | HeartbeatError::NotLive,
        ) => (
            StatusCode::CONFLICT,
            "session or command acknowledgement is stale",
        )
            .into_response(),
        Err(HeartbeatError::MissingAppliedState) => (
            StatusCode::UNPROCESSABLE_ENTITY,
            "a command acknowledgement must include actual applied state",
        )
            .into_response(),
        Err(
            HeartbeatError::InvalidCapabilities(error) | HeartbeatError::InvalidAppliedState(error),
        ) => (StatusCode::UNPROCESSABLE_ENTITY, error).into_response(),
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct StopRequest {
    session_id: String,
}

async fn stop(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(request): Json<StopRequest>,
) -> Response {
    if !authorized(&headers, &state, Role::Desktop) && !authorized(&headers, &state, Role::Phone) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    let mut session = state.session.lock().unwrap();
    if request.session_id != session.session_id {
        return StatusCode::CONFLICT.into_response();
    }
    session.stop("Camera stopped");
    Json(session.refresh_snapshot()).into_response()
}

async fn preview(State(state): State<AppState>, headers: HeaderMap) -> Response {
    if !authorized(&headers, &state, Role::Desktop) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    if !state.allow_poll(true, Instant::now()) {
        return StatusCode::TOO_MANY_REQUESTS.into_response();
    }
    let session = state.session.lock().unwrap();
    if session.status == "live"
        && let Some(frame) = &session.preview
        && frame.captured_at.elapsed().as_millis() < 500
        && frame.jpeg.len() <= MAX_PREVIEW_BYTES
        && frame.jpeg.starts_with(&[0xff, 0xd8])
        && frame.jpeg.ends_with(&[0xff, 0xd9])
    {
        return (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "image/jpeg"),
                (header::CACHE_CONTROL, "no-store"),
                (header::X_CONTENT_TYPE_OPTIONS, "nosniff"),
            ],
            frame.jpeg.clone(),
        )
            .into_response();
    }
    StatusCode::NO_CONTENT.into_response()
}

async fn offer(State(state): State<AppState>, headers: HeaderMap, body: Bytes) -> Response {
    if !authorized(&headers, &state, Role::Phone) {
        return (StatusCode::UNAUTHORIZED, "invalid or expired QR token").into_response();
    }
    let Ok(request) = serde_json::from_slice::<BrowserOffer>(&body) else {
        return (StatusCode::BAD_REQUEST, "invalid SDP offer JSON").into_response();
    };
    let Ok(format) = request.format.validate() else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            "unsupported camera format",
        )
            .into_response();
    };
    let (Some(capabilities), Some(applied)) = (request.capabilities, request.applied) else {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            "capabilities and actual applied camera state are required",
        )
            .into_response();
    };
    if let Err(error) = capabilities.validate() {
        return (StatusCode::UNPROCESSABLE_ENTITY, error).into_response();
    }
    if let Err(error) = applied.validate(&capabilities) {
        return (StatusCode::UNPROCESSABLE_ENTITY, error).into_response();
    }
    if !format.matches(&applied) {
        return (
            StatusCode::UNPROCESSABLE_ENTITY,
            "offer format must match the actual camera settings",
        )
            .into_response();
    }
    if state
        .claimed
        .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
        .is_err()
    {
        return (StatusCode::CONFLICT, "this QR code was already used").into_response();
    }
    let token = match URL_SAFE_NO_PAD.decode(&state.phone_token) {
        Ok(token) => token,
        Err(error) => {
            state
                .session
                .lock()
                .unwrap()
                .stop("Phone token could not be decoded");
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    let binding = match binding_for_token(&token) {
        Ok(binding) => binding,
        Err(error) => {
            state
                .session
                .lock()
                .unwrap()
                .stop("Media identity could not be created");
            return (StatusCode::INTERNAL_SERVER_ERROR, error.to_string()).into_response();
        }
    };
    {
        let mut session = state.session.lock().unwrap();
        if session.status != "waiting" {
            return (StatusCode::CONFLICT, "session is no longer waiting").into_response();
        }
        session.activate(capabilities, applied, format, Instant::now());
    }
    match super::rtc::run_rtc(
        request.offer,
        state.media_endpoint,
        state.output_device.clone(),
        binding,
        format,
        state.session.clone(),
    ) {
        Ok(mut answer) => {
            let mut session = state.session.lock().unwrap();
            if let Some(answer) = answer.as_object_mut() {
                answer.insert(
                    "sessionId".into(),
                    Value::String(session.session_id.clone()),
                );
                answer.insert("revision".into(), Value::from(session.revision));
                Json(answer.clone()).into_response()
            } else {
                session.stop("WebRTC answer was malformed");
                StatusCode::INTERNAL_SERVER_ERROR.into_response()
            }
        }
        Err(error) => {
            state.session.lock().unwrap().stop(&error.to_string());
            (StatusCode::BAD_REQUEST, error.to_string()).into_response()
        }
    }
}
