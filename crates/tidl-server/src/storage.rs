use anyhow::Result;
use async_trait::async_trait;
use chrono::{DateTime, Utc};
use sqlx::{
    sqlite::{SqliteConnectOptions, SqliteJournalMode},
    Row, SqlitePool,
};
use std::{path::Path, time::Duration};
use tokio::fs;

use crate::config::Config;

// ── CalendarStore trait ───────────────────────────────────────────────────────

/// Trait that abstracts all CalDAV storage operations (calendars, events,
/// WebDAV locks, and dead properties).
///
/// `Storage` is the production implementation.  An alternative implementation
/// (e.g. an in-memory store for tests) can be injected via `AppState`.
#[async_trait]
pub trait CalendarStore: Send + Sync {
    // ── Calendars ─────────────────────────────────────────────────────────────
    async fn create_calendar(
        &self,
        username: &str,
        calendar: &str,
        display_name: Option<&str>,
    ) -> Result<()>;
    async fn get_calendar(&self, username: &str, calendar: &str) -> Result<Option<Calendar>>;
    async fn list_calendars(&self, username: &str) -> Result<Vec<Calendar>>;
    async fn delete_calendar(&self, username: &str, calendar: &str) -> Result<()>;
    async fn set_calendar_sync(
        &self,
        username: &str,
        calendar: &str,
        read_only: bool,
        sync_user: Option<&str>,
    ) -> Result<()>;
    /// Set or clear the webcal subscription URL. Passing `None` detaches the feed and marks the
    /// calendar writable again; passing `Some(url)` attaches it and marks it read-only.
    async fn set_calendar_sync_url(
        &self,
        username: &str,
        calendar: &str,
        url: Option<&str>,
    ) -> Result<()>;
    /// Persist the outcome of a webcal fetch: update ETag, Last-Modified, timestamp, and error.
    async fn update_webcal_sync_status(
        &self,
        username: &str,
        calendar: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
        error: Option<&str>,
    ) -> Result<()>;
    async fn copy_calendar(
        &self,
        src_username: &str,
        src_calendar: &str,
        dst_username: &str,
        dst_calendar: &str,
        deep: bool,
    ) -> Result<bool>;

    // ── Events ────────────────────────────────────────────────────────────────
    async fn store_event(
        &self,
        username: &str,
        calendar: &str,
        uid: &str,
        ical_data: &str,
        etag: &str,
    ) -> Result<()>;
    async fn get_event(
        &self,
        username: &str,
        calendar: &str,
        uid: &str,
    ) -> Result<Option<(String, String)>>;
    async fn delete_event(&self, username: &str, calendar: &str, uid: &str) -> Result<()>;
    async fn list_events(&self, username: &str, calendar: &str) -> Result<Vec<Event>>;
    async fn list_todos(&self, username: &str) -> Result<Vec<Event>>;
    async fn copy_event(
        &self,
        src_username: &str,
        src_calendar: &str,
        src_uid: &str,
        dst_username: &str,
        dst_calendar: &str,
        dst_uid: &str,
    ) -> Result<Option<String>>;
    /// Propagates a CalDAV client's `PRIORITY` edit on a note-mirrored VTODO
    /// back onto the originating note's own 0-100 priority — a no-op if
    /// `uid` isn't a note mirror. See `tidl_web::ssr_types::sync_note_priority_from_caldav`.
    async fn sync_note_priority_from_mirror(&self, uid: &str, caldav_priority: i64) -> Result<()>;
    /// Set a VTODO's completion state directly — for a plain (non-note)
    /// task. Note-mirrored tasks never reach this: completing one is
    /// handled by `complete_note_mirror_if_any` instead (backlog #40 parity
    /// — see its doc comment).
    async fn set_event_completed(
        &self,
        username: &str,
        calendar: &str,
        uid: &str,
        completed: bool,
    ) -> Result<()>;
    /// If `uid` is a note's mirrored VTODO, clear that note's `kind`/
    /// `due_date`/`mirrored_event_uid` (reverting it to a plain, non-todo
    /// note) and return `true`. A no-op returning `false` for any other
    /// uid. Mirrors the cleanup `tidl-web`'s own `complete_todo_item`
    /// does for the app's own completion checkbox (backlog #40) — used so
    /// completing *or deleting* a note-linked task via a raw external
    /// CalDAV client behaves identically to completing it in the app
    /// (backlog #71). Does not delete the `events` row itself — callers
    /// differ on whether that still needs doing.
    async fn complete_note_mirror_if_any(&self, uid: &str) -> Result<bool>;
    /// Immediately reflect an external CalDAV client's VTODO completion (or
    /// un-completion) in the `done_items` index (backlog #136) — a no-op for a
    /// uid that isn't a completed VTODO when `completed`, and for a uid with no
    /// live done row when `!completed`. See
    /// `tidl_web::ssr_types::sync_done_item_from_caldav`.
    async fn sync_done_item_from_caldav(
        &self,
        username: &str,
        calendar: &str,
        uid: &str,
        completed: bool,
    ) -> Result<()>;
    async fn move_event(
        &self,
        src_username: &str,
        src_calendar: &str,
        src_uid: &str,
        dst_username: &str,
        dst_calendar: &str,
        dst_uid: &str,
    ) -> Result<bool>;

    // ── WebDAV Locks ──────────────────────────────────────────────────────────
    async fn create_lock(
        &self,
        username: &str,
        resource_path: &str,
        scope: &str,
        depth: &str,
        owner_info: Option<&str>,
        timeout_secs: i64,
    ) -> Result<String>;
    async fn get_lock(&self, token: &str) -> Result<Option<LockInfo>>;
    async fn get_effective_locks(&self, resource_path: &str) -> Result<Vec<LockInfo>>;
    async fn get_locks_for_resource(&self, resource_path: &str) -> Result<Vec<LockInfo>>;
    async fn delete_lock(&self, token: &str) -> Result<bool>;
    async fn refresh_lock(&self, token: &str, timeout_secs: i64) -> Result<bool>;

    // ── Dead WebDAV Properties ────────────────────────────────────────────────
    async fn set_property(
        &self,
        username: &str,
        resource_path: &str,
        namespace: &str,
        name: &str,
        value: &str,
    ) -> Result<()>;
    async fn delete_property(
        &self,
        username: &str,
        resource_path: &str,
        namespace: &str,
        name: &str,
    ) -> Result<()>;
    async fn get_properties(
        &self,
        username: &str,
        resource_path: &str,
    ) -> Result<Vec<(String, String, String)>>;
    async fn copy_properties(
        &self,
        src_username: &str,
        src_path: &str,
        dst_username: &str,
        dst_path: &str,
    ) -> Result<()>;
    async fn delete_properties(&self, username: &str, resource_path: &str) -> Result<()>;

    // ── Calendar metadata ─────────────────────────────────────────────────────
    async fn update_calendar_color(
        &self,
        username: &str,
        calendar: &str,
        color: &str,
    ) -> Result<()>;
    async fn update_calendar_display_name(
        &self,
        username: &str,
        calendar: &str,
        display_name: &str,
    ) -> Result<()>;
    async fn ensure_personal_calendar(&self, username: &str) -> Result<()>;

    // ── Sync tokens (RFC 6578 / Apple getctag) ────────────────────────────────
    /// Return the current sync token for a calendar (0 if calendar not found).
    async fn get_sync_token(&self, username: &str, calendar: &str) -> Result<i64>;
    /// Increment the sync token, append a change-log entry, and return the new token.
    async fn bump_sync_token(
        &self,
        username: &str,
        calendar: &str,
        uid: &str,
        operation: &str,
    ) -> Result<i64>;
    /// Return changes in the half-open range `(since_token, until_token]`.
    /// Pass `None` for `until_token` to get everything since `since_token`.
    /// Latest operation per uid wins.
    async fn get_sync_changes(
        &self,
        username: &str,
        calendar: &str,
        since_token: i64,
        until_token: Option<i64>,
    ) -> Result<Vec<SyncChange>>;

    // ── Calendar sharing ──────────────────────────────────────────────────────

    /// Insert a pending share invitation (token link to be sent by email).
    async fn create_share_invitation(
        &self,
        owner: &str,
        resource: ShareResource,
        invitee_email: &str,
        token: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()>;

    /// Look up a pending invitation by its token.
    async fn get_share_invitation(&self, token: &str) -> Result<Option<ShareInvitation>>;

    /// Mark an invitation as accepted and create the `calendar_shares` row.
    /// `grantee_email` must match the invitation's `invitee_email`
    /// (case-insensitively) — otherwise a different account than the one
    /// invited could accept the share by obtaining/guessing the token.
    /// Returns the owner's username on success, so the caller can notify
    /// both sides over SSE without a second lookup.
    async fn accept_share_invitation(
        &self,
        token: &str,
        grantee_username: &str,
        grantee_email: &str,
    ) -> Result<String>;

    /// Mark an invitation as declined.
    async fn decline_share_invitation(&self, token: &str) -> Result<()>;

    /// List all accepted grants for a calendar owned by `owner`.
    async fn list_shares_for_calendar(
        &self,
        owner: &str,
        calendar: &str,
    ) -> Result<Vec<ShareGrant>>;

    /// Look up a specific grant.
    async fn get_share_grant(
        &self,
        owner: &str,
        calendar: &str,
        grantee: &str,
    ) -> Result<Option<ShareGrant>>;

    /// Look up a share grant directly by its stable ID.
    async fn get_share_grant_by_id(&self, id: i64) -> Result<Option<ShareGrant>>;

    /// Remove a share grant (revoke).  Also marks any matching pending invitations as declined.
    async fn revoke_share(&self, owner: &str, calendar: &str, grantee: &str) -> Result<()>;

    /// List all calendars shared *with* `grantee` (by any owner).
    async fn list_shared_with_user(&self, grantee: &str) -> Result<Vec<SharedCalendarInfo>>;

    // ── SMTP configuration ────────────────────────────────────────────────────

    async fn get_smtp_config(&self) -> Result<Option<SmtpConfigRow>>;
    async fn save_smtp_config(&self, cfg: &SmtpConfigRow) -> Result<()>;
}

/// Trait that abstracts CardDAV storage operations (backlog #29) — the
/// `contacts`/`contact_emails`/`contact_phones` tables from #79 plus the
/// `address_books` sync-token bookkeeping. Kept separate from
/// `CalendarStore`: a different resource with its own lifecycle (no
/// per-object collection like calendars — one implicit address book per
/// user), and `CalendarStore` is already large.
#[async_trait]
pub trait ContactStore: Send + Sync {
    /// Every contact in one address book (backlog #81 — a `contact_collections`
    /// row, not a user). Access must already have been resolved by the
    /// caller (`resolve_addressbook_identity` for CardDAV) — this method
    /// trusts `collection_id`, it doesn't re-check who may see it.
    async fn list_contacts_in_collection(&self, collection_id: i64) -> Result<Vec<ContactRecord>>;
    async fn get_contact_by_uid(
        &self,
        collection_id: i64,
        uid: &str,
    ) -> Result<Option<ContactRecord>>;
    /// Create-or-update keyed by `uid` (the CardDAV resource identity) — an
    /// `INSERT ... ON CONFLICT(uid) DO UPDATE`, mirroring `store_event`'s own
    /// upsert reasoning: a client's PUT only ever supplies the fields a
    /// vCard carries, so this must not clobber columns the vCard has no
    /// representation for (there are none today, but keeps the two upsert
    /// paths consistent). `owner` is creator attribution (`contacts.owner_username`
    /// — the *authenticated* caller, backlog #81's Plan 1), separate from
    /// `collection_id` (the resolved target address book) — these aren't
    /// always the same value once a shared collection is involved.
    async fn store_contact_vcard(
        &self,
        owner: &str,
        collection_id: i64,
        uid: &str,
        fields: &tidl_carddav::VCardFields,
        etag: &str,
    ) -> Result<()>;
    /// Returns `true` if a contact was actually deleted.
    async fn delete_contact_by_uid(&self, collection_id: i64, uid: &str) -> Result<bool>;

    /// Targeted `(photo bytes, content type)` fetch for the plain `GET
    /// /api/contacts/{id}/photo` route (#86) — deliberately not part of
    /// `ContactRecord`'s normal id-based lookups. `username` gates access
    /// via collection ownership/membership (owner or member of the
    /// contact's collection), not a literal creator match — a shared
    /// contact's photo is visible to every collection member, matching
    /// every other collection-scoped read. `None` covers "no such
    /// accessible contact" and "contact exists but has no photo" alike —
    /// the route treats both as 404.
    async fn get_contact_photo_by_id(
        &self,
        username: &str,
        id: i64,
    ) -> Result<Option<(Vec<u8>, String)>>;

    // ── Address-book sync token (RFC 6578 / Apple getctag) ───────────────────
    async fn get_address_book_sync_token(&self, collection_id: i64) -> Result<i64>;
    async fn bump_address_book_sync_token(
        &self,
        collection_id: i64,
        uid: &str,
        operation: &str,
    ) -> Result<i64>;
    async fn get_contact_sync_changes(
        &self,
        collection_id: i64,
        since_token: i64,
        until_token: Option<i64>,
    ) -> Result<Vec<SyncChange>>;

    // ── Collection access resolution (backlog #81) ────────────────────────────
    /// Every collection `username` owns or is a member of — for
    /// address-book-home-set enumeration.
    async fn list_contact_collections_for(
        &self,
        username: &str,
    ) -> Result<Vec<ContactCollectionRecord>>;
    /// Resolves whether `username` may access `collection_id`, distinguishing
    /// "no such collection" from "exists but not accessible" — CardDAV
    /// handlers need this distinction to return 404 vs. 403 correctly
    /// (found via `tests/test_carddav_sharing.py`'s own `book-999999` case).
    async fn contact_collection_access(
        &self,
        username: &str,
        collection_id: i64,
    ) -> Result<ContactCollectionAccess>;
    /// The owner's default collection ("My Contacts"), creating one if
    /// missing — idempotent.
    async fn ensure_default_contact_collection(&self, owner: &str) -> Result<i64>;
    /// Every contact `username` can access across every collection they
    /// own or are a member of — for the web-UI export/photo routes, not
    /// CardDAV (a CardDAV client sees shared collections individually via
    /// its own multi-address-book discovery instead).
    async fn list_accessible_contacts(&self, username: &str) -> Result<Vec<ContactRecord>>;
}

/// Read access to note images (backlog #110) for the plain
/// `GET /api/notes/{note_id}/images/{image_id}` route. Note CRUD itself lives
/// in `tidl-web`'s server fns; this trait is only the serving path
/// `tidl-server` cannot reach through those.
#[async_trait]
pub trait NoteStore: Send + Sync {
    /// `(bytes, content_type)` for one accessible note image, or `None` if it
    /// doesn't exist, doesn't belong to `note_id`, or `note_id` is not in a
    /// collection `username` owns or is a member of.
    async fn get_note_image(
        &self,
        username: &str,
        note_id: i64,
        image_id: i64,
    ) -> Result<Option<(Vec<u8>, String)>>;
}

#[derive(Clone)]
pub struct Storage {
    pub(crate) pool: SqlitePool,
    pub(crate) data_dir: String,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct LockInfo {
    pub token: String,
    pub username: String,
    pub resource_path: String,
    pub scope: String,
    pub depth: String,
    pub owner_info: Option<String>,
    pub timeout_secs: i64,
}

/// One entry from the sync change log: an event was created/modified or deleted.
#[derive(Debug, Clone)]
pub struct SyncChange {
    pub uid: String,
    /// `"modified"` (covers both creates and updates) or `"deleted"`.
    pub operation: String,
}

/// One `contacts` row plus its `contact_emails`/`contact_phones` rows —
/// `tidl-server`'s own plain-struct view (per CLAUDE.md §11, no sqlx types
/// leak past this boundary). Distinct from `tidl_web::types::ContactDto`,
/// which lives across the compilation boundary in `tidl-web` and is shaped
/// for the web UI, not vCard mapping.
#[derive(Debug, Clone)]
pub struct ContactRecord {
    pub id: i64,
    pub uid: String,
    pub display_name: String,
    pub given_name: Option<String>,
    pub family_name: Option<String>,
    pub middle_name: Option<String>,
    pub name_prefix: Option<String>,
    pub name_suffix: Option<String>,
    pub nickname: Option<String>,
    pub org: Option<String>,
    pub title: Option<String>,
    /// `yyyy-mm-dd`.
    pub birthday: Option<String>,
    /// `yyyy-mm-dd`.
    pub anniversary: Option<String>,
    pub notes: Option<String>,
    pub etag: String,
    pub categories: Vec<String>,
    pub emails: Vec<ContactMethodRecord>,
    pub phones: Vec<ContactMethodRecord>,
    pub urls: Vec<ContactMethodRecord>,
    pub impps: Vec<ContactMethodRecord>,
    pub addresses: Vec<ContactAddressRecord>,
    /// Full photo bytes — populated here (not just a `has_photo` flag)
    /// because CardDAV's `addressbook-query` REPORT builds full vCards
    /// straight from `list_contacts`'s result (see `carddav_handlers.rs`),
    /// so both `list_contacts` and `get_contact_by_uid` need the bytes, not
    /// just single-contact fetches. The web UI's own, separate list query
    /// (`tidl-web/api/contacts.rs`, duplicated across the compilation
    /// boundary like the rest of this trait) stays light with just a
    /// `has_photo` bool instead.
    pub photo: Option<Vec<u8>>,
    pub photo_content_type: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ContactMethodRecord {
    pub value: String,
    /// "home" | "work" | "mobile" | "other".
    pub kind: String,
    pub is_primary: bool,
}

#[derive(Debug, Clone)]
pub struct ContactAddressRecord {
    /// "home" | "work" | "other" — no "mobile", doesn't apply to an address.
    pub kind: String,
    pub is_primary: bool,
    pub street: Option<String>,
    pub city: Option<String>,
    pub region: Option<String>,
    pub postal_code: Option<String>,
    pub country: Option<String>,
}

/// Returned by [`Storage::store_contact_vcard`] when the request's `uid`
/// already belongs to a different owner's contact — see that method's doc
/// comment. Handlers downcast for this to distinguish "forbidden" from a
/// generic storage failure.
#[derive(Debug, Clone, Copy, thiserror::Error)]
#[error("uid belongs to a different owner's address book")]
pub struct UidOwnershipConflict;

/// One contact collection accessible to a CardDAV user — used to
/// enumerate `book-{id}` address books in the address-book-home-set
/// PROPFIND (backlog #81). Unified shape for owned and member
/// collections, unlike calendars' separate owned/`SharedCalendar` types —
/// CardDAV exposes every accessible collection through the same
/// `book-{collection_id}` scheme regardless of ownership.
#[derive(Debug, Clone)]
pub struct ContactCollectionRecord {
    pub id: i64,
    pub name: String,
    pub owner_username: String,
}

/// Outcome of resolving whether a user may access a contact collection —
/// distinguishes "no such collection" from "exists but not accessible to
/// this user" (backlog #81), so CardDAV handlers can return 404 vs. 403
/// correctly instead of collapsing both into one case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContactCollectionAccess {
    NotFound,
    Forbidden,
    Allowed { owner_username: String },
}

#[derive(Debug, Clone)]
pub struct Calendar {
    pub id: Option<i64>, // AUTOINCREMENT → Option<i64> from sqlx
    pub username: String,
    pub name: String,
    pub display_name: Option<String>,
    pub description: Option<String>,
    pub color: Option<String>,
    pub read_only: bool,
    pub sync_user: Option<String>,
    /// Webcal subscription URL (webcal:// or https://). When set the calendar is read-only.
    pub sync_url: Option<String>,
    pub sync_etag: Option<String>,
    pub sync_last_modified: Option<String>,
    pub last_synced_at: Option<chrono::NaiveDateTime>,
    pub sync_error: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub id: Option<i64>, // AUTOINCREMENT → Option<i64> from sqlx
    pub username: String,
    pub calendar: String,
    pub uid: String,
    pub event_type: String,
    pub summary: Option<String>,
    pub description: Option<String>,
    pub location: Option<String>,
    pub start_date: Option<DateTime<Utc>>,
    pub end_date: Option<DateTime<Utc>>,
    pub due_date: Option<DateTime<Utc>>,
    pub priority: Option<i64>, // INTEGER → i64 in SQLite
    pub status: Option<String>,
    pub response_status: Option<String>,
    pub etag: String,
    pub ical_data: String,
}

/// Which resource a share invitation grants access to. Encodes the invariant
/// that a calendar invite always carries a permission tier and a note-
/// collection invite never does, instead of a bag of nullable fields.
#[derive(Debug, Clone, PartialEq)]
pub enum ShareResource {
    Calendar { name: String, permission: String },
    NoteCollection { id: i64 },
    ContactCollection { id: i64 },
}

/// A pending share invitation sent by email.
#[derive(Debug, Clone)]
pub struct ShareInvitation {
    pub owner_username: String,
    pub resource: ShareResource,
    pub invitee_email: String,
    pub token: String,
    pub expires_at: DateTime<Utc>,
    pub status: String,
}

/// An accepted share grant between two users.
#[derive(Debug, Clone)]
pub struct ShareGrant {
    pub id: i64,
    pub owner_username: String,
    pub calendar_name: String,
    pub grantee_username: String,
    pub permission: String,
}

/// A calendar shared with the calling user (returned when listing their view).
#[derive(Debug, Clone)]
pub struct SharedCalendarInfo {
    pub id: i64,
    pub owner_username: String,
    pub calendar_name: String,
    pub calendar_display_name: Option<String>,
    pub calendar_color: Option<String>,
    pub permission: String,
}

/// SMTP configuration row (singleton, id = 1).
/// `password_encrypted` stores `"{nonce_b64}:{ct_b64}"` (AES-256-GCM).
#[derive(Debug, Clone)]
pub struct SmtpConfigRow {
    pub host: String,
    pub port: i64,
    pub tls_mode: String,
    pub username: String,
    pub password_encrypted: String,
    pub from_address: String,
    pub from_name: String,
}

/// Escape `%`, `_`, and `\` in a LIKE pattern operand. Use with `ESCAPE '\'`.
fn escape_like(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

/// Pool-only implementation of [`Storage::bump_sync_token`], extracted so
/// callers that only hold a `SqlitePool` (e.g. `markdown_sync`'s background
/// loop, which has no `Storage` handle) can bump a calendar's ctag without
/// constructing one. See `migrations/20240109000000_sync_tokens.sql` — every
/// write to `events` must go through this so CalDAV clients that key their
/// refetch on `getctag`/`sync-token` (e.g. Thunderbird) notice the change.
pub(crate) async fn bump_sync_token_for(
    pool: &SqlitePool,
    username: &str,
    calendar: &str,
    uid: &str,
    operation: &str,
) -> Result<i64> {
    sqlx::query("UPDATE calendars SET sync_token = sync_token + 1 WHERE username = ? AND name = ?")
        .bind(username)
        .bind(calendar)
        .execute(pool)
        .await?;

    let row = sqlx::query("SELECT sync_token FROM calendars WHERE username = ? AND name = ?")
        .bind(username)
        .bind(calendar)
        .fetch_optional(pool)
        .await?;

    let token: i64 = match row {
        Some(r) => r.get("sync_token"),
        None => return Ok(0),
    };

    sqlx::query(
        "INSERT INTO sync_log (username, calendar, uid, operation, token) \
         VALUES (?, ?, ?, ?, ?)",
    )
    .bind(username)
    .bind(calendar)
    .bind(uid)
    .bind(operation)
    .bind(token)
    .execute(pool)
    .await?;

    Ok(token)
}

impl Storage {
    /// Expose the underlying connection pool (e.g. for Axum state injection).
    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn new(config: &Config) -> Result<Self> {
        let data_dir = &config.paths.data_dir;

        fs::create_dir_all(data_dir).await?;

        let db_path = Path::new(data_dir).join("tidl.db");
        tracing::info!("Using database at {}", db_path.display());
        let options = SqliteConnectOptions::new()
            .filename(&db_path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(Duration::from_secs(30));

        let pool = SqlitePool::connect_with(options).await?;

        sqlx::migrate!("../../migrations").run(&pool).await?;

        // One-time conversion of pre-#112 note-level task/await rows into
        // inline note tokens; idempotent (no-op once `notes.kind` is NULL).
        tidl_web::api::backfill_note_kind_tokens(&pool)
            .await
            .map_err(|e| anyhow::anyhow!("note token backfill failed: {e}"))?;

        Ok(Self {
            pool,
            data_dir: data_dir.clone(),
        })
    }

    pub async fn create_calendar(
        &self,
        username: &str,
        calendar: &str,
        display_name: Option<&str>,
    ) -> Result<()> {
        sqlx::query!(
            "INSERT INTO calendars (username, name, display_name) VALUES (?, ?, ?)",
            username,
            calendar,
            display_name
        )
        .execute(&self.pool)
        .await?;

        let cal_path = Path::new(&self.data_dir)
            .join("calendars")
            .join(username)
            .join(calendar);
        fs::create_dir_all(cal_path).await?;

        Ok(())
    }

    pub async fn get_calendar(&self, username: &str, calendar: &str) -> Result<Option<Calendar>> {
        let result = sqlx::query_as!(
            Calendar,
            "SELECT id, username, name, display_name, description, color, read_only, sync_user,
                    sync_url, sync_etag, sync_last_modified, last_synced_at, sync_error
             FROM calendars WHERE username = ? AND name = ?",
            username,
            calendar
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(result)
    }

    pub async fn list_calendars(&self, username: &str) -> Result<Vec<Calendar>> {
        let calendars = sqlx::query_as!(
            Calendar,
            "SELECT id, username, name, display_name, description, color, read_only, sync_user,
                    sync_url, sync_etag, sync_last_modified, last_synced_at, sync_error
             FROM calendars WHERE username = ?",
            username
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(calendars)
    }

    pub async fn delete_calendar(&self, username: &str, calendar: &str) -> Result<()> {
        // Build prefix for sub-collections (e.g. "litmus/%").
        let sub_prefix = format!("{}/", calendar);
        let sub_like = format!("{}%", escape_like(&sub_prefix));

        // Delete events in this calendar and all sub-collections.
        sqlx::query!(
            "DELETE FROM events WHERE username = ? AND (calendar = ? OR calendar LIKE ? ESCAPE '\\')",
            username,
            calendar,
            sub_like,
        )
        .execute(&self.pool)
        .await?;

        // Delete this calendar row and all sub-collection rows.
        sqlx::query!(
            "DELETE FROM calendars WHERE username = ? AND (name = ? OR name LIKE ? ESCAPE '\\')",
            username,
            calendar,
            sub_like,
        )
        .execute(&self.pool)
        .await?;

        // Drop the calendar's sync_log too (and its sub-collections'). Leaving
        // stale tombstones behind would let them outrank a recreated calendar's
        // entries (whose sync_token restarts at 1), feeding CalDAV clients
        // deletions for events that exist.
        sqlx::query(
            "DELETE FROM sync_log WHERE username = ? AND (calendar = ? OR calendar LIKE ? ESCAPE '\\')",
        )
        .bind(username)
        .bind(calendar)
        .bind(&sub_like)
        .execute(&self.pool)
        .await?;

        // Delete properties and locks for resources under this calendar.
        let cal_prefix = format!("/caldav/{}/{}", username, calendar);
        let cal_prefix_like = format!("{}%", cal_prefix);
        sqlx::query!(
            "DELETE FROM properties WHERE username = ? AND resource_path LIKE ?",
            username,
            cal_prefix_like,
        )
        .execute(&self.pool)
        .await?;
        sqlx::query!(
            "DELETE FROM locks WHERE username = ? AND resource_path LIKE ?",
            username,
            cal_prefix_like,
        )
        .execute(&self.pool)
        .await?;

        // Remove the filesystem directory (sub-collections live inside it).
        let cal_path = Path::new(&self.data_dir)
            .join("calendars")
            .join(username)
            .join(calendar);
        if cal_path.exists() {
            fs::remove_dir_all(cal_path).await?;
        }

        Ok(())
    }

    /// Mark a calendar as a read-only sync mirror and set which user may write to it.
    /// Pass `read_only = false` and `sync_user = None` to revert to a normal calendar.
    pub async fn set_calendar_sync(
        &self,
        username: &str,
        calendar: &str,
        read_only: bool,
        sync_user: Option<&str>,
    ) -> Result<()> {
        sqlx::query!(
            "UPDATE calendars SET read_only = ?, sync_user = ? WHERE username = ? AND name = ?",
            read_only,
            sync_user,
            username,
            calendar,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Attach or detach a webcal subscription URL.
    /// When `url` is `Some`, marks the calendar read-only; when `None`, clears the URL and
    /// resets read_only/sync_error/sync_etag/sync_last_modified/last_synced_at.
    pub async fn set_calendar_sync_url(
        &self,
        username: &str,
        calendar: &str,
        url: Option<&str>,
    ) -> Result<()> {
        match url {
            Some(u) => {
                sqlx::query!(
                    "UPDATE calendars
                     SET sync_url = ?, read_only = 1, sync_error = NULL
                     WHERE username = ? AND name = ?",
                    u,
                    username,
                    calendar,
                )
                .execute(&self.pool)
                .await?;
            }
            None => {
                sqlx::query!(
                    "UPDATE calendars
                     SET sync_url = NULL, sync_etag = NULL, sync_last_modified = NULL,
                         last_synced_at = NULL, sync_error = NULL, read_only = 0
                     WHERE username = ? AND name = ?",
                    username,
                    calendar,
                )
                .execute(&self.pool)
                .await?;
            }
        }
        Ok(())
    }

    /// Record the outcome of a webcal fetch attempt.
    pub async fn update_webcal_sync_status(
        &self,
        username: &str,
        calendar: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        sqlx::query!(
            "UPDATE calendars
             SET sync_etag = ?, sync_last_modified = ?,
                 last_synced_at = datetime('now'), sync_error = ?
             WHERE username = ? AND name = ?",
            etag,
            last_modified,
            error,
            username,
            calendar,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Copy a single event to a new calendar/uid. Returns the new etag.
    /// Also copies dead WebDAV properties from the source resource path.
    pub async fn copy_event(
        &self,
        src_username: &str,
        src_calendar: &str,
        src_uid: &str,
        dst_username: &str,
        dst_calendar: &str,
        dst_uid: &str,
    ) -> Result<Option<String>> {
        let row = sqlx::query!(
            "SELECT ical_data, etag FROM events WHERE username = ? AND calendar = ? AND uid = ?",
            src_username,
            src_calendar,
            src_uid,
        )
        .fetch_optional(&self.pool)
        .await?;

        let row = match row {
            Some(r) => r,
            None => return Ok(None),
        };

        self.store_event(
            dst_username,
            dst_calendar,
            dst_uid,
            &row.ical_data,
            &row.etag,
        )
        .await?;

        // Copy dead properties.
        let src_path = format!("/caldav/{}/{}/{}", src_username, src_calendar, src_uid);
        let dst_path = format!("/caldav/{}/{}/{}", dst_username, dst_calendar, dst_uid);
        self.copy_properties(src_username, &src_path, dst_username, &dst_path)
            .await?;

        Ok(Some(row.etag))
    }

    /// Move a single event (copy then delete source).
    /// Also moves dead WebDAV properties.
    pub async fn move_event(
        &self,
        src_username: &str,
        src_calendar: &str,
        src_uid: &str,
        dst_username: &str,
        dst_calendar: &str,
        dst_uid: &str,
    ) -> Result<bool> {
        let copied = self
            .copy_event(
                src_username,
                src_calendar,
                src_uid,
                dst_username,
                dst_calendar,
                dst_uid,
            )
            .await?;
        if copied.is_none() {
            return Ok(false);
        }
        // Move locks from source path to destination path.
        let src_path = format!("/caldav/{}/{}/{}", src_username, src_calendar, src_uid);
        let dst_path = format!("/caldav/{}/{}/{}", dst_username, dst_calendar, dst_uid);
        sqlx::query!(
            "UPDATE locks SET resource_path = ?, username = ? WHERE resource_path = ?",
            dst_path,
            dst_username,
            src_path,
        )
        .execute(&self.pool)
        .await?;
        // delete_event also cleans up dead properties for the source path.
        self.delete_event(src_username, src_calendar, src_uid)
            .await?;
        Ok(true)
    }

    /// Copy all dead properties from one resource path to another.
    pub async fn copy_properties(
        &self,
        src_username: &str,
        src_path: &str,
        dst_username: &str,
        dst_path: &str,
    ) -> Result<()> {
        sqlx::query!(
            "INSERT OR REPLACE INTO properties (username, resource_path, namespace, name, value, updated_at)
             SELECT ?, ?, namespace, name, value, datetime('now')
             FROM properties WHERE username = ? AND resource_path = ?",
            dst_username,
            dst_path,
            src_username,
            src_path,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Delete all dead properties for a resource path.
    pub async fn delete_properties(&self, username: &str, resource_path: &str) -> Result<()> {
        sqlx::query!(
            "DELETE FROM properties WHERE username = ? AND resource_path = ?",
            username,
            resource_path,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Copy a calendar collection to a new name.
    ///
    /// If `deep` is true (Depth: infinity), also copies all events and
    /// recursively copies nested sub-collections (names starting with
    /// `{src_calendar}/`). If `deep` is false (Depth: 0), only the collection
    /// resource itself is created — no children are copied.
    pub async fn copy_calendar(
        &self,
        src_username: &str,
        src_calendar: &str,
        dst_username: &str,
        dst_calendar: &str,
        deep: bool,
    ) -> Result<bool> {
        self.copy_calendar_depth(
            src_username,
            src_calendar,
            dst_username,
            dst_calendar,
            deep,
            0,
        )
        .await
    }

    async fn copy_calendar_depth(
        &self,
        src_username: &str,
        src_calendar: &str,
        dst_username: &str,
        dst_calendar: &str,
        deep: bool,
        depth: u32,
    ) -> Result<bool> {
        if depth > 10 {
            return Err(anyhow::anyhow!(
                "Maximum collection nesting depth (10) exceeded"
            ));
        }

        let cal = sqlx::query!(
            "SELECT display_name FROM calendars WHERE username = ? AND name = ?",
            src_username,
            src_calendar,
        )
        .fetch_optional(&self.pool)
        .await?;

        let cal = match cal {
            Some(c) => c,
            None => return Ok(false),
        };

        // Create destination calendar (ignore if already exists).
        let display_name = cal.display_name.as_deref();
        let _ = self
            .create_calendar(dst_username, dst_calendar, display_name)
            .await;

        if !deep {
            return Ok(true);
        }

        // Copy all direct events.
        let events = sqlx::query!(
            "SELECT uid, ical_data, etag FROM events WHERE username = ? AND calendar = ?",
            src_username,
            src_calendar,
        )
        .fetch_all(&self.pool)
        .await?;

        for ev in events {
            self.store_event(dst_username, dst_calendar, &ev.uid, &ev.ical_data, &ev.etag)
                .await?;
        }

        // Recursively copy nested sub-collections.
        let sub_prefix = format!("{}/", src_calendar);
        let sub_like = format!("{}%", escape_like(&sub_prefix));
        let sub_exclude = format!("{}%/%", escape_like(&sub_prefix));
        let sub_cols = sqlx::query!(
            "SELECT name FROM calendars WHERE username = ? AND name LIKE ? ESCAPE '\\' AND name NOT LIKE ? ESCAPE '\\'",
            src_username,
            sub_like,
            sub_exclude,
        )
        .fetch_all(&self.pool)
        .await?;

        for sub in sub_cols {
            let suffix = sub.name.trim_start_matches(&sub_prefix);
            let dst_sub = format!("{}/{}", dst_calendar, suffix);
            Box::pin(self.copy_calendar_depth(
                src_username,
                &sub.name,
                dst_username,
                &dst_sub,
                true,
                depth + 1,
            ))
            .await?;
        }

        Ok(true)
    }

    pub async fn store_event(
        &self,
        username: &str,
        calendar: &str,
        uid: &str,
        ical_data: &str,
        etag: &str,
    ) -> Result<()> {
        let response_status = tidl_caldav::extract_partstat(ical_data);
        let response_status = response_status.as_deref();

        // Parse derived fields so the web UI can display CalDAV-stored events.
        let fields = tidl_caldav::extract_event_fields(ical_data);
        let component_type = if fields.component_type.is_empty() {
            "VEVENT"
        } else {
            &fields.component_type
        };

        // `fields.priority` is RFC 5545's native 1-9 range, straight off the
        // wire — the DB column instead stores the app's canonical 1-100
        // scale (backlog #70 / 2026-07-30 investigation), so any client
        // (Thunderbird, DAVx5, ...) creating or editing a task directly via
        // CalDAV lands in the same domain as notes/markdown/Tasks-view
        // tasks. The raw `ical_data` blob is untouched and still round-trips
        // verbatim to clients.
        // RFC 5545's `PRIORITY:0` means "undefined", not "highest". For
        // VEVENTs that must become NULL (no attenuation, backlog #156);
        // VTODO keeps the historical 0 -> highest mapping.
        let canonical_priority = fields
            .priority
            .filter(|p| component_type != "VEVENT" || *p != 0)
            .map(tidl_web::types::caldav_priority_to_source_priority)
            .map(i64::from);

        // An upsert (not `INSERT OR REPLACE`) — REPLACE resolves a UNIQUE
        // conflict by deleting the existing row and inserting a fresh one,
        // which silently resets every column *not* listed here (`status`,
        // `source`, `source_file`, `source_line`, `topic`, `kind`, …) to
        // NULL/its default. Those columns are app-managed bookkeeping a raw
        // CalDAV client's PUT body has no way to supply, so an external
        // client editing an existing VTODO used to un-complete it and strip
        // a note/markdown mirror back to a plain "caldav" task — losing the
        // "go to source" link (buglist item 4) and silently reviving
        // completed items. `ON CONFLICT ... DO UPDATE` only touches the
        // columns this PUT actually has data for, leaving the rest alone.
        sqlx::query(
            "INSERT INTO events
             (username, calendar, uid, type, summary, description, location,
              start_date, end_date, due_date, priority, etag, ical_data, response_status, updated_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, datetime(?), datetime(?), datetime(?), ?, ?, ?, ?, datetime('now'))
             ON CONFLICT(username, calendar, uid) DO UPDATE SET
                 type            = excluded.type,
                 summary         = excluded.summary,
                 description     = excluded.description,
                 location        = excluded.location,
                 start_date      = excluded.start_date,
                 end_date        = excluded.end_date,
                 due_date        = excluded.due_date,
                 priority        = excluded.priority,
                 etag            = excluded.etag,
                 ical_data       = excluded.ical_data,
                 response_status = excluded.response_status,
                 updated_at      = excluded.updated_at"
        )
        .bind(username)
        .bind(calendar)
        .bind(uid)
        .bind(component_type)
        .bind(fields.summary.as_deref())
        .bind(fields.description.as_deref())
        .bind(fields.location.as_deref())
        .bind(fields.start_date.as_deref())
        .bind(fields.end_date.as_deref())
        .bind(fields.due_date.as_deref())
        .bind(canonical_priority)
        .bind(etag)
        .bind(ical_data)
        .bind(response_status)
        .execute(&self.pool)
        .await?;

        let cal_dir = Path::new(&self.data_dir)
            .join("calendars")
            .join(username)
            .join(calendar);
        fs::create_dir_all(&cal_dir).await?;
        fs::write(cal_dir.join(format!("{}.ics", uid)), ical_data).await?;

        self.bump_sync_token(username, calendar, uid, "modified")
            .await?;

        Ok(())
    }

    async fn sync_note_priority_from_mirror(&self, uid: &str, caldav_priority: i64) -> Result<()> {
        tidl_web::ssr_types::sync_note_priority_from_caldav(&self.pool, uid, caldav_priority)
            .await?;
        Ok(())
    }

    async fn set_event_completed(
        &self,
        username: &str,
        calendar: &str,
        uid: &str,
        completed: bool,
    ) -> Result<()> {
        let status = if completed {
            "COMPLETED"
        } else {
            "NEEDS-ACTION"
        };
        sqlx::query(
            "UPDATE events SET status = ?, completed_date = CASE WHEN ? THEN datetime('now') ELSE NULL END
             WHERE username = ? AND calendar = ? AND uid = ?",
        )
        .bind(status)
        .bind(completed)
        .bind(username)
        .bind(calendar)
        .bind(uid)
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    async fn complete_note_mirror_if_any(&self, uid: &str) -> Result<bool> {
        let result = sqlx::query(
            "UPDATE notes SET kind = NULL, due_date = NULL, mirrored_event_uid = NULL
             WHERE mirrored_event_uid = ?",
        )
        .bind(uid)
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    async fn sync_done_item_from_caldav(
        &self,
        username: &str,
        calendar: &str,
        uid: &str,
        completed: bool,
    ) -> Result<()> {
        tidl_web::ssr_types::sync_done_item_from_caldav(
            &self.pool, username, calendar, uid, completed,
        )
        .await?;
        Ok(())
    }

    pub async fn get_event(
        &self,
        username: &str,
        calendar: &str,
        uid: &str,
    ) -> Result<Option<(String, String)>> {
        let result = sqlx::query!(
            "SELECT ical_data, etag FROM events WHERE username = ? AND calendar = ? AND uid = ?",
            username,
            calendar,
            uid
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(result.map(|r| (r.ical_data, r.etag)))
    }

    pub async fn delete_event(&self, username: &str, calendar: &str, uid: &str) -> Result<()> {
        sqlx::query!(
            "DELETE FROM events WHERE username = ? AND calendar = ? AND uid = ?",
            username,
            calendar,
            uid
        )
        .execute(&self.pool)
        .await?;

        let event_path = Path::new(&self.data_dir)
            .join("calendars")
            .join(username)
            .join(calendar)
            .join(format!("{}.ics", uid));

        if event_path.exists() {
            fs::remove_file(event_path).await?;
        }

        // Clean up dead properties for this resource (locks are managed separately).
        let resource_path = format!("/caldav/{}/{}/{}", username, calendar, uid);
        self.delete_properties(username, &resource_path).await?;

        self.bump_sync_token(username, calendar, uid, "deleted")
            .await?;

        Ok(())
    }

    pub async fn list_events(&self, username: &str, calendar: &str) -> Result<Vec<Event>> {
        let events = sqlx::query_as!(
            Event,
            r#"SELECT
                id, username, calendar, uid,
                type as "event_type: String",
                summary, description, location,
                start_date as "start_date: DateTime<Utc>",
                end_date as "end_date: DateTime<Utc>",
                due_date as "due_date: DateTime<Utc>",
                priority, status, response_status, etag, ical_data
            FROM events
            WHERE username = ? AND calendar = ?"#,
            username,
            calendar
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(events)
    }

    // ── WebDAV Locks ─────────────────────────────────────────────────────────

    /// Create a new lock. Returns the lock token UUID.
    pub async fn create_lock(
        &self,
        username: &str,
        resource_path: &str,
        scope: &str,
        depth: &str,
        owner_info: Option<&str>,
        timeout_secs: i64,
    ) -> Result<String> {
        let token = format!("opaquelocktoken:{}", uuid::Uuid::new_v4());
        sqlx::query!(
            "INSERT INTO locks (token, username, resource_path, scope, depth, owner_info, timeout_secs, expires_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, datetime('now', ?||' seconds'))",
            token,
            username,
            resource_path,
            scope,
            depth,
            owner_info,
            timeout_secs,
            timeout_secs,
        )
        .execute(&self.pool)
        .await?;
        Ok(token)
    }

    /// Look up an active lock by token.
    pub async fn get_lock(&self, token: &str) -> Result<Option<LockInfo>> {
        let row = sqlx::query_as!(
            LockInfo,
            "SELECT token, username, resource_path, scope, depth, owner_info, timeout_secs
             FROM locks WHERE token = ? AND expires_at > datetime('now')",
            token,
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row)
    }

    /// Get all active locks that apply to a resource path, including depth-infinity
    /// locks on ancestor collections.
    pub async fn get_effective_locks(&self, resource_path: &str) -> Result<Vec<LockInfo>> {
        let rp = resource_path.trim_end_matches('/');
        // Collect all possible parent paths for this resource.
        // e.g. for /caldav/admin/litmus/lockcoll/lockme.txt:
        //   /caldav/admin/litmus/lockcoll/lockme.txt
        //   /caldav/admin/litmus/lockcoll
        //   /caldav/admin/litmus
        //   etc.
        let rows = sqlx::query_as!(
            LockInfo,
            "SELECT token, username, resource_path, scope, depth, owner_info, timeout_secs
             FROM locks WHERE expires_at > datetime('now')
               AND (resource_path = ?
                 OR (depth = 'infinity' AND ? LIKE resource_path || '/%'))",
            rp,
            rp,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Get all active locks on a resource path.
    pub async fn get_locks_for_resource(&self, resource_path: &str) -> Result<Vec<LockInfo>> {
        let rows = sqlx::query_as!(
            LockInfo,
            "SELECT token, username, resource_path, scope, depth, owner_info, timeout_secs
             FROM locks WHERE resource_path = ? AND expires_at > datetime('now')",
            resource_path,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows)
    }

    /// Delete a lock by token.
    pub async fn delete_lock(&self, token: &str) -> Result<bool> {
        let result = sqlx::query!("DELETE FROM locks WHERE token = ?", token)
            .execute(&self.pool)
            .await?;
        Ok(result.rows_affected() > 0)
    }

    /// Refresh a lock's expiry time.
    pub async fn refresh_lock(&self, token: &str, timeout_secs: i64) -> Result<bool> {
        let result = sqlx::query!(
            "UPDATE locks SET expires_at = datetime('now', ?||' seconds'), timeout_secs = ?
             WHERE token = ? AND expires_at > datetime('now')",
            timeout_secs,
            timeout_secs,
            token,
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    // ── Dead WebDAV properties ───────────────────────────────────────────────

    /// Set (upsert) a dead property on a resource path.
    pub async fn set_property(
        &self,
        username: &str,
        resource_path: &str,
        namespace: &str,
        name: &str,
        value: &str,
    ) -> Result<()> {
        sqlx::query!(
            "INSERT INTO properties (username, resource_path, namespace, name, value, updated_at)
             VALUES (?, ?, ?, ?, ?, datetime('now'))
             ON CONFLICT(username, resource_path, namespace, name)
             DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
            username,
            resource_path,
            namespace,
            name,
            value,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Remove a dead property from a resource path.
    pub async fn delete_property(
        &self,
        username: &str,
        resource_path: &str,
        namespace: &str,
        name: &str,
    ) -> Result<()> {
        sqlx::query!(
            "DELETE FROM properties WHERE username = ? AND resource_path = ? AND namespace = ? AND name = ?",
            username,
            resource_path,
            namespace,
            name,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Get all dead properties for a resource path.
    /// Returns `Vec<(namespace, name, value)>`.
    pub async fn get_properties(
        &self,
        username: &str,
        resource_path: &str,
    ) -> Result<Vec<(String, String, String)>> {
        let rows = sqlx::query!(
            "SELECT namespace, name, value FROM properties WHERE username = ? AND resource_path = ?",
            username,
            resource_path,
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| (r.namespace, r.name, r.value.unwrap_or_default()))
            .collect())
    }

    pub async fn update_calendar_color(
        &self,
        username: &str,
        calendar: &str,
        color: &str,
    ) -> Result<()> {
        sqlx::query!(
            "UPDATE calendars SET color = ?, updated_at = datetime('now') WHERE username = ? AND name = ?",
            color,
            username,
            calendar,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn update_calendar_display_name(
        &self,
        username: &str,
        calendar: &str,
        display_name: &str,
    ) -> Result<()> {
        sqlx::query!(
            "UPDATE calendars SET display_name = ?, updated_at = datetime('now') WHERE username = ? AND name = ?",
            display_name,
            username,
            calendar,
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    /// Ensure the user's default "Personal" calendar exists, creating it if absent.
    pub async fn ensure_personal_calendar(&self, username: &str) -> Result<()> {
        sqlx::query!(
            "INSERT OR IGNORE INTO calendars (username, name, display_name, color) \
             VALUES (?, 'personal', 'Personal', '#3b82f6')",
            username,
        )
        .execute(&self.pool)
        .await?;
        let cal_path = Path::new(&self.data_dir)
            .join("calendars")
            .join(username)
            .join("personal");
        fs::create_dir_all(cal_path).await?;
        Ok(())
    }

    // ── Sync tokens ──────────────────────────────────────────────────────────

    /// Return the current `sync_token` for a calendar collection (0 if absent).
    /// Uses runtime-typed query to avoid disturbing the SQLX offline cache.
    pub async fn get_sync_token(&self, username: &str, calendar: &str) -> Result<i64> {
        let row = sqlx::query("SELECT sync_token FROM calendars WHERE username = ? AND name = ?")
            .bind(username)
            .bind(calendar)
            .fetch_optional(&self.pool)
            .await?;

        Ok(row.map(|r| r.get::<i64, _>("sync_token")).unwrap_or(0))
    }

    /// Atomically increment the calendar's `sync_token`, record the change in
    /// `sync_log`, and return the new token value.
    pub async fn bump_sync_token(
        &self,
        username: &str,
        calendar: &str,
        uid: &str,
        operation: &str,
    ) -> Result<i64> {
        bump_sync_token_for(&self.pool, username, calendar, uid, operation).await
    }

    /// Return all changes since `since_token` (exclusive), one entry per uid
    /// with the latest operation winning (e.g. if modified then deleted, only
    /// the deletion is returned).
    pub async fn get_sync_changes(
        &self,
        username: &str,
        calendar: &str,
        since_token: i64,
        until_token: Option<i64>,
    ) -> Result<Vec<SyncChange>> {
        // Fetch changes in (since_token, until_token], newest first.
        // Use i64::MAX when no upper bound is specified.
        let until = until_token.unwrap_or(i64::MAX);
        let rows = sqlx::query(
            "SELECT id, uid, operation FROM sync_log \
             WHERE username = ? AND calendar = ? AND token > ? AND token <= ? \
             ORDER BY token DESC, id DESC",
        )
        .bind(username)
        .bind(calendar)
        .bind(since_token)
        .bind(until)
        .fetch_all(&self.pool)
        .await?;

        // Deduplicate: keep only the latest operation per uid.
        let mut seen = std::collections::HashSet::new();
        let mut changes = Vec::new();
        for row in rows {
            let uid: String = row.get("uid");
            if seen.insert(uid.clone()) {
                changes.push(SyncChange {
                    uid,
                    operation: row.get("operation"),
                });
            }
        }
        Ok(changes)
    }

    // ── Contacts / CardDAV (backlog #29) ──────────────────────────────────────

    /// Fetch every email/phone row for every contact `owner` owns, grouped by
    /// `contact_id` — avoids an N+1 fetch when listing many contacts. Mirrors
    /// `tidl_web::api::contacts::methods_by_contact`; can't share the code
    /// across the compilation boundary (CLAUDE.md §8), so it's duplicated
    /// here in `tidl-server`'s own plain-struct shape.
    async fn contact_methods_by_id(
        pool: &SqlitePool,
        table: &str,
        collection_id: i64,
    ) -> Result<std::collections::HashMap<i64, Vec<ContactMethodRecord>>> {
        let query = format!(
            "SELECT m.contact_id, m.value, m.type, m.is_primary
             FROM {table} m
             JOIN contacts c ON c.id = m.contact_id
             WHERE c.collection_id = ?
             ORDER BY m.is_primary DESC, m.id ASC"
        );
        let rows = sqlx::query(&query)
            .bind(collection_id)
            .fetch_all(pool)
            .await?;

        let mut out: std::collections::HashMap<i64, Vec<ContactMethodRecord>> =
            std::collections::HashMap::new();
        for r in rows {
            let contact_id: i64 = r.get("contact_id");
            out.entry(contact_id)
                .or_default()
                .push(ContactMethodRecord {
                    value: r.get("value"),
                    kind: r.get("type"),
                    is_primary: r.get("is_primary"),
                });
        }
        Ok(out)
    }

    /// Like [`Storage::contact_methods_by_id`] but for `contact_addresses`,
    /// whose shape (5 address fields instead of one `value`) doesn't fit
    /// that helper's generic query.
    async fn contact_addresses_by_id(
        pool: &SqlitePool,
        collection_id: i64,
    ) -> Result<std::collections::HashMap<i64, Vec<ContactAddressRecord>>> {
        let rows = sqlx::query(
            "SELECT a.contact_id, a.type, a.is_primary, a.street, a.city, a.region,
                    a.postal_code, a.country
             FROM contact_addresses a
             JOIN contacts c ON c.id = a.contact_id
             WHERE c.collection_id = ?
             ORDER BY a.is_primary DESC, a.id ASC",
        )
        .bind(collection_id)
        .fetch_all(pool)
        .await?;

        let mut out: std::collections::HashMap<i64, Vec<ContactAddressRecord>> =
            std::collections::HashMap::new();
        for r in rows {
            let contact_id: i64 = r.get("contact_id");
            out.entry(contact_id)
                .or_default()
                .push(ContactAddressRecord {
                    kind: r.get("type"),
                    is_primary: r.get("is_primary"),
                    street: r.get("street"),
                    city: r.get("city"),
                    region: r.get("region"),
                    postal_code: r.get("postal_code"),
                    country: r.get("country"),
                });
        }
        Ok(out)
    }

    /// Like [`Storage::contact_methods_by_id`] but for `contact_categories`
    /// — a flat tag list, no kind/primary.
    async fn contact_categories_by_id(
        pool: &SqlitePool,
        collection_id: i64,
    ) -> Result<std::collections::HashMap<i64, Vec<String>>> {
        let rows = sqlx::query(
            "SELECT g.contact_id, g.value
             FROM contact_categories g
             JOIN contacts c ON c.id = g.contact_id
             WHERE c.collection_id = ?
             ORDER BY g.id ASC",
        )
        .bind(collection_id)
        .fetch_all(pool)
        .await?;

        let mut out: std::collections::HashMap<i64, Vec<String>> = std::collections::HashMap::new();
        for r in rows {
            let contact_id: i64 = r.get("contact_id");
            out.entry(contact_id).or_default().push(r.get("value"));
        }
        Ok(out)
    }

    pub async fn list_contacts_in_collection(
        &self,
        collection_id: i64,
    ) -> Result<Vec<ContactRecord>> {
        let rows = sqlx::query!(
            r#"SELECT id as "id!: i64", uid, display_name, given_name, family_name,
                      middle_name, name_prefix, name_suffix, nickname, org, title,
                      CAST(birthday AS TEXT) AS "birthday: String",
                      CAST(anniversary AS TEXT) AS "anniversary: String",
                      notes, etag, photo, photo_content_type
               FROM contacts WHERE collection_id = ? ORDER BY id ASC"#,
            collection_id
        )
        .fetch_all(&self.pool)
        .await?;

        let mut emails_by_id =
            Self::contact_methods_by_id(&self.pool, "contact_emails", collection_id).await?;
        let mut phones_by_id =
            Self::contact_methods_by_id(&self.pool, "contact_phones", collection_id).await?;
        let mut urls_by_id =
            Self::contact_methods_by_id(&self.pool, "contact_urls", collection_id).await?;
        let mut impps_by_id =
            Self::contact_methods_by_id(&self.pool, "contact_impps", collection_id).await?;
        let mut addresses_by_id = Self::contact_addresses_by_id(&self.pool, collection_id).await?;
        let mut categories_by_id =
            Self::contact_categories_by_id(&self.pool, collection_id).await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let id = r.id;
                ContactRecord {
                    id,
                    uid: r.uid,
                    display_name: r.display_name,
                    given_name: r.given_name,
                    family_name: r.family_name,
                    middle_name: r.middle_name,
                    name_prefix: r.name_prefix,
                    name_suffix: r.name_suffix,
                    nickname: r.nickname,
                    org: r.org,
                    title: r.title,
                    birthday: r.birthday,
                    anniversary: r.anniversary,
                    notes: r.notes,
                    etag: r.etag,
                    categories: categories_by_id.remove(&id).unwrap_or_default(),
                    emails: emails_by_id.remove(&id).unwrap_or_default(),
                    phones: phones_by_id.remove(&id).unwrap_or_default(),
                    urls: urls_by_id.remove(&id).unwrap_or_default(),
                    impps: impps_by_id.remove(&id).unwrap_or_default(),
                    addresses: addresses_by_id.remove(&id).unwrap_or_default(),
                    photo: r.photo,
                    photo_content_type: r.photo_content_type,
                }
            })
            .collect())
    }

    pub async fn get_contact_by_uid(
        &self,
        collection_id: i64,
        uid: &str,
    ) -> Result<Option<ContactRecord>> {
        let row = sqlx::query!(
            r#"SELECT id as "id!: i64", uid, display_name, given_name, family_name,
                      middle_name, name_prefix, name_suffix, nickname, org, title,
                      CAST(birthday AS TEXT) AS "birthday: String",
                      CAST(anniversary AS TEXT) AS "anniversary: String",
                      notes, etag, photo, photo_content_type
               FROM contacts WHERE collection_id = ? AND uid = ?"#,
            collection_id,
            uid
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(row) = row else { return Ok(None) };

        let emails = sqlx::query_as!(
            ContactMethodRecord,
            r#"SELECT value, type as kind, is_primary as "is_primary: bool"
               FROM contact_emails WHERE contact_id = ? ORDER BY is_primary DESC, id ASC"#,
            row.id
        )
        .fetch_all(&self.pool)
        .await?;
        let phones = sqlx::query_as!(
            ContactMethodRecord,
            r#"SELECT value, type as kind, is_primary as "is_primary: bool"
               FROM contact_phones WHERE contact_id = ? ORDER BY is_primary DESC, id ASC"#,
            row.id
        )
        .fetch_all(&self.pool)
        .await?;
        let urls = sqlx::query_as!(
            ContactMethodRecord,
            r#"SELECT value, type as kind, is_primary as "is_primary: bool"
               FROM contact_urls WHERE contact_id = ? ORDER BY is_primary DESC, id ASC"#,
            row.id
        )
        .fetch_all(&self.pool)
        .await?;
        let impps = sqlx::query_as!(
            ContactMethodRecord,
            r#"SELECT value, type as kind, is_primary as "is_primary: bool"
               FROM contact_impps WHERE contact_id = ? ORDER BY is_primary DESC, id ASC"#,
            row.id
        )
        .fetch_all(&self.pool)
        .await?;
        let addresses = sqlx::query_as!(
            ContactAddressRecord,
            r#"SELECT type as kind, is_primary as "is_primary: bool", street, city, region,
                      postal_code, country
               FROM contact_addresses WHERE contact_id = ? ORDER BY is_primary DESC, id ASC"#,
            row.id
        )
        .fetch_all(&self.pool)
        .await?;
        let categories: Vec<String> = sqlx::query_scalar!(
            "SELECT value FROM contact_categories WHERE contact_id = ? ORDER BY id ASC",
            row.id
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(Some(ContactRecord {
            id: row.id,
            uid: row.uid,
            display_name: row.display_name,
            given_name: row.given_name,
            family_name: row.family_name,
            middle_name: row.middle_name,
            name_prefix: row.name_prefix,
            name_suffix: row.name_suffix,
            nickname: row.nickname,
            org: row.org,
            title: row.title,
            birthday: row.birthday,
            anniversary: row.anniversary,
            notes: row.notes,
            etag: row.etag,
            categories,
            emails,
            phones,
            urls,
            impps,
            addresses,
            photo: row.photo,
            photo_content_type: row.photo_content_type,
        }))
    }

    /// The owner's default contact collection ("My Contacts", backfilled
    /// for every user by the contact_collections migration), creating one
    /// if somehow missing — mirrors `bump_address_book_sync_token`'s own
    /// lazy-"ensure" idiom just above. Used so a CardDAV PUT (which has no
    /// notion of "which collection") never leaves `contacts.collection_id`
    /// null (backlog #81).
    async fn ensure_default_contact_collection(&self, owner: &str) -> Result<i64> {
        sqlx::query!(
            "INSERT INTO contact_collections (owner_username, name, color)
             VALUES (?, 'My Contacts', '#3b82f6')
             ON CONFLICT(owner_username, name) DO NOTHING",
            owner
        )
        .execute(&self.pool)
        .await?;

        let id = sqlx::query_scalar!(
            "SELECT id as \"id!\" FROM contact_collections WHERE owner_username = ? AND name = 'My Contacts'",
            owner
        )
        .fetch_one(&self.pool)
        .await?;
        Ok(id)
    }

    /// Create-or-update a contact keyed by its (client-supplied, vCard) `uid`.
    ///
    /// `contacts.uid` is globally unique (backlog #79's migration), not
    /// unique per-owner, so a plain `ON CONFLICT(uid) DO UPDATE` would let
    /// one user's PUT silently overwrite another user's contact if their
    /// clients ever generated colliding UIDs. Guard against that explicitly
    /// rather than relying on UUID collision odds alone.
    pub async fn store_contact_vcard(
        &self,
        owner: &str,
        collection_id: i64,
        uid: &str,
        fields: &tidl_carddav::VCardFields,
        etag: &str,
    ) -> Result<()> {
        let mut tx = self.pool.begin().await?;

        // Guards against one collection's PUT silently overwriting a
        // different collection's contact that just happens to share a
        // uid — `contacts.uid` is globally unique (backlog #79's
        // migration), not unique per-collection. `collection_id` (not
        // `owner`) is the identity boundary here: a shared collection can
        // contain contacts created by different members (backlog #81),
        // so two different people can both legitimately PUT to the same
        // collection's contact.
        let existing_collection =
            sqlx::query_scalar!("SELECT collection_id FROM contacts WHERE uid = ?", uid)
                .fetch_optional(&mut *tx)
                .await?
                .flatten();
        if let Some(existing_collection) = existing_collection {
            if existing_collection != collection_id {
                return Err(UidOwnershipConflict.into());
            }
        }

        let photo_bytes = fields.photo.as_ref().map(|p| p.bytes.clone());
        let photo_content_type = fields.photo.as_ref().map(|p| p.content_type.clone());

        let id: i64 = sqlx::query(
            "INSERT INTO contacts
             (owner_username, collection_id, uid, display_name, given_name, family_name, middle_name,
              name_prefix, name_suffix, nickname, org, title, birthday, anniversary, notes, etag,
              photo, photo_content_type)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
             ON CONFLICT(uid) DO UPDATE SET
                 display_name       = excluded.display_name,
                 given_name         = excluded.given_name,
                 family_name        = excluded.family_name,
                 middle_name        = excluded.middle_name,
                 name_prefix        = excluded.name_prefix,
                 name_suffix        = excluded.name_suffix,
                 nickname           = excluded.nickname,
                 org                = excluded.org,
                 title              = excluded.title,
                 birthday           = excluded.birthday,
                 anniversary        = excluded.anniversary,
                 notes              = excluded.notes,
                 etag               = excluded.etag,
                 photo              = excluded.photo,
                 photo_content_type = excluded.photo_content_type,
                 updated_at         = datetime('now')
             RETURNING id",
        )
        .bind(owner)
        .bind(collection_id)
        .bind(uid)
        .bind(&fields.display_name)
        .bind(&fields.given_name)
        .bind(&fields.family_name)
        .bind(&fields.middle_name)
        .bind(&fields.name_prefix)
        .bind(&fields.name_suffix)
        .bind(&fields.nickname)
        .bind(&fields.org)
        .bind(&fields.title)
        .bind(&fields.birthday)
        .bind(&fields.anniversary)
        .bind(&fields.notes)
        .bind(etag)
        .bind(photo_bytes)
        .bind(photo_content_type)
        .fetch_one(&mut *tx)
        .await?
        .get("id");

        sqlx::query!("DELETE FROM contact_emails WHERE contact_id = ?", id)
            .execute(&mut *tx)
            .await?;
        for m in fields.emails.iter().filter(|m| !m.value.trim().is_empty()) {
            sqlx::query!(
                "INSERT INTO contact_emails (contact_id, value, type, is_primary) VALUES (?, ?, ?, ?)",
                id, m.value, m.kind, m.is_primary
            )
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query!("DELETE FROM contact_phones WHERE contact_id = ?", id)
            .execute(&mut *tx)
            .await?;
        for m in fields.phones.iter().filter(|m| !m.value.trim().is_empty()) {
            sqlx::query!(
                "INSERT INTO contact_phones (contact_id, value, type, is_primary) VALUES (?, ?, ?, ?)",
                id, m.value, m.kind, m.is_primary
            )
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query!("DELETE FROM contact_urls WHERE contact_id = ?", id)
            .execute(&mut *tx)
            .await?;
        for m in fields.urls.iter().filter(|m| !m.value.trim().is_empty()) {
            sqlx::query!(
                "INSERT INTO contact_urls (contact_id, value, type, is_primary) VALUES (?, ?, ?, ?)",
                id, m.value, m.kind, m.is_primary
            )
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query!("DELETE FROM contact_impps WHERE contact_id = ?", id)
            .execute(&mut *tx)
            .await?;
        for m in fields.impps.iter().filter(|m| !m.value.trim().is_empty()) {
            sqlx::query!(
                "INSERT INTO contact_impps (contact_id, value, type, is_primary) VALUES (?, ?, ?, ?)",
                id, m.value, m.kind, m.is_primary
            )
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query!("DELETE FROM contact_addresses WHERE contact_id = ?", id)
            .execute(&mut *tx)
            .await?;
        for a in &fields.addresses {
            sqlx::query!(
                "INSERT INTO contact_addresses
                 (contact_id, type, is_primary, street, city, region, postal_code, country)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
                id,
                a.kind,
                a.is_primary,
                a.street,
                a.city,
                a.region,
                a.postal_code,
                a.country
            )
            .execute(&mut *tx)
            .await?;
        }

        sqlx::query!("DELETE FROM contact_categories WHERE contact_id = ?", id)
            .execute(&mut *tx)
            .await?;
        for category in fields.categories.iter().filter(|c| !c.trim().is_empty()) {
            sqlx::query!(
                "INSERT INTO contact_categories (contact_id, value) VALUES (?, ?)",
                id,
                category
            )
            .execute(&mut *tx)
            .await?;
        }

        tx.commit().await?;
        Ok(())
    }

    pub async fn delete_contact_by_uid(&self, collection_id: i64, uid: &str) -> Result<bool> {
        let result = sqlx::query!(
            "DELETE FROM contacts WHERE collection_id = ? AND uid = ?",
            collection_id,
            uid
        )
        .execute(&self.pool)
        .await?;
        Ok(result.rows_affected() > 0)
    }

    pub async fn get_contact_photo_by_id(
        &self,
        username: &str,
        id: i64,
    ) -> Result<Option<(Vec<u8>, String)>> {
        let row = sqlx::query!(
            "SELECT c.photo, c.photo_content_type
             FROM contacts c JOIN contact_collections cc ON cc.id = c.collection_id
             WHERE c.id = ?
                AND (cc.owner_username = ?
                     OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?))",
            id, username, username
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.and_then(|r| match (r.photo, r.photo_content_type) {
            (Some(bytes), Some(content_type)) => Some((bytes, content_type)),
            _ => None,
        }))
    }

    /// Return the current sync token for `collection_id`'s address book (0
    /// if no `address_books` row exists yet — the collection-creation path
    /// always creates one alongside, so this stays a defensive fallback
    /// rather than an error).
    pub async fn get_address_book_sync_token(&self, collection_id: i64) -> Result<i64> {
        let row = sqlx::query!(
            "SELECT sync_token FROM address_books WHERE collection_id = ?",
            collection_id
        )
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| r.sync_token).unwrap_or(0))
    }

    /// Atomically increment the address book's `sync_token`, record the
    /// change in `contact_sync_log`, and return the new token — mirrors
    /// `bump_sync_token_for`'s calendar equivalent.
    pub async fn bump_address_book_sync_token(
        &self,
        collection_id: i64,
        uid: &str,
        operation: &str,
    ) -> Result<i64> {
        sqlx::query!(
            "INSERT INTO address_books (collection_id, sync_token) VALUES (?, 0)
             ON CONFLICT(collection_id) DO NOTHING",
            collection_id
        )
        .execute(&self.pool)
        .await?;

        sqlx::query!(
            "UPDATE address_books SET sync_token = sync_token + 1 WHERE collection_id = ?",
            collection_id
        )
        .execute(&self.pool)
        .await?;

        let token = sqlx::query!(
            "SELECT sync_token FROM address_books WHERE collection_id = ?",
            collection_id
        )
        .fetch_one(&self.pool)
        .await?
        .sync_token;

        sqlx::query!(
            "INSERT INTO contact_sync_log (collection_id, uid, operation, token) VALUES (?, ?, ?, ?)",
            collection_id, uid, operation, token
        )
        .execute(&self.pool)
        .await?;

        Ok(token)
    }

    /// Return contact changes in the half-open range `(since_token,
    /// until_token]`, latest operation per uid winning — mirrors
    /// `get_sync_changes`'s calendar equivalent.
    pub async fn get_contact_sync_changes(
        &self,
        collection_id: i64,
        since_token: i64,
        until_token: Option<i64>,
    ) -> Result<Vec<SyncChange>> {
        let until = until_token.unwrap_or(i64::MAX);
        // Runtime `query` (not `query!`): mirrors `get_sync_changes`, and lets
        // the `ORDER BY ... id DESC` tie-break change the SQL without an
        // offline-cache regeneration.
        let rows = sqlx::query(
            "SELECT uid, operation FROM contact_sync_log
             WHERE collection_id = ? AND token > ? AND token <= ?
             ORDER BY token DESC, id DESC",
        )
        .bind(collection_id)
        .bind(since_token)
        .bind(until)
        .fetch_all(&self.pool)
        .await?;

        let mut seen = std::collections::HashSet::new();
        let mut changes = Vec::new();
        for row in rows {
            let uid: String = row.get("uid");
            if seen.insert(uid.clone()) {
                changes.push(SyncChange {
                    uid,
                    operation: row.get("operation"),
                });
            }
        }
        Ok(changes)
    }

    // ── Contact-collection access resolution (backlog #81) ─────────────────────

    pub async fn list_contact_collections_for(
        &self,
        username: &str,
    ) -> Result<Vec<ContactCollectionRecord>> {
        let rows = sqlx::query!(
            "SELECT id as \"id!: i64\", name, owner_username FROM contact_collections
             WHERE owner_username = ?
                OR id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?)
             ORDER BY name ASC",
            username, username
        )
        .fetch_all(&self.pool)
        .await?;
        Ok(rows
            .into_iter()
            .map(|r| ContactCollectionRecord {
                id: r.id,
                name: r.name,
                owner_username: r.owner_username,
            })
            .collect())
    }

    pub async fn contact_collection_access(
        &self,
        username: &str,
        collection_id: i64,
    ) -> Result<ContactCollectionAccess> {
        let owner = sqlx::query_scalar!(
            "SELECT owner_username FROM contact_collections WHERE id = ?",
            collection_id
        )
        .fetch_optional(&self.pool)
        .await?;
        let Some(owner) = owner else {
            return Ok(ContactCollectionAccess::NotFound);
        };
        if owner == username {
            return Ok(ContactCollectionAccess::Allowed {
                owner_username: owner,
            });
        }
        let is_member = sqlx::query_scalar!(
            "SELECT 1 as \"x!: i64\" FROM contact_collection_members WHERE collection_id = ? AND member_username = ?",
            collection_id, username
        )
        .fetch_optional(&self.pool)
        .await?;
        match is_member {
            Some(_) => Ok(ContactCollectionAccess::Allowed {
                owner_username: owner,
            }),
            None => Ok(ContactCollectionAccess::Forbidden),
        }
    }

    async fn contact_methods_by_accessible(
        pool: &SqlitePool,
        table: &str,
        username: &str,
    ) -> Result<std::collections::HashMap<i64, Vec<ContactMethodRecord>>> {
        let query = format!(
            "SELECT m.contact_id, m.value, m.type, m.is_primary
             FROM {table} m
             JOIN contacts c ON c.id = m.contact_id
             JOIN contact_collections cc ON cc.id = c.collection_id
             WHERE cc.owner_username = ?
                OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?)
             ORDER BY m.is_primary DESC, m.id ASC"
        );
        let rows = sqlx::query(&query)
            .bind(username)
            .bind(username)
            .fetch_all(pool)
            .await?;
        let mut out: std::collections::HashMap<i64, Vec<ContactMethodRecord>> =
            std::collections::HashMap::new();
        for r in rows {
            let contact_id: i64 = r.get("contact_id");
            out.entry(contact_id)
                .or_default()
                .push(ContactMethodRecord {
                    value: r.get("value"),
                    kind: r.get("type"),
                    is_primary: r.get("is_primary"),
                });
        }
        Ok(out)
    }

    async fn contact_addresses_by_accessible(
        pool: &SqlitePool,
        username: &str,
    ) -> Result<std::collections::HashMap<i64, Vec<ContactAddressRecord>>> {
        let rows = sqlx::query(
            "SELECT a.contact_id, a.type, a.is_primary, a.street, a.city, a.region, a.postal_code, a.country
             FROM contact_addresses a
             JOIN contacts c ON c.id = a.contact_id
             JOIN contact_collections cc ON cc.id = c.collection_id
             WHERE cc.owner_username = ?
                OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?)
             ORDER BY a.is_primary DESC, a.id ASC",
        )
        .bind(username).bind(username).fetch_all(pool).await?;
        let mut out: std::collections::HashMap<i64, Vec<ContactAddressRecord>> =
            std::collections::HashMap::new();
        for r in rows {
            let contact_id: i64 = r.get("contact_id");
            out.entry(contact_id)
                .or_default()
                .push(ContactAddressRecord {
                    kind: r.get("type"),
                    is_primary: r.get("is_primary"),
                    street: r.get("street"),
                    city: r.get("city"),
                    region: r.get("region"),
                    postal_code: r.get("postal_code"),
                    country: r.get("country"),
                });
        }
        Ok(out)
    }

    async fn contact_categories_by_accessible(
        pool: &SqlitePool,
        username: &str,
    ) -> Result<std::collections::HashMap<i64, Vec<String>>> {
        let rows = sqlx::query(
            "SELECT g.contact_id, g.value
             FROM contact_categories g
             JOIN contacts c ON c.id = g.contact_id
             JOIN contact_collections cc ON cc.id = c.collection_id
             WHERE cc.owner_username = ?
                OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?)
             ORDER BY g.id ASC",
        )
        .bind(username).bind(username).fetch_all(pool).await?;
        let mut out: std::collections::HashMap<i64, Vec<String>> = std::collections::HashMap::new();
        for r in rows {
            let contact_id: i64 = r.get("contact_id");
            out.entry(contact_id).or_default().push(r.get("value"));
        }
        Ok(out)
    }

    pub async fn list_accessible_contacts(&self, username: &str) -> Result<Vec<ContactRecord>> {
        let rows = sqlx::query!(
            r#"SELECT c.id as "id!: i64", c.uid, c.display_name, c.given_name, c.family_name,
                      c.middle_name, c.name_prefix, c.name_suffix, c.nickname, c.org, c.title,
                      CAST(c.birthday AS TEXT) AS "birthday: String",
                      CAST(c.anniversary AS TEXT) AS "anniversary: String",
                      c.notes, c.etag, c.photo, c.photo_content_type
               FROM contacts c JOIN contact_collections cc ON cc.id = c.collection_id
               WHERE cc.owner_username = ?
                  OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?)
               ORDER BY c.id ASC"#,
            username, username
        )
        .fetch_all(&self.pool)
        .await?;

        let mut emails_by_id =
            Self::contact_methods_by_accessible(&self.pool, "contact_emails", username).await?;
        let mut phones_by_id =
            Self::contact_methods_by_accessible(&self.pool, "contact_phones", username).await?;
        let mut urls_by_id =
            Self::contact_methods_by_accessible(&self.pool, "contact_urls", username).await?;
        let mut impps_by_id =
            Self::contact_methods_by_accessible(&self.pool, "contact_impps", username).await?;
        let mut addresses_by_id =
            Self::contact_addresses_by_accessible(&self.pool, username).await?;
        let mut categories_by_id =
            Self::contact_categories_by_accessible(&self.pool, username).await?;

        Ok(rows
            .into_iter()
            .map(|r| {
                let id = r.id;
                ContactRecord {
                    id,
                    uid: r.uid,
                    display_name: r.display_name,
                    given_name: r.given_name,
                    family_name: r.family_name,
                    middle_name: r.middle_name,
                    name_prefix: r.name_prefix,
                    name_suffix: r.name_suffix,
                    nickname: r.nickname,
                    org: r.org,
                    title: r.title,
                    birthday: r.birthday,
                    anniversary: r.anniversary,
                    notes: r.notes,
                    etag: r.etag,
                    categories: categories_by_id.remove(&id).unwrap_or_default(),
                    emails: emails_by_id.remove(&id).unwrap_or_default(),
                    phones: phones_by_id.remove(&id).unwrap_or_default(),
                    urls: urls_by_id.remove(&id).unwrap_or_default(),
                    impps: impps_by_id.remove(&id).unwrap_or_default(),
                    addresses: addresses_by_id.remove(&id).unwrap_or_default(),
                    photo: r.photo,
                    photo_content_type: r.photo_content_type,
                }
            })
            .collect())
    }

    pub async fn list_todos(&self, username: &str) -> Result<Vec<Event>> {
        let todos = sqlx::query_as!(
            Event,
            r#"SELECT
                id, username, calendar, uid,
                type as "event_type: String",
                summary, description, location,
                start_date as "start_date: DateTime<Utc>",
                end_date as "end_date: DateTime<Utc>",
                due_date as "due_date: DateTime<Utc>",
                priority, status, response_status, etag, ical_data
            FROM events
            WHERE username = ? AND type = 'VTODO'
            ORDER BY priority ASC, due_date ASC"#,
            username
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(todos)
    }

    // ── Calendar sharing ──────────────────────────────────────────────────────

    pub async fn create_share_invitation(
        &self,
        owner: &str,
        resource: ShareResource,
        invitee_email: &str,
        token: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        match resource {
            ShareResource::Calendar { name, permission } => {
                sqlx::query!(
                    "INSERT INTO share_invitations
                         (owner_username, resource_type, calendar_name, invitee_email, permission, token, expires_at)
                     VALUES (?, 'calendar', ?, ?, ?, ?, ?)",
                    owner, name, invitee_email, permission, token, expires_at
                )
                .execute(&self.pool)
                .await?;
            }
            ShareResource::NoteCollection { id } => {
                sqlx::query!(
                    "INSERT INTO share_invitations
                         (owner_username, resource_type, collection_id, invitee_email, permission, token, expires_at)
                     VALUES (?, 'note_collection', ?, ?, 'member', ?, ?)",
                    owner, id, invitee_email, token, expires_at
                )
                .execute(&self.pool)
                .await?;
            }
            ShareResource::ContactCollection { id } => {
                sqlx::query!(
                    "INSERT INTO share_invitations
                         (owner_username, resource_type, contact_collection_id, invitee_email, permission, token, expires_at)
                     VALUES (?, 'contact_collection', ?, ?, 'member', ?, ?)",
                    owner, id, invitee_email, token, expires_at
                )
                .execute(&self.pool)
                .await?;
            }
        }
        Ok(())
    }

    pub async fn get_share_invitation(&self, token: &str) -> Result<Option<ShareInvitation>> {
        let row = sqlx::query!(
            "SELECT owner_username, resource_type, calendar_name, collection_id, contact_collection_id, permission,
                    invitee_email, token, expires_at, status
             FROM share_invitations WHERE token = ?",
            token
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| {
            let resource = match r.resource_type.as_str() {
                "calendar" => ShareResource::Calendar {
                    name: r.calendar_name.unwrap_or_default(),
                    permission: r.permission,
                },
                "note_collection" => ShareResource::NoteCollection {
                    id: r.collection_id.unwrap_or_default(),
                },
                _ => ShareResource::ContactCollection {
                    id: r.contact_collection_id.unwrap_or_default(),
                },
            };
            ShareInvitation {
                owner_username: r.owner_username,
                resource,
                invitee_email: r.invitee_email,
                token: r.token,
                expires_at: r.expires_at.and_utc(),
                status: r.status,
            }
        }))
    }

    pub async fn accept_share_invitation(
        &self,
        token: &str,
        grantee_username: &str,
        grantee_email: &str,
    ) -> Result<String> {
        let mut tx = self.pool.begin().await?;

        let inv = sqlx::query!(
            "SELECT owner_username, resource_type, calendar_name, collection_id, contact_collection_id, invitee_email, permission, status, expires_at
             FROM share_invitations WHERE token = ?",
            token
        )
        .fetch_optional(&mut *tx)
        .await?
        .ok_or_else(|| anyhow::anyhow!("invitation not found"))?;

        if inv.status != "pending" {
            return Err(anyhow::anyhow!("invitation already {}", inv.status));
        }
        let now = Utc::now().naive_utc();
        if inv.expires_at < now {
            return Err(anyhow::anyhow!("invitation has expired"));
        }
        if inv.owner_username == grantee_username {
            return Err(anyhow::anyhow!("cannot accept your own share invitation"));
        }
        if !inv.invitee_email.eq_ignore_ascii_case(grantee_email) {
            return Err(anyhow::anyhow!(
                "this invitation was sent to a different email address"
            ));
        }

        sqlx::query!(
            "UPDATE share_invitations SET status = 'accepted' WHERE token = ?",
            token
        )
        .execute(&mut *tx)
        .await?;

        match inv.resource_type.as_str() {
            "calendar" => {
                let calendar_name = inv.calendar_name.clone().unwrap_or_default();
                sqlx::query!(
                    "INSERT INTO calendar_shares (owner_username, calendar_name, grantee_username, permission)
                     VALUES (?, ?, ?, ?)
                     ON CONFLICT(owner_username, calendar_name, grantee_username)
                     DO UPDATE SET permission = excluded.permission",
                    inv.owner_username, calendar_name, grantee_username, inv.permission
                )
                .execute(&mut *tx)
                .await?;
            }
            "note_collection" => {
                let collection_id = inv.collection_id.ok_or_else(|| {
                    anyhow::anyhow!("note-collection invitation missing collection_id")
                })?;
                sqlx::query!(
                    "INSERT OR IGNORE INTO note_collection_members (collection_id, member_username) VALUES (?, ?)",
                    collection_id, grantee_username
                )
                .execute(&mut *tx)
                .await?;
                sqlx::query!(
                    "UPDATE note_collections SET shared = 1, updated_at = datetime('now') WHERE id = ?",
                    collection_id
                )
                .execute(&mut *tx)
                .await?;
            }
            _ => {
                let contact_collection_id = inv.contact_collection_id.ok_or_else(|| {
                    anyhow::anyhow!("contact-collection invitation missing contact_collection_id")
                })?;
                sqlx::query!(
                    "INSERT OR IGNORE INTO contact_collection_members (collection_id, member_username) VALUES (?, ?)",
                    contact_collection_id, grantee_username
                )
                .execute(&mut *tx)
                .await?;
                sqlx::query!(
                    "UPDATE contact_collections SET shared = 1, updated_at = datetime('now') WHERE id = ?",
                    contact_collection_id
                )
                .execute(&mut *tx)
                .await?;
            }
        }

        tx.commit().await?;
        Ok(inv.owner_username)
    }

    pub async fn decline_share_invitation(&self, token: &str) -> Result<()> {
        sqlx::query!(
            "UPDATE share_invitations SET status = 'declined' WHERE token = ? AND status = 'pending'",
            token
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_shares_for_calendar(
        &self,
        owner: &str,
        calendar: &str,
    ) -> Result<Vec<ShareGrant>> {
        let rows = sqlx::query!(
            "SELECT id, owner_username, calendar_name, grantee_username, permission
             FROM calendar_shares
             WHERE owner_username = ? AND calendar_name = ?
             ORDER BY grantee_username",
            owner,
            calendar
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| ShareGrant {
                id: r.id.unwrap_or(0),
                owner_username: r.owner_username,
                calendar_name: r.calendar_name,
                grantee_username: r.grantee_username,
                permission: r.permission,
            })
            .collect())
    }

    pub async fn get_share_grant(
        &self,
        owner: &str,
        calendar: &str,
        grantee: &str,
    ) -> Result<Option<ShareGrant>> {
        let row = sqlx::query!(
            "SELECT id, owner_username, calendar_name, grantee_username, permission
             FROM calendar_shares
             WHERE owner_username = ? AND calendar_name = ? AND grantee_username = ?",
            owner,
            calendar,
            grantee
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| ShareGrant {
            id: r.id.unwrap_or(0),
            owner_username: r.owner_username,
            calendar_name: r.calendar_name,
            grantee_username: r.grantee_username,
            permission: r.permission,
        }))
    }

    /// Look up a share grant directly by its stable ID — used to resolve the
    /// `shared-{id}` CalDAV alias back to the real `(owner, calendar)` pair.
    pub async fn get_share_grant_by_id(&self, id: i64) -> Result<Option<ShareGrant>> {
        let row = sqlx::query!(
            "SELECT id, owner_username, calendar_name, grantee_username, permission
             FROM calendar_shares WHERE id = ?",
            id
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| ShareGrant {
            id: r.id,
            owner_username: r.owner_username,
            calendar_name: r.calendar_name,
            grantee_username: r.grantee_username,
            permission: r.permission,
        }))
    }

    pub async fn revoke_share(&self, owner: &str, calendar: &str, grantee: &str) -> Result<()> {
        sqlx::query!(
            "DELETE FROM calendar_shares
             WHERE owner_username = ? AND calendar_name = ? AND grantee_username = ?",
            owner,
            calendar,
            grantee
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn list_shared_with_user(&self, grantee: &str) -> Result<Vec<SharedCalendarInfo>> {
        let rows = sqlx::query!(
            "SELECT s.id, s.owner_username, s.calendar_name, s.permission,
                    c.display_name, c.color
             FROM calendar_shares s
             JOIN calendars c ON c.username = s.owner_username AND c.name = s.calendar_name
             WHERE s.grantee_username = ?
             ORDER BY s.owner_username, s.calendar_name",
            grantee
        )
        .fetch_all(&self.pool)
        .await?;

        Ok(rows
            .into_iter()
            .map(|r| SharedCalendarInfo {
                id: r.id.unwrap_or(0),
                owner_username: r.owner_username,
                calendar_name: r.calendar_name,
                calendar_display_name: r.display_name,
                calendar_color: r.color,
                permission: r.permission,
            })
            .collect())
    }

    // ── SMTP configuration ────────────────────────────────────────────────────

    pub async fn get_smtp_config(&self) -> Result<Option<SmtpConfigRow>> {
        let row = sqlx::query!(
            "SELECT host, port, tls_mode, username, password_encrypted, from_address, from_name
             FROM smtp_config WHERE id = 1"
        )
        .fetch_optional(&self.pool)
        .await?;

        Ok(row.map(|r| SmtpConfigRow {
            host: r.host,
            port: r.port,
            tls_mode: r.tls_mode,
            username: r.username,
            password_encrypted: r.password_encrypted,
            from_address: r.from_address,
            from_name: r.from_name,
        }))
    }

    pub async fn save_smtp_config(&self, cfg: &SmtpConfigRow) -> Result<()> {
        sqlx::query!(
            "INSERT INTO smtp_config
                 (id, host, port, tls_mode, username, password_encrypted, from_address, from_name, updated_at)
             VALUES (1, ?, ?, ?, ?, ?, ?, ?, datetime('now'))
             ON CONFLICT(id) DO UPDATE SET
                 host               = excluded.host,
                 port               = excluded.port,
                 tls_mode           = excluded.tls_mode,
                 username           = excluded.username,
                 password_encrypted = excluded.password_encrypted,
                 from_address       = excluded.from_address,
                 from_name          = excluded.from_name,
                 updated_at         = excluded.updated_at",
            cfg.host, cfg.port, cfg.tls_mode, cfg.username,
            cfg.password_encrypted, cfg.from_address, cfg.from_name
        )
        .execute(&self.pool)
        .await?;
        Ok(())
    }
}

// ── CalendarStore impl for Storage ────────────────────────────────────────────

#[async_trait]
impl CalendarStore for Storage {
    async fn create_calendar(
        &self,
        username: &str,
        calendar: &str,
        display_name: Option<&str>,
    ) -> Result<()> {
        self.create_calendar(username, calendar, display_name).await
    }
    async fn get_calendar(&self, username: &str, calendar: &str) -> Result<Option<Calendar>> {
        self.get_calendar(username, calendar).await
    }
    async fn list_calendars(&self, username: &str) -> Result<Vec<Calendar>> {
        self.list_calendars(username).await
    }
    async fn delete_calendar(&self, username: &str, calendar: &str) -> Result<()> {
        self.delete_calendar(username, calendar).await
    }
    async fn set_calendar_sync(
        &self,
        username: &str,
        calendar: &str,
        read_only: bool,
        sync_user: Option<&str>,
    ) -> Result<()> {
        self.set_calendar_sync(username, calendar, read_only, sync_user)
            .await
    }
    async fn set_calendar_sync_url(
        &self,
        username: &str,
        calendar: &str,
        url: Option<&str>,
    ) -> Result<()> {
        self.set_calendar_sync_url(username, calendar, url).await
    }
    async fn update_webcal_sync_status(
        &self,
        username: &str,
        calendar: &str,
        etag: Option<&str>,
        last_modified: Option<&str>,
        error: Option<&str>,
    ) -> Result<()> {
        self.update_webcal_sync_status(username, calendar, etag, last_modified, error)
            .await
    }
    async fn copy_calendar(
        &self,
        src_username: &str,
        src_calendar: &str,
        dst_username: &str,
        dst_calendar: &str,
        deep: bool,
    ) -> Result<bool> {
        self.copy_calendar(src_username, src_calendar, dst_username, dst_calendar, deep)
            .await
    }
    async fn store_event(
        &self,
        username: &str,
        calendar: &str,
        uid: &str,
        ical_data: &str,
        etag: &str,
    ) -> Result<()> {
        self.store_event(username, calendar, uid, ical_data, etag)
            .await
    }
    async fn sync_note_priority_from_mirror(&self, uid: &str, caldav_priority: i64) -> Result<()> {
        self.sync_note_priority_from_mirror(uid, caldav_priority)
            .await
    }
    async fn set_event_completed(
        &self,
        username: &str,
        calendar: &str,
        uid: &str,
        completed: bool,
    ) -> Result<()> {
        self.set_event_completed(username, calendar, uid, completed)
            .await
    }
    async fn complete_note_mirror_if_any(&self, uid: &str) -> Result<bool> {
        self.complete_note_mirror_if_any(uid).await
    }
    async fn sync_done_item_from_caldav(
        &self,
        username: &str,
        calendar: &str,
        uid: &str,
        completed: bool,
    ) -> Result<()> {
        self.sync_done_item_from_caldav(username, calendar, uid, completed)
            .await
    }
    async fn get_event(
        &self,
        username: &str,
        calendar: &str,
        uid: &str,
    ) -> Result<Option<(String, String)>> {
        self.get_event(username, calendar, uid).await
    }
    async fn delete_event(&self, username: &str, calendar: &str, uid: &str) -> Result<()> {
        self.delete_event(username, calendar, uid).await
    }
    async fn list_events(&self, username: &str, calendar: &str) -> Result<Vec<Event>> {
        self.list_events(username, calendar).await
    }
    async fn list_todos(&self, username: &str) -> Result<Vec<Event>> {
        self.list_todos(username).await
    }
    async fn copy_event(
        &self,
        src_username: &str,
        src_calendar: &str,
        src_uid: &str,
        dst_username: &str,
        dst_calendar: &str,
        dst_uid: &str,
    ) -> Result<Option<String>> {
        self.copy_event(
            src_username,
            src_calendar,
            src_uid,
            dst_username,
            dst_calendar,
            dst_uid,
        )
        .await
    }
    async fn move_event(
        &self,
        src_username: &str,
        src_calendar: &str,
        src_uid: &str,
        dst_username: &str,
        dst_calendar: &str,
        dst_uid: &str,
    ) -> Result<bool> {
        self.move_event(
            src_username,
            src_calendar,
            src_uid,
            dst_username,
            dst_calendar,
            dst_uid,
        )
        .await
    }
    async fn create_lock(
        &self,
        username: &str,
        resource_path: &str,
        scope: &str,
        depth: &str,
        owner_info: Option<&str>,
        timeout_secs: i64,
    ) -> Result<String> {
        self.create_lock(
            username,
            resource_path,
            scope,
            depth,
            owner_info,
            timeout_secs,
        )
        .await
    }
    async fn get_lock(&self, token: &str) -> Result<Option<LockInfo>> {
        self.get_lock(token).await
    }
    async fn get_effective_locks(&self, resource_path: &str) -> Result<Vec<LockInfo>> {
        self.get_effective_locks(resource_path).await
    }
    async fn get_locks_for_resource(&self, resource_path: &str) -> Result<Vec<LockInfo>> {
        self.get_locks_for_resource(resource_path).await
    }
    async fn delete_lock(&self, token: &str) -> Result<bool> {
        self.delete_lock(token).await
    }
    async fn refresh_lock(&self, token: &str, timeout_secs: i64) -> Result<bool> {
        self.refresh_lock(token, timeout_secs).await
    }
    async fn set_property(
        &self,
        username: &str,
        resource_path: &str,
        namespace: &str,
        name: &str,
        value: &str,
    ) -> Result<()> {
        self.set_property(username, resource_path, namespace, name, value)
            .await
    }
    async fn delete_property(
        &self,
        username: &str,
        resource_path: &str,
        namespace: &str,
        name: &str,
    ) -> Result<()> {
        self.delete_property(username, resource_path, namespace, name)
            .await
    }
    async fn get_properties(
        &self,
        username: &str,
        resource_path: &str,
    ) -> Result<Vec<(String, String, String)>> {
        self.get_properties(username, resource_path).await
    }
    async fn copy_properties(
        &self,
        src_username: &str,
        src_path: &str,
        dst_username: &str,
        dst_path: &str,
    ) -> Result<()> {
        self.copy_properties(src_username, src_path, dst_username, dst_path)
            .await
    }
    async fn delete_properties(&self, username: &str, resource_path: &str) -> Result<()> {
        self.delete_properties(username, resource_path).await
    }
    async fn update_calendar_color(
        &self,
        username: &str,
        calendar: &str,
        color: &str,
    ) -> Result<()> {
        self.update_calendar_color(username, calendar, color).await
    }
    async fn update_calendar_display_name(
        &self,
        username: &str,
        calendar: &str,
        display_name: &str,
    ) -> Result<()> {
        self.update_calendar_display_name(username, calendar, display_name)
            .await
    }
    async fn ensure_personal_calendar(&self, username: &str) -> Result<()> {
        self.ensure_personal_calendar(username).await
    }
    async fn get_sync_token(&self, username: &str, calendar: &str) -> Result<i64> {
        self.get_sync_token(username, calendar).await
    }
    async fn bump_sync_token(
        &self,
        username: &str,
        calendar: &str,
        uid: &str,
        operation: &str,
    ) -> Result<i64> {
        self.bump_sync_token(username, calendar, uid, operation)
            .await
    }
    async fn get_sync_changes(
        &self,
        username: &str,
        calendar: &str,
        since_token: i64,
        until_token: Option<i64>,
    ) -> Result<Vec<SyncChange>> {
        self.get_sync_changes(username, calendar, since_token, until_token)
            .await
    }

    // ── Sharing ───────────────────────────────────────────────────────────────

    async fn create_share_invitation(
        &self,
        owner: &str,
        resource: ShareResource,
        invitee_email: &str,
        token: &str,
        expires_at: DateTime<Utc>,
    ) -> Result<()> {
        self.create_share_invitation(owner, resource, invitee_email, token, expires_at)
            .await
    }
    async fn get_share_invitation(&self, token: &str) -> Result<Option<ShareInvitation>> {
        self.get_share_invitation(token).await
    }
    async fn accept_share_invitation(
        &self,
        token: &str,
        grantee_username: &str,
        grantee_email: &str,
    ) -> Result<String> {
        self.accept_share_invitation(token, grantee_username, grantee_email)
            .await
    }
    async fn decline_share_invitation(&self, token: &str) -> Result<()> {
        self.decline_share_invitation(token).await
    }
    async fn list_shares_for_calendar(
        &self,
        owner: &str,
        calendar: &str,
    ) -> Result<Vec<ShareGrant>> {
        self.list_shares_for_calendar(owner, calendar).await
    }
    async fn get_share_grant(
        &self,
        owner: &str,
        calendar: &str,
        grantee: &str,
    ) -> Result<Option<ShareGrant>> {
        self.get_share_grant(owner, calendar, grantee).await
    }
    async fn get_share_grant_by_id(&self, id: i64) -> Result<Option<ShareGrant>> {
        self.get_share_grant_by_id(id).await
    }
    async fn revoke_share(&self, owner: &str, calendar: &str, grantee: &str) -> Result<()> {
        self.revoke_share(owner, calendar, grantee).await
    }
    async fn list_shared_with_user(&self, grantee: &str) -> Result<Vec<SharedCalendarInfo>> {
        self.list_shared_with_user(grantee).await
    }
    async fn get_smtp_config(&self) -> Result<Option<SmtpConfigRow>> {
        self.get_smtp_config().await
    }
    async fn save_smtp_config(&self, cfg: &SmtpConfigRow) -> Result<()> {
        self.save_smtp_config(cfg).await
    }
}

#[async_trait]
impl NoteStore for Storage {
    async fn get_note_image(
        &self,
        username: &str,
        note_id: i64,
        image_id: i64,
    ) -> Result<Option<(Vec<u8>, String)>> {
        // Runtime query (not `query!`) so adding it needs no `.sqlx` cache
        // regeneration.
        let row = sqlx::query(
            "SELECT i.data, i.content_type
             FROM note_images i
             JOIN notes n ON n.id = i.note_id
             JOIN note_collections c ON c.id = n.collection_id
             WHERE i.id = ? AND i.note_id = ?
                AND (c.owner_username = ?
                     OR c.id IN (SELECT collection_id FROM note_collection_members WHERE member_username = ?))",
        )
        .bind(image_id)
        .bind(note_id)
        .bind(username)
        .bind(username)
        .fetch_optional(&self.pool)
        .await?;
        Ok(row.map(|r| {
            (
                r.get::<Vec<u8>, _>("data"),
                r.get::<String, _>("content_type"),
            )
        }))
    }
}

#[async_trait]
impl ContactStore for Storage {
    async fn list_contacts_in_collection(&self, collection_id: i64) -> Result<Vec<ContactRecord>> {
        self.list_contacts_in_collection(collection_id).await
    }
    async fn get_contact_by_uid(
        &self,
        collection_id: i64,
        uid: &str,
    ) -> Result<Option<ContactRecord>> {
        self.get_contact_by_uid(collection_id, uid).await
    }
    async fn store_contact_vcard(
        &self,
        owner: &str,
        collection_id: i64,
        uid: &str,
        fields: &tidl_carddav::VCardFields,
        etag: &str,
    ) -> Result<()> {
        self.store_contact_vcard(owner, collection_id, uid, fields, etag)
            .await
    }
    async fn delete_contact_by_uid(&self, collection_id: i64, uid: &str) -> Result<bool> {
        self.delete_contact_by_uid(collection_id, uid).await
    }
    async fn get_contact_photo_by_id(
        &self,
        username: &str,
        id: i64,
    ) -> Result<Option<(Vec<u8>, String)>> {
        self.get_contact_photo_by_id(username, id).await
    }
    async fn get_address_book_sync_token(&self, collection_id: i64) -> Result<i64> {
        self.get_address_book_sync_token(collection_id).await
    }
    async fn bump_address_book_sync_token(
        &self,
        collection_id: i64,
        uid: &str,
        operation: &str,
    ) -> Result<i64> {
        self.bump_address_book_sync_token(collection_id, uid, operation)
            .await
    }
    async fn get_contact_sync_changes(
        &self,
        collection_id: i64,
        since_token: i64,
        until_token: Option<i64>,
    ) -> Result<Vec<SyncChange>> {
        self.get_contact_sync_changes(collection_id, since_token, until_token)
            .await
    }
    async fn list_contact_collections_for(
        &self,
        username: &str,
    ) -> Result<Vec<ContactCollectionRecord>> {
        self.list_contact_collections_for(username).await
    }
    async fn contact_collection_access(
        &self,
        username: &str,
        collection_id: i64,
    ) -> Result<ContactCollectionAccess> {
        self.contact_collection_access(username, collection_id)
            .await
    }
    async fn ensure_default_contact_collection(&self, owner: &str) -> Result<i64> {
        self.ensure_default_contact_collection(owner).await
    }
    async fn list_accessible_contacts(&self, username: &str) -> Result<Vec<ContactRecord>> {
        self.list_accessible_contacts(username).await
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqliteConnectOptions;
    use std::str::FromStr;

    /// Build an in-memory SQLite pool and run all migrations on it.
    async fn test_pool() -> SqlitePool {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let pool = SqlitePool::connect_with(opts).await.unwrap();
        sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
        pool
    }

    /// Build a `Storage` backed by the given pool, using a throwaway temp directory.
    fn test_storage(pool: SqlitePool) -> (Storage, std::path::PathBuf) {
        let dir = std::env::temp_dir().join(format!("tidl-test-{}", uuid::Uuid::new_v4()));
        let storage = Storage {
            pool,
            data_dir: dir.to_str().unwrap().to_string(),
        };
        (storage, dir)
    }

    // ── ensure_personal_calendar ──────────────────────────────────────────────

    #[tokio::test]
    async fn ensure_personal_calendar_creates_it() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool);

        storage.ensure_personal_calendar("alice").await.unwrap();

        let cal = storage.get_calendar("alice", "personal").await.unwrap();
        assert!(cal.is_some());
        let cal = cal.unwrap();
        assert_eq!(cal.name, "personal");
        assert_eq!(cal.display_name, Some("Personal".to_string()));
        assert_eq!(cal.color, Some("#3b82f6".to_string()));
        assert!(!cal.read_only);
    }

    #[tokio::test]
    async fn ensure_personal_calendar_is_idempotent() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool);

        storage.ensure_personal_calendar("alice").await.unwrap();
        storage.ensure_personal_calendar("alice").await.unwrap(); // must not fail or duplicate

        let cals = storage.list_calendars("alice").await.unwrap();
        assert_eq!(cals.len(), 1, "should have exactly one personal calendar");
    }

    // ── create / get calendar ─────────────────────────────────────────────────

    #[tokio::test]
    async fn create_and_get_calendar_roundtrip() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool);

        storage
            .create_calendar("alice", "work", Some("Work"))
            .await
            .unwrap();

        let cal = storage.get_calendar("alice", "work").await.unwrap();
        assert!(cal.is_some());
        let cal = cal.unwrap();
        assert_eq!(cal.name, "work");
        assert_eq!(cal.display_name, Some("Work".to_string()));
    }

    #[tokio::test]
    async fn get_nonexistent_calendar_returns_none() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool);

        let result = storage.get_calendar("alice", "nonexistent").await.unwrap();
        assert!(result.is_none());
    }

    // ── list_calendars ────────────────────────────────────────────────────────

    #[tokio::test]
    async fn list_calendars_scoped_per_user() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool);

        storage
            .create_calendar("alice", "personal", Some("Personal"))
            .await
            .unwrap();
        storage
            .create_calendar("alice", "work", Some("Work"))
            .await
            .unwrap();
        storage
            .create_calendar("bob", "personal", Some("Personal"))
            .await
            .unwrap();

        let alice_cals = storage.list_calendars("alice").await.unwrap();
        assert_eq!(alice_cals.len(), 2);
        assert!(alice_cals.iter().any(|c| c.name == "personal"));
        assert!(alice_cals.iter().any(|c| c.name == "work"));

        let bob_cals = storage.list_calendars("bob").await.unwrap();
        assert_eq!(bob_cals.len(), 1);
        assert_eq!(bob_cals[0].name, "personal");
    }

    // ── delete_calendar ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn delete_calendar_removes_the_row() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool);

        storage
            .create_calendar("alice", "work", Some("Work"))
            .await
            .unwrap();
        storage.delete_calendar("alice", "work").await.unwrap();

        let cal = storage.get_calendar("alice", "work").await.unwrap();
        assert!(cal.is_none());
    }

    #[tokio::test]
    async fn delete_calendar_also_removes_its_events() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool);

        storage
            .create_calendar("alice", "work", Some("Work"))
            .await
            .unwrap();

        let ical = "BEGIN:VCALENDAR\r\nVERSION:2.0\r\n\
                    BEGIN:VEVENT\r\nUID:ev-1\r\nSUMMARY:Meeting\r\n\
                    END:VEVENT\r\nEND:VCALENDAR\r\n";
        storage
            .store_event("alice", "work", "ev-1", ical, "etag1")
            .await
            .unwrap();

        // Event should be present before deletion.
        assert!(storage
            .get_event("alice", "work", "ev-1")
            .await
            .unwrap()
            .is_some());

        storage.delete_calendar("alice", "work").await.unwrap();

        // Event must be gone after calendar deletion.
        assert!(storage
            .get_event("alice", "work", "ev-1")
            .await
            .unwrap()
            .is_none());
    }

    // ── update_calendar_color ─────────────────────────────────────────────────

    #[tokio::test]
    async fn update_calendar_color_persists() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool);

        storage
            .create_calendar("alice", "work", Some("Work"))
            .await
            .unwrap();
        storage
            .update_calendar_color("alice", "work", "#ff0000")
            .await
            .unwrap();

        let cal = storage
            .get_calendar("alice", "work")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cal.color, Some("#ff0000".to_string()));
    }

    #[tokio::test]
    async fn update_calendar_color_only_affects_correct_user_and_calendar() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool);

        storage
            .create_calendar("alice", "work", Some("Work"))
            .await
            .unwrap();
        storage
            .create_calendar("bob", "work", Some("Work"))
            .await
            .unwrap();

        storage
            .update_calendar_color("alice", "work", "#ff0000")
            .await
            .unwrap();

        // Bob's calendar must be unchanged.
        let bob_cal = storage.get_calendar("bob", "work").await.unwrap().unwrap();
        assert_ne!(bob_cal.color, Some("#ff0000".to_string()));
    }

    // ── update_calendar_display_name ──────────────────────────────────────────

    #[tokio::test]
    async fn update_calendar_display_name_persists() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool);

        storage
            .create_calendar("alice", "work", Some("Work"))
            .await
            .unwrap();
        storage
            .update_calendar_display_name("alice", "work", "Office")
            .await
            .unwrap();

        let cal = storage
            .get_calendar("alice", "work")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(cal.display_name, Some("Office".to_string()));
    }

    // ── store_event ───────────────────────────────────────────────────────────
    //
    // Regression coverage for buglist item 4: a raw CalDAV PUT re-storing an
    // existing VTODO must not clobber the app-managed columns that PUT body
    // has no way to supply — `source`/`source_file`/`source_line`/`topic`/
    // `kind` (which is how a note- or markdown-origin task keeps its "go to
    // source" link) and `status`/`completed_date` (which used to silently
    // un-complete an already-completed item on every external edit).

    fn vtodo_ical(uid: &str, summary: &str) -> String {
        format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nBEGIN:VTODO\r\nUID:{uid}\r\nSUMMARY:{summary}\r\nEND:VTODO\r\nEND:VCALENDAR\r\n"
        )
    }

    #[tokio::test]
    async fn store_event_update_preserves_app_managed_columns() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        storage
            .create_calendar("alice", "personal", None)
            .await
            .unwrap();

        // Simulate an existing note-mirrored, completed task — exactly the
        // shape `sync_mirror`/`complete_todo_item` would have left behind.
        sqlx::query(
            "INSERT INTO events
             (username, calendar, uid, type, summary, etag, ical_data,
              source, kind, status, completed_date)
             VALUES ('alice', 'personal', 'note-1', 'VTODO', 'Old summary',
                     'etag-0', 'old-ical', 'note', 'task', 'COMPLETED', datetime('now'))",
        )
        .execute(&pool)
        .await
        .unwrap();

        // An external CalDAV client re-PUTs the same resource with an
        // edited summary — its ical body can't know about `source`/`kind`/
        // `status` at all.
        storage
            .store_event(
                "alice",
                "personal",
                "note-1",
                &vtodo_ical("note-1", "New summary"),
                "etag-1",
            )
            .await
            .unwrap();

        let row = sqlx::query(
            "SELECT summary, source, kind, status FROM events WHERE username = 'alice' AND uid = 'note-1'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        use sqlx::Row;
        assert_eq!(
            row.try_get::<String, _>("summary").unwrap(),
            "New summary",
            "content fields still update"
        );
        assert_eq!(
            row.try_get::<String, _>("source").unwrap(),
            "note",
            "note mirror link must survive an external edit"
        );
        assert_eq!(
            row.try_get::<String, _>("kind").unwrap(),
            "task",
            "kind must survive an external edit"
        );
        assert_eq!(
            row.try_get::<String, _>("status").unwrap(),
            "COMPLETED",
            "completion must survive an external edit"
        );
    }

    #[tokio::test]
    async fn store_event_insert_creates_a_new_row() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        storage
            .create_calendar("alice", "personal", None)
            .await
            .unwrap();

        storage
            .store_event(
                "alice",
                "personal",
                "new-1",
                &vtodo_ical("new-1", "Brand new"),
                "etag-0",
            )
            .await
            .unwrap();

        let (ical_data, etag) = storage
            .get_event("alice", "personal", "new-1")
            .await
            .unwrap()
            .unwrap();
        assert!(ical_data.contains("Brand new"));
        assert_eq!(etag, "etag-0");
    }

    // ── set_event_completed / complete_note_mirror_if_any (backlog #71) ───────

    #[tokio::test]
    async fn set_event_completed_true_sets_status_and_completed_date() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        storage
            .create_calendar("alice", "personal", None)
            .await
            .unwrap();
        storage
            .store_event(
                "alice",
                "personal",
                "t1",
                &vtodo_ical("t1", "A task"),
                "etag-0",
            )
            .await
            .unwrap();

        storage
            .set_event_completed("alice", "personal", "t1", true)
            .await
            .unwrap();

        use sqlx::Row;
        let row = sqlx::query("SELECT status, completed_date FROM events WHERE uid = 't1'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.try_get::<String, _>("status").unwrap(), "COMPLETED");
        assert!(row
            .try_get::<Option<String>, _>("completed_date")
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn set_event_completed_false_clears_status_and_completed_date() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        storage
            .create_calendar("alice", "personal", None)
            .await
            .unwrap();
        storage
            .store_event(
                "alice",
                "personal",
                "t1",
                &vtodo_ical("t1", "A task"),
                "etag-0",
            )
            .await
            .unwrap();
        storage
            .set_event_completed("alice", "personal", "t1", true)
            .await
            .unwrap();

        storage
            .set_event_completed("alice", "personal", "t1", false)
            .await
            .unwrap();

        use sqlx::Row;
        let row = sqlx::query("SELECT status, completed_date FROM events WHERE uid = 't1'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.try_get::<String, _>("status").unwrap(), "NEEDS-ACTION");
        assert!(row
            .try_get::<Option<String>, _>("completed_date")
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn complete_note_mirror_if_any_clears_the_note_and_returns_true() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        let coll_id = seed_collection(&pool, "alice", "Personal").await;
        sqlx::query(
            "INSERT INTO notes (collection_id, author_username, text, kind, due_date, mirrored_event_uid)
             VALUES (?, 'alice', 'Buy milk', 'task', NULL, 'note-1')",
        )
        .bind(coll_id)
        .execute(&pool)
        .await
        .unwrap();

        let matched = storage.complete_note_mirror_if_any("note-1").await.unwrap();
        assert!(matched);

        use sqlx::Row;
        let row = sqlx::query(
            "SELECT kind, due_date, mirrored_event_uid FROM notes WHERE collection_id = ?",
        )
        .bind(coll_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(row.try_get::<Option<String>, _>("kind").unwrap().is_none());
        assert!(row
            .try_get::<Option<String>, _>("due_date")
            .unwrap()
            .is_none());
        assert!(row
            .try_get::<Option<String>, _>("mirrored_event_uid")
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn complete_note_mirror_if_any_is_a_no_op_for_a_plain_task() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());

        let matched = storage
            .complete_note_mirror_if_any("not-a-mirror")
            .await
            .unwrap();
        assert!(!matched);
    }

    // ── share grants ──────────────────────────────────────────────────────────

    #[tokio::test]
    async fn get_share_grant_by_id_returns_the_grant() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool);

        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();
        sqlx::query!(
            "INSERT INTO calendar_shares (owner_username, calendar_name, grantee_username, permission)
             VALUES (?, ?, ?, ?)",
            "alice", "family", "bob", "read-write"
        )
        .execute(&storage.pool)
        .await
        .unwrap();

        let share_id_opt = sqlx::query_scalar!(
            "SELECT id FROM calendar_shares WHERE owner_username = ? AND calendar_name = ? AND grantee_username = ?",
            "alice", "family", "bob"
        )
        .fetch_one(&storage.pool)
        .await
        .unwrap();
        let share_id = share_id_opt.unwrap_or(0);

        let grant = storage.get_share_grant_by_id(share_id).await.unwrap();
        assert!(grant.is_some());
        let grant = grant.unwrap();
        assert_eq!(grant.id, share_id);
        assert_eq!(grant.owner_username, "alice");
        assert_eq!(grant.calendar_name, "family");
        assert_eq!(grant.grantee_username, "bob");
        assert_eq!(grant.permission, "read-write");
    }

    #[tokio::test]
    async fn get_share_grant_by_id_returns_none_for_unknown_id() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool);

        let grant = storage.get_share_grant_by_id(999999).await.unwrap();
        assert!(grant.is_none());
    }

    // ── accept_share_invitation ───────────────────────────────────────────────

    #[tokio::test]
    async fn accept_share_invitation_creates_the_grant() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool);

        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();
        storage
            .create_share_invitation(
                "alice",
                ShareResource::Calendar {
                    name: "family".to_string(),
                    permission: "read".to_string(),
                },
                "bob@example.com",
                "tok1",
                Utc::now() + chrono::Duration::days(7),
            )
            .await
            .unwrap();

        let owner = storage
            .accept_share_invitation("tok1", "bob", "bob@example.com")
            .await
            .unwrap();
        assert_eq!(
            owner, "alice",
            "should return the invitation's owner so the caller can notify both sides"
        );

        let shares = storage
            .list_shares_for_calendar("alice", "family")
            .await
            .unwrap();
        assert_eq!(shares.len(), 1);
        assert_eq!(shares[0].grantee_username, "bob");
    }

    #[tokio::test]
    async fn accept_share_invitation_accepts_case_insensitive_email_match() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool);

        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();
        storage
            .create_share_invitation(
                "alice",
                ShareResource::Calendar {
                    name: "family".to_string(),
                    permission: "read".to_string(),
                },
                "Bob@Example.com",
                "tok1",
                Utc::now() + chrono::Duration::days(7),
            )
            .await
            .unwrap();

        storage
            .accept_share_invitation("tok1", "bob", "bob@example.com")
            .await
            .unwrap();

        let shares = storage
            .list_shares_for_calendar("alice", "family")
            .await
            .unwrap();
        assert_eq!(shares.len(), 1);
    }

    #[tokio::test]
    async fn accept_share_invitation_rejects_the_owner_accepting_their_own_invite() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool);

        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();
        storage
            .create_share_invitation(
                "alice",
                ShareResource::Calendar {
                    name: "family".to_string(),
                    permission: "read".to_string(),
                },
                "alice@example.com",
                "tok1",
                Utc::now() + chrono::Duration::days(7),
            )
            .await
            .unwrap();

        let result = storage
            .accept_share_invitation("tok1", "alice", "alice@example.com")
            .await;
        assert!(
            result.is_err(),
            "owner accepting their own invitation must be rejected"
        );

        // No grant should have been created, and the invitation should still
        // be pending (not consumed) so it can't be silently accepted twice.
        let shares = storage
            .list_shares_for_calendar("alice", "family")
            .await
            .unwrap();
        assert!(shares.is_empty());
        let inv = storage.get_share_invitation("tok1").await.unwrap().unwrap();
        assert_eq!(inv.status, "pending");
    }

    #[tokio::test]
    async fn accept_share_invitation_rejects_a_different_users_email() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool);

        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();
        storage
            .create_share_invitation(
                "alice",
                ShareResource::Calendar {
                    name: "family".to_string(),
                    permission: "read".to_string(),
                },
                "bob@example.com",
                "tok1",
                Utc::now() + chrono::Duration::days(7),
            )
            .await
            .unwrap();

        // Eve is a different registered user who obtained/guessed the token
        // and tries to accept an invitation addressed to Bob.
        let result = storage
            .accept_share_invitation("tok1", "eve", "eve@example.com")
            .await;
        assert!(
            result.is_err(),
            "accepting with an email that doesn't match the invitation must be rejected"
        );

        let shares = storage
            .list_shares_for_calendar("alice", "family")
            .await
            .unwrap();
        assert!(shares.is_empty());
        let inv = storage.get_share_invitation("tok1").await.unwrap().unwrap();
        assert_eq!(inv.status, "pending");
    }

    // ── accept_share_invitation (note_collection) ─────────────────────────────

    async fn seed_user(pool: &SqlitePool, username: &str) {
        let email = format!("{username}@example.com");
        sqlx::query!(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES (?, ?, ?, 'x', 0)",
            username, email, username
        )
        .execute(pool)
        .await
        .unwrap();
    }

    async fn seed_collection(pool: &SqlitePool, owner: &str, name: &str) -> i64 {
        sqlx::query_scalar!(
            "INSERT INTO note_collections (owner_username, name, color) VALUES (?, ?, '#000000') RETURNING id as \"id!\"",
            owner, name
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[tokio::test]
    async fn accept_share_invitation_note_collection_creates_membership_and_flips_shared() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());

        seed_user(&pool, "alice").await;
        seed_user(&pool, "bob").await;
        let collection_id = seed_collection(&pool, "alice", "Recipes").await;

        storage
            .create_share_invitation(
                "alice",
                ShareResource::NoteCollection { id: collection_id },
                "bob@example.com",
                "tok1",
                Utc::now() + chrono::Duration::days(7),
            )
            .await
            .unwrap();

        let owner = storage
            .accept_share_invitation("tok1", "bob", "bob@example.com")
            .await
            .unwrap();
        assert_eq!(owner, "alice");

        let is_member: Option<i64> = sqlx::query_scalar!(
            "SELECT 1 as \"x!: i64\" FROM note_collection_members WHERE collection_id = ? AND member_username = ?",
            collection_id, "bob"
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(
            is_member.is_some(),
            "bob should be a member of the collection"
        );

        let shared: bool = sqlx::query_scalar!(
            "SELECT shared FROM note_collections WHERE id = ?",
            collection_id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(shared, "collection should be flipped to shared");
    }

    #[tokio::test]
    async fn accept_share_invitation_note_collection_accepts_case_insensitive_email_match() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());

        seed_user(&pool, "alice").await;
        seed_user(&pool, "bob").await;
        let collection_id = seed_collection(&pool, "alice", "Recipes").await;

        storage
            .create_share_invitation(
                "alice",
                ShareResource::NoteCollection { id: collection_id },
                "Bob@Example.com",
                "tok1",
                Utc::now() + chrono::Duration::days(7),
            )
            .await
            .unwrap();

        storage
            .accept_share_invitation("tok1", "bob", "bob@example.com")
            .await
            .unwrap();

        let is_member: Option<i64> = sqlx::query_scalar!(
            "SELECT 1 as \"x!: i64\" FROM note_collection_members WHERE collection_id = ? AND member_username = ?",
            collection_id, "bob"
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(is_member.is_some());
    }

    #[tokio::test]
    async fn accept_share_invitation_note_collection_rejects_the_owner_accepting_their_own_invite()
    {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());

        seed_user(&pool, "alice").await;
        let collection_id = seed_collection(&pool, "alice", "Recipes").await;

        storage
            .create_share_invitation(
                "alice",
                ShareResource::NoteCollection { id: collection_id },
                "alice@example.com",
                "tok1",
                Utc::now() + chrono::Duration::days(7),
            )
            .await
            .unwrap();

        let result = storage
            .accept_share_invitation("tok1", "alice", "alice@example.com")
            .await;
        assert!(result.is_err());

        let is_member: Option<i64> = sqlx::query_scalar!(
            "SELECT 1 as \"x!: i64\" FROM note_collection_members WHERE collection_id = ? AND member_username = ?",
            collection_id, "alice"
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(is_member.is_none());
    }

    #[tokio::test]
    async fn accept_share_invitation_note_collection_rejects_a_different_users_email() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());

        seed_user(&pool, "alice").await;
        seed_user(&pool, "eve").await;
        let collection_id = seed_collection(&pool, "alice", "Recipes").await;

        storage
            .create_share_invitation(
                "alice",
                ShareResource::NoteCollection { id: collection_id },
                "bob@example.com",
                "tok1",
                Utc::now() + chrono::Duration::days(7),
            )
            .await
            .unwrap();

        let result = storage
            .accept_share_invitation("tok1", "eve", "eve@example.com")
            .await;
        assert!(result.is_err());

        let is_member: Option<i64> = sqlx::query_scalar!(
            "SELECT 1 as \"x!: i64\" FROM note_collection_members WHERE collection_id = ? AND member_username = ?",
            collection_id, "eve"
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(is_member.is_none());
    }

    // ── accept_share_invitation (contact_collection) ───────────────────────────

    async fn seed_contact_collection(pool: &SqlitePool, owner: &str, name: &str) -> i64 {
        sqlx::query_scalar!(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES (?, ?, '#000000') RETURNING id as \"id!\"",
            owner, name
        )
        .fetch_one(pool)
        .await
        .unwrap()
    }

    // ── Multi-address-book CardDAV exposure (backlog #81, phase 2) ─────────────

    #[tokio::test]
    async fn contact_collection_access_returns_owner_for_the_owner() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        let collection_id = seed_contact_collection(&pool, "alice", "Family").await;

        let result = storage
            .contact_collection_access("alice", collection_id)
            .await
            .unwrap();
        assert_eq!(
            result,
            ContactCollectionAccess::Allowed {
                owner_username: "alice".to_string()
            }
        );
    }

    #[tokio::test]
    async fn contact_collection_access_returns_owner_for_a_member() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        seed_user(&pool, "bob").await;
        let collection_id = seed_contact_collection(&pool, "alice", "Family").await;
        sqlx::query!(
            "INSERT INTO contact_collection_members (collection_id, member_username) VALUES (?, 'bob')",
            collection_id
        ).execute(&pool).await.unwrap();

        let result = storage
            .contact_collection_access("bob", collection_id)
            .await
            .unwrap();
        assert_eq!(
            result,
            ContactCollectionAccess::Allowed {
                owner_username: "alice".to_string()
            },
            "a member resolves to the collection's real owner"
        );
    }

    #[tokio::test]
    async fn contact_collection_access_returns_forbidden_for_a_non_member() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        seed_user(&pool, "eve").await;
        let collection_id = seed_contact_collection(&pool, "alice", "Family").await;

        let result = storage
            .contact_collection_access("eve", collection_id)
            .await
            .unwrap();
        assert_eq!(result, ContactCollectionAccess::Forbidden);
    }

    #[tokio::test]
    async fn contact_collection_access_returns_not_found_for_an_unknown_collection() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;

        let result = storage
            .contact_collection_access("alice", 999_999)
            .await
            .unwrap();
        assert_eq!(result, ContactCollectionAccess::NotFound);
    }

    #[tokio::test]
    async fn ensure_default_contact_collection_is_idempotent() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;

        let first = storage
            .ensure_default_contact_collection("alice")
            .await
            .unwrap();
        let second = storage
            .ensure_default_contact_collection("alice")
            .await
            .unwrap();
        assert_eq!(
            first, second,
            "a second call must return the same collection, not create a duplicate"
        );

        let count: i64 = sqlx::query_scalar!(
            "SELECT COUNT(*) as \"count!: i64\" FROM contact_collections WHERE owner_username = 'alice'"
        ).fetch_one(&pool).await.unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn list_contact_collections_for_includes_owned_and_member_collections() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        seed_user(&pool, "bob").await;
        let alice_owned = seed_contact_collection(&pool, "alice", "Family").await;
        let bob_owned = seed_contact_collection(&pool, "bob", "Work").await;
        sqlx::query!(
            "INSERT INTO contact_collection_members (collection_id, member_username) VALUES (?, 'alice')",
            bob_owned
        ).execute(&pool).await.unwrap();

        let list = storage.list_contact_collections_for("alice").await.unwrap();
        let ids: Vec<i64> = list.iter().map(|c| c.id).collect();
        assert!(ids.contains(&alice_owned));
        assert!(
            ids.contains(&bob_owned),
            "a collection alice is a member of, not owner of, must be included"
        );
        assert_eq!(list.len(), 2);
    }

    #[tokio::test]
    async fn list_accessible_contacts_includes_a_shared_collections_contacts() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        seed_user(&pool, "bob").await;
        let shared = seed_contact_collection(&pool, "alice", "Family").await;
        sqlx::query!(
            "INSERT INTO contact_collection_members (collection_id, member_username) VALUES (?, 'bob')",
            shared
        ).execute(&pool).await.unwrap();

        let fields = sample_vcard_fields("uid-1", "Jane Doe");
        storage
            .store_contact_vcard("alice", shared, "uid-1", &fields, "etag-1")
            .await
            .unwrap();

        let visible = storage.list_accessible_contacts("bob").await.unwrap();
        assert_eq!(visible.len(), 1);
        assert_eq!(visible[0].uid, "uid-1");
    }

    #[tokio::test]
    async fn accept_share_invitation_contact_collection_creates_membership_and_flips_shared() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());

        seed_user(&pool, "alice").await;
        seed_user(&pool, "bob").await;
        let collection_id = seed_contact_collection(&pool, "alice", "Family").await;

        storage
            .create_share_invitation(
                "alice",
                ShareResource::ContactCollection { id: collection_id },
                "bob@example.com",
                "tok1",
                Utc::now() + chrono::Duration::days(7),
            )
            .await
            .unwrap();

        let owner = storage
            .accept_share_invitation("tok1", "bob", "bob@example.com")
            .await
            .unwrap();
        assert_eq!(owner, "alice");

        let is_member: Option<i64> = sqlx::query_scalar!(
            "SELECT 1 as \"x!: i64\" FROM contact_collection_members WHERE collection_id = ? AND member_username = ?",
            collection_id, "bob"
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(
            is_member.is_some(),
            "bob should be a member of the collection"
        );

        let shared: bool = sqlx::query_scalar!(
            "SELECT shared FROM contact_collections WHERE id = ?",
            collection_id
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(shared, "collection should be flipped to shared");
    }

    #[tokio::test]
    async fn accept_share_invitation_contact_collection_accepts_case_insensitive_email_match() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());

        seed_user(&pool, "alice").await;
        seed_user(&pool, "bob").await;
        let collection_id = seed_contact_collection(&pool, "alice", "Family").await;

        storage
            .create_share_invitation(
                "alice",
                ShareResource::ContactCollection { id: collection_id },
                "Bob@Example.com",
                "tok1",
                Utc::now() + chrono::Duration::days(7),
            )
            .await
            .unwrap();

        storage
            .accept_share_invitation("tok1", "bob", "bob@example.com")
            .await
            .unwrap();

        let is_member: Option<i64> = sqlx::query_scalar!(
            "SELECT 1 as \"x!: i64\" FROM contact_collection_members WHERE collection_id = ? AND member_username = ?",
            collection_id, "bob"
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(is_member.is_some());
    }

    #[tokio::test]
    async fn accept_share_invitation_contact_collection_rejects_the_owner_accepting_their_own_invite(
    ) {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());

        seed_user(&pool, "alice").await;
        let collection_id = seed_contact_collection(&pool, "alice", "Family").await;

        storage
            .create_share_invitation(
                "alice",
                ShareResource::ContactCollection { id: collection_id },
                "alice@example.com",
                "tok1",
                Utc::now() + chrono::Duration::days(7),
            )
            .await
            .unwrap();

        let result = storage
            .accept_share_invitation("tok1", "alice", "alice@example.com")
            .await;
        assert!(result.is_err());

        let is_member: Option<i64> = sqlx::query_scalar!(
            "SELECT 1 as \"x!: i64\" FROM contact_collection_members WHERE collection_id = ? AND member_username = ?",
            collection_id, "alice"
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(is_member.is_none());
    }

    #[tokio::test]
    async fn accept_share_invitation_contact_collection_rejects_a_different_users_email() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());

        seed_user(&pool, "alice").await;
        seed_user(&pool, "eve").await;
        let collection_id = seed_contact_collection(&pool, "alice", "Family").await;

        storage
            .create_share_invitation(
                "alice",
                ShareResource::ContactCollection { id: collection_id },
                "bob@example.com",
                "tok1",
                Utc::now() + chrono::Duration::days(7),
            )
            .await
            .unwrap();

        let result = storage
            .accept_share_invitation("tok1", "eve", "eve@example.com")
            .await;
        assert!(result.is_err());

        let is_member: Option<i64> = sqlx::query_scalar!(
            "SELECT 1 as \"x!: i64\" FROM contact_collection_members WHERE collection_id = ? AND member_username = ?",
            collection_id, "eve"
        )
        .fetch_optional(&pool)
        .await
        .unwrap();
        assert!(is_member.is_none());
    }

    // ── Contacts / CardDAV (backlog #29) ──────────────────────────────────────

    fn sample_vcard_fields(uid: &str, display_name: &str) -> tidl_carddav::VCardFields {
        tidl_carddav::VCardFields {
            uid: uid.to_string(),
            display_name: display_name.to_string(),
            given_name: Some("Ada".to_string()),
            family_name: Some("Lovelace".to_string()),
            middle_name: None,
            name_prefix: None,
            name_suffix: None,
            nickname: None,
            org: Some("Analytical Engines".to_string()),
            title: None,
            birthday: Some("1815-12-10".to_string()),
            anniversary: None,
            notes: Some("First programmer".to_string()),
            categories: vec![],
            emails: vec![tidl_carddav::VCardMethod {
                value: "ada@example.com".to_string(),
                kind: "work".to_string(),
                is_primary: true,
            }],
            phones: vec![tidl_carddav::VCardMethod {
                value: "+44 20 1234".to_string(),
                kind: "mobile".to_string(),
                is_primary: false,
            }],
            urls: vec![],
            impps: vec![],
            addresses: vec![tidl_carddav::VCardAddress {
                kind: "home".to_string(),
                is_primary: true,
                street: None,
                city: Some("London".to_string()),
                region: None,
                postal_code: None,
                country: None,
            }],
            photo: Some(tidl_carddav::VCardPhoto {
                bytes: vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10],
                content_type: "image/jpeg".to_string(),
            }),
        }
    }

    #[tokio::test]
    async fn list_contacts_is_empty_for_a_user_with_none() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        let collection_id = storage
            .ensure_default_contact_collection("alice")
            .await
            .unwrap();

        let contacts = storage
            .list_contacts_in_collection(collection_id)
            .await
            .unwrap();
        assert!(contacts.is_empty());
    }

    #[tokio::test]
    async fn store_contact_vcard_creates_and_round_trips_fields_and_methods() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        let collection_id = storage
            .ensure_default_contact_collection("alice")
            .await
            .unwrap();

        let fields = sample_vcard_fields("uid-1", "Ada Lovelace");
        storage
            .store_contact_vcard("alice", collection_id, "uid-1", &fields, "\"etag-1\"")
            .await
            .unwrap();

        let record = storage
            .get_contact_by_uid(collection_id, "uid-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.display_name, "Ada Lovelace");
        assert_eq!(record.given_name.as_deref(), Some("Ada"));
        assert_eq!(record.birthday.as_deref(), Some("1815-12-10"));
        assert_eq!(record.etag, "\"etag-1\"");
        assert_eq!(record.emails.len(), 1);
        assert_eq!(record.emails[0].value, "ada@example.com");
        assert_eq!(record.emails[0].kind, "work");
        assert!(record.emails[0].is_primary);
        assert_eq!(record.phones.len(), 1);
        assert_eq!(record.phones[0].value, "+44 20 1234");
        assert_eq!(
            record.photo.as_deref(),
            Some(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10][..])
        );
        assert_eq!(record.photo_content_type.as_deref(), Some("image/jpeg"));

        let listed = storage
            .list_contacts_in_collection(collection_id)
            .await
            .unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].uid, "uid-1");
        assert_eq!(
            listed[0].photo.as_deref(),
            Some(&[0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10][..])
        );

        let (photo_bytes, photo_content_type) = storage
            .get_contact_photo_by_id("alice", record.id)
            .await
            .unwrap()
            .expect("photo present");
        assert_eq!(photo_bytes, vec![0xFF, 0xD8, 0xFF, 0xE0, 0x00, 0x10]);
        assert_eq!(photo_content_type, "image/jpeg");

        // A non-member and non-existent id both come back as `None`, not an
        // error — the route this backs treats both as a plain 404.
        seed_user(&pool, "eve").await;
        assert!(storage
            .get_contact_photo_by_id("eve", record.id)
            .await
            .unwrap()
            .is_none());
        assert!(storage
            .get_contact_photo_by_id("alice", record.id + 999)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn get_note_image_scopes_by_collection_access() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        seed_user(&pool, "bob").await;
        let collection_id = seed_collection(&pool, "alice", "Notes").await;

        let note_id: i64 = sqlx::query_scalar(
            "INSERT INTO notes (collection_id, author_username, text) VALUES (?, 'alice', 'hi') RETURNING id",
        )
        .bind(collection_id)
        .fetch_one(&pool)
        .await
        .unwrap();
        let image_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_images (note_id, data, content_type, position) VALUES (?, ?, 'image/png', 0) RETURNING id",
        )
        .bind(note_id)
        .bind(vec![1u8, 2, 3])
        .fetch_one(&pool)
        .await
        .unwrap();

        let (bytes, content_type) = storage
            .get_note_image("alice", note_id, image_id)
            .await
            .unwrap()
            .expect("image present for its collection's owner");
        assert_eq!(bytes, vec![1, 2, 3]);
        assert_eq!(content_type, "image/png");

        // A non-member, a mismatched note id, and a missing image all come
        // back as `None` — the route treats them all as 404.
        assert!(storage
            .get_note_image("bob", note_id, image_id)
            .await
            .unwrap()
            .is_none());
        assert!(storage
            .get_note_image("alice", note_id + 999, image_id)
            .await
            .unwrap()
            .is_none());
        assert!(storage
            .get_note_image("alice", note_id, image_id + 999)
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn store_contact_vcard_assigns_the_passed_collection() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        let collection_id = seed_contact_collection(&pool, "alice", "Family").await;

        let fields = sample_vcard_fields("uid-1", "Jane Doe");
        storage
            .store_contact_vcard("alice", collection_id, "uid-1", &fields, "etag-1")
            .await
            .unwrap();

        let stored_collection: Option<i64> =
            sqlx::query_scalar!("SELECT collection_id FROM contacts WHERE uid = 'uid-1'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(stored_collection, Some(collection_id));
    }

    #[tokio::test]
    async fn store_contact_vcard_update_rejects_a_mismatched_collection_id() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        let collection_id = storage
            .ensure_default_contact_collection("alice")
            .await
            .unwrap();
        let other_collection = seed_contact_collection(&pool, "alice", "Family").await;

        let fields = sample_vcard_fields("uid-1", "Jane Doe");
        storage
            .store_contact_vcard("alice", collection_id, "uid-1", &fields, "etag-1")
            .await
            .unwrap();

        // A second PUT for the same uid, targeting a *different* collection,
        // is a conflict — not a silent move. `collection_id` is the identity
        // boundary a uid is scoped to (see `store_contact_vcard`'s doc
        // comment); re-PUTting must target the same collection it's already
        // in.
        let updated_fields = sample_vcard_fields("uid-1", "Jane D.");
        let result = storage
            .store_contact_vcard(
                "alice",
                other_collection,
                "uid-1",
                &updated_fields,
                "etag-2",
            )
            .await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .downcast_ref::<UidOwnershipConflict>()
            .is_some());

        // The contact stays in its original collection, untouched.
        let collection_after: i64 = sqlx::query_scalar!(
            "SELECT collection_id as \"id!\" FROM contacts WHERE uid = 'uid-1'"
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(collection_after, collection_id);
    }

    #[tokio::test]
    async fn store_contact_vcard_update_replaces_and_can_clear_photo() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        let collection_id = storage
            .ensure_default_contact_collection("alice")
            .await
            .unwrap();

        let mut fields = sample_vcard_fields("uid-1", "Ada Lovelace");
        storage
            .store_contact_vcard("alice", collection_id, "uid-1", &fields, "\"etag-1\"")
            .await
            .unwrap();

        // A second PUT with a different photo replaces it, not accumulates —
        // full-replace semantics, same as every other field on this record.
        fields.photo = Some(tidl_carddav::VCardPhoto {
            bytes: vec![1, 2, 3],
            content_type: "image/png".to_string(),
        });
        storage
            .store_contact_vcard("alice", collection_id, "uid-1", &fields, "\"etag-2\"")
            .await
            .unwrap();
        let record = storage
            .get_contact_by_uid(collection_id, "uid-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.photo.as_deref(), Some(&[1u8, 2, 3][..]));
        assert_eq!(record.photo_content_type.as_deref(), Some("image/png"));

        // A PUT with no PHOTO property clears it.
        fields.photo = None;
        storage
            .store_contact_vcard("alice", collection_id, "uid-1", &fields, "\"etag-3\"")
            .await
            .unwrap();
        let record = storage
            .get_contact_by_uid(collection_id, "uid-1")
            .await
            .unwrap()
            .unwrap();
        assert!(record.photo.is_none());
        assert!(record.photo_content_type.is_none());
    }

    #[tokio::test]
    async fn store_contact_vcard_persists_extended_fields_and_child_lists() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        let collection_id = storage
            .ensure_default_contact_collection("alice")
            .await
            .unwrap();

        let mut fields = sample_vcard_fields("uid-1", "Ada Lovelace");
        fields.middle_name = Some("Augusta".to_string());
        fields.name_prefix = Some("Countess".to_string());
        fields.name_suffix = Some("I".to_string());
        fields.nickname = Some("Lady A".to_string());
        fields.anniversary = Some("1835-07-08".to_string());
        fields.categories = vec!["Colleague".to_string(), "VIP".to_string()];
        fields.urls = vec![tidl_carddav::VCardMethod {
            value: "https://ada.example".to_string(),
            kind: "home".to_string(),
            is_primary: true,
        }];
        fields.impps = vec![tidl_carddav::VCardMethod {
            value: "xmpp:ada@example.com".to_string(),
            kind: "other".to_string(),
            is_primary: false,
        }];
        fields.addresses = vec![
            tidl_carddav::VCardAddress {
                kind: "home".to_string(),
                is_primary: true,
                street: Some("1 Engine Row".to_string()),
                city: Some("London".to_string()),
                region: None,
                postal_code: Some("EC1".to_string()),
                country: Some("UK".to_string()),
            },
            tidl_carddav::VCardAddress {
                kind: "work".to_string(),
                is_primary: false,
                street: Some("2 Difference St".to_string()),
                city: Some("London".to_string()),
                region: None,
                postal_code: Some("EC2".to_string()),
                country: Some("UK".to_string()),
            },
        ];

        storage
            .store_contact_vcard("alice", collection_id, "uid-1", &fields, "\"etag-1\"")
            .await
            .unwrap();

        let record = storage
            .get_contact_by_uid(collection_id, "uid-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.middle_name.as_deref(), Some("Augusta"));
        assert_eq!(record.name_prefix.as_deref(), Some("Countess"));
        assert_eq!(record.name_suffix.as_deref(), Some("I"));
        assert_eq!(record.nickname.as_deref(), Some("Lady A"));
        assert_eq!(record.anniversary.as_deref(), Some("1835-07-08"));
        assert_eq!(
            record.categories,
            vec!["Colleague".to_string(), "VIP".to_string()]
        );
        assert_eq!(record.urls.len(), 1);
        assert_eq!(record.urls[0].value, "https://ada.example");
        assert_eq!(record.impps.len(), 1);
        assert_eq!(
            record.addresses.len(),
            2,
            "both addresses must survive, not just the last one"
        );
        assert_eq!(record.addresses[0].kind, "home");
        assert_eq!(record.addresses[0].street.as_deref(), Some("1 Engine Row"));
        assert_eq!(record.addresses[1].kind, "work");
        assert_eq!(
            record.addresses[1].street.as_deref(),
            Some("2 Difference St")
        );

        // list_contacts_in_collection must return the same data as
        // get_contact_by_uid — they're two separate query paths sharing no
        // code.
        let listed = storage
            .list_contacts_in_collection(collection_id)
            .await
            .unwrap();
        assert_eq!(listed[0].addresses.len(), 2);
        assert_eq!(listed[0].urls.len(), 1);
        assert_eq!(listed[0].categories.len(), 2);
    }

    #[tokio::test]
    async fn store_contact_vcard_replaces_addresses_and_urls_on_update_not_accumulate() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        let collection_id = storage
            .ensure_default_contact_collection("alice")
            .await
            .unwrap();

        let mut fields = sample_vcard_fields("uid-1", "Ada Lovelace");
        fields.addresses = vec![tidl_carddav::VCardAddress {
            kind: "home".to_string(),
            is_primary: true,
            street: Some("Old St".to_string()),
            city: None,
            region: None,
            postal_code: None,
            country: None,
        }];
        storage
            .store_contact_vcard("alice", collection_id, "uid-1", &fields, "\"etag-1\"")
            .await
            .unwrap();

        fields.addresses = vec![tidl_carddav::VCardAddress {
            kind: "work".to_string(),
            is_primary: true,
            street: Some("New St".to_string()),
            city: None,
            region: None,
            postal_code: None,
            country: None,
        }];
        storage
            .store_contact_vcard("alice", collection_id, "uid-1", &fields, "\"etag-2\"")
            .await
            .unwrap();

        let record = storage
            .get_contact_by_uid(collection_id, "uid-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            record.addresses.len(),
            1,
            "old address must be replaced, not accumulated"
        );
        assert_eq!(record.addresses[0].street.as_deref(), Some("New St"));
    }

    #[tokio::test]
    async fn store_contact_vcard_upserts_in_place_and_replaces_methods() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        let collection_id = storage
            .ensure_default_contact_collection("alice")
            .await
            .unwrap();

        storage
            .store_contact_vcard(
                "alice",
                collection_id,
                "uid-1",
                &sample_vcard_fields("uid-1", "Ada Lovelace"),
                "\"etag-1\"",
            )
            .await
            .unwrap();

        let mut updated = sample_vcard_fields("uid-1", "Ada King");
        updated.emails = vec![tidl_carddav::VCardMethod {
            value: "ada@new.example".to_string(),
            kind: "home".to_string(),
            is_primary: true,
        }];
        storage
            .store_contact_vcard("alice", collection_id, "uid-1", &updated, "\"etag-2\"")
            .await
            .unwrap();

        let contacts = storage
            .list_contacts_in_collection(collection_id)
            .await
            .unwrap();
        assert_eq!(
            contacts.len(),
            1,
            "PUT with the same uid must update in place, not duplicate"
        );
        assert_eq!(contacts[0].display_name, "Ada King");
        assert_eq!(contacts[0].etag, "\"etag-2\"");
        assert_eq!(
            contacts[0].emails.len(),
            1,
            "old email row must be replaced, not accumulated"
        );
        assert_eq!(contacts[0].emails[0].value, "ada@new.example");
    }

    #[tokio::test]
    async fn store_contact_vcard_rejects_a_uid_used_by_a_different_collection() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        seed_user(&pool, "eve").await;
        let alice_collection = storage
            .ensure_default_contact_collection("alice")
            .await
            .unwrap();
        let eve_collection = storage
            .ensure_default_contact_collection("eve")
            .await
            .unwrap();

        storage
            .store_contact_vcard(
                "alice",
                alice_collection,
                "shared-uid",
                &sample_vcard_fields("shared-uid", "Alice's contact"),
                "\"etag-1\"",
            )
            .await
            .unwrap();

        let result = storage
            .store_contact_vcard(
                "eve",
                eve_collection,
                "shared-uid",
                &sample_vcard_fields("shared-uid", "Eve's overwrite attempt"),
                "\"etag-2\"",
            )
            .await;

        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .downcast_ref::<UidOwnershipConflict>()
            .is_some());
        // Alice's contact must be untouched.
        let record = storage
            .get_contact_by_uid(alice_collection, "shared-uid")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(record.display_name, "Alice's contact");
        assert!(storage
            .get_contact_by_uid(eve_collection, "shared-uid")
            .await
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn delete_contact_by_uid_removes_the_contact_and_cascades_methods() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        let collection_id = storage
            .ensure_default_contact_collection("alice")
            .await
            .unwrap();
        storage
            .store_contact_vcard(
                "alice",
                collection_id,
                "uid-1",
                &sample_vcard_fields("uid-1", "Ada Lovelace"),
                "\"etag-1\"",
            )
            .await
            .unwrap();

        let deleted = storage
            .delete_contact_by_uid(collection_id, "uid-1")
            .await
            .unwrap();
        assert!(deleted);
        assert!(storage
            .get_contact_by_uid(collection_id, "uid-1")
            .await
            .unwrap()
            .is_none());

        let email_count: i64 =
            sqlx::query_scalar!("SELECT COUNT(*) as \"c!: i64\" FROM contact_emails")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            email_count, 0,
            "contact_emails rows must cascade-delete with the contact"
        );

        let deleted_again = storage
            .delete_contact_by_uid(collection_id, "uid-1")
            .await
            .unwrap();
        assert!(!deleted_again);
    }

    #[tokio::test]
    async fn bump_address_book_sync_token_increments_and_logs_a_change() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        let collection_id = storage
            .ensure_default_contact_collection("alice")
            .await
            .unwrap();

        assert_eq!(
            storage
                .get_address_book_sync_token(collection_id)
                .await
                .unwrap(),
            0
        );

        let t1 = storage
            .bump_address_book_sync_token(collection_id, "uid-1", "modified")
            .await
            .unwrap();
        assert_eq!(t1, 1);
        assert_eq!(
            storage
                .get_address_book_sync_token(collection_id)
                .await
                .unwrap(),
            1
        );

        let t2 = storage
            .bump_address_book_sync_token(collection_id, "uid-2", "modified")
            .await
            .unwrap();
        assert_eq!(t2, 2);

        let changes = storage
            .get_contact_sync_changes(collection_id, 0, None)
            .await
            .unwrap();
        assert_eq!(changes.len(), 2);
    }

    #[tokio::test]
    async fn get_contact_sync_changes_dedupes_to_the_latest_operation_per_uid() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        let collection_id = storage
            .ensure_default_contact_collection("alice")
            .await
            .unwrap();

        storage
            .bump_address_book_sync_token(collection_id, "uid-1", "modified")
            .await
            .unwrap();
        storage
            .bump_address_book_sync_token(collection_id, "uid-1", "modified")
            .await
            .unwrap();
        storage
            .bump_address_book_sync_token(collection_id, "uid-1", "deleted")
            .await
            .unwrap();

        let changes = storage
            .get_contact_sync_changes(collection_id, 0, None)
            .await
            .unwrap();
        assert_eq!(
            changes.len(),
            1,
            "only the latest operation for a repeatedly-changed uid should be reported"
        );
        assert_eq!(changes[0].uid, "uid-1");
        assert_eq!(changes[0].operation, "deleted");
    }

    #[tokio::test]
    async fn get_contact_sync_changes_only_returns_changes_after_since_token() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        let collection_id = storage
            .ensure_default_contact_collection("alice")
            .await
            .unwrap();

        storage
            .bump_address_book_sync_token(collection_id, "uid-1", "modified")
            .await
            .unwrap();
        let since = storage
            .get_address_book_sync_token(collection_id)
            .await
            .unwrap();
        storage
            .bump_address_book_sync_token(collection_id, "uid-2", "modified")
            .await
            .unwrap();

        let changes = storage
            .get_contact_sync_changes(collection_id, since, None)
            .await
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].uid, "uid-2");
    }

    /// CardDAV mirror of `get_sync_changes_same_token_ties_resolve_by_insertion_order`.
    #[tokio::test]
    async fn get_contact_sync_changes_same_token_ties_resolve_by_insertion_order() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        let collection_id = storage
            .ensure_default_contact_collection("alice")
            .await
            .unwrap();

        sqlx::query(
            "INSERT INTO contact_sync_log (collection_id, uid, operation, token) \
             VALUES (?, 'uid-1', 'modified', 2)",
        )
        .bind(collection_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO contact_sync_log (collection_id, uid, operation, token) \
             VALUES (?, 'uid-1', 'deleted', 2)",
        )
        .bind(collection_id)
        .execute(&pool)
        .await
        .unwrap();

        let changes = storage
            .get_contact_sync_changes(collection_id, 0, None)
            .await
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].operation, "deleted",
            "later insert at the same token must win"
        );
    }

    // ── deleting a calendar clears its sync_log ──────────────────────────────

    /// Deleting a calendar must drop its `sync_log` too. Otherwise the old
    /// tombstones outrank the entries a recreated calendar writes from a reset
    /// `sync_token`, which is the root cause the repair migration fixes.
    #[tokio::test]
    async fn delete_calendar_removes_its_sync_log() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;

        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();
        storage
            .store_event(
                "alice",
                "family",
                "e1",
                "BEGIN:VCALENDAR\r\nEND:VCALENDAR",
                "\"e1\"",
            )
            .await
            .unwrap();

        let before: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sync_log WHERE username = 'alice' AND calendar = 'family'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(before > 0, "store_event should have logged a change");

        storage.delete_calendar("alice", "family").await.unwrap();

        let after: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sync_log WHERE username = 'alice' AND calendar = 'family'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(after, 0, "deleting a calendar must drop its sync_log");
    }

    // ── stale CalDAV sync_log tombstone repair (migration 20261004000000) ─────

    /// Regression for a calendar that was deleted and recreated: the old life's
    /// `deleted` tombstone sits at a higher token than the surviving `modified`
    /// entry for an event that still exists, so `get_sync_changes` reports it
    /// as deleted and CalDAV clients hide it. The one-off migration must
    /// re-affirm the present event at a token above every existing log entry.
    #[tokio::test]
    async fn repair_migration_reaffirms_events_with_stale_delete_tombstones() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;

        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();
        storage
            .store_event(
                "alice",
                "family",
                "keep",
                "BEGIN:VCALENDAR\r\nEND:VCALENDAR",
                "\"etag-keep\"",
            )
            .await
            .unwrap();

        // Simulate the delete/recreate inversion: a recreated calendar's counter
        // is low (2), while its old life's tombstone sits far above it (5), so
        // the present event's `modified` entry is shadowed.
        sqlx::query(
            "UPDATE calendars SET sync_token = 2 WHERE username = 'alice' AND name = 'family'",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sync_log (username, calendar, uid, operation, token) \
             VALUES ('alice', 'family', 'keep', 'deleted', 5)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let before = storage
            .get_sync_changes("alice", "family", 0, None)
            .await
            .unwrap();
        assert_eq!(before.len(), 1);
        assert_eq!(
            before[0].operation, "deleted",
            "precondition: the stale tombstone wins before repair"
        );

        let repair_sql = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../migrations/20261004000000_repair_stale_caldav_sync_tombstones.sql"
        ))
        .expect("repair migration must be readable");
        sqlx::raw_sql(&repair_sql).execute(&pool).await.unwrap();

        let after = storage
            .get_sync_changes("alice", "family", 0, None)
            .await
            .unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(
            after[0].operation, "modified",
            "repair must re-affirm the present event"
        );

        let token = storage.get_sync_token("alice", "family").await.unwrap();
        assert!(
            token > 5,
            "sync_token must outrank the stale tombstone (got {token})"
        );
    }

    /// The repair is a no-op when the log is already consistent.
    #[tokio::test]
    async fn repair_migration_leaves_consistent_calendars_untouched() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;

        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();
        storage
            .store_event(
                "alice",
                "family",
                "keep",
                "BEGIN:VCALENDAR\r\nEND:VCALENDAR",
                "\"etag-keep\"",
            )
            .await
            .unwrap();
        let token_before = storage.get_sync_token("alice", "family").await.unwrap();

        let repair_sql = std::fs::read_to_string(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../migrations/20261004000000_repair_stale_caldav_sync_tombstones.sql"
        ))
        .unwrap();
        sqlx::raw_sql(&repair_sql).execute(&pool).await.unwrap();

        assert_eq!(
            storage.get_sync_token("alice", "family").await.unwrap(),
            token_before,
            "a consistent calendar must not have its token bumped"
        );
    }

    // ── CalDAV sync-token primitives ─────────────────────────────────────────

    #[tokio::test]
    async fn bump_sync_token_increments_and_logs_a_modified_change() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();

        let before = storage.get_sync_token("alice", "family").await.unwrap();
        let token = storage
            .bump_sync_token("alice", "family", "e1", "modified")
            .await
            .unwrap();
        assert_eq!(token, before + 1);
        assert_eq!(
            storage.get_sync_token("alice", "family").await.unwrap(),
            token
        );

        let changes = storage
            .get_sync_changes("alice", "family", before, None)
            .await
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].uid, "e1");
        assert_eq!(changes[0].operation, "modified");
    }

    #[tokio::test]
    async fn bump_sync_token_logs_a_deleted_change_at_the_new_token() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();

        storage
            .bump_sync_token("alice", "family", "e1", "deleted")
            .await
            .unwrap();

        let changes = storage
            .get_sync_changes("alice", "family", 0, None)
            .await
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].uid, "e1");
        assert_eq!(changes[0].operation, "deleted");
    }

    #[tokio::test]
    async fn bump_sync_token_is_a_noop_for_a_missing_calendar() {
        // The synthetic `__markdown__` label has no `calendars` row.
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());

        let token = storage
            .bump_sync_token("alice", "__markdown__", "md:1", "modified")
            .await
            .unwrap();
        assert_eq!(token, 0, "missing calendar must not invent a token");

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM sync_log WHERE username = 'alice' AND calendar = '__markdown__'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(count, 0, "nothing should be logged for a missing calendar");
    }

    #[tokio::test]
    async fn get_sync_token_returns_zero_for_a_missing_calendar() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool);
        assert_eq!(storage.get_sync_token("alice", "nope").await.unwrap(), 0);
    }

    #[tokio::test]
    async fn get_sync_changes_returns_only_changes_after_since_token() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();

        storage
            .bump_sync_token("alice", "family", "e1", "modified")
            .await
            .unwrap();
        let since = storage.get_sync_token("alice", "family").await.unwrap();
        storage
            .bump_sync_token("alice", "family", "e2", "modified")
            .await
            .unwrap();

        let changes = storage
            .get_sync_changes("alice", "family", since, None)
            .await
            .unwrap();
        assert_eq!(
            changes.len(),
            1,
            "changes at or before `since` must be excluded"
        );
        assert_eq!(changes[0].uid, "e2");
    }

    #[tokio::test]
    async fn get_sync_changes_respects_the_until_token_bound() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();

        storage
            .bump_sync_token("alice", "family", "e1", "modified")
            .await
            .unwrap();
        let mid = storage.get_sync_token("alice", "family").await.unwrap();
        storage
            .bump_sync_token("alice", "family", "e2", "modified")
            .await
            .unwrap();

        let changes = storage
            .get_sync_changes("alice", "family", 0, Some(mid))
            .await
            .unwrap();
        assert_eq!(changes.len(), 1, "changes after `until` must be excluded");
        assert_eq!(changes[0].uid, "e1");
    }

    #[tokio::test]
    async fn get_sync_changes_dedupes_to_the_latest_operation_per_uid() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();

        for op in ["modified", "modified", "deleted"] {
            storage
                .bump_sync_token("alice", "family", "e1", op)
                .await
                .unwrap();
        }

        let changes = storage
            .get_sync_changes("alice", "family", 0, None)
            .await
            .unwrap();
        assert_eq!(changes.len(), 1, "one row per uid");
        assert_eq!(changes[0].operation, "deleted");
    }

    #[tokio::test]
    async fn get_sync_changes_is_scoped_per_user_and_calendar() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        seed_user(&pool, "bob").await;
        storage
            .create_calendar("alice", "family", None)
            .await
            .unwrap();
        storage
            .create_calendar("alice", "work", None)
            .await
            .unwrap();
        storage
            .create_calendar("bob", "family", None)
            .await
            .unwrap();

        storage
            .bump_sync_token("alice", "family", "a-fam", "modified")
            .await
            .unwrap();
        storage
            .bump_sync_token("alice", "work", "a-work", "modified")
            .await
            .unwrap();
        storage
            .bump_sync_token("bob", "family", "b-fam", "modified")
            .await
            .unwrap();

        let changes = storage
            .get_sync_changes("alice", "family", 0, None)
            .await
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].uid, "a-fam");
    }

    /// Inserts two operations for the same uid at an identical token and
    /// asserts the later-inserted one wins — the `id DESC` tie-break.
    #[tokio::test]
    async fn get_sync_changes_same_token_ties_resolve_by_insertion_order() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();

        // Same token, modified inserted first, deleted second → deleted wins.
        sqlx::query(
            "INSERT INTO sync_log (username, calendar, uid, operation, token) \
             VALUES ('alice', 'family', 'e1', 'modified', 2)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sync_log (username, calendar, uid, operation, token) \
             VALUES ('alice', 'family', 'e1', 'deleted', 2)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let changes = storage
            .get_sync_changes("alice", "family", 0, None)
            .await
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].operation, "deleted",
            "later insert at the same token must win"
        );

        // Reverse insertion order at a fresh token → modified wins.
        sqlx::query(
            "INSERT INTO sync_log (username, calendar, uid, operation, token) \
             VALUES ('alice', 'family', 'e2', 'deleted', 3)",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO sync_log (username, calendar, uid, operation, token) \
             VALUES ('alice', 'family', 'e2', 'modified', 3)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let changes = storage
            .get_sync_changes("alice", "family", 2, None)
            .await
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].uid, "e2");
        assert_eq!(
            changes[0].operation, "modified",
            "later insert at the same token must win regardless of operation"
        );
    }

    #[tokio::test]
    async fn store_event_create_bumps_token_and_logs_modified() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();
        let before = storage.get_sync_token("alice", "family").await.unwrap();

        storage
            .store_event(
                "alice",
                "family",
                "e1",
                "BEGIN:VCALENDAR\r\nEND:VCALENDAR",
                "\"etag-e1\"",
            )
            .await
            .unwrap();

        assert_eq!(
            storage.get_sync_token("alice", "family").await.unwrap(),
            before + 1
        );
        let changes = storage
            .get_sync_changes("alice", "family", before, None)
            .await
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].uid, "e1");
        assert_eq!(changes[0].operation, "modified");
    }

    #[tokio::test]
    async fn store_event_update_bumps_token_and_logs_modified() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();

        storage
            .store_event(
                "alice",
                "family",
                "e1",
                "BEGIN:VCALENDAR\r\nEND:VCALENDAR",
                "\"etag-1\"",
            )
            .await
            .unwrap();
        let after_create = storage.get_sync_token("alice", "family").await.unwrap();

        storage
            .store_event(
                "alice",
                "family",
                "e1",
                "BEGIN:VCALENDAR\r\nEND:VCALENDAR",
                "\"etag-2\"",
            )
            .await
            .unwrap();

        assert_eq!(
            storage.get_sync_token("alice", "family").await.unwrap(),
            after_create + 1
        );
        let changes = storage
            .get_sync_changes("alice", "family", after_create, None)
            .await
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].uid, "e1");
        assert_eq!(changes[0].operation, "modified");
    }

    #[tokio::test]
    async fn delete_event_bumps_token_and_logs_deleted() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();
        storage
            .store_event(
                "alice",
                "family",
                "e1",
                "BEGIN:VCALENDAR\r\nEND:VCALENDAR",
                "\"etag-1\"",
            )
            .await
            .unwrap();
        let after_create = storage.get_sync_token("alice", "family").await.unwrap();

        storage.delete_event("alice", "family", "e1").await.unwrap();

        assert_eq!(
            storage.get_sync_token("alice", "family").await.unwrap(),
            after_create + 1
        );
        let changes = storage
            .get_sync_changes("alice", "family", after_create, None)
            .await
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].uid, "e1");
        assert_eq!(changes[0].operation, "deleted");
    }

    #[tokio::test]
    async fn delete_then_recreate_same_uid_leaves_modified_latest() {
        let pool = test_pool().await;
        let (storage, _dir) = test_storage(pool.clone());
        seed_user(&pool, "alice").await;
        storage
            .create_calendar("alice", "family", Some("Family"))
            .await
            .unwrap();

        storage
            .store_event(
                "alice",
                "family",
                "e1",
                "BEGIN:VCALENDAR\r\nEND:VCALENDAR",
                "\"e1\"",
            )
            .await
            .unwrap();
        storage.delete_event("alice", "family", "e1").await.unwrap();
        storage
            .store_event(
                "alice",
                "family",
                "e1",
                "BEGIN:VCALENDAR\r\nEND:VCALENDAR",
                "\"e1-again\"",
            )
            .await
            .unwrap();

        let changes = storage
            .get_sync_changes("alice", "family", 0, None)
            .await
            .unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(
            changes[0].operation, "modified",
            "re-created uid must resolve to modified"
        );
    }
}
