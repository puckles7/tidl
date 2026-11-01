//! Server functions for Notes-tab collections and notes (backlog #37).

use leptos::prelude::*;
use server_fn::ServerFnError;

#[cfg(feature = "ssr")]
use crate::grid::{self, CARD_GRID_COLS};
use crate::types::{
    GridPosition, NewNote, NewNoteCollection, NewNoteImage, NoteCollectionInfo, NoteInfo,
    NoteLabelInfo,
};

#[cfg(feature = "ssr")]
use crate::types::{note_image_ids_in_text, rewrite_note_image_temp_tokens, NoteImageInfo};

#[cfg(feature = "ssr")]
use sqlx::{Row, SqlitePool};

#[cfg(feature = "ssr")]
use crate::api::auth;

#[cfg(feature = "ssr")]
use crate::components::notes::note_label_editor_modal::LABEL_PALETTE;

// ── Access control (SSR-only) ─────────────────────────────────────────────────

/// True if `username` owns collection `collection_id`.
#[cfg(feature = "ssr")]
async fn is_collection_owner(
    pool: &SqlitePool,
    username: &str,
    collection_id: i64,
) -> Result<bool, ServerFnError> {
    let owner: Option<String> =
        sqlx::query_scalar("SELECT owner_username FROM note_collections WHERE id = ?")
            .bind(collection_id)
            .fetch_optional(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    Ok(owner.as_deref() == Some(username))
}

/// True if `username` may read/write notes in `collection_id` — the owner,
/// or a member of a shared collection.
#[cfg(feature = "ssr")]
pub(crate) async fn has_collection_access(
    pool: &SqlitePool,
    username: &str,
    collection_id: i64,
) -> Result<bool, ServerFnError> {
    if is_collection_owner(pool, username, collection_id).await? {
        return Ok(true);
    }
    let member: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM note_collection_members WHERE collection_id = ? AND member_username = ?",
    )
    .bind(collection_id)
    .bind(username)
    .fetch_optional(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    Ok(member.is_some())
}

// ── Collections ────────────────────────────────────────────────────────────────

/// List all collections the current user owns or is a member of.
#[server]
pub async fn list_note_collections() -> Result<Vec<NoteCollectionInfo>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        let rows = sqlx::query(
            "SELECT id, owner_username, name, color, shared FROM note_collections
             WHERE owner_username = ?
                OR id IN (SELECT collection_id FROM note_collection_members WHERE member_username = ?)
             ORDER BY name ASC",
        )
        .bind(&username)
        .bind(&username)
        .fetch_all(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let mut out = Vec::with_capacity(rows.len());
        for r in rows {
            let id: i64 = r.try_get("id").unwrap_or_default();
            let owner: String = r.try_get("owner_username").unwrap_or_default();
            let shared: bool = r.try_get("shared").unwrap_or_default();

            let members: Vec<String> = if shared {
                sqlx::query_scalar(
                    "SELECT member_username FROM note_collection_members WHERE collection_id = ?",
                )
                .bind(id)
                .fetch_all(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
            } else {
                vec![]
            };

            out.push(NoteCollectionInfo {
                id,
                name: r.try_get("name").unwrap_or_default(),
                color: r.try_get("color").unwrap_or_default(),
                shared,
                is_owner: owner == username,
                members,
                owner_username: owner,
            });
        }
        return Ok(out);
    }
    #[allow(unreachable_code)]
    Ok(vec![])
}

/// The set of usernames currently confirmed as members of `collection_id`
/// (empty for a brand-new collection that doesn't exist yet). Used to filter
/// client-submitted member lists on save so `create_note_collection`/
/// `update_note_collection` can only ever *shrink* membership, never grow
/// it — growing only happens via an accepted `share_note_collection` invite.
/// Without this filter, a caller could probe whether a username exists by
/// submitting it here and checking if it round-trips back from
/// `list_note_collections`.
#[cfg(feature = "ssr")]
async fn existing_note_collection_members<'e, E>(
    executor: E,
    collection_id: i64,
) -> Result<std::collections::HashSet<String>, ServerFnError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT member_username FROM note_collection_members WHERE collection_id = ?",
    )
    .bind(collection_id)
    .fetch_all(executor)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    Ok(rows.into_iter().collect())
}

/// Create a new collection owned by the current user.
///
/// The collection insert and its member inserts happen in one transaction —
/// matching the `pool.begin()`/`tx.commit()` convention already used for
/// multi-write operations in `api/calendars.rs::delete_calendar_api` — so a
/// mid-way failure never leaves a collection with a half-written member list.
#[server(input = server_fn::codec::Json)]
pub async fn create_note_collection(input: NewNoteCollection) -> Result<i64, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        if input.name.trim().is_empty() {
            return Err(ServerFnError::new("Name is required"));
        }

        let mut tx = pool
            .begin()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let id: i64 = sqlx::query(
            "INSERT INTO note_collections (owner_username, name, color, shared)
             VALUES (?, ?, ?, ?) RETURNING id",
        )
        .bind(&username)
        .bind(input.name.trim())
        .bind(&input.color)
        .bind(input.shared)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
        .try_get("id")
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // A brand-new collection has no confirmed members yet, so any
        // usernames the client passed in `input.members` are dropped —
        // membership can only grow via an accepted `share_note_collection`
        // invite (see `existing_note_collection_members`'s doc comment,
        // used the same way by `update_note_collection`).

        tx.commit()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        return Ok(id);
    }
    #[allow(unreachable_code)]
    Ok(0)
}

/// Update an existing collection's name/color/sharing. Owner only.
/// Member-list replacement is transactional for the same reason as create.
#[server(input = server_fn::codec::Json)]
pub async fn update_note_collection(
    id: i64,
    input: NewNoteCollection,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        if !is_collection_owner(&pool, &username, id).await? {
            return Err(ServerFnError::new("Not authorized to edit this collection"));
        }
        if input.name.trim().is_empty() {
            return Err(ServerFnError::new("Name is required"));
        }

        let mut tx = pool
            .begin()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        sqlx::query(
            "UPDATE note_collections SET name = ?, color = ?, shared = ?, updated_at = datetime('now')
             WHERE id = ?",
        )
        .bind(input.name.trim())
        .bind(&input.color)
        .bind(input.shared)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Capture confirmed membership before wiping it, so the submitted
        // `input.members` can only ever retain a subset of it — never add a
        // username that wasn't already a real member (see
        // existing_note_collection_members's doc comment for why).
        let existing = existing_note_collection_members(&mut *tx, id).await?;

        // Replace the member list wholesale (small lists, simplest correct approach).
        sqlx::query("DELETE FROM note_collection_members WHERE collection_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        if input.shared {
            for member in input.members.iter().filter(|m| existing.contains(*m)) {
                sqlx::query(
                    "INSERT OR IGNORE INTO note_collection_members (collection_id, member_username) VALUES (?, ?)",
                )
                .bind(id)
                .bind(member)
                .execute(&mut *tx)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            }
        }

        tx.commit()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Create a share invitation for one of the current user's note collections
/// and email an accept/decline link to `invitee_email` — mirrors
/// `calendars::share_calendar`. Never reveals whether `invitee_email`
/// belongs to an existing account: the invite is always "sent" (silently
/// no-op if SMTP isn't configured), and only actually grants access once
/// the accepting, logged-in user's own email matches it.
#[server]
pub async fn share_note_collection(
    collection_id: i64,
    invitee_email: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::{ServerBaseUrl, ServerSecret};
        use axum::Extension;
        use chrono::{Duration, Utc};
        use leptos_axum::extract;
        use uuid::Uuid;

        let (pool, username) = auth().await?;

        if !is_collection_owner(&pool, &username, collection_id).await? {
            return Err(ServerFnError::new(
                "Not authorized to share this collection",
            ));
        }

        let base_url = extract::<Extension<ServerBaseUrl>>()
            .await
            .map(|e| e.0 .0.clone())
            .unwrap_or_default();
        let server_secret = extract::<Extension<ServerSecret>>()
            .await
            .map(|e| e.0 .0.clone())
            .unwrap_or_default();

        let name: String = sqlx::query_scalar(
            "SELECT name FROM note_collections WHERE id = ? AND owner_username = ?",
        )
        .bind(collection_id)
        .bind(&username)
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
        .ok_or_else(|| ServerFnError::new("Collection not found"))?;

        let token = Uuid::new_v4().to_string();
        let expires_at = Utc::now() + Duration::days(7);
        let expires_str = expires_at.format("%Y-%m-%d %H:%M:%S").to_string();

        sqlx::query(
            "INSERT INTO share_invitations \
             (owner_username, resource_type, collection_id, invitee_email, permission, token, expires_at) \
             VALUES (?, 'note_collection', ?, ?, 'member', ?, ?)"
        )
        .bind(&username).bind(collection_id).bind(&invitee_email)
        .bind(&token).bind(&expires_str)
        .execute(&pool)
        .await
        .map_err(|e| ServerFnError::new(e.to_string()))?;

        super::sharing::send_share_invitation_email(
            &pool,
            &server_secret,
            &base_url,
            &username,
            "collection",
            &name,
            None,
            &token,
            &invitee_email,
        )
        .await?;

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Remove one member from a shared collection, without touching the rest of
/// the member list. A dedicated single-row DELETE (vs. reusing
/// `update_note_collection`'s wholesale replace) so a concurrent edit from
/// the collection editor modal can't clobber this removal or vice versa.
/// Owner only.
#[server]
pub async fn remove_note_collection_member(
    collection_id: i64,
    member_username: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        if !is_collection_owner(&pool, &username, collection_id).await? {
            return Err(ServerFnError::new("Not authorized to edit this collection"));
        }

        sqlx::query(
            "DELETE FROM note_collection_members WHERE collection_id = ? AND member_username = ?",
        )
        .bind(collection_id)
        .bind(&member_username)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Remove the current user's own membership from a collection they don't
/// own — the member's side of `remove_note_collection_member`, since that
/// one is hard-scoped to the owner. Mirrors `leave_shared_calendar`.
#[server]
pub async fn leave_note_collection(collection_id: i64) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        sqlx::query(
            "DELETE FROM note_collection_members WHERE collection_id = ? AND member_username = ?",
        )
        .bind(collection_id)
        .bind(&username)
        .execute(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Turn a shared collection back to private: clears its member list and
/// flips `shared` off in one transaction. Owner only.
#[server]
pub async fn unshare_note_collection(collection_id: i64) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        if !is_collection_owner(&pool, &username, collection_id).await? {
            return Err(ServerFnError::new("Not authorized to edit this collection"));
        }

        let mut tx = pool
            .begin()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        sqlx::query(
            "UPDATE note_collections SET shared = 0, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(collection_id)
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        sqlx::query("DELETE FROM note_collection_members WHERE collection_id = ?")
            .bind(collection_id)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Delete a collection (cascades to its notes, and to any mirrored VTODOs —
/// see Task 4's `delete_note` for why cascading the mirror requires an
/// explicit sweep rather than relying on `ON DELETE CASCADE`). Owner only.
/// The whole sweep — mirror deletes + collection delete — is one transaction.
#[server]
pub async fn delete_note_collection(id: i64) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        if !is_collection_owner(&pool, &username, id).await? {
            return Err(ServerFnError::new(
                "Not authorized to delete this collection",
            ));
        }

        let mut tx = pool
            .begin()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Deleting the collection cascades to its notes and — via
        // `events.note_id`'s ON DELETE CASCADE — their task/await events.
        // Bump each affected calendar first so CalDAV clients resync.
        let affected: Vec<(String, String)> = sqlx::query(
            "SELECT e.calendar, e.uid FROM events e
             JOIN notes n ON n.id = e.note_id
             WHERE n.collection_id = ? AND e.source = 'note'",
        )
        .bind(id)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
        .into_iter()
        .filter_map(|r| {
            let cal: Option<String> = r.try_get("calendar").ok();
            let uid: Option<String> = r.try_get("uid").ok();
            Some((cal?, uid?))
        })
        .collect();

        for (cal, uid) in affected {
            bump_calendar_sync_token(&mut tx, &username, &cal, &uid, "deleted").await?;
        }

        sqlx::query("DELETE FROM note_collections WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

// ── Notes ──────────────────────────────────────────────────────────────────────

/// Computes where a new note (or one that just crossed the pinned/
/// non-pinned boundary) should go: whichever column in `pinned`'s zone
/// currently has the smallest total occupied height (`row + span`, maxed
/// per column), scoped to whatever collections `username` can see
/// (matching `list_notes`'s access filter, not the whole table — two
/// unrelated users' notes must never be treated as sharing space).
/// `exclude_id`, if given, is left out (used when re-placing a note that
/// already has a — about to be overwritten — position of its own). Both
/// zones start at row 0 (backlog #90 removed the pinned zone's former
/// "+ Take a note…" tile reservation, moving that trigger to a toolbar
/// button instead). `new_span` is
/// unused today (which column is shortest doesn't depend on the
/// placement's own span, only on how much it grows the chosen column by
/// afterward) but kept in the signature so callers always have it on hand
/// for a future column-capacity heuristic.
#[cfg(feature = "ssr")]
async fn place_in_first_gap(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    username: &str,
    pinned: bool,
    exclude_id: Option<i64>,
    new_span: i32,
) -> Result<(i32, i32), ServerFnError> {
    let _ = new_span;
    let rows: Vec<(i32, i32, i32)> = sqlx::query_as(
        "SELECT n.grid_row, n.grid_col, n.grid_row_span
         FROM notes n
         JOIN note_collections c ON c.id = n.collection_id
         WHERE n.pinned = ?
           AND n.id != COALESCE(?, -1)
           AND (c.owner_username = ?
                OR c.id IN (SELECT collection_id FROM note_collection_members WHERE member_username = ?))",
    )
    .bind(pinned)
    .bind(exclude_id)
    .bind(username)
    .bind(username)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    Ok(grid::shortest_column(&rows, CARD_GRID_COLS, 0, new_span))
}

/// Writes every `(id, grid_row, grid_col)` in `moves` in a temporary,
/// mutually-unique parking position first, then in a second pass to their
/// real final positions — so applying a batch of moves that includes notes
/// swapping/cascading through each other's cells never transiently
/// collides with another move in the *same* batch, regardless of what
/// order the batch happens to list them in.
#[cfg(feature = "ssr")]
async fn apply_positions(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    moves: &[GridPosition],
) -> Result<(), ServerFnError> {
    for (i, m) in moves.iter().enumerate() {
        sqlx::query("UPDATE notes SET grid_row = ?, grid_col = 0 WHERE id = ?")
            .bind(-1000 - i as i32)
            .bind(m.id)
            .execute(&mut **tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }
    for m in moves {
        sqlx::query("UPDATE notes SET grid_row = ?, grid_col = ?, grid_row_span = ? WHERE id = ?")
            .bind(m.grid_row)
            .bind(m.grid_col)
            .bind(m.grid_row_span)
            .bind(m.id)
            .execute(&mut **tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }
    Ok(())
}

/// Bumps the target calendar's `sync_token` and appends a `sync_log` row,
/// mirroring what `tidl-server`'s `Storage::bump_sync_token` does for every
/// CalDAV-originated `store_event`/`delete_event` (see
/// `migrations/20240109000000_sync_tokens.sql`). `tidl-web` cannot depend on
/// `tidl-server` (CLAUDE.md §6), so the note-mirror path — which writes
/// straight to `events` instead of going through `Storage` — has to
/// replicate this rather than call it: without it, a mirrored VTODO's
/// `getctag`/`sync-token` never changes, so CalDAV clients that only
/// re-fetch a collection when its ctag changes (e.g. Thunderbird) never
/// notice the new/changed/deleted task. A no-op (not an error) when
/// `calendar` has no matching `calendars` row, since notes can mirror into
/// a `calendar_name` that doesn't correspond to one.
#[cfg(feature = "ssr")]
async fn bump_calendar_sync_token(
    conn: &mut sqlx::SqliteConnection,
    username: &str,
    calendar: &str,
    uid: &str,
    operation: &str,
) -> Result<(), ServerFnError> {
    sqlx::query("UPDATE calendars SET sync_token = sync_token + 1 WHERE username = ? AND name = ?")
        .bind(username)
        .bind(calendar)
        .execute(&mut *conn)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let token: Option<i64> =
        sqlx::query_scalar("SELECT sync_token FROM calendars WHERE username = ? AND name = ?")
            .bind(username)
            .bind(calendar)
            .fetch_optional(&mut *conn)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let Some(token) = token else {
        return Ok(());
    };

    sqlx::query(
        "INSERT INTO sync_log (username, calendar, uid, operation, token) VALUES (?, ?, ?, ?, ?)",
    )
    .bind(username)
    .bind(calendar)
    .bind(uid)
    .bind(operation)
    .bind(token)
    .execute(&mut *conn)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    Ok(())
}

/// Reconciles a note's inline `#task`/`#await` tokens with its `source='note'`
/// VTODO rows (backlog #113). The note body is the source of truth, so every
/// save deletes the note's existing events and rebuilds them from the body —
/// exactly the shape `markdown_sync.rs` uses for markdown tags. `#done` tokens
/// are inert (no event). Uids are `note:<id>:<kind>:<line>`, so an in-line
/// completion (which never removes a line) leaves every other token's uid
/// stable.
///
/// Takes `&mut SqliteConnection` (i.e. `&mut *tx`, not `&pool`) so callers run
/// this in the same transaction as the `notes` table write — otherwise a crash
/// between the two writes would leave orphaned `events` rows or a note with no
/// events. Same transactional-write convention as
/// `api/calendars.rs::delete_calendar_api`.
#[cfg(feature = "ssr")]
pub(crate) async fn sync_note_tokens(
    conn: &mut sqlx::SqliteConnection,
    username: &str,
    note_id: i64,
    calendar_name: Option<&str>,
    text: &str,
) -> Result<(), ServerFnError> {
    use crate::note_tokens::{scan_note_tokens, slug_to_title, ScannedTokenKind};

    // Calendars the rows about to be deleted lived in, fetched first so their
    // sync tokens can be bumped after the delete.
    let old_calendars: Vec<String> = sqlx::query_scalar(
        "SELECT DISTINCT calendar FROM events WHERE source = 'note' AND note_id = ?",
    )
    .bind(note_id)
    .fetch_all(&mut *conn)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    sqlx::query("DELETE FROM events WHERE source = 'note' AND note_id = ?")
        .bind(note_id)
        .execute(&mut *conn)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    for old_calendar in old_calendars {
        bump_calendar_sync_token(conn, username, &old_calendar, "", "deleted").await?;
    }

    let calendar = calendar_name
        .filter(|c| !c.is_empty())
        .unwrap_or("personal");

    // Uid is `note:<id>:<line>:<ordinal>`, where the ordinal counts every token
    // on that line in order — so two tokens on one line stay distinct, and an
    // in-place completion (which keeps the token count) leaves the others'
    // ordinals untouched.
    let mut line_ord: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();

    for token in scan_note_tokens(text) {
        let ord = {
            let e = line_ord.entry(token.line).or_insert(0);
            let this = *e;
            *e += 1;
            this
        };
        let (kind, priority, due_date) = match &token.kind {
            ScannedTokenKind::Task { priority, .. } => ("task", Some(*priority), None),
            ScannedTokenKind::Await { date, .. } => {
                ("await", None, Some(date.format("%Y-%m-%d").to_string()))
            }
            ScannedTokenKind::Done { .. } => continue,
        };
        let slug = token.kind.slug();
        let summary = slug_to_title(slug);
        let uid = format!("note:{note_id}:{}:{ord}", token.line);
        let etag = format!("etag-{}", uuid::Uuid::new_v4().simple());

        let due_line = due_date
            .as_deref()
            .map(|d| format!("\r\nDUE;VALUE=DATE:{}", d.replace('-', "")))
            .unwrap_or_default();
        // The full note text in DESCRIPTION, not the per-token summary —
        // anyone who can see this to-do via calendar sharing (but has no
        // Notes-UI access to the note's collection) should still be able to
        // read the whole note. See backlog #39.
        let description_line = {
            use tidl_caldav::icalendar_utils::{escape_ical_text, fold_ical_line};
            format!(
                "\r\n{}",
                fold_ical_line(&format!("DESCRIPTION:{}", escape_ical_text(text)))
            )
        };
        // The ical PRIORITY line is the only place the canonical 1-100 scale
        // gets projected onto RFC 5545's native 1-9 range — see
        // `types::source_priority_to_caldav` (backlog #70). An await has no
        // stored priority (Await sorts by date; Focus scores it by
        // pending_urgency).
        let priority_line = if kind == "task" {
            format!(
                "\r\nPRIORITY:{}",
                crate::types::source_priority_to_caldav(priority.unwrap_or(50))
            )
        } else {
            String::new()
        };
        let ical_data = format!(
            "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:-//tidl//tidl//EN\r\n\
             BEGIN:VTODO\r\nUID:{uid}\r\nSUMMARY:{summary}{priority_line}\r\n\
             STATUS:NEEDS-ACTION{due_line}{description_line}\r\n\
             END:VTODO\r\nEND:VCALENDAR\r\n"
        );

        // The DB column stores the canonical 1-100 value verbatim — no
        // projection — matching every other task source (markdown, CalDAV).
        let db_priority = if kind == "task" {
            Some(priority.unwrap_or(50) as i64)
        } else {
            None
        };
        // `events.due_date` is a DATETIME column; a bare `YYYY-MM-DD` fails
        // `storage::list_events`'s `DateTime<Utc>` decode, so normalize to
        // midnight (same as the CalDAV-import path). The ical `DUE` line above
        // is unaffected.
        let events_due_date = due_date.as_deref().map(|d| format!("{d} 00:00:00"));

        sqlx::query(
            "INSERT INTO events
             (username, calendar, uid, type, summary, priority, status, due_date,
              source, etag, ical_data, kind, source_line, topic, note_id, description)
             VALUES (?, ?, ?, 'VTODO', ?, ?, 'NEEDS-ACTION', ?, 'note', ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(username)
        .bind(calendar)
        .bind(&uid)
        .bind(&summary)
        .bind(db_priority)
        .bind(&events_due_date)
        .bind(&etag)
        .bind(&ical_data)
        .bind(kind)
        .bind(token.line as i64)
        .bind(slug)
        .bind(note_id)
        .bind(text)
        .execute(&mut *conn)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        bump_calendar_sync_token(conn, username, calendar, &uid, "modified").await?;
    }

    Ok(())
}

/// Overwrites one note's body text in place — re-syncing its inline
/// `#task`/`#await` events via `sync_note_tokens` — without touching any other
/// field. Used by contact merge (backlog #88) to re-point `@mention`s at the
/// surviving contact's new name. Unlike `update_note`, this is a
/// system-initiated rewrite of already-known-owned text (the caller selected
/// `note_id` by querying `notes.author_username` itself), so it skips access
/// checks and pinned-zone repositioning — `pinned` never changes here.
#[cfg(feature = "ssr")]
pub(crate) async fn rewrite_note_body(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    username: &str,
    note_id: i64,
    new_text: String,
) -> Result<(), ServerFnError> {
    let calendar_name: Option<String> =
        sqlx::query_scalar("SELECT calendar_name FROM notes WHERE id = ?")
            .bind(note_id)
            .fetch_one(&mut **tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    sqlx::query("UPDATE notes SET text = ?, updated_at = datetime('now') WHERE id = ?")
        .bind(new_text.trim())
        .bind(note_id)
        .execute(&mut **tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    sync_note_tokens(
        tx,
        username,
        note_id,
        calendar_name.as_deref(),
        new_text.trim(),
    )
    .await?;
    Ok(())
}

/// List all notes visible to the current user (own collections + shared memberships).
#[server]
pub async fn list_notes() -> Result<Vec<NoteInfo>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        return list_notes_for(&pool, &username).await;
    }
    #[allow(unreachable_code)]
    Ok(vec![])
}

/// `list_notes`'s body, split out so it's directly testable without a
/// live `#[server]` request context (mirrors `api/contacts.rs`'s
/// `list_contacts_for` split).
#[cfg(feature = "ssr")]
async fn list_notes_for(pool: &SqlitePool, username: &str) -> Result<Vec<NoteInfo>, ServerFnError> {
    let rows = sqlx::query(
        "SELECT n.id, n.collection_id, n.title, n.text, n.pinned,
                n.calendar_name,
                CAST(n.updated_at AS TEXT) AS updated_at, n.grid_row, n.grid_col,
                n.grid_row_span, n.manual_order
         FROM notes n
         JOIN note_collections c ON c.id = n.collection_id
         WHERE c.owner_username = ?
            OR c.id IN (SELECT collection_id FROM note_collection_members WHERE member_username = ?)
         ORDER BY n.pinned DESC, n.grid_row ASC, n.grid_col ASC",
    )
    .bind(username)
    .bind(username)
    .fetch_all(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let mut notes: Vec<NoteInfo> = rows
        .into_iter()
        .map(|r| NoteInfo {
            id: r.try_get("id").unwrap_or_default(),
            collection_id: r.try_get("collection_id").unwrap_or_default(),
            title: r.try_get("title").unwrap_or_default(),
            text: r.try_get("text").unwrap_or_default(),
            pinned: r.try_get("pinned").unwrap_or_default(),
            calendar_name: r.try_get("calendar_name").unwrap_or_default(),
            updated_at: r.try_get("updated_at").unwrap_or_default(),
            grid_row: r.try_get("grid_row").unwrap_or_default(),
            grid_col: r.try_get("grid_col").unwrap_or_default(),
            grid_row_span: r.try_get("grid_row_span").unwrap_or(1),
            manual_order: r.try_get("manual_order").unwrap_or_default(),
            images: Vec::new(),
            labels: Vec::new(),
        })
        .collect();

    // Image metadata (never the blob) for every note the caller can see, in
    // one query, grouped by note id (backlog #110).
    let image_rows = sqlx::query(
        "SELECT i.id, i.note_id, i.content_type, i.filename, i.position
         FROM note_images i
         JOIN notes n ON n.id = i.note_id
         JOIN note_collections c ON c.id = n.collection_id
         WHERE c.owner_username = ?
            OR c.id IN (SELECT collection_id FROM note_collection_members WHERE member_username = ?)
         ORDER BY i.note_id, i.position, i.id",
    )
    .bind(username)
    .bind(username)
    .fetch_all(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let mut by_note: std::collections::HashMap<i64, Vec<NoteImageInfo>> =
        std::collections::HashMap::new();
    for r in image_rows {
        let note_id: i64 = r.try_get("note_id").unwrap_or_default();
        by_note.entry(note_id).or_default().push(NoteImageInfo {
            id: r.try_get("id").unwrap_or_default(),
            content_type: r.try_get("content_type").unwrap_or_default(),
            filename: r.try_get("filename").unwrap_or_default(),
            position: r.try_get("position").unwrap_or_default(),
        });
    }
    for note in &mut notes {
        if let Some(images) = by_note.remove(&note.id) {
            note.images = images;
        }
    }

    // Label metadata for every note the caller can see, in one grouped query
    // (backlog #114), mirroring the image block above.
    let label_rows = sqlx::query(
        "SELECT l.id, lnk.note_id, l.name, l.color
         FROM note_labels l
         JOIN note_label_links lnk ON lnk.label_id = l.id
         JOIN notes n ON n.id = lnk.note_id
         JOIN note_collections c ON c.id = n.collection_id
         WHERE c.owner_username = ?
            OR c.id IN (SELECT collection_id FROM note_collection_members WHERE member_username = ?)
         ORDER BY lnk.note_id, l.name COLLATE NOCASE",
    )
    .bind(username)
    .bind(username)
    .fetch_all(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let mut labels_by_note: std::collections::HashMap<i64, Vec<NoteLabelInfo>> =
        std::collections::HashMap::new();
    for r in label_rows {
        let note_id: i64 = r.try_get("note_id").unwrap_or_default();
        labels_by_note
            .entry(note_id)
            .or_default()
            .push(NoteLabelInfo {
                id: r.try_get("id").unwrap_or_default(),
                name: r.try_get("name").unwrap_or_default(),
                color: r.try_get("color").unwrap_or_default(),
            });
    }
    for note in &mut notes {
        if let Some(labels) = labels_by_note.remove(&note.id) {
            note.labels = labels;
        }
    }

    Ok(notes)
}

/// Existing-note state + access check shared by [`update_note`] and
/// [`save_note`], fetched before opening the write transaction.
#[cfg(feature = "ssr")]
struct NoteEditState {
    existing_pinned: bool,
    existing_span: i32,
}

#[cfg(feature = "ssr")]
async fn note_edit_state(
    pool: &SqlitePool,
    username: &str,
    id: i64,
    new_collection_id: i64,
) -> Result<NoteEditState, ServerFnError> {
    let existing_collection: Option<i64> =
        sqlx::query_scalar("SELECT collection_id FROM notes WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    let existing_collection =
        existing_collection.ok_or_else(|| ServerFnError::new("Note not found"))?;

    if !has_collection_access(pool, username, existing_collection).await?
        || !has_collection_access(pool, username, new_collection_id).await?
    {
        return Err(ServerFnError::new("Not authorized for this collection"));
    }

    let existing_pinned: bool = sqlx::query_scalar("SELECT pinned FROM notes WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    let existing_span: i32 = sqlx::query_scalar("SELECT grid_row_span FROM notes WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    Ok(NoteEditState {
        existing_pinned,
        existing_span,
    })
}

/// Stable palette colour for a label name (backlog #114/#127) — used when a
/// label is created without an explicit colour (implicitly via the note
/// editor, or a blank colour passed to `create_note_label`).
#[cfg(feature = "ssr")]
fn label_color_for_name(name: &str) -> &'static str {
    let hash = name.bytes().fold(0usize, |acc, b| {
        acc.wrapping_mul(31).wrapping_add(b as usize)
    });
    LABEL_PALETTE[hash % LABEL_PALETTE.len()]
}

/// Resolves a label name to the note author's existing label (case-insensitive)
/// or creates one, using the label palette + stable name-hash colour
/// (backlog #114, #127). `cache` memoises lowercased-name → id across a batch.
/// `Ok(None)` for a blank name.
#[cfg(feature = "ssr")]
async fn resolve_label(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    owner: &str,
    name: &str,
    cache: &mut std::collections::HashMap<String, i64>,
) -> Result<Option<i64>, ServerFnError> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let key = trimmed.to_lowercase();
    if let Some(id) = cache.get(&key) {
        return Ok(Some(*id));
    }

    let existing: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM note_labels WHERE owner_username = ? AND LOWER(name) = ?",
    )
    .bind(owner)
    .bind(&key)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let id = match existing {
        Some(id) => id,
        None => {
            let color = label_color_for_name(trimmed);
            sqlx::query_scalar(
                "INSERT INTO note_labels (owner_username, name, color)
                 VALUES (?, ?, ?) RETURNING id",
            )
            .bind(owner)
            .bind(trimmed)
            .bind(color)
            .fetch_one(&mut **tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
        }
    };

    cache.insert(key, id);
    Ok(Some(id))
}

/// Replaces a note's label links with exactly `names` (blank/duplicate names
/// dropped). The label owner is the note's **author**, never the caller — a
/// grantee editing a shared note links the author's labels (backlog #114).
/// Returns the number of links written.
#[cfg(feature = "ssr")]
pub(crate) async fn set_note_labels(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    note_id: i64,
    names: &[String],
) -> Result<u32, ServerFnError> {
    let owner: String = sqlx::query_scalar("SELECT author_username FROM notes WHERE id = ?")
        .bind(note_id)
        .fetch_one(&mut **tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let mut cache = std::collections::HashMap::new();
    let mut ids: Vec<i64> = Vec::new();
    for name in names {
        if let Some(id) = resolve_label(tx, &owner, name, &mut cache).await? {
            if !ids.contains(&id) {
                ids.push(id);
            }
        }
    }

    sqlx::query("DELETE FROM note_label_links WHERE note_id = ?")
        .bind(note_id)
        .execute(&mut **tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    for label_id in &ids {
        sqlx::query("INSERT INTO note_label_links (note_id, label_id) VALUES (?, ?)")
            .bind(note_id)
            .bind(label_id)
            .execute(&mut **tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }

    Ok(ids.len() as u32)
}

/// Inserts a note row (plus its inline task/await events) inside an open
/// transaction. Shared by [`create_note`], [`save_note`], and the Google Keep
/// importer (`api/keep.rs`, backlog #109), which reuses it so an imported note
/// is placed on the grid exactly like a hand-created one.
#[cfg(feature = "ssr")]
pub(crate) async fn insert_note_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    username: &str,
    input: &NewNote,
) -> Result<i64, ServerFnError> {
    // A real line count, not a hardcoded placeholder (backlog #106
    // sixth follow-up, found live): `place_in_first_gap`'s own
    // shortest-column search -- and the row/span this note actually
    // gets stored with -- used to always assume span 1 regardless of
    // content, wildly undershooting a real card's rendered height
    // (a client-side height-correction pass fixes this up after the
    // fact, but only once it runs, and only after `notes_res`
    // eventually refetches to pick up the correction -- in the
    // meantime, Auto-layout's own masonry pack read the stale,
    // far-too-short span and could stack the very next note almost on
    // top of this one).
    let initial_span = grid::estimate_span(input.text.trim().lines().count() as i32);
    let (grid_row, grid_col) =
        place_in_first_gap(tx, username, input.pinned, None, initial_span).await?;

    let id: i64 = sqlx::query(
        "INSERT INTO notes
         (collection_id, author_username, title, text, pinned,
          calendar_name, grid_row, grid_col, grid_row_span)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(input.collection_id)
    .bind(username)
    .bind(
        input
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty()),
    )
    .bind(input.text.trim())
    .bind(input.pinned)
    .bind(&input.calendar_name)
    .bind(grid_row)
    .bind(grid_col)
    .bind(initial_span)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
    .try_get("id")
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    sync_note_tokens(
        tx,
        username,
        id,
        input.calendar_name.as_deref(),
        input.text.trim(),
    )
    .await?;

    set_note_labels(tx, id, &input.labels).await?;

    Ok(id)
}

/// Updates a note row (plus its inline task/await events) inside an open
/// transaction. Shared by [`update_note`] and [`save_note`].
#[cfg(feature = "ssr")]
async fn update_note_row(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    username: &str,
    id: i64,
    input: &NewNote,
    existing_pinned: bool,
    existing_span: i32,
) -> Result<(), ServerFnError> {
    sqlx::query(
        "UPDATE notes SET collection_id = ?, title = ?, text = ?, pinned = ?,
                calendar_name = ?, updated_at = datetime('now')
         WHERE id = ?",
    )
    .bind(input.collection_id)
    .bind(
        input
            .title
            .as_deref()
            .map(str::trim)
            .filter(|t| !t.is_empty()),
    )
    .bind(input.text.trim())
    .bind(input.pinned)
    .bind(&input.calendar_name)
    .bind(id)
    .execute(&mut **tx)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    if input.pinned != existing_pinned {
        let (grid_row, grid_col) =
            place_in_first_gap(tx, username, input.pinned, Some(id), existing_span).await?;
        sqlx::query("UPDATE notes SET grid_row = ?, grid_col = ? WHERE id = ?")
            .bind(grid_row)
            .bind(grid_col)
            .bind(id)
            .execute(&mut **tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }

    sync_note_tokens(
        tx,
        username,
        id,
        input.calendar_name.as_deref(),
        input.text.trim(),
    )
    .await?;

    set_note_labels(tx, id, &input.labels).await?;

    Ok(())
}

/// Create a note. Requires read/write access to `input.collection_id`.
/// The event sync and the `notes` insert are one transaction (see
/// `sync_note_tokens`'s doc comment for why).
#[server(input = server_fn::codec::Json)]
pub async fn create_note(input: NewNote) -> Result<i64, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        if !has_collection_access(&pool, &username, input.collection_id).await? {
            return Err(ServerFnError::new("Not authorized for this collection"));
        }
        if input.text.trim().is_empty() {
            return Err(ServerFnError::new("Note text is required"));
        }

        let mut tx = pool
            .begin()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let id = insert_note_row(&mut tx, &username, &input).await?;

        tx.commit()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        return Ok(id);
    }
    #[allow(unreachable_code)]
    Ok(0)
}

/// Update a note (text/collection/pin/calendar). Re-syncs its inline
/// task/await events in the same transaction as the `notes` update.
#[server(input = server_fn::codec::Json)]
pub async fn update_note(id: i64, input: NewNote) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        let state = note_edit_state(&pool, &username, id, input.collection_id).await?;
        if input.text.trim().is_empty() {
            return Err(ServerFnError::new("Note text is required"));
        }

        let mut tx = pool
            .begin()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        update_note_row(
            &mut tx,
            &username,
            id,
            &input,
            state.existing_pinned,
            state.existing_span,
        )
        .await?;

        tx.commit()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// List every label owned by the current user (backlog #114), ordered by name.
/// Used by the note editor's picker and the filter sidebar's Labels panel.
#[server(input = server_fn::codec::Json)]
pub async fn list_note_labels() -> Result<Vec<NoteLabelInfo>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        let rows = sqlx::query(
            "SELECT id, name, color FROM note_labels
             WHERE owner_username = ?
             ORDER BY name COLLATE NOCASE",
        )
        .bind(&username)
        .fetch_all(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        return Ok(rows
            .into_iter()
            .map(|r| NoteLabelInfo {
                id: r.try_get("id").unwrap_or_default(),
                name: r.try_get("name").unwrap_or_default(),
                color: r.try_get("color").unwrap_or_default(),
            })
            .collect());
    }
    #[allow(unreachable_code)]
    Ok(vec![])
}

/// True when `username` owns label `id`.
#[cfg(feature = "ssr")]
async fn owns_label(pool: &SqlitePool, username: &str, id: i64) -> Result<bool, ServerFnError> {
    let owner: Option<String> =
        sqlx::query_scalar("SELECT owner_username FROM note_labels WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    Ok(owner.as_deref() == Some(username))
}

/// Create one of the current user's labels with an explicit colour (backlog
/// #128) — the sidebar's "+" button. Rejects a blank name and a name that
/// (case-insensitively) collides with another of the user's labels. A blank
/// colour falls back to the stable name-hash palette colour.
#[server(input = server_fn::codec::Json)]
pub async fn create_note_label(name: String, color: String) -> Result<i64, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(ServerFnError::new("Label name is required"));
        }
        let collision: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM note_labels WHERE owner_username = ? AND LOWER(name) = ?",
        )
        .bind(&username)
        .bind(trimmed.to_lowercase())
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        if collision.is_some() {
            return Err(ServerFnError::new(
                "You already have a label with that name",
            ));
        }
        let color = if color.trim().is_empty() {
            label_color_for_name(trimmed).to_string()
        } else {
            color.trim().to_string()
        };
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO note_labels (owner_username, name, color)
             VALUES (?, ?, ?) RETURNING id",
        )
        .bind(&username)
        .bind(trimmed)
        .bind(&color)
        .fetch_one(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        return Ok(id);
    }
    #[allow(unreachable_code)]
    Ok(0)
}

/// Update one of the current user's labels' name and colour (backlog #114,
/// #128). Rejects a blank name and a name that (case-insensitively) collides
/// with another of the user's labels. A blank colour keeps the existing one.
#[server(input = server_fn::codec::Json)]
pub async fn update_note_label(id: i64, name: String, color: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        if !owns_label(&pool, &username, id).await? {
            return Err(ServerFnError::new("Not authorized for this label"));
        }
        let trimmed = name.trim();
        if trimmed.is_empty() {
            return Err(ServerFnError::new("Label name is required"));
        }
        let collision: Option<i64> = sqlx::query_scalar(
            "SELECT id FROM note_labels
             WHERE owner_username = ? AND LOWER(name) = ? AND id != ?",
        )
        .bind(&username)
        .bind(trimmed.to_lowercase())
        .bind(id)
        .fetch_optional(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        if collision.is_some() {
            return Err(ServerFnError::new(
                "You already have a label with that name",
            ));
        }

        let resolved_color = if color.trim().is_empty() {
            let existing: Option<String> =
                sqlx::query_scalar("SELECT color FROM note_labels WHERE id = ?")
                    .bind(id)
                    .fetch_optional(&pool)
                    .await
                    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            existing.unwrap_or_else(|| label_color_for_name(trimmed).to_string())
        } else {
            color.trim().to_string()
        };

        sqlx::query("UPDATE note_labels SET name = ?, color = ? WHERE id = ?")
            .bind(trimmed)
            .bind(&resolved_color)
            .bind(id)
            .execute(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Delete one of the current user's labels. Its `note_label_links` cascade, so
/// affected notes simply lose that label (backlog #114).
#[server(input = server_fn::codec::Json)]
pub async fn delete_note_label(id: i64) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        if !owns_label(&pool, &username, id).await? {
            return Err(ServerFnError::new("Not authorized for this label"));
        }
        sqlx::query("DELETE FROM note_labels WHERE id = ?")
            .bind(id)
            .execute(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Max raw (decoded) bytes per note image (backlog #110) — mirrors the
/// contact-photo cap (`api/contacts.rs::MAX_PHOTO_BYTES`, #86). The web-UI
/// upload path never goes through a vCard parser, so it needs its own check.
#[cfg(feature = "ssr")]
const MAX_NOTE_IMAGE_BYTES: usize = 5 * 1024 * 1024;

/// Content types accepted on the note-image upload path (backlog #110). A
/// superset of #86's contact-photo allowlist — GIF/WebP are allowed (and may
/// animate) here.
#[cfg(feature = "ssr")]
const NOTE_IMAGE_CONTENT_TYPES: &[&str] = &["image/jpeg", "image/png", "image/gif", "image/webp"];

/// Validates a note image's `content_type` and raw decoded bytes against the
/// allowlist and size cap. Split out from `decode_and_validate_note_image` so
/// the Google Keep importer (`api/keep.rs`, #109), which already holds raw
/// attachment bytes from the Takeout zip, can apply the identical rules
/// without a base64 round trip.
#[cfg(feature = "ssr")]
pub(crate) fn validate_note_image_bytes(
    bytes: &[u8],
    content_type: &str,
) -> Result<(), ServerFnError> {
    if !NOTE_IMAGE_CONTENT_TYPES.contains(&content_type) {
        return Err(ServerFnError::new("Unsupported image type"));
    }
    if bytes.is_empty() {
        return Err(ServerFnError::new("Image is empty"));
    }
    if bytes.len() > MAX_NOTE_IMAGE_BYTES {
        return Err(ServerFnError::new("Image is too large (max 5 MiB)"));
    }
    Ok(())
}

/// Validates a note image's `content_type` and decodes+size-checks its base64
/// payload. Separate from [`save_note`] so it's testable without the
/// `#[server]` macro's request-context requirement (mirrors
/// `api/contacts.rs::decode_and_validate_photo`).
#[cfg(feature = "ssr")]
fn decode_and_validate_note_image(
    base64_data: &str,
    content_type: &str,
) -> Result<Vec<u8>, ServerFnError> {
    if !NOTE_IMAGE_CONTENT_TYPES.contains(&content_type) {
        return Err(ServerFnError::new("Unsupported image type"));
    }

    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_data.trim())
        .map_err(|_| ServerFnError::new("Invalid image data"))?;
    validate_note_image_bytes(&bytes, content_type)?;
    Ok(bytes)
}

/// Inserts one image row for `note_id`, appended after the existing ones.
/// The single insert path for note images, so a future Keep importer (#109)
/// can hand parsed attachment bytes in without a browser round trip.
#[cfg(feature = "ssr")]
pub(crate) async fn insert_note_image(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    note_id: i64,
    bytes: &[u8],
    content_type: &str,
    filename: Option<&str>,
) -> Result<i64, ServerFnError> {
    let position: i64 = sqlx::query_scalar(
        "SELECT COALESCE(MAX(position), -1) + 1 FROM note_images WHERE note_id = ?",
    )
    .bind(note_id)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let id: i64 = sqlx::query(
        "INSERT INTO note_images (note_id, data, content_type, filename, position)
         VALUES (?, ?, ?, ?, ?) RETURNING id",
    )
    .bind(note_id)
    .bind(bytes)
    .bind(content_type)
    .bind(filename)
    .bind(position)
    .fetch_one(&mut **tx)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
    .try_get("id")
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    Ok(id)
}

/// Deletes `note_images` rows for `note_id` whose `@img:<id>` token no longer
/// appears in `text` (the user removed that image in the editor).
#[cfg(feature = "ssr")]
async fn prune_orphan_note_images(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    note_id: i64,
    text: &str,
) -> Result<(), ServerFnError> {
    let keep = note_image_ids_in_text(text);
    let existing: Vec<i64> = sqlx::query_scalar("SELECT id FROM note_images WHERE note_id = ?")
        .bind(note_id)
        .fetch_all(&mut **tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    for image_id in existing {
        if !keep.contains(&image_id) {
            sqlx::query("DELETE FROM note_images WHERE id = ?")
                .bind(image_id)
                .execute(&mut **tx)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        }
    }
    Ok(())
}

/// Create (`id: None`) or update a note together with its inline images,
/// atomically (backlog #110). Staged images arrive base64-encoded with a
/// `temp_key`; their `@img:t<key>` placeholders in `input.text` are rewritten
/// to the real `@img:<id>` tokens, and any image row whose token the user
/// removed from the body is pruned. Returns the note id.
#[server(input = server_fn::codec::Json)]
pub async fn save_note(
    id: Option<i64>,
    input: NewNote,
    new_images: Vec<NewNoteImage>,
) -> Result<i64, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        if input.text.trim().is_empty() {
            return Err(ServerFnError::new("Note text is required"));
        }

        // Decode/validate every new image before opening the transaction, so
        // a bad payload never leaves a half-written note behind.
        let mut decoded: Vec<(String, Vec<u8>, String, Option<String>)> =
            Vec::with_capacity(new_images.len());
        for img in &new_images {
            let bytes = decode_and_validate_note_image(&img.base64_data, &img.content_type)?;
            decoded.push((
                img.temp_key.clone(),
                bytes,
                img.content_type.clone(),
                img.filename.clone(),
            ));
        }

        let mut tx = pool
            .begin()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let note_id = match id {
            Some(note_id) => {
                let state = note_edit_state(&pool, &username, note_id, input.collection_id).await?;

                // Images first (the note id is already known), then rewrite
                // the temp tokens to the real ids and persist the text.
                let mut map = Vec::with_capacity(decoded.len());
                for (key, bytes, content_type, filename) in &decoded {
                    let image_id = insert_note_image(
                        &mut tx,
                        note_id,
                        bytes,
                        content_type,
                        filename.as_deref(),
                    )
                    .await?;
                    map.push((key.clone(), image_id));
                }

                let mut input = input.clone();
                input.text = rewrite_note_image_temp_tokens(input.text.trim(), &map);
                update_note_row(
                    &mut tx,
                    &username,
                    note_id,
                    &input,
                    state.existing_pinned,
                    state.existing_span,
                )
                .await?;
                prune_orphan_note_images(&mut tx, note_id, &input.text).await?;
                note_id
            }
            None => {
                if !has_collection_access(&pool, &username, input.collection_id).await? {
                    return Err(ServerFnError::new("Not authorized for this collection"));
                }

                let note_id = insert_note_row(&mut tx, &username, &input).await?;
                let mut map = Vec::with_capacity(decoded.len());
                for (key, bytes, content_type, filename) in &decoded {
                    let image_id = insert_note_image(
                        &mut tx,
                        note_id,
                        bytes,
                        content_type,
                        filename.as_deref(),
                    )
                    .await?;
                    map.push((key.clone(), image_id));
                }

                let final_text = rewrite_note_image_temp_tokens(input.text.trim(), &map);
                sqlx::query("UPDATE notes SET text = ?, updated_at = datetime('now') WHERE id = ?")
                    .bind(&final_text)
                    .bind(note_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

                // `insert_note_row` already synced tokens against the
                // temp-token text; re-sync now that the real image ids are
                // known so a task/await event's CalDAV DESCRIPTION carries
                // `@img:<id>`, not the transient `@img:t<key>`.
                sync_note_tokens(
                    &mut tx,
                    &username,
                    note_id,
                    input.calendar_name.as_deref(),
                    &final_text,
                )
                .await?;

                prune_orphan_note_images(&mut tx, note_id, &final_text).await?;
                note_id
            }
        };

        tx.commit()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        return Ok(note_id);
    }
    #[allow(unreachable_code)]
    Ok(0)
}

/// Body of [`delete_note`], factored out so [`clear_notes`] can loop it
/// per note — deleting a note and its mirrored VTODO (if any) always needs
/// the same access check, calendar cleanup, and sync-token bump, whether
/// it's one note from the editor modal or a batch from "Clear all".
#[cfg(feature = "ssr")]
pub(crate) async fn delete_note_inner(
    pool: &SqlitePool,
    username: &str,
    id: i64,
) -> Result<(), ServerFnError> {
    let collection_id: Option<i64> =
        sqlx::query_scalar("SELECT collection_id FROM notes WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    let collection_id = collection_id.ok_or_else(|| ServerFnError::new("Note not found"))?;

    if !has_collection_access(pool, username, collection_id).await? {
        return Err(ServerFnError::new("Not authorized for this collection"));
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    // Deleting the note cascades to its task/await events (`events.note_id`).
    // Bump each affected calendar first so CalDAV clients resync.
    let affected: Vec<(String, String)> =
        sqlx::query("SELECT calendar, uid FROM events WHERE source = 'note' AND note_id = ?")
            .bind(id)
            .fetch_all(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
            .into_iter()
            .filter_map(|r| {
                let cal: Option<String> = r.try_get("calendar").ok();
                let uid: Option<String> = r.try_get("uid").ok();
                Some((cal?, uid?))
            })
            .collect();

    for (cal, uid) in affected {
        bump_calendar_sync_token(&mut tx, username, &cal, &uid, "deleted").await?;
    }

    sqlx::query("DELETE FROM notes WHERE id = ?")
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    Ok(())
}

/// Delete a note and its mirrored VTODO (if any), as one transaction.
#[server]
pub async fn delete_note(id: i64) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        return delete_note_inner(&pool, &username, id).await;
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Delete every note visible to the current user ("Clear all" toolbar
/// button), optionally scoped to one collection. `collection_id: None`
/// clears every collection the user owns or is a member of — the same
/// visibility scope `list_notes` uses. Loops [`delete_note_inner`] per note
/// rather than a bulk `DELETE`, so each note's mirrored VTODO (if any) and
/// its calendar's sync token get cleaned up exactly as they would from an
/// individual delete.
#[server]
pub async fn clear_notes(collection_id: Option<i64>) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        let ids: Vec<i64> = match collection_id {
            Some(cid) => {
                if !has_collection_access(&pool, &username, cid).await? {
                    return Err(ServerFnError::new("Not authorized for this collection"));
                }
                sqlx::query_scalar("SELECT id FROM notes WHERE collection_id = ?")
                    .bind(cid)
                    .fetch_all(&pool)
                    .await
                    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
            }
            None => sqlx::query_scalar(
                "SELECT n.id FROM notes n JOIN note_collections c ON c.id = n.collection_id
                 WHERE c.owner_username = ?
                    OR c.id IN (SELECT collection_id FROM note_collection_members WHERE member_username = ?)",
            )
            .bind(&username)
            .bind(&username)
            .fetch_all(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?,
        };

        for id in ids {
            delete_note_inner(&pool, &username, id).await?;
        }
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Toggle a note's pinned state. Does not touch the mirror (pin is Notes-only).
#[server]
pub async fn toggle_note_pin(id: i64, pinned: bool) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        let collection_id: Option<i64> =
            sqlx::query_scalar("SELECT collection_id FROM notes WHERE id = ?")
                .bind(id)
                .fetch_optional(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        let collection_id = collection_id.ok_or_else(|| ServerFnError::new("Note not found"))?;

        if !has_collection_access(&pool, &username, collection_id).await? {
            return Err(ServerFnError::new("Not authorized for this collection"));
        }
        let existing_span: i32 = sqlx::query_scalar("SELECT grid_row_span FROM notes WHERE id = ?")
            .bind(id)
            .fetch_one(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let mut tx = pool
            .begin()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        sqlx::query("UPDATE notes SET pinned = ?, updated_at = datetime('now') WHERE id = ?")
            .bind(pinned)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let (grid_row, grid_col) =
            place_in_first_gap(&mut tx, &username, pinned, Some(id), existing_span).await?;
        sqlx::query("UPDATE notes SET grid_row = ?, grid_col = ? WHERE id = ?")
            .bind(grid_row)
            .bind(grid_col)
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        tx.commit()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Bulk-applies grid positions computed by the client's drag-drop cascade
/// (`resolve_drop` in `notes/mod.rs`) — the dragged note plus everyone it
/// displaced to make room for it, applied together in one transaction so a
/// mid-cascade failure can't leave the grid half-updated. Every note in
/// the batch must share the same `pinned` value — enforced here,
/// server-side, mirroring #42's pinned-boundary invariant: a drag can
/// reorder within a zone but never move a note across it. Requires access
/// to every note's own collection (not just the dragged one — a crafted
/// batch could otherwise use another collection's notes as cascade
/// fodder).
#[server(input = server_fn::codec::Json)]
pub async fn move_notes(moves: Vec<GridPosition>) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        if moves.is_empty() {
            return Ok(());
        }

        // BEGIN IMMEDIATE, not the default deferred BEGIN: this transaction
        // does reads (the access-check loop below, partly via `&pool` on a
        // *different* connection) before its first write in `apply_positions`.
        // A deferred transaction only takes its read snapshot at that first
        // read — if another writer commits before we reach our own write, WAL
        // mode returns SQLITE_BUSY_SNAPSHOT immediately, bypassing
        // `busy_timeout` entirely (that handler only retries lock contention,
        // not a stale snapshot — retrying wouldn't fix it). Grabbing the
        // write lock up front makes this ordinary lock contention instead,
        // which the pool's busy_timeout does retry (backlog #83).
        let mut tx = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let mut batch_pinned: Option<bool> = None;
        for m in &moves {
            let row = sqlx::query("SELECT collection_id, pinned FROM notes WHERE id = ?")
                .bind(m.id)
                .fetch_optional(&mut *tx)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            let Some(row) = row else {
                return Err(ServerFnError::new("Note not found"));
            };
            let collection_id: i64 = row.try_get("collection_id").unwrap_or_default();
            let pinned: bool = row.try_get("pinned").unwrap_or_default();

            if !has_collection_access(&pool, &username, collection_id).await? {
                return Err(ServerFnError::new("Not authorized for this collection"));
            }
            match batch_pinned {
                None => batch_pinned = Some(pinned),
                Some(p) if p != pinned => {
                    return Err(ServerFnError::new(
                        "Cannot move notes across the pinned boundary in one batch",
                    ));
                }
                _ => {}
            }
        }

        apply_positions(&mut tx, &moves).await?;

        tx.commit()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// See `api/contacts.rs::MANUAL_ORDER_SPACING`'s doc comment -- same
/// constant, duplicated per the compilation-boundary rule (CLAUDE.md
/// §6/§8): `tidl-web` doesn't share code between its own sibling
/// server-fn modules any more than it does across crates here.
#[cfg(feature = "ssr")]
const MANUAL_ORDER_SPACING: f64 = 1000.0;

/// Reorders `id` to sit between `predecessor_id` and `successor_id` in
/// Auto mode's manual-order sequence, scoped to `id`'s own `pinned`
/// zone -- mirrors `api/contacts.rs::reorder_contact`'s backfill-on-
/// first-use semantics exactly, just partitioned by pinned/non-pinned
/// the way every other Notes pack/placement function already is.
#[server]
pub async fn reorder_note(
    id: i64,
    predecessor_id: Option<i64>,
    successor_id: Option<i64>,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        return reorder_note_inner(&pool, &username, id, predecessor_id, successor_id).await;
    }
    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(feature = "ssr")]
async fn reorder_note_inner(
    pool: &SqlitePool,
    username: &str,
    id: i64,
    predecessor_id: Option<i64>,
    successor_id: Option<i64>,
) -> Result<(), ServerFnError> {
    let collection_id: Option<i64> =
        sqlx::query_scalar("SELECT collection_id FROM notes WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    let collection_id = collection_id.ok_or_else(|| ServerFnError::new("Note not found"))?;
    if !has_collection_access(pool, username, collection_id).await? {
        return Err(ServerFnError::new("Not authorized for this collection"));
    }
    let pinned: bool = sqlx::query_scalar("SELECT pinned FROM notes WHERE id = ?")
        .bind(id)
        .fetch_one(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let mut tx = pool
        .begin()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let any_ordered: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM notes n
         JOIN note_collections c ON c.id = n.collection_id
         WHERE n.pinned = ?
           AND (c.owner_username = ? OR c.id IN (SELECT collection_id FROM note_collection_members WHERE member_username = ?))
           AND n.manual_order IS NOT NULL
         LIMIT 1",
    )
    .bind(pinned)
    .bind(username)
    .bind(username)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    if any_ordered.is_none() {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT n.id FROM notes n
             JOIN note_collections c ON c.id = n.collection_id
             WHERE n.pinned = ?
               AND (c.owner_username = ? OR c.id IN (SELECT collection_id FROM note_collection_members WHERE member_username = ?))
             ORDER BY n.grid_row ASC, n.grid_col ASC",
        )
        .bind(pinned)
        .bind(username)
        .bind(username)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        for (i, nid) in ids.iter().enumerate() {
            sqlx::query("UPDATE notes SET manual_order = ? WHERE id = ?")
                .bind((i as f64 + 1.0) * MANUAL_ORDER_SPACING)
                .bind(nid)
                .execute(&mut *tx)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        }
    }

    // Scoped by the same owner-or-member access join as everywhere else
    // in this file, plus the matching `pinned` zone -- an inaccessible
    // neighbor id (another user's private note) or one from the wrong
    // zone must be treated as absent (falls through to extrapolation),
    // never have its manual_order read and used to compute the dragged
    // note's own value. A bare `WHERE id = ?` here would silently leak
    // a cross-user float, and use it to place this caller's own note
    // relative to it.
    let predecessor_order: Option<f64> = match predecessor_id {
        Some(nid) => sqlx::query_scalar(
            "SELECT n.manual_order FROM notes n
             JOIN note_collections c ON c.id = n.collection_id
             WHERE n.id = ? AND n.pinned = ?
                AND (c.owner_username = ? OR c.id IN (SELECT collection_id FROM note_collection_members WHERE member_username = ?))",
        )
        .bind(nid)
        .bind(pinned)
        .bind(username)
        .bind(username)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
        .flatten(),
        None => None,
    };
    let successor_order: Option<f64> = match successor_id {
        Some(nid) => sqlx::query_scalar(
            "SELECT n.manual_order FROM notes n
             JOIN note_collections c ON c.id = n.collection_id
             WHERE n.id = ? AND n.pinned = ?
                AND (c.owner_username = ? OR c.id IN (SELECT collection_id FROM note_collection_members WHERE member_username = ?))",
        )
        .bind(nid)
        .bind(pinned)
        .bind(username)
        .bind(username)
        .fetch_optional(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
        .flatten(),
        None => None,
    };

    let new_order = match (predecessor_order, successor_order) {
        (Some(p), Some(s)) => (p + s) / 2.0,
        (Some(p), None) => p + MANUAL_ORDER_SPACING,
        (None, Some(s)) => s / 2.0,
        (None, None) => MANUAL_ORDER_SPACING,
    };

    sqlx::query("UPDATE notes SET manual_order = ? WHERE id = ?")
        .bind(new_order)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    tx.commit()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    Ok(())
}

/// Re-opens a completed inline task/await from its note card: rewrites the
/// `#done/...` token at the uid's `(line, ordinal)` back to its original
/// `#task`/`#await` form and re-syncs, recreating the event (backlog #113).
/// A no-op if the token is no longer a `#done` record.
#[server(input = server_fn::codec::Json)]
pub async fn reopen_note_token(uid: String) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::note_tokens::{reopen_token, scan_note_tokens, ScannedTokenKind};
        use sqlx::Row;

        let (pool, username) = auth().await?;

        // uid = note:<note_id>:<line>:<ordinal>
        let parts: Vec<&str> = uid.split(':').collect();
        let note_id: i64 = parts
            .get(1)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| ServerFnError::new("Invalid token uid"))?;
        let line: usize = parts
            .get(2)
            .and_then(|s| s.parse().ok())
            .ok_or_else(|| ServerFnError::new("Invalid token uid"))?;
        let ordinal: usize = parts.get(3).and_then(|s| s.parse().ok()).unwrap_or(0);

        let collection_id: Option<i64> =
            sqlx::query_scalar("SELECT collection_id FROM notes WHERE id = ?")
                .bind(note_id)
                .fetch_optional(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        let collection_id = collection_id.ok_or_else(|| ServerFnError::new("Note not found"))?;
        if !has_collection_access(&pool, &username, collection_id).await? {
            return Err(ServerFnError::new("Not authorized for this collection"));
        }

        let row = sqlx::query("SELECT text, calendar_name FROM notes WHERE id = ?")
            .bind(note_id)
            .fetch_one(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        let text: String = row.try_get("text").unwrap_or_default();
        let calendar: Option<String> = row.try_get("calendar_name").unwrap_or_default();

        let Some(target) = scan_note_tokens(&text)
            .into_iter()
            .filter(|t| t.line == line)
            .nth(ordinal)
        else {
            return Ok(());
        };
        let ScannedTokenKind::Done { original, slug, .. } = &target.kind else {
            return Ok(());
        };
        let replacement = reopen_token(original, slug);

        let mut new_text = String::with_capacity(text.len());
        new_text.push_str(&text[..target.start]);
        new_text.push_str(&replacement);
        new_text.push_str(&text[target.end..]);

        let mut tx = pool
            .begin()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        sqlx::query("UPDATE notes SET text = ?, updated_at = datetime('now') WHERE id = ?")
            .bind(&new_text)
            .bind(note_id)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        sync_note_tokens(&mut tx, &username, note_id, calendar.as_deref(), &new_text).await?;
        tx.commit()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        let _ = crate::api::done::forget_done_item(&pool, &username, "note", &uid).await;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// One-time, idempotent migration of the pre-#112 single-task-per-note model:
/// every note with a `kind` gets an inline token appended to its body, its old
/// mirrored VTODO removed, and `kind`/`priority`/`due_date`/
/// `mirrored_event_uid` cleared. Called at server startup after migrations; a
/// no-op once every note has `kind IS NULL`.
#[cfg(feature = "ssr")]
pub async fn backfill_note_kind_tokens(pool: &SqlitePool) -> Result<(), ServerFnError> {
    use crate::note_tokens::{format_await_token, format_task_token, slugify};
    use chrono::NaiveDate;
    use sqlx::Row;

    let rows = sqlx::query(
        "SELECT id, author_username, title, text, kind, priority, calendar_name,
                mirrored_event_uid, CAST(due_date AS TEXT) AS due_date
         FROM notes WHERE kind IS NOT NULL",
    )
    .fetch_all(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    for r in rows {
        let id: i64 = r.try_get("id").unwrap_or_default();
        let username: String = r.try_get("author_username").unwrap_or_default();
        let text: String = r.try_get("text").unwrap_or_default();
        let title: Option<String> = r.try_get("title").unwrap_or_default();
        let kind: String = r.try_get("kind").unwrap_or_default();
        let priority: Option<i64> = r.try_get("priority").unwrap_or_default();
        let calendar: Option<String> = r.try_get("calendar_name").unwrap_or_default();
        let old_uid: Option<String> = r.try_get("mirrored_event_uid").unwrap_or_default();
        let due: Option<String> = r.try_get("due_date").unwrap_or_default();

        let mut slug = slugify(title.as_deref().unwrap_or("").trim());
        if slug.is_empty() {
            let first = text.lines().find(|l| !l.trim().is_empty()).unwrap_or("");
            slug = slugify(first);
        }
        if slug.is_empty() {
            slug = format!("task-{id}");
        }

        let token = if kind == "await" {
            match due
                .as_deref()
                .and_then(|d| NaiveDate::parse_from_str(d.get(..10).unwrap_or(d), "%Y-%m-%d").ok())
            {
                Some(date) => format_await_token(date, &slug),
                None => continue,
            }
        } else {
            format_task_token(priority.unwrap_or(50).clamp(1, 100) as u32, &slug)
        };

        let new_text = if text.trim().is_empty() {
            token
        } else {
            format!("{}\n\n{token}", text.trim())
        };

        let mut tx = pool
            .begin()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        if let Some(uid) = old_uid {
            let cal: Option<String> =
                sqlx::query_scalar("SELECT calendar FROM events WHERE uid = ? AND username = ?")
                    .bind(&uid)
                    .bind(&username)
                    .fetch_optional(&mut *tx)
                    .await
                    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            sqlx::query("DELETE FROM events WHERE uid = ? AND username = ?")
                .bind(&uid)
                .bind(&username)
                .execute(&mut *tx)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            if let Some(cal) = cal {
                bump_calendar_sync_token(&mut tx, &username, &cal, &uid, "deleted").await?;
            }
        }

        sqlx::query(
            "UPDATE notes SET text = ?, kind = NULL, priority = NULL, due_date = NULL,
                    mirrored_event_uid = NULL, updated_at = datetime('now')
             WHERE id = ?",
        )
        .bind(&new_text)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        sync_note_tokens(&mut tx, &username, id, calendar.as_deref(), &new_text).await?;

        tx.commit()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::time::Duration;

    #[test]
    fn label_color_for_name_is_stable_and_in_palette() {
        let color = label_color_for_name("Work");
        assert!(LABEL_PALETTE.contains(&color));
        assert_eq!(color, label_color_for_name("Work"));
    }

    /// A real on-disk WAL-mode pool, mirroring
    /// `tidl-server::storage::Storage::new`'s connect options — WAL requires
    /// a real file (`:memory:` databases silently keep the default rollback
    /// journal), and backlog #83's race only reproduces under WAL, so
    /// `#[sqlx::test]`'s own pool (no WAL) can't be used for it.
    async fn wal_pool_at(path: &std::path::Path, busy_timeout: Duration) -> SqlitePool {
        use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode};
        let options = SqliteConnectOptions::new()
            .filename(path)
            .create_if_missing(true)
            .journal_mode(SqliteJournalMode::Wal)
            .busy_timeout(busy_timeout);
        let pool = SqlitePool::connect_with(options).await.unwrap();
        sqlx::migrate!("../../migrations").run(&pool).await.unwrap();
        pool
    }

    fn unique_db_path(tag: &str) -> std::path::PathBuf {
        use std::sync::atomic::{AtomicU64, Ordering};
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "tidl-backlog83-{tag}-{}-{n}.db",
            std::process::id()
        ))
    }

    fn cleanup_wal_db(path: &std::path::Path) {
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(path.with_extension("db-wal"));
        let _ = std::fs::remove_file(path.with_extension("db-shm"));
    }

    async fn seed_alice_with_two_notes(pool: &SqlitePool) -> (i64, i64) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Inbox', '#000000') RETURNING id"
        ).fetch_one(pool).await.unwrap();
        let note_a: i64 = sqlx::query_scalar(
            "INSERT INTO notes (collection_id, author_username, text, pinned) VALUES (?, 'alice', 'A', 0) RETURNING id"
        ).bind(collection_id).fetch_one(pool).await.unwrap();
        let note_b: i64 = sqlx::query_scalar(
            "INSERT INTO notes (collection_id, author_username, text, pinned) VALUES (?, 'alice', 'B', 0) RETURNING id"
        ).bind(collection_id).fetch_one(pool).await.unwrap();
        (note_a, note_b)
    }

    /// Pins backlog #83's root cause. A plain `BEGIN` (deferred) transaction
    /// takes its read snapshot at its *first read*, not at `BEGIN`. If it
    /// reads, then a different connection commits a write, then this
    /// transaction tries to write, WAL mode returns SQLITE_BUSY_SNAPSHOT —
    /// and unlike ordinary lock contention, the busy-handler installed by
    /// `busy_timeout` does not retry a stale snapshot (retrying would just
    /// see the same stale view), so it fails immediately instead of waiting
    /// out the timeout. This is exactly the shape `move_notes`/`move_contacts`
    /// had: read (the per-note access-check loop, partly via a *second*
    /// connection) before the first write in `apply_positions`.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn deferred_begin_fails_instantly_when_a_concurrent_writer_advances_the_wal_snapshot() {
        let path = unique_db_path("deferred");
        let pool = wal_pool_at(&path, Duration::from_secs(2)).await;
        let (note_a, note_b) = seed_alice_with_two_notes(&pool).await;

        let a_has_read = Arc::new(tokio::sync::Notify::new());
        let b_has_committed = Arc::new(tokio::sync::Notify::new());

        let task_b = {
            let pool = pool.clone();
            let a_has_read = a_has_read.clone();
            let b_has_committed = b_has_committed.clone();
            tokio::spawn(async move {
                a_has_read.notified().await;
                let mut tx = pool.begin().await.unwrap();
                let _: i32 = sqlx::query_scalar("SELECT grid_row FROM notes WHERE id = ?")
                    .bind(note_b)
                    .fetch_one(&mut *tx)
                    .await
                    .unwrap();
                apply_positions(
                    &mut tx,
                    &[GridPosition {
                        id: note_b,
                        grid_row: 1,
                        grid_col: 0,
                        grid_row_span: 1,
                    }],
                )
                .await
                .unwrap();
                tx.commit().await.unwrap();
                b_has_committed.notify_one();
            })
        };

        let mut tx_a = pool.begin().await.unwrap();
        let _: i32 = sqlx::query_scalar("SELECT grid_row FROM notes WHERE id = ?")
            .bind(note_a)
            .fetch_one(&mut *tx_a)
            .await
            .unwrap();
        a_has_read.notify_one();
        b_has_committed.notified().await;

        let start = std::time::Instant::now();
        let result = apply_positions(
            &mut tx_a,
            &[GridPosition {
                id: note_a,
                grid_row: 1,
                grid_col: 0,
                grid_row_span: 1,
            }],
        )
        .await;
        let elapsed = start.elapsed();
        task_b.await.unwrap();

        let err = result.expect_err(
            "task A's deferred-transaction write must fail: its read snapshot predates task B's commit",
        );
        assert!(
            err.to_string().contains("locked"),
            "expected a database-locked error, got: {err}"
        );
        assert!(
            elapsed < Duration::from_millis(500),
            "must fail near-instantly (SQLITE_BUSY_SNAPSHOT isn't retried by busy_timeout), not after waiting it out — took {elapsed:?}"
        );

        cleanup_wal_db(&path);
    }

    /// Validates the fix: `BEGIN IMMEDIATE` (what `move_notes`/`move_contacts`
    /// use now) grabs the write lock at `BEGIN` time, before any reads — so
    /// two overlapping move transactions become ordinary lock contention,
    /// which `busy_timeout` *does* retry, instead of the instant
    /// SQLITE_BUSY_SNAPSHOT reproduced above.
    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn immediate_begin_serializes_concurrent_note_moves_instead_of_erroring() {
        let path = unique_db_path("immediate");
        let pool = wal_pool_at(&path, Duration::from_secs(2)).await;
        let (note_a, note_b) = seed_alice_with_two_notes(&pool).await;

        async fn move_one(pool: SqlitePool, note_id: i64) -> Result<(), ServerFnError> {
            let mut tx = pool.begin_with("BEGIN IMMEDIATE").await.unwrap();
            let _: i32 = sqlx::query_scalar("SELECT grid_row FROM notes WHERE id = ?")
                .bind(note_id)
                .fetch_one(&mut *tx)
                .await
                .unwrap();
            // Simulate the real handler doing more work (the access-check
            // loop, over however many notes are in the batch) while holding
            // the write lock, so the other task's own BEGIN IMMEDIATE really
            // does have to wait rather than never overlapping at all.
            tokio::time::sleep(Duration::from_millis(80)).await;
            apply_positions(
                &mut tx,
                &[GridPosition {
                    id: note_id,
                    grid_row: 1,
                    grid_col: 0,
                    grid_row_span: 1,
                }],
            )
            .await?;
            tx.commit().await.unwrap();
            Ok(())
        }

        let task_a = tokio::spawn(move_one(pool.clone(), note_a));
        let task_b = tokio::spawn(move_one(pool.clone(), note_b));
        task_a.await.unwrap().expect("first mover must succeed");
        task_b
            .await
            .unwrap()
            .expect("second mover must wait for the lock, then succeed, not error");

        cleanup_wal_db(&path);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_notes_exposes_manual_order(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Notes', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO notes (author_username, collection_id, text, manual_order) VALUES ('alice', ?, 'Ordered', 7.0)"
        ).bind(collection_id).execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO notes (author_username, collection_id, text) VALUES ('alice', ?, 'Unordered')"
        ).bind(collection_id).execute(&pool).await.unwrap();

        let notes = list_notes_for(&pool, "alice").await.unwrap();
        let ordered = notes.iter().find(|n| n.text == "Ordered").unwrap();
        let unordered = notes.iter().find(|n| n.text == "Unordered").unwrap();
        assert_eq!(ordered.manual_order, Some(7.0));
        assert_eq!(unordered.manual_order, None);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_notes_exposes_labels_scoped_to_visible_notes(pool: sqlx::SqlitePool) {
        for who in ["alice", "bob"] {
            sqlx::query(
                "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES (?, ?, ?, 'x', 0)",
            )
            .bind(who)
            .bind(format!("{who}@example.com"))
            .bind(who)
            .execute(&pool)
            .await
            .unwrap();
        }
        let alice_coll: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'A', '#000000') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let bob_coll: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('bob', 'B', '#000000') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        let alice_note: i64 = sqlx::query_scalar(
            "INSERT INTO notes (author_username, collection_id, text) VALUES ('alice', ?, 'A note') RETURNING id",
        )
        .bind(alice_coll)
        .fetch_one(&pool)
        .await
        .unwrap();
        let bob_note: i64 = sqlx::query_scalar(
            "INSERT INTO notes (author_username, collection_id, text) VALUES ('bob', ?, 'B note') RETURNING id",
        )
        .bind(bob_coll)
        .fetch_one(&pool)
        .await
        .unwrap();

        let work: i64 = sqlx::query_scalar(
            "INSERT INTO note_labels (owner_username, name, color) VALUES ('alice', 'Work', '#111111') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO note_labels (owner_username, name, color) VALUES ('bob', 'Private', '#222222')",
        )
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query("INSERT INTO note_label_links (note_id, label_id) VALUES (?, ?)")
            .bind(alice_note)
            .bind(work)
            .execute(&pool)
            .await
            .unwrap();

        let notes = list_notes_for(&pool, "alice").await.unwrap();
        let alice_loaded = notes.iter().find(|n| n.id == alice_note).unwrap();
        assert_eq!(alice_loaded.labels.len(), 1);
        assert_eq!(alice_loaded.labels[0].name, "Work");
        assert_eq!(alice_loaded.labels[0].color, "#111111");
        // Bob's note (and its label) is invisible to Alice.
        assert!(notes.iter().all(|n| n.id != bob_note));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn set_note_labels_dedupes_and_reuses_case_insensitively(pool: sqlx::SqlitePool) {
        let note_id = seed_user_collection_note(&pool, "body").await;

        let mut tx = pool.begin().await.unwrap();
        let written = set_note_labels(
            &mut tx,
            note_id,
            &[
                "Work".to_string(),
                "work".to_string(),
                "   ".to_string(),
                "Home".to_string(),
            ],
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();
        assert_eq!(written, 2);

        let names: Vec<String> = sqlx::query_scalar(
            "SELECT name FROM note_labels WHERE owner_username = 'alice' ORDER BY name",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(names, vec!["Home".to_string(), "Work".to_string()]);

        // Re-saving with a case-different set reuses the same rows and rewrites
        // (not appends) the links.
        let mut tx = pool.begin().await.unwrap();
        set_note_labels(&mut tx, note_id, &["work".to_string(), "home".to_string()])
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let label_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM note_labels WHERE owner_username = 'alice'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(label_count, 2);
        let link_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM note_label_links WHERE note_id = ?")
                .bind(note_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(link_count, 2);

        // An empty set clears the links but leaves the labels themselves.
        let mut tx = pool.begin().await.unwrap();
        set_note_labels(&mut tx, note_id, &[]).await.unwrap();
        tx.commit().await.unwrap();
        let link_count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM note_label_links WHERE note_id = ?")
                .bind(note_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(link_count, 0);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn existing_note_collection_members_is_empty_for_a_fresh_collection(
        pool: sqlx::SqlitePool,
    ) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Recipes', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        let existing = existing_note_collection_members(&pool, id).await.unwrap();
        assert!(existing.is_empty());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn existing_note_collection_members_returns_confirmed_members_only(
        pool: sqlx::SqlitePool,
    ) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('bob', 'bob@example.com', 'bob', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Recipes', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO note_collection_members (collection_id, member_username) VALUES (?, 'bob')"
        ).bind(id).execute(&pool).await.unwrap();

        let existing = existing_note_collection_members(&pool, id).await.unwrap();
        assert_eq!(
            existing,
            std::collections::HashSet::from(["bob".to_string()])
        );
    }

    async fn seed_user_collection_note(pool: &sqlx::SqlitePool, text: &str) -> i64 {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Inbox', '#000000') RETURNING id"
        ).fetch_one(pool).await.unwrap();
        sqlx::query_scalar(
            "INSERT INTO notes (collection_id, author_username, text) VALUES (?, 'alice', ?) RETURNING id",
        )
        .bind(collection_id)
        .bind(text)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn sync_note_tokens_creates_one_event_per_token(pool: sqlx::SqlitePool) {
        let note_id =
            seed_user_collection_note(&pool, "#task/p10/buy-milk and #await/2026-12-31/renew")
                .await;
        let text = "#task/p10/buy-milk and #await/2026-12-31/renew";
        let mut conn = pool.acquire().await.unwrap();
        sync_note_tokens(&mut conn, "alice", note_id, Some("personal"), text)
            .await
            .unwrap();

        let rows = sqlx::query(
            "SELECT uid, topic, kind, priority, CAST(due_date AS TEXT) AS due
             FROM events WHERE source = 'note' ORDER BY uid",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        // Both tokens are on line 1; uid sorts by ordinal: task(0) then await(1).
        assert_eq!(
            rows[0].get::<String, _>("uid"),
            format!("note:{note_id}:1:0")
        );
        assert_eq!(rows[0].get::<String, _>("kind"), "task");
        assert_eq!(rows[0].get::<Option<i64>, _>("priority"), Some(10));
        assert_eq!(rows[0].get::<String, _>("topic"), "buy-milk");
        assert_eq!(
            rows[1].get::<String, _>("uid"),
            format!("note:{note_id}:1:1")
        );
        assert_eq!(rows[1].get::<String, _>("kind"), "await");
        assert_eq!(rows[1].get::<Option<i64>, _>("priority"), None);
        assert_eq!(rows[1].get::<String, _>("topic"), "renew");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn sync_note_tokens_replaces_events_on_resave(pool: sqlx::SqlitePool) {
        let note_id = seed_user_collection_note(&pool, "").await;
        let mut conn = pool.acquire().await.unwrap();
        sync_note_tokens(
            &mut conn,
            "alice",
            note_id,
            None,
            "#task/p1/first\n#task/p2/second",
        )
        .await
        .unwrap();
        assert_eq!(count_note_events(&pool, note_id).await, 2);

        // Removing the first token drops its event; the second remains.
        sync_note_tokens(&mut conn, "alice", note_id, None, "#task/p2/second")
            .await
            .unwrap();
        let topics: Vec<String> =
            sqlx::query_scalar("SELECT topic FROM events WHERE source='note' AND note_id = ?")
                .bind(note_id)
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(topics, vec!["second".to_string()]);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn sync_note_tokens_ignores_done_tokens(pool: sqlx::SqlitePool) {
        let note_id = seed_user_collection_note(&pool, "").await;
        let mut conn = pool.acquire().await.unwrap();
        sync_note_tokens(
            &mut conn,
            "alice",
            note_id,
            None,
            "#done/2026-10-01/p1/buy-milk\n#done/2026-10-01/2026-12-31/renew",
        )
        .await
        .unwrap();
        assert_eq!(count_note_events(&pool, note_id).await, 0);
    }

    /// Regression test: `storage::list_events`/`list_todos` (CalDAV listing,
    /// calendar `.ics` export) decode `events.due_date` straight into
    /// `chrono::DateTime<Utc>`. A bare `YYYY-MM-DD` — what an await token's date
    /// looks like — used to fail that decode with "invalid datetime".
    #[sqlx::test(migrations = "../../migrations")]
    async fn sync_note_tokens_await_due_date_decodes_as_datetime(pool: sqlx::SqlitePool) {
        let note_id = seed_user_collection_note(&pool, "").await;
        let mut conn = pool.acquire().await.unwrap();
        sync_note_tokens(&mut conn, "alice", note_id, None, "#await/2026-09-21/reply")
            .await
            .unwrap();

        let due_date: chrono::DateTime<chrono::Utc> =
            sqlx::query_scalar("SELECT due_date FROM events WHERE username = 'alice'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(due_date.format("%Y-%m-%d").to_string(), "2026-09-21");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn sync_note_tokens_caps_priority_and_titles_from_slug(pool: sqlx::SqlitePool) {
        let note_id = seed_user_collection_note(&pool, "").await;
        let mut conn = pool.acquire().await.unwrap();
        sync_note_tokens(
            &mut conn,
            "alice",
            note_id,
            None,
            "#task/p9999/critical-bug",
        )
        .await
        .unwrap();
        let row = sqlx::query("SELECT summary, priority FROM events WHERE source='note'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(row.get::<String, _>("summary"), "Critical Bug");
        assert_eq!(row.get::<i64, _>("priority"), 100);
    }

    async fn count_note_events(pool: &sqlx::SqlitePool, note_id: i64) -> i64 {
        sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE source='note' AND note_id = ?")
            .bind(note_id)
            .fetch_one(pool)
            .await
            .unwrap()
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn backfill_converts_note_kind_into_a_token(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Inbox', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let note_id: i64 = sqlx::query_scalar(
            "INSERT INTO notes (collection_id, author_username, title, text, kind, priority, calendar_name)
             VALUES (?, 'alice', 'Buy milk', 'and eggs', 'task', 42, 'personal') RETURNING id"
        ).bind(collection_id).fetch_one(&pool).await.unwrap();

        backfill_note_kind_tokens(&pool).await.unwrap();

        let (text, kind): (String, Option<String>) =
            sqlx::query_as("SELECT text, kind FROM notes WHERE id = ?")
                .bind(note_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(text, "and eggs\n\n#task/p42/buy-milk");
        assert_eq!(kind, None);
        assert_eq!(count_note_events(&pool, note_id).await, 1);

        // Idempotent: a second run changes nothing.
        backfill_note_kind_tokens(&pool).await.unwrap();
        assert_eq!(count_note_events(&pool, note_id).await, 1);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn backfill_maps_await_to_a_dated_token(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Inbox', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let note_id: i64 = sqlx::query_scalar(
            "INSERT INTO notes (collection_id, author_username, title, text, kind, due_date)
             VALUES (?, 'alice', 'Renew passport', '', 'await', '2026-12-31') RETURNING id",
        )
        .bind(collection_id)
        .fetch_one(&pool)
        .await
        .unwrap();

        backfill_note_kind_tokens(&pool).await.unwrap();

        let text: String = sqlx::query_scalar("SELECT text FROM notes WHERE id = ?")
            .bind(note_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(text, "#await/2026-12-31/renew-passport");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn rewrite_note_body_updates_text_and_leaves_other_fields_untouched(
        pool: sqlx::SqlitePool,
    ) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Inbox', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let note_id: i64 = sqlx::query_scalar(
            "INSERT INTO notes (collection_id, author_username, text, pinned) VALUES (?, 'alice', 'Ping @Jane Doe', 1) RETURNING id"
        ).bind(collection_id).fetch_one(&pool).await.unwrap();

        let mut tx = pool.begin().await.unwrap();
        rewrite_note_body(&mut tx, "alice", note_id, "Ping @Jane Smith".to_string())
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let (text, pinned): (String, bool) =
            sqlx::query_as("SELECT text, pinned FROM notes WHERE id = ?")
                .bind(note_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(text, "Ping @Jane Smith");
        assert!(pinned, "unrelated fields must be untouched");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn rewrite_note_body_resyncs_inline_task_events(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Inbox', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let note_id: i64 = sqlx::query_scalar(
            "INSERT INTO notes (collection_id, author_username, text, calendar_name)
             VALUES (?, 'alice', 'Ping @Jane Doe #task/p50/jane-followup', 'personal') RETURNING id"
        ).bind(collection_id).fetch_one(&pool).await.unwrap();

        let mut tx = pool.begin().await.unwrap();
        rewrite_note_body(
            &mut tx,
            "alice",
            note_id,
            "Ping @Jane Smith #task/p50/jane-followup".to_string(),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        let ical: String = sqlx::query_scalar(
            "SELECT ical_data FROM events WHERE username = 'alice' AND source = 'note'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert!(
            ical.contains("Jane Smith"),
            "task event's DESCRIPTION must carry the rewritten text: {ical}"
        );
    }

    #[test]
    fn pack_shortest_column_fills_the_lowest_column_first() {
        // Three 1-unit-tall entries into a 2-column grid with equal
        // floors: first goes to column 0 (tie broken by lowest index),
        // second to column 1 (now the shortest), third back to column 0.
        let placed = grid::pack_shortest_column(&[(1, 1), (2, 1), (3, 1)], 2, &[0, 0]);
        assert_eq!(placed, vec![(1, 0, 0, 1), (2, 0, 1, 1), (3, 1, 0, 1)]);
    }

    #[test]
    fn pack_shortest_column_respects_a_tall_entry() {
        // A 10-unit-tall entry in column 0, then a 1-unit entry must go
        // to column 1 (height 0) rather than stacking on top of the tall
        // one in column 0 (height 10).
        let placed = grid::pack_shortest_column(&[(1, 10), (2, 1)], 2, &[0, 0]);
        assert_eq!(placed, vec![(1, 0, 0, 10), (2, 0, 1, 1)]);
    }

    #[test]
    fn pack_shortest_column_honors_col_floors() {
        // Column 0 starts pre-floored (add-tile reservation) above
        // column 1's floor — the first entry must go to column 1.
        let placed = grid::pack_shortest_column(&[(1, 1)], 2, &[12, 0]);
        assert_eq!(placed, vec![(1, 0, 1, 1)]);
    }

    #[test]
    fn pack_shortest_column_span_passes_through_unchanged() {
        let placed = grid::pack_shortest_column(&[(1, 7)], 1, &[0]);
        assert_eq!(placed, vec![(1, 0, 0, 7)]);
    }

    async fn insert_test_note(
        pool: &sqlx::SqlitePool,
        author: &str,
        collection_id: i64,
        text: &str,
        pinned: bool,
    ) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO notes (author_username, collection_id, text, pinned) VALUES (?, ?, ?, ?) RETURNING id",
        )
        .bind(author)
        .bind(collection_id)
        .bind(text)
        .bind(pinned)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reorder_note_first_call_backfills_the_whole_zone(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Notes', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let a = insert_test_note(&pool, "alice", collection_id, "Aardvark", false).await;
        let b = insert_test_note(&pool, "alice", collection_id, "Bear", false).await;
        let c = insert_test_note(&pool, "alice", collection_id, "Camel", false).await;

        reorder_note_inner(&pool, "alice", c, Some(a), Some(b))
            .await
            .unwrap();

        let orders: Vec<(String, Option<f64>)> = sqlx::query_as(
            "SELECT text, manual_order FROM notes WHERE author_username = 'alice' ORDER BY manual_order",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(orders.len(), 3);
        assert!(orders.iter().all(|(_, o)| o.is_some()));
        let order_of = |text: &str| orders.iter().find(|(t, _)| t == text).unwrap().1.unwrap();
        assert!(order_of("Aardvark") < order_of("Camel"));
        assert!(order_of("Camel") < order_of("Bear"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reorder_note_only_backfills_the_matching_pinned_zone(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Notes', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let pinned_a = insert_test_note(&pool, "alice", collection_id, "Pinned A", true).await;
        let pinned_b = insert_test_note(&pool, "alice", collection_id, "Pinned B", true).await;
        let unpinned = insert_test_note(&pool, "alice", collection_id, "Unpinned", false).await;

        reorder_note_inner(&pool, "alice", pinned_b, Some(pinned_a), None)
            .await
            .unwrap();

        let unpinned_order: Option<f64> =
            sqlx::query_scalar("SELECT manual_order FROM notes WHERE id = ?")
                .bind(unpinned)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            unpinned_order, None,
            "reordering a pinned note must not backfill the unpinned zone"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reorder_note_second_call_does_not_rebackfill(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Notes', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let a = insert_test_note(&pool, "alice", collection_id, "Aardvark", false).await;
        let b = insert_test_note(&pool, "alice", collection_id, "Bear", false).await;

        reorder_note_inner(&pool, "alice", b, Some(a), None)
            .await
            .unwrap();
        let a_order_after_first: f64 =
            sqlx::query_scalar("SELECT manual_order FROM notes WHERE id = ?")
                .bind(a)
                .fetch_one(&pool)
                .await
                .unwrap();

        let c = insert_test_note(&pool, "alice", collection_id, "Camel", false).await;
        reorder_note_inner(&pool, "alice", c, Some(b), None)
            .await
            .unwrap();

        let a_order_after_second: f64 =
            sqlx::query_scalar("SELECT manual_order FROM notes WHERE id = ?")
                .bind(a)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(a_order_after_first, a_order_after_second);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reorder_note_the_only_card_in_its_zone_does_not_panic(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Notes', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let a = insert_test_note(&pool, "alice", collection_id, "Aardvark", false).await;

        let result = reorder_note_inner(&pool, "alice", a, None, None).await;
        assert!(result.is_ok());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reorder_note_rejects_a_non_member(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0), ('eve', 'eve@example.com', 'eve', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Notes', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let a = insert_test_note(&pool, "alice", collection_id, "Aardvark", false).await;

        let result = reorder_note_inner(&pool, "eve", a, None, None).await;
        assert!(result.is_err());
    }

    /// Found in final review: `predecessor_id`/`successor_id` weren't
    /// scope-checked -- only the dragged `id` itself was. Mirrors the
    /// equivalent fix and test in `api/contacts.rs::reorder_contact`.
    #[sqlx::test(migrations = "../../migrations")]
    async fn reorder_note_ignores_a_predecessor_outside_the_callers_access(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0), ('eve', 'eve@example.com', 'eve', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let alice_collection: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Notes', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let eve_collection: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('eve', 'Eve Private', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let alice_note =
            insert_test_note(&pool, "alice", alice_collection, "Alice's note", false).await;
        let eve_note =
            insert_test_note(&pool, "eve", eve_collection, "Eve's private note", false).await;
        sqlx::query("UPDATE notes SET manual_order = 999999.0 WHERE id = ?")
            .bind(eve_note)
            .execute(&pool)
            .await
            .unwrap();

        reorder_note_inner(&pool, "alice", alice_note, Some(eve_note), None)
            .await
            .unwrap();

        let alice_order: f64 = sqlx::query_scalar("SELECT manual_order FROM notes WHERE id = ?")
            .bind(alice_note)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(
            alice_order < 999999.0,
            "alice's note must not be ordered relative to eve's inaccessible private note's manual_order, got {alice_order}"
        );
    }

    // ── Note images (backlog #110) ──────────────────────────────────────────

    #[test]
    fn decode_and_validate_note_image_accepts_supported_types() {
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode(b"hello");
        for ct in ["image/jpeg", "image/png", "image/gif", "image/webp"] {
            assert_eq!(decode_and_validate_note_image(&data, ct).unwrap(), b"hello");
        }
    }

    #[test]
    fn decode_and_validate_note_image_rejects_unsupported_type() {
        use base64::Engine;
        let data = base64::engine::general_purpose::STANDARD.encode(b"hello");
        assert!(decode_and_validate_note_image(&data, "image/svg+xml").is_err());
    }

    #[test]
    fn decode_and_validate_note_image_rejects_empty_and_garbage() {
        assert!(decode_and_validate_note_image("", "image/png").is_err());
        assert!(decode_and_validate_note_image("not base64!!", "image/png").is_err());
    }

    #[test]
    fn decode_and_validate_note_image_rejects_over_the_cap() {
        use base64::Engine;
        let big = vec![0u8; MAX_NOTE_IMAGE_BYTES + 1];
        let data = base64::engine::general_purpose::STANDARD.encode(&big);
        let err = decode_and_validate_note_image(&data, "image/png").unwrap_err();
        assert!(err.to_string().contains("too large"), "got: {err}");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn insert_note_image_appends_with_incrementing_position(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Notes', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let note_id = insert_test_note(&pool, "alice", collection_id, "with images", false).await;

        let mut tx = pool.begin().await.unwrap();
        let first = insert_note_image(&mut tx, note_id, b"one", "image/png", Some("a.png"))
            .await
            .unwrap();
        let second = insert_note_image(&mut tx, note_id, b"two", "image/jpeg", None)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        assert!(second > first);
        let rows: Vec<(i64, String, Option<String>)> = sqlx::query_as(
            "SELECT position, content_type, filename FROM note_images WHERE note_id = ? ORDER BY position",
        )
        .bind(note_id)
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].0, 0);
        assert_eq!(rows[0].1, "image/png");
        assert_eq!(rows[0].2.as_deref(), Some("a.png"));
        assert_eq!(rows[1].0, 1);
        assert_eq!(rows[1].1, "image/jpeg");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn prune_orphan_note_images_removes_tokens_no_longer_present(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Notes', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let note_id = insert_test_note(&pool, "alice", collection_id, "with images", false).await;

        let mut tx = pool.begin().await.unwrap();
        let kept = insert_note_image(&mut tx, note_id, b"keep", "image/png", None)
            .await
            .unwrap();
        let dropped = insert_note_image(&mut tx, note_id, b"drop", "image/png", None)
            .await
            .unwrap();
        let text = format!("before @img:{kept} after");
        prune_orphan_note_images(&mut tx, note_id, &text)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let ids: Vec<i64> = sqlx::query_scalar("SELECT id FROM note_images WHERE note_id = ?")
            .bind(note_id)
            .fetch_all(&pool)
            .await
            .unwrap();
        assert_eq!(ids, vec![kept]);
        assert!(!ids.contains(&dropped));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_notes_for_includes_image_metadata(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Notes', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let note_id = insert_test_note(&pool, "alice", collection_id, "has an image", false).await;

        let mut tx = pool.begin().await.unwrap();
        insert_note_image(&mut tx, note_id, b"bytes", "image/webp", Some("x.webp"))
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let notes = list_notes_for(&pool, "alice").await.unwrap();
        let note = notes.iter().find(|n| n.id == note_id).unwrap();
        assert_eq!(note.images.len(), 1);
        assert_eq!(note.images[0].content_type, "image/webp");
        assert_eq!(note.images[0].filename.as_deref(), Some("x.webp"));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn note_images_cascade_when_the_note_is_deleted(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Notes', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let note_id = insert_test_note(&pool, "alice", collection_id, "with images", false).await;

        let mut tx = pool.begin().await.unwrap();
        insert_note_image(&mut tx, note_id, b"bytes", "image/png", None)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        sqlx::query("DELETE FROM notes WHERE id = ?")
            .bind(note_id)
            .execute(&pool)
            .await
            .unwrap();

        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM note_images WHERE note_id = ?")
            .bind(note_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 0);
    }
}
