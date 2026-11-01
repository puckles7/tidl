//! Authentication, session management, registration, and password reset.

use leptos::prelude::*;

#[cfg(feature = "ssr")]
use super::{create_and_set_session, current_user};
#[cfg(feature = "ssr")]
use crate::ssr_types::{SmtpConfigRow, SMTP_CONFIG_QUERY};

use super::SessionInfo;

/// Return the username for the current session, or `None` if not logged in.
#[server]
pub async fn get_session() -> Result<Option<String>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use axum::http::HeaderMap;
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::SqlitePool;

        let Extension(pool): Extension<SqlitePool> = extract().await?;
        let headers: HeaderMap = extract().await?;
        return Ok(current_user(&pool, &headers).await.ok());
    }
    #[allow(unreachable_code)]
    Ok(None)
}

/// Return username and admin flag for the current session, or `None` if not logged in.
#[server]
pub async fn get_session_info() -> Result<Option<SessionInfo>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use axum::http::HeaderMap;
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::{Row, SqlitePool};

        let Extension(pool): Extension<SqlitePool> = extract().await?;
        let headers: HeaderMap = extract().await?;

        let username = match current_user(&pool, &headers).await {
            Ok(u) => u,
            Err(_) => return Ok(None),
        };

        let row = sqlx::query("SELECT is_admin FROM users WHERE username = ? AND is_active = 1")
            .bind(&username)
            .fetch_optional(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let is_admin: i64 = row
            .and_then(|r| r.try_get::<i64, _>("is_admin").ok())
            .unwrap_or(0);

        return Ok(Some(SessionInfo {
            username,
            is_admin: is_admin != 0,
        }));
    }
    #[allow(unreachable_code)]
    Ok(None)
}

/// Verify credentials and establish a session, setting the `session` cookie.
///
/// `email` is the login identifier.  On success a `session` cookie is set and
/// the caller should trigger a page refresh to reflect the new session state.
#[server(input = server_fn::codec::Json)]
#[allow(unused_variables)]
pub async fn do_login(email: String, password: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use axum::http::StatusCode;
        use axum::Extension;
        use leptos::prelude::expect_context;
        use leptos_axum::{extract, ResponseOptions};
        use sqlx::{Row, SqlitePool};

        let Extension(pool): Extension<SqlitePool> = extract().await?;
        let opts = expect_context::<ResponseOptions>();

        // Look up the user by email (stored lowercase).
        let email_lc = email.to_lowercase();
        let row = sqlx::query(
            "SELECT username, password_hash FROM users \
             WHERE email = ? AND is_active = 1",
        )
        .bind(&email_lc)
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let (username, hash) = match row {
            Some(r) => (
                r.get::<String, _>("username"),
                r.get::<String, _>("password_hash"),
            ),
            None => {
                opts.set_status(StatusCode::UNAUTHORIZED);
                return Err(ServerFnError::new("Invalid email or password"));
            }
        };

        if !verify_password_ssr(&password, &hash) {
            opts.set_status(StatusCode::UNAUTHORIZED);
            return Err(ServerFnError::new("Invalid email or password"));
        }

        create_and_set_session(&username, &pool, &opts).await?;

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Verify a plaintext password against an argon2id PHC string or a legacy
/// bcrypt hash (`$2b$`/`$2y$`).  SSR-only — not compiled into WASM.
#[cfg(feature = "ssr")]
pub(super) fn verify_password_ssr(password: &str, hash: &str) -> bool {
    use argon2::{Argon2, PasswordHash, PasswordVerifier};
    if hash.starts_with("$2b$") || hash.starts_with("$2y$") || hash.starts_with("$2a$") {
        bcrypt::verify(password, hash).unwrap_or(false)
    } else {
        PasswordHash::new(hash).ok().is_some_and(|h| {
            Argon2::default()
                .verify_password(password.as_bytes(), &h)
                .is_ok()
        })
    }
}

/// Destroy the current session and clear the `session` cookie.
#[server]
pub async fn do_logout() -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use axum::http::{header, HeaderMap, HeaderValue};
        use axum::Extension;
        use leptos::prelude::expect_context;
        use leptos_axum::{extract, ResponseOptions};
        use sqlx::SqlitePool;

        let Extension(pool): Extension<SqlitePool> = extract().await?;
        let headers: HeaderMap = extract().await?;
        let Extension(cookie_name): Extension<crate::ssr_types::SessionCookieName> =
            extract().await?;

        let cookies = headers
            .get(header::COOKIE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        if let Some(token) = cookie_name.extract(cookies) {
            // Intentionally ignored: the cookie is cleared regardless of whether
            // the session row existed (e.g. already expired or double-logout).
            let _ = sqlx::query("DELETE FROM sessions WHERE token = ?")
                .bind(&token)
                .execute(&pool)
                .await;
        }

        let clear = HeaderValue::from_str(&cookie_name.clear_header_value())
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        let opts = expect_context::<ResponseOptions>();
        opts.append_header(header::SET_COOKIE, clear);

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Return whether self-registration is enabled on this server.
#[server]
pub async fn get_registration_enabled() -> Result<bool, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::AllowRegistration;
        use axum::Extension;
        use leptos_axum::extract;

        let enabled = extract::<Extension<AllowRegistration>>()
            .await
            .map(|e| e.0 .0)
            .unwrap_or(true);
        return Ok(enabled);
    }
    #[allow(unreachable_code)]
    Ok(true)
}

/// Create a new (unverified) user account and send a 6-digit email verification
/// code.  The account is created with `is_active = 0`; it becomes active only
/// after the user submits the correct code via [`do_verify_email`].
///
/// Returns the normalised username so the UI can pass it straight to the
/// verification form.
///
/// Any previous unverified account with the same username **or** email is
/// silently replaced so that the user can re-attempt signup freely.
///
/// Fails if SMTP is not configured (email verification is mandatory).
#[server(input = server_fn::codec::Json)]
#[allow(unused_variables)]
pub async fn do_signup(
    username: String,
    email: String,
    display_name: String,
    password: String,
) -> Result<String, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::{AdminEmail, AllowRegistration, ServerBaseUrl, ServerSecret};
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::SqlitePool;

        let Extension(pool): Extension<SqlitePool> = extract().await?;

        let reg_enabled = extract::<Extension<AllowRegistration>>()
            .await
            .map(|e| e.0 .0)
            .unwrap_or(true);
        if !reg_enabled {
            return Err(ServerFnError::new(
                "Registration is disabled on this server",
            ));
        }

        let server_secret = extract::<Extension<ServerSecret>>()
            .await
            .map(|e| e.0 .0.clone())
            .unwrap_or_default();
        let base_url = extract::<Extension<ServerBaseUrl>>()
            .await
            .map(|e| e.0 .0.clone())
            .unwrap_or_default();
        let admin_email = extract::<Extension<AdminEmail>>()
            .await
            .ok()
            .and_then(|e| e.0 .0.clone());

        // Normalise inputs.
        let username = username.trim().to_lowercase();
        let email = email.trim().to_lowercase();
        let display_name = display_name.trim().to_string();
        let display_name = if display_name.is_empty() {
            username.clone()
        } else {
            display_name
        };

        // Validate username: 2–50 chars, a-z / 0-9 / hyphen, no leading/trailing hyphen.
        let valid_slug = username.len() >= 2
            && username.len() <= 50
            && username
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !username.starts_with('-')
            && !username.ends_with('-');
        if !valid_slug {
            return Err(ServerFnError::new(
                "Username must be 2–50 characters and contain only letters, numbers, or hyphens",
            ));
        }

        if email.is_empty() || !email.contains('@') {
            return Err(ServerFnError::new("A valid email address is required"));
        }

        if password.len() < 8 {
            return Err(ServerFnError::new("Password must be at least 8 characters"));
        }

        // Silently remove any previous unverified accounts with conflicting
        // credentials so the user can re-attempt signup without hitting a
        // uniqueness error.
        sqlx::query("DELETE FROM users WHERE (username = ? OR email = ?) AND is_active = 0")
            .bind(&username)
            .bind(&email)
            .execute(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Now check against active accounts only.
        let username_taken: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE username = ? AND is_active = 1")
                .bind(&username)
                .fetch_one(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        if username_taken > 0 {
            return Err(ServerFnError::new("That username is already taken"));
        }

        let email_taken: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE email = ? AND is_active = 1")
                .bind(&email)
                .fetch_one(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        if email_taken > 0 {
            return Err(ServerFnError::new(
                "An account with that email already exists",
            ));
        }

        // Make sure SMTP is configured before creating the user — there's no
        // point creating an unverifiable account.
        let smtp_row = sqlx::query_as::<_, SmtpConfigRow>(SMTP_CONFIG_QUERY)
            .fetch_optional(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
            .ok_or_else(|| {
                ServerFnError::new(
                    "Email verification is required but SMTP is not configured. \
             Ask the administrator to set up email in Settings.",
                )
            })?;

        let hash = hash_password_ssr(&password).map_err(ServerFnError::new)?;

        // Create user as inactive pending email verification.
        sqlx::query(
            "INSERT INTO users \
             (username, email, display_name, password_hash, is_admin, is_active) \
             VALUES (?, ?, ?, ?, 0, 0)",
        )
        .bind(&username)
        .bind(&email)
        .bind(&display_name)
        .bind(&hash)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Generate and store a 6-digit verification code (60-minute window).
        let code = generate_verification_code();

        if let Err(e) = sqlx::query("DELETE FROM email_verifications WHERE username = ?")
            .bind(&username)
            .execute(&pool)
            .await
        {
            tracing::warn!("failed to clean up stale email_verifications for {username}: {e}");
        }

        sqlx::query(
            "INSERT INTO email_verifications (username, code, expires_at) \
             VALUES (?, ?, datetime('now', '+60 minutes'))",
        )
        .bind(&username)
        .bind(&code)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Send the verification email.
        send_verification_email_via_smtp(
            smtp_row,
            &server_secret,
            &email,
            &display_name,
            &code,
            &base_url,
        )
        .await?;

        // Fire-and-forget admin notification (best-effort; does not fail the signup).
        if let Some(addr) = admin_email {
            let pool2 = pool.clone();
            let secret2 = server_secret.clone();
            let new_username = username.clone();
            let new_email = email.clone();
            tokio::spawn(async move {
                if let Err(e) = send_admin_new_user_notification(
                    pool2,
                    secret2,
                    &new_username,
                    &new_email,
                    &addr,
                )
                .await
                {
                    tracing::warn!("admin new-user notification failed: {e}");
                }
            });
        }

        return Ok(username);
    }
    #[allow(unreachable_code)]
    Ok(String::new())
}

/// Verify the 6-digit code sent to the user's email address after sign-up.
///
/// On success the account is activated and a session cookie is set, logging
/// the user in immediately.
#[server(input = server_fn::codec::Json)]
#[allow(unused_variables)]
pub async fn do_verify_email(username: String, code: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use axum::Extension;
        use leptos::prelude::expect_context;
        use leptos_axum::{extract, ResponseOptions};
        use sqlx::SqlitePool;

        let Extension(pool): Extension<SqlitePool> = extract().await?;

        let code = code.trim().to_string();

        // Validate the code (exact match, not expired).
        let row = sqlx::query(
            "SELECT id FROM email_verifications \
             WHERE username = ? AND code = ? AND expires_at > datetime('now')",
        )
        .bind(&username)
        .bind(&code)
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        if row.is_none() {
            return Err(ServerFnError::new(
                "Invalid or expired code — please check your email",
            ));
        }

        // Activate the account.
        sqlx::query(
            "UPDATE users SET is_active = 1, updated_at = CURRENT_TIMESTAMP \
             WHERE username = ?",
        )
        .bind(&username)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Clean up the used verification record.
        if let Err(e) = sqlx::query("DELETE FROM email_verifications WHERE username = ?")
            .bind(&username)
            .execute(&pool)
            .await
        {
            tracing::warn!("failed to delete email_verifications for {username}: {e}");
        }

        // Log the user in immediately.
        let opts = expect_context::<ResponseOptions>();
        create_and_set_session(&username, &pool, &opts).await?;

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

// ── Signup helpers ────────────────────────────────────────────────────────────

/// Hash a plaintext password with argon2id.  Returns the PHC string.
#[cfg(feature = "ssr")]
pub(super) fn hash_password_ssr(password: &str) -> Result<String, String> {
    use argon2::{
        password_hash::{rand_core::OsRng, SaltString},
        Argon2, PasswordHasher,
    };
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

/// Generate a random 6-digit numeric verification code.
#[cfg(feature = "ssr")]
pub(super) fn generate_verification_code() -> String {
    // uuid::Uuid::new_v4() uses a CSPRNG; take its bits mod 1_000_000 for
    // a uniform 6-digit code with a leading-zero-preserving format.
    let n = uuid::Uuid::new_v4().as_u128() % 1_000_000;
    format!("{:06}", n)
}

/// Build the plain-text and HTML bodies for a transactional code email.
///
/// Both verification and password-reset emails share the same structure
/// (greeting → large code → action instruction → expiry → footer disclaimer).
/// The three parameters capture what differs between them.
#[cfg(feature = "ssr")]
pub(super) fn code_email_body(
    display_name: &str,
    code: &str,
    code_label: &str,    // e.g. "verification code" | "password reset code"
    action: &str,        // e.g. "activate your account" | "set a new password"
    footer_clause: &str, // e.g. "sign up for tidl" | "request a password reset"
) -> (String, String) {
    use super::sharing::html_escape;

    let body_text = format!(
        "Hi {display_name},\n\n\
         Your {code_label} is:\n\n    {code}\n\n\
         Enter this code in the browser to {action}.\n\
         The code expires in 60 minutes.\n\n\
         If you did not {footer_clause}, you can safely ignore this email.",
    );
    let body_html = format!(
        "<p>Hi <strong>{name}</strong>,</p>\
         <p>Your {code_label} is:</p>\
         <p style=\"font-size:2rem;letter-spacing:.25em;font-weight:700;\
                    font-family:monospace;color:#1d4ed8\">{code}</p>\
         <p>Enter this code in the browser to {action}.<br>\
         The code expires in 60&nbsp;minutes.</p>\
         <p style=\"color:#6b7280;font-size:0.85em\">\
         If you did not {footer_clause}, you can safely ignore this email.</p>",
        name = html_escape(display_name),
    );
    (body_text, body_html)
}

/// Send a verification email using the SMTP configuration row fetched by the
/// caller.  This avoids a second database round-trip from `do_verify_email`.
#[cfg(feature = "ssr")]
async fn send_verification_email_via_smtp(
    cfg_row: SmtpConfigRow,
    server_secret: &str,
    to_email: &str,
    display_name: &str,
    code: &str,
    _base_url: &str,
) -> Result<(), ServerFnError> {
    use crate::ssr_types::decrypt_password;
    use lettre::{
        message::{header::ContentType, MultiPart, SinglePart},
        Message,
    };

    let password =
        decrypt_password(&cfg_row.password_encrypted, server_secret).map_err(ServerFnError::new)?;
    let from = cfg_row.header_from();

    let subject = "Verify your tidl account".to_string();
    let (body_text, body_html) = code_email_body(
        display_name,
        code,
        "verification code",
        "activate your account",
        "sign up for tidl",
    );

    let msg = Message::builder()
        .from(
            from.parse()
                .map_err(|e: lettre::address::AddressError| ServerFnError::new(e.to_string()))?,
        )
        .to(to_email
            .parse()
            .map_err(|e: lettre::address::AddressError| ServerFnError::new(e.to_string()))?)
        .subject(subject)
        .multipart(
            MultiPart::alternative()
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_PLAIN)
                        .body(body_text),
                )
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_HTML)
                        .body(body_html),
                ),
        )
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    crate::ssr_types::smtp_send(
        &cfg_row.host,
        cfg_row.port as u16,
        &cfg_row.tls_mode,
        &cfg_row.username,
        &password,
        msg,
    )
    .await
    .map_err(ServerFnError::new)?;

    Ok(())
}

/// Send a best-effort admin notification when a new user account is created.
///
/// Fetches SMTP config from the database and sends a plain-text email to
/// `admin_addr`.  Failures are logged as warnings but never propagate to the
/// caller — the function returns `Result<(), String>` so it can be spawned
/// as a fire-and-forget task.
#[cfg(feature = "ssr")]
async fn send_admin_new_user_notification(
    pool: sqlx::SqlitePool,
    server_secret: String,
    new_username: &str,
    new_email: &str,
    admin_addr: &str,
) -> Result<(), String> {
    use crate::ssr_types::decrypt_password;
    use lettre::{
        message::{header::ContentType, SinglePart},
        Message,
    };

    let cfg_row = sqlx::query_as::<_, SmtpConfigRow>(SMTP_CONFIG_QUERY)
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| e.to_string())?;

    let cfg_row = match cfg_row {
        Some(r) => r,
        None => return Ok(()), // SMTP not configured; skip silently
    };

    let password = decrypt_password(&cfg_row.password_encrypted, &server_secret)?;
    let from = cfg_row.header_from();

    let body = format!(
        "A new tidl account has been created.\n\nUsername: {new_username}\nEmail:    {new_email}\n",
    );

    let msg = Message::builder()
        .from(
            from.parse()
                .map_err(|e: lettre::address::AddressError| e.to_string())?,
        )
        .to(admin_addr
            .parse()
            .map_err(|e: lettre::address::AddressError| e.to_string())?)
        .subject(format!("New tidl account: {new_username}"))
        .singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_PLAIN)
                .body(body),
        )
        .map_err(|e| e.to_string())?;

    crate::ssr_types::smtp_send(
        &cfg_row.host,
        cfg_row.port as u16,
        &cfg_row.tls_mode,
        &cfg_row.username,
        &password,
        msg,
    )
    .await?;
    Ok(())
}

// ── Password reset ────────────────────────────────────────────────────────────

/// Request a password-reset code by email.
///
/// If the email is unknown or SMTP is not configured this still returns `Ok`
/// (to avoid leaking whether an account exists).  The only failure modes
/// surfaced to the user are configuration problems.
#[server(input = server_fn::codec::Json)]
#[allow(unused_variables)]
pub async fn do_request_password_reset(email: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::ServerSecret;
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::{Row, SqlitePool};

        let Extension(pool): Extension<SqlitePool> = extract().await?;
        let server_secret = extract::<Extension<ServerSecret>>()
            .await
            .map(|e| e.0 .0.clone())
            .unwrap_or_default();

        let email_lc = email.trim().to_lowercase();

        // Look up user — silently succeed if not found.
        let row = sqlx::query(
            "SELECT username, display_name FROM users WHERE email = ? AND is_active = 1",
        )
        .bind(&email_lc)
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let (username, display_name) = match row {
            Some(r) => (
                r.get::<String, _>("username"),
                r.get::<String, _>("display_name"),
            ),
            // Unknown email — return Ok silently so as not to leak whether the
            // address is registered.
            None => {
                tracing::info!(email = %email_lc, "password reset requested for unknown email — ignoring silently");
                return Ok(());
            }
        };

        tracing::info!(username = %username, "password reset requested — sending code");

        // Fetch SMTP config (row from smtp_config table).
        let smtp_row = sqlx::query_as::<_, SmtpConfigRow>(SMTP_CONFIG_QUERY)
            .fetch_optional(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
            .ok_or_else(|| {
                ServerFnError::new(
                    "Email is not configured on this server. Ask the administrator to set up \
             SMTP in Settings, or use the server CLI to reset your password.",
                )
            })?;

        // Generate a 6-digit code, valid for 60 minutes.
        let code = generate_verification_code();

        // Replace any previous pending reset for this email.
        if let Err(e) = sqlx::query("DELETE FROM password_resets WHERE email = ?")
            .bind(&email_lc)
            .execute(&pool)
            .await
        {
            tracing::warn!("failed to delete stale password_resets for {email_lc}: {e}");
        }

        sqlx::query(
            "INSERT INTO password_resets (email, code, expires_at) \
             VALUES (?, ?, datetime('now', '+60 minutes'))",
        )
        .bind(&email_lc)
        .bind(&code)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        send_password_reset_email(smtp_row, &server_secret, &email_lc, &display_name, &code)
            .await?;

        tracing::info!(username = %username, "password reset email sent successfully");

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Validate a password-reset code and replace the user's password.
///
/// On success the account password is updated and a session cookie is set,
/// logging the user in immediately.
#[server(input = server_fn::codec::Json)]
#[allow(unused_variables)]
pub async fn do_reset_password(
    email: String,
    code: String,
    new_password: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use axum::Extension;
        use leptos::prelude::expect_context;
        use leptos_axum::{extract, ResponseOptions};
        use sqlx::SqlitePool;

        let Extension(pool): Extension<SqlitePool> = extract().await?;

        let email_lc = email.trim().to_lowercase();
        let code = code.trim().to_string();

        if new_password.len() < 8 {
            return Err(ServerFnError::new("Password must be at least 8 characters"));
        }

        // Validate the code (exact match, not expired).
        let reset_row = sqlx::query(
            "SELECT id FROM password_resets \
             WHERE email = ? AND code = ? AND expires_at > datetime('now')",
        )
        .bind(&email_lc)
        .bind(&code)
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        if reset_row.is_none() {
            return Err(ServerFnError::new(
                "Invalid or expired code — please request a new one",
            ));
        }

        // Consume the reset record.
        if let Err(e) = sqlx::query("DELETE FROM password_resets WHERE email = ?")
            .bind(&email_lc)
            .execute(&pool)
            .await
        {
            tracing::warn!("failed to delete password_resets for {email_lc}: {e}");
        }

        // Fetch the user.
        let username: String =
            sqlx::query_scalar("SELECT username FROM users WHERE email = ? AND is_active = 1")
                .bind(&email_lc)
                .fetch_optional(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
                .ok_or_else(|| ServerFnError::new("Account not found"))?;

        // Update the password.
        let hash = hash_password_ssr(&new_password).map_err(ServerFnError::new)?;
        sqlx::query(
            "UPDATE users SET password_hash = ?, updated_at = CURRENT_TIMESTAMP \
             WHERE username = ?",
        )
        .bind(&hash)
        .bind(&username)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Log the user in immediately.
        let opts = expect_context::<ResponseOptions>();
        create_and_set_session(&username, &pool, &opts).await?;

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(feature = "ssr")]
async fn send_password_reset_email(
    cfg_row: SmtpConfigRow,
    server_secret: &str,
    to_email: &str,
    display_name: &str,
    code: &str,
) -> Result<(), ServerFnError> {
    use crate::ssr_types::decrypt_password;
    use lettre::{
        message::{header::ContentType, MultiPart, SinglePart},
        Message,
    };

    let password =
        decrypt_password(&cfg_row.password_encrypted, server_secret).map_err(ServerFnError::new)?;
    let from = cfg_row.header_from();

    let (body_text, body_html) = code_email_body(
        display_name,
        code,
        "password reset code",
        "set a new password",
        "request a password reset",
    );

    let msg = Message::builder()
        .from(
            from.parse()
                .map_err(|e: lettre::address::AddressError| ServerFnError::new(e.to_string()))?,
        )
        .to(to_email
            .parse()
            .map_err(|e: lettre::address::AddressError| ServerFnError::new(e.to_string()))?)
        .subject("Reset your tidl password")
        .multipart(
            MultiPart::alternative()
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_PLAIN)
                        .body(body_text),
                )
                .singlepart(
                    SinglePart::builder()
                        .header(ContentType::TEXT_HTML)
                        .body(body_html),
                ),
        )
        .map_err(|e| ServerFnError::new(e.to_string()))?;

    crate::ssr_types::smtp_send(
        &cfg_row.host,
        cfg_row.port as u16,
        &cfg_row.tls_mode,
        &cfg_row.username,
        &password,
        msg,
    )
    .await
    .map_err(ServerFnError::new)?;

    Ok(())
}
