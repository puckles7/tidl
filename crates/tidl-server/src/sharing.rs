//! HTTP endpoints for share invitation accept/decline links.
//!
//! These are plain GET requests sent from invitation emails:
//!   GET /share/accept/{token}
//!   GET /share/decline/{token}
//!
//! If the user is logged in (valid session cookie) the action is completed
//! immediately.  Otherwise they are redirected to the login page with a
//! `?next=` query param so they can authenticate and return.

use axum::{
    extract::{Path, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use percent_encoding::{utf8_percent_encode, NON_ALPHANUMERIC};
use tidl_caldav::change::ChangeKind;

use tidl_web::ssr_types::SessionCookieName;

use crate::{session::session_token_from_headers, AppState};

pub async fn accept_handler(
    State(state): State<AppState>,
    headers: HeaderMap,
    Path(token): Path<String>,
) -> Response {
    let username = match authenticated_user(&state, &headers).await {
        Some(u) => u,
        None => {
            let next = utf8_percent_encode(&format!("/share/accept/{token}"), NON_ALPHANUMERIC)
                .to_string();
            return Redirect::to(&format!("/?login_next={next}")).into_response();
        }
    };

    let email = match state.users.find_active_by_username(&username).await {
        Ok(Some(u)) => u.email,
        Ok(None) => {
            tracing::warn!(
                username,
                "accept_handler: authenticated user has no active account"
            );
            return (
                StatusCode::BAD_REQUEST,
                "Could not accept invitation: account not found",
            )
                .into_response();
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to look up user email for share acceptance");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                "Could not accept invitation",
            )
                .into_response();
        }
    };

    match state
        .storage
        .accept_share_invitation(&token, &username, &email)
        .await
    {
        Ok(owner_username) => {
            tracing::info!(username, token, "share invitation accepted");
            // Notify both sides over SSE so an already-open tab picks up the
            // new calendar without a manual refresh — the tab that just
            // accepted gets a full page reload via the redirect below anyway,
            // but any *other* open tab of either account needs this push.
            let _ = state.change_tx.send(ChangeKind::All.to_msg(&username));
            let _ = state
                .change_tx
                .send(ChangeKind::All.to_msg(&owner_username));
            Redirect::to("/?shared=accepted").into_response()
        }
        Err(e) => {
            tracing::warn!(error = %e, "failed to accept share invitation");
            (
                StatusCode::BAD_REQUEST,
                format!("Could not accept invitation: {e}"),
            )
                .into_response()
        }
    }
}

pub async fn decline_handler(
    State(state): State<AppState>,
    _headers: HeaderMap,
    Path(token): Path<String>,
) -> Response {
    match state.storage.decline_share_invitation(&token).await {
        Ok(()) => Redirect::to("/?shared=declined").into_response(),
        Err(e) => (
            StatusCode::BAD_REQUEST,
            format!("Could not decline invitation: {e}"),
        )
            .into_response(),
    }
}

async fn authenticated_user(state: &AppState, headers: &HeaderMap) -> Option<String> {
    let cookie_name =
        SessionCookieName::new(state.config.server.port, state.config.server.secure_cookie);
    let token = session_token_from_headers(headers, &cookie_name)?;
    state.sessions.get_session_user(&token).await.ok().flatten()
}
