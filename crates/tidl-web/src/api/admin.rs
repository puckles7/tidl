//! Admin-only user account management: list, create, promote/demote,
//! activate/deactivate, reset password, and delete.
//!
//! `tidl-web` cannot depend on `tidl-server`'s `UserStore` (see the module
//! doc on `rename_username_tx` in `profile.rs` for why), so this duplicates
//! the minimal set of raw `sqlx` queries needed, gated by `auth_admin()`.
//!
//! An admin can never target their own account here (delete, deactivate,
//! demote) — every mutating `#[server]` fn checks `username == acting`
//! before touching the DB. This also guarantees at least one active admin
//! always remains, with no separate "last admin" counter needed.

use leptos::prelude::*;

#[cfg(feature = "ssr")]
use super::auth_admin;

use super::UserAdminInfo;

/// List every user account (including inactive), ordered by username.
#[server]
pub async fn list_users() -> Result<Vec<UserAdminInfo>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;

        let (pool, _acting) = auth_admin().await?;
        let rows = sqlx::query(
            "SELECT username, email, display_name, is_admin, is_active \
             FROM users ORDER BY username",
        )
        .fetch_all(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        return Ok(rows
            .into_iter()
            .map(|r| UserAdminInfo {
                username: r.get("username"),
                email: r.get("email"),
                display_name: r.get("display_name"),
                is_admin: r.get::<i64, _>("is_admin") != 0,
                is_active: r.get::<i64, _>("is_active") != 0,
            })
            .collect());
    }
    #[allow(unreachable_code)]
    Ok(Vec::new())
}

#[cfg(feature = "ssr")]
async fn create_user_admin_tx(
    pool: &sqlx::SqlitePool,
    username: &str,
    email: &str,
    display_name: &str,
    password: &str,
) -> Result<(), ServerFnError> {
    let username = username.trim().to_lowercase();
    let email = email.trim().to_lowercase();
    let display_name = display_name.trim().to_string();
    let display_name = if display_name.is_empty() {
        username.clone()
    } else {
        display_name
    };

    // Same slug rules as signup (see `do_signup` in auth.rs).
    let valid_slug = username.len() >= 2
        && username.len() <= 50
        && username
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-')
        && !username.starts_with('-')
        && !username.ends_with('-');
    if !valid_slug {
        return Err(ServerFnError::new(
            "Username must be 2\u{2013}50 characters and contain only letters, numbers, or hyphens",
        ));
    }
    if email.is_empty() || !email.contains('@') {
        return Err(ServerFnError::new("A valid email address is required"));
    }
    if password.len() < 8 {
        return Err(ServerFnError::new("Password must be at least 8 characters"));
    }

    let username_taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE username = ?")
        .bind(&username)
        .fetch_one(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    if username_taken > 0 {
        return Err(ServerFnError::new("That username is already taken"));
    }

    let email_taken: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE email = ?")
        .bind(&email)
        .fetch_one(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    if email_taken > 0 {
        return Err(ServerFnError::new(
            "An account with that email already exists",
        ));
    }

    let hash = super::auth::hash_password_ssr(password).map_err(ServerFnError::new)?;

    sqlx::query(
        "INSERT INTO users (username, email, display_name, password_hash, is_admin, is_active) \
         VALUES (?, ?, ?, ?, 0, 1)",
    )
    .bind(&username)
    .bind(&email)
    .bind(&display_name)
    .bind(&hash)
    .execute(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    Ok(())
}

/// Create a new, active, non-admin user account. Admin flag is set
/// afterward via [`set_user_admin`].
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn create_user_admin(
    username: String,
    email: String,
    display_name: String,
    password: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, _acting) = auth_admin().await?;
        create_user_admin_tx(&pool, &username, &email, &display_name, &password).await?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(feature = "ssr")]
async fn set_user_admin_tx(
    pool: &sqlx::SqlitePool,
    username: &str,
    is_admin: bool,
) -> Result<(), ServerFnError> {
    let affected = sqlx::query(
        "UPDATE users SET is_admin = ?, updated_at = CURRENT_TIMESTAMP WHERE username = ?",
    )
    .bind(is_admin as i64)
    .bind(username)
    .execute(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
    .rows_affected();

    if affected == 0 {
        return Err(ServerFnError::new("User not found"));
    }
    Ok(())
}

#[cfg(feature = "ssr")]
async fn set_user_active_tx(
    pool: &sqlx::SqlitePool,
    username: &str,
    is_active: bool,
) -> Result<(), ServerFnError> {
    let affected = sqlx::query(
        "UPDATE users SET is_active = ?, updated_at = CURRENT_TIMESTAMP WHERE username = ?",
    )
    .bind(is_active as i64)
    .bind(username)
    .execute(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
    .rows_affected();

    if affected == 0 {
        return Err(ServerFnError::new("User not found"));
    }
    Ok(())
}

/// Grant or revoke admin rights. Rejects targeting the caller's own account.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn set_user_admin(username: String, is_admin: bool) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, acting) = auth_admin().await?;
        if username == acting {
            return Err(ServerFnError::new("You can't change your own account here"));
        }
        set_user_admin_tx(&pool, &username, is_admin).await?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Enable or disable a user account. Rejects targeting the caller's own account.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn set_user_active(username: String, is_active: bool) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, acting) = auth_admin().await?;
        if username == acting {
            return Err(ServerFnError::new("You can't change your own account here"));
        }
        set_user_active_tx(&pool, &username, is_active).await?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Update `username`'s password hash and revoke every one of their sessions
/// (unlike self-service `change_password`, there's no "current session" to
/// preserve — the target isn't the caller). Returns `(email, display_name)`
/// so the caller can send a best-effort notice.
#[cfg(feature = "ssr")]
async fn reset_password_core(
    pool: &sqlx::SqlitePool,
    username: &str,
    new_password: &str,
) -> Result<(String, String), ServerFnError> {
    use sqlx::Row;

    if new_password.len() < 8 {
        return Err(ServerFnError::new("Password must be at least 8 characters"));
    }

    let row = sqlx::query("SELECT email, display_name FROM users WHERE username = ?")
        .bind(username)
        .fetch_optional(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("User not found"))?;
    let email: String = row.get("email");
    let display_name: String = row.get("display_name");

    let hash = super::auth::hash_password_ssr(new_password).map_err(ServerFnError::new)?;

    sqlx::query(
        "UPDATE users SET password_hash = ?, updated_at = CURRENT_TIMESTAMP WHERE username = ?",
    )
    .bind(&hash)
    .bind(username)
    .execute(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    sqlx::query("DELETE FROM sessions WHERE username = ?")
        .bind(username)
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    Ok((email, display_name))
}

/// Best-effort notice that an admin reset this user's password. Fetches SMTP
/// config itself and returns `Ok(())` silently if it isn't configured — same
/// pattern as `send_email_changed_notice` in `profile.rs`.
#[cfg(feature = "ssr")]
async fn send_admin_password_reset_notice(
    pool: sqlx::SqlitePool,
    server_secret: String,
    email: &str,
    display_name: &str,
) -> Result<(), String> {
    use crate::ssr_types::{decrypt_password, smtp_send, SmtpConfigRow, SMTP_CONFIG_QUERY};
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
        "Hi {display_name},\n\n\
         An administrator just reset the password on your tidl account.\n\n\
         If you didn't expect this, contact your administrator immediately.",
    );

    let msg = Message::builder()
        .from(
            from.parse()
                .map_err(|e: lettre::address::AddressError| e.to_string())?,
        )
        .to(email
            .parse()
            .map_err(|e: lettre::address::AddressError| e.to_string())?)
        .subject("Your tidl account password was reset")
        .singlepart(
            SinglePart::builder()
                .header(ContentType::TEXT_PLAIN)
                .body(body),
        )
        .map_err(|e| e.to_string())?;

    smtp_send(
        &cfg_row.host,
        cfg_row.port as u16,
        &cfg_row.tls_mode,
        &cfg_row.username,
        &password,
        msg,
    )
    .await
}

/// Set a new password for `username` directly (no confirmation email flow),
/// revoking all of their sessions and firing a best-effort notice. Rejects
/// targeting the caller's own account.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn admin_reset_password(
    username: String,
    new_password: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::ServerSecret;
        use axum::Extension;
        use leptos_axum::extract;

        let (pool, acting) = auth_admin().await?;
        if username == acting {
            return Err(ServerFnError::new(
                "You can't reset your own password here — use your profile settings",
            ));
        }

        let (email, display_name) = reset_password_core(&pool, &username, &new_password).await?;

        let server_secret = extract::<Extension<ServerSecret>>()
            .await
            .map(|e| e.0 .0.clone())
            .unwrap_or_default();

        tokio::spawn(async move {
            if let Err(e) =
                send_admin_password_reset_notice(pool, server_secret, &email, &display_name).await
            {
                tracing::warn!("admin password-reset notice failed for {username}: {e}");
            }
        });

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(feature = "ssr")]
async fn deletion_summary_core(
    pool: &sqlx::SqlitePool,
    username: &str,
) -> Result<super::UserDeletionSummary, ServerFnError> {
    let calendars: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM calendars WHERE username = ?")
        .bind(username)
        .fetch_one(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let notes: i64 = sqlx::query_scalar(
        "SELECT COUNT(*) FROM notes WHERE author_username = ? \
         OR collection_id IN (SELECT id FROM note_collections WHERE owner_username = ?)",
    )
    .bind(username)
    .bind(username)
    .fetch_one(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let collections: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM note_collections WHERE owner_username = ?")
            .bind(username)
            .fetch_one(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let shares_owned: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM calendar_shares WHERE owner_username = ?")
            .bind(username)
            .fetch_one(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let app_tokens: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM app_tokens WHERE username = ?")
        .bind(username)
        .fetch_one(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    Ok(super::UserDeletionSummary {
        calendars,
        notes,
        collections,
        shares_owned,
        app_tokens,
    })
}

/// Permanently delete `username` and every table row it owns, in one
/// transaction. See `backlog/52-admin-user-management-ui.md` for the full
/// ordered rationale — this list is cross-checked against `USERNAME_COLUMNS`
/// in `profile.rs`.
#[cfg(feature = "ssr")]
async fn delete_user_tx(pool: &sqlx::SqlitePool, username: &str) -> Result<(), ServerFnError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    // Not owned by this user, but may point at them as a write identity for
    // a read-only calendar someone else owns — null it out rather than
    // deleting someone else's calendar.
    sqlx::query("UPDATE calendars SET sync_user = NULL WHERE sync_user = ?")
        .bind(username)
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    for sql in [
        "DELETE FROM events WHERE username = ?",
        "DELETE FROM calendars WHERE username = ?",
        "DELETE FROM sync_log WHERE username = ?",
        "DELETE FROM properties WHERE username = ?",
        "DELETE FROM locks WHERE username = ?",
    ] {
        sqlx::query(sql)
            .bind(username)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }

    sqlx::query("DELETE FROM calendar_shares WHERE owner_username = ? OR grantee_username = ?")
        .bind(username)
        .bind(username)
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    sqlx::query("DELETE FROM share_invitations WHERE owner_username = ?")
        .bind(username)
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    // Notes/members inside collections this user *owns* must go before the
    // collections themselves — the subquery needs them to still exist.
    sqlx::query(
        "DELETE FROM notes WHERE collection_id IN \
         (SELECT id FROM note_collections WHERE owner_username = ?)",
    )
    .bind(username)
    .execute(&mut *tx)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    sqlx::query(
        "DELETE FROM note_collection_members WHERE collection_id IN \
         (SELECT id FROM note_collections WHERE owner_username = ?)",
    )
    .bind(username)
    .execute(&mut *tx)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    sqlx::query("DELETE FROM note_collection_members WHERE member_username = ?")
        .bind(username)
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    sqlx::query("DELETE FROM notes WHERE author_username = ?")
        .bind(username)
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    sqlx::query("DELETE FROM note_collections WHERE owner_username = ?")
        .bind(username)
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    for sql in [
        "DELETE FROM user_preferences WHERE username = ?",
        "DELETE FROM app_tokens WHERE username = ?",
        "DELETE FROM remote_caldav_sources WHERE username = ?",
        "DELETE FROM inbound_email_tokens WHERE username = ?",
        "DELETE FROM trusted_sender_addresses WHERE username = ?",
        "DELETE FROM sessions WHERE username = ?",
        "DELETE FROM email_verifications WHERE username = ?",
        "DELETE FROM email_change_requests WHERE username = ?",
        "DELETE FROM markdown_sources WHERE username = ?",
        "DELETE FROM done_items WHERE username = ?",
        "DELETE FROM done_scopes WHERE username = ?",
        "DELETE FROM done_settings WHERE username = ?",
    ] {
        sqlx::query(sql)
            .bind(username)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }

    let affected = sqlx::query("DELETE FROM users WHERE username = ?")
        .bind(username)
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
        .rows_affected();

    if affected == 0 {
        return Err(ServerFnError::new("User not found"));
    }

    tx.commit()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    Ok(())
}

/// Return counts of data `username` still owns, for the delete-confirmation
/// screen.
#[allow(unused_variables)]
#[server]
pub async fn get_user_deletion_summary(
    username: String,
) -> Result<super::UserDeletionSummary, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, _acting) = auth_admin().await?;
        return deletion_summary_core(&pool, &username).await;
    }
    #[allow(unreachable_code)]
    Ok(super::UserDeletionSummary {
        calendars: 0,
        notes: 0,
        collections: 0,
        shares_owned: 0,
        app_tokens: 0,
    })
}

/// Permanently delete a user account and everything it owns. Requires typing
/// the username back as `confirm_username` (defense in depth — the client
/// already gates the button on this match). Rejects targeting the caller's
/// own account.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn admin_delete_user(
    username: String,
    confirm_username: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, acting) = auth_admin().await?;
        if username == acting {
            return Err(ServerFnError::new("You can't delete your own account"));
        }
        if confirm_username.trim() != username {
            return Err(ServerFnError::new(
                "Confirmation text doesn't match the username",
            ));
        }
        delete_user_tx(&pool, &username).await?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use sqlx::SqlitePool;

    async fn create_test_user(
        pool: &SqlitePool,
        username: &str,
        email: &str,
        is_admin: bool,
        is_active: bool,
    ) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin, is_active) \
             VALUES (?, ?, ?, 'hash', ?, ?)",
        )
        .bind(username)
        .bind(email)
        .bind(username)
        .bind(is_admin as i64)
        .bind(is_active as i64)
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn lists_all_users_ordered_by_username_including_inactive(pool: SqlitePool) {
        create_test_user(&pool, "bob", "bob@example.com", false, true).await;
        create_test_user(&pool, "alice", "alice@example.com", true, false).await;

        let rows: Vec<(String, i64, i64)> =
            sqlx::query_as("SELECT username, is_admin, is_active FROM users ORDER BY username")
                .fetch_all(&pool)
                .await
                .unwrap();

        assert_eq!(
            rows,
            vec![("alice".to_string(), 1, 0), ("bob".to_string(), 0, 1),]
        );
    }

    async fn create_user_admin_core(
        pool: &SqlitePool,
        username: &str,
        email: &str,
        display_name: &str,
        password: &str,
    ) -> Result<(), leptos::server_fn::ServerFnError> {
        super::create_user_admin_tx(pool, username, email, display_name, password).await
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn create_user_admin_rejects_invalid_slug(pool: SqlitePool) {
        let err = create_user_admin_core(&pool, "a", "a@example.com", "A", "password123")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("2\u{2013}50 characters"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn create_user_admin_rejects_short_password(pool: SqlitePool) {
        let err = create_user_admin_core(&pool, "carol", "carol@example.com", "Carol", "short")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("8 characters"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn create_user_admin_rejects_duplicate_username(pool: SqlitePool) {
        create_test_user(&pool, "carol", "carol@example.com", false, true).await;
        let err =
            create_user_admin_core(&pool, "carol", "other@example.com", "Carol2", "password123")
                .await
                .unwrap_err();
        assert!(err.to_string().contains("username"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn create_user_admin_rejects_duplicate_email(pool: SqlitePool) {
        create_test_user(&pool, "carol", "carol@example.com", false, true).await;
        let err = create_user_admin_core(
            &pool,
            "carol2",
            "carol@example.com",
            "Carol2",
            "password123",
        )
        .await
        .unwrap_err();
        assert!(err.to_string().contains("email"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn create_user_admin_creates_active_non_admin_user(pool: SqlitePool) {
        create_user_admin_core(&pool, "dave", "dave@example.com", "Dave", "password123")
            .await
            .unwrap();

        let row: (i64, i64) =
            sqlx::query_as("SELECT is_admin, is_active FROM users WHERE username = \'dave\'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(row, (0, 1));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn set_user_admin_tx_toggles_flag(pool: SqlitePool) {
        create_test_user(&pool, "erin", "erin@example.com", false, true).await;
        super::set_user_admin_tx(&pool, "erin", true).await.unwrap();
        let is_admin: i64 =
            sqlx::query_scalar("SELECT is_admin FROM users WHERE username = \'erin\'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(is_admin, 1);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn set_user_admin_tx_rejects_unknown_user(pool: SqlitePool) {
        let err = super::set_user_admin_tx(&pool, "nobody", true)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn set_user_active_tx_toggles_flag(pool: SqlitePool) {
        create_test_user(&pool, "frank", "frank@example.com", false, true).await;
        super::set_user_active_tx(&pool, "frank", false)
            .await
            .unwrap();
        let is_active: i64 =
            sqlx::query_scalar("SELECT is_active FROM users WHERE username = \'frank\'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(is_active, 0);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reset_password_core_rejects_short_password(pool: SqlitePool) {
        create_test_user(&pool, "grace", "grace@example.com", false, true).await;
        let err = super::reset_password_core(&pool, "grace", "short")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("8 characters"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reset_password_core_rejects_unknown_user(pool: SqlitePool) {
        let err = super::reset_password_core(&pool, "nobody", "password123")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reset_password_core_revokes_all_sessions_for_target(pool: SqlitePool) {
        create_test_user(&pool, "grace", "grace@example.com", false, true).await;
        sqlx::query(
            "INSERT INTO sessions (token, username, expires_at) \
             VALUES (\'tok-1\', \'grace\', datetime(\'now\', \'+1 day\'))",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sessions (token, username, expires_at) \
             VALUES (\'tok-2\', \'grace\', datetime(\'now\', \'+1 day\'))",
        )
        .execute(&pool)
        .await
        .unwrap();

        super::reset_password_core(&pool, "grace", "newpassword123")
            .await
            .unwrap();

        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE username = \'grace\'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(remaining, 0);

        let new_hash: String =
            sqlx::query_scalar("SELECT password_hash FROM users WHERE username = \'grace\'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(super::super::auth::verify_password_ssr(
            "newpassword123",
            &new_hash
        ));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn deletion_summary_counts_owned_resources(pool: SqlitePool) {
        create_test_user(&pool, "henry", "henry@example.com", false, true).await;
        sqlx::query("INSERT INTO calendars (username, name) VALUES (\'henry\', \'personal\')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO note_collections (owner_username, name, color) VALUES (\'henry\', \'inbox\', \'#fff\')",
        )
        .execute(&pool).await.unwrap();
        let collection_id: i64 =
            sqlx::query_scalar("SELECT id FROM note_collections WHERE owner_username = \'henry\'")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query("INSERT INTO notes (collection_id, author_username, text) VALUES (?, \'henry\', \'hi\')")
            .bind(collection_id)
            .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO calendar_shares (owner_username, calendar_name, grantee_username, permission) \
             VALUES (\'henry\', \'personal\', \'iris\', \'read\')",
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO app_tokens (id, username, name, token_hash) VALUES (\'tok\', \'henry\', \'laptop\', \'h\')",
        )
        .execute(&pool).await.unwrap();

        let summary = super::deletion_summary_core(&pool, "henry").await.unwrap();
        assert_eq!(summary.calendars, 1);
        assert_eq!(summary.notes, 1);
        assert_eq!(summary.collections, 1);
        assert_eq!(summary.shares_owned, 1);
        assert_eq!(summary.app_tokens, 1);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn delete_user_tx_cascades_and_preserves_other_members_data(pool: SqlitePool) {
        create_test_user(&pool, "henry", "henry@example.com", false, true).await;
        create_test_user(&pool, "iris", "iris@example.com", false, true).await;

        // henry owns a calendar with an event, and a note collection shared
        // with iris; iris authored a note in henry\'s collection.
        sqlx::query("INSERT INTO calendars (username, name) VALUES (\'henry\', \'personal\')")
            .execute(&pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO events (username, calendar, uid, type, etag, ical_data) \
             VALUES (\'henry\', \'personal\', \'uid-1\', \'VEVENT\', \'etag-1\', \'ICAL\')",
        )
        .execute(&pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO note_collections (owner_username, name, color) VALUES (\'henry\', \'shared\', \'#fff\')",
        )
        .execute(&pool).await.unwrap();
        let henry_collection: i64 =
            sqlx::query_scalar("SELECT id FROM note_collections WHERE owner_username = \'henry\'")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query(
            "INSERT INTO note_collection_members (collection_id, member_username) VALUES (?, \'iris\')",
        )
        .bind(henry_collection)
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO notes (collection_id, author_username, text) VALUES (?, \'iris\', \'note from iris\')",
        )
        .bind(henry_collection)
        .execute(&pool).await.unwrap();

        // iris also owns her own collection, unrelated to henry, with her
        // own note — this must survive untouched.
        sqlx::query(
            "INSERT INTO note_collections (owner_username, name, color) VALUES (\'iris\', \'personal\', \'#000\')",
        )
        .execute(&pool).await.unwrap();
        let iris_collection: i64 =
            sqlx::query_scalar("SELECT id FROM note_collections WHERE owner_username = \'iris\'")
                .fetch_one(&pool)
                .await
                .unwrap();
        sqlx::query(
            "INSERT INTO notes (collection_id, author_username, text) VALUES (?, \'iris\', \'iris own note\')",
        )
        .bind(iris_collection)
        .execute(&pool).await.unwrap();

        sqlx::query(
            "INSERT INTO calendar_shares (owner_username, calendar_name, grantee_username, permission) \
             VALUES (\'henry\', \'personal\', \'iris\', \'read\')",
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO app_tokens (id, username, name, token_hash) VALUES (\'tok\', \'henry\', \'laptop\', \'h\')",
        )
        .execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO sessions (token, username, expires_at) \
             VALUES (\'sess-henry\', \'henry\', datetime(\'now\', \'+1 day\'))",
        )
        .execute(&pool)
        .await
        .unwrap();

        super::delete_user_tx(&pool, "henry").await.unwrap();

        // henry is fully gone.
        let henry_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE username = \'henry\'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(henry_count, 0);
        let cal_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM calendars WHERE username = \'henry\'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(cal_count, 0);
        let event_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE username = \'henry\'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(event_count, 0);
        let henry_collection_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM note_collections WHERE owner_username = \'henry\'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(henry_collection_count, 0);
        let iris_note_in_henry_collection: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM notes WHERE collection_id = ?")
                .bind(henry_collection)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(iris_note_in_henry_collection, 0);
        let share_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM calendar_shares WHERE owner_username = \'henry\' OR grantee_username = \'henry\'",
        )
        .fetch_one(&pool).await.unwrap();
        assert_eq!(share_count, 0);
        let token_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM app_tokens WHERE username = \'henry\'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(token_count, 0);
        let session_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE username = \'henry\'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(session_count, 0);

        // iris and her own collection/note survive untouched.
        let iris_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE username = \'iris\'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(iris_count, 1);
        let iris_collection_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM note_collections WHERE owner_username = \'iris\'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(iris_collection_count, 1);
        let iris_note_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM notes WHERE collection_id = ?")
                .bind(iris_collection)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(iris_note_count, 1);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn delete_user_tx_nulls_sync_user_on_other_owned_calendars(pool: SqlitePool) {
        create_test_user(&pool, "henry", "henry@example.com", false, true).await;
        create_test_user(&pool, "iris", "iris@example.com", false, true).await;
        sqlx::query(
            "INSERT INTO calendars (username, name, sync_user) VALUES (\'iris\', \'shared-in\', \'henry\')",
        )
        .execute(&pool).await.unwrap();

        super::delete_user_tx(&pool, "henry").await.unwrap();

        let sync_user: Option<String> = sqlx::query_scalar(
            "SELECT sync_user FROM calendars WHERE username = \'iris\' AND name = \'shared-in\'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(sync_user, None);
    }
}
