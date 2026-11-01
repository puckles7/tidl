//! Calendar management, sharing, sync, and import server functions.

use leptos::prelude::*;
use std::fmt::Debug;

#[cfg(feature = "ssr")]
use super::{auth, current_user, extract_recurrence_id, notify_change, ChangeKind};
#[cfg(feature = "ssr")]
use crate::ssr_types::EncryptionKey;

use super::{
    CalendarInfo, ImportResult, MarkdownSourceInfo, NewGitSource, NewMarkdownSource,
    NewRemoteCalDavSource, RemoteCalDavSource, ShareInfo, SharedCalendar, UpdateRemoteCalDavSource,
};

/// Return all calendars owned by the current user.
///
/// Creates the default "Personal" calendar if the user has none yet.
#[server]
pub async fn get_calendars() -> Result<Vec<CalendarInfo>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;

        let (pool, username) = auth().await?;

        // Ensure the personal calendar exists.
        sqlx::query(
            "INSERT OR IGNORE INTO calendars (username, name, display_name, color) \
             VALUES (?, 'personal', 'Personal', '#3b82f6')",
        )
        .bind(&username)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let rows = sqlx::query(
            "SELECT id, name, COALESCE(display_name, name) AS display_name, \
                    COALESCE(color, '#3b82f6') AS color, \
                    read_only, sync_url, sync_error, last_synced_at, \
                    EXISTS(SELECT 1 FROM calendar_shares cs \
                           WHERE cs.owner_username = calendars.username \
                             AND cs.calendar_name = calendars.name) AS shared \
             FROM calendars \
             WHERE username = ? AND (
                 read_only = 0                          -- regular writable calendars
                 OR (read_only = 1 AND sync_url IS NOT NULL AND sync_url != '')  -- webcal feeds
             ) \
             ORDER BY CASE name WHEN 'personal' THEN 0 ELSE 1 END, display_name",
        )
        .bind(&username)
        .fetch_all(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        return Ok(rows
            .into_iter()
            .map(|r| CalendarInfo {
                id: r.try_get("id").unwrap_or(0),
                name: r.try_get("name").unwrap_or_default(),
                display_name: r.try_get("display_name").unwrap_or_default(),
                color: r.try_get("color").unwrap_or_else(|_| "#3b82f6".to_string()),
                read_only: r.try_get::<bool, _>("read_only").unwrap_or(false),
                sync_url: r.try_get("sync_url").ok().flatten(),
                sync_error: r.try_get("sync_error").ok().flatten(),
                last_synced_at: r.try_get("last_synced_at").ok().flatten(),
                shared: r.try_get::<bool, _>("shared").unwrap_or(false),
            })
            .collect());
    }
    #[allow(unreachable_code)]
    Ok(vec![])
}

/// Create a new calendar. Returns the slug (name) used as the CalDAV collection.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn create_calendar_api(
    display_name: String,
    color: String,
) -> Result<String, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        let base_slug: String = display_name
            .to_lowercase()
            .chars()
            .map(|c| if c.is_alphanumeric() { c } else { '-' })
            .collect::<String>()
            .split('-')
            .filter(|s| !s.is_empty())
            .collect::<Vec<_>>()
            .join("-");
        let base_slug = if base_slug.is_empty() {
            "calendar".to_string()
        } else {
            base_slug
        };

        // Find a unique slug by appending a counter if needed.
        let mut slug = base_slug.clone();
        let mut counter: u32 = 1;
        loop {
            let exists: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM calendars WHERE username = ? AND name = ?",
            )
            .bind(&username)
            .bind(&slug)
            .fetch_one(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

            if exists == 0 {
                break;
            }
            counter += 1;
            slug = format!("{}-{}", base_slug, counter);
        }

        let color = if color.trim().is_empty() {
            "#3b82f6".to_string()
        } else {
            color
        };

        sqlx::query(
            "INSERT INTO calendars (username, name, display_name, color) VALUES (?, ?, ?, ?)",
        )
        .bind(&username)
        .bind(&slug)
        .bind(&display_name)
        .bind(&color)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        notify_change(&username, ChangeKind::Cals).await;
        return Ok(slug);
    }
    #[allow(unreachable_code)]
    Ok(String::new())
}

/// Rename a calendar (update its display name).
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn rename_calendar_api(name: String, display_name: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        sqlx::query(
            "UPDATE calendars SET display_name = ?, updated_at = datetime('now') \
             WHERE username = ? AND name = ?",
        )
        .bind(&display_name)
        .bind(&username)
        .bind(&name)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        notify_change(&username, ChangeKind::Cals).await;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Update a calendar's colour.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn update_calendar_color_api(name: String, color: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        sqlx::query(
            "UPDATE calendars SET color = ?, updated_at = datetime('now') \
             WHERE username = ? AND name = ?",
        )
        .bind(&color)
        .bind(&username)
        .bind(&name)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Mirror the colour into the WebDAV dead-property so iOS reads it back
        // on its next PROPFIND instead of overwriting with its own value.
        sqlx::query(
            "INSERT INTO properties (username, resource_path, namespace, name, value, updated_at) \
             VALUES (?, ?, 'http://apple.com/ns/ical/', 'calendar-color', ?, datetime('now')) \
             ON CONFLICT(username, resource_path, namespace, name) \
             DO UPDATE SET value = excluded.value, updated_at = excluded.updated_at",
        )
        .bind(&username)
        .bind(format!("/caldav/{}/{}", username, name))
        .bind(&color)
        .execute(&pool)
        .await
        .ok(); // non-fatal: colour sync failure should not block the UI response

        notify_change(&username, ChangeKind::Cals).await;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Set or clear the webcal subscription URL for a calendar.
///
/// Passing `Some(url)` marks the calendar read-only and attaches the feed.
/// Passing `None` detaches the feed, deletes cached events, and makes the calendar
/// writable again.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn set_calendar_sync_url_api(
    name: String,
    url: Option<String>,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        // Validate that the calendar belongs to this user.
        let exists: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM calendars WHERE username = ? AND name = ?")
                .bind(&username)
                .bind(&name)
                .fetch_one(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        if exists == 0 {
            return Err(ServerFnError::new("Calendar not found."));
        }

        match &url {
            Some(u) => {
                // Normalise webcal:// → https://
                let fetch_url = match u.strip_prefix("webcal://") {
                    Some(rest) => format!("https://{rest}"),
                    None => u.clone(),
                };

                // Validate the URL is reachable and returns parseable iCalendar data.
                let resp = reqwest::get(&fetch_url)
                    .await
                    .map_err(|e| ServerFnError::new(format!("Cannot reach URL: {e}")))?;

                if !resp.status().is_success() {
                    return Err(ServerFnError::new(format!(
                        "URL returned HTTP {}",
                        resp.status()
                    )));
                }
                let body = resp
                    .text()
                    .await
                    .map_err(|e| ServerFnError::new(format!("Cannot read feed: {e}")))?;

                if !body.trim().contains("BEGIN:VCALENDAR") {
                    return Err(ServerFnError::new(
                        "URL does not appear to be an iCalendar feed.",
                    ));
                }

                // Persist the URL (normalised) and mark read-only.
                sqlx::query(
                    "UPDATE calendars SET sync_url = ?, read_only = 1, sync_error = NULL
                     WHERE username = ? AND name = ?",
                )
                .bind(u) // store original URL (webcal:// preserved)
                .bind(&username)
                .bind(&name)
                .execute(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            }
            None => {
                // Clear the URL, reset read-only, delete cached feed events.
                sqlx::query(
                    "UPDATE calendars
                     SET sync_url = NULL, sync_etag = NULL, sync_last_modified = NULL,
                         last_synced_at = NULL, sync_error = NULL, read_only = 0
                     WHERE username = ? AND name = ?",
                )
                .bind(&username)
                .bind(&name)
                .execute(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

                sqlx::query("DELETE FROM events WHERE username = ? AND calendar = ?")
                    .bind(&username)
                    .bind(&name)
                    .execute(&pool)
                    .await
                    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            }
        }

        notify_change(&username, ChangeKind::All).await;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Trigger an immediate webcal sync for a single calendar (manual "refresh now").
///
/// Delegates to the background loop's shared `sync_calendar_now` via the
/// server-injected [`crate::ssr_types::WebcalSyncer`] extension, so the manual
/// and background sync paths can never drift apart. The old inline copy that
/// re-implemented fetch/parse/store (and silently NULLed fields) is gone
/// (backlog #108).
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn refresh_webcal_calendar_api(name: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::WebcalSyncer;
        use axum::Extension;
        use leptos_axum::extract;

        let (pool, username) = auth().await?;

        let Extension(syncer): Extension<WebcalSyncer> = extract().await?;
        syncer.0(username.clone(), name.clone())
            .await
            .map_err(ServerFnError::new)?;

        // Push the change to the owner and any grantees — same notification the
        // background loop fires when a feed changes.
        super::events::notify_calendar_watchers(&pool, &username, &name, ChangeKind::Events).await;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Delete all events from the personal calendar without removing the calendar itself.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn clear_personal_calendar() -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        // Collect UIDs before deleting so we can write sync_log tombstones.
        let uids: Vec<String> =
            sqlx::query_scalar("SELECT uid FROM events WHERE username = ? AND calendar = ?")
                .bind(&username)
                .bind("personal")
                .fetch_all(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        sqlx::query("DELETE FROM events WHERE username = ? AND calendar = ?")
            .bind(&username)
            .bind("personal")
            .execute(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Bump sync token and write a tombstone per deleted UID so CalDAV
        // clients receive 404 entries in the next sync-collection REPORT.
        if !uids.is_empty() {
            sqlx::query(
                "UPDATE calendars SET sync_token = sync_token + 1 WHERE username = ? AND name = ?",
            )
            .bind(&username)
            .bind("personal")
            .execute(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

            let new_token: i64 = sqlx::query_scalar(
                "SELECT sync_token FROM calendars WHERE username = ? AND name = ?",
            )
            .bind(&username)
            .bind("personal")
            .fetch_one(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

            for uid in &uids {
                sqlx::query(
                    "INSERT INTO sync_log (username, calendar, uid, operation, token) \
                     VALUES (?, ?, ?, 'deleted', ?)",
                )
                .bind(&username)
                .bind("personal")
                .bind(uid)
                .bind(new_token)
                .execute(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            }
        }

        notify_change(&username, ChangeKind::Events).await;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Delete a calendar and all its events. The "personal" calendar cannot be deleted.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn delete_calendar_api(name: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        if name == "personal" {
            return Err(ServerFnError::new("Cannot delete the Personal calendar."));
        }

        let mut tx = pool
            .begin()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Collect UIDs and bump the sync token before deleting the calendar
        // row, so CalDAV clients can discover the deletions via sync-collection.
        // All writes are in one transaction — O(1) disk syncs regardless of size.
        let uids: Vec<String> =
            sqlx::query_scalar("SELECT uid FROM events WHERE username = ? AND calendar = ?")
                .bind(&username)
                .bind(&name)
                .fetch_all(&mut *tx)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        if !uids.is_empty() {
            sqlx::query(
                "UPDATE calendars SET sync_token = sync_token + 1 WHERE username = ? AND name = ?",
            )
            .bind(&username)
            .bind(&name)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

            let new_token: i64 = sqlx::query_scalar(
                "SELECT sync_token FROM calendars WHERE username = ? AND name = ?",
            )
            .bind(&username)
            .bind(&name)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

            for uid in &uids {
                sqlx::query(
                    "INSERT INTO sync_log (username, calendar, uid, operation, token) \
                     VALUES (?, ?, ?, 'deleted', ?)",
                )
                .bind(&username)
                .bind(&name)
                .bind(uid)
                .bind(new_token)
                .execute(&mut *tx)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            }
        }

        sqlx::query("DELETE FROM events WHERE username = ? AND calendar = ?")
            .bind(&username)
            .bind(&name)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Look up grantees before dropping the share rows, so they can be
        // notified after commit — otherwise calendar_shares would silently
        // outlive the calendar it grants access to (the grant row itself
        // stays orphaned forever with nothing to clean it up).
        let grantees: Vec<String> = sqlx::query_scalar(
            "SELECT grantee_username FROM calendar_shares WHERE owner_username = ? AND calendar_name = ?",
        )
        .bind(&username)
        .bind(&name)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        sqlx::query("DELETE FROM calendar_shares WHERE owner_username = ? AND calendar_name = ?")
            .bind(&username)
            .bind(&name)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        sqlx::query("DELETE FROM calendars WHERE username = ? AND name = ?")
            .bind(&username)
            .bind(&name)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Drop the calendar's sync_log too. Leaving it behind would let its
        // old tombstones outrank the entries a recreated calendar writes from
        // a reset sync_token, feeding CalDAV clients deletions for events that
        // exist.
        sqlx::query("DELETE FROM sync_log WHERE username = ? AND calendar = ?")
            .bind(&username)
            .bind(&name)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        notify_change(&username, ChangeKind::All).await;
        for grantee in &grantees {
            notify_change(grantee, ChangeKind::All).await;
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

// ── Calendar import ───────────────────────────────────────────────────────────

/// Import events from a raw iCalendar string (.ics file content).
///
/// `calendar`  — slug name of the target calendar (created if missing).
/// `overwrite` — replace existing events with the same UID when `true`;
///               skip them when `false`.
///
/// Returns counts of imported / skipped / failed events.
#[allow(unused_variables)]
#[server(input = server_fn::codec::Json)]
pub async fn import_ics(
    ical_content: String,
    calendar: String,
    overwrite: bool,
) -> Result<ImportResult, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use axum::http::HeaderMap;
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::SqlitePool;
        use tidl_caldav::etag::generate_etag;
        use tidl_caldav::icalendar_utils::{
            extract_event_fields, extract_uid, rewrite_uid_in_ical,
            sanitize_google_continuation_uid,
        };

        const MAX_BYTES: usize = 10 * 1024 * 1024; // 10 MB
        if ical_content.len() > MAX_BYTES {
            return Err(ServerFnError::new("File exceeds the 10 MB limit"));
        }

        let Extension(pool): Extension<SqlitePool> = extract().await?;
        let headers: HeaderMap = extract().await?;
        let username = current_user(&pool, &headers).await?;

        let cal_name = if calendar.trim().is_empty() {
            "personal".to_string()
        } else {
            calendar.clone()
        };

        // Ensure the target calendar exists before opening the transaction.
        sqlx::query(
            "INSERT OR IGNORE INTO calendars (username, name, display_name, color) VALUES (?, ?, ?, ?)",
        )
        .bind(&username)
        .bind(&cal_name)
        .bind(&cal_name)
        .bind("#3b82f6")
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let event_cals = split_vcalendar_events(&ical_content);

        let mut imported = 0u32;
        let mut skipped = 0u32;
        let mut failed = 0u32;
        let mut imported_uids: Vec<String> = Vec::new();

        // Wrap all event writes in one transaction — O(1) disk syncs instead of
        // O(n), which is what caused large imports to exceed the proxy timeout.
        let mut tx = pool
            .begin()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        for ical_str in event_cals {
            let raw_uid = match extract_uid(&ical_str) {
                Ok(u) => u,
                Err(_) => {
                    failed += 1;
                    continue;
                }
            };

            // Normalise Google "continuation series" UIDs.  Google ends a
            // recurring series and starts a new one by encoding the last
            // recurrence-ID into the new UID: `{base}_R{datetime}@domain`.
            // iOS pattern-matches this and treats the event as an override of
            // the already-ended parent, so RRULE occurrences are never shown.
            // Rewriting to `{base}-{datetime}@tidl.local` breaks the
            // mis-association while keeping the UID stable and unique.
            let normalized_uid = sanitize_google_continuation_uid(&raw_uid);
            let (uid_for_db_key, ical_str) = if normalized_uid != raw_uid {
                // Rewrite the UID inside the ICS data so CalDAV clients receive
                // the normalised form; also clean up any stale old-UID entry.
                let new_ical = rewrite_uid_in_ical(&ical_str, &raw_uid, &normalized_uid);
                sqlx::query("DELETE FROM events WHERE username = ? AND calendar = ? AND uid = ?")
                    .bind(&username)
                    .bind(&cal_name)
                    .bind(&raw_uid)
                    .execute(&mut *tx)
                    .await
                    .ok();
                (normalized_uid, new_ical)
            } else {
                (raw_uid, ical_str)
            };

            // RFC 5545: a VEVENT with RECURRENCE-ID is an override of a single
            // occurrence within a recurring series.  If we store it under the
            // bare UID it will overwrite the master (which carries the RRULE),
            // destroying the whole series.  Give the override a composite key
            // so it coexists with the master while not shadowing it.
            let uid = if let Some(rid) = extract_recurrence_id(&ical_str) {
                format!("{}_R{}", uid_for_db_key, rid)
            } else {
                uid_for_db_key
            };

            // Check whether this UID already exists in the target calendar.
            let exists =
                sqlx::query("SELECT 1 FROM events WHERE username = ? AND calendar = ? AND uid = ?")
                    .bind(&username)
                    .bind(&cal_name)
                    .bind(&uid)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
                    .is_some();

            if exists && !overwrite {
                skipped += 1;
                continue;
            }

            let fields = extract_event_fields(&ical_str);
            let etag = generate_etag(&ical_str);
            let event_type = if fields.component_type.is_empty() {
                "VEVENT"
            } else {
                &fields.component_type
            };

            let ok = sqlx::query(
                r#"INSERT OR REPLACE INTO events
                   (username, calendar, uid, type, summary, description, location, priority,
                    start_date, end_date, etag, ical_data, source, updated_at)
                   VALUES (?, ?, ?, ?, ?, ?, ?, ?,
                           datetime(?), datetime(?), ?, ?, 'import', datetime('now'))"#,
            )
            .bind(&username)
            .bind(&cal_name)
            .bind(&uid)
            .bind(event_type)
            .bind(&fields.summary)
            .bind(&fields.description)
            .bind(&fields.location)
            .bind(
                fields
                    .priority
                    .filter(|p| *p != 0)
                    .map(crate::types::caldav_priority_to_source_priority)
                    .map(i64::from),
            )
            .bind(&fields.start_date)
            .bind(&fields.end_date)
            .bind(&etag)
            .bind(&ical_str)
            .execute(&mut *tx)
            .await
            .is_ok();

            if ok {
                imported += 1;
                imported_uids.push(uid);
            } else {
                failed += 1;
            }
        }

        // Bump the CalDAV sync token so iOS/CalDAV clients notice the import.
        // We do one token increment for the whole batch then write one sync_log
        // entry per UID at the new token value — matching what store_event does.
        if !imported_uids.is_empty() {
            sqlx::query(
                "UPDATE calendars SET sync_token = sync_token + 1 WHERE username = ? AND name = ?",
            )
            .bind(&username)
            .bind(&cal_name)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

            let new_token: i64 = sqlx::query_scalar(
                "SELECT sync_token FROM calendars WHERE username = ? AND name = ?",
            )
            .bind(&username)
            .bind(&cal_name)
            .fetch_one(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

            for uid in &imported_uids {
                sqlx::query(
                    "INSERT INTO sync_log (username, calendar, uid, operation, token) \
                     VALUES (?, ?, ?, 'modified', ?)",
                )
                .bind(&username)
                .bind(&cal_name)
                .bind(uid)
                .bind(new_token)
                .execute(&mut *tx)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            }
        }

        tx.commit()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        super::events::notify_calendar_watchers(&pool, &username, &cal_name, ChangeKind::Events)
            .await;
        return Ok(ImportResult {
            imported,
            skipped,
            failed,
        });
    }
    #[allow(unreachable_code)]
    Ok(ImportResult {
        imported: 0,
        skipped: 0,
        failed: 0,
    })
}

/// Split a VCALENDAR string (potentially containing many VEVENTs) into a
/// `Vec` of single-event VCALENDAR strings.
///
/// Each output element is a complete `BEGIN:VCALENDAR … END:VCALENDAR` block
/// containing exactly one `VEVENT`.  Any `VTIMEZONE` components found in the
/// source are copied into every output block so that timezone references remain
/// resolvable without the original file.
#[cfg(feature = "ssr")]
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

// ── Calendar sharing ──────────────────────────────────────────────────────────

/// Create a share invitation for one of the current user's calendars and
/// send an email invitation to the invitee.
/// `permission` must be `"read"` or `"read-write"`.
#[server]
pub async fn share_calendar(
    calendar_name: String,
    invitee_email: String,
    permission: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::{ServerBaseUrl, ServerSecret};
        use axum::http::HeaderMap;
        use axum::Extension;
        use chrono::{Duration, Utc};
        use leptos_axum::extract;
        use sqlx::{Row, SqlitePool};
        use uuid::Uuid;

        let Extension(pool): Extension<SqlitePool> = extract().await?;
        let headers: HeaderMap = extract().await?;
        let base_url = extract::<Extension<ServerBaseUrl>>()
            .await
            .map(|e| e.0 .0.clone())
            .unwrap_or_default();
        let server_secret = extract::<Extension<ServerSecret>>()
            .await
            .map(|e| e.0 .0.clone())
            .unwrap_or_default();
        let username = current_user(&pool, &headers).await?;

        if permission != "read" && permission != "read-write" {
            return Err(ServerFnError::new(
                "permission must be 'read' or 'read-write'",
            ));
        }

        // Verify the calendar belongs to the current user; fetch its display name.
        let cal_row = sqlx::query(
            "SELECT name, COALESCE(display_name, name) AS display_name \
             FROM calendars WHERE username = ? AND name = ?",
        )
        .bind(&username)
        .bind(&calendar_name)
        .fetch_optional(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("Calendar not found"))?;

        let cal_display: String = cal_row
            .try_get("display_name")
            .unwrap_or_else(|_| calendar_name.clone());

        let token = Uuid::new_v4().to_string();
        let expires_at = Utc::now() + Duration::days(7);
        let expires_str = expires_at.format("%Y-%m-%d %H:%M:%S").to_string();

        sqlx::query(
            "INSERT INTO share_invitations \
             (owner_username, resource_type, calendar_name, invitee_email, permission, token, expires_at) \
             VALUES (?, 'calendar', ?, ?, ?, ?, ?)"
        )
        .bind(&username).bind(&calendar_name).bind(&invitee_email)
        .bind(&permission).bind(&token).bind(&expires_str)
        .execute(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        let perm_label = if permission == "read" {
            "read-only"
        } else {
            "read and write"
        };
        super::sharing::send_share_invitation_email(
            &pool,
            &server_secret,
            &base_url,
            &username,
            "calendar",
            &cal_display,
            Some(perm_label),
            &token,
            &invitee_email,
        )
        .await?;

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Return the list of accepted share grants for a calendar owned by the current user.
#[server]
pub async fn get_calendar_shares(calendar_name: String) -> Result<Vec<ShareInfo>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;

        let (pool, username) = auth().await?;

        let rows = sqlx::query(
            "SELECT grantee_username, permission FROM calendar_shares \
             WHERE owner_username = ? AND calendar_name = ? ORDER BY grantee_username",
        )
        .bind(&username)
        .bind(&calendar_name)
        .fetch_all(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        return Ok(rows
            .into_iter()
            .map(|r| ShareInfo {
                grantee_username: r.try_get("grantee_username").unwrap_or_default(),
                permission: r.try_get("permission").unwrap_or_default(),
            })
            .collect());
    }
    #[allow(unreachable_code)]
    Ok(vec![])
}

/// Revoke a share grant for a calendar owned by the current user.
#[server]
pub async fn revoke_calendar_share(
    calendar_name: String,
    grantee_username: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        sqlx::query(
            "DELETE FROM calendar_shares WHERE owner_username = ? AND calendar_name = ? AND grantee_username = ?"
        )
        .bind(&username).bind(&calendar_name).bind(&grantee_username)
        .execute(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        // Notify the grantee (loses the calendar and its events — any open
        // tab should drop both without a manual refresh) and the owner
        // (their own "Your shared calendars" list changed too).
        notify_change(&grantee_username, ChangeKind::All).await;
        notify_change(&username, ChangeKind::All).await;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Remove a share grant extended to the current user — the grantee's side
/// of `revoke_calendar_share`, since that one is hard-scoped to the owner.
#[server]
pub async fn leave_shared_calendar(
    owner_username: String,
    calendar_name: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        sqlx::query(
            "DELETE FROM calendar_shares WHERE owner_username = ? AND calendar_name = ? AND grantee_username = ?"
        )
        .bind(&owner_username).bind(&calendar_name).bind(&username)
        .execute(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        // Notify both sides, same as `revoke_calendar_share`: the grantee
        // (this calendar and its events disappear from their own view) and
        // the owner (their "Your shared calendars" list changed too).
        notify_change(&username, ChangeKind::All).await;
        notify_change(&owner_username, ChangeKind::All).await;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Set the current user's own display color for a calendar shared with them.
/// Purely a per-viewer override (`calendar_shares.grantee_color`) — never
/// touches the owner's `calendars.color`, so it doesn't affect the owner or
/// any other grantee's view of the same calendar.
#[server]
pub async fn set_shared_calendar_color(
    owner_username: String,
    calendar_name: String,
    color: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        sqlx::query(
            "UPDATE calendar_shares SET grantee_color = ? \
             WHERE owner_username = ? AND calendar_name = ? AND grantee_username = ?",
        )
        .bind(&color)
        .bind(&owner_username)
        .bind(&calendar_name)
        .bind(&username)
        .execute(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        // Only the caller's own view changed — no need to notify the owner
        // or other grantees.
        notify_change(&username, ChangeKind::Cals).await;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Core DB logic for [`clear_shared_calendar`], factored out so the
/// `read-write`-permission gate and the delete/tombstone transaction are
/// testable without the Leptos server-fn request context. Returns the list
/// of grantees to notify (owner is notified separately by the caller) on
/// success.
#[cfg(feature = "ssr")]
async fn clear_shared_calendar_tx(
    pool: &sqlx::SqlitePool,
    username: &str,
    owner_username: &str,
    calendar_name: &str,
) -> Result<Vec<String>, ServerFnError> {
    use sqlx::Row;

    let has_write: bool = sqlx::query(
        "SELECT 1 FROM calendar_shares \
         WHERE owner_username = ? AND calendar_name = ? AND grantee_username = ? \
           AND permission = 'read-write'",
    )
    .bind(owner_username)
    .bind(calendar_name)
    .bind(username)
    .fetch_optional(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
    .is_some();
    if !has_write {
        return Err(ServerFnError::new(
            "write access to this calendar is required",
        ));
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    // Same tombstone dance as `clear_personal_calendar`/`delete_calendar_api`:
    // collect UIDs before deleting so CalDAV clients see the deletions via
    // sync-collection, keyed to the *owner's* username/calendar (this is
    // their calendar; the grantee is only exercising a write permission on it).
    let uids: Vec<String> =
        sqlx::query_scalar("SELECT uid FROM events WHERE username = ? AND calendar = ?")
            .bind(owner_username)
            .bind(calendar_name)
            .fetch_all(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    sqlx::query("DELETE FROM events WHERE username = ? AND calendar = ?")
        .bind(owner_username)
        .bind(calendar_name)
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    if !uids.is_empty() {
        sqlx::query(
            "UPDATE calendars SET sync_token = sync_token + 1 WHERE username = ? AND name = ?",
        )
        .bind(owner_username)
        .bind(calendar_name)
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let new_token: i64 =
            sqlx::query_scalar("SELECT sync_token FROM calendars WHERE username = ? AND name = ?")
                .bind(owner_username)
                .bind(calendar_name)
                .fetch_one(&mut *tx)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        for uid in &uids {
            sqlx::query(
                "INSERT INTO sync_log (username, calendar, uid, operation, token) \
                 VALUES (?, ?, ?, 'deleted', ?)",
            )
            .bind(owner_username)
            .bind(calendar_name)
            .bind(uid)
            .bind(new_token)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        }
    }

    // Every grantee (this one included) needs their view refreshed, same
    // as `delete_calendar_api` notifying all grantees when the owner
    // deletes the calendar outright.
    let grantees: Vec<String> = sqlx::query(
        "SELECT grantee_username FROM calendar_shares WHERE owner_username = ? AND calendar_name = ?",
    )
    .bind(owner_username)
    .bind(calendar_name)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
    .into_iter()
    .map(|r| r.get("grantee_username"))
    .collect();

    tx.commit()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    Ok(grantees)
}

/// Delete every event in a calendar shared with the current user under
/// `read-write` permission. Requires an explicit write grant — unlike
/// `set_shared_calendar_color`, this touches the owner's real events, so a
/// `read`-only grantee (or anyone without a grant at all) can't call it.
#[server(input = server_fn::codec::Json)]
pub async fn clear_shared_calendar(
    owner_username: String,
    calendar_name: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        let grantees =
            clear_shared_calendar_tx(&pool, &username, &owner_username, &calendar_name).await?;

        notify_change(&owner_username, ChangeKind::Events).await;
        for grantee in &grantees {
            notify_change(grantee, ChangeKind::Events).await;
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Return the calendars shared *with* the current user by other users.
#[server]
pub async fn get_shared_calendars() -> Result<Vec<SharedCalendar>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;

        let (pool, username) = auth().await?;

        let rows = sqlx::query(
            "SELECT s.owner_username, s.calendar_name, s.permission, \
                    COALESCE(c.display_name, s.calendar_name) AS display_name, \
                    COALESCE(s.grantee_color, c.color, '#6366f1') AS color \
             FROM calendar_shares s \
             JOIN calendars c ON c.username = s.owner_username AND c.name = s.calendar_name \
             WHERE s.grantee_username = ? \
             ORDER BY s.owner_username, s.calendar_name",
        )
        .bind(&username)
        .fetch_all(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        return Ok(rows
            .into_iter()
            .map(|r| SharedCalendar {
                owner_username: r.try_get("owner_username").unwrap_or_default(),
                calendar_name: r.try_get("calendar_name").unwrap_or_default(),
                display_name: r.try_get("display_name").unwrap_or_default(),
                color: r.try_get("color").unwrap_or_else(|_| "#6366f1".to_string()),
                permission: r.try_get("permission").unwrap_or_default(),
            })
            .collect());
    }
    #[allow(unreachable_code)]
    Ok(vec![])
}

// ── Remote CalDAV sources ─────────────────────────────────────────────────────

/// List all remote CalDAV sources for the authenticated user.
#[server]
pub async fn list_remote_sources() -> Result<Vec<RemoteCalDavSource>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;
        let (pool, username) = auth().await?;

        let rows = sqlx::query(
            "SELECT id, name, remote_base_url, remote_username, remote_calendar, local_calendar,
                    sync_direction, enabled, last_synced_at, last_error, sync_interval_secs
             FROM remote_caldav_sources WHERE username = ? ORDER BY created_at",
        )
        .bind(&username)
        .fetch_all(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        return Ok(rows
            .into_iter()
            .map(|r| {
                let enabled: i64 = r.get("enabled");
                let local: String = r.get("local_calendar");
                let rc: String = r.try_get("remote_calendar").unwrap_or_default();
                RemoteCalDavSource {
                    id: r.get("id"),
                    name: r.get("name"),
                    remote_base_url: r.get("remote_base_url"),
                    remote_username: r.get("remote_username"),
                    remote_calendar: if rc.is_empty() { local.clone() } else { rc },
                    local_calendar: local,
                    sync_direction: r.get("sync_direction"),
                    enabled: enabled != 0,
                    last_synced_at: r.try_get("last_synced_at").ok().flatten(),
                    last_error: r.try_get("last_error").ok().flatten(),
                    sync_interval_secs: r.get::<i64, _>("sync_interval_secs") as u32,
                }
            })
            .collect());
    }
    #[allow(unreachable_code)]
    Ok(vec![])
}

/// Add a new remote CalDAV source. Returns the new source's id, so the
/// caller can track its first sync the same way it tracks a manually
/// triggered one (see `trigger_remote_caldav_sync`).
#[server(input = server_fn::codec::Json)]
pub async fn add_remote_source(source: NewRemoteCalDavSource) -> Result<String, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::types::MIN_SYNC_INTERVAL_SECS;
        use axum::Extension;
        use leptos_axum::extract;
        let (pool, username) = auth().await?;

        let Extension(server_secret): Extension<crate::ssr_types::ServerSecret> = extract().await?;
        let server_secret = server_secret.0;

        if source.remote_token.is_empty() {
            return Err(ServerFnError::new("remote token is required"));
        }
        if source.remote_calendar.trim().is_empty() {
            return Err(ServerFnError::new("remote calendar is required"));
        }
        if source.local_calendar.trim().is_empty() {
            return Err(ServerFnError::new("local calendar slug is required"));
        }
        if source.sync_interval_secs < MIN_SYNC_INTERVAL_SECS {
            return Err(ServerFnError::new(format!(
                "sync interval must be at least {MIN_SYNC_INTERVAL_SECS} seconds"
            )));
        }

        let token_enc = crate::ssr_types::encrypt_password(&source.remote_token, &server_secret)
            .map_err(ServerFnError::new)?;

        let id = uuid::Uuid::new_v4().to_string();
        let direction = match source.sync_direction.as_str() {
            "pull" | "push" | "both" => source.sync_direction.clone(),
            _ => "both".to_string(),
        };

        sqlx::query(
            "INSERT INTO remote_caldav_sources
                 (id, username, name, remote_base_url, remote_username, remote_token_enc,
                  remote_calendar, local_calendar, sync_direction, sync_interval_secs)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(&id)
        .bind(&username)
        .bind(&source.name)
        .bind(&source.remote_base_url)
        .bind(&source.remote_username)
        .bind(&token_enc)
        .bind(&source.remote_calendar)
        .bind(&source.local_calendar)
        .bind(&direction)
        .bind(source.sync_interval_secs as i64)
        .execute(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        // Trigger an immediate sync for the newly added source.
        if let Ok(Extension(trigger)) = extract::<Extension<crate::ssr_types::SyncTrigger>>().await
        {
            let _ = trigger.0.send(id.clone());
        }

        return Ok(id);
    }
    #[allow(unreachable_code)]
    Ok(String::new())
}

/// Core DB logic for [`update_remote_source`], factored out so it's testable
/// without the Leptos server-fn request context (`ServerSecret`/`SyncTrigger`
/// extraction). `new_token_enc` is the already-encrypted replacement token,
/// or `None` to keep the existing one unchanged. Assumes `update` has
/// already been validated (non-empty `remote_calendar`/`local_calendar`,
/// `sync_interval_secs >= MIN_SYNC_INTERVAL_SECS`).
#[cfg(feature = "ssr")]
async fn update_remote_source_tx(
    pool: &sqlx::SqlitePool,
    username: &str,
    source_id: &str,
    update: &UpdateRemoteCalDavSource,
    new_token_enc: Option<&str>,
) -> Result<(), ServerFnError> {
    use sqlx::Row;

    let existing = sqlx::query(
        "SELECT remote_base_url, remote_username, remote_calendar, local_calendar
         FROM remote_caldav_sources WHERE id = ? AND username = ?",
    )
    .bind(source_id)
    .bind(username)
    .fetch_optional(pool)
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?
    .ok_or_else(|| ServerFnError::new("source not found"))?;

    // Either the remote collection being synced changed, or the local
    // calendar it's mirrored into did. Either way a stored RFC 6578
    // sync-token (remote side) no longer describes what should be pulled, so
    // force a full pull. `last_push_token` (local side) resets to NULL too:
    // on the next sync, `remote_sync::pull()` wipes any pre-existing events
    // in whichever calendar is now mapped (so the merge behaves like a fresh
    // calendar — the UI warns about this before saving), then
    // `seed_last_push_token_if_unset` seeds the cursor past that wipe's own
    // tombstones. Events left behind in the old local calendar (if it
    // changed) are untouched — they just stop being synced.
    let resync_needed = existing.get::<String, _>("remote_base_url") != update.remote_base_url
        || existing.get::<String, _>("remote_username") != update.remote_username
        || existing.get::<String, _>("remote_calendar") != update.remote_calendar
        || existing.get::<String, _>("local_calendar") != update.local_calendar;

    let direction = match update.sync_direction.as_str() {
        "pull" | "push" | "both" => update.sync_direction.as_str(),
        _ => "both",
    };

    sqlx::query(
        "UPDATE remote_caldav_sources SET
             name = ?, remote_base_url = ?, remote_username = ?,
             remote_token_enc = CASE WHEN ? THEN ? ELSE remote_token_enc END,
             remote_calendar = ?, local_calendar = ?, sync_direction = ?, sync_interval_secs = ?,
             last_sync_token = CASE WHEN ? THEN NULL ELSE last_sync_token END,
             last_push_token = CASE WHEN ? THEN NULL ELSE last_push_token END
         WHERE id = ? AND username = ?",
    )
    .bind(&update.name)
    .bind(&update.remote_base_url)
    .bind(&update.remote_username)
    .bind(new_token_enc.is_some())
    .bind(new_token_enc.unwrap_or(""))
    .bind(&update.remote_calendar)
    .bind(&update.local_calendar)
    .bind(direction)
    .bind(update.sync_interval_secs as i64)
    .bind(resync_needed)
    .bind(resync_needed)
    .bind(source_id)
    .bind(username)
    .execute(pool)
    .await
    .map_err(|e| ServerFnError::new(e.to_string()))?;

    Ok(())
}

/// Update an existing remote CalDAV source.
#[server(input = server_fn::codec::Json)]
pub async fn update_remote_source(
    source_id: String,
    update: UpdateRemoteCalDavSource,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::types::MIN_SYNC_INTERVAL_SECS;
        use axum::Extension;
        use leptos_axum::extract;
        let (pool, username) = auth().await?;

        if update.remote_calendar.trim().is_empty() {
            return Err(ServerFnError::new("remote calendar is required"));
        }
        if update.local_calendar.trim().is_empty() {
            return Err(ServerFnError::new("local calendar slug is required"));
        }
        if update.sync_interval_secs < MIN_SYNC_INTERVAL_SECS {
            return Err(ServerFnError::new(format!(
                "sync interval must be at least {MIN_SYNC_INTERVAL_SECS} seconds"
            )));
        }

        let new_token = update.remote_token.as_deref().filter(|t| !t.is_empty());
        let token_enc = match new_token {
            Some(t) => {
                let Extension(server_secret): Extension<crate::ssr_types::ServerSecret> =
                    extract().await?;
                Some(
                    crate::ssr_types::encrypt_password(t, &server_secret.0)
                        .map_err(ServerFnError::new)?,
                )
            }
            None => None,
        };

        update_remote_source_tx(&pool, &username, &source_id, &update, token_enc.as_deref())
            .await?;

        // Trigger an immediate resync so the edited settings get exercised right away.
        if let Ok(Extension(trigger)) = extract::<Extension<crate::ssr_types::SyncTrigger>>().await
        {
            let _ = trigger.0.send(source_id);
        }

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Delete a remote CalDAV source by ID.
#[server(input = server_fn::codec::Json)]
pub async fn delete_remote_source(source_id: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        let affected =
            sqlx::query("DELETE FROM remote_caldav_sources WHERE id = ? AND username = ?")
                .bind(&source_id)
                .bind(&username)
                .execute(&pool)
                .await
                .map_err(|e| ServerFnError::new(e.to_string()))?
                .rows_affected();

        if affected == 0 {
            return Err(ServerFnError::new("source not found"));
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Trigger an immediate sync for an existing remote CalDAV source.
/// Returns immediately — the sync runs in the background.
#[server]
pub async fn trigger_remote_caldav_sync(source_id: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use axum::Extension;
        use leptos_axum::extract;
        let (pool, username) = auth().await?;

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM remote_caldav_sources WHERE id = ? AND username = ?",
        )
        .bind(&source_id)
        .bind(&username)
        .fetch_one(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        if count == 0 {
            return Err(ServerFnError::new("source not found"));
        }

        if let Ok(Extension(trigger)) = extract::<Extension<crate::ssr_types::SyncTrigger>>().await
        {
            let _ = trigger.0.send(source_id);
        }

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Discover available calendar slugs on a remote CalDAV server.
///
/// Performs a PROPFIND Depth:1 on the user's calendar home to list
/// all calendar collections.  Used by the UI "Discover" button before
/// a source is saved.
#[server(input = server_fn::codec::Json)]
pub async fn discover_remote_calendars(
    url: String,
    username: String,
    token: String,
) -> Result<Vec<String>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use base64::{engine::general_purpose::STANDARD as B64, Engine as _};

        let base_url = url.trim_end_matches('/');
        let propfind_url = format!("{}/caldav/{}/", base_url, username);
        let auth = format!("Basic {}", B64.encode(format!("{}:{}", username, token)));
        let body = r#"<?xml version="1.0" encoding="utf-8"?>
<D:propfind xmlns:D="DAV:">
  <D:prop><D:resourcetype/><D:displayname/></D:prop>
</D:propfind>"#;

        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .build()
            .map_err(ServerFnError::new)?;

        let resp = client
            .request(
                reqwest::Method::from_bytes(b"PROPFIND").unwrap(),
                &propfind_url,
            )
            .header("Authorization", auth)
            .header("Depth", "1")
            .header("Content-Type", "application/xml")
            .body(body)
            .send()
            .await
            .map_err(|e| ServerFnError::new(format!("Could not reach server: {e}")))?;

        if resp.status().as_u16() != 207 {
            return Err(ServerFnError::new(format!(
                "Server returned {} — check credentials and URL",
                resp.status()
            )));
        }

        let text = resp
            .text()
            .await
            .map_err(|e| ServerFnError::new(format!("Could not read response: {e}")))?;

        return Ok(parse_calendar_slugs_from_propfind(&text, &username));
    }
    #[allow(unreachable_code)]
    Ok(vec![])
}

#[cfg(feature = "ssr")]
fn parse_calendar_slugs_from_propfind(xml: &str, username: &str) -> Vec<String> {
    let mut slugs = Vec::new();
    let mut rest = xml;
    while let Some(start) = rest.find("<D:href>").or_else(|| rest.find("<href>")) {
        let (open, close) = if rest[start..].starts_with("<D:") {
            ("<D:href>", "</D:href>")
        } else {
            ("<href>", "</href>")
        };
        let cs = start + open.len();
        let Some(ce) = rest[cs..].find(close).map(|i| cs + i) else {
            break;
        };
        let href = rest[cs..ce].trim();
        rest = &rest[ce + close.len()..];
        let slug = href.trim_end_matches('/').rsplit('/').next().unwrap_or("");
        if !slug.is_empty() && slug != username {
            slugs.push(slug.to_string());
        }
    }
    slugs
}

// ── Markdown sources ──────────────────────────────────────────────────────────

/// List all configured markdown sources for the current user.
#[server]
pub async fn list_markdown_sources() -> Result<Vec<MarkdownSourceInfo>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use sqlx::Row;

        let (pool, username) = auth().await?;

        let rows = sqlx::query(
            "SELECT id, name, type, config, config_managed, enabled, sync_interval_secs,
                    CAST(last_scan AS TEXT) AS last_scan, last_error,
                    COALESCE(color, 'var(--pal-md-1)') AS color
             FROM markdown_sources WHERE username = ? ORDER BY created_at",
        )
        .bind(&username)
        .fetch_all(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        return Ok(rows
            .into_iter()
            .map(|r| {
                let config_str: String = r.try_get("config").unwrap_or_default();
                let source_type: String = r
                    .try_get("type")
                    .unwrap_or_else(|_| "filesystem".to_string());
                let config_managed: i64 = r.try_get("config_managed").unwrap_or(0);
                let path = if source_type == "filesystem" {
                    serde_json::from_str::<serde_json::Value>(&config_str)
                        .ok()
                        .and_then(|v| {
                            v.get("path")
                                .and_then(|p| p.as_str())
                                .map(|s| s.to_string())
                        })
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                let (url, branch) = if source_type == "git" {
                    let parsed = serde_json::from_str::<serde_json::Value>(&config_str).ok();
                    let url = parsed
                        .as_ref()
                        .and_then(|v| v.get("url").and_then(|u| u.as_str()).map(str::to_string));
                    let branch = parsed
                        .and_then(|v| v.get("branch").and_then(|b| b.as_str()).map(str::to_string));
                    (url, branch)
                } else {
                    (None, None)
                };
                let enabled: i64 = r.try_get("enabled").unwrap_or(1);
                let sync_interval_secs: i64 = r.try_get("sync_interval_secs").unwrap_or(60);
                MarkdownSourceInfo {
                    id: r.try_get("id").unwrap_or_default(),
                    name: r.try_get("name").unwrap_or_default(),
                    path,
                    enabled: enabled != 0,
                    last_scan: r.try_get("last_scan").ok().flatten(),
                    color: r
                        .try_get("color")
                        .unwrap_or_else(|_| "var(--pal-md-1)".to_string()),
                    source_type,
                    url,
                    branch,
                    config_managed: config_managed != 0,
                    sync_interval_secs: sync_interval_secs as u32,
                    last_error: r.try_get("last_error").ok().flatten(),
                }
            })
            .collect());
    }
    #[allow(unreachable_code)]
    Ok(vec![])
}

/// Returns true if the server has an encryption key configured (TIDL_ENCRYPTION_KEY_FILE).
/// Used by the UI to decide whether to show the PAT field for git sources.
#[server]
pub async fn is_enc_key_configured() -> Result<bool, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use axum::Extension;
        use leptos_axum::extract;

        let _ = auth().await?; // must be authenticated
        let Extension(EncryptionKey(key)): Extension<EncryptionKey> = extract().await?;
        return Ok(key.is_some());
    }
    #[allow(unreachable_code)]
    Ok(false)
}

/// Encrypt a PAT for storage. AES-256-GCM: stores base64(nonce || ciphertext)
/// — matches `tidl-server`'s `crypto::decrypt_pat` format.
#[cfg(feature = "ssr")]
fn encrypt_pat_for_storage(plaintext: &str, key: &[u8]) -> Result<String, ServerFnError> {
    use aes_gcm::{
        aead::{Aead, AeadCore, KeyInit, OsRng},
        Aes256Gcm, Key,
    };
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

    let key_arr = Key::<Aes256Gcm>::from_slice(key);
    let cipher = Aes256Gcm::new(key_arr);
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plaintext.as_bytes())
        .map_err(|e| ServerFnError::new(format!("Encryption failed: {e}")))?;
    let mut combined = nonce.to_vec();
    combined.extend_from_slice(&ciphertext);
    Ok(BASE64.encode(&combined))
}

/// Decrypt a PAT stored by [`encrypt_pat_for_storage`].
#[cfg(feature = "ssr")]
fn decrypt_stored_pat(encoded: &str, key: &[u8]) -> Result<String, ServerFnError> {
    use aes_gcm::{
        aead::{Aead, KeyInit},
        Aes256Gcm, Key, Nonce,
    };
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

    let combined = BASE64
        .decode(encoded)
        .map_err(|e| ServerFnError::new(format!("Bad PAT encoding: {e}")))?;
    if combined.len() < 12 {
        return Err(ServerFnError::new("Stored PAT is truncated"));
    }
    let (nonce_bytes, ciphertext) = combined.split_at(12);
    let key_arr = Key::<Aes256Gcm>::from_slice(key);
    let cipher = Aes256Gcm::new(key_arr);
    let plaintext = cipher
        .decrypt(Nonce::from_slice(nonce_bytes), ciphertext)
        .map_err(|e| ServerFnError::new(format!("Decryption failed: {e}")))?;
    String::from_utf8(plaintext)
        .map_err(|e| ServerFnError::new(format!("Stored PAT is not UTF-8: {e}")))
}

/// Resolve a git source's branch: use it verbatim if the caller specified
/// one, otherwise ask the remote what it actually defaults to (Forgejo
/// instances commonly still default new repos to "master", not "main").
#[cfg(feature = "ssr")]
async fn resolve_git_branch(
    branch_input: &str,
    url: &str,
    pat: &str,
) -> Result<String, ServerFnError> {
    let trimmed = branch_input.trim();
    if !trimmed.is_empty() {
        return Ok(trimmed.to_string());
    }

    use crate::ssr_types::GitBranchDetector;
    use axum::Extension;
    use leptos_axum::extract;

    let Extension(detector): Extension<GitBranchDetector> = extract().await?;
    (detector.0)(url.to_string(), pat.to_string())
        .await
        .map_err(|e| {
            ServerFnError::new(format!(
                "Could not detect the remote's default branch: {e}. Specify a branch explicitly."
            ))
        })
}

/// Add a new git-backed markdown source. Encrypts the PAT with the app key.
#[server(input = server_fn::codec::Json)]
pub async fn add_git_source(source: NewGitSource) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::types::MIN_SYNC_INTERVAL_SECS;
        use axum::Extension;
        use leptos_axum::extract;

        let (pool, username) = auth().await?;
        let Extension(EncryptionKey(enc_key)): Extension<EncryptionKey> = extract().await?;

        if source.name.trim().is_empty() {
            return Err(ServerFnError::new("Name is required"));
        }
        if source.url.trim().is_empty() {
            return Err(ServerFnError::new("URL is required"));
        }
        if source.pat.trim().is_empty() {
            return Err(ServerFnError::new("PAT is required"));
        }
        if source.sync_interval_secs < MIN_SYNC_INTERVAL_SECS {
            return Err(ServerFnError::new(format!(
                "sync interval must be at least {MIN_SYNC_INTERVAL_SECS} seconds"
            )));
        }

        let key = enc_key.ok_or_else(|| {
            ServerFnError::new(
                "Encryption key not configured — set encryptionKeyFile in the NixOS module",
            )
        })?;

        let url = source.url.trim().to_string();
        let pat = source.pat.trim().to_string();
        let branch = resolve_git_branch(&source.branch, &url, &pat).await?;
        let pat_encrypted = encrypt_pat_for_storage(&pat, &key)?;

        let config = serde_json::json!({ "url": url, "branch": branch }).to_string();
        let color = if source.color.trim().is_empty() {
            "var(--pal-md-1)"
        } else {
            source.color.trim()
        };

        let inserted = sqlx::query(
            "INSERT INTO markdown_sources (username, name, type, config, enabled, color, pat_encrypted, sync_interval_secs)
             VALUES (?, ?, 'git', ?, 1, ?, ?, ?)",
        )
        .bind(&username)
        .bind(source.name.trim())
        .bind(&config)
        .bind(color)
        .bind(&pat_encrypted)
        .bind(source.sync_interval_secs as i64)
        .execute(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        // Sync straight away so the new source's items show up in the list
        // views (Focus/Tasks/Await) without waiting for the next poll tick.
        trigger_markdown_sync_now(inserted.last_insert_rowid()).await;

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Update an existing git-backed markdown source (name, URL, branch, colour,
/// and optionally the PAT). Leave `pat` blank to keep the currently stored one.
///
/// Config-managed (flake-provisioned) sources reject this — their URL/branch/PAT
/// are owned by the NixOS config, matching the read-only treatment already
/// applied to their row in the Settings UI.
#[server(input = server_fn::codec::Json)]
pub async fn update_git_source(
    source_id: i64,
    name: String,
    url: String,
    branch: String,
    color: String,
    pat: String,
    sync_interval_secs: u32,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::DataDir;
        use crate::types::MIN_SYNC_INTERVAL_SECS;
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::Row;

        let (pool, username) = auth().await?;
        let Extension(EncryptionKey(enc_key)): Extension<EncryptionKey> = extract().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;

        if name.trim().is_empty() {
            return Err(ServerFnError::new("Name is required"));
        }
        if url.trim().is_empty() {
            return Err(ServerFnError::new("URL is required"));
        }
        if sync_interval_secs < MIN_SYNC_INTERVAL_SECS {
            return Err(ServerFnError::new(format!(
                "sync interval must be at least {MIN_SYNC_INTERVAL_SECS} seconds"
            )));
        }

        let row = sqlx::query(
            "SELECT type, config, config_managed, pat_encrypted
             FROM markdown_sources WHERE id = ? AND username = ?",
        )
        .bind(source_id)
        .bind(&username)
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("Source not found"))?;

        let source_type: String = row.try_get("type").unwrap_or_default();
        if source_type != "git" {
            return Err(ServerFnError::new("Not a git source"));
        }
        let config_managed: i64 = row.try_get("config_managed").unwrap_or(0);
        if config_managed != 0 {
            // The NixOS config owns URL/branch/PAT for these sources — only
            // name, colour, and sync interval are ours to change, since they're
            // per-install preferences rather than part of the source's identity.
            let color = if color.trim().is_empty() {
                "var(--pal-md-1)"
            } else {
                color.trim()
            };
            sqlx::query(
                "UPDATE markdown_sources SET name = ?, color = ?, sync_interval_secs = ? WHERE id = ? AND username = ?",
            )
            .bind(name.trim())
            .bind(color)
            .bind(sync_interval_secs as i64)
            .bind(source_id)
            .bind(&username)
            .execute(&pool)
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
            trigger_markdown_sync_now(source_id).await;
            return Ok(());
        }

        let old_config: String = row.try_get("config").unwrap_or_default();
        let old_v: serde_json::Value = serde_json::from_str(&old_config).unwrap_or_default();
        let old_url = old_v
            .get("url")
            .and_then(|u| u.as_str())
            .unwrap_or("")
            .to_string();
        let old_branch = old_v
            .get("branch")
            .and_then(|b| b.as_str())
            .unwrap_or("")
            .to_string();
        let existing_pat_encrypted: Option<String> = row.try_get("pat_encrypted").ok().flatten();

        let key = enc_key.ok_or_else(|| {
            ServerFnError::new(
                "Encryption key not configured — set encryptionKeyFile in the NixOS module",
            )
        })?;

        let url = url.trim().to_string();
        let new_pat = pat.trim().to_string();
        let pat_for_ops = if !new_pat.is_empty() {
            new_pat.clone()
        } else {
            let existing = existing_pat_encrypted
                .clone()
                .ok_or_else(|| ServerFnError::new("Source has no stored PAT — enter one"))?;
            decrypt_stored_pat(&existing, &key)?
        };

        let branch = resolve_git_branch(&branch, &url, &pat_for_ops).await?;
        let config = serde_json::json!({ "url": url, "branch": branch }).to_string();
        let color = if color.trim().is_empty() {
            "var(--pal-md-1)"
        } else {
            color.trim()
        };

        if !new_pat.is_empty() {
            let pat_encrypted = encrypt_pat_for_storage(&new_pat, &key)?;
            sqlx::query(
                "UPDATE markdown_sources SET name = ?, config = ?, color = ?, pat_encrypted = ?, sync_interval_secs = ?
                 WHERE id = ? AND username = ?",
            )
            .bind(name.trim())
            .bind(&config)
            .bind(color)
            .bind(&pat_encrypted)
            .bind(sync_interval_secs as i64)
            .bind(source_id)
            .bind(&username)
            .execute(&pool)
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        } else {
            sqlx::query(
                "UPDATE markdown_sources SET name = ?, config = ?, color = ?, sync_interval_secs = ?
                 WHERE id = ? AND username = ?",
            )
            .bind(name.trim())
            .bind(&config)
            .bind(color)
            .bind(sync_interval_secs as i64)
            .bind(source_id)
            .bind(&username)
            .execute(&pool)
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?;
        }

        // The local clone under data_dir/repos/<id>/ tracks the *old* url/branch
        // (its .git/config still points at the old remote). If either changed,
        // wipe it so the next sync does a fresh clone against the new config
        // instead of silently pulling from a stale remote.
        if url != old_url || branch != old_branch {
            let local_path = data_dir.join("repos").join(source_id.to_string());
            let _ = tokio::fs::remove_dir_all(&local_path).await;
        }

        trigger_markdown_sync_now(source_id).await;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Add a new markdown source (filesystem path).
#[server(input = server_fn::codec::Json)]
pub async fn add_markdown_source(source: NewMarkdownSource) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::types::MIN_SYNC_INTERVAL_SECS;

        let (pool, username) = auth().await?;

        if source.name.trim().is_empty() {
            return Err(ServerFnError::new("Name is required"));
        }
        if source.path.trim().is_empty() {
            return Err(ServerFnError::new("Path is required"));
        }
        if source.sync_interval_secs < MIN_SYNC_INTERVAL_SECS {
            return Err(ServerFnError::new(format!(
                "sync interval must be at least {MIN_SYNC_INTERVAL_SECS} seconds"
            )));
        }

        let config = serde_json::json!({ "path": source.path.trim(), "recursive": true });
        let color = if source.color.trim().is_empty() {
            "var(--pal-md-1)"
        } else {
            source.color.trim()
        };

        let inserted = sqlx::query(
            "INSERT INTO markdown_sources (username, name, type, config, enabled, color, sync_interval_secs)
             VALUES (?, ?, 'filesystem', ?, 1, ?, ?)",
        )
        .bind(&username)
        .bind(source.name.trim())
        .bind(config.to_string())
        .bind(color)
        .bind(source.sync_interval_secs as i64)
        .execute(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        trigger_markdown_sync_now(inserted.last_insert_rowid()).await;

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Delete a markdown source and all its synced todos.
#[server(input = server_fn::codec::Json)]
pub async fn delete_markdown_source(source_id: i64) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        // Verify ownership before deleting.
        let exists = sqlx::query_scalar::<_, i64>(
            "SELECT COUNT(*) FROM markdown_sources WHERE id = ? AND username = ?",
        )
        .bind(source_id)
        .bind(&username)
        .fetch_one(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        if exists == 0 {
            return Err(ServerFnError::new("Source not found"));
        }

        // Remove all events synced from this source.
        // UIDs have the format "md:{source_id}:{rel_path}:{line}:{ordinal}", so matching
        // on the prefix "md:{source_id}:" is precise and avoids any path
        // ambiguity (source_file values are relative, not absolute).
        let uid_prefix = format!("md:{}:", source_id);
        sqlx::query(
            "DELETE FROM events
             WHERE username = ? AND source = 'markdown' AND uid LIKE ? ESCAPE '\\'",
        )
        .bind(&username)
        .bind(format!("{}%", uid_prefix))
        .execute(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        // Delete the source row (cascade deletes sync_state via FK).
        let affected = sqlx::query("DELETE FROM markdown_sources WHERE id = ? AND username = ?")
            .bind(source_id)
            .bind(&username)
            .execute(&pool)
            .await
            .map_err(|e| ServerFnError::new(e.to_string()))?
            .rows_affected();

        if affected == 0 {
            return Err(ServerFnError::new("Source not found"));
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Trigger an immediate sync for an existing markdown source.
/// Returns immediately — the sync runs in the background.
#[server]
pub async fn trigger_markdown_sync(source_id: i64) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use axum::Extension;
        use leptos_axum::extract;
        let (pool, username) = auth().await?;

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM markdown_sources WHERE id = ? AND username = ?",
        )
        .bind(source_id)
        .bind(&username)
        .fetch_one(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        if count == 0 {
            return Err(ServerFnError::new("source not found"));
        }

        if let Ok(Extension(trigger)) =
            extract::<Extension<crate::ssr_types::MarkdownSyncTrigger>>().await
        {
            let _ = trigger.0.send(source_id);
        }

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Best-effort nudge to the background markdown sync loop so an add/edit is
/// reflected in list views (Focus, Tasks, Await) without waiting for the next
/// 10-second poll. The trigger is only present in a running server; a missing
/// one (e.g. unit tests) is a silent no-op, since the poll still catches up.
#[cfg(feature = "ssr")]
async fn trigger_markdown_sync_now(source_id: i64) {
    use axum::Extension;
    use leptos_axum::extract;
    if let Ok(Extension(trigger)) =
        extract::<Extension<crate::ssr_types::MarkdownSyncTrigger>>().await
    {
        let _ = trigger.0.send(source_id);
    }
}

/// Update an existing markdown source (name, path, display colour, and sync interval).
#[server(input = server_fn::codec::Json)]
pub async fn update_markdown_source(
    source_id: i64,
    name: String,
    path: String,
    color: String,
    sync_interval_secs: u32,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::types::MIN_SYNC_INTERVAL_SECS;

        let (pool, username) = auth().await?;

        if name.trim().is_empty() {
            return Err(ServerFnError::new("Name is required"));
        }
        if path.trim().is_empty() {
            return Err(ServerFnError::new("Path is required"));
        }
        if sync_interval_secs < MIN_SYNC_INTERVAL_SECS {
            return Err(ServerFnError::new(format!(
                "sync interval must be at least {MIN_SYNC_INTERVAL_SECS} seconds"
            )));
        }

        let config = serde_json::json!({ "path": path.trim(), "recursive": true });
        let color = if color.trim().is_empty() {
            "var(--pal-md-1)"
        } else {
            color.trim()
        };

        let affected = sqlx::query(
            "UPDATE markdown_sources
             SET name = ?, config = ?, color = ?, sync_interval_secs = ?
             WHERE id = ? AND username = ?",
        )
        .bind(name.trim())
        .bind(config.to_string())
        .bind(color)
        .bind(sync_interval_secs as i64)
        .bind(source_id)
        .bind(&username)
        .execute(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?
        .rows_affected();

        if affected == 0 {
            return Err(ServerFnError::new("Source not found"));
        }
        trigger_markdown_sync_now(source_id).await;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(all(test, feature = "ssr"))]
mod remote_source_update_tests {
    use super::{update_remote_source_tx, UpdateRemoteCalDavSource};
    use sqlx::{Row, SqlitePool};

    /// Builds an `UpdateRemoteCalDavSource` matching `seed()`'s row, with
    /// only the given fields overridden from their seeded defaults.
    fn update(
        name: &str,
        remote_base_url: &str,
        remote_username: &str,
        remote_calendar: &str,
        local_calendar: &str,
        sync_direction: &str,
        sync_interval_secs: u32,
    ) -> UpdateRemoteCalDavSource {
        UpdateRemoteCalDavSource {
            name: name.to_string(),
            remote_base_url: remote_base_url.to_string(),
            remote_username: remote_username.to_string(),
            remote_token: None,
            remote_calendar: remote_calendar.to_string(),
            local_calendar: local_calendar.to_string(),
            sync_direction: sync_direction.to_string(),
            sync_interval_secs,
        }
    }

    async fn seed(pool: &SqlitePool, username: &str, source_id: &str) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) \
             VALUES (?, ?, ?, 'x', 0)",
        )
        .bind(username)
        .bind(format!("{username}@example.com"))
        .bind(username)
        .execute(pool)
        .await
        .unwrap();

        sqlx::query(
            "INSERT INTO remote_caldav_sources
                 (id, username, name, remote_base_url, remote_username, remote_token_enc,
                  remote_calendar, local_calendar, sync_direction, sync_interval_secs,
                  last_sync_token, last_push_token)
             VALUES (?, ?, 'work', 'https://old.example.com', 'olduser', 'oldtoken-enc',
                     'oldcal', 'local', 'both', 300, 'sync-tok-1', 42)",
        )
        .bind(source_id)
        .bind(username)
        .execute(pool)
        .await
        .unwrap();
    }

    async fn row(pool: &SqlitePool, source_id: &str) -> sqlx::sqlite::SqliteRow {
        sqlx::query(
            "SELECT name, remote_base_url, remote_username, remote_token_enc, remote_calendar,
                    local_calendar, sync_direction, sync_interval_secs, last_sync_token,
                    last_push_token
             FROM remote_caldav_sources WHERE id = ?",
        )
        .bind(source_id)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn non_critical_edit_preserves_sync_tokens(pool: SqlitePool) {
        seed(&pool, "alice", "src-1").await;

        let u = update(
            "renamed",
            "https://old.example.com",
            "olduser",
            "oldcal",
            "local",
            "pull",
            600,
        );
        update_remote_source_tx(&pool, "alice", "src-1", &u, None)
            .await
            .unwrap();

        let r = row(&pool, "src-1").await;
        assert_eq!(r.get::<String, _>("name"), "renamed");
        assert_eq!(r.get::<String, _>("sync_direction"), "pull");
        assert_eq!(r.get::<i64, _>("sync_interval_secs"), 600);
        assert_eq!(r.get::<String, _>("remote_token_enc"), "oldtoken-enc");
        assert_eq!(
            r.get::<Option<String>, _>("last_sync_token"),
            Some("sync-tok-1".to_string())
        );
        assert_eq!(r.get::<Option<i64>, _>("last_push_token"), Some(42));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn changing_remote_target_resets_sync_tokens(pool: SqlitePool) {
        seed(&pool, "alice", "src-1").await;

        let u = update(
            "work",
            "https://new.example.com",
            "olduser",
            "oldcal",
            "local",
            "both",
            300,
        );
        update_remote_source_tx(&pool, "alice", "src-1", &u, None)
            .await
            .unwrap();

        let r = row(&pool, "src-1").await;
        assert_eq!(r.get::<Option<String>, _>("last_sync_token"), None);
        assert_eq!(r.get::<Option<i64>, _>("last_push_token"), None);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn changing_local_calendar_updates_mapping_and_resets_sync_tokens(pool: SqlitePool) {
        seed(&pool, "alice", "src-1").await;

        let u = update(
            "work",
            "https://old.example.com",
            "olduser",
            "oldcal",
            "different-local",
            "both",
            300,
        );
        update_remote_source_tx(&pool, "alice", "src-1", &u, None)
            .await
            .unwrap();

        let r = row(&pool, "src-1").await;
        assert_eq!(r.get::<String, _>("local_calendar"), "different-local");
        // Reset to NULL — the next sync's pull() wipes any pre-existing
        // events in the new calendar, then `seed_last_push_token_if_unset`
        // re-seeds the cursor past that wipe's own tombstones.
        assert_eq!(r.get::<Option<String>, _>("last_sync_token"), None);
        assert_eq!(r.get::<Option<i64>, _>("last_push_token"), None);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn blank_token_keeps_existing_encrypted_token(pool: SqlitePool) {
        seed(&pool, "alice", "src-1").await;

        let u = update(
            "work",
            "https://old.example.com",
            "olduser",
            "oldcal",
            "local",
            "both",
            300,
        );
        update_remote_source_tx(&pool, "alice", "src-1", &u, None)
            .await
            .unwrap();

        let r = row(&pool, "src-1").await;
        assert_eq!(r.get::<String, _>("remote_token_enc"), "oldtoken-enc");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn new_token_replaces_encrypted_token(pool: SqlitePool) {
        seed(&pool, "alice", "src-1").await;

        let u = update(
            "work",
            "https://old.example.com",
            "olduser",
            "oldcal",
            "local",
            "both",
            300,
        );
        update_remote_source_tx(&pool, "alice", "src-1", &u, Some("newtoken-enc"))
            .await
            .unwrap();

        let r = row(&pool, "src-1").await;
        assert_eq!(r.get::<String, _>("remote_token_enc"), "newtoken-enc");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn errors_for_missing_or_unowned_source(pool: SqlitePool) {
        seed(&pool, "alice", "src-1").await;

        let u = update(
            "work",
            "https://old.example.com",
            "olduser",
            "oldcal",
            "local",
            "both",
            300,
        );

        let err = update_remote_source_tx(&pool, "bob", "src-1", &u, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not found"));

        let err = update_remote_source_tx(&pool, "alice", "no-such-id", &u, None)
            .await
            .unwrap_err();
        assert!(err.to_string().contains("not found"));
    }
}

#[cfg(all(test, feature = "ssr"))]
mod clear_shared_calendar_tests {
    use super::clear_shared_calendar_tx;
    use sqlx::{Row, SqlitePool};

    async fn seed_user(pool: &SqlitePool, username: &str) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) \
             VALUES (?, ?, ?, 'x', 0)",
        )
        .bind(username)
        .bind(format!("{username}@example.com"))
        .bind(username)
        .execute(pool)
        .await
        .unwrap();
    }

    /// Seeds `owner`'s "work" calendar with two events, and (if `permission`
    /// is `Some`) a `calendar_shares` grant to `grantee` at that permission.
    async fn seed(pool: &SqlitePool, owner: &str, grantee: &str, permission: Option<&str>) {
        seed_user(pool, owner).await;
        seed_user(pool, grantee).await;

        sqlx::query(
            "INSERT INTO calendars (username, name, display_name, color, sync_token) \
             VALUES (?, 'work', 'Work', '#000', 5)",
        )
        .bind(owner)
        .execute(pool)
        .await
        .unwrap();

        for uid in ["ev-1", "ev-2"] {
            sqlx::query(
                "INSERT INTO events (username, calendar, uid, type, summary, etag, ical_data) \
                 VALUES (?, 'work', ?, 'VEVENT', 'test', 'etag1', 'BEGIN:VEVENT\r\nEND:VEVENT')",
            )
            .bind(owner)
            .bind(uid)
            .execute(pool)
            .await
            .unwrap();
        }

        if let Some(perm) = permission {
            sqlx::query(
                "INSERT INTO calendar_shares (owner_username, calendar_name, grantee_username, permission) \
                 VALUES (?, 'work', ?, ?)",
            )
            .bind(owner)
            .bind(grantee)
            .bind(perm)
            .execute(pool)
            .await
            .unwrap();
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn read_only_grantee_is_rejected(pool: SqlitePool) {
        seed(&pool, "alice", "bob", Some("read")).await;

        let err = clear_shared_calendar_tx(&pool, "bob", "alice", "work")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("write access"));

        let remaining: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events WHERE username = 'alice' AND calendar = 'work'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(remaining, 2, "read-only grantee must not delete any events");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn ungranted_user_is_rejected(pool: SqlitePool) {
        seed(&pool, "alice", "bob", None).await;

        let err = clear_shared_calendar_tx(&pool, "bob", "alice", "work")
            .await
            .unwrap_err();
        assert!(err.to_string().contains("write access"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn write_grantee_clears_events_bumps_token_and_tombstones(pool: SqlitePool) {
        seed(&pool, "alice", "bob", Some("read-write")).await;

        let grantees = clear_shared_calendar_tx(&pool, "bob", "alice", "work")
            .await
            .unwrap();
        assert_eq!(grantees, vec!["bob".to_string()]);

        let remaining: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM events WHERE username = 'alice' AND calendar = 'work'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(remaining, 0);

        let token: i64 = sqlx::query_scalar(
            "SELECT sync_token FROM calendars WHERE username = 'alice' AND name = 'work'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(token, 6, "sync_token must bump from its seeded value of 5");

        let tombstones = sqlx::query(
            "SELECT uid FROM sync_log WHERE username = 'alice' AND calendar = 'work' AND operation = 'deleted'"
        )
        .fetch_all(&pool).await.unwrap();
        let mut uids: Vec<String> = tombstones.iter().map(|r| r.get("uid")).collect();
        uids.sort();
        assert_eq!(uids, vec!["ev-1".to_string(), "ev-2".to_string()]);
    }
}
