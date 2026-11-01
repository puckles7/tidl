//! OIDC Authorization Code + PKCE flow handlers.

use axum::{
    extract::{Query, State},
    http::{header, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use openidconnect::{
    core::{CoreAuthenticationFlow, CoreTokenResponse},
    AuthorizationCode, CsrfToken, Nonce, PkceCodeChallenge, PkceCodeVerifier, Scope, TokenResponse,
};
use serde::Deserialize;
use sqlx::Row;
use uuid::Uuid;

use tidl_web::ssr_types::SessionCookieName;

use crate::{session_store::SessionStore, AppState};

const SESSION_TTL_SECS: i64 = 86_400; // 24 hours

// ── /auth/oidc/start ─────────────────────────────────────────────────────────

/// Initiates the OIDC Authorization Code + PKCE flow.
/// Sweeps expired pending rows, then redirects the user to the IdP.
pub async fn start_handler(State(state): State<AppState>) -> Response {
    let Some(oidc) = &state.oidc else {
        return (StatusCode::NOT_FOUND, "OIDC not configured").into_response();
    };

    // Sweep expired pending rows before inserting a new one.
    let _ = sqlx::query("DELETE FROM oidc_pending WHERE expires_at <= CURRENT_TIMESTAMP")
        .execute(state.users.pool())
        .await;

    let (pkce_challenge, pkce_verifier) = PkceCodeChallenge::new_random_sha256();
    let (auth_url, csrf_token, nonce) = oidc
        .client
        .authorize_url(
            CoreAuthenticationFlow::AuthorizationCode,
            CsrfToken::new_random,
            Nonce::new_random,
        )
        .add_scope(Scope::new("openid".to_string()))
        .add_scope(Scope::new("email".to_string()))
        .add_scope(Scope::new("profile".to_string()))
        .set_pkce_challenge(pkce_challenge)
        .url();

    let result = sqlx::query(
        "INSERT INTO oidc_pending (state, pkce_secret, nonce_secret, expires_at)
         VALUES (?, ?, ?, datetime('now', '+10 minutes'))",
    )
    .bind(csrf_token.secret())
    .bind(pkce_verifier.secret())
    .bind(nonce.secret())
    .execute(state.users.pool())
    .await;

    if let Err(e) = result {
        tracing::error!("Failed to store OIDC pending state: {}", e);
        return Redirect::to("/login?error=oidc_failed").into_response();
    }

    Redirect::to(auth_url.as_str()).into_response()
}

// ── /auth/oidc/callback ───────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct CallbackParams {
    code: Option<String>,
    state: Option<String>,
    error: Option<String>,
}

/// Handles the IdP redirect. Validates state, exchanges code, and either:
/// 1. Creates a session for an existing OIDC-linked user.
/// 2. Redirects to the linking page if email matches a local account.
/// 3. Auto-provisions a new account and creates a session.
pub async fn callback_handler(
    State(state): State<AppState>,
    Query(params): Query<CallbackParams>,
) -> Response {
    let Some(oidc) = &state.oidc else {
        return (StatusCode::NOT_FOUND, "OIDC not configured").into_response();
    };

    if let Some(err) = params.error {
        tracing::warn!("OIDC error from IdP: {}", err);
        return Redirect::to("/login?error=oidc_failed").into_response();
    }

    let (code, csrf_state) = match (params.code, params.state) {
        (Some(c), Some(s)) => (c, s),
        _ => return Redirect::to("/login?error=oidc_failed").into_response(),
    };

    // Look up and validate the pending state.
    let row = sqlx::query(
        "SELECT pkce_secret, nonce_secret FROM oidc_pending
         WHERE state = ? AND expires_at > CURRENT_TIMESTAMP",
    )
    .bind(&csrf_state)
    .fetch_optional(state.users.pool())
    .await;

    let row = match row {
        Ok(Some(r)) => r,
        Ok(None) => {
            tracing::warn!(
                "OIDC callback: state '{}' not found or expired",
                &csrf_state
            );
            return Redirect::to("/login?error=oidc_expired").into_response();
        }
        Err(e) => {
            tracing::error!("OIDC DB error: {}", e);
            return Redirect::to("/login?error=oidc_failed").into_response();
        }
    };

    let pkce_secret: String = row.get("pkce_secret");
    let nonce_secret: String = row.get("nonce_secret");

    // Clean up the pending row now — all subsequent paths (success or error) should leave no orphan.
    let _ = sqlx::query("DELETE FROM oidc_pending WHERE state = ?")
        .bind(&csrf_state)
        .execute(state.users.pool())
        .await;

    let pkce_verifier = PkceCodeVerifier::new(pkce_secret);
    let nonce = Nonce::new(nonce_secret);

    // Exchange the authorization code for tokens.
    let token_response: CoreTokenResponse =
        match oidc.client.exchange_code(AuthorizationCode::new(code)) {
            Err(e) => {
                tracing::warn!("OIDC token exchange failed: {}", e);
                return Redirect::to("/login?error=oidc_failed").into_response();
            }
            Ok(req) => match req
                .set_pkce_verifier(pkce_verifier)
                .request_async(&oidc.http_client)
                .await
            {
                Ok(t) => t,
                Err(e) => {
                    tracing::warn!("OIDC token exchange failed: {}", e);
                    return Redirect::to("/login?error=oidc_failed").into_response();
                }
            },
        };

    // Validate and extract ID token claims.
    let id_token = match token_response.id_token() {
        Some(t) => t,
        None => {
            tracing::warn!("OIDC callback: no ID token in response");
            return Redirect::to("/login?error=oidc_failed").into_response();
        }
    };

    let claims = match id_token.claims(&oidc.client.id_token_verifier(), &nonce) {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("OIDC ID token validation failed: {}", e);
            return Redirect::to("/login?error=oidc_failed").into_response();
        }
    };

    let sub = claims.subject().to_string();
    let issuer = claims.issuer().to_string();
    let email = claims.email().map(|e| e.to_string());
    let name = claims
        .name()
        .and_then(|n| n.get(None))
        .map(|n| n.as_str().to_string());

    let email = match email {
        Some(e) => e,
        None => {
            tracing::warn!("OIDC callback: no email claim for sub={}", sub);
            return Redirect::to("/login?error=oidc_no_email").into_response();
        }
    };

    let cookie_name =
        SessionCookieName::new(state.config.server.port, state.config.server.secure_cookie);

    // Case 1: existing OIDC-linked user.
    if let Ok(Some(user)) = state.users.find_by_oidc_subject(&sub, &issuer).await {
        return create_session_and_redirect(&state.sessions, &user.username, &cookie_name).await;
    }

    // Case 2: email matches an existing local account → link flow.
    if let Ok(Some(_local)) = state.users.find_active_by_email(&email).await {
        // Re-insert a pending row carrying the OIDC identity for the link handler.
        let link_state = Uuid::new_v4().to_string();
        let insert = sqlx::query(
            "INSERT INTO oidc_pending
               (state, pkce_secret, nonce_secret, pending_sub, pending_issuer,
                pending_email, pending_name, expires_at)
             VALUES (?, '', '', ?, ?, ?, ?, datetime('now', '+10 minutes'))",
        )
        .bind(&link_state)
        .bind(&sub)
        .bind(&issuer)
        .bind(&email)
        .bind(name.as_deref())
        .execute(state.users.pool())
        .await;

        if insert.is_err() {
            return Redirect::to("/login?error=oidc_failed").into_response();
        }
        return Redirect::to(&format!("/auth/oidc/link?state={}", link_state)).into_response();
    }

    // Case 3: new user — auto-provision.
    let email_local = email.split('@').next().unwrap_or("user");
    let username = match state
        .users
        .generate_unique_username(&sanitize_username_base(email_local))
        .await
    {
        Ok(u) => u,
        Err(e) => {
            tracing::error!("Failed to generate username: {}", e);
            return Redirect::to("/login?error=oidc_failed").into_response();
        }
    };
    let display_name = name.unwrap_or_else(|| email_local.to_string());

    match state
        .users
        .create_oidc_user(&username, &email, &display_name, &sub, &issuer)
        .await
    {
        Ok(user) => {
            create_session_and_redirect(&state.sessions, &user.username, &cookie_name).await
        }
        Err(e) => {
            tracing::error!("Failed to create OIDC user: {}", e);
            Redirect::to("/login?error=oidc_failed").into_response()
        }
    }
}

/// Converts an email local-part to a valid username base.
/// Keeps only a-z, 0-9, hyphens; collapses non-alphanumeric runs to a single hyphen;
/// strips leading/trailing hyphens; truncates to 50 chars; falls back to "user"
/// when the result is shorter than 2 characters.
fn sanitize_username_base(raw: &str) -> String {
    let lowered = raw.to_lowercase();
    let mut slug = String::new();
    let mut last_was_sep = true; // start true so leading separators are skipped
    for c in lowered.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
            last_was_sep = false;
        } else if !last_was_sep {
            slug.push('-');
            last_was_sep = true;
        }
    }
    // Strip trailing hyphen left by a separator at the end.
    let slug = slug.trim_end_matches('-');
    // Truncate to 50 characters (all chars are ASCII so byte == char boundary).
    let slug = &slug[..slug.len().min(50)];
    // A username shorter than 2 chars is unusable — fall back to the generic base.
    if slug.len() < 2 {
        "user".to_string()
    } else {
        slug.to_string()
    }
}

async fn create_session_and_redirect(
    sessions: &SessionStore,
    username: &str,
    cookie_name: &SessionCookieName,
) -> Response {
    match sessions.create_session(username, SESSION_TTL_SECS).await {
        Ok(token) => {
            let cookie = cookie_name.set_header_value(&token, SESSION_TTL_SECS);
            (
                StatusCode::SEE_OTHER,
                [
                    (header::LOCATION, "/".to_string()),
                    (header::SET_COOKIE, cookie),
                ],
                "",
            )
                .into_response()
        }
        Err(e) => {
            tracing::error!("Failed to create session: {}", e);
            Redirect::to("/login?error=oidc_failed").into_response()
        }
    }
}

// ── POST /auth/oidc/link ──────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct LinkForm {
    state: String,
    password: String,
}

/// Verifies the local password and links the pending OIDC identity to the
/// existing account.  On success sets the session cookie and redirects to /.
pub async fn link_handler(
    State(state): State<AppState>,
    axum::Form(form): axum::Form<LinkForm>,
) -> Response {
    // Load the pending OIDC identity from the database.
    let row = sqlx::query(
        "SELECT pending_sub, pending_issuer, pending_email
         FROM oidc_pending
         WHERE state = ?
           AND expires_at > CURRENT_TIMESTAMP
           AND pending_sub IS NOT NULL",
    )
    .bind(&form.state)
    .fetch_optional(state.users.pool())
    .await;

    let row = match row {
        Ok(Some(r)) => r,
        Ok(None) => {
            tracing::warn!(
                "OIDC link: state '{}' not found, expired, or not a link flow",
                &form.state
            );
            return Redirect::to("/login?error=oidc_expired").into_response();
        }
        Err(e) => {
            tracing::error!("OIDC link DB error: {}", e);
            return Redirect::to("/login?error=oidc_failed").into_response();
        }
    };

    let pending_sub: String = row.get("pending_sub");
    let pending_issuer: String = row.get("pending_issuer");
    let pending_email: String = row.get("pending_email");

    // Find the existing local account by email.
    let user = match state.users.find_active_by_email(&pending_email).await {
        Ok(Some(u)) => u,
        _ => {
            tracing::warn!("OIDC link: no active user for email {}", pending_email);
            let _ = sqlx::query("DELETE FROM oidc_pending WHERE state = ?")
                .bind(&form.state)
                .execute(state.users.pool())
                .await;
            return Redirect::to("/login?error=oidc_failed").into_response();
        }
    };

    // Verify the local password.
    if !crate::user_store::UserStore::verify_password(&form.password, &user.password_hash) {
        // Wrong password — delete the pending row so the state UUID cannot be used to brute-force
        // the local password within the 10-minute window.  The user must restart the OIDC flow.
        let _ = sqlx::query("DELETE FROM oidc_pending WHERE state = ?")
            .bind(&form.state)
            .execute(state.users.pool())
            .await;
        return Redirect::to("/login?error=oidc_link_failed").into_response();
    }

    // Link the OIDC identity to the existing account.
    if let Err(e) = state
        .users
        .set_oidc_identity(&user.username, &pending_sub, &pending_issuer)
        .await
    {
        tracing::error!("OIDC link: failed to set identity: {}", e);
        return Redirect::to("/login?error=oidc_failed").into_response();
    }

    // Delete the pending row.
    let _ = sqlx::query("DELETE FROM oidc_pending WHERE state = ?")
        .bind(&form.state)
        .execute(state.users.pool())
        .await;

    // Create a session.
    let cookie_name =
        SessionCookieName::new(state.config.server.port, state.config.server.secure_cookie);
    create_session_and_redirect(&state.sessions, &user.username, &cookie_name).await
}

#[cfg(test)]
mod tests {
    use super::sanitize_username_base;

    #[test]
    fn normal_local_part() {
        assert_eq!(sanitize_username_base("alice"), "alice");
    }

    #[test]
    fn uppercase_and_dots() {
        assert_eq!(sanitize_username_base("Hello.World"), "hello-world");
    }

    #[test]
    fn plus_tagged() {
        assert_eq!(sanitize_username_base("user+tag"), "user-tag");
    }

    #[test]
    fn empty_string_falls_back() {
        assert_eq!(sanitize_username_base(""), "user");
    }

    #[test]
    fn single_char_falls_back() {
        assert_eq!(sanitize_username_base("a"), "user");
    }

    #[test]
    fn long_string_truncated_to_50() {
        let long = "a".repeat(80);
        let result = sanitize_username_base(&long);
        assert_eq!(result.len(), 50);
    }

    #[test]
    fn leading_separator_stripped() {
        // An address like "@example.com" would give "" as local-part, but test the
        // sanitiser directly with a leading dot/plus.
        assert_eq!(sanitize_username_base(".leading"), "leading");
    }

    #[test]
    fn trailing_separator_stripped() {
        assert_eq!(sanitize_username_base("trailing."), "trailing");
    }
}
