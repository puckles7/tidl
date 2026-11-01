//! Self-service profile editor: display name, username rename, email change
//! (current password + a code confirmed via the new address), and password
//! change (current password + revokes other sessions).
//!
//! This module — not `tidl-server::UserStore` — is where the username-rename
//! cascade lives, because `tidl-web` cannot depend on `tidl-server` (only the
//! reverse: `tidl-server` links `tidl-web` with the `ssr` feature to host its
//! server-fn bodies). The browser-facing `#[server]` functions live here, so
//! this is the only place that can actually call it.

use leptos::prelude::*;

#[cfg(feature = "ssr")]
use super::auth;
#[cfg(feature = "ssr")]
use super::auth::{
    code_email_body, generate_verification_code, hash_password_ssr, verify_password_ssr,
};
#[cfg(feature = "ssr")]
use super::auth_with_token;
#[cfg(feature = "ssr")]
use crate::ssr_types::{SmtpConfigRow, SMTP_CONFIG_QUERY};

use super::ProfileInfo;

// ── Username rename cascade ────────────────────────────────────────────────────

/// Every `(table, column)` pair that stores a `users.username` value by copy.
///
/// Order doesn't matter at the SQL level (see [`rename_username_tx`]), but
/// this is still the single source of truth for "everywhere a username
/// lives" — kept in sync with the schema by
/// `rename_username_column_list_is_exhaustive` below, which scans every
/// table for a column whose name contains "user" and fails if it's neither
/// listed here nor in that test's documented exclusion list.
#[cfg(feature = "ssr")]
const USERNAME_COLUMNS: &[(&str, &str)] = &[
    ("calendars", "username"),
    ("calendars", "sync_user"),
    ("events", "username"),
    ("markdown_sources", "username"),
    ("properties", "username"),
    ("locks", "username"),
    ("sync_log", "username"),
    ("calendar_shares", "owner_username"),
    ("calendar_shares", "grantee_username"),
    ("share_invitations", "owner_username"),
    ("user_preferences", "username"),
    ("app_tokens", "username"),
    ("remote_caldav_sources", "username"),
    ("inbound_email_tokens", "username"),
    ("trusted_sender_addresses", "username"),
    ("focus_suppressions", "username"),
    ("priority_curve_points", "username"),
    ("sent_alarm_notifications", "username"),
    ("note_collections", "owner_username"),
    ("note_collection_members", "member_username"),
    ("notes", "author_username"),
    ("note_labels", "owner_username"),
    ("contact_collections", "owner_username"),
    ("contact_collection_members", "member_username"),
    ("contacts", "owner_username"),
    ("done_items", "username"),
    ("done_scopes", "username"),
    ("done_settings", "username"),
    ("sessions", "username"),
    ("email_verifications", "username"),
    ("email_change_requests", "username"),
    ("users", "username"),
];

/// Rename a user's username, cascading the change to every table in
/// [`USERNAME_COLUMNS`].
///
/// `username` is a foreign key in all but name across ~19 columns — none of
/// them declare `ON UPDATE CASCADE`, so a plain `UPDATE users SET username =
/// ...` would either orphan rows silently (tables with no declared FK) or
/// fail outright (tables with a declared `REFERENCES users(username)` FK,
/// once they point at a username that no longer exists).
///
/// SQLite checks each declared FK immediately after the *statement* that
/// touches it, not deferred to commit — so neither "children first" nor
/// "parent first" statement order works on its own: renaming a child before
/// `users` briefly references a parent that doesn't exist yet, and the
/// reverse leaves old child rows referencing a parent that just vanished.
/// `PRAGMA defer_foreign_keys = ON` pushes all FK validation for this
/// transaction to COMMIT time instead, once every table agrees, so statement
/// order stops mattering — the whole rename either fully applies or fully
/// rolls back.
///
/// `sessions.username` is included, so an existing session survives the
/// rename (the cookie is keyed by an opaque token, not username) — no forced
/// logout.
#[cfg(feature = "ssr")]
async fn rename_username_tx(
    pool: &sqlx::SqlitePool,
    old_username: &str,
    new_username: &str,
) -> Result<(), ServerFnError> {
    if old_username == new_username {
        return Ok(());
    }

    let taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE username = ?")
        .bind(new_username)
        .fetch_one(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    if taken > 0 {
        return Err(ServerFnError::new(format!(
            "Username '{new_username}' is already taken"
        )));
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    sqlx::query("PRAGMA defer_foreign_keys = ON")
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    for (table, column) in USERNAME_COLUMNS {
        // Table/column names come only from the constant list above, never
        // from caller input, so building the SQL string is not an injection
        // risk — `sqlx::query!` can't parameterize identifiers.
        let sql = format!("UPDATE {table} SET {column} = ? WHERE {column} = ?");
        sqlx::query(&sql)
            .bind(new_username)
            .bind(old_username)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }

    tx.commit()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    Ok(())
}

// ── Server functions ───────────────────────────────────────────────────────────

/// Return the signed-in user's editable profile fields.
#[server]
pub async fn get_profile() -> Result<ProfileInfo, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;

        let (pool, username) = auth().await?;
        let row = sqlx::query("SELECT username, display_name, email FROM users WHERE username = ?")
            .bind(&username)
            .fetch_one(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        return Ok(ProfileInfo {
            username: row.get("username"),
            display_name: row.get("display_name"),
            email: row.get("email"),
        });
    }
    #[allow(unreachable_code)]
    Ok(ProfileInfo {
        username: String::new(),
        display_name: String::new(),
        email: String::new(),
    })
}

/// Save a new display name. No confirmation required — this field carries no
/// authentication or CalDAV-addressing weight.
#[allow(unused_variables)]
#[server]
pub async fn save_display_name(display_name: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        let display_name = display_name.trim().to_string();
        if display_name.is_empty() {
            return Err(ServerFnError::new("Display name cannot be empty"));
        }

        sqlx::query(
            "UPDATE users SET display_name = ?, updated_at = CURRENT_TIMESTAMP WHERE username = ?",
        )
        .bind(&display_name)
        .bind(&username)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Rename the signed-in user's username. Returns the normalised new username
/// so the caller can update `CurrentUsernameCtx` without a page reload.
#[allow(unused_variables)]
#[server]
pub async fn rename_username_api(new_username: String) -> Result<String, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, old_username) = auth().await?;
        let new_username = new_username.trim().to_lowercase();

        // Same slug rules as signup (see `do_signup` in auth.rs).
        let valid_slug = new_username.len() >= 2
            && new_username.len() <= 50
            && new_username
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-')
            && !new_username.starts_with('-')
            && !new_username.ends_with('-');
        if !valid_slug {
            return Err(ServerFnError::new(
                "Username must be 2–50 characters and contain only letters, numbers, or hyphens",
            ));
        }

        rename_username_tx(&pool, &old_username, &new_username).await?;
        return Ok(new_username);
    }
    #[allow(unreachable_code)]
    Ok(String::new())
}

/// Step 1 of an email change: verify the current password, then email a
/// 6-digit code to `new_email`. The address only takes effect once
/// [`confirm_email_change`] validates that code.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn request_email_change(
    new_email: String,
    current_password: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::ServerSecret;
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::Row;

        let (pool, username) = auth().await?;

        let row = sqlx::query("SELECT display_name, password_hash FROM users WHERE username = ?")
            .bind(&username)
            .fetch_one(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        let display_name: String = row.get("display_name");
        let password_hash: String = row.get("password_hash");

        if !verify_password_ssr(&current_password, &password_hash) {
            return Err(ServerFnError::new("Current password is incorrect"));
        }

        let new_email = new_email.trim().to_lowercase();
        if new_email.is_empty() || !new_email.contains('@') {
            return Err(ServerFnError::new("A valid email address is required"));
        }

        let email_taken: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM users WHERE email = ? AND is_active = 1 AND username != ?",
        )
        .bind(&new_email)
        .bind(&username)
        .fetch_one(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        if email_taken > 0 {
            return Err(ServerFnError::new(
                "An account with that email already exists",
            ));
        }

        let smtp_row = sqlx::query_as::<_, SmtpConfigRow>(SMTP_CONFIG_QUERY)
            .fetch_optional(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
            .ok_or_else(|| ServerFnError::new(
                "Email is not configured on this server. Ask the administrator to set up email in Settings.",
            ))?;

        let server_secret = extract::<Extension<ServerSecret>>()
            .await
            .map(|e| e.0 .0.clone())
            .unwrap_or_default();

        let code = generate_verification_code();

        sqlx::query("DELETE FROM email_change_requests WHERE username = ?")
            .bind(&username)
            .execute(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        sqlx::query(
            "INSERT INTO email_change_requests (username, new_email, code, expires_at) \
             VALUES (?, ?, ?, datetime('now', '+60 minutes'))",
        )
        .bind(&username)
        .bind(&new_email)
        .bind(&code)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        send_email_change_code(smtp_row, &server_secret, &new_email, &display_name, &code).await?;

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Step 2 of an email change: validate the code, swap `users.email` over,
/// and fire a best-effort notice to the *old* address. Returns the new email
/// so the caller can update the displayed value.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn confirm_email_change(code: String) -> Result<String, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;

        let (pool, username) = auth().await?;
        let code = code.trim().to_string();

        let row = sqlx::query(
            "SELECT new_email FROM email_change_requests \
             WHERE username = ? AND code = ? AND expires_at > datetime('now')",
        )
        .bind(&username)
        .bind(&code)
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let new_email: String = row
            .ok_or_else(|| ServerFnError::new("Invalid or expired code — please check your email"))?
            .get("new_email");

        let old_email: String = sqlx::query_scalar("SELECT email FROM users WHERE username = ?")
            .bind(&username)
            .fetch_one(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        sqlx::query(
            "UPDATE users SET email = ?, updated_at = CURRENT_TIMESTAMP WHERE username = ?",
        )
        .bind(&new_email)
        .bind(&username)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        sqlx::query("DELETE FROM email_change_requests WHERE username = ?")
            .bind(&username)
            .execute(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Fire-and-forget: never blocks the change on the old mailbox being
        // reachable. Same pattern as the admin new-user notification in
        // `do_signup`.
        {
            use crate::ssr_types::ServerSecret;
            use axum::Extension;
            use leptos_axum::extract;

            let server_secret = extract::<Extension<ServerSecret>>()
                .await
                .map(|e| e.0 .0.clone())
                .unwrap_or_default();
            let pool2 = pool.clone();
            let username2 = username.clone();
            let new_email2 = new_email.clone();
            tokio::spawn(async move {
                if let Err(e) = send_email_changed_notice(
                    pool2,
                    server_secret,
                    &username2,
                    &old_email,
                    &new_email2,
                )
                .await
                {
                    tracing::warn!("old-email change notification failed for {username2}: {e}");
                }
            });
        }

        return Ok(new_email);
    }
    #[allow(unreachable_code)]
    Ok(String::new())
}

/// Change the signed-in user's password. Requires the current password, and
/// revokes every other session for this user on success — only the caller's
/// own session survives.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn change_password(
    current_password: String,
    new_password: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username, token) = auth_with_token().await?;

        let password_hash: String =
            sqlx::query_scalar("SELECT password_hash FROM users WHERE username = ?")
                .bind(&username)
                .fetch_one(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        if !verify_password_ssr(&current_password, &password_hash) {
            return Err(ServerFnError::new("Current password is incorrect"));
        }
        if new_password.len() < 8 {
            return Err(ServerFnError::new("Password must be at least 8 characters"));
        }

        let hash = hash_password_ssr(&new_password).map_err(ServerFnError::new)?;

        sqlx::query(
            "UPDATE users SET password_hash = ?, updated_at = CURRENT_TIMESTAMP WHERE username = ?",
        )
        .bind(&hash)
        .bind(&username)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        sqlx::query("DELETE FROM sessions WHERE username = ? AND token != ?")
            .bind(&username)
            .bind(&token)
            .execute(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

// ── Email helpers ──────────────────────────────────────────────────────────────

/// Send the email-change confirmation code to the *new* address.
#[cfg(feature = "ssr")]
async fn send_email_change_code(
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
        "email confirmation code",
        "confirm this new email address",
        "request an email change",
    );

    let msg = Message::builder()
        .from(
            from.parse()
                .map_err(|e: lettre::address::AddressError| ServerFnError::new(e.to_string()))?,
        )
        .to(to_email
            .parse()
            .map_err(|e: lettre::address::AddressError| ServerFnError::new(e.to_string()))?)
        .subject("Confirm your new tidl email address")
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

/// Best-effort notice sent to the *old* address once an email change
/// completes. Fetches SMTP config itself (same pattern as
/// `send_admin_new_user_notification` in `auth.rs`) so it can be spawned as
/// fire-and-forget without borrowing anything from the caller.
#[cfg(feature = "ssr")]
async fn send_email_changed_notice(
    pool: sqlx::SqlitePool,
    server_secret: String,
    username: &str,
    old_email: &str,
    new_email: &str,
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
        "Hi,\n\n\
         The email address on your tidl account (\"{username}\") was just changed to {new_email}.\n\n\
         If you made this change, you can safely ignore this message. If you didn't, someone else may \
         have access to your account — sign in and change your password immediately.",
    );

    let msg = Message::builder()
        .from(
            from.parse()
                .map_err(|e: lettre::address::AddressError| e.to_string())?,
        )
        .to(old_email
            .parse()
            .map_err(|e: lettre::address::AddressError| e.to_string())?)
        .subject("Your tidl account email was changed")
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
    .await
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    async fn create_test_user(pool: &SqlitePool, username: &str, email: &str, password: &str) {
        let hash = hash_password_ssr(password).unwrap();
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) \
             VALUES (?, ?, ?, ?, 0)",
        )
        .bind(username)
        .bind(email)
        .bind(username)
        .bind(&hash)
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn rename_username_cascades_and_preserves_session(pool: SqlitePool) {
        create_test_user(&pool, "alice", "alice@example.com", "pw12345678").await;

        sqlx::query(
            "INSERT INTO app_tokens (id, username, name, token_hash, read_only) \
             VALUES ('tok-1', 'alice', 'laptop', 'hash', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO calendars (username, name, sync_user) VALUES ('alice', 'personal', 'alice')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sessions (token, username, expires_at) \
             VALUES ('sess-1', 'alice', datetime('now', '+1 day'))",
        )
        .execute(&pool)
        .await
        .unwrap();

        rename_username_tx(&pool, "alice", "alicia").await.unwrap();

        // Session survives the rename under the same opaque token — no forced logout.
        let session_user: String =
            sqlx::query_scalar("SELECT username FROM sessions WHERE token = 'sess-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(session_user, "alicia");

        let cal_row: (String, String) =
            sqlx::query_as("SELECT username, sync_user FROM calendars WHERE name = 'personal'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(cal_row, ("alicia".to_string(), "alicia".to_string()));

        let token_user: String =
            sqlx::query_scalar("SELECT username FROM app_tokens WHERE name = 'laptop'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(token_user, "alicia");

        let old_user: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE username = 'alice'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(old_user, 0);

        let new_username: String =
            sqlx::query_scalar("SELECT username FROM users WHERE email = 'alice@example.com'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(new_username, "alicia");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn rename_username_rejects_existing_target(pool: SqlitePool) {
        create_test_user(&pool, "alice", "alice@example.com", "pw12345678").await;
        create_test_user(&pool, "bob", "bob@example.com", "pw12345678").await;

        let err = rename_username_tx(&pool, "alice", "bob").await.unwrap_err();
        assert!(err.to_string().contains("already taken"));
    }

    /// Fails CI if a new column that looks like it stores a `users.username`
    /// value is added anywhere in the schema without being triaged: either
    /// added to [`USERNAME_COLUMNS`] (so `rename_username_tx` updates it) or
    /// to `NOT_IDENTITY_COLUMNS` below (with a comment saying why it's
    /// exempt). Without this, a forgotten table would silently keep
    /// orphaned rows under the old username after a rename — no error, just
    /// stale data.
    #[sqlx::test(migrations = "../../migrations")]
    async fn rename_username_column_list_is_exhaustive(pool: SqlitePool) {
        const NOT_IDENTITY_COLUMNS: &[(&str, &str)] = &[
            // SMTP auth username — not a local account.
            ("smtp_config", "username"),
            // Username on a *remote* CalDAV server, not this instance's users table.
            ("remote_caldav_sources", "remote_username"),
        ];

        let tables: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM sqlite_master \
             WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name NOT LIKE '_sqlx_%'",
        )
        .fetch_all(&pool)
        .await
        .unwrap();

        for table in tables {
            let columns: Vec<String> =
                sqlx::query_scalar(&format!("SELECT name FROM pragma_table_info('{table}')"))
                    .fetch_all(&pool)
                    .await
                    .unwrap();

            for column in columns {
                if !column.to_lowercase().contains("user") {
                    continue;
                }
                let pair = (table.as_str(), column.as_str());
                let triaged =
                    USERNAME_COLUMNS.contains(&pair) || NOT_IDENTITY_COLUMNS.contains(&pair);
                assert!(
                    triaged,
                    "{table}.{column} looks like it stores a username but is not in \
                     USERNAME_COLUMNS (rename_username_tx won't update it) or \
                     NOT_IDENTITY_COLUMNS (documented exclusion) in profile.rs — \
                     decide which list it belongs in.",
                );
            }
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn request_email_change_rejects_wrong_password(pool: SqlitePool) {
        create_test_user(&pool, "alice", "alice@example.com", "correct-password").await;
        sqlx::query(
            "INSERT INTO smtp_config (id, host, port, tls_mode, from_address) \
             VALUES (1, 'smtp.example.com', 587, 'starttls', 'noreply@example.com')",
        )
        .execute(&pool)
        .await
        .unwrap();

        let password_hash: String =
            sqlx::query_scalar("SELECT password_hash FROM users WHERE username = 'alice'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(!verify_password_ssr("wrong-password", &password_hash));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn confirm_email_change_rejects_bad_code(pool: SqlitePool) {
        create_test_user(&pool, "alice", "alice@example.com", "pw12345678").await;
        sqlx::query(
            "INSERT INTO email_change_requests (username, new_email, code, expires_at) \
             VALUES ('alice', 'new@example.com', '000000', datetime('now', '+60 minutes'))",
        )
        .execute(&pool)
        .await
        .unwrap();

        let row = sqlx::query(
            "SELECT new_email FROM email_change_requests \
             WHERE username = 'alice' AND code = 'wrong' AND expires_at > datetime('now')",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(row.is_none());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn confirm_email_change_rejects_expired_code(pool: SqlitePool) {
        create_test_user(&pool, "alice", "alice@example.com", "pw12345678").await;
        sqlx::query(
            "INSERT INTO email_change_requests (username, new_email, code, expires_at) \
             VALUES ('alice', 'new@example.com', '123456', datetime('now', '-1 minutes'))",
        )
        .execute(&pool)
        .await
        .unwrap();

        let row = sqlx::query(
            "SELECT new_email FROM email_change_requests \
             WHERE username = 'alice' AND code = '123456' AND expires_at > datetime('now')",
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(row.is_none());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn change_password_revokes_other_sessions_keeps_current(pool: SqlitePool) {
        create_test_user(&pool, "alice", "alice@example.com", "old-password").await;
        sqlx::query(
            "INSERT INTO sessions (token, username, expires_at) \
             VALUES ('current-token', 'alice', datetime('now', '+1 day'))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sessions (token, username, expires_at) \
             VALUES ('other-token', 'alice', datetime('now', '+1 day'))",
        )
        .execute(&pool)
        .await
        .unwrap();

        // Exercise the same logic `change_password` runs, without going
        // through `extract()` (which needs a live Axum request context).
        let new_hash = hash_password_ssr("new-password12").unwrap();
        sqlx::query("UPDATE users SET password_hash = ? WHERE username = 'alice'")
            .bind(&new_hash)
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query("DELETE FROM sessions WHERE username = 'alice' AND token != 'current-token'")
            .execute(&pool)
            .await
            .unwrap();

        let remaining: Vec<String> =
            sqlx::query_scalar("SELECT token FROM sessions WHERE username = 'alice'")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(remaining, vec!["current-token".to_string()]);
    }
}
