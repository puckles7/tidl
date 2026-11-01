//! Permanent per-user Focus item suppression (backlog #69).
//!
//! `item_key` is either a bare item id (== `events.uid`, since tasks,
//! awaits, and markdown-origin items are all rows in the `events` table —
//! see `tidl-server/src/markdown_sync.rs`) or, for a single suppressed
//! occurrence of a recurring event, `"{uid}|{occurrence_start}"`.

use leptos::prelude::*;

#[cfg(feature = "ssr")]
use super::auth;

/// List this user's suppressed item keys, first deleting any row whose
/// underlying item no longer exists (self-healing — reconciling against
/// Focus's own bounded event fetch would risk false positives for
/// infrequent recurring events, so this checks the `events` table directly).
#[server]
pub async fn list_suppressed_focus_items() -> Result<Vec<String>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        return list_suppressed_focus_items_core(&pool, &username).await;
    }
    #[allow(unreachable_code)]
    Ok(vec![])
}

/// Suppress an item. Idempotent — suppressing an already-suppressed key is a no-op.
#[server(input = server_fn::codec::Json)]
pub async fn suppress_focus_item(item_key: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        return suppress_focus_item_core(&pool, &username, &item_key).await;
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Unsuppress an item. A no-op if it wasn't suppressed.
#[server(input = server_fn::codec::Json)]
pub async fn unsuppress_focus_item(item_key: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        return unsuppress_focus_item_core(&pool, &username, &item_key).await;
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Strip a trailing `"|{occurrence_start}"` suffix, if present, leaving the
/// base `events.uid` this key is ultimately anchored to.
#[cfg(feature = "ssr")]
fn base_uid_of(item_key: &str) -> &str {
    item_key.split('|').next().unwrap_or(item_key)
}

#[cfg(feature = "ssr")]
async fn suppress_focus_item_core(
    pool: &sqlx::SqlitePool,
    username: &str,
    item_key: &str,
) -> Result<(), ServerFnError> {
    let id = uuid::Uuid::new_v4().to_string();
    sqlx::query(
        "INSERT OR IGNORE INTO focus_suppressions (id, username, item_key) VALUES (?, ?, ?)",
    )
    .bind(&id)
    .bind(username)
    .bind(item_key)
    .execute(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    Ok(())
}

#[cfg(feature = "ssr")]
async fn unsuppress_focus_item_core(
    pool: &sqlx::SqlitePool,
    username: &str,
    item_key: &str,
) -> Result<(), ServerFnError> {
    sqlx::query("DELETE FROM focus_suppressions WHERE username = ? AND item_key = ?")
        .bind(username)
        .bind(item_key)
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    Ok(())
}

#[cfg(feature = "ssr")]
async fn list_suppressed_focus_items_core(
    pool: &sqlx::SqlitePool,
    username: &str,
) -> Result<Vec<String>, ServerFnError> {
    use sqlx::Row;

    let rows = sqlx::query("SELECT item_key FROM focus_suppressions WHERE username = ?")
        .bind(username)
        .fetch_all(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    let keys: Vec<String> = rows
        .into_iter()
        .map(|r| r.get::<String, _>("item_key"))
        .collect();

    if keys.is_empty() {
        return Ok(keys);
    }

    let base_uids: std::collections::HashSet<&str> = keys.iter().map(|k| base_uid_of(k)).collect();
    let placeholders = base_uids.iter().map(|_| "?").collect::<Vec<_>>().join(",");
    let sql = format!("SELECT uid FROM events WHERE username = ? AND uid IN ({placeholders})");
    let mut q = sqlx::query(&sql).bind(username);
    for uid in &base_uids {
        q = q.bind(*uid);
    }
    let existing_rows = q
        .fetch_all(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    let existing: std::collections::HashSet<String> = existing_rows
        .into_iter()
        .map(|r| r.get::<String, _>("uid"))
        .collect();

    let stale: Vec<&String> = keys
        .iter()
        .filter(|k| !existing.contains(base_uid_of(k)))
        .collect();
    for key in &stale {
        sqlx::query("DELETE FROM focus_suppressions WHERE username = ? AND item_key = ?")
            .bind(username)
            .bind(key.as_str())
            .execute(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }

    Ok(keys
        .into_iter()
        .filter(|k| existing.contains(base_uid_of(k)))
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::SqlitePool;

    async fn create_test_user(pool: &SqlitePool, username: &str) {
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

    async fn create_test_event(pool: &SqlitePool, username: &str, uid: &str) {
        // `etag` is NOT NULL in the `events` schema — must be supplied even
        // though these tests never read it back.
        sqlx::query(
            "INSERT INTO events (uid, username, calendar, type, etag, summary, start_date, end_date, ical_data) \
             VALUES (?, ?, 'personal', 'VEVENT', 'test-etag', 'Test', '2026-01-01 00:00:00', '2026-01-01 01:00:00', '')",
        )
        .bind(uid)
        .bind(username)
        .execute(pool)
        .await
        .unwrap();
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn suppress_then_list_roundtrip(pool: SqlitePool) {
        create_test_user(&pool, "alice").await;
        create_test_event(&pool, "alice", "evt-1").await;

        suppress_focus_item_core(&pool, "alice", "evt-1")
            .await
            .unwrap();
        let keys = list_suppressed_focus_items_core(&pool, "alice")
            .await
            .unwrap();
        assert_eq!(keys, vec!["evt-1".to_string()]);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn double_suppress_is_idempotent(pool: SqlitePool) {
        create_test_user(&pool, "alice").await;
        create_test_event(&pool, "alice", "evt-1").await;

        suppress_focus_item_core(&pool, "alice", "evt-1")
            .await
            .unwrap();
        suppress_focus_item_core(&pool, "alice", "evt-1")
            .await
            .unwrap();
        let keys = list_suppressed_focus_items_core(&pool, "alice")
            .await
            .unwrap();
        assert_eq!(keys, vec!["evt-1".to_string()]);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn unsuppress_removes_it(pool: SqlitePool) {
        create_test_user(&pool, "alice").await;
        create_test_event(&pool, "alice", "evt-1").await;

        suppress_focus_item_core(&pool, "alice", "evt-1")
            .await
            .unwrap();
        unsuppress_focus_item_core(&pool, "alice", "evt-1")
            .await
            .unwrap();
        let keys = list_suppressed_focus_items_core(&pool, "alice")
            .await
            .unwrap();
        assert!(keys.is_empty());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reconcile_deletes_suppression_for_deleted_item_keeps_live_one(pool: SqlitePool) {
        create_test_user(&pool, "alice").await;
        create_test_event(&pool, "alice", "evt-live").await;
        // "evt-gone" is suppressed but never inserted into `events` — simulates deletion.
        suppress_focus_item_core(&pool, "alice", "evt-live")
            .await
            .unwrap();
        suppress_focus_item_core(&pool, "alice", "evt-gone")
            .await
            .unwrap();

        let keys = list_suppressed_focus_items_core(&pool, "alice")
            .await
            .unwrap();
        assert_eq!(keys, vec!["evt-live".to_string()]);

        // The stale row is actually gone from the table, not just filtered.
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM focus_suppressions WHERE username = ?")
                .bind("alice")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 1);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reconcile_keeps_occurrence_suppression_whose_series_still_exists(pool: SqlitePool) {
        create_test_user(&pool, "alice").await;
        create_test_event(&pool, "alice", "evt-series").await;

        suppress_focus_item_core(&pool, "alice", "evt-series|20260615T090000")
            .await
            .unwrap();
        let keys = list_suppressed_focus_items_core(&pool, "alice")
            .await
            .unwrap();
        assert_eq!(keys, vec!["evt-series|20260615T090000".to_string()]);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reconcile_is_scoped_to_the_calling_user(pool: SqlitePool) {
        create_test_user(&pool, "alice").await;
        create_test_user(&pool, "bob").await;
        create_test_event(&pool, "bob", "evt-bob").await;
        // Alice suppresses a key that happens to match an id only bob owns —
        // reconciling alice's list must not treat bob's event as "hers".
        suppress_focus_item_core(&pool, "alice", "evt-bob")
            .await
            .unwrap();

        let alice_keys = list_suppressed_focus_items_core(&pool, "alice")
            .await
            .unwrap();
        assert!(alice_keys.is_empty());
    }

    #[test]
    fn base_uid_of_strips_occurrence_suffix() {
        assert_eq!(base_uid_of("abc123"), "abc123");
        assert_eq!(base_uid_of("abc123|20260615T090000"), "abc123");
    }
}
