//! Background sync loop for webcal (iCalendar feed) subscriptions.
//!
//! Spawned at server startup.  Every `SYNC_INTERVAL_SECS` it loads all
//! calendars that have a `sync_url` set and fetches each feed.
//!
//! ## Sync semantics
//!
//! Full replace: on each successful fetch every event in the calendar is
//! wiped and replaced with the events from the feed.  HTTP ETag and
//! Last-Modified headers are used to skip re-parsing when the feed has
//! not changed since the last fetch.

use std::{collections::HashSet, sync::Arc, time::Duration};

use anyhow::{Context, Result};
use reqwest::{
    header::{ETAG, IF_MODIFIED_SINCE, IF_NONE_MATCH, LAST_MODIFIED},
    StatusCode,
};
use sqlx::{Row, SqlitePool};
use tidl_caldav::{change::ChangeKind, etag::generate_etag, icalendar_utils::extract_uid};
use tokio::sync::broadcast;
use tracing::{debug, error, info, warn};

use crate::storage::CalendarStore;

/// Default sync interval — every hour.
const SYNC_INTERVAL_SECS: u64 = 3600;

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run_webcal_sync_loop(
    pool: SqlitePool,
    storage: Arc<dyn CalendarStore + Send + Sync>,
    change_tx: broadcast::Sender<String>,
) {
    // Sync immediately on startup so existing subscriptions are up-to-date
    // as soon as the server is ready (rather than waiting a full hour).
    sync_all(&pool, &storage, &change_tx).await;
    loop {
        tokio::time::sleep(Duration::from_secs(SYNC_INTERVAL_SECS)).await;
        sync_all(&pool, &storage, &change_tx).await;
    }
}

// ── Core sync logic ───────────────────────────────────────────────────────────

async fn sync_all(
    pool: &SqlitePool,
    storage: &Arc<dyn CalendarStore + Send + Sync>,
    change_tx: &broadcast::Sender<String>,
) {
    let feeds = match load_webcal_feeds(pool).await {
        Ok(f) => f,
        Err(e) => {
            error!("webcal_sync: failed to load feeds: {e}");
            return;
        }
    };

    debug!(count = feeds.len(), "webcal_sync: syncing feeds");

    for feed in feeds {
        let username = feed.username.clone();
        let calendar = feed.calendar.clone();
        match sync_calendar_now(pool, storage, &feed).await {
            Ok(changed) => {
                if changed {
                    crate::notify::notify_calendar_watchers(
                        storage,
                        change_tx,
                        &username,
                        &calendar,
                        ChangeKind::Events,
                    )
                    .await;
                }
            }
            Err(e) => {
                warn!(
                    username = %username,
                    calendar = %calendar,
                    "webcal_sync: sync failed: {e}"
                );
                let _ = storage
                    .update_webcal_sync_status(
                        &username,
                        &calendar,
                        None,
                        None,
                        Some(&e.to_string()),
                    )
                    .await;
            }
        }
    }
}

/// Sync a single webcal feed. Returns `true` if any events were stored or
/// deleted (i.e. the caller should notify connected browsers to refresh).
///
/// Public so the API can call it for manual "refresh now" requests.
pub async fn sync_calendar_now(
    pool: &SqlitePool,
    storage: &Arc<dyn CalendarStore + Send + Sync>,
    feed: &WebcalFeed,
) -> Result<bool> {
    let url = rewrite_webcal_url(&feed.sync_url);

    info!(
        username = %feed.username,
        calendar = %feed.calendar,
        url = %url,
        "webcal_sync: fetching"
    );

    let client = crate::ssrf_guard::guarded_client_builder()
        .redirect(reqwest::redirect::Policy::limited(10))
        .build()
        .context("building HTTP client")?;

    let mut req = client.get(&url);
    if let Some(etag) = &feed.sync_etag {
        req = req.header(IF_NONE_MATCH, etag);
    }
    if let Some(lm) = &feed.sync_last_modified {
        req = req.header(IF_MODIFIED_SINCE, lm);
    }

    let response = req.send().await.context("fetching webcal URL")?;

    if response.status() == StatusCode::NOT_MODIFIED {
        debug!(
            username = %feed.username,
            calendar = %feed.calendar,
            "webcal_sync: 304 Not Modified, skipping"
        );
        // Update last_synced_at but keep existing etag/last_modified.
        storage
            .update_webcal_sync_status(
                &feed.username,
                &feed.calendar,
                feed.sync_etag.as_deref(),
                feed.sync_last_modified.as_deref(),
                None,
            )
            .await
            .context("updating sync status after 304")?;
        return Ok(false);
    }

    if !response.status().is_success() {
        return Err(anyhow::anyhow!("HTTP {} from {}", response.status(), url));
    }

    let new_etag = response
        .headers()
        .get(ETAG)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let new_last_modified = response
        .headers()
        .get(LAST_MODIFIED)
        .and_then(|v| v.to_str().ok())
        .map(str::to_owned);

    let body = response.text().await.context("reading feed body")?;

    // Parse the feed into individual VCALENDAR-wrapped events.
    let events = split_vcalendar_events(&body);
    if events.is_empty() {
        warn!(
            username = %feed.username,
            calendar = %feed.calendar,
            "webcal_sync: feed contained no parseable events"
        );
    }

    // Store all events (upsert by uid) and collect the uid set.
    let mut new_uids: HashSet<String> = HashSet::new();
    let mut changed = false;

    for ical in &events {
        let uid = match extract_uid(ical) {
            Ok(u) => u,
            Err(e) => {
                warn!("webcal_sync: skipping event without UID: {e}");
                continue;
            }
        };
        let etag = generate_etag(ical);
        storage
            .store_event(&feed.username, &feed.calendar, &uid, ical, &etag)
            .await
            .context("storing webcal event")?;
        new_uids.insert(uid);
        changed = true;
    }

    // Delete any events no longer in the feed (full-replace semantics).
    let deleted = delete_stale_events(pool, &feed.username, &feed.calendar, &new_uids).await?;
    if deleted > 0 {
        changed = true;
    }

    // Update sync metadata.
    storage
        .update_webcal_sync_status(
            &feed.username,
            &feed.calendar,
            new_etag.as_deref(),
            new_last_modified.as_deref(),
            None,
        )
        .await
        .context("updating sync status")?;

    debug!(
        username = %feed.username,
        calendar = %feed.calendar,
        stored = new_uids.len(),
        deleted,
        "webcal_sync: done"
    );

    Ok(changed)
}

// ── Helpers ───────────────────────────────────────────────────────────────────

/// Rewrite `webcal://` → `https://` (with `http://` fallback if no TLS).
fn rewrite_webcal_url(url: &str) -> String {
    if let Some(rest) = url.strip_prefix("webcal://") {
        format!("https://{}", rest)
    } else {
        url.to_owned()
    }
}

/// Delete events in `calendar` whose uid is NOT in `keep_uids`.
/// Returns the number of rows deleted.
async fn delete_stale_events(
    pool: &SqlitePool,
    username: &str,
    calendar: &str,
    keep_uids: &HashSet<String>,
) -> Result<usize> {
    // Load all current UIDs so we can compute the diff without dynamic SQL.
    let rows = sqlx::query("SELECT uid FROM events WHERE username = ? AND calendar = ?")
        .bind(username)
        .bind(calendar)
        .fetch_all(pool)
        .await
        .context("loading current event UIDs")?;

    let mut deleted = 0usize;
    for row in rows {
        let uid: String = row.get("uid");
        if !keep_uids.contains(&uid) {
            sqlx::query("DELETE FROM events WHERE username = ? AND calendar = ? AND uid = ?")
                .bind(username)
                .bind(calendar)
                .bind(&uid)
                .execute(pool)
                .await
                .context("deleting stale event")?;
            deleted += 1;
        }
    }
    Ok(deleted)
}

/// Split a VCALENDAR feed into individual VCALENDAR-wrapped VEVENT strings.
/// VTIMEZONE components are preserved and prepended to every wrapped event.
fn split_vcalendar_events(ical_str: &str) -> Vec<String> {
    let mut events = Vec::new();
    let mut vtimezones = String::new();
    let mut in_vevent = false;
    let mut in_vtz = false;
    let mut current = String::new();
    let mut tz_current = String::new();

    for line in ical_str.lines() {
        let trimmed = line.trim_end();
        let upper = trimmed.to_ascii_uppercase();

        if upper == "BEGIN:VTIMEZONE" {
            in_vtz = true;
            tz_current.clear();
            tz_current.push_str(trimmed);
            tz_current.push_str("\r\n");
        } else if upper == "END:VTIMEZONE" && in_vtz {
            tz_current.push_str(trimmed);
            tz_current.push_str("\r\n");
            vtimezones.push_str(&tz_current);
            in_vtz = false;
        } else if in_vtz {
            tz_current.push_str(trimmed);
            tz_current.push_str("\r\n");
        } else if upper == "BEGIN:VEVENT" {
            in_vevent = true;
            current.clear();
            current.push_str(trimmed);
            current.push_str("\r\n");
        } else if upper == "END:VEVENT" && in_vevent {
            current.push_str(trimmed);
            current.push_str("\r\n");
            let wrapped = format!(
                "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//tidl//tidl//EN\r\n\
                 {}{}END:VCALENDAR\r\n",
                vtimezones, current,
            );
            events.push(wrapped);
            in_vevent = false;
        } else if in_vevent {
            current.push_str(trimmed);
            current.push_str("\r\n");
        }
    }
    events
}

// ── Database helpers ──────────────────────────────────────────────────────────

#[derive(Debug)]
pub struct WebcalFeed {
    pub username: String,
    pub calendar: String,
    pub sync_url: String,
    pub sync_etag: Option<String>,
    pub sync_last_modified: Option<String>,
}

pub async fn load_webcal_feeds(pool: &SqlitePool) -> Result<Vec<WebcalFeed>> {
    let rows = sqlx::query(
        "SELECT username, name AS calendar, sync_url, sync_etag, sync_last_modified
         FROM calendars
         WHERE sync_url IS NOT NULL AND sync_url != ''
         ORDER BY username, name",
    )
    .fetch_all(pool)
    .await
    .context("loading webcal feeds")?;

    Ok(rows
        .into_iter()
        .map(|r| WebcalFeed {
            username: r.get("username"),
            calendar: r.get("calendar"),
            sync_url: r.get("sync_url"),
            sync_etag: r.try_get("sync_etag").ok().flatten(),
            sync_last_modified: r.try_get("sync_last_modified").ok().flatten(),
        })
        .collect())
}

/// Load a single webcal feed by owner + calendar name, or `None` if that
/// calendar has no subscription URL. Used by the manual "refresh now" path so
/// it feeds the same [`sync_calendar_now`] implementation as the background
/// loop.
pub async fn load_webcal_feed(
    pool: &SqlitePool,
    username: &str,
    calendar: &str,
) -> Result<Option<WebcalFeed>> {
    let row = sqlx::query(
        "SELECT username, name AS calendar, sync_url, sync_etag, sync_last_modified
         FROM calendars
         WHERE username = ? AND name = ? AND sync_url IS NOT NULL AND sync_url != ''",
    )
    .bind(username)
    .bind(calendar)
    .fetch_optional(pool)
    .await
    .context("loading webcal feed")?;

    Ok(row.map(|r| WebcalFeed {
        username: r.get("username"),
        calendar: r.get("calendar"),
        sync_url: r.get("sync_url"),
        sync_etag: r.try_get("sync_etag").ok().flatten(),
        sync_last_modified: r.try_get("sync_last_modified").ok().flatten(),
    }))
}
