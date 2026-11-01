//! GET /api/contacts/{id}/photo — serve a contact's photo bytes (#86).
//!
//! A plain Axum route rather than a `#[server]` fn: embedding photo bytes in
//! `ContactDto`'s JSON would bloat every grid load, so the web UI instead
//! points an `<img src>` straight at this route and lets normal HTTP
//! caching do its job. Session-cookie auth mirrors `export.rs` exactly —
//! this is the same "authenticated non-CalDAV/CardDAV GET route" shape.

use axum::{
    extract::{Path, State},
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
};

use tidl_web::ssr_types::SessionCookieName;

use crate::{session::session_token_from_headers, AppState};

pub async fn get_contact_photo(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(id): Path<i64>,
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

    let photo = match state.contacts.get_contact_photo_by_id(&username, id).await {
        Ok(Some(p)) => p,
        Ok(None) => return (StatusCode::NOT_FOUND, "No photo").into_response(),
        Err(e) => {
            tracing::error!(error = %e, "failed to fetch contact photo");
            return (StatusCode::INTERNAL_SERVER_ERROR, "Internal error").into_response();
        }
    };
    let (bytes, content_type) = photo;

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
