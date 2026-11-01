//! GET /api/notes/{note_id}/images/{image_id} — serve an inline note image
//! (backlog #110).
//!
//! Same shape as `contact_photo.rs` (#86): a plain Axum route rather than a
//! `#[server]` fn, because the note card points an `<img src>` straight at it
//! and leans on normal HTTP caching — embedding image bytes in the notes grid
//! JSON would bloat every load. Session-cookie auth mirrors `export.rs`.

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

use tidl_web::ssr_types::SessionCookieName;

use crate::{session::session_token_from_headers, AppState};

pub async fn get_note_image(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path((note_id, image_id)): Path<(i64, i64)>,
) -> Response {
    let cookie_name =
        SessionCookieName::new(state.config.server.port, state.config.server.secure_cookie);
    let token = match session_token_from_headers(&headers, &cookie_name) {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, "Not authenticated").into_response(),
    };
    let username = match state.sessions.get_session_user(&token).await {
        Ok(Some(u)) => u,
        _ => return (StatusCode::UNAUTHORIZED, "Session invalid or expired").into_response(),
    };

    let image = match state
        .notes
        .get_note_image(&username, note_id, image_id)
        .await
    {
        Ok(Some(p)) => p,
        Ok(None) => return (StatusCode::NOT_FOUND, "No image").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch note image");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response();
        }
    };
    let (bytes, content_type) = image;

    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, content_type),
            (header::CACHE_CONTROL, "private, max-age=86400".to_string()),
        ],
        bytes,
    )
        .into_response()
}
