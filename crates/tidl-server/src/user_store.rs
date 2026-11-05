//! Database-backed user accounts.
//!
//! [`UserStore`] is the single point of contact for all user operations:
//! creation, lookup, password hashing/verification, and admin management.
//!
//! ## Password hashing
//!
//! New passwords are hashed with **argon2id** (PHC string format).  Legacy
//! bcrypt hashes (`$2b$`/`$2y$`) produced by the old config-file path are
//! still accepted so that existing deployments can migrate gradually; the hash
//! is replaced with argon2id on the next explicit password change.
//!
//! ## Identity model
//!
//! | Field        | Role |
//! |-------------|------|
//! | `email`     | Login identifier (case-insensitive, stored lowercase) |
//! | `username`  | Short slug for CalDAV paths (`/caldav/{username}/…`) |
//!
//! CalDAV `Authorization: Basic` headers may contain either the email or the
//! username; the extractor in `auth.rs` tries both.

use anyhow::{bail, Result};
use argon2::{
    password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString},
    Argon2,
};
use rand::RngCore;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};

// ── Data structures ───────────────────────────────────────────────────────────

/// A named app token entry as returned from the database (token itself is never returned).
#[derive(Debug, Clone)]
pub struct AppTokenRow {
    pub id: String,
    pub username: String,
    pub name: String,
    pub read_only: bool,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

/// An inbound email token entry as returned from the database (token itself is never returned).
#[derive(Debug, Clone)]
pub struct InboundEmailTokenRow {
    pub id: String,
    pub label: String,
    pub calendar_id: i64,
    pub calendar_name: String,
    pub address: String,
    pub created_at: String,
    pub last_used_at: Option<String>,
}

/// A trusted sender address entry as returned from the database.
#[derive(Debug, Clone)]
pub struct TrustedSenderRow {
    pub id: String,
    pub email: String,
    pub created_at: String,
}

/// A user row as returned from the database.
#[derive(Debug, Clone)]
pub struct DbUser {
    pub id: i64,
    pub username: String,
    pub email: String,
    pub display_name: String,
    pub password_hash: String,
    pub is_admin: bool,
    pub is_active: bool,
}

// ── UserStore ─────────────────────────────────────────────────────────────────

/// Thin wrapper around the SQLite pool that owns all user-related queries.
#[derive(Clone)]
pub struct UserStore {
    pool: SqlitePool,
}

impl UserStore {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    // ── Password helpers ──────────────────────────────────────────────────────

    /// Hash a plaintext password with argon2id.  Returns the PHC string.
    pub fn hash_password(password: &str) -> Result<String> {
        let salt = SaltString::generate(&mut OsRng);
        Ok(Argon2::default()
            .hash_password(password.as_bytes(), &salt)?
            .to_string())
    }

    /// Verify a plaintext password against a stored hash.
    ///
    /// Accepts:
    /// - **argon2id** PHC strings (new default)
    /// - **bcrypt** hashes starting with `$2b$`, `$2y$`, or `$2a$` (legacy
    ///   migration bridge — valid until the user changes their password)
    pub fn verify_password(password: &str, hash: &str) -> bool {
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

    // ── Write operations ──────────────────────────────────────────────────────

    /// Create a new user. Fails if `email` or `username` already exists.
    pub async fn create_user(
        &self,
        username: &str,
        email: &str,
        display_name: &str,
        password_hash: &str,
        is_admin: bool,
    ) -> Result<DbUser> {
        let email_lc = email.to_lowercase();
        let id = sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(username)
        .bind(&email_lc)
        .bind(display_name)
        .bind(password_hash)
        .bind(is_admin as i64)
        .execute(&self.pool)
        .await?
        .last_insert_rowid();

        Ok(DbUser {
            id,
            username: username.to_string(),
            email: email_lc,
            display_name: display_name.to_string(),
            password_hash: password_hash.to_string(),
            is_admin,
            is_active: true,
        })
    }

    /// Create a user unless one with the same email or username already
    /// exists. Returns `Ok(true)` when the account was created and
    /// `Ok(false)` when an existing account was left untouched.
    ///
    /// Used for `[[users]]` entries in `config.toml`: never overwrites an
    /// existing password or admin flag, so UI-made changes survive restarts.
    pub async fn ensure_user(
        &self,
        username: &str,
        email: &str,
        display_name: &str,
        password_hash: &str,
        is_admin: bool,
    ) -> Result<bool> {
        let email_lc = email.to_lowercase();
        let existing: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM users WHERE email = ? OR username = ? LIMIT 1",
        )
        .bind(&email_lc)
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;

        if existing.is_some() {
            return Ok(false);
        }

        self.create_user(username, email, display_name, password_hash, is_admin)
            .await?;
        Ok(true)
    }

    /// Enable or disable a user account.
    pub async fn set_active(&self, username: &str, is_active: bool) -> Result<()> {
        let affected = sqlx::query(
            "UPDATE users SET is_active = ?, updated_at = CURRENT_TIMESTAMP
             WHERE username = ?",
        )
        .bind(is_active as i64)
        .bind(username)
        .execute(&self.pool)
        .await?
        .rows_affected();

        if affected == 0 {
            bail!("user '{}' not found", username);
        }
        Ok(())
    }

    /// Grant or revoke admin rights.
    pub async fn set_admin(&self, username: &str, is_admin: bool) -> Result<()> {
        let affected = sqlx::query(
            "UPDATE users SET is_admin = ?, updated_at = CURRENT_TIMESTAMP
             WHERE username = ?",
        )
        .bind(is_admin as i64)
        .bind(username)
        .execute(&self.pool)
        .await?
        .rows_affected();

        if affected == 0 {
            bail!("user '{}' not found", username);
        }
        Ok(())
    }

    /// Replace a user's stored password hash (use [`hash_password`] first).
    pub async fn set_password_hash(&self, username: &str, hash: &str) -> Result<()> {
        sqlx::query(
            "UPDATE users SET password_hash = ?, updated_at = CURRENT_TIMESTAMP
             WHERE username = ?",
        )
        .bind(hash)
        .bind(username)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    // ── Read operations ───────────────────────────────────────────────────────

    /// Find an **active** user by email (case-insensitive).
    /// Returns `None` if not found or disabled.
    pub async fn find_active_by_email(&self, email: &str) -> Result<Option<DbUser>> {
        let email_lc = email.to_lowercase();
        let row = sqlx::query(
            "SELECT id, username, email, display_name, password_hash, is_admin, is_active
             FROM users WHERE email = ? AND is_active = 1",
        )
        .bind(&email_lc)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(row_to_user))
    }

    /// Find an **active** user by CalDAV username slug.
    /// Returns `None` if not found or disabled.
    pub async fn find_active_by_username(&self, username: &str) -> Result<Option<DbUser>> {
        let row = sqlx::query(
            "SELECT id, username, email, display_name, password_hash, is_admin, is_active
             FROM users WHERE username = ? AND is_active = 1",
        )
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(row_to_user))
    }

    /// Return all users ordered by username (includes inactive — admin view).
    pub async fn list_users(&self) -> Result<Vec<DbUser>> {
        let rows = sqlx::query(
            "SELECT id, username, email, display_name, password_hash, is_admin, is_active
             FROM users ORDER BY username",
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows.into_iter().map(row_to_user).collect())
    }

    /// Returns `true` if no users exist yet (bootstrap guard).
    pub async fn is_empty(&self) -> Result<bool> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
            .fetch_one(&self.pool)
            .await?;
        Ok(count == 0)
    }

    // ── App token operations ──────────────────────────────────────────────────

    /// Generate a new app token for `username`, store its hash, and return the
    /// raw token string (only opportunity to see it).
    ///
    /// Token format: `tidl_<64 hex chars>` (32 random bytes).
    /// The stored hash is SHA-256 of the raw token string (hex-encoded).
    pub async fn create_app_token(
        &self,
        username: &str,
        name: &str,
        read_only: bool,
    ) -> Result<String> {
        let mut raw_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut raw_bytes);
        let token = format!("tidl_{}", hex::encode(raw_bytes));

        let hash = Self::hash_token(&token);
        let id = uuid::Uuid::new_v4().to_string();

        sqlx::query(
            "INSERT INTO app_tokens (id, username, name, token_hash, read_only)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(username)
        .bind(name)
        .bind(&hash)
        .bind(read_only as i64)
        .execute(&self.pool)
        .await?;

        Ok(token)
    }

    /// List all app tokens for `username` (token hashes are not returned).
    pub async fn list_app_tokens(&self, username: &str) -> Result<Vec<AppTokenRow>> {
        let rows = sqlx::query(
            "SELECT id, username, name, read_only, created_at, last_used_at
             FROM app_tokens WHERE username = ? ORDER BY created_at DESC",
        )
        .bind(username)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let read_only: i64 = r.get("read_only");
                AppTokenRow {
                    id: r.get("id"),
                    username: r.get("username"),
                    name: r.get("name"),
                    read_only: read_only != 0,
                    created_at: r.get("created_at"),
                    last_used_at: r.try_get("last_used_at").ok().flatten(),
                }
            })
            .collect())
    }

    /// Delete an app token by ID, verifying ownership.
    pub async fn revoke_app_token(&self, username: &str, token_id: &str) -> Result<()> {
        let affected = sqlx::query("DELETE FROM app_tokens WHERE id = ? AND username = ?")
            .bind(token_id)
            .bind(username)
            .execute(&self.pool)
            .await?
            .rows_affected();

        if affected == 0 {
            bail!("token not found or not owned by user");
        }
        Ok(())
    }

    /// Validate a raw token string. Returns the owning username if valid
    /// (active user, token not expired), or `None` on failure.
    ///
    /// Updates `last_used_at` on success.
    pub async fn validate_app_token(&self, raw_token: &str) -> Result<Option<(String, bool)>> {
        let hash = Self::hash_token(raw_token);
        let row = sqlx::query(
            "SELECT t.username, t.read_only
             FROM app_tokens t
             JOIN users u ON u.username = t.username
             WHERE t.token_hash = ?
               AND u.is_active = 1
               AND (t.expires_at IS NULL OR t.expires_at > CURRENT_TIMESTAMP)",
        )
        .bind(&hash)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(r) = row {
            let username: String = r.get("username");
            let read_only: i64 = r.get("read_only");

            // Fire-and-forget last_used_at update.
            let _ = sqlx::query(
                "UPDATE app_tokens SET last_used_at = CURRENT_TIMESTAMP WHERE token_hash = ?",
            )
            .bind(&hash)
            .execute(&self.pool)
            .await;

            Ok(Some((username, read_only != 0)))
        } else {
            Ok(None)
        }
    }

    fn hash_token(raw: &str) -> String {
        let mut h = Sha256::new();
        h.update(raw.as_bytes());
        hex::encode(h.finalize())
    }

    // ── Inbound email token operations ────────────────────────────────────────

    /// Generate a new inbound email token for `username` and `calendar_id`.
    ///
    /// Token format: `tidl_<64 hex chars>` (32 random bytes).
    /// Address format: `<raw_token>@<domain>`.
    /// Returns `(raw_token, full_address)` — the only opportunity to see the token.
    pub async fn create_inbound_email_token(
        &self,
        username: &str,
        calendar_id: i64,
        label: &str,
        domain: &str,
    ) -> Result<(String, String)> {
        let mut raw_bytes = [0u8; 32];
        OsRng.fill_bytes(&mut raw_bytes);
        let token = format!("tidl_{}", hex::encode(raw_bytes));

        let token_hash = Self::hash_token(&token);
        let id = uuid::Uuid::new_v4().to_string();
        let address = format!("{}@{}", token, domain);

        sqlx::query(
            "INSERT INTO inbound_email_tokens (id, username, calendar_id, label, token_hash, address)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(username)
        .bind(calendar_id)
        .bind(label)
        .bind(&token_hash)
        .bind(&address)
        .execute(&self.pool)
        .await?;

        Ok((token, address))
    }

    /// List all inbound email tokens for `username`, joined with calendar name.
    pub async fn list_inbound_email_tokens(
        &self,
        username: &str,
    ) -> Result<Vec<InboundEmailTokenRow>> {
        let rows = sqlx::query(
            "SELECT t.id, t.label, t.calendar_id, c.name AS calendar_name,
                    t.address, t.created_at, t.last_used_at
             FROM inbound_email_tokens t
             JOIN calendars c ON c.id = t.calendar_id
             WHERE t.username = ?
             ORDER BY t.created_at DESC",
        )
        .bind(username)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| InboundEmailTokenRow {
                id: r.get("id"),
                label: r.get("label"),
                calendar_id: r.get("calendar_id"),
                calendar_name: r.get("calendar_name"),
                address: r.get("address"),
                created_at: r.get("created_at"),
                last_used_at: r.try_get("last_used_at").ok().flatten(),
            })
            .collect())
    }

    /// Delete an inbound email token by ID, verifying ownership.
    pub async fn revoke_inbound_email_token(&self, username: &str, id: &str) -> Result<()> {
        let affected =
            sqlx::query("DELETE FROM inbound_email_tokens WHERE id = ? AND username = ?")
                .bind(id)
                .bind(username)
                .execute(&self.pool)
                .await?
                .rows_affected();

        if affected == 0 {
            bail!("inbound email token not found or not owned by user");
        }
        Ok(())
    }

    /// Validate a raw inbound email token. Returns `(username, calendar_id)` if valid,
    /// or `None` if the token is not found or the owning user is inactive.
    ///
    /// Updates `last_used_at` on success.
    pub async fn validate_inbound_email_token(
        &self,
        raw_token: &str,
    ) -> Result<Option<(String, i64)>> {
        let hash = Self::hash_token(raw_token);
        let row = sqlx::query(
            "SELECT t.username, t.calendar_id
             FROM inbound_email_tokens t
             JOIN users u ON u.username = t.username
             WHERE t.token_hash = ?
               AND u.is_active = 1",
        )
        .bind(&hash)
        .fetch_optional(&self.pool)
        .await?;

        if let Some(r) = row {
            let username: String = r.get("username");
            let calendar_id: i64 = r.get("calendar_id");

            // Fire-and-forget last_used_at update.
            let _ = sqlx::query(
                "UPDATE inbound_email_tokens SET last_used_at = CURRENT_TIMESTAMP
                 WHERE token_hash = ?",
            )
            .bind(&hash)
            .execute(&self.pool)
            .await;

            Ok(Some((username, calendar_id)))
        } else {
            Ok(None)
        }
    }

    // ── Trusted sender operations ─────────────────────────────────────────────

    /// Add a trusted sender email address for `username`. No-ops if already present.
    pub async fn add_trusted_sender(&self, username: &str, email: &str) -> Result<()> {
        let id = uuid::Uuid::new_v4().to_string();
        sqlx::query(
            "INSERT OR IGNORE INTO trusted_sender_addresses (id, username, email)
             VALUES (?, ?, ?)",
        )
        .bind(&id)
        .bind(username)
        .bind(email)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Remove a trusted sender address by ID, verifying ownership.
    pub async fn remove_trusted_sender(&self, username: &str, id: &str) -> Result<()> {
        let affected =
            sqlx::query("DELETE FROM trusted_sender_addresses WHERE id = ? AND username = ?")
                .bind(id)
                .bind(username)
                .execute(&self.pool)
                .await?
                .rows_affected();

        if affected == 0 {
            bail!("trusted sender not found or not owned by user");
        }
        Ok(())
    }

    /// List all trusted sender addresses for `username`, ordered by `created_at` ascending.
    pub async fn list_trusted_senders(&self, username: &str) -> Result<Vec<TrustedSenderRow>> {
        let rows = sqlx::query(
            "SELECT id, email, created_at
             FROM trusted_sender_addresses
             WHERE username = ?
             ORDER BY created_at ASC",
        )
        .bind(username)
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| TrustedSenderRow {
                id: r.get("id"),
                email: r.get("email"),
                created_at: r.get("created_at"),
            })
            .collect())
    }

    /// Returns `true` if `email` is in the trusted sender list for `username`.
    pub async fn is_trusted_sender(&self, username: &str, email: &str) -> Result<bool> {
        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM trusted_sender_addresses WHERE username = ? AND email = ?",
        )
        .bind(username)
        .bind(email)
        .fetch_one(&self.pool)
        .await?;
        Ok(count > 0)
    }

    // ── OIDC operations ───────────────────────────────────────────────────────

    /// Find an active user by their OIDC subject + issuer pair.
    pub async fn find_by_oidc_subject(&self, sub: &str, issuer: &str) -> Result<Option<DbUser>> {
        let row = sqlx::query(
            "SELECT id, username, email, display_name, password_hash, is_admin, is_active
             FROM users WHERE oidc_subject = ? AND oidc_issuer = ? AND is_active = 1",
        )
        .bind(sub)
        .bind(issuer)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(row_to_user))
    }

    /// Write the OIDC subject + issuer onto an existing user row.
    pub async fn set_oidc_identity(&self, username: &str, sub: &str, issuer: &str) -> Result<()> {
        let affected = sqlx::query(
            "UPDATE users SET oidc_subject = ?, oidc_issuer = ?, updated_at = CURRENT_TIMESTAMP
             WHERE username = ?",
        )
        .bind(sub)
        .bind(issuer)
        .bind(username)
        .execute(&self.pool)
        .await?
        .rows_affected();

        if affected == 0 {
            bail!("user '{}' not found", username);
        }
        Ok(())
    }

    /// Create a new user that authenticates exclusively via OIDC.
    ///
    /// The password_hash sentinel "oidc_only" can never match `verify_password`.
    pub async fn create_oidc_user(
        &self,
        username: &str,
        email: &str,
        display_name: &str,
        sub: &str,
        issuer: &str,
    ) -> Result<DbUser> {
        let email_lc = email.to_lowercase();
        let id = sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin, oidc_subject, oidc_issuer)
             VALUES (?, ?, ?, 'oidc_only', 0, ?, ?)",
        )
        .bind(username)
        .bind(&email_lc)
        .bind(display_name)
        .bind(sub)
        .bind(issuer)
        .execute(&self.pool)
        .await?
        .last_insert_rowid();

        Ok(DbUser {
            id,
            username: username.to_string(),
            email: email_lc,
            display_name: display_name.to_string(),
            password_hash: "oidc_only".to_string(),
            is_admin: false,
            is_active: true,
        })
    }

    /// Return `base` if no user with that username exists; otherwise append
    /// incrementing numeric suffixes until a free slot is found.
    pub async fn generate_unique_username(&self, base: &str) -> Result<String> {
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE username = ?")
            .bind(base)
            .fetch_one(&self.pool)
            .await?;
        if count == 0 {
            return Ok(base.to_string());
        }
        for n in 1u32.. {
            let candidate = format!("{}{}", base, n);
            let c: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users WHERE username = ?")
                .bind(&candidate)
                .fetch_one(&self.pool)
                .await?;
            if c == 0 {
                return Ok(candidate);
            }
        }
        bail!("could not find a free username slot for base '{}'", base);
    }

    /// Expose the underlying pool (e.g. for Axum state extraction).
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn row_to_user(r: sqlx::sqlite::SqliteRow) -> DbUser {
    let is_admin: i64 = r.get("is_admin");
    let is_active: i64 = r.get("is_active");
    DbUser {
        id: r.get("id"),
        username: r.get("username"),
        email: r.get("email"),
        display_name: r.get("display_name"),
        password_hash: r.get("password_hash"),
        is_admin: is_admin != 0,
        is_active: is_active != 0,
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    #[sqlx::test(migrations = "../../migrations")]
    async fn find_by_oidc_subject_returns_user(pool: SqlitePool) {
        let store = UserStore::new(pool);
        let hash = UserStore::hash_password("pw").unwrap();
        store
            .create_user("alice", "alice@example.com", "Alice", &hash, false)
            .await
            .unwrap();
        store
            .set_oidc_identity("alice", "sub-123", "https://idp.example.com")
            .await
            .unwrap();

        let user = store
            .find_by_oidc_subject("sub-123", "https://idp.example.com")
            .await
            .unwrap();
        assert_eq!(user.unwrap().username, "alice");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn find_by_oidc_subject_returns_none_for_unknown(pool: SqlitePool) {
        let store = UserStore::new(pool);
        let result = store
            .find_by_oidc_subject("nope", "https://idp.example.com")
            .await
            .unwrap();
        assert!(result.is_none());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn create_oidc_user_creates_active_user(pool: SqlitePool) {
        let store = UserStore::new(pool);
        let user = store
            .create_oidc_user(
                "bob",
                "bob@example.com",
                "Bob",
                "sub-bob",
                "https://idp.example.com",
            )
            .await
            .unwrap();
        assert_eq!(user.username, "bob");
        assert!(user.is_active);
        assert!(!user.is_admin);
        // Verify it can be found by OIDC subject
        let found = store
            .find_by_oidc_subject("sub-bob", "https://idp.example.com")
            .await
            .unwrap();
        assert!(found.is_some());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn generate_unique_username_deduplicates(pool: SqlitePool) {
        let store = UserStore::new(pool);
        let hash = UserStore::hash_password("pw").unwrap();
        store
            .create_user("john", "john@example.com", "John", &hash, false)
            .await
            .unwrap();

        let name = store.generate_unique_username("john").await.unwrap();
        assert_eq!(name, "john1");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn generate_unique_username_no_conflict(pool: SqlitePool) {
        let store = UserStore::new(pool);
        let name = store.generate_unique_username("alice").await.unwrap();
        assert_eq!(name, "alice");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn app_token_auth_matches_resolved_user(pool: SqlitePool) {
        let store = UserStore::new(pool);
        let hash = UserStore::hash_password("pw12345678").unwrap();
        store
            .create_user("alice", "alice@example.com", "Alice", &hash, false)
            .await
            .unwrap();
        let token = store
            .create_app_token("alice", "laptop", false)
            .await
            .unwrap();

        let user = store
            .find_active_by_username("alice")
            .await
            .unwrap()
            .unwrap();
        let (token_username, read_only) = store.validate_app_token(&token).await.unwrap().unwrap();
        assert_eq!(token_username, user.username);
        assert!(!read_only);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn ensure_user_creates_missing_account(pool: SqlitePool) {
        let store = UserStore::new(pool);
        let hash = UserStore::hash_password("pw12345678").unwrap();
        let created = store
            .ensure_user("alice", "Alice@Example.com", "Alice", &hash, true)
            .await
            .unwrap();
        assert!(created);

        let user = store
            .find_active_by_email("alice@example.com")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(user.username, "alice");
        assert!(user.is_admin);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn ensure_user_leaves_existing_account_untouched(pool: SqlitePool) {
        let store = UserStore::new(pool);
        let hash = UserStore::hash_password("pw12345678").unwrap();
        store
            .create_user("alice", "alice@example.com", "Alice", &hash, false)
            .await
            .unwrap();

        let created = store
            .ensure_user(
                "alice",
                "alice@example.com",
                "Different Name",
                &UserStore::hash_password("new-password").unwrap(),
                true,
            )
            .await
            .unwrap();
        assert!(!created);

        let user = store
            .find_active_by_username("alice")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(user.display_name, "Alice");
        assert!(!user.is_admin);
        assert!(UserStore::verify_password("pw12345678", &user.password_hash));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn ensure_user_matches_existing_by_username_too(pool: SqlitePool) {
        let store = UserStore::new(pool);
        let hash = UserStore::hash_password("pw12345678").unwrap();
        store
            .create_user("alice", "alice@example.com", "Alice", &hash, false)
            .await
            .unwrap();

        let created = store
            .ensure_user("alice", "other@example.com", "", &hash, false)
            .await
            .unwrap();
        assert!(!created);
    }
}
