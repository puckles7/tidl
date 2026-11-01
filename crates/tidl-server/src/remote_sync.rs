//! Background sync loop for remote CalDAV sources.
//!
//! Spawned at server startup.  Every `TICK_SECS` it checks all enabled remote
//! CalDAV sources and syncs those whose per-source `sync_interval_secs` has elapsed.
//!
//! ## Sync semantics
//!
//! **Pull** (remote → local):
//! 1. Use RFC 6578 `sync-collection` with the stored `last_sync_token`.
//! 2. For each changed href: GET the event and `store_event` locally.
//! 3. For each deleted href: `delete_event` locally.
//! 4. Update `last_sync_token` to the value returned by the server.
//!
//! **Push** (local → remote):
//! 1. Query `get_sync_changes` since `last_push_token`.
//! 2. For each modified uid: PUT the current event to the remote.
//! 3. For each deleted uid: DELETE from the remote.
//! 4. Update `last_push_token` to the current local sync token.
//!
//! Conflict resolution: last-writer-wins via ETag.  A 412 Precondition Failed
//! on PUT means the remote has a newer version — we skip the push for that uid
//! and let the next pull bring the remote version in.
//!
//! **First sync of a source**: if its local calendar already has events
//! (e.g. an existing calendar was picked instead of a new one), they're
//! wiped before anything else happens — the merge then behaves exactly
//! like a fresh calendar, with no need to track which events belong to
//! this source versus predate it. See `wipe_calendar_events`.

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
use sqlx::{Row, SqlitePool};
use tidl_caldav::change::ChangeKind;
use tidl_web::components::cal_edit_modal::CALENDAR_PALETTE;
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, info, warn};

use crate::{caldav_client::CalDavClient, mailer::decrypt_password, storage::CalendarStore};

/// How often the loop wakes to check which sources are due for a sync.
const TICK_SECS: u64 = 10;

/// Outcome of syncing one source, used by callers to decide which SSE
/// `ChangeKind` to broadcast: event-only changes only need to invalidate the
/// events resource, but a newly-created local calendar (first sync of a
/// freshly-added source) also needs the calendar-list resource to refetch,
/// or the new calendar silently doesn't show up until the browser is
/// manually refreshed.
struct SyncOutcome {
    events_changed: bool,
    calendar_created: bool,
}

impl SyncOutcome {
    fn change_kind(&self) -> Option<ChangeKind> {
        match (self.calendar_created, self.events_changed) {
            (true, _) => Some(ChangeKind::All),
            (false, true) => Some(ChangeKind::Events),
            (false, false) => None,
        }
    }
}

/// Pick the first colour in the shared calendar palette not already used by
/// one of `username`'s other calendars, so a freshly auto-created local
/// mirror calendar doesn't default to the same colour as everything else.
/// Falls back to the first palette colour if every one is already taken.
async fn pick_unused_color(
    storage: &Arc<dyn CalendarStore + Send + Sync>,
    username: &str,
) -> &'static str {
    let used: std::collections::HashSet<String> = storage
        .list_calendars(username)
        .await
        .unwrap_or_default()
        .into_iter()
        .filter_map(|c| c.color)
        .collect();

    CALENDAR_PALETTE
        .iter()
        .find(|c| !used.contains(**c))
        .copied()
        .unwrap_or(CALENDAR_PALETTE[0])
}

/// Delete every event currently in `calendar` — used when a remote source
/// attaches to a pre-existing calendar for the first time (see `pull()`),
/// so the merge behaves like a fresh calendar rather than needing to track
/// which events belong to this source versus predate it. No-op for an
/// empty or nonexistent calendar.
async fn wipe_calendar_events(
    storage: &Arc<dyn CalendarStore + Send + Sync>,
    username: &str,
    calendar: &str,
) {
    if let Ok(existing_events) = storage.list_events(username, calendar).await {
        for ev in existing_events {
            let _ = storage.delete_event(username, calendar, &ev.uid).await;
        }
    }
}

/// Seed `source.last_push_token` to the local calendar's *current* sync
/// token if it hasn't been set yet — the first sync of this source, or the
/// first sync since its local calendar was changed. By this point any
/// pre-existing events in the calendar have already been wiped (see the
/// caller in `pull()`), so "current" just means "after that wipe" — this
/// only exists so the wipe's own delete tombstones aren't themselves
/// mistaken for new changes and pushed/reconciled on the very next step.
/// No-op if already set. Updates both the DB row and the in-memory
/// `source`, since `pull()`/`push()` read the in-memory value for the rest
/// of this sync cycle.
async fn seed_last_push_token_if_unset(
    pool: &SqlitePool,
    storage: &Arc<dyn CalendarStore + Send + Sync>,
    source: &mut SourceRow,
) -> Result<()> {
    if source.last_push_token.is_some() {
        return Ok(());
    }

    let seed_token = storage
        .get_sync_token(&source.username, &source.local_calendar)
        .await
        .unwrap_or(0);

    sqlx::query("UPDATE remote_caldav_sources SET last_push_token = ? WHERE id = ?")
        .bind(seed_token)
        .bind(&source.id)
        .execute(pool)
        .await
        .context("seeding last_push_token")?;

    source.last_push_token = Some(seed_token);
    Ok(())
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run_sync_loop(
    pool: SqlitePool,
    storage: Arc<dyn CalendarStore + Send + Sync>,
    server_secret: String,
    change_tx: broadcast::Sender<String>,
    mut trigger_rx: mpsc::UnboundedReceiver<String>,
) {
    // Tracks when each source (by id) should next be synced.
    // Absent entry ⇒ never synced ⇒ sync immediately on first tick.
    let mut next_sync_at: HashMap<String, Instant> = HashMap::new();

    loop {
        let tick = tokio::time::sleep(Duration::from_secs(TICK_SECS));
        tokio::pin!(tick);
        tokio::select! {
            _ = &mut tick => {
                let sources = match load_sources(&pool).await {
                    Ok(s) => s,
                    Err(e) => {
                        error!("remote_sync: failed to load sources: {e}");
                        continue;
                    }
                };

                debug!(count = sources.len(), "remote_sync: tick — checking sources");

                for mut source in sources {
                    let due = next_sync_at
                        .get(&source.id)
                        .is_none_or(|t| Instant::now() >= *t);

                    if due {
                        let id = source.id.clone();
                        let interval = Duration::from_secs(source.sync_interval_secs as u64);
                        let username = source.username.clone();
                        let local_calendar = source.local_calendar.clone();
                        match sync_source(&pool, &storage, &server_secret, &mut source).await {
                            Ok(outcome) => {
                                if let Some(kind) = outcome.change_kind() {
                                    crate::notify::notify_calendar_watchers(
                                        &storage, &change_tx, &username, &local_calendar, kind,
                                    ).await;
                                }
                            }
                            Err(e) => {
                                warn!(source_id = %id, "remote_sync: sync failed: {e:#}");
                                let _ = set_source_error(&pool, &id, &e.to_string()).await;
                            }
                        }
                        next_sync_at.insert(id, Instant::now() + interval);
                    }
                }
            }
            Some(source_id) = trigger_rx.recv() => {
                sync_single(&pool, &storage, &server_secret, &change_tx, &source_id).await;
                // Reset the timer so the next scheduled sync runs at the correct interval
                // from now, rather than treating the absent entry as "never synced".
                if let Ok(Some(src)) = load_source_by_id(&pool, &source_id).await {
                    let interval = Duration::from_secs(src.sync_interval_secs as u64);
                    next_sync_at.insert(source_id, Instant::now() + interval);
                }
            }
        }
    }
}

// ── Core sync logic ───────────────────────────────────────────────────────────

/// Sync one source by ID (used for immediate post-add trigger).
async fn sync_single(
    pool: &SqlitePool,
    storage: &Arc<dyn CalendarStore + Send + Sync>,
    server_secret: &str,
    change_tx: &broadcast::Sender<String>,
    source_id: &str,
) {
    match load_source_by_id(pool, source_id).await {
        Ok(Some(mut source)) => {
            let id = source.id.clone();
            let username = source.username.clone();
            let local_calendar = source.local_calendar.clone();
            match sync_source(pool, storage, server_secret, &mut source).await {
                Ok(outcome) => {
                    if let Some(kind) = outcome.change_kind() {
                        crate::notify::notify_calendar_watchers(
                            storage,
                            change_tx,
                            &username,
                            &local_calendar,
                            kind,
                        )
                        .await;
                    }
                }
                Err(e) => {
                    warn!(source_id = %id, "remote_sync: triggered sync failed: {e:#}");
                    let _ = set_source_error(pool, &id, &e.to_string()).await;
                }
            }
        }
        Ok(None) => {
            warn!(source_id = %source_id, "remote_sync: triggered source not found");
        }
        Err(e) => {
            error!(source_id = %source_id, "remote_sync: failed to load triggered source: {e}");
        }
    }
}

/// Sync a single source. See [`SyncOutcome`] for what the caller should do
/// with the result.
async fn sync_source(
    pool: &SqlitePool,
    storage: &Arc<dyn CalendarStore + Send + Sync>,
    server_secret: &str,
    source: &mut SourceRow,
) -> Result<SyncOutcome> {
    let token = decrypt_password(&source.remote_token_enc, server_secret)
        .context("decrypting remote token")?;

    let client = CalDavClient::new(&source.remote_base_url, &source.remote_username, &token)
        .context("building CalDAV client")?;

    info!(
        source = %source.name,
        direction = %source.sync_direction,
        "remote_sync: starting sync"
    );

    // Owned rather than borrowed so `source` remains free to pass as `&mut` below.
    let direction = source.sync_direction.clone();
    let mut changed = false;
    let mut calendar_created = false;

    // For "both" direction: snapshot the local sync token BEFORE pull.
    // This becomes the upper bound for push so that events stored during pull
    // (which land at tokens > snapshot) are excluded from this push cycle.
    // Push then processes only locally-made changes in (last_push_token, snapshot].
    let pre_pull_token: Option<i64> = if direction == "both" {
        storage
            .get_sync_token(&source.username, &source.local_calendar)
            .await
            .ok()
    } else {
        None
    };

    if direction == "pull" || direction == "both" {
        let pull_only = direction == "pull";
        let outcome = pull(pool, storage, &client, source, pull_only).await?;
        changed = outcome.events_changed;
        calendar_created = outcome.calendar_created;
    }

    if direction == "push" || direction == "both" {
        push(pool, storage, &client, source, pre_pull_token).await?;
    }

    // Update last_synced_at and clear any previous error.
    sqlx::query(
        "UPDATE remote_caldav_sources
         SET last_synced_at = CURRENT_TIMESTAMP, last_error = NULL
         WHERE id = ?",
    )
    .bind(&source.id)
    .execute(pool)
    .await
    .context("updating last_synced_at")?;

    Ok(SyncOutcome {
        events_changed: changed,
        calendar_created,
    })
}

// ── Pull ──────────────────────────────────────────────────────────────────────

/// Pull remote changes into the local calendar via RFC 6578 `sync-collection`.
///
/// Creates the local calendar on first sync if it doesn't exist, wipes any
/// pre-existing events in it if it did (see [`wipe_calendar_events`]), then
/// applies the remote's changes. For `pull_only` sources, also reconciles
/// local edits/deletions made since the last sync against the remote
/// (remote wins). See [`SyncOutcome`] for what the caller does with the result.
async fn pull(
    pool: &SqlitePool,
    storage: &Arc<dyn CalendarStore + Send + Sync>,
    client: &CalDavClient,
    source: &mut SourceRow,
    pull_only: bool,
) -> Result<SyncOutcome> {
    // Owned rather than borrowed so `source.last_push_token` can be seeded
    // in place below without fighting the borrow checker.
    let username = source.username.clone();
    let calendar = source.local_calendar.clone();

    // Snapshot local token BEFORE the incremental sync so that changes written by
    // the sync itself (store_event bumps the local token) are excluded from the
    // reconciliation window below.
    let local_before_pull = if pull_only {
        storage
            .get_sync_token(&username, &calendar)
            .await
            .ok()
            .unwrap_or(0)
    } else {
        0
    };

    let sync_result = client
        .sync_collection(
            &source.remote_username,
            &source.remote_calendar,
            source.last_sync_token.as_deref(),
        )
        .await
        .context("sync-collection REPORT")?;

    let new_token = sync_result.new_sync_token;

    // Ensure the local calendar exists.
    let mut calendar_created = false;
    if storage.get_calendar(&username, &calendar).await?.is_none() {
        storage
            .create_calendar(&username, &calendar, Some(&calendar))
            .await
            .context("creating local calendar")?;
        let color = pick_unused_color(storage, &username).await;
        storage
            .update_calendar_color(&username, &calendar, color)
            .await
            .context("setting new calendar color")?;
        calendar_created = true;
    }

    // First sync of this source: attaching it to an existing calendar wipes
    // whatever was already in it, so from here on the merge behaves exactly
    // like a fresh calendar — no per-event provenance tracking is needed to
    // tell "belongs to this remote source" apart from "pre-existing local
    // content". The UI warns about this before saving. No-op for a calendar
    // just created above (nothing to wipe).
    if source.last_push_token.is_none() {
        wipe_calendar_events(storage, &username, &calendar).await;
    }

    seed_last_push_token_if_unset(pool, storage, source).await?;

    let mut changed = false;

    for change in &sync_result.changes {
        let status = change.status.as_deref().unwrap_or("HTTP/1.1 200 OK");
        let uid = uid_from_href(&change.href);

        if status.contains("404") || status.contains("410") {
            // Deleted on remote.
            let _ = storage.delete_event(&username, &calendar, &uid).await;
            debug!(%uid, "remote_sync: pull deleted");
            changed = true;
        } else {
            // Modified or new — fetch and store.
            match client
                .get_event(&source.remote_username, &source.remote_calendar, &uid)
                .await
            {
                Ok((ical, etag)) => {
                    use tidl_caldav::etag::generate_etag;
                    let local_etag = etag.trim_matches('"');
                    let stored_etag = if local_etag.is_empty() {
                        generate_etag(&ical)
                    } else {
                        local_etag.to_string()
                    };
                    storage
                        .store_event(&username, &calendar, &uid, &ical, &stored_etag)
                        .await
                        .context("storing pulled event")?;
                    debug!(%uid, "remote_sync: pull stored");
                    changed = true;
                }
                Err(e) => warn!(%uid, "remote_sync: GET event failed: {e}"),
            }
        }
    }

    // Persist the new sync token.
    if let Some(token) = &new_token {
        sqlx::query("UPDATE remote_caldav_sources SET last_sync_token = ? WHERE id = ?")
            .bind(token)
            .bind(&source.id)
            .execute(pool)
            .await
            .context("saving new sync token")?;
    }

    // Pull-only reconciliation: the remote is the source of truth, so any local
    // edit or deletion that happened between syncs should be overridden.
    //
    // We look at local changes in (last_push_token, local_before_pull] — i.e.
    // only changes that existed BEFORE this sync ran.  Changes written by the
    // incremental sync above (tokens > local_before_pull) are intentional and
    // must not be re-processed.
    //
    // `last_push_token` is unused for pull-only sources; we repurpose it to
    // track the last local sync_token that was reconciled.
    if pull_only {
        let local_since = source.last_push_token.unwrap_or(0);

        if local_since < local_before_pull {
            let local_changes = storage
                .get_sync_changes(&username, &calendar, local_since, Some(local_before_pull))
                .await
                .context("getting local sync changes for reconciliation")?;

            for lc in local_changes {
                use tidl_caldav::etag::generate_etag;
                match client
                    .get_event(&source.remote_username, &source.remote_calendar, &lc.uid)
                    .await
                {
                    Ok((ical, etag)) => {
                        // Remote still has this event — restore/overwrite the local copy.
                        let local_etag = etag.trim_matches('"');
                        let stored_etag = if local_etag.is_empty() {
                            generate_etag(&ical)
                        } else {
                            local_etag.to_string()
                        };
                        storage
                            .store_event(&username, &calendar, &lc.uid, &ical, &stored_etag)
                            .await
                            .context("restoring locally-changed event")?;
                        debug!(uid = %lc.uid, op = %lc.operation,
                               "remote_sync: pull reconciled local change");
                        changed = true;
                    }
                    Err(_) => {
                        // Remote doesn't have it.  If we had locally modified (not
                        // deleted) it, the remote deletion wins — delete locally too.
                        if lc.operation != "deleted" {
                            let _ = storage.delete_event(&username, &calendar, &lc.uid).await;
                            debug!(uid = %lc.uid,
                                   "remote_sync: pull removed locally-modified event absent on remote");
                            changed = true;
                        }
                        // If local op was already "deleted" and remote agrees → nothing to do.
                    }
                }
            }
        }

        // Advance the reconciliation cursor to the current local token (includes
        // any store/delete calls made by the reconciliation itself).
        let local_after = storage
            .get_sync_token(&username, &calendar)
            .await
            .context("getting post-reconciliation local token")?;

        sqlx::query("UPDATE remote_caldav_sources SET last_push_token = ? WHERE id = ?")
            .bind(local_after)
            .bind(&source.id)
            .execute(pool)
            .await
            .context("saving last_push_token for pull reconciliation")?;
    }

    Ok(SyncOutcome {
        events_changed: changed,
        calendar_created,
    })
}

// ── Push ──────────────────────────────────────────────────────────────────────

async fn push(
    pool: &SqlitePool,
    storage: &Arc<dyn CalendarStore + Send + Sync>,
    client: &CalDavClient,
    source: &SourceRow,
    // For "both" direction: sync token snapshotted before the pull phase ran.
    // Used as the UPPER bound so pull-created entries are excluded from the push.
    push_until: Option<i64>,
) -> Result<()> {
    let username = &source.username;
    let calendar = &source.local_calendar;

    // Determine the current local sync token (post-pull; saved as last_push_token).
    let current_token = storage
        .get_sync_token(username, calendar)
        .await
        .context("getting local sync token")?;

    // Push only locally-made changes in (last_push_token, push_until].
    // When push_until is None (push-only direction) there is no upper bound.
    let since = source.last_push_token.unwrap_or(0);
    let changes = storage
        .get_sync_changes(username, calendar, since, push_until)
        .await
        .context("getting local sync changes")?;

    for change in &changes {
        let uid = &change.uid;

        if change.operation == "deleted" {
            if let Err(e) = client
                .delete_event(&source.remote_username, &source.remote_calendar, uid, None)
                .await
            {
                warn!(%uid, "remote_sync: push delete failed: {e}");
            } else {
                debug!(%uid, "remote_sync: push deleted");
            }
        } else {
            // Modified or new — fetch local data and PUT.
            match storage.get_event(username, calendar, uid).await? {
                Some((ical, local_etag)) => {
                    // Optimistic: use If-Match with the local ETag, which is the
                    // ETag we'd have set after the last pull.  A 412 means the
                    // remote already has a newer version — skip.
                    match client
                        .put_event(
                            &source.remote_username,
                            &source.remote_calendar,
                            uid,
                            &ical,
                            Some("*"), // create-or-overwrite unconditionally
                        )
                        .await
                    {
                        Ok(_) => debug!(%uid, "remote_sync: push put"),
                        Err(e) if e.to_string().contains("precondition failed") => {
                            warn!(%uid, "remote_sync: push skipped (ETag conflict)");
                        }
                        Err(e) => warn!(%uid, "remote_sync: push put failed: {e}"),
                    }
                    let _ = local_etag; // used above indirectly
                }
                None => {
                    // Event was deleted between getting changes and now.
                    let _ = client
                        .delete_event(&source.remote_username, &source.remote_calendar, uid, None)
                        .await;
                }
            }
        }
    }

    // Update last_push_token.
    sqlx::query("UPDATE remote_caldav_sources SET last_push_token = ? WHERE id = ?")
        .bind(current_token)
        .bind(&source.id)
        .execute(pool)
        .await
        .context("saving last_push_token")?;

    Ok(())
}

// ── Database helpers ──────────────────────────────────────────────────────────

#[derive(Debug)]
struct SourceRow {
    id: String,
    username: String,
    name: String,
    remote_base_url: String,
    remote_username: String,
    remote_token_enc: String,
    remote_calendar: String,
    local_calendar: String,
    sync_direction: String,
    last_sync_token: Option<String>,
    last_push_token: Option<i64>,
    sync_interval_secs: u32,
}

async fn load_source_by_id(pool: &SqlitePool, id: &str) -> Result<Option<SourceRow>> {
    let row = sqlx::query(
        "SELECT id, username, name, remote_base_url, remote_username, remote_token_enc,
                remote_calendar, local_calendar, sync_direction, last_sync_token, last_push_token,
                sync_interval_secs
         FROM remote_caldav_sources
         WHERE id = ? AND enabled = 1",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .context("loading remote_caldav_source by id")?;

    Ok(row.map(|r| {
        let local: String = r.get("local_calendar");
        let rc: String = r.try_get("remote_calendar").unwrap_or_default();
        SourceRow {
            id: r.get("id"),
            username: r.get("username"),
            name: r.get("name"),
            remote_base_url: r.get("remote_base_url"),
            remote_username: r.get("remote_username"),
            remote_token_enc: r.get("remote_token_enc"),
            remote_calendar: if rc.is_empty() { local.clone() } else { rc },
            local_calendar: local,
            sync_direction: r.get("sync_direction"),
            last_sync_token: r.try_get("last_sync_token").ok().flatten(),
            last_push_token: r.try_get("last_push_token").ok().flatten(),
            sync_interval_secs: r.get::<i64, _>("sync_interval_secs") as u32,
        }
    }))
}

async fn load_sources(pool: &SqlitePool) -> Result<Vec<SourceRow>> {
    let rows = sqlx::query(
        "SELECT id, username, name, remote_base_url, remote_username, remote_token_enc,
                remote_calendar, local_calendar, sync_direction, last_sync_token, last_push_token,
                sync_interval_secs
         FROM remote_caldav_sources
         WHERE enabled = 1
         ORDER BY created_at",
    )
    .fetch_all(pool)
    .await
    .context("loading remote_caldav_sources")?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let local: String = r.get("local_calendar");
            let rc: String = r.try_get("remote_calendar").unwrap_or_default();
            SourceRow {
                id: r.get("id"),
                username: r.get("username"),
                name: r.get("name"),
                remote_base_url: r.get("remote_base_url"),
                remote_username: r.get("remote_username"),
                remote_token_enc: r.get("remote_token_enc"),
                remote_calendar: if rc.is_empty() { local.clone() } else { rc },
                local_calendar: local,
                sync_direction: r.get("sync_direction"),
                last_sync_token: r.try_get("last_sync_token").ok().flatten(),
                last_push_token: r.try_get("last_push_token").ok().flatten(),
                sync_interval_secs: r.get::<i64, _>("sync_interval_secs") as u32,
            }
        })
        .collect())
}

async fn set_source_error(pool: &SqlitePool, id: &str, error: &str) {
    let _ = sqlx::query("UPDATE remote_caldav_sources SET last_error = ? WHERE id = ?")
        .bind(error)
        .bind(id)
        .execute(pool)
        .await;
}

fn uid_from_href(href: &str) -> String {
    href.trim_end_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or(href)
        .trim_end_matches(".ics")
        .to_string()
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::storage::Storage;
    use sqlx::sqlite::SqliteConnectOptions;
    use std::str::FromStr;

    async fn test_storage() -> Storage {
        let opts = SqliteConnectOptions::from_str("sqlite::memory:").unwrap();
        let pool = SqlitePool::connect_with(opts).await.unwrap();
        sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
        Storage {
            pool,
            data_dir: std::env::temp_dir()
                .join(format!("tidl-test-{}", uuid::Uuid::new_v4()))
                .to_str()
                .unwrap()
                .to_string(),
        }
    }

    #[tokio::test]
    async fn pick_unused_color_returns_first_palette_entry_when_no_calendars() {
        let storage: Arc<dyn CalendarStore + Send + Sync> = Arc::new(test_storage().await);
        assert_eq!(
            pick_unused_color(&storage, "alice").await,
            CALENDAR_PALETTE[0]
        );
    }

    #[tokio::test]
    async fn pick_unused_color_skips_colors_already_in_use() {
        let storage: Arc<dyn CalendarStore + Send + Sync> = Arc::new(test_storage().await);
        storage
            .create_calendar("alice", "personal", Some("Personal"))
            .await
            .unwrap();
        storage
            .update_calendar_color("alice", "personal", CALENDAR_PALETTE[0])
            .await
            .unwrap();
        storage
            .create_calendar("alice", "work", Some("Work"))
            .await
            .unwrap();
        storage
            .update_calendar_color("alice", "work", CALENDAR_PALETTE[1])
            .await
            .unwrap();

        assert_eq!(
            pick_unused_color(&storage, "alice").await,
            CALENDAR_PALETTE[2]
        );
    }

    #[tokio::test]
    async fn pick_unused_color_falls_back_to_first_when_all_taken() {
        let storage: Arc<dyn CalendarStore + Send + Sync> = Arc::new(test_storage().await);
        for (i, color) in CALENDAR_PALETTE.iter().enumerate() {
            let name = format!("cal{i}");
            storage
                .create_calendar("alice", &name, Some(&name))
                .await
                .unwrap();
            storage
                .update_calendar_color("alice", &name, color)
                .await
                .unwrap();
        }

        assert_eq!(
            pick_unused_color(&storage, "alice").await,
            CALENDAR_PALETTE[0]
        );
    }

    #[test]
    fn sync_outcome_prioritizes_all_when_calendar_created() {
        let outcome = SyncOutcome {
            events_changed: true,
            calendar_created: true,
        };
        assert_eq!(outcome.change_kind(), Some(ChangeKind::All));

        let outcome = SyncOutcome {
            events_changed: false,
            calendar_created: true,
        };
        assert_eq!(outcome.change_kind(), Some(ChangeKind::All));
    }

    #[test]
    fn sync_outcome_events_only_when_no_new_calendar() {
        let outcome = SyncOutcome {
            events_changed: true,
            calendar_created: false,
        };
        assert_eq!(outcome.change_kind(), Some(ChangeKind::Events));
    }

    #[test]
    fn sync_outcome_none_when_nothing_changed() {
        let outcome = SyncOutcome {
            events_changed: false,
            calendar_created: false,
        };
        assert_eq!(outcome.change_kind(), None);
    }

    // ── seed_last_push_token_if_unset ─────────────────────────────────────────

    fn test_source(
        id: &str,
        username: &str,
        local_calendar: &str,
        last_push_token: Option<i64>,
    ) -> SourceRow {
        SourceRow {
            id: id.to_string(),
            username: username.to_string(),
            name: "test".to_string(),
            remote_base_url: "https://example.com".to_string(),
            remote_username: "remoteuser".to_string(),
            remote_token_enc: "enc".to_string(),
            remote_calendar: "remotecal".to_string(),
            local_calendar: local_calendar.to_string(),
            sync_direction: "both".to_string(),
            last_sync_token: None,
            last_push_token,
            sync_interval_secs: 300,
        }
    }

    async fn seed_source_row(pool: &SqlitePool, source: &SourceRow) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) \
             VALUES (?, ?, ?, 'x', 0)",
        )
        .bind(&source.username)
        .bind(format!("{}@example.com", source.username))
        .bind(&source.username)
        .execute(pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO remote_caldav_sources
                 (id, username, name, remote_base_url, remote_username, remote_token_enc,
                  remote_calendar, local_calendar, sync_direction, sync_interval_secs, last_push_token)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&source.id)
        .bind(&source.username)
        .bind(&source.name)
        .bind(&source.remote_base_url)
        .bind(&source.remote_username)
        .bind(&source.remote_token_enc)
        .bind(&source.remote_calendar)
        .bind(&source.local_calendar)
        .bind(&source.sync_direction)
        .bind(source.sync_interval_secs as i64)
        .bind(source.last_push_token)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn stored_last_push_token(pool: &SqlitePool, id: &str) -> Option<i64> {
        sqlx::query_scalar("SELECT last_push_token FROM remote_caldav_sources WHERE id = ?")
            .bind(id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    // ── wipe_calendar_events ──────────────────────────────────────────────────

    const TEST_ICAL: &str =
        "BEGIN:VCALENDAR\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nSUMMARY:Test\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n";

    #[tokio::test]
    async fn wipe_calendar_events_deletes_all_existing_events() {
        let storage = test_storage().await;
        storage
            .create_calendar("alice", "personal", Some("Personal"))
            .await
            .unwrap();
        for uid in ["uid-1", "uid-2", "uid-3"] {
            let ical = TEST_ICAL.replace("{uid}", uid);
            storage
                .store_event("alice", "personal", uid, &ical, "etag")
                .await
                .unwrap();
        }
        assert_eq!(
            storage
                .list_events("alice", "personal")
                .await
                .unwrap()
                .len(),
            3
        );

        let storage: Arc<dyn CalendarStore + Send + Sync> = Arc::new(storage);
        wipe_calendar_events(&storage, "alice", "personal").await;

        assert!(storage
            .list_events("alice", "personal")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn wipe_calendar_events_noop_for_empty_calendar() {
        let storage = test_storage().await;
        storage
            .create_calendar("alice", "personal", Some("Personal"))
            .await
            .unwrap();
        let storage: Arc<dyn CalendarStore + Send + Sync> = Arc::new(storage);

        wipe_calendar_events(&storage, "alice", "personal").await;

        assert!(storage
            .list_events("alice", "personal")
            .await
            .unwrap()
            .is_empty());
    }

    #[tokio::test]
    async fn seed_last_push_token_noop_when_already_set() {
        let storage = test_storage().await;
        let pool = storage.pool.clone();
        let mut source = test_source("src-1", "alice", "personal", Some(5));
        seed_source_row(&pool, &source).await;

        let storage: Arc<dyn CalendarStore + Send + Sync> = Arc::new(storage);
        seed_last_push_token_if_unset(&pool, &storage, &mut source)
            .await
            .unwrap();

        assert_eq!(source.last_push_token, Some(5));
        assert_eq!(stored_last_push_token(&pool, "src-1").await, Some(5));
    }

    #[tokio::test]
    async fn seed_last_push_token_seeds_to_calendars_current_token_when_unset() {
        let storage = test_storage().await;
        let pool = storage.pool.clone();
        storage
            .create_calendar("alice", "personal", Some("Personal"))
            .await
            .unwrap();
        // In the real pull() flow this function only ever sees a calendar
        // whose pre-existing events have already been wiped (see
        // wipe_calendar_events); a non-zero token at this point is normally
        // just the wipe's own delete tombstones. Bump the token directly
        // here (rather than storing real events) to test this function's
        // seeding logic in isolation from that wipe step.
        for i in 0..3 {
            storage
                .bump_sync_token("alice", "personal", &format!("uid-{i}"), "deleted")
                .await
                .unwrap();
        }

        let mut source = test_source("src-1", "alice", "personal", None);
        seed_source_row(&pool, &source).await;

        let storage: Arc<dyn CalendarStore + Send + Sync> = Arc::new(storage);
        seed_last_push_token_if_unset(&pool, &storage, &mut source)
            .await
            .unwrap();

        // Seeded to "now" (token 4, since calendars start at 1 and were
        // bumped 3 times), not to 0 — so those prior tombstones are never
        // treated as "new" and re-pushed/reconciled.
        assert_eq!(source.last_push_token, Some(4));
        assert_eq!(stored_last_push_token(&pool, "src-1").await, Some(4));
    }

    #[tokio::test]
    async fn seed_last_push_token_defaults_to_zero_for_nonexistent_calendar() {
        let storage = test_storage().await;
        let pool = storage.pool.clone();
        let mut source = test_source("src-1", "alice", "does-not-exist-yet", None);
        seed_source_row(&pool, &source).await;

        let storage: Arc<dyn CalendarStore + Send + Sync> = Arc::new(storage);
        seed_last_push_token_if_unset(&pool, &storage, &mut source)
            .await
            .unwrap();

        assert_eq!(source.last_push_token, Some(0));
        assert_eq!(stored_last_push_token(&pool, "src-1").await, Some(0));
    }
}
