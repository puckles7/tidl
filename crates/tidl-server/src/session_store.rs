//! Cookie-session persistence, separate from CalDAV calendar storage.

use anyhow::Result;
use sqlx::SqlitePool;

/// Thin wrapper around the SQLite pool that owns all session-related queries.
///
/// Session management is kept in its own struct so that `Storage` (which
/// handles CalDAV calendars, events, locks, and dead properties) does not
/// carry unrelated session responsibilities.
#[derive(Clone)]
pub struct SessionStore {
    pool: SqlitePool,
}

impl SessionStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    /// Create a new session token for the given user. Returns the token.
    pub async fn create_session(&self, username: &str, ttl_secs: i64) -> Result<String> {
        let token = uuid::Uuid::new_v4().to_string();
        sqlx::query!(
            "INSERT INTO sessions (token, username, expires_at)
             VALUES (?, ?, datetime('now', ? || ' seconds'))",
            token,
            username,
            ttl_secs,
        )
        .execute(&self.pool)
        .await?;
        Ok(token)
    }

    /// Look up the username for an active session token.
    pub async fn get_session_user(&self, token: &str) -> Result<Option<String>> {
        let row = sqlx::query!(
            "SELECT username FROM sessions WHERE token = ? AND expires_at > datetime('now')",
            token,
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.username))
    }

    /// Delete a session (logout).
    pub async fn delete_session(&self, token: &str) -> Result<()> {
        sqlx::query!("DELETE FROM sessions WHERE token = ?", token)
            .execute(&self.pool)
            .await?;
        Ok(())
    }

    /// Remove all expired sessions (intended for periodic cleanup).
    pub async fn delete_expired_sessions(&self) -> Result<()> {
        sqlx::query!("DELETE FROM sessions WHERE expires_at <= datetime('now')")
            .execute(&self.pool)
            .await?;
        Ok(())
    }
}
