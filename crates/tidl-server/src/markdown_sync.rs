//! Background polling loop that syncs markdown sources into the `events` table.
//!
//! Every `TICK_SECS` it:
//! 1. Loads all enabled `markdown_sources` rows from the DB.
//! 2. Syncs those whose per-source `sync_interval_secs` has elapsed since their
//!    last sync: pulls/clones git repos (if applicable), then walks for `.md` files.
//! 3. Uses SHA-256 hashes (stored in `sync_state`) for change detection.
//! 4. On change: deletes stale events and inserts fresh VTODOs for the file.
//! 5. Broadcasts an `"events:<username>"` message on the `change_tx` channel so connected
//!    browsers refetch their todo list.
//!
//! Markdown todos are stored in the `events` table with `calendar = '__markdown__'`
//! and `source = 'markdown'`.  No backing row in `calendars` is required — the FK
//! was removed in migration 20240119 so this synthetic label never appears in the UI.

use std::{
    collections::{HashMap, HashSet},
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};

use anyhow::Result;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use tokio::sync::{broadcast, mpsc};
use tracing::{debug, error, info, warn};
use uuid::Uuid;

use tidl_caldav::change::ChangeKind;
use tidl_markdown::parser::MarkdownParser;

/// How often the loop wakes to check which sources are due for a sync.
/// Sources can't sync faster than this regardless of their configured
/// `sync_interval_secs` — kept in sync with `tidl_web::types::MIN_SYNC_INTERVAL_SECS`.
const TICK_SECS: u64 = 10;

// ── Entry point ───────────────────────────────────────────────────────────────

pub async fn run_markdown_sync_loop(
    pool: SqlitePool,
    change_tx: broadcast::Sender<String>,
    mut trigger_rx: mpsc::UnboundedReceiver<i64>,
    data_dir: PathBuf,
    enc_key: Option<Arc<Vec<u8>>>,
) {
    // Tracks when each source (by id) should next be synced.
    // Absent entry ⇒ never synced ⇒ sync immediately on first check.
    let mut next_sync_at: HashMap<i64, Instant> = HashMap::new();

    // Run an initial sync immediately on startup so todos appear without
    // waiting for the first tick to elapse.
    sync_due(
        &pool,
        &change_tx,
        &data_dir,
        enc_key.as_deref().map(|k| k.as_slice()),
        &mut next_sync_at,
    )
    .await;

    loop {
        let tick = tokio::time::sleep(Duration::from_secs(TICK_SECS));
        tokio::pin!(tick);
        tokio::select! {
            _ = &mut tick => {
                sync_due(&pool, &change_tx, &data_dir, enc_key.as_deref().map(|k| k.as_slice()), &mut next_sync_at).await;
            }
            Some(source_id) = trigger_rx.recv() => {
                sync_single(&pool, &change_tx, source_id, &data_dir, enc_key.as_deref().map(|k| k.as_slice())).await;
                // Reset the timer so the next scheduled sync runs at the correct
                // interval from now, rather than treating it as never-synced.
                if let Ok(sources) = load_sources(&pool, &data_dir).await {
                    if let Some(s) = sources.iter().find(|s| s.id == source_id) {
                        let interval = Duration::from_secs(s.sync_interval_secs as u64);
                        next_sync_at.insert(source_id, Instant::now() + interval);
                    }
                }
            }
        }
    }
}

async fn sync_single(
    pool: &SqlitePool,
    change_tx: &broadcast::Sender<String>,
    source_id: i64,
    data_dir: &Path,
    enc_key: Option<&[u8]>,
) {
    let sources = match load_sources(pool, data_dir).await {
        Ok(s) => s,
        Err(e) => {
            error!("markdown_sync: failed to load sources: {e}");
            return;
        }
    };
    if let Some(source) = sources.into_iter().find(|s| s.id == source_id) {
        match sync_source(pool, &source, enc_key).await {
            Ok(changed) => {
                if changed {
                    let _ = change_tx.send(ChangeKind::Events.to_msg(&source.username));
                }
                info!(source_id, "markdown_sync: manual sync complete");
            }
            Err(e) => {
                warn!(
                    source_id,
                    username = %source.username,
                    "markdown_sync: manual sync failed: {e}"
                );
            }
        }
    } else {
        warn!(
            source_id,
            "markdown_sync: source not found for manual trigger"
        );
    }
}

// ── Core sync logic ───────────────────────────────────────────────────────────

/// Syncs every source whose `sync_interval_secs` has elapsed since it last
/// ran, and reschedules each one for its next check.
async fn sync_due(
    pool: &SqlitePool,
    change_tx: &broadcast::Sender<String>,
    data_dir: &Path,
    enc_key: Option<&[u8]>,
    next_sync_at: &mut HashMap<i64, Instant>,
) {
    let sources = match load_sources(pool, data_dir).await {
        Ok(s) => s,
        Err(e) => {
            error!("markdown_sync: failed to load sources: {e}");
            return;
        }
    };

    debug!(
        count = sources.len(),
        "markdown_sync: tick — checking sources"
    );

    for source in sources {
        let due = next_sync_at
            .get(&source.id)
            .is_none_or(|t| Instant::now() >= *t);
        if !due {
            continue;
        }

        let id = source.id;
        let interval = Duration::from_secs(source.sync_interval_secs as u64);
        match sync_source(pool, &source, enc_key).await {
            Ok(changed) => {
                if changed {
                    let _ = change_tx.send(ChangeKind::Events.to_msg(&source.username));
                }
            }
            Err(e) => {
                warn!(
                    source_id = source.id,
                    username = %source.username,
                    "markdown_sync: source failed: {e}"
                );
            }
        }
        next_sync_at.insert(id, Instant::now() + interval);
    }
}

// ── Data types ────────────────────────────────────────────────────────────────

struct SourceRow {
    id: i64,
    username: String,
    source_type: String, // "filesystem" or "git"
    base_path: PathBuf,  // for filesystem: from config; for git: data_dir/repos/<id>/
    recursive: bool,
    // git-only fields (from config JSON):
    url: Option<String>,
    branch: Option<String>,
    pat_file: Option<String>,
    pat_encrypted: Option<String>, // from DB column
    sync_interval_secs: u32,
}

// ── DB helpers ────────────────────────────────────────────────────────────────

async fn load_sources(pool: &SqlitePool, data_dir: &Path) -> Result<Vec<SourceRow>> {
    let rows = sqlx::query(
        "SELECT id, username, type, config, pat_encrypted, sync_interval_secs
         FROM markdown_sources WHERE enabled = 1",
    )
    .fetch_all(pool)
    .await?;

    let mut out = Vec::new();
    for r in rows {
        let id: i64 = r.try_get("id")?;
        let username: String = r.try_get("username")?;
        let source_type: String = r.try_get("type")?;
        let config: String = r.try_get("config")?;
        let pat_encrypted: Option<String> = r.try_get("pat_encrypted")?;
        let sync_interval_secs: i64 = r.try_get("sync_interval_secs")?;

        let v: serde_json::Value = serde_json::from_str(&config).unwrap_or_default();

        let (base_path, url, branch, pat_file, recursive) = if source_type == "git" {
            let url = v.get("url").and_then(|x| x.as_str()).map(str::to_string);
            let branch = v
                .get("branch")
                .and_then(|x| x.as_str())
                .unwrap_or("main")
                .to_string();
            let pat_file = v
                .get("pat_file")
                .and_then(|x| x.as_str())
                .map(str::to_string);
            let local_path = data_dir.join("repos").join(id.to_string());
            (local_path, url, Some(branch), pat_file, true)
        } else {
            let path_str = v
                .get("path")
                .and_then(|p| p.as_str())
                .unwrap_or("")
                .to_string();
            if path_str.is_empty() {
                warn!(
                    source_id = id,
                    "markdown_sync: filesystem source has no path, skipping"
                );
                continue;
            }
            let recursive = v.get("recursive").and_then(|r| r.as_bool()).unwrap_or(true);
            (PathBuf::from(path_str), None, None, None, recursive)
        };

        out.push(SourceRow {
            id,
            username,
            source_type,
            base_path,
            recursive,
            url,
            branch,
            pat_file,
            pat_encrypted,
            sync_interval_secs: sync_interval_secs as u32,
        });
    }
    Ok(out)
}

// ── Per-source sync ───────────────────────────────────────────────────────────

/// Returns `true` if any events were inserted/deleted (caller should broadcast).
async fn sync_source(
    pool: &SqlitePool,
    source: &SourceRow,
    enc_key: Option<&[u8]>,
) -> Result<bool> {
    let parser = MarkdownParser::new();
    let mut changed = false;
    let mut sync_error: Option<String> = None;

    // For git sources: pull (or clone) before the file walk.
    if source.source_type == "git" {
        let url = match &source.url {
            Some(u) => u.clone(),
            None => {
                let msg = "git source has no url configured";
                warn!(source_id = source.id, "{msg}");
                update_source_status(pool, source.id, Some(msg)).await?;
                return Ok(false);
            }
        };
        let branch = source.branch.clone().unwrap_or_else(|| "main".to_string());
        let git_cfg = crate::git_sync::GitSourceConfig {
            url: url.clone(),
            branch: branch.clone(),
            pat_file: source.pat_file.clone(),
            pat_encrypted: source.pat_encrypted.clone(),
            enc_key: enc_key.map(|k| k.to_vec()),
        };
        match git_cfg.resolve_pat() {
            Err(e) => {
                let msg = format!("git credentials unavailable: {e}");
                warn!(source_id = source.id, "{msg}");
                update_source_status(pool, source.id, Some(&msg)).await?;
                return Ok(false);
            }
            Ok(pat) => {
                let local_path = source.base_path.clone();
                let exists = local_path.exists();
                let result = tokio::task::spawn_blocking(move || {
                    if exists {
                        crate::git_sync::pull_repo(&local_path, &branch, &pat)
                    } else {
                        if let Err(e) = std::fs::create_dir_all(&local_path) {
                            return Err(crate::git_sync::GitError::PatFileRead(format!(
                                "create_dir_all: {e}"
                            )));
                        }
                        let clone_result =
                            crate::git_sync::clone_repo(&url, &local_path, &branch, &pat);
                        if clone_result.is_err() {
                            // Clean up empty dir on clone failure so the next attempt gets a clean slate.
                            let _ = std::fs::remove_dir_all(&local_path);
                        }
                        clone_result
                    }
                })
                .await;
                match result {
                    Ok(Ok(())) => {}
                    Ok(Err(e)) => {
                        warn!(source_id = source.id, "git pull/clone failed: {e}");
                        sync_error = Some(format!("git pull/clone failed: {e}"));
                        if !source.base_path.exists() {
                            update_source_status(pool, source.id, sync_error.as_deref()).await?;
                            return Ok(false); // can't walk if clone failed
                        }
                        // Fall through to walk existing (possibly stale) files.
                    }
                    Err(e) => {
                        let msg = format!("git task panicked: {e}");
                        warn!(source_id = source.id, "{msg}");
                        update_source_status(pool, source.id, Some(&msg)).await?;
                        return Ok(false);
                    }
                }
            }
        }
    }

    // Bail out if the base path doesn't exist rather than proceeding to the
    // stale-entry cleanup, which would delete all events when the directory is
    // temporarily unavailable (e.g. unmounted drive, changed path).
    if !source.base_path.exists() {
        let msg = format!(
            "markdown source path does not exist: {}",
            source.base_path.display()
        );
        warn!(
            source_id = source.id,
            path = %source.base_path.display(),
            "markdown_sync: base path does not exist, skipping source"
        );
        update_source_status(pool, source.id, Some(&msg)).await?;
        return Ok(false);
    }

    // Walk the configured directory for .md files.
    let files = collect_markdown_files(&source.base_path, source.recursive);
    let mut seen: HashSet<String> = HashSet::new();

    for abs_path in &files {
        let rel_path = abs_path
            .strip_prefix(&source.base_path)
            .unwrap_or(abs_path.as_path())
            .to_string_lossy()
            .to_string();

        seen.insert(rel_path.clone());

        let content = match tokio::fs::read_to_string(abs_path).await {
            Ok(c) => c,
            Err(e) => {
                warn!("markdown_sync: cannot read {}: {e}", abs_path.display());
                continue;
            }
        };

        let hash = sha256_hex(&content);

        // Check whether file has changed since last sync.
        let stored_hash: Option<String> = sqlx::query_scalar(
            "SELECT file_hash FROM sync_state WHERE source_id = ? AND file_path = ?",
        )
        .bind(source.id)
        .bind(&rel_path)
        .fetch_optional(pool)
        .await?;

        if stored_hash.as_deref() == Some(&hash) {
            continue; // unchanged
        }

        info!(
            source_id = source.id,
            path = %rel_path,
            "markdown_sync: processing changed file"
        );

        // Remove stale events for this file. `source_file` is the source's own
        // relative path, so it is NOT unique across sources — a second source
        // sharing the same relative path would otherwise have its events wiped
        // by this one's sync. Scope by the `md:{source_id}:` UID prefix too.
        sqlx::query(
            "DELETE FROM events
             WHERE username = ? AND source = 'markdown' AND source_file = ? AND uid LIKE ?",
        )
        .bind(&source.username)
        .bind(&rel_path)
        .bind(format!("md:{}:%", source.id))
        .execute(pool)
        .await?;

        // Parse and insert fresh items.
        let (tasks, awaits) = parser.parse_all(&content);

        for item in &tasks {
            let uid = format!(
                "md:{}:{}:{}:{}",
                source.id, rel_path, item.line, item.ordinal
            );
            sqlx::query(
                "INSERT OR REPLACE INTO events
                 (username, calendar, uid, type, summary, priority, due_date, status,
                  source, source_file, source_line, topic, etag, ical_data, kind)
                 VALUES (?, '__markdown__', ?, 'VTODO', ?, ?, ?, 'NEEDS-ACTION',
                         'markdown', ?, ?, ?, ?, '', 'task')",
            )
            .bind(&source.username)
            .bind(&uid)
            .bind(&item.title)
            .bind(item.priority.map(|p| p as i64))
            .bind(item.due_date.map(|d| d.format("%Y-%m-%d").to_string()))
            .bind(&rel_path)
            .bind(item.line as i64)
            .bind(&item.subject)
            .bind(format!("md-{}", Uuid::new_v4().simple()))
            .execute(pool)
            .await?;
        }

        for item in &awaits {
            let uid = format!(
                "md:{}:{}:{}:{}",
                source.id, rel_path, item.line, item.ordinal
            );
            sqlx::query(
                "INSERT OR REPLACE INTO events
                 (username, calendar, uid, type, summary, due_date, status,
                  source, source_file, source_line, topic, etag, ical_data, kind)
                 VALUES (?, '__markdown__', ?, 'VTODO', ?, date(?), 'NEEDS-ACTION',
                         'markdown', ?, ?, ?, ?, '', 'await')",
            )
            .bind(&source.username)
            .bind(&uid)
            .bind(&item.title)
            .bind(item.date.format("%Y-%m-%d").to_string())
            .bind(&rel_path)
            .bind(item.line as i64)
            .bind(&item.subject)
            .bind(format!("md-{}", Uuid::new_v4().simple()))
            .execute(pool)
            .await?;
        }

        // Upsert sync_state for this file.
        sqlx::query(
            "INSERT INTO sync_state (source_id, file_path, file_hash, last_synced)
             VALUES (?, ?, ?, datetime('now'))
             ON CONFLICT(source_id, file_path)
             DO UPDATE SET file_hash = excluded.file_hash, last_synced = excluded.last_synced",
        )
        .bind(source.id)
        .bind(&rel_path)
        .bind(&hash)
        .execute(pool)
        .await?;

        changed = true;
    }

    // Remove events for files that no longer exist.
    let known_paths: Vec<String> =
        sqlx::query_scalar("SELECT file_path FROM sync_state WHERE source_id = ?")
            .bind(source.id)
            .fetch_all(pool)
            .await?;

    for old_path in known_paths {
        if !seen.contains(&old_path) {
            info!(
                source_id = source.id,
                path = %old_path,
                "markdown_sync: removing events for deleted file"
            );
            sqlx::query(
                "DELETE FROM events
                 WHERE username = ? AND source = 'markdown' AND source_file = ? AND uid LIKE ?",
            )
            .bind(&source.username)
            .bind(&old_path)
            .bind(format!("md:{}:%", source.id))
            .execute(pool)
            .await?;

            sqlx::query("DELETE FROM sync_state WHERE source_id = ? AND file_path = ?")
                .bind(source.id)
                .bind(&old_path)
                .execute(pool)
                .await?;

            changed = true;
        }
    }

    update_source_status(pool, source.id, sync_error.as_deref()).await?;

    Ok(changed)
}

/// Record the source's last scan time and, when the sync failed, the error to
/// show in Settings/preview (cleared on a successful scan).
async fn update_source_status(
    pool: &SqlitePool,
    source_id: i64,
    error: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "UPDATE markdown_sources
         SET last_scan = strftime('%Y-%m-%d %H:%M:%f', 'now'), last_error = ?
         WHERE id = ?",
    )
    .bind(error)
    .bind(source_id)
    .execute(pool)
    .await?;
    Ok(())
}

// ── Utilities ─────────────────────────────────────────────────────────────────

fn sha256_hex(content: &str) -> String {
    let mut h = Sha256::new();
    h.update(content.as_bytes());
    hex::encode(h.finalize())
}

/// Collect all `.md` / `.markdown` files under `base`.
fn collect_markdown_files(base: &Path, recursive: bool) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let walker = if recursive {
        walkdir::WalkDir::new(base)
    } else {
        walkdir::WalkDir::new(base).max_depth(1)
    };
    for entry in walker.into_iter().filter_map(|e| e.ok()) {
        let p = entry.path();
        if p.is_file() {
            if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                if ext == "md" || ext == "markdown" {
                    out.push(p.to_path_buf());
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(std::path::PathBuf);
    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn temp_dir(label: &str) -> TempDir {
        TempDir(std::env::temp_dir().join(format!(
            "tidl_markdown_sync_test_{label}_{}_{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        )))
    }

    fn source_row(id: i64, base_path: &Path) -> SourceRow {
        SourceRow {
            id,
            username: "alice".to_string(),
            source_type: "filesystem".to_string(),
            base_path: base_path.to_path_buf(),
            recursive: true,
            url: None,
            branch: None,
            pat_file: None,
            pat_encrypted: None,
            sync_interval_secs: 60,
        }
    }

    async fn seed_user_and_source(pool: &SqlitePool, id: i64, name: &str, base_path: &Path) {
        sqlx::query(
            "INSERT OR IGNORE INTO users (username, email, display_name, password_hash, is_admin) \
             VALUES ('alice', 'alice@example.com', 'Alice', 'x', 0)",
        )
        .execute(pool)
        .await
        .unwrap();

        let config = serde_json::json!({
            "path": base_path.to_string_lossy(),
            "recursive": true,
        })
        .to_string();
        sqlx::query(
            "INSERT INTO markdown_sources
             (id, username, name, type, config, enabled, sync_interval_secs)
             VALUES (?, 'alice', ?, 'filesystem', ?, 1, 60)",
        )
        .bind(id)
        .bind(name)
        .bind(config)
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn sync_records_and_clears_source_errors(pool: SqlitePool) {
        let dir = temp_dir("errors");
        let missing = dir.0.join("not-created-yet");
        seed_user_and_source(&pool, 1, "notes", &missing).await;
        let source = source_row(1, &missing);

        sync_source(&pool, &source, None).await.unwrap();
        let error: Option<String> =
            sqlx::query_scalar("SELECT last_error FROM markdown_sources WHERE id = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(
            error.is_some(),
            "missing path should be recorded as an error"
        );

        std::fs::create_dir_all(&missing).unwrap();
        sync_source(&pool, &source, None).await.unwrap();
        let error: Option<String> =
            sqlx::query_scalar("SELECT last_error FROM markdown_sources WHERE id = 1")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert!(error.is_none(), "successful sync should clear the error");
    }

    /// Regression test: two sources may hold files with the *same relative
    /// path* (e.g. each repo has its own `shared.md`). Syncing one must not
    /// delete the other's events for that path — the delete is scoped by the
    /// `md:{source_id}:` UID prefix, not just `source_file`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn sync_does_not_delete_another_sources_events_for_same_rel_path(pool: SqlitePool) {
        let dir_a = temp_dir("a");
        let dir_b = temp_dir("b");
        std::fs::create_dir_all(&dir_a.0).unwrap();
        std::fs::create_dir_all(&dir_b.0).unwrap();

        std::fs::write(
            dir_a.0.join("shared.md"),
            "# A\n\n- [ ] Shared Alpha #task/p1/shared-alpha\n",
        )
        .unwrap();
        // Source B's copy of the same relative path has no task at all.
        std::fs::write(dir_b.0.join("shared.md"), "# B\n\nnothing here\n").unwrap();

        seed_user_and_source(&pool, 1, "a", &dir_a.0).await;
        seed_user_and_source(&pool, 2, "b", &dir_b.0).await;

        let a = source_row(1, &dir_a.0);
        let b = source_row(2, &dir_b.0);

        assert!(sync_source(&pool, &a, None).await.unwrap());
        let before: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE uid LIKE 'md:1:%'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(before, 1, "source A's task should have been synced");

        assert!(sync_source(&pool, &b, None).await.unwrap());

        let after_a: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE uid LIKE 'md:1:%'")
                .fetch_one(&pool)
                .await
                .unwrap();
        let after_b: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE uid LIKE 'md:2:%'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            after_a, 1,
            "syncing source B must not delete source A's events for the shared path"
        );
        assert_eq!(after_b, 0, "source B has no tasks, so it inserts none");
    }
}
