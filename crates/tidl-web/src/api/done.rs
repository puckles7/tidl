//! Done-item lifecycle server functions (backlog #124, #125).
//!
//! Backs the central Done view and the clean-up engine. `done_items` is a cache
//! over three origins (markdown sources, note bodies and CalDAV VTODOs); this
//! module owns its reconciliation, the per-item / bulk purge, re-open, and the
//! per-scope clean-up settings.
//!
//! Reconciliation runs on every Done-view load (see [`reconcile_done_items`]):
//! a scan can regenerate live rows but can never regenerate archived ones, so it
//! upserts live rows and archives any live row whose origin record has vanished.
//! Re-open deletes the live row explicitly (so reconcile does not mistake a
//! restored `#task` for an archived tag).
//!
//! See `docs/superpowers/specs/2026-10-08-done-item-lifecycle-design.md`.

use leptos::prelude::*;

use crate::types::{
    DoneFilter, DoneItem, DoneScope, DoneSettings, PurgePreview, PurgeResult, Retention,
};

#[cfg(feature = "ssr")]
use super::auth;

// ── Reconciliation (SSR) ──────────────────────────────────────────────────────

/// One live done item discovered by an origin scan, ready to upsert.
#[cfg(feature = "ssr")]
struct Scanned {
    origin: &'static str,
    scope_id: String,
    origin_key: String,
    kind: String,
    slug: String,
    title: String,
    priority: Option<i64>,
    due_date: Option<String>,
    completed_at: String,
    source_label: String,
    source_file: Option<String>,
    note_id: Option<i64>,
    calendar: Option<String>,
}

/// Reconcile the whole index for `username` against all three origins. Called
/// on every Done-view load (and the startup backfill). Read-only origins that
/// fail to scan are skipped entirely — never archived — so a temporarily
/// missing directory cannot wipe history.
#[cfg(feature = "ssr")]
pub async fn reconcile_done_items(
    pool: &sqlx::SqlitePool,
    username: &str,
    data_dir: &std::path::Path,
) -> Result<(), ServerFnError> {
    reconcile_caldav(pool, username).await?;
    reconcile_notes(pool, username).await?;
    reconcile_markdown(pool, username, data_dir).await?;
    Ok(())
}

/// Upsert a single live row. Atomic `INSERT … ON CONFLICT` keyed on the
/// partial unique index `(username, origin, origin_key) WHERE archived_at IS
/// NULL`, so concurrent reconciles (the Done screen loads `list_done_items` and
/// `list_done_scopes` in parallel, both of which reconcile) can never insert two
/// live rows for one origin. A fresh insert takes the next generation so an
/// archived tombstone with the same key is never clobbered.
#[cfg(feature = "ssr")]
async fn upsert_live(
    pool: &sqlx::SqlitePool,
    username: &str,
    s: &Scanned,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "INSERT INTO done_items \
         (username, origin, scope_id, origin_key, generation, kind, slug, title, priority, \
          due_date, completed_at, source_label, source_file, note_id, calendar) \
         VALUES (?, ?, ?, ?, \
           COALESCE((SELECT MAX(generation) + 1 FROM done_items \
                     WHERE username = ? AND origin = ? AND origin_key = ?), 0), \
           ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) \
         ON CONFLICT(username, origin, origin_key) WHERE archived_at IS NULL \
         DO UPDATE SET \
           scope_id = excluded.scope_id, kind = excluded.kind, slug = excluded.slug, \
           title = excluded.title, priority = excluded.priority, due_date = excluded.due_date, \
           completed_at = excluded.completed_at, source_label = excluded.source_label, \
           source_file = excluded.source_file, note_id = excluded.note_id, \
           calendar = excluded.calendar",
    )
    .bind(username)
    .bind(s.origin)
    .bind(&s.scope_id)
    .bind(&s.origin_key)
    .bind(username)
    .bind(s.origin)
    .bind(&s.origin_key)
    .bind(&s.kind)
    .bind(&s.slug)
    .bind(&s.title)
    .bind(s.priority)
    .bind(&s.due_date)
    .bind(&s.completed_at)
    .bind(&s.source_label)
    .bind(&s.source_file)
    .bind(s.note_id)
    .bind(&s.calendar)
    .execute(pool)
    .await?;
    Ok(())
}

/// Archive every live row in `scope_id` whose `origin_key` is absent from
/// `present` (the tag/token/event is gone from the origin). Archived rows
/// are never touched.
#[cfg(feature = "ssr")]
async fn archive_missing(
    pool: &sqlx::SqlitePool,
    username: &str,
    origin: &str,
    scope_id: &str,
    present: &std::collections::HashSet<String>,
) -> Result<(), sqlx::Error> {
    let rows: Vec<(i64, String)> = sqlx::query_as(
        "SELECT id, origin_key FROM done_items \
         WHERE username = ? AND origin = ? AND scope_id = ? AND archived_at IS NULL",
    )
    .bind(username)
    .bind(origin)
    .bind(scope_id)
    .fetch_all(pool)
    .await?;
    for (id, key) in rows {
        if !present.contains(&key) {
            sqlx::query("UPDATE done_items SET archived_at = datetime('now') WHERE id = ?")
                .bind(id)
                .execute(pool)
                .await?;
        }
    }
    Ok(())
}

/// The `events` columns every CalDAV done-row scan needs, aliased to the names
/// [`caldav_scanned`] reads.
#[cfg(feature = "ssr")]
const CALDAV_DONE_COLUMNS: &str = "uid, COALESCE(summary, '') AS summary, \
     COALESCE(kind, 'task') AS kind, priority, CAST(due_date AS TEXT) AS due_date, calendar, \
     topic, COALESCE(CAST(completed_date AS TEXT), CAST(updated_at AS TEXT), datetime('now')) \
       AS completed_at";

/// Build the live-row fields for one completed CalDAV VTODO from its `events`
/// row. Shared by the full [`reconcile_caldav`] scan and the immediate
/// external-completion hook [`upsert_caldav_done_item`] (backlog #136).
#[cfg(feature = "ssr")]
fn caldav_scanned(r: &sqlx::sqlite::SqliteRow) -> (String, Scanned) {
    use sqlx::Row;

    let uid: String = r.try_get("uid").unwrap_or_default();
    let summary: String = r.try_get("summary").unwrap_or_default();
    let kind: String = r.try_get("kind").unwrap_or_default();
    let calendar: String = r.try_get("calendar").unwrap_or_default();
    let topic: Option<String> = r.try_get("topic").unwrap_or_default();
    let priority: Option<i64> = r.try_get("priority").unwrap_or_default();
    let due: Option<String> = r.try_get("due_date").unwrap_or_default();
    let completed: String = r.try_get("completed_at").unwrap_or_default();
    let slug = topic
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| crate::note_tokens::slugify(&summary));
    let title = if summary.is_empty() {
        crate::note_tokens::slug_to_title(&slug)
    } else {
        summary
    };
    let scanned = Scanned {
        origin: "caldav",
        scope_id: calendar.clone(),
        origin_key: uid.clone(),
        kind: if kind == "await" {
            "await".into()
        } else {
            "task".into()
        },
        slug,
        title,
        priority,
        due_date: due.map(|d| d.chars().take(10).collect()),
        completed_at: completed.chars().take(10).collect(),
        source_label: calendar.clone(),
        source_file: None,
        note_id: None,
        calendar: Some(calendar),
    };
    (uid, scanned)
}

#[cfg(feature = "ssr")]
async fn reconcile_caldav(pool: &sqlx::SqlitePool, username: &str) -> Result<(), ServerFnError> {
    let rows = sqlx::query(&format!(
        "SELECT {CALDAV_DONE_COLUMNS} \
         FROM events WHERE username = ? AND type = 'VTODO' AND status = 'COMPLETED'"
    ))
    .bind(username)
    .fetch_all(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let mut present: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();
    for r in rows {
        let (uid, scanned) = caldav_scanned(&r);
        present
            .entry(scanned.scope_id.clone())
            .or_default()
            .insert(uid);
        upsert_live(pool, username, &scanned)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }

    // Archive caldav live rows whose event is no longer completed/around.
    let calendars: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT calendar FROM done_items WHERE username = ? AND origin = 'caldav'",
    )
    .bind(username)
    .fetch_all(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    for cal in calendars {
        let empty = std::collections::HashSet::new();
        archive_missing(
            pool,
            username,
            "caldav",
            &cal,
            present.get(&cal).unwrap_or(&empty),
        )
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }
    Ok(())
}

#[cfg(feature = "ssr")]
async fn reconcile_notes(pool: &sqlx::SqlitePool, username: &str) -> Result<(), ServerFnError> {
    use sqlx::Row;

    let rows = sqlx::query(
        "SELECT n.id, n.collection_id, n.text, c.name AS collection_name \
         FROM notes n JOIN note_collections c ON c.id = n.collection_id \
         WHERE c.owner_username = ? \
            OR c.id IN (SELECT collection_id FROM note_collection_members WHERE member_username = ?)",
    )
    .bind(username)
    .bind(username)
    .fetch_all(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let mut present: std::collections::HashMap<String, std::collections::HashSet<String>> =
        std::collections::HashMap::new();
    for r in rows {
        let note_id: i64 = r.try_get("id").unwrap_or_default();
        let collection_id: i64 = r.try_get("collection_id").unwrap_or_default();
        let collection_name: String = r.try_get("collection_name").unwrap_or_default();
        let text: String = r.try_get("text").unwrap_or_default();
        let scope = collection_id.to_string();

        let tokens = crate::note_tokens::scan_note_tokens(&text);
        for (idx, t) in tokens.iter().enumerate() {
            let crate::note_tokens::ScannedTokenKind::Done {
                completed,
                original,
                slug,
            } = &t.kind
            else {
                continue;
            };
            let ordinal = tokens[..idx].iter().filter(|o| o.line == t.line).count();
            let (kind, priority, due) = match original {
                crate::note_tokens::DoneOriginal::Task { priority } => {
                    ("task", Some(*priority as i64), None)
                }
                crate::note_tokens::DoneOriginal::Await { date } => {
                    ("await", None, Some(date.format("%Y-%m-%d").to_string()))
                }
            };
            let origin_key = format!("note:{note_id}:{}:{ordinal}", t.line);
            present
                .entry(scope.clone())
                .or_default()
                .insert(origin_key.clone());
            upsert_live(
                pool,
                username,
                &Scanned {
                    origin: "note",
                    scope_id: scope.clone(),
                    origin_key,
                    kind: kind.into(),
                    slug: slug.clone(),
                    title: crate::note_tokens::slug_to_title(slug),
                    priority,
                    due_date: due,
                    completed_at: completed.format("%Y-%m-%d").to_string(),
                    source_label: collection_name.clone(),
                    source_file: None,
                    note_id: Some(note_id),
                    calendar: None,
                },
            )
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        }
    }

    let scopes: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT scope_id FROM done_items WHERE username = ? AND origin = 'note'",
    )
    .bind(username)
    .fetch_all(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    for scope in scopes {
        let empty = std::collections::HashSet::new();
        archive_missing(
            pool,
            username,
            "note",
            &scope,
            present.get(&scope).unwrap_or(&empty),
        )
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }
    Ok(())
}

#[cfg(feature = "ssr")]
async fn reconcile_markdown(
    pool: &sqlx::SqlitePool,
    username: &str,
    data_dir: &std::path::Path,
) -> Result<(), ServerFnError> {
    use sqlx::Row;

    let sources = sqlx::query(
        "SELECT id, name, type, config FROM markdown_sources WHERE username = ? AND enabled = 1",
    )
    .bind(username)
    .fetch_all(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    for src in sources {
        let source_id: i64 = src.try_get("id").unwrap_or_default();
        let name: String = src.try_get("name").unwrap_or_default();
        let source_type: String = src.try_get("type").unwrap_or_default();
        let config_json: String = src.try_get("config").unwrap_or_default();
        let config: serde_json::Value = serde_json::from_str(&config_json).unwrap_or_default();

        let Some(base_dir) =
            super::files::resolve_markdown_source_dir(&source_type, &config, source_id, data_dir)
        else {
            continue;
        };
        // Skip a source we can't read rather than archiving all its rows.
        if !tokio::fs::metadata(&base_dir)
            .await
            .map(|m| m.is_dir())
            .unwrap_or(false)
        {
            continue;
        }

        let mut rel_paths = Vec::new();
        super::files::walk_markdown_files(&base_dir, &base_dir, &mut rel_paths).await;

        let scope = source_id.to_string();
        let mut present: std::collections::HashSet<String> = std::collections::HashSet::new();
        let mut readable = true;
        for rel_path in rel_paths {
            let Ok(content) = tokio::fs::read_to_string(base_dir.join(&rel_path)).await else {
                readable = false;
                continue;
            };
            for tag in tidl_markdown::scan_done_tags(&content) {
                let origin_key = format!("md:{source_id}:{rel_path}:{}:{}", tag.line, tag.ordinal);
                present.insert(origin_key.clone());
                let (kind, priority, due) = match tag.kind {
                    tidl_markdown::DoneKind::Task => ("task", tag.priority.map(|p| p as i64), None),
                    tidl_markdown::DoneKind::DatedTask => (
                        "task",
                        None,
                        tag.due_date.map(|d| d.format("%Y-%m-%d").to_string()),
                    ),
                    tidl_markdown::DoneKind::Await => (
                        "await",
                        None,
                        tag.due_date.map(|d| d.format("%Y-%m-%d").to_string()),
                    ),
                };
                upsert_live(
                    pool,
                    username,
                    &Scanned {
                        origin: "markdown",
                        scope_id: scope.clone(),
                        origin_key,
                        kind: kind.into(),
                        slug: tag.slug.clone(),
                        title: tidl_markdown::slug_to_title(&tag.slug),
                        priority,
                        due_date: due,
                        completed_at: tag.completed.format("%Y-%m-%d").to_string(),
                        source_label: name.clone(),
                        source_file: Some(rel_path.clone()),
                        note_id: None,
                        calendar: None,
                    },
                )
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            }
        }
        if readable {
            archive_missing(pool, username, "markdown", &scope, &present)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        }
    }
    Ok(())
}

/// Delete the live done row for an origin record being re-opened, so the next
/// reconcile does not mistake the restored active tag for an archived one.
#[cfg(feature = "ssr")]
pub async fn forget_done_item(
    pool: &sqlx::SqlitePool,
    username: &str,
    origin: &str,
    origin_key: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "DELETE FROM done_items WHERE username = ? AND origin = ? AND origin_key = ? \
         AND archived_at IS NULL",
    )
    .bind(username)
    .bind(origin)
    .bind(origin_key)
    .execute(pool)
    .await?;
    Ok(())
}

/// Immediately upsert the live `done_items` row for a single completed CalDAV
/// VTODO (backlog #136), so an external client's completion lands on the Done
/// screen without waiting for the next Done-view reconcile. A no-op unless the
/// event is still a `COMPLETED` VTODO — e.g. once a note-mirror completion has
/// deleted the row, or if the event vanished. Produces exactly what
/// [`reconcile_caldav`] would for this uid.
#[cfg(feature = "ssr")]
pub(crate) async fn upsert_caldav_done_item(
    pool: &sqlx::SqlitePool,
    username: &str,
    calendar: &str,
    uid: &str,
) -> Result<(), sqlx::Error> {
    let row = sqlx::query(&format!(
        "SELECT {CALDAV_DONE_COLUMNS} \
         FROM events WHERE username = ? AND calendar = ? AND uid = ? \
           AND type = 'VTODO' AND status = 'COMPLETED'"
    ))
    .bind(username)
    .bind(calendar)
    .bind(uid)
    .fetch_optional(pool)
    .await?;
    if let Some(r) = row {
        let (_, scanned) = caldav_scanned(&r);
        upsert_live(pool, username, &scanned).await?;
    }
    Ok(())
}

/// Archive the live `done_items` row for a CalDAV VTODO that an external client
/// deleted or un-completed (backlog #136), rather than waiting for the next
/// reconcile to notice the origin is gone. Archived history is kept (never
/// deleted); a no-op when there is no live row.
#[cfg(feature = "ssr")]
pub(crate) async fn archive_caldav_done_item(
    pool: &sqlx::SqlitePool,
    username: &str,
    uid: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        "UPDATE done_items SET archived_at = datetime('now') \
         WHERE username = ? AND origin = 'caldav' AND origin_key = ? AND archived_at IS NULL",
    )
    .bind(username)
    .bind(uid)
    .execute(pool)
    .await?;
    Ok(())
}

// ── Read-side server functions ────────────────────────────────────────────────

/// List done items for the current user, reconciling the index first. Live rows
/// only unless `filter.include_archived` is set; archived rows older than the
/// user's archive-retention window are pruned lazily.
#[server]
pub async fn list_done_items(filter: DoneFilter) -> Result<Vec<DoneItem>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::DataDir;
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::Row;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;
        reconcile_done_items(&pool, &username, &data_dir).await?;
        prune_archived(&pool, &username).await?;

        let rows = sqlx::query(
            "SELECT id, origin, scope_id, origin_key, kind, slug, title, priority, \
                    CAST(due_date AS TEXT) AS due_date, CAST(completed_at AS TEXT) AS completed_at, \
                    CAST(archived_at AS TEXT) AS archived_at, source_label, source_file, note_id, \
                    calendar \
             FROM done_items WHERE username = ? ORDER BY completed_at DESC, id DESC",
        )
        .bind(&username)
        .fetch_all(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Source colours for the card stripe / source pill (mirrors Focus).
        let md_colors: std::collections::HashMap<String, String> = sqlx::query_as(
            "SELECT id, COALESCE(color, 'var(--pal-md-1)') FROM markdown_sources WHERE username = ?",
        )
        .bind(&username)
        .fetch_all(&pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(id, c): (i64, String)| (id.to_string(), c))
        .collect();
        let nc_colors: std::collections::HashMap<String, String> = sqlx::query_as(
            "SELECT id, color FROM note_collections WHERE owner_username = ? \
                OR id IN (SELECT collection_id FROM note_collection_members WHERE member_username = ?)",
        )
        .bind(&username)
        .bind(&username)
        .fetch_all(&pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|(id, c): (i64, String)| (id.to_string(), c))
        .collect();
        let cal_colors: std::collections::HashMap<String, String> = sqlx::query_as(
            "SELECT name, COALESCE(color, '#cfd2d6') FROM calendars WHERE username = ?",
        )
        .bind(&username)
        .fetch_all(&pool)
        .await
        .unwrap_or_default()
        .into_iter()
        .collect();

        let query_lc = filter.query.as_deref().map(|q| q.to_lowercase());
        let items = rows
            .into_iter()
            .filter_map(|r| {
                let archived_at: Option<String> = r.try_get("archived_at").unwrap_or_default();
                if archived_at.is_some() && !filter.include_archived {
                    return None;
                }
                let origin: String = r.try_get("origin").unwrap_or_default();
                let scope_id: String = r.try_get("scope_id").unwrap_or_default();
                let color = match origin.as_str() {
                    "markdown" => md_colors.get(&scope_id),
                    "note" => nc_colors.get(&scope_id),
                    _ => cal_colors.get(&scope_id),
                }
                .cloned()
                .unwrap_or_else(|| "#cfd2d6".to_string());
                let item = DoneItem {
                    id: r.try_get("id").unwrap_or_default(),
                    origin: origin.clone(),
                    scope_id,
                    origin_key: r.try_get("origin_key").unwrap_or_default(),
                    kind: r.try_get("kind").unwrap_or_default(),
                    slug: r.try_get("slug").unwrap_or_default(),
                    title: r.try_get("title").unwrap_or_default(),
                    priority: r
                        .try_get::<Option<i64>, _>("priority")
                        .unwrap_or_default()
                        .map(|v| v as u32),
                    due_date: r
                        .try_get::<Option<String>, _>("due_date")
                        .unwrap_or_default()
                        .map(|d| d.chars().take(10).collect()),
                    completed_at: r
                        .try_get::<Option<String>, _>("completed_at")
                        .unwrap_or_default()
                        .unwrap_or_default()
                        .chars()
                        .take(10)
                        .collect(),
                    archived_at,
                    source_label: r.try_get("source_label").unwrap_or_default(),
                    color,
                    source_file: r.try_get("source_file").unwrap_or_default(),
                    note_id: r.try_get("note_id").unwrap_or_default(),
                    calendar: r.try_get("calendar").unwrap_or_default(),
                };
                if let Some(o) = &filter.origin {
                    if &item.origin != o {
                        return None;
                    }
                }
                if let Some(s) = &filter.scope_id {
                    if &item.scope_id != s {
                        return None;
                    }
                }
                if let Some(since) = &filter.since {
                    if &item.completed_at < since {
                        return None;
                    }
                }
                if let Some(until) = &filter.until {
                    if &item.completed_at > until {
                        return None;
                    }
                }
                if let Some(q) = &query_lc {
                    if !item.title.to_lowercase().contains(q)
                        && !item.slug.to_lowercase().contains(q)
                    {
                        return None;
                    }
                }
                Some(item)
            })
            .collect();

        return Ok(items);
    }
    #[allow(unreachable_code)]
    Ok(Vec::new())
}

/// Prune archived rows past the user's configured retention window. `NULL`
/// (blank) keeps archived history indefinitely; a window of `0` keeps no history
/// at all, deleting every archived row.
#[cfg(feature = "ssr")]
async fn prune_archived(pool: &sqlx::SqlitePool, username: &str) -> Result<(), ServerFnError> {
    let days: Option<i64> =
        sqlx::query_scalar("SELECT archive_retention_days FROM done_settings WHERE username = ?")
            .bind(username)
            .fetch_optional(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
            .flatten();
    let Some(days) = days else {
        return Ok(());
    };
    if days <= 0 {
        // A zero (or, defensively, negative) window keeps no archived history:
        // delete every archived row outright. A window query with `-0 days`
        // would miss a row archived in the same second.
        sqlx::query("DELETE FROM done_items WHERE username = ? AND archived_at IS NOT NULL")
            .bind(username)
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        return Ok(());
    }
    sqlx::query(
        "DELETE FROM done_items WHERE username = ? AND archived_at IS NOT NULL \
         AND archived_at < datetime('now', ?)",
    )
    .bind(username)
    .bind(format!("-{days} days"))
    .execute(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    Ok(())
}

/// Every clean-up scope (markdown source / note collection / calendar) with its
/// settings and live/archived counts, for the filter and Clean up… dialog.
#[server]
pub async fn list_done_scopes() -> Result<Vec<DoneScope>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::DataDir;
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::Row;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;
        reconcile_done_items(&pool, &username, &data_dir).await?;

        let mut scopes: Vec<DoneScope> = Vec::new();

        let rows = sqlx::query(
            "SELECT id, name FROM markdown_sources WHERE username = ? AND enabled = 1 ORDER BY name",
        )
        .bind(&username)
        .fetch_all(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        for r in rows {
            scopes.push(DoneScope {
                origin: "markdown".into(),
                scope_id: r.try_get::<i64, _>("id").unwrap_or_default().to_string(),
                label: r.try_get("name").unwrap_or_default(),
                live_count: 0,
                archived_count: 0,
                auto_archive: false,
                keep_newest: None,
                older_than_days: None,
            });
        }

        let rows = sqlx::query(
            "SELECT id, name FROM note_collections \
             WHERE owner_username = ? \
                OR id IN (SELECT collection_id FROM note_collection_members WHERE member_username = ?) \
             ORDER BY name",
        )
        .bind(&username)
        .bind(&username)
        .fetch_all(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        for r in rows {
            scopes.push(DoneScope {
                origin: "note".into(),
                scope_id: r.try_get::<i64, _>("id").unwrap_or_default().to_string(),
                label: r.try_get("name").unwrap_or_default(),
                live_count: 0,
                archived_count: 0,
                auto_archive: false,
                keep_newest: None,
                older_than_days: None,
            });
        }

        let rows = sqlx::query("SELECT name FROM calendars WHERE username = ? ORDER BY name")
            .bind(&username)
            .fetch_all(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        for r in rows {
            let name: String = r.try_get("name").unwrap_or_default();
            scopes.push(DoneScope {
                origin: "caldav".into(),
                scope_id: name.clone(),
                label: name,
                live_count: 0,
                archived_count: 0,
                auto_archive: false,
                keep_newest: None,
                older_than_days: None,
            });
        }

        // Counts + settings.
        for scope in &mut scopes {
            let live: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM done_items WHERE username = ? AND origin = ? \
                 AND scope_id = ? AND archived_at IS NULL",
            )
            .bind(&username)
            .bind(&scope.origin)
            .bind(&scope.scope_id)
            .fetch_one(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            let archived: i64 = sqlx::query_scalar(
                "SELECT COUNT(*) FROM done_items WHERE username = ? AND origin = ? \
                 AND scope_id = ? AND archived_at IS NOT NULL",
            )
            .bind(&username)
            .bind(&scope.origin)
            .bind(&scope.scope_id)
            .fetch_one(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            scope.live_count = live as u32;
            scope.archived_count = archived as u32;

            let row = sqlx::query(
                "SELECT auto_archive, keep_newest, older_than_days FROM done_scopes \
                 WHERE username = ? AND origin = ? AND scope_id = ?",
            )
            .bind(&username)
            .bind(&scope.origin)
            .bind(&scope.scope_id)
            .fetch_optional(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            if let Some(row) = row {
                scope.auto_archive = row.try_get::<i64, _>("auto_archive").unwrap_or(0) != 0;
                scope.keep_newest = row
                    .try_get::<Option<i64>, _>("keep_newest")
                    .unwrap_or_default()
                    .map(|v| v as u32);
                scope.older_than_days = row
                    .try_get::<Option<i64>, _>("older_than_days")
                    .unwrap_or_default()
                    .map(|v| v as u32);
            }
        }

        return Ok(scopes);
    }
    #[allow(unreachable_code)]
    Ok(Vec::new())
}

/// Read the user's archive settings.
#[server]
pub async fn get_done_settings() -> Result<DoneSettings, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        let days: Option<i64> = sqlx::query_scalar(
            "SELECT archive_retention_days FROM done_settings WHERE username = ?",
        )
        .bind(&username)
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
        .flatten();
        return Ok(DoneSettings {
            archive_retention_days: days.map(|d| d as u32),
        });
    }
    #[allow(unreachable_code)]
    Ok(DoneSettings::default())
}

/// Persist the user's archive settings.
#[server(input = server_fn::codec::Json)]
pub async fn set_done_settings(settings: DoneSettings) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        sqlx::query(
            "INSERT INTO done_settings (username, archive_retention_days, updated_at) \
             VALUES (?, ?, datetime('now')) \
             ON CONFLICT(username) DO UPDATE SET \
               archive_retention_days = excluded.archive_retention_days, \
               updated_at = datetime('now')",
        )
        .bind(&username)
        .bind(settings.archive_retention_days.map(|d| d as i64))
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Persist one scope's clean-up settings (auto-archive + retention rule).
#[server(input = server_fn::codec::Json)]
pub async fn set_done_scope(scope: DoneScope) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        sqlx::query(
            "INSERT INTO done_scopes \
             (username, origin, scope_id, auto_archive, keep_newest, older_than_days) \
             VALUES (?, ?, ?, ?, ?, ?) \
             ON CONFLICT(username, origin, scope_id) DO UPDATE SET \
               auto_archive = excluded.auto_archive, \
               keep_newest = excluded.keep_newest, \
               older_than_days = excluded.older_than_days",
        )
        .bind(&username)
        .bind(&scope.origin)
        .bind(&scope.scope_id)
        .bind(if scope.auto_archive { 1i64 } else { 0 })
        .bind(scope.keep_newest.map(|v| v as i64))
        .bind(scope.older_than_days.map(|v| v as i64))
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Hard-delete every archived row for the user (the only way archived rows are
/// deleted besides the retention window).
#[server]
pub async fn clear_archived_done_items() -> Result<u32, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        let affected =
            sqlx::query("DELETE FROM done_items WHERE username = ? AND archived_at IS NOT NULL")
                .bind(&username)
                .execute(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
                .rows_affected();
        return Ok(affected as u32);
    }
    #[allow(unreachable_code)]
    Ok(0)
}

// ── Purge + re-open ───────────────────────────────────────────────────────────

/// A live row selected for archiving.
#[cfg(feature = "ssr")]
struct Target {
    id: i64,
    origin: String,
    scope_id: String,
    origin_key: String,
    source_file: Option<String>,
    note_id: Option<i64>,
    calendar: Option<String>,
    completed_at: String,
    source_label: String,
}

/// Select the live rows a `(filter, retention)` pair targets for archiving.
#[cfg(feature = "ssr")]
async fn compute_targets(
    pool: &sqlx::SqlitePool,
    username: &str,
    filter: &DoneFilter,
    retention: &Retention,
) -> Result<Vec<Target>, ServerFnError> {
    use sqlx::Row;

    let rows = sqlx::query(
        "SELECT id, origin, scope_id, origin_key, slug, title, source_file, note_id, calendar, \
                CAST(completed_at AS TEXT) AS completed_at, source_label \
         FROM done_items WHERE username = ? AND archived_at IS NULL",
    )
    .bind(username)
    .fetch_all(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let query_lc = filter.query.as_deref().map(|q| q.to_lowercase());
    let mut targets: Vec<Target> = Vec::new();
    for r in rows {
        let origin: String = r.try_get("origin").unwrap_or_default();
        let scope_id: String = r.try_get("scope_id").unwrap_or_default();
        let slug: String = r.try_get("slug").unwrap_or_default();
        let title: String = r.try_get("title").unwrap_or_default();
        let completed_at: String = r
            .try_get::<Option<String>, _>("completed_at")
            .unwrap_or_default()
            .unwrap_or_default();

        if let Some(o) = &filter.origin {
            if &origin != o {
                continue;
            }
        }
        if let Some(s) = &filter.scope_id {
            if &scope_id != s {
                continue;
            }
        }
        if let Some(since) = &filter.since {
            if &completed_at < since {
                continue;
            }
        }
        if let Some(until) = &filter.until {
            if &completed_at > until {
                continue;
            }
        }
        if let Some(q) = &query_lc {
            if !title.to_lowercase().contains(q) && !slug.to_lowercase().contains(q) {
                continue;
            }
        }

        targets.push(Target {
            id: r.try_get("id").unwrap_or_default(),
            origin,
            scope_id,
            origin_key: r.try_get("origin_key").unwrap_or_default(),
            source_file: r.try_get("source_file").unwrap_or_default(),
            note_id: r.try_get("note_id").unwrap_or_default(),
            calendar: r.try_get("calendar").unwrap_or_default(),
            completed_at,
            source_label: r.try_get("source_label").unwrap_or_default(),
        });
    }

    match retention {
        Retention::None => {}
        Retention::KeepNewest(n) => {
            let mut groups: std::collections::HashMap<(String, String), Vec<usize>> =
                std::collections::HashMap::new();
            for (i, t) in targets.iter().enumerate() {
                groups
                    .entry((t.origin.clone(), t.scope_id.clone()))
                    .or_default()
                    .push(i);
            }
            let mut keep = vec![false; targets.len()];
            for idxs in groups.values() {
                let mut sorted = idxs.clone();
                sorted.sort_by(|a, b| targets[*b].completed_at.cmp(&targets[*a].completed_at));
                for &i in sorted.iter().take(*n as usize) {
                    keep[i] = true;
                }
            }
            let mut it = keep.iter();
            targets.retain(|_| *it.next().unwrap_or(&false));
        }
        Retention::OlderThan(days) => {
            let cutoff = (chrono::Utc::now().date_naive() - chrono::Duration::days(*days as i64))
                .format("%Y-%m-%d")
                .to_string();
            targets.retain(|t| t.completed_at.chars().take(10).collect::<String>() < cutoff);
        }
    }

    Ok(targets)
}

/// What [`purge_done_items`] would archive for the given filter + retention.
#[server(input = server_fn::codec::Json)]
pub async fn preview_done_purge(
    filter: DoneFilter,
    retention: Retention,
) -> Result<PurgePreview, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::DataDir;
        use axum::Extension;
        use leptos_axum::extract;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;
        reconcile_done_items(&pool, &username, &data_dir).await?;
        let targets = compute_targets(&pool, &username, &filter, &retention).await?;

        let mut groups: std::collections::BTreeMap<(String, String, String), u32> =
            std::collections::BTreeMap::new();
        for t in &targets {
            *groups
                .entry((t.origin.clone(), t.scope_id.clone(), t.source_label.clone()))
                .or_default() += 1;
        }
        let groups = groups
            .into_iter()
            .map(
                |((origin, scope_id, label), count)| crate::types::PurgeGroup {
                    origin,
                    scope_id,
                    label,
                    count,
                },
            )
            .collect::<Vec<_>>();
        return Ok(PurgePreview {
            total: targets.len() as u32,
            groups,
        });
    }
    #[allow(unreachable_code)]
    Ok(PurgePreview::default())
}

/// Strip the targeted done tags/tokens/events from their origins and archive
/// the matching index rows. The origin content is hard-deleted; the row is
/// retained as history unless the archive-retention window is `0`. Shared by
/// the [`purge_done_items`] server fn and the background sweep
/// ([`sweep_due_done_items`], backlog #135).
///
/// Caller is responsible for reconciling the index first; this mutates only
/// `done_items` for `username` plus the origins the targets point at.
#[cfg(feature = "ssr")]
async fn purge_scope(
    pool: &sqlx::SqlitePool,
    username: &str,
    data_dir: &std::path::Path,
    git_pusher: &crate::ssr_types::GitCommitPusher,
    md_trigger: &crate::ssr_types::MarkdownSyncTrigger,
    filter: &DoneFilter,
    retention: &Retention,
) -> Result<u32, ServerFnError> {
    let targets = compute_targets(pool, username, filter, retention).await?;
    apply_done_archive(pool, username, data_dir, git_pusher, md_trigger, &targets).await
}

/// Strip the targeted done tags/tokens/events from their origins and archive
/// the matching index rows. The origin content is hard-deleted; the row is
/// retained as history unless the archive-retention window is `0`.
#[server(input = server_fn::codec::Json)]
pub async fn purge_done_items(
    filter: DoneFilter,
    retention: Retention,
) -> Result<PurgeResult, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::{DataDir, GitCommitPusher, MarkdownSyncTrigger};
        use axum::Extension;
        use leptos_axum::extract;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;
        let Extension(git_pusher): Extension<GitCommitPusher> = extract().await?;
        let Extension(md_trigger): Extension<MarkdownSyncTrigger> = extract().await?;
        reconcile_done_items(&pool, &username, &data_dir).await?;

        let archived = purge_scope(
            &pool,
            &username,
            &data_dir,
            &git_pusher,
            &md_trigger,
            &filter,
            &retention,
        )
        .await?;
        return Ok(PurgeResult { archived });
    }
    #[allow(unreachable_code)]
    Ok(PurgeResult::default())
}

/// Apply every due scope's retention rule automatically (backlog #135).
///
/// For each `done_scopes` row carrying an `older_than_days` rule, reconcile its
/// owner's index and strip completed items older than the threshold, archiving
/// them exactly as the manual "Save & apply" / "Clean up all matching" paths do.
/// Driven by a periodic loop in `tidl-server`; returns the number of items
/// archived across all scopes so the caller can log it. A scope whose reconcile
/// fails is skipped rather than risking a purge against a stale index.
#[cfg(feature = "ssr")]
pub async fn sweep_due_done_items(
    pool: &sqlx::SqlitePool,
    data_dir: &std::path::Path,
    git_pusher: &crate::ssr_types::GitCommitPusher,
    md_trigger: &crate::ssr_types::MarkdownSyncTrigger,
) -> Result<u32, ServerFnError> {
    use sqlx::Row;

    let rows = sqlx::query(
        "SELECT username, origin, scope_id, older_than_days FROM done_scopes \
         WHERE older_than_days IS NOT NULL AND older_than_days > 0 \
         ORDER BY username, origin, scope_id",
    )
    .fetch_all(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    // Reconcile each affected user once, before purging any of their scopes.
    let mut reconciled: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut archived = 0u32;
    for r in rows {
        let username: String = r.try_get("username").unwrap_or_default();
        let origin: String = r.try_get("origin").unwrap_or_default();
        let scope_id: String = r.try_get("scope_id").unwrap_or_default();
        let days: i64 = r.try_get("older_than_days").unwrap_or(0);

        if reconciled.insert(username.clone()) {
            if let Err(e) = reconcile_done_items(pool, &username, data_dir).await {
                tracing::warn!(%username, "done sweep: reconcile failed: {e}");
                continue;
            }
        }

        let filter = DoneFilter {
            origin: Some(origin),
            scope_id: Some(scope_id),
            ..Default::default()
        };
        match purge_scope(
            pool,
            &username,
            data_dir,
            git_pusher,
            md_trigger,
            &filter,
            &Retention::OlderThan(days as u32),
        )
        .await
        {
            Ok(n) => archived += n,
            Err(e) => tracing::warn!(%username, "done sweep: purge failed: {e}"),
        }
    }
    Ok(archived)
}

/// Archive a specific set of live done items by id: strip each from its origin
/// and archive its index row. The per-row counterpart of [`purge_done_items`],
/// used by the Done row's Archive action (backlog #133).
#[server(input = server_fn::codec::Json)]
pub async fn purge_done_ids(ids: Vec<i64>) -> Result<PurgeResult, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::{DataDir, GitCommitPusher, MarkdownSyncTrigger};
        use axum::Extension;
        use leptos_axum::extract;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;
        let Extension(git_pusher): Extension<GitCommitPusher> = extract().await?;
        let Extension(md_trigger): Extension<MarkdownSyncTrigger> = extract().await?;

        let targets = targets_by_ids(&pool, &username, &ids).await?;
        let archived = apply_done_archive(
            &pool,
            &username,
            &data_dir,
            &git_pusher,
            &md_trigger,
            &targets,
        )
        .await?;
        return Ok(PurgeResult { archived });
    }
    #[allow(unreachable_code)]
    Ok(PurgeResult::default())
}

/// Load the live done rows with the given ids for a user, as archiving targets.
#[cfg(feature = "ssr")]
async fn targets_by_ids(
    pool: &sqlx::SqlitePool,
    username: &str,
    ids: &[i64],
) -> Result<Vec<Target>, ServerFnError> {
    use sqlx::Row;

    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let placeholders = vec!["?"; ids.len()].join(",");
    let sql = format!(
        "SELECT id, origin, scope_id, origin_key, source_file, note_id, calendar, \
                CAST(completed_at AS TEXT) AS completed_at, source_label \
         FROM done_items WHERE username = ? AND archived_at IS NULL AND id IN ({placeholders})"
    );
    let mut q = sqlx::query(&sql).bind(username);
    for id in ids {
        q = q.bind(id);
    }
    let rows = q
        .fetch_all(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    Ok(rows
        .into_iter()
        .map(|r| Target {
            id: r.try_get("id").unwrap_or_default(),
            origin: r.try_get("origin").unwrap_or_default(),
            scope_id: r.try_get("scope_id").unwrap_or_default(),
            origin_key: r.try_get("origin_key").unwrap_or_default(),
            source_file: r.try_get("source_file").unwrap_or_default(),
            note_id: r.try_get("note_id").unwrap_or_default(),
            calendar: r.try_get("calendar").unwrap_or_default(),
            completed_at: r
                .try_get::<Option<String>, _>("completed_at")
                .unwrap_or_default()
                .unwrap_or_default(),
            source_label: r.try_get("source_label").unwrap_or_default(),
        })
        .collect())
}

/// Strip the targeted done tags/tokens/events from their origins and archive
/// the matching index rows. Shared by the filter+retention bulk purge and the
/// explicit per-item purge; returns the number of rows archived. The origin
/// content is hard-deleted; the row is retained as history unless the user's
/// archive-retention window is `0` (no history), in which case it is pruned.
#[cfg(feature = "ssr")]
async fn apply_done_archive(
    pool: &sqlx::SqlitePool,
    username: &str,
    data_dir: &std::path::Path,
    git_pusher: &crate::ssr_types::GitCommitPusher,
    md_trigger: &crate::ssr_types::MarkdownSyncTrigger,
    targets: &[Target],
) -> Result<u32, ServerFnError> {
    use sqlx::Row;

    let total = targets.len() as u32;
    if targets.is_empty() {
        return Ok(0);
    }

    // markdown: group by (source_id, rel_path), rewrite each file once.
    let mut md_groups: std::collections::HashMap<(i64, String), Vec<(i64, i64)>> =
        std::collections::HashMap::new();
    let mut note_groups: std::collections::HashMap<i64, Vec<(i64, i64)>> =
        std::collections::HashMap::new();
    let mut caldav_groups: std::collections::HashMap<String, Vec<(String, i64)>> =
        std::collections::HashMap::new();

    for t in targets {
        match t.origin.as_str() {
            "markdown" => {
                let Some(u) = crate::types::MarkdownUid::parse(&t.origin_key) else {
                    continue;
                };
                if let (Some(sid), Some(sf)) = (u.source_id_i64(), t.source_file.clone()) {
                    md_groups
                        .entry((sid, sf))
                        .or_default()
                        .push((u.line_u32() as i64, u.ordinal_u32() as i64));
                }
            }
            "note" => {
                if let (Some(nid), Some((_, line, ord))) =
                    (t.note_id, parse_note_key(&t.origin_key))
                {
                    note_groups
                        .entry(nid)
                        .or_default()
                        .push((line as i64, ord as i64));
                }
            }
            "caldav" => {
                if let Some(cal) = t.calendar.clone() {
                    caldav_groups
                        .entry(cal)
                        .or_default()
                        .push((t.origin_key.clone(), t.id));
                }
            }
            _ => {}
        }
    }

    // ── markdown ──
    for ((source_id, rel_path), archives) in md_groups {
        let row =
            sqlx::query("SELECT type, config FROM markdown_sources WHERE id = ? AND username = ?")
                .bind(source_id)
                .bind(username)
                .fetch_optional(pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        let Some(row) = row else { continue };
        let source_type: String = row.try_get("type").unwrap_or_default();
        let config_json: String = row.try_get("config").unwrap_or_default();
        let config: serde_json::Value = serde_json::from_str(&config_json).unwrap_or_default();
        let base_dir =
            super::files::resolve_markdown_source_dir(&source_type, &config, source_id, data_dir)
                .ok_or_else(|| ServerFnError::new("Invalid source config"))?;
        let content = tokio::fs::read_to_string(base_dir.join(&rel_path))
            .await
            .map_err(|e| ServerFnError::new(format!("Cannot read file: {e}")))?;
        let refs: Vec<(usize, usize)> = archives
            .iter()
            .map(|(l, o)| (*l as usize, *o as usize))
            .collect();
        let (new_content, _) = tidl_markdown::remove_done_tags(&content, &refs);
        crate::api::events::markdown_write_content(
            data_dir,
            git_pusher,
            md_trigger,
            &source_type,
            &config,
            source_id,
            &rel_path,
            new_content,
        )
        .await?;
    }

    // ── note ──
    for (note_id, archives) in note_groups {
        let text: Option<String> = sqlx::query_scalar("SELECT text FROM notes WHERE id = ?")
            .bind(note_id)
            .fetch_optional(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        let Some(text) = text else { continue };
        let refs: Vec<(usize, usize)> = archives
            .iter()
            .map(|(l, o)| (*l as usize, *o as usize))
            .collect();
        let new_text = crate::note_tokens::remove_tokens(&text, &refs);
        sqlx::query("UPDATE notes SET text = ?, updated_at = datetime('now') WHERE id = ?")
            .bind(&new_text)
            .bind(note_id)
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }

    // ── caldav ──
    for (cal, uids) in caldav_groups {
        let placeholders = vec!["?"; uids.len()].join(",");
        let sql = format!(
            "DELETE FROM events WHERE username = ? AND calendar = ? AND uid IN ({placeholders})"
        );
        let mut q = sqlx::query(&sql).bind(username).bind(&cal);
        for (uid, _) in &uids {
            q = q.bind(uid);
        }
        q.execute(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        for (uid, _) in &uids {
            crate::api::events::bump_caldav_sync_token(pool, username, &cal, uid, "deleted").await;
        }
    }

    // Archive every target row (never delete).
    for t in targets {
        sqlx::query("UPDATE done_items SET archived_at = datetime('now') WHERE id = ?")
            .bind(t.id)
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }

    // Apply the archive-retention window immediately. Normally the tombstones
    // are pruned lazily on the next `list_done_items`, but a window of `0`
    // keeps no history, so the rows must be gone as soon as they're archived.
    prune_archived(pool, username).await?;

    Ok(total)
}

/// Re-open a live done item: restore its origin record (the `#done` tag/token
/// rewritten back to the active form, or the VTODO back to `NEEDS-ACTION`) and
/// delete its index row. Archived rows cannot be re-opened.
#[server]
pub async fn reopen_done_item(id: i64) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::{DataDir, GitCommitPusher, MarkdownSyncTrigger};
        use axum::Extension;
        use leptos_axum::extract;
        use sqlx::Row;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;
        let Extension(git_pusher): Extension<GitCommitPusher> = extract().await?;
        let Extension(md_trigger): Extension<MarkdownSyncTrigger> = extract().await?;

        let row = sqlx::query(
            "SELECT origin, origin_key, calendar FROM done_items \
             WHERE id = ? AND username = ? AND archived_at IS NULL",
        )
        .bind(id)
        .bind(&username)
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("Item is not a live done item"))?;

        let origin: String = row.try_get("origin").unwrap_or_default();
        let origin_key: String = row.try_get("origin_key").unwrap_or_default();

        match origin.as_str() {
            "markdown" => {
                let u = crate::types::MarkdownUid::parse(&origin_key)
                    .ok_or_else(|| ServerFnError::new("Bad markdown key"))?;
                let source_id = u
                    .source_id_i64()
                    .ok_or_else(|| ServerFnError::new("Bad markdown source"))?;
                let rel_path = u.rel_path.to_string();
                let line_idx = (u.line_u32() as usize).saturating_sub(1);
                let ordinal = u.ordinal_u32() as usize;
                let src = sqlx::query(
                    "SELECT type, config FROM markdown_sources WHERE id = ? AND username = ?",
                )
                .bind(source_id)
                .bind(&username)
                .fetch_optional(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
                .ok_or_else(|| ServerFnError::new("Markdown source not found"))?;
                let source_type: String = src.try_get("type").unwrap_or_default();
                let config_json: String = src.try_get("config").unwrap_or_default();
                let config: serde_json::Value =
                    serde_json::from_str(&config_json).unwrap_or_default();
                crate::api::events::markdown_line_transform(
                    &data_dir,
                    &git_pusher,
                    &md_trigger,
                    &source_type,
                    &config,
                    source_id,
                    &rel_path,
                    line_idx,
                    |l| tidl_markdown::reopen_tag_at(l, ordinal).unwrap_or_else(|| l.to_string()),
                )
                .await?;
            }
            "note" => {
                let (note_id, line, ordinal) = parse_note_key(&origin_key)
                    .ok_or_else(|| ServerFnError::new("Bad note key"))?;
                let text: Option<String> =
                    sqlx::query_scalar("SELECT text FROM notes WHERE id = ?")
                        .bind(note_id)
                        .fetch_optional(&pool)
                        .await
                        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
                let text = text.ok_or_else(|| ServerFnError::new("Note not found"))?;
                if let Some(new_text) = crate::note_tokens::reopen_token_at(&text, line, ordinal) {
                    sqlx::query(
                        "UPDATE notes SET text = ?, updated_at = datetime('now') WHERE id = ?",
                    )
                    .bind(&new_text)
                    .bind(note_id)
                    .execute(&pool)
                    .await
                    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
                }
            }
            "caldav" => {
                let calendar: Option<String> = row.try_get("calendar").unwrap_or_default();
                sqlx::query(
                    "UPDATE events SET status = 'NEEDS-ACTION', completed_date = NULL \
                     WHERE uid = ? AND username = ? AND type = 'VTODO'",
                )
                .bind(&origin_key)
                .bind(&username)
                .execute(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
                if let Some(cal) = calendar {
                    crate::api::events::bump_caldav_sync_token(
                        &pool,
                        &username,
                        &cal,
                        &origin_key,
                        "modified",
                    )
                    .await;
                }
            }
            _ => {}
        }

        forget_done_item(&pool, &username, &origin, &origin_key)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Parse `note:{note_id}:{line}:{ordinal}` into `(note_id, 1-based line, 0-based
/// ordinal)`.
#[cfg(feature = "ssr")]
fn parse_note_key(key: &str) -> Option<(i64, usize, usize)> {
    let rest = key.strip_prefix("note:")?;
    let mut parts = rest.split(':');
    let note_id: i64 = parts.next()?.parse().ok()?;
    let line: usize = parts.next()?.parse().ok()?;
    let ordinal: usize = parts.next()?.parse().ok()?;
    Some((note_id, line, ordinal))
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    fn scanned() -> Scanned {
        Scanned {
            origin: "note",
            scope_id: "1".into(),
            origin_key: "note:1:1:0".into(),
            kind: "task".into(),
            slug: "foo".into(),
            title: "Foo".into(),
            priority: Some(5),
            due_date: None,
            completed_at: "2026-10-08".into(),
            source_label: "Col".into(),
            source_file: None,
            note_id: Some(1),
            calendar: None,
        }
    }

    /// A repeated reconcile of the same origin must not create a second live
    /// row — the exact duplicate the Done screen showed before the upsert was
    /// made atomic (two parallel resources both reconcile on load).
    #[sqlx::test(migrations = "../../migrations")]
    async fn upsert_live_is_idempotent(pool: SqlitePool) {
        upsert_live(&pool, "alice", &scanned()).await.unwrap();
        upsert_live(&pool, "alice", &scanned()).await.unwrap();
        let live: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM done_items WHERE username = 'alice' AND archived_at IS NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(live, 1);
        let gens: Vec<i64> =
            sqlx::query_scalar("SELECT generation FROM done_items WHERE username = 'alice'")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(gens, vec![0]);
    }

    fn noop_pusher() -> crate::ssr_types::GitCommitPusher {
        crate::ssr_types::GitCommitPusher(std::sync::Arc::new(|_id, _paths| {
            Box::pin(async { Ok::<(), String>(()) })
        }))
    }

    fn noop_trigger() -> crate::ssr_types::MarkdownSyncTrigger {
        let (tx, _rx) = tokio::sync::mpsc::unbounded_channel::<i64>();
        crate::ssr_types::MarkdownSyncTrigger(tx)
    }

    /// The automatic sweep (backlog #135) reconciles the index, then strips +
    /// archives every completed note token older than its scope's retention age.
    #[sqlx::test(migrations = "../../migrations")]
    async fn sweep_applies_older_than_rule(pool: SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) \
             VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) \
             VALUES ('alice', 'Notes', '#000000') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO notes (author_username, collection_id, text) \
             VALUES ('alice', ?, '#done/2020-01-01/p50/old-thing')",
        )
        .bind(collection_id)
        .execute(&pool)
        .await
        .unwrap();
        // A 1-day retention rule on the collection.
        sqlx::query(
            "INSERT INTO done_scopes (username, origin, scope_id, older_than_days) \
             VALUES ('alice', 'note', ?, 1)",
        )
        .bind(collection_id.to_string())
        .execute(&pool)
        .await
        .unwrap();

        let archived = sweep_due_done_items(
            &pool,
            std::path::Path::new("."),
            &noop_pusher(),
            &noop_trigger(),
        )
        .await
        .unwrap();
        assert_eq!(archived, 1, "the stale token is swept exactly once");

        let text: String = sqlx::query_scalar("SELECT text FROM notes WHERE collection_id = ?")
            .bind(collection_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(
            !text.contains("#done"),
            "token stripped from the origin: {text:?}"
        );

        let live: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM done_items WHERE username = 'alice' AND archived_at IS NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let archived: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM done_items WHERE username = 'alice' AND archived_at IS NOT NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(live, 0);
        assert_eq!(archived, 1, "row retained as archived history");
    }

    /// A sweep with no retention rules configured does nothing (and reconciles
    /// no one).
    #[sqlx::test(migrations = "../../migrations")]
    async fn sweep_is_noop_without_rules(pool: SqlitePool) {
        let archived = sweep_due_done_items(
            &pool,
            std::path::Path::new("."),
            &noop_pusher(),
            &noop_trigger(),
        )
        .await
        .unwrap();
        assert_eq!(archived, 0);
    }

    /// Re-completing after the row was archived takes the next generation and
    /// leaves the tombstone in place.
    #[sqlx::test(migrations = "../../migrations")]
    async fn upsert_live_after_archive_bumps_generation(pool: SqlitePool) {
        upsert_live(&pool, "alice", &scanned()).await.unwrap();
        sqlx::query("UPDATE done_items SET archived_at = datetime('now') WHERE username = 'alice'")
            .execute(&pool)
            .await
            .unwrap();
        upsert_live(&pool, "alice", &scanned()).await.unwrap();

        let rows: Vec<(i64, Option<String>)> = sqlx::query_as(
            "SELECT generation, archived_at FROM done_items WHERE username = 'alice' \
             ORDER BY generation",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, 0);
        assert!(rows[0].1.is_some(), "archived row keeps its tombstone");
        assert_eq!(rows[1].0, 1);
        assert!(rows[1].1.is_none(), "new live row");
    }

    /// `targets_by_ids` returns only the caller's *live* rows, ignoring
    /// archived ids, other users' ids and unknown ids (backlog #133).
    #[sqlx::test(migrations = "../../migrations")]
    async fn targets_by_ids_only_returns_live_own_rows(pool: SqlitePool) {
        upsert_live(&pool, "alice", &scanned()).await.unwrap();
        let mut archived = scanned();
        archived.origin_key = "note:1:2:0".into();
        upsert_live(&pool, "alice", &archived).await.unwrap();
        sqlx::query(
            "UPDATE done_items SET archived_at = datetime('now') WHERE origin_key = 'note:1:2:0'",
        )
        .execute(&pool)
        .await
        .unwrap();

        let live_id: i64 =
            sqlx::query_scalar("SELECT id FROM done_items WHERE origin_key = 'note:1:1:0'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let archived_id: i64 =
            sqlx::query_scalar("SELECT id FROM done_items WHERE origin_key = 'note:1:2:0'")
                .fetch_one(&pool)
                .await
                .unwrap();

        let targets = targets_by_ids(&pool, "alice", &[live_id, archived_id, 9999])
            .await
            .unwrap();
        assert_eq!(targets.len(), 1);
        assert_eq!(targets[0].id, live_id);

        upsert_live(&pool, "bob", &scanned()).await.unwrap();
        let bob_id: i64 = sqlx::query_scalar("SELECT id FROM done_items WHERE username = 'bob'")
            .fetch_one(&pool)
            .await
            .unwrap();
        let none = targets_by_ids(&pool, "alice", &[bob_id]).await.unwrap();
        assert!(none.is_empty(), "another user's row is never a target");
    }

    /// Insert a minimal VTODO `events` row for the CalDAV hook tests.
    async fn seed_vtodo(pool: &SqlitePool, uid: &str, status: Option<&str>) {
        sqlx::query(
            "INSERT INTO events \
             (username, calendar, uid, type, summary, status, priority, topic, completed_date, \
              etag, ical_data) \
             VALUES ('alice', 'personal', ?, 'VTODO', 'Buy milk', ?, 5, 'buy-milk', \
                     '2026-10-09', 'e1', '')",
        )
        .bind(uid)
        .bind(status)
        .execute(pool)
        .await
        .unwrap();
    }

    /// An external completion is indexed immediately, with the same fields
    /// `reconcile_caldav` would have produced (backlog #136).
    #[sqlx::test(migrations = "../../migrations")]
    async fn upsert_caldav_done_item_indexes_a_completed_vtodo(pool: SqlitePool) {
        seed_vtodo(&pool, "t1", Some("COMPLETED")).await;
        upsert_caldav_done_item(&pool, "alice", "personal", "t1")
            .await
            .unwrap();

        let (origin, scope, key, slug, title, kind, completed): (
            String,
            String,
            String,
            String,
            String,
            String,
            String,
        ) = sqlx::query_as(
            "SELECT origin, scope_id, origin_key, slug, title, kind, \
                    CAST(completed_at AS TEXT) FROM done_items WHERE username = 'alice'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(origin, "caldav");
        assert_eq!(scope, "personal");
        assert_eq!(key, "t1");
        assert_eq!(slug, "buy-milk");
        assert_eq!(title, "Buy milk");
        assert_eq!(kind, "task");
        assert_eq!(completed, "2026-10-09");
    }

    /// Only a completed VTODO is indexed — a missing or un-completed event is a
    /// no-op (the note-mirror completion path deletes its row before this runs).
    #[sqlx::test(migrations = "../../migrations")]
    async fn upsert_caldav_done_item_is_a_no_op_without_a_completed_vtodo(pool: SqlitePool) {
        seed_vtodo(&pool, "open", Some("NEEDS-ACTION")).await;
        upsert_caldav_done_item(&pool, "alice", "personal", "open")
            .await
            .unwrap();
        upsert_caldav_done_item(&pool, "alice", "personal", "missing")
            .await
            .unwrap();

        let live: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM done_items WHERE username = 'alice' AND archived_at IS NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(live, 0);
    }

    /// An external delete/un-completion archives the live row (keeping history)
    /// rather than waiting for the next reconcile, and is idempotent.
    #[sqlx::test(migrations = "../../migrations")]
    async fn archive_caldav_done_item_archives_the_live_row(pool: SqlitePool) {
        seed_vtodo(&pool, "t1", Some("COMPLETED")).await;
        upsert_caldav_done_item(&pool, "alice", "personal", "t1")
            .await
            .unwrap();

        archive_caldav_done_item(&pool, "alice", "t1")
            .await
            .unwrap();
        archive_caldav_done_item(&pool, "alice", "t1")
            .await
            .unwrap();

        let (archived, count): (Option<String>, i64) = sqlx::query_as(
            "SELECT CAST(archived_at AS TEXT), COUNT(*) FROM done_items WHERE origin = 'caldav'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(archived.is_some(), "live row is archived, not deleted");
        assert_eq!(count, 1);
    }

    /// The archive-retention window `0` keeps no history: archived rows are
    /// deleted, whereas blank (NULL) keeps them indefinitely.
    #[sqlx::test(migrations = "../../migrations")]
    async fn prune_archived_zero_window_deletes_history(pool: SqlitePool) {
        upsert_live(&pool, "alice", &scanned()).await.unwrap();
        sqlx::query("UPDATE done_items SET archived_at = datetime('now') WHERE username = 'alice'")
            .execute(&pool)
            .await
            .unwrap();

        // No setting → blank → history kept.
        prune_archived(&pool, "alice").await.unwrap();
        let archived: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM done_items WHERE username = 'alice' AND archived_at IS NOT NULL",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(archived, 1, "blank keeps history forever");

        // Window 0 → no history.
        sqlx::query(
            "INSERT INTO done_settings (username, archive_retention_days) VALUES ('alice', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        prune_archived(&pool, "alice").await.unwrap();
        let rows: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM done_items WHERE username = 'alice'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(rows, 0, "a zero window deletes archived rows");
    }
}
