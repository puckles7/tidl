//! Extension types injected by `tidl-server` and consumed by server functions.
//!
//! All types here are SSR-only (gated behind `#[cfg(feature = "ssr")]`).

// ── AllowRegistration ─────────────────────────────────────────────────────────

/// Whether self-registration is enabled on this server instance.
///
/// Injected by `tidl-server` as `axum::Extension(AllowRegistration(…))` and
/// extracted by server functions that gate sign-up behaviour.
#[derive(Clone)]
pub struct AllowRegistration(pub bool);

// ── ServerSecret ──────────────────────────────────────────────────────────────

/// Wraps the server-side secret string used to derive the AES key for
/// encrypting stored credentials (e.g. SMTP password).
///
/// Injected by `tidl-server` as `axum::Extension(ServerSecret(…))` and
/// extracted by server functions that need to encrypt or decrypt.
#[derive(Clone)]
pub struct ServerSecret(pub String);

// ── ServerBaseUrl ─────────────────────────────────────────────────────────────

/// The public base URL of the server, e.g. `"https://tidl.example.com"`.
/// Used to build invitation accept/decline links.
#[derive(Clone)]
pub struct ServerBaseUrl(pub String);

// ── AdminEmail ────────────────────────────────────────────────────────────────

/// Optional admin email address to notify when a new account is created.
///
/// Injected by `tidl-server` as `axum::Extension(AdminEmail(…))` and
/// extracted by `do_signup` to fire-and-forget an admin notification.
#[derive(Clone)]
pub struct AdminEmail(pub Option<String>);

// ── InboundDomain ─────────────────────────────────────────────────────────────

/// The domain used for inbound email token addresses, e.g. `"calendar.example.com"`.
///
/// Derived from `config.inbound_smtp.domain` when set, otherwise falls back to
/// `config.server.domain`.  Injected by `tidl-server` as an Axum Extension and
/// extracted by server functions that construct inbound email token addresses.
#[derive(Clone)]
pub struct InboundDomain(pub String);

// ── OidcProviderName ──────────────────────────────────────────────────────────

/// The display name of the configured OIDC provider (e.g. "Kanidm"), or `None`
/// when OIDC is disabled or discovery failed at startup.
///
/// Injected by `tidl-server` as `axum::Extension(OidcProviderName(…))` and
/// consumed by `get_oidc_config()` to tell the login page what to show.
#[derive(Clone)]
pub struct OidcProviderName(pub Option<String>);

// ── SessionCookieName ─────────────────────────────────────────────────────────

/// The `Set-Cookie`/`Cookie` name used for the session token, including the
/// configured port (e.g. `"session_3061"`).
///
/// HTTP cookies are scoped by host and path, but *not* by port (RFC 6265) —
/// so a bare `"session"` name causes two `tidl-server` instances bound to
/// different ports on the same hostname (e.g. two local dev instances on
/// `localhost:3061` and `localhost:3062`) to silently share one browser
/// cookie slot: logging into either overwrites the value the other relies
/// on (found 2026-07-21: this looked like "logging into one instance kills
/// my session in the other"). Baking the port into the name gives each
/// instance its own slot.
///
/// Injected by `tidl-server` as `axum::Extension(SessionCookieName(…))` and
/// used by every place that sets or reads the session cookie, both in
/// `tidl-web` server functions and in `tidl-server`'s own Axum handlers
/// (export/sharing links, the SSE stream, OIDC callback).
#[derive(Clone)]
pub struct SessionCookieName {
    pub name: String,
    /// Whether `Set-Cookie` headers built from this get the `Secure`
    /// attribute. Sourced from `server.secure_cookie` (default `true`) —
    /// see that field's doc comment for when to turn it off.
    pub secure: bool,
}

impl SessionCookieName {
    pub fn new(port: u16, secure: bool) -> Self {
        Self {
            name: format!("session_{port}"),
            secure,
        }
    }

    /// Extract the raw session token from a `Cookie` header value, if present.
    pub fn extract(&self, cookie_header: &str) -> Option<String> {
        let prefix = format!("{}=", self.name);
        cookie_header.split(';').find_map(|part| {
            part.trim()
                .strip_prefix(prefix.as_str())
                .map(|t| t.trim().to_string())
        })
    }

    fn secure_attr(&self) -> &'static str {
        if self.secure {
            "; Secure"
        } else {
            ""
        }
    }

    /// Build a `Set-Cookie` value for a new session token with the given TTL (seconds).
    pub fn set_header_value(&self, token: &str, ttl_secs: i64) -> String {
        format!(
            "{}={token}; HttpOnly; SameSite=Strict; Path=/; Max-Age={ttl_secs}{}",
            self.name,
            self.secure_attr()
        )
    }

    /// Build a `Set-Cookie` value that clears the session cookie immediately.
    pub fn clear_header_value(&self) -> String {
        format!(
            "{}=; HttpOnly; SameSite=Strict; Path=/; Max-Age=0{}",
            self.name,
            self.secure_attr()
        )
    }
}

#[cfg(test)]
mod session_cookie_name_tests {
    use super::SessionCookieName;

    #[test]
    fn different_ports_get_different_names() {
        assert_ne!(
            SessionCookieName::new(3061, true).name,
            SessionCookieName::new(3062, true).name
        );
    }

    #[test]
    fn extract_finds_own_cookie_among_others() {
        let name = SessionCookieName::new(3061, true);
        let header = "foo=bar; session_3061=abc123; other=xyz";
        assert_eq!(name.extract(header), Some("abc123".to_string()));
    }

    #[test]
    fn extract_ignores_a_different_ports_cookie() {
        // The exact bug this type fixes: a cookie set by the *other* instance
        // must not be picked up as this instance's session.
        let name_3061 = SessionCookieName::new(3061, true);
        let header_from_3062 =
            SessionCookieName::new(3062, true).set_header_value("othertoken", 86_400);
        assert_eq!(name_3061.extract(&header_from_3062), None);
    }

    #[test]
    fn set_header_value_round_trips_through_extract() {
        let name = SessionCookieName::new(3061, true);
        let header = name.set_header_value("mytoken", 86_400);
        assert_eq!(name.extract(&header), Some("mytoken".to_string()));
    }

    #[test]
    fn clear_header_value_has_no_token() {
        let name = SessionCookieName::new(3061, true);
        let header = name.clear_header_value();
        assert_eq!(name.extract(&header), Some(String::new()));
        assert!(header.contains("Max-Age=0"));
    }

    #[test]
    fn set_header_value_includes_secure_when_enabled() {
        let name = SessionCookieName::new(3061, true);
        assert!(name.set_header_value("tok", 86_400).contains("; Secure"));
    }

    #[test]
    fn set_header_value_omits_secure_when_disabled() {
        let name = SessionCookieName::new(3061, false);
        assert!(!name.set_header_value("tok", 86_400).contains("; Secure"));
    }

    #[test]
    fn clear_header_value_includes_secure_when_enabled() {
        let name = SessionCookieName::new(3061, true);
        assert!(name.clear_header_value().contains("; Secure"));
    }
}

// ── SyncTrigger ───────────────────────────────────────────────────────────────

/// Sender side of the remote-sync trigger channel.
///
/// Injected by `tidl-server` as `axum::Extension(SyncTrigger(…))`.  Server
/// functions that add a remote CalDAV source send the new source ID here to
/// wake the sync loop immediately instead of waiting up to 5 minutes.
#[cfg(feature = "ssr")]
#[derive(Clone)]
pub struct SyncTrigger(pub tokio::sync::mpsc::UnboundedSender<String>);

// ── MarkdownSyncTrigger ───────────────────────────────────────────────────────────────

/// Sender side of the markdown-sync trigger channel.
///
/// Injected by `tidl-server` as `axum::Extension(MarkdownSyncTrigger(…))`.
/// Server functions send a source ID here to wake the markdown sync loop
/// immediately instead of waiting up to 60 seconds.
#[cfg(feature = "ssr")]
#[derive(Clone)]
pub struct MarkdownSyncTrigger(pub tokio::sync::mpsc::UnboundedSender<i64>);

// ── EncryptionKey ─────────────────────────────────────────────────────────────

/// 32-byte AES-256-GCM key for encrypting/decrypting ad-hoc PATs.
/// None if TIDL_ENCRYPTION_KEY_FILE was not configured at startup.
#[cfg(feature = "ssr")]
#[derive(Clone)]
pub struct EncryptionKey(pub Option<std::sync::Arc<Vec<u8>>>);

// ── DataDir ───────────────────────────────────────────────────────────────────

/// The server's data directory path (config.paths.data_dir / TIDL_DATA_DIR).
/// Used to resolve git clone paths: data_dir/repos/<source_id>/
#[cfg(feature = "ssr")]
#[derive(Clone)]
pub struct DataDir(pub std::path::PathBuf);

// ── GitCommitPusher ───────────────────────────────────────────────────────────

/// Boxed closure that stages, commits, and pushes files in a git-backed source.
///
/// Wraps `Arc<dyn Fn(source_id, rel_paths) -> impl Future<Output = Result<(), String>>>`.
/// Using a concrete newtype avoids the higher-ranked lifetime errors that arise when
/// extracting raw `Arc<dyn Fn(...)>` type aliases from Axum extensions in server fns.
///
/// The implementation lives in `tidl-server` (which can use git2) and is injected
/// as an Axum Extension. `tidl-web` only uses the type — it never calls git2 directly.
#[cfg(feature = "ssr")]
type GitCommitPusherFn = dyn Fn(
        i64,         // source_id
        Vec<String>, // rel_paths (a missing path is staged as a deletion)
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>
    + Send
    + Sync;

#[cfg(feature = "ssr")]
#[derive(Clone)]
pub struct GitCommitPusher(pub std::sync::Arc<GitCommitPusherFn>);

// ── GitStatusReader ───────────────────────────────────────────────────────────

/// Boxed closure that returns the repo-relative paths with uncommitted changes
/// in a git-backed source's server-side clone.
///
/// Mirrors `GitCommitPusher`: the implementation lives in `tidl-server` (which
/// can use git2) and is injected as an Axum Extension. `tidl-web` only uses the
/// type.
#[cfg(feature = "ssr")]
type GitStatusReaderFn = dyn Fn(
        i64, // source_id
    )
        -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<Vec<String>, String>> + Send>>
    + Send
    + Sync;

#[cfg(feature = "ssr")]
#[derive(Clone)]
pub struct GitStatusReader(pub std::sync::Arc<GitStatusReaderFn>);

// ── GitBranchDetector ─────────────────────────────────────────────────────────

/// Boxed closure that connects to a git remote and returns its default branch
/// name, so a git source's blank branch field can be resolved to what the
/// remote actually checks out instead of guessing "main".
///
/// Wraps `Arc<dyn Fn(url, pat) -> impl Future<Output = Result<String, String>>>`,
/// mirroring `GitCommitPusher`. The implementation lives in `tidl-server`
/// (which can use git2) and is injected as an Axum Extension.
#[cfg(feature = "ssr")]
type GitBranchDetectorFn = dyn Fn(
        String, // url
        String, // pat
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<String, String>> + Send>>
    + Send
    + Sync;

#[cfg(feature = "ssr")]
#[derive(Clone)]
pub struct GitBranchDetector(pub std::sync::Arc<GitBranchDetectorFn>);

// ── WebcalSyncer ──────────────────────────────────────────────────────────────

/// Boxed closure that syncs a single webcal feed and reports whether anything
/// changed, mirroring `GitCommitPusher`.
///
/// The implementation lives in `tidl-server` (which owns the shared
/// `CalendarStore`-backed `sync_calendar_now` path used by the background sync
/// loop) and is injected as an Axum Extension. `tidl-web`'s manual
/// "refresh now" server function calls it so the manual and background syncs
/// share one code path instead of the API carrying its own divergent copy of
/// the fetch/parse/store logic (backlog #108).
#[cfg(feature = "ssr")]
type WebcalSyncerFn = dyn Fn(
        String, // username
        String, // calendar
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<bool, String>> + Send>>
    + Send
    + Sync;

#[cfg(feature = "ssr")]
#[derive(Clone)]
pub struct WebcalSyncer(pub std::sync::Arc<WebcalSyncerFn>);

// ── MailSender ────────────────────────────────────────────────────────────────

/// A boxed async function for sending transactional emails.
///
/// Signature: `(to_address, to_name, subject, body_text, body_html) -> Result<(), String>`
///
/// Injected by `tidl-server` and extracted by server functions that need to
/// send email (share invitations, notifications, …).
pub type MailSender = std::sync::Arc<
    dyn Fn(
            String, // to_address
            String, // to_name
            String, // subject
            String, // body_text
            String, // body_html
        )
            -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>> + Send>>
        + Send
        + Sync,
>;

// ── SmtpConfigRow ────────────────────────────────────────────────────────────

/// Typed row for reading SMTP configuration from the database.
///
/// Lives here rather than `api.rs` because all SMTP behaviour belongs alongside
/// the transport helper.  Callers use `query_as::<_, SmtpConfigRow>` with
/// [`SMTP_CONFIG_QUERY`] instead of extracting each field manually.
#[cfg(feature = "ssr")]
#[derive(sqlx::FromRow)]
pub(crate) struct SmtpConfigRow {
    pub host: String,
    pub port: i64,
    pub tls_mode: String,
    pub username: String,
    pub password_encrypted: String,
    pub from_address: String,
    pub from_name: String,
}

#[cfg(feature = "ssr")]
impl SmtpConfigRow {
    /// Format the `From:` header value, including display name when present.
    pub fn header_from(&self) -> String {
        if self.from_name.is_empty() {
            self.from_address.clone()
        } else {
            format!("{} <{}>", self.from_name, self.from_address)
        }
    }
}

/// SQL fragment that selects all columns needed to populate a [`SmtpConfigRow`].
#[cfg(feature = "ssr")]
pub(crate) const SMTP_CONFIG_QUERY: &str =
    "SELECT host, port, tls_mode, username, password_encrypted, from_address, from_name \
     FROM smtp_config WHERE id = 1";

// ── SMTP send helper ─────────────────────────────────────────────────────────

/// Build an SMTP transport and send `message`, applying a 15-second Tokio
/// timeout so an unreachable server never blocks the caller for 60 seconds.
///
/// Returns a human-readable error string so both `mailer.rs` and the
/// "send test email" server function can map it to their own error type.
#[cfg(feature = "ssr")]
pub async fn smtp_send(
    host: &str,
    port: u16,
    tls_mode: &str,
    username: &str,
    password: &str,
    message: lettre::Message,
) -> Result<(), String> {
    use lettre::transport::smtp::authentication::Credentials;
    use lettre::{AsyncSmtpTransport, AsyncTransport, Tokio1Executor};

    const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(15);

    let creds = if username.is_empty() {
        None
    } else {
        Some(Credentials::new(username.to_string(), password.to_string()))
    };

    let transport: AsyncSmtpTransport<Tokio1Executor> = match tls_mode {
        "tls" => {
            let mut b = AsyncSmtpTransport::<Tokio1Executor>::relay(host)
                .map_err(|e| format!("invalid SMTP relay host: {e}"))?
                .port(port)
                .timeout(Some(TIMEOUT));
            if let Some(c) = creds {
                b = b.credentials(c);
            }
            b.build()
        }
        "starttls" => {
            let mut b = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(host)
                .map_err(|e| format!("invalid SMTP relay host: {e}"))?
                .port(port)
                .timeout(Some(TIMEOUT));
            if let Some(c) = creds {
                b = b.credentials(c);
            }
            b.build()
        }
        _ => {
            // "none" — plain SMTP (dev/testing only)
            let mut b = AsyncSmtpTransport::<Tokio1Executor>::builder_dangerous(host)
                .port(port)
                .timeout(Some(TIMEOUT));
            if let Some(c) = creds {
                b = b.credentials(c);
            }
            b.build()
        }
    };

    let result = tokio::time::timeout(TIMEOUT, transport.send(message)).await;
    match result {
        Err(_) => {
            let msg = format!(
                "SMTP connection to {host}:{port} timed out — server is unreachable or not responding"
            );
            tracing::error!("{msg}");
            Err(msg)
        }
        Ok(Err(e)) => {
            let msg = format!("SMTP send failed: {e}");
            tracing::error!("{msg}");
            Err(msg)
        }
        Ok(Ok(_)) => {
            tracing::info!(host = %host, port = %port, "SMTP message delivered successfully");
            Ok(())
        }
    }
}

// ── Password encryption helpers ───────────────────────────────────────────────

#[cfg(feature = "ssr")]
pub fn encrypt_password(plaintext: &str, secret: &str) -> Result<String, String> {
    use aes_gcm::aead::OsRng;
    use aes_gcm::{
        aead::rand_core::RngCore,
        aead::{Aead, KeyInit},
        Aes256Gcm, Nonce,
    };
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    use sha2::{Digest, Sha256};

    if plaintext.is_empty() {
        return Ok(String::new());
    }

    let key: [u8; 32] = {
        let mut h = Sha256::new();
        h.update(secret.as_bytes());
        h.update(b"tidl-smtp-v1");
        h.finalize().into()
    };

    let cipher = Aes256Gcm::new(key.as_ref().into());
    let mut nonce_bytes = [0u8; 12];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);

    let ct = cipher
        .encrypt(nonce, plaintext.as_bytes())
        .map_err(|e| format!("encrypt failed: {e}"))?;

    Ok(format!(
        "{}:{}",
        BASE64.encode(nonce_bytes),
        BASE64.encode(ct)
    ))
}

#[cfg(feature = "ssr")]
pub fn decrypt_password(encrypted: &str, secret: &str) -> Result<String, String> {
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm, Nonce,
    };
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    use sha2::{Digest, Sha256};

    if encrypted.is_empty() {
        return Ok(String::new());
    }

    let (nonce_b64, ct_b64) = encrypted
        .split_once(':')
        .ok_or("malformed encrypted password")?;

    let nonce_bytes = BASE64
        .decode(nonce_b64)
        .map_err(|e| format!("bad nonce base64: {e}"))?;
    let ct = BASE64
        .decode(ct_b64)
        .map_err(|e| format!("bad ciphertext base64: {e}"))?;

    let key: [u8; 32] = {
        let mut h = Sha256::new();
        h.update(secret.as_bytes());
        h.update(b"tidl-smtp-v1");
        h.finalize().into()
    };

    let cipher = Aes256Gcm::new(key.as_ref().into());
    let nonce = Nonce::from_slice(&nonce_bytes);

    let plaintext = cipher
        .decrypt(nonce, ct.as_ref())
        .map_err(|e| format!("decrypt failed: {e}"))?;

    String::from_utf8(plaintext).map_err(|e| format!("UTF-8 error: {e}"))
}

// ── Note priority reverse-sync ──────────────────────────────────────────────

/// Propagates an external CalDAV client's priority edit on a note-mirrored
/// VTODO back onto the note's own canonical 1-100 priority — the inverse of
/// `api::notes::sync_mirror`'s one-way note-priority → VTODO-priority
/// write. Called by `tidl-server`'s CalDAV PUT handler after a successful
/// `store_event`, so a client editing priority through its own native UI
/// stays in sync with what the Notes tab slider and Focus/Tasks show
/// (2026-07-30 investigation).
///
/// `caldav_priority` (RFC 5545's native 1-9 range) is projected onto the
/// canonical 1-100 scale via `caldav_priority_to_source_priority` — the same
/// projection applied to any other task source at the CalDAV boundary (see
/// backlog #70) — rather than being stored verbatim, since a note's
/// `priority` column lives on that 1-100 domain. A no-op (0 rows affected)
/// if `mirrored_event_uid` doesn't belong to any note, which is the common
/// case for ordinary (non-note) CalDAV tasks.
#[cfg(feature = "ssr")]
pub async fn sync_note_priority_from_caldav(
    pool: &sqlx::SqlitePool,
    mirrored_event_uid: &str,
    caldav_priority: i64,
) -> Result<(), sqlx::Error> {
    let source_priority = crate::types::caldav_priority_to_source_priority(caldav_priority);

    sqlx::query(
        "UPDATE notes SET priority = ?, updated_at = datetime('now') WHERE mirrored_event_uid = ?",
    )
    .bind(source_priority as i64)
    .bind(mirrored_event_uid)
    .execute(pool)
    .await?;

    Ok(())
}

// ── External CalDAV done-item hook ──────────────────────────────────────────

/// Immediately reflects an external CalDAV client's VTODO completion (or
/// un-completion) in the `done_items` index, instead of waiting for the next
/// Done-view reconcile (backlog #136). `completed` upserts a live row for a
/// completed VTODO (a no-op if the event is gone — e.g. a note-mirror
/// completion deleted it — or isn't completed); `!completed` archives any live
/// row, keeping the history.
///
/// Called by `tidl-server`'s CalDAV PUT (`sync_vtodo_completion`) and DELETE
/// (`caldav_event_delete_or_subcollection`) handlers — the same cross-crate
/// pattern as `sync_note_priority_from_caldav`.
#[cfg(feature = "ssr")]
pub async fn sync_done_item_from_caldav(
    pool: &sqlx::SqlitePool,
    username: &str,
    calendar: &str,
    uid: &str,
    completed: bool,
) -> Result<(), sqlx::Error> {
    if completed {
        crate::api::done::upsert_caldav_done_item(pool, username, calendar, uid).await
    } else {
        crate::api::done::archive_caldav_done_item(pool, username, uid).await
    }
}
