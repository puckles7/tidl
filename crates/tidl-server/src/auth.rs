use axum::{
    extract::FromRequestParts,
    http::{header, request::Parts, StatusCode},
    response::{IntoResponse, Response},
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

use crate::{user_store::UserStore, AppState};

/// Credentials extracted from an `Authorization: Basic …` header and
/// validated against the users database.
///
/// `username` is always the canonical **slug** (not necessarily what was sent
/// in the header — clients may send either the slug or the email address).
///
/// `token_read_only` is `true` when the request authenticated via a read-only
/// app token.  Handlers that perform writes should check this flag (or rely on
/// `require_calendar_writable` which enforces it).
#[derive(Clone)]
pub struct BasicAuth {
    pub username: String,
    #[allow(dead_code)]
    // stored from the Basic auth header; auth is done by token lookup, not direct comparison
    pub password: String,
    /// `true` when authenticated via a read-only app token.
    pub token_read_only: bool,
}

impl BasicAuth {
    fn decode_header(auth_header: &str) -> Option<(String, String)> {
        let encoded = auth_header.strip_prefix("Basic ")?;
        let decoded = BASE64.decode(encoded.as_bytes()).ok()?;
        let s = String::from_utf8(decoded).ok()?;
        let mut parts = s.splitn(2, ':');
        Some((parts.next()?.to_string(), parts.next()?.to_string()))
    }
}

// axum-core 0.5 supports async extractors natively — no async_trait needed.
impl FromRequestParts<AppState> for BasicAuth {
    type Rejection = Response;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &AppState,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get(header::AUTHORIZATION)
            .and_then(|h| h.to_str().ok())
            .ok_or_else(create_challenge_response)?;

        let (credential, password) =
            BasicAuth::decode_header(auth_header).ok_or_else(create_challenge_response)?;

        // Accept either the username slug or the email address.
        // find_active_by_username is tried first because CalDAV clients are
        // typically configured with the slug that appears in their CalDAV URL.
        let user = state
            .users
            .find_active_by_username(&credential)
            .await
            .ok()
            .flatten()
            .or({
                // Synchronous placeholder — the async find_active_by_email
                // call happens below via a secondary await if the slug lookup
                // returned nothing.
                None
            });

        // If slug lookup missed, try email (handles clients configured with
        // an email address as the CalDAV username).
        let user = match user {
            Some(u) => Some(u),
            None => state
                .users
                .find_active_by_email(&credential)
                .await
                .ok()
                .flatten(),
        };

        let user = user.ok_or_else(create_challenge_response)?;

        // First try the user's real password.
        if UserStore::verify_password(&password, &user.password_hash) {
            return Ok(BasicAuth {
                username: user.username,
                password,
                token_read_only: false,
            });
        }

        // Fall back to app token validation (password field contains the raw token).
        if password.starts_with("tidl_") {
            if let Ok(Some((token_username, read_only))) =
                state.users.validate_app_token(&password).await
            {
                // The token must belong to the same user as the credential sent in
                // the Basic Auth header.
                if token_username == user.username {
                    return Ok(BasicAuth {
                        username: user.username,
                        password,
                        token_read_only: read_only,
                    });
                }
            }
        }

        Err(create_challenge_response())
    }
}

pub fn create_challenge_response() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        [(
            header::WWW_AUTHENTICATE,
            "Basic realm=\"tidl CalDAV Server\"",
        )],
        "Unauthorized",
    )
        .into_response()
}
