//! Server functions for the Contacts tab (backlog #79).
//!
//! Layout (grid placement, auto-layout, bulk position moves) mirrors
//! `api/notes.rs` closely, sharing the underlying masonry math via
//! `crate::grid` — see that module's doc comment. Contacts have a single
//! flat zone (no pinning, no reserved-column chrome), so the placement
//! helpers here are simpler than Notes' pinned/non-pinned pair.

use leptos::prelude::*;
use server_fn::ServerFnError;

use crate::types::{
    ContactCollectionInfo, ContactDto, GridPosition, ImportResult, MergeContactsInput, NewContact,
    NewContactCollection, VcfImportPreview,
};

#[cfg(feature = "ssr")]
use crate::grid::{self, CARD_GRID_COLS};
#[cfg(feature = "ssr")]
use crate::types::{ContactAddress, ContactMethod};
#[cfg(feature = "ssr")]
use sqlx::{Row, SqlitePool};

#[cfg(feature = "ssr")]
use crate::api::auth;

#[cfg(feature = "ssr")]
use tidl_carddav::vcard::{self, VCardFields};

// ── Access control (SSR-only) ─────────────────────────────────────────────────

/// True if `username` may access contact `id` — owner or member of the
/// contact's collection. `contacts.owner_username` is who created the
/// contact, not the access-control boundary (backlog #81) — access always
/// resolves through `collection_id`.
#[cfg(feature = "ssr")]
async fn is_contact_owner(
    pool: &SqlitePool,
    username: &str,
    id: i64,
) -> Result<bool, ServerFnError> {
    let collection_id: Option<i64> =
        sqlx::query_scalar("SELECT collection_id FROM contacts WHERE id = ?")
            .bind(id)
            .fetch_optional(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
            .flatten();
    let Some(collection_id) = collection_id else {
        return Ok(false);
    };
    has_contact_collection_access(pool, username, collection_id).await
}

/// True if `username` owns collection `collection_id`.
#[cfg(feature = "ssr")]
async fn is_contact_collection_owner(
    pool: &SqlitePool,
    username: &str,
    collection_id: i64,
) -> Result<bool, ServerFnError> {
    let owner: Option<String> =
        sqlx::query_scalar("SELECT owner_username FROM contact_collections WHERE id = ?")
            .bind(collection_id)
            .fetch_optional(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    Ok(owner.as_deref() == Some(username))
}

/// True if `username` may read/write contacts in `collection_id` — the
/// owner, or a member of a shared collection.
#[cfg(feature = "ssr")]
async fn has_contact_collection_access(
    pool: &SqlitePool,
    username: &str,
    collection_id: i64,
) -> Result<bool, ServerFnError> {
    if is_contact_collection_owner(pool, username, collection_id).await? {
        return Ok(true);
    }
    let member: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM contact_collection_members WHERE collection_id = ? AND member_username = ?",
    )
    .bind(collection_id)
    .bind(username)
    .fetch_optional(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    Ok(member.is_some())
}

/// The set of usernames currently confirmed as members of `collection_id`
/// — mirrors `api/notes.rs::existing_note_collection_members` exactly
/// (same anti-enumeration rationale: filters client-submitted member
/// lists on save so membership can only shrink, never grow, outside an
/// accepted invite).
#[cfg(feature = "ssr")]
async fn existing_contact_collection_members<'e, E>(
    executor: E,
    collection_id: i64,
) -> Result<std::collections::HashSet<String>, ServerFnError>
where
    E: sqlx::Executor<'e, Database = sqlx::Sqlite>,
{
    let rows: Vec<String> = sqlx::query_scalar(
        "SELECT member_username FROM contact_collection_members WHERE collection_id = ?",
    )
    .bind(collection_id)
    .fetch_all(executor)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    Ok(rows.into_iter().collect())
}

/// The current user's own default collection ("My Contacts", created by
/// the contact_collections migration for every existing user) — used as
/// the fallback target for contact-creation paths with no explicit
/// collection choice. Lazily creates one if somehow missing (mirrors
/// `bump_contacts_sync_token`'s `INSERT ... ON CONFLICT DO NOTHING`
/// "ensure" idiom below), so a user created after that migration landed
/// is never left without one.
#[cfg(feature = "ssr")]
async fn ensure_default_contact_collection(
    pool: &SqlitePool,
    username: &str,
) -> Result<i64, ServerFnError> {
    sqlx::query(
        "INSERT INTO contact_collections (owner_username, name, color)
         VALUES (?, 'My Contacts', '#3b82f6')
         ON CONFLICT(owner_username, name) DO NOTHING",
    )
    .bind(username)
    .execute(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    sqlx::query_scalar(
        "SELECT id FROM contact_collections WHERE owner_username = ? AND name = 'My Contacts'",
    )
    .bind(username)
    .fetch_one(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))
}

// ── Contact collections ─────────────────────────────────────────────────────

/// List all contact collections the current user owns or is a member of.
#[server]
pub async fn list_contact_collections() -> Result<Vec<ContactCollectionInfo>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        let rows = sqlx::query(
            "SELECT id, owner_username, name, color, shared FROM contact_collections
             WHERE owner_username = ?
                OR id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?)
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
                    "SELECT member_username FROM contact_collection_members WHERE collection_id = ?",
                )
                .bind(id)
                .fetch_all(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
            } else {
                vec![]
            };

            out.push(ContactCollectionInfo {
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

/// Create a new contact collection owned by the current user.
#[server(input = server_fn::codec::Json)]
pub async fn create_contact_collection(input: NewContactCollection) -> Result<i64, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        if input.name.trim().is_empty() {
            return Err(ServerFnError::new("Name is required"));
        }

        let id: i64 = sqlx::query(
            "INSERT INTO contact_collections (owner_username, name, color, shared)
             VALUES (?, ?, ?, ?) RETURNING id",
        )
        .bind(&username)
        .bind(input.name.trim())
        .bind(&input.color)
        .bind(input.shared)
        .fetch_one(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
        .try_get("id")
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        return Ok(id);
    }
    #[allow(unreachable_code)]
    Ok(0)
}

/// Update an existing contact collection's name/color/sharing. Owner only.
#[server(input = server_fn::codec::Json)]
pub async fn update_contact_collection(
    id: i64,
    input: NewContactCollection,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        if !is_contact_collection_owner(&pool, &username, id).await? {
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
            "UPDATE contact_collections SET name = ?, color = ?, shared = ?, updated_at = datetime('now')
             WHERE id = ?",
        )
        .bind(input.name.trim())
        .bind(&input.color)
        .bind(input.shared)
        .bind(id)
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let existing = existing_contact_collection_members(&mut *tx, id).await?;

        sqlx::query("DELETE FROM contact_collection_members WHERE collection_id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        if input.shared {
            for member in input.members.iter().filter(|m| existing.contains(*m)) {
                sqlx::query(
                    "INSERT OR IGNORE INTO contact_collection_members (collection_id, member_username) VALUES (?, ?)",
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

/// Create a share invitation for one of the current user's contact
/// collections and email an accept/decline link — mirrors
/// `notes::share_note_collection` exactly.
#[server]
pub async fn share_contact_collection(
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

        if !is_contact_collection_owner(&pool, &username, collection_id).await? {
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
            "SELECT name FROM contact_collections WHERE id = ? AND owner_username = ?",
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
             (owner_username, resource_type, contact_collection_id, invitee_email, permission, token, expires_at) \
             VALUES (?, 'contact_collection', ?, ?, 'member', ?, ?)"
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

/// Remove one member from a shared contact collection. Owner only.
#[server]
pub async fn remove_contact_collection_member(
    collection_id: i64,
    member_username: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        if !is_contact_collection_owner(&pool, &username, collection_id).await? {
            return Err(ServerFnError::new("Not authorized to edit this collection"));
        }

        sqlx::query(
            "DELETE FROM contact_collection_members WHERE collection_id = ? AND member_username = ?",
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
/// own — mirrors `notes::leave_note_collection`.
#[server]
pub async fn leave_contact_collection(collection_id: i64) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        sqlx::query(
            "DELETE FROM contact_collection_members WHERE collection_id = ? AND member_username = ?",
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

/// Turn a shared contact collection back to private. Owner only.
#[server]
pub async fn unshare_contact_collection(collection_id: i64) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;

        if !is_contact_collection_owner(&pool, &username, collection_id).await? {
            return Err(ServerFnError::new("Not authorized to edit this collection"));
        }

        let mut tx = pool
            .begin()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        sqlx::query(
            "UPDATE contact_collections SET shared = 0, updated_at = datetime('now') WHERE id = ?",
        )
        .bind(collection_id)
        .execute(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        sqlx::query("DELETE FROM contact_collection_members WHERE collection_id = ?")
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

/// Delete a contact collection and every contact in it (their
/// email/phone/etc. rows cascade via `ON DELETE CASCADE`; the collection
/// row's own delete cascades `contact_collection_members` the same way).
/// Owner only. Refuses to delete a user's *last* collection — a contact
/// must always belong to somewhere, and `ensure_default_contact_collection`
/// assumes the owner always has a real row to fall back to.
#[server]
pub async fn delete_contact_collection(id: i64) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        return delete_contact_collection_inner(&pool, &username, id).await;
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// `delete_contact_collection`'s body, split out so it's testable directly
/// against a `#[sqlx::test]` pool without a live `#[server]` request
/// context (mirrors this file's `clear_contacts_for`/`clear_contacts` and
/// `merge_contacts_inner` splits).
#[cfg(feature = "ssr")]
async fn delete_contact_collection_inner(
    pool: &SqlitePool,
    username: &str,
    id: i64,
) -> Result<(), ServerFnError> {
    if !is_contact_collection_owner(pool, username, id).await? {
        return Err(ServerFnError::new(
            "Not authorized to delete this collection",
        ));
    }

    let owned_count: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM contact_collections WHERE owner_username = ?")
            .bind(username)
            .fetch_one(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    if owned_count <= 1 {
        return Err(ServerFnError::new("Cannot delete your only collection"));
    }

    // contacts.collection_id has no ON DELETE CASCADE (unlike
    // contact_collection_members.collection_id) — it must be cleared out
    // explicitly before the collection row itself can go, or SQLite's
    // foreign_keys=ON rejects the collection delete outright (code 787).
    // Each contact's own child rows (emails/phones/etc.) do cascade off
    // `contacts.id`, so this alone is enough. No sync-token bump is needed
    // for these deletions: `address_books`/`contact_sync_log` are keyed by
    // `collection_id` and cascade away with the collection itself right
    // below, so there's no address book left for a CardDAV client to sync
    // against — logging "deleted" for contacts in a book that no longer
    // exists would itself violate the same FK (an INSERT into
    // address_books/contact_sync_log referencing an already-deleted
    // collection_id) rather than mean anything.
    sqlx::query("DELETE FROM contacts WHERE collection_id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    sqlx::query("DELETE FROM contact_collections WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    Ok(())
}

/// Loads every email/phone row for a set of contact ids in one query each,
/// grouped by `contact_id` — avoids an N+1 fetch when listing many contacts.
#[cfg(feature = "ssr")]
async fn methods_by_contact(
    pool: &SqlitePool,
    table: &str,
    username: &str,
) -> Result<std::collections::HashMap<i64, Vec<ContactMethod>>, ServerFnError> {
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
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let mut out: std::collections::HashMap<i64, Vec<ContactMethod>> =
        std::collections::HashMap::new();
    for r in rows {
        let contact_id: i64 = r.try_get("contact_id").unwrap_or_default();
        out.entry(contact_id).or_default().push(ContactMethod {
            value: r.try_get("value").unwrap_or_default(),
            kind: r.try_get("type").unwrap_or_default(),
            is_primary: r.try_get("is_primary").unwrap_or_default(),
        });
    }
    Ok(out)
}

/// Like [`methods_by_contact`] but for `contact_addresses`, whose shape (5
/// address fields instead of one `value`) doesn't fit that helper's
/// generic query.
#[cfg(feature = "ssr")]
async fn addresses_by_contact(
    pool: &SqlitePool,
    username: &str,
) -> Result<std::collections::HashMap<i64, Vec<ContactAddress>>, ServerFnError> {
    let rows = sqlx::query(
        "SELECT a.contact_id, a.type, a.is_primary, a.street, a.city, a.region,
                a.postal_code, a.country
         FROM contact_addresses a
         JOIN contacts c ON c.id = a.contact_id
         JOIN contact_collections cc ON cc.id = c.collection_id
         WHERE cc.owner_username = ?
            OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?)
         ORDER BY a.is_primary DESC, a.id ASC",
    )
    .bind(username)
    .bind(username)
    .fetch_all(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let mut out: std::collections::HashMap<i64, Vec<ContactAddress>> =
        std::collections::HashMap::new();
    for r in rows {
        let contact_id: i64 = r.try_get("contact_id").unwrap_or_default();
        out.entry(contact_id).or_default().push(ContactAddress {
            kind: r.try_get("type").unwrap_or_default(),
            is_primary: r.try_get("is_primary").unwrap_or_default(),
            street: r.try_get("street").unwrap_or_default(),
            city: r.try_get("city").unwrap_or_default(),
            region: r.try_get("region").unwrap_or_default(),
            postal_code: r.try_get("postal_code").unwrap_or_default(),
            country: r.try_get("country").unwrap_or_default(),
        });
    }
    Ok(out)
}

/// Like [`methods_by_contact`] but for `contact_categories` — a flat tag
/// list, no kind/primary.
#[cfg(feature = "ssr")]
async fn categories_by_contact(
    pool: &SqlitePool,
    username: &str,
) -> Result<std::collections::HashMap<i64, Vec<String>>, ServerFnError> {
    let rows = sqlx::query(
        "SELECT g.contact_id, g.value
         FROM contact_categories g
         JOIN contacts c ON c.id = g.contact_id
         JOIN contact_collections cc ON cc.id = c.collection_id
         WHERE cc.owner_username = ?
            OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?)
         ORDER BY g.id ASC",
    )
    .bind(username)
    .bind(username)
    .fetch_all(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let mut out: std::collections::HashMap<i64, Vec<String>> = std::collections::HashMap::new();
    for r in rows {
        let contact_id: i64 = r.try_get("contact_id").unwrap_or_default();
        out.entry(contact_id)
            .or_default()
            .push(r.try_get("value").unwrap_or_default());
    }
    Ok(out)
}

/// Given a contact's own `(row, col, span)` in `contacts`, the `(row, col)`
/// for a new contact of `new_span` in the currently-shortest column,
/// scoped to `username`'s own contacts (single flat zone — no reserved
/// floor, unlike Notes' pinned zone). `exclude_id`, if given, is left out
/// (used when re-placing an existing contact).
#[cfg(feature = "ssr")]
async fn place_contact(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    username: &str,
    exclude_id: Option<i64>,
    new_span: i32,
) -> Result<(i32, i32), ServerFnError> {
    let rows: Vec<(i32, i32, i32)> = sqlx::query_as(
        "SELECT grid_row, grid_col, grid_row_span FROM contacts
         WHERE owner_username = ? AND id != COALESCE(?, -1)",
    )
    .bind(username)
    .bind(exclude_id)
    .fetch_all(&mut **tx)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    Ok(grid::shortest_column(&rows, CARD_GRID_COLS, 0, new_span))
}

/// Bump the CardDAV address-book sync token for `collection_id` and log the
/// change to `contact_sync_log`, so an external CardDAV client's
/// `sync-collection` REPORT (or `getctag`) notices a contact
/// created/edited/deleted through the web UI — not just through a CardDAV
/// PUT/DELETE. Mirrors `Storage::bump_address_book_sync_token` in
/// `crates/tidl-server/src/storage.rs`; duplicated rather than shared
/// because `tidl-web` cannot depend on `tidl-server` (CLAUDE.md §6/§8).
/// Keyed by `collection_id`, not `owner`, since backlog #81 makes
/// `address_books`/`contact_sync_log` one row per collection, not per user
/// — a shared collection's sync stream is one stream regardless of which
/// member touched it.
#[cfg(feature = "ssr")]
async fn bump_contacts_sync_token(
    pool: &SqlitePool,
    collection_id: i64,
    uid: &str,
    operation: &str,
) -> Result<(), ServerFnError> {
    sqlx::query(
        "INSERT INTO address_books (collection_id, sync_token) VALUES (?, 0)
         ON CONFLICT(collection_id) DO NOTHING",
    )
    .bind(collection_id)
    .execute(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    sqlx::query("UPDATE address_books SET sync_token = sync_token + 1 WHERE collection_id = ?")
        .bind(collection_id)
        .execute(pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let token: i64 =
        sqlx::query_scalar("SELECT sync_token FROM address_books WHERE collection_id = ?")
            .bind(collection_id)
            .fetch_one(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    sqlx::query(
        "INSERT INTO contact_sync_log (collection_id, uid, operation, token) VALUES (?, ?, ?, ?)",
    )
    .bind(collection_id)
    .bind(uid)
    .bind(operation)
    .bind(token)
    .execute(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    Ok(())
}

/// Batched form of [`bump_contacts_sync_token`] for `import_vcf`: one shared
/// token and one bulk `contact_sync_log` insert for the whole import — same
/// fix as `clear_contacts_for`'s doc comment describes for "Clear all".
/// Takes `&mut SqliteConnection` so `import_vcf` can run this as the last
/// step of its one whole-batch transaction rather than its own separate
/// commit (see `create_contact_from_vcard`'s doc comment for why the batch
/// is one transaction at all: per-contact commits, not per-contact
/// sync-token bumps, turned out to be the dominant cost once the latter was
/// fixed — measured via timing instrumentation, see the `.vcf` import perf
/// investigation). Sharing one token across the batch is safe for
/// `get_contact_sync_changes` for the same reason `clear_contacts_for`
/// gives: it dedups by uid (not by token) within a `(since, until]` range,
/// so N rows stamped with the same token still come back as N distinct
/// changes.
#[cfg(feature = "ssr")]
async fn bump_contacts_sync_token_batch(
    conn: &mut sqlx::SqliteConnection,
    collection_id: i64,
    uids: &[String],
    operation: &str,
) -> Result<(), ServerFnError> {
    if uids.is_empty() {
        return Ok(());
    }

    sqlx::query(
        "INSERT INTO address_books (collection_id, sync_token) VALUES (?, 0)
         ON CONFLICT(collection_id) DO NOTHING",
    )
    .bind(collection_id)
    .execute(&mut *conn)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    sqlx::query("UPDATE address_books SET sync_token = sync_token + 1 WHERE collection_id = ?")
        .bind(collection_id)
        .execute(&mut *conn)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let token: i64 =
        sqlx::query_scalar("SELECT sync_token FROM address_books WHERE collection_id = ?")
            .bind(collection_id)
            .fetch_one(&mut *conn)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let mut qb = sqlx::QueryBuilder::new(
        "INSERT INTO contact_sync_log (collection_id, uid, operation, token) ",
    );
    qb.push_values(uids.iter(), |mut b, uid| {
        b.push_bind(collection_id)
            .push_bind(uid)
            .push_bind(operation)
            .push_bind(token);
    });
    qb.build()
        .execute(&mut *conn)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    Ok(())
}

/// Replaces `table`'s rows for `contact_id` with `methods` — delete then
/// reinsert, matching `note_collection_members`' "small lists, simplest
/// correct approach" convention (see `api/notes.rs::update_note_collection`).
#[cfg(feature = "ssr")]
async fn replace_methods(
    conn: &mut sqlx::SqliteConnection,
    table: &str,
    contact_id: i64,
    methods: &[ContactMethod],
) -> Result<(), ServerFnError> {
    let delete_query = format!("DELETE FROM {table} WHERE contact_id = ?");
    sqlx::query(&delete_query)
        .bind(contact_id)
        .execute(&mut *conn)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let insert_query =
        format!("INSERT INTO {table} (contact_id, value, type, is_primary) VALUES (?, ?, ?, ?)");
    for m in methods {
        if m.value.trim().is_empty() {
            continue;
        }
        sqlx::query(&insert_query)
            .bind(contact_id)
            .bind(m.value.trim())
            .bind(&m.kind)
            .bind(m.is_primary)
            .execute(&mut *conn)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }
    Ok(())
}

/// Replaces `contact_addresses` for `contact_id` — same delete-then-reinsert
/// approach as [`replace_methods`], just a different (wider) row shape.
#[cfg(feature = "ssr")]
async fn replace_addresses(
    conn: &mut sqlx::SqliteConnection,
    contact_id: i64,
    addresses: &[ContactAddress],
) -> Result<(), ServerFnError> {
    sqlx::query("DELETE FROM contact_addresses WHERE contact_id = ?")
        .bind(contact_id)
        .execute(&mut *conn)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    for a in addresses {
        sqlx::query(
            "INSERT INTO contact_addresses
             (contact_id, type, is_primary, street, city, region, postal_code, country)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(contact_id)
        .bind(&a.kind)
        .bind(a.is_primary)
        .bind(&a.street)
        .bind(&a.city)
        .bind(&a.region)
        .bind(&a.postal_code)
        .bind(&a.country)
        .execute(&mut *conn)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }
    Ok(())
}

/// Replaces `contact_categories` for `contact_id` — same delete-then-reinsert
/// approach as [`replace_methods`], just a flat tag list (no kind/primary).
#[cfg(feature = "ssr")]
async fn replace_categories(
    conn: &mut sqlx::SqliteConnection,
    contact_id: i64,
    categories: &[String],
) -> Result<(), ServerFnError> {
    sqlx::query("DELETE FROM contact_categories WHERE contact_id = ?")
        .bind(contact_id)
        .execute(&mut *conn)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    for c in categories {
        if c.trim().is_empty() {
            continue;
        }
        sqlx::query("INSERT INTO contact_categories (contact_id, value) VALUES (?, ?)")
            .bind(contact_id)
            .bind(c.trim())
            .execute(&mut *conn)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }
    Ok(())
}

/// Fetches one contact row + its email/phone rows into a `ContactDto`.
/// Caller is responsible for the ownership check.
#[cfg(feature = "ssr")]
async fn fetch_contact(pool: &SqlitePool, id: i64) -> Result<Option<ContactDto>, ServerFnError> {
    let row = sqlx::query(
        "SELECT id, display_name, given_name, family_name, middle_name, name_prefix,
                name_suffix, nickname, org, title,
                CAST(birthday AS TEXT) AS birthday, CAST(anniversary AS TEXT) AS anniversary,
                notes, CAST(updated_at AS TEXT) AS updated_at, grid_row, grid_col, grid_row_span,
                photo IS NOT NULL AS has_photo, collection_id, manual_order
         FROM contacts WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    let Some(row) = row else { return Ok(None) };

    let fetch_methods = |table: &'static str| async move {
        let query = format!(
            "SELECT value, type, is_primary FROM {table} WHERE contact_id = ? ORDER BY is_primary DESC, id ASC"
        );
        sqlx::query(&query)
            .bind(id)
            .fetch_all(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))
            .map(|rows| {
                rows.into_iter()
                    .map(|r| ContactMethod {
                        value: r.try_get("value").unwrap_or_default(),
                        kind: r.try_get("type").unwrap_or_default(),
                        is_primary: r.try_get("is_primary").unwrap_or_default(),
                    })
                    .collect::<Vec<_>>()
            })
    };
    let emails = fetch_methods("contact_emails").await?;
    let phones = fetch_methods("contact_phones").await?;
    let urls = fetch_methods("contact_urls").await?;
    let impps = fetch_methods("contact_impps").await?;

    let addresses: Vec<ContactAddress> = sqlx::query(
        "SELECT type, is_primary, street, city, region, postal_code, country
         FROM contact_addresses WHERE contact_id = ? ORDER BY is_primary DESC, id ASC",
    )
    .bind(id)
    .fetch_all(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
    .into_iter()
    .map(|r| ContactAddress {
        kind: r.try_get("type").unwrap_or_default(),
        is_primary: r.try_get("is_primary").unwrap_or_default(),
        street: r.try_get("street").unwrap_or_default(),
        city: r.try_get("city").unwrap_or_default(),
        region: r.try_get("region").unwrap_or_default(),
        postal_code: r.try_get("postal_code").unwrap_or_default(),
        country: r.try_get("country").unwrap_or_default(),
    })
    .collect();

    let categories: Vec<String> =
        sqlx::query("SELECT value FROM contact_categories WHERE contact_id = ? ORDER BY id ASC")
            .bind(id)
            .fetch_all(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
            .into_iter()
            .map(|r| r.try_get("value").unwrap_or_default())
            .collect();

    Ok(Some(ContactDto {
        id: row.try_get("id").unwrap_or_default(),
        display_name: row.try_get("display_name").unwrap_or_default(),
        given_name: row.try_get("given_name").unwrap_or_default(),
        family_name: row.try_get("family_name").unwrap_or_default(),
        middle_name: row.try_get("middle_name").unwrap_or_default(),
        name_prefix: row.try_get("name_prefix").unwrap_or_default(),
        name_suffix: row.try_get("name_suffix").unwrap_or_default(),
        nickname: row.try_get("nickname").unwrap_or_default(),
        org: row.try_get("org").unwrap_or_default(),
        title: row.try_get("title").unwrap_or_default(),
        birthday: row.try_get("birthday").unwrap_or_default(),
        anniversary: row.try_get("anniversary").unwrap_or_default(),
        notes: row.try_get("notes").unwrap_or_default(),
        categories,
        emails,
        phones,
        urls,
        impps,
        addresses,
        has_photo: row.try_get("has_photo").unwrap_or_default(),
        updated_at: row.try_get("updated_at").unwrap_or_default(),
        grid_row: row.try_get("grid_row").unwrap_or_default(),
        grid_col: row.try_get("grid_col").unwrap_or_default(),
        grid_row_span: row.try_get("grid_row_span").unwrap_or(1),
        collection_id: row.try_get("collection_id").unwrap_or_default(),
        manual_order: row.try_get("manual_order").unwrap_or_default(),
    }))
}

/// Validates `input.display_name` and every method's/address's `kind`,
/// matching the `type` CHECK constraints on `contact_emails`/
/// `contact_phones`/`contact_urls`/`contact_impps`/`contact_addresses` — a
/// bad value would otherwise surface as an opaque SQL constraint error.
#[cfg(feature = "ssr")]
fn validate_new_contact(input: &NewContact) -> Result<(), ServerFnError> {
    if input.display_name.trim().is_empty() {
        return Err(ServerFnError::new("Name is required"));
    }
    for m in input
        .emails
        .iter()
        .chain(input.phones.iter())
        .chain(input.urls.iter())
        .chain(input.impps.iter())
    {
        if !matches!(m.kind.as_str(), "home" | "work" | "mobile" | "other") {
            return Err(ServerFnError::new("Invalid contact method type"));
        }
    }
    for a in &input.addresses {
        if !matches!(a.kind.as_str(), "home" | "work" | "other") {
            return Err(ServerFnError::new("Invalid address type"));
        }
    }
    Ok(())
}

/// Unions repeatable email/phone/url/IM-handle lists across every selected
/// contact for a merge (backlog #88) — no data is dropped, only exact
/// duplicates (same value, case/whitespace-insensitive, and same kind)
/// collapse to one row. At most one surviving entry keeps `is_primary`:
/// the first one encountered that had it set, so the primary contact's own
/// primary (lists are passed primary-first by the caller) wins ties.
#[cfg(feature = "ssr")]
fn union_methods(lists: impl Iterator<Item = Vec<ContactMethod>>) -> Vec<ContactMethod> {
    let mut seen: std::collections::HashSet<(String, String)> = std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut primary_taken = false;
    for list in lists {
        for mut m in list {
            let trimmed = m.value.trim().to_string();
            if trimmed.is_empty() {
                continue;
            }
            let key = (trimmed.to_lowercase(), m.kind.clone());
            if !seen.insert(key) {
                continue;
            }
            m.value = trimmed;
            if m.is_primary {
                if primary_taken {
                    m.is_primary = false;
                } else {
                    primary_taken = true;
                }
            }
            out.push(m);
        }
    }
    out
}

/// Same idea as [`union_methods`] for postal addresses — dedupe key is
/// every field (kind + all five address parts, case/whitespace-
/// insensitive), since two addresses only count as "the same" when they
/// truly are.
#[cfg(feature = "ssr")]
fn union_addresses(lists: impl Iterator<Item = Vec<ContactAddress>>) -> Vec<ContactAddress> {
    fn norm(s: &Option<String>) -> String {
        s.as_deref().unwrap_or("").trim().to_lowercase()
    }
    let mut seen: std::collections::HashSet<(String, String, String, String, String, String)> =
        std::collections::HashSet::new();
    let mut out = Vec::new();
    let mut primary_taken = false;
    for list in lists {
        for mut a in list {
            let key = (
                a.kind.clone(),
                norm(&a.street),
                norm(&a.city),
                norm(&a.region),
                norm(&a.postal_code),
                norm(&a.country),
            );
            if key.1.is_empty()
                && key.2.is_empty()
                && key.3.is_empty()
                && key.4.is_empty()
                && key.5.is_empty()
            {
                continue; // a fully-blank address carries no data worth keeping
            }
            if !seen.insert(key) {
                continue;
            }
            if a.is_primary {
                if primary_taken {
                    a.is_primary = false;
                } else {
                    primary_taken = true;
                }
            }
            out.push(a);
        }
    }
    out
}

/// Same idea as [`union_methods`] for the flat `categories` tag list — no
/// primary concept, just case/whitespace-insensitive dedupe that keeps the
/// first spelling encountered.
#[cfg(feature = "ssr")]
fn union_categories(lists: impl Iterator<Item = Vec<String>>) -> Vec<String> {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut out = Vec::new();
    for list in lists {
        for c in list {
            let trimmed = c.trim().to_string();
            if trimmed.is_empty() || !seen.insert(trimmed.to_lowercase()) {
                continue;
            }
            out.push(trimmed);
        }
    }
    out
}

// ── Contacts ─────────────────────────────────────────────────────────────────

/// List every contact owned by the current user.
#[server]
pub async fn list_contacts() -> Result<Vec<ContactDto>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        return list_contacts_for(&pool, &username).await;
    }
    #[allow(unreachable_code)]
    Ok(vec![])
}

/// `list_contacts`'s body, split out so it's directly testable without a
/// live `#[server]` request context (mirrors this file's
/// `clear_contacts_for`/`auto_layout_contacts_for` split).
#[cfg(feature = "ssr")]
async fn list_contacts_for(
    pool: &SqlitePool,
    username: &str,
) -> Result<Vec<ContactDto>, ServerFnError> {
    let rows = sqlx::query(
        "SELECT c.id, c.display_name, c.given_name, c.family_name, c.middle_name, c.name_prefix,
                c.name_suffix, c.nickname, c.org, c.title,
                CAST(c.birthday AS TEXT) AS birthday, CAST(c.anniversary AS TEXT) AS anniversary,
                c.notes, CAST(c.updated_at AS TEXT) AS updated_at, c.grid_row, c.grid_col, c.grid_row_span,
                c.photo IS NOT NULL AS has_photo, c.collection_id, c.manual_order
         FROM contacts c
         JOIN contact_collections cc ON cc.id = c.collection_id
         WHERE cc.owner_username = ?
            OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?)
         ORDER BY c.grid_row ASC, c.grid_col ASC",
    )
    .bind(username)
    .bind(username)
    .fetch_all(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let mut emails_by_id = methods_by_contact(pool, "contact_emails", username).await?;
    let mut phones_by_id = methods_by_contact(pool, "contact_phones", username).await?;
    let mut urls_by_id = methods_by_contact(pool, "contact_urls", username).await?;
    let mut impps_by_id = methods_by_contact(pool, "contact_impps", username).await?;
    let mut addresses_by_id = addresses_by_contact(pool, username).await?;
    let mut categories_by_id = categories_by_contact(pool, username).await?;

    Ok(rows
        .into_iter()
        .map(|r| {
            let id: i64 = r.try_get("id").unwrap_or_default();
            ContactDto {
                id,
                display_name: r.try_get("display_name").unwrap_or_default(),
                given_name: r.try_get("given_name").unwrap_or_default(),
                family_name: r.try_get("family_name").unwrap_or_default(),
                middle_name: r.try_get("middle_name").unwrap_or_default(),
                name_prefix: r.try_get("name_prefix").unwrap_or_default(),
                name_suffix: r.try_get("name_suffix").unwrap_or_default(),
                nickname: r.try_get("nickname").unwrap_or_default(),
                org: r.try_get("org").unwrap_or_default(),
                title: r.try_get("title").unwrap_or_default(),
                birthday: r.try_get("birthday").unwrap_or_default(),
                anniversary: r.try_get("anniversary").unwrap_or_default(),
                notes: r.try_get("notes").unwrap_or_default(),
                categories: categories_by_id.remove(&id).unwrap_or_default(),
                emails: emails_by_id.remove(&id).unwrap_or_default(),
                phones: phones_by_id.remove(&id).unwrap_or_default(),
                urls: urls_by_id.remove(&id).unwrap_or_default(),
                impps: impps_by_id.remove(&id).unwrap_or_default(),
                addresses: addresses_by_id.remove(&id).unwrap_or_default(),
                has_photo: r.try_get("has_photo").unwrap_or_default(),
                updated_at: r.try_get("updated_at").unwrap_or_default(),
                grid_row: r.try_get("grid_row").unwrap_or_default(),
                grid_col: r.try_get("grid_col").unwrap_or_default(),
                grid_row_span: r.try_get("grid_row_span").unwrap_or(1),
                collection_id: r.try_get("collection_id").unwrap_or_default(),
                manual_order: r.try_get("manual_order").unwrap_or_default(),
            }
        })
        .collect())
}

/// Fetch a single contact. Owner only.
#[server]
pub async fn get_contact(id: i64) -> Result<ContactDto, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        if !is_contact_owner(&pool, &username, id).await? {
            return Err(ServerFnError::new("Not authorized for this contact"));
        }
        return fetch_contact(&pool, id)
            .await?
            .ok_or_else(|| ServerFnError::new("Contact not found"));
    }
    #[allow(unreachable_code)]
    Err(ServerFnError::new("unreachable"))
}

/// Finds the current user's own contact whose `display_name` matches `name`
/// case/whitespace-insensitively — the same "first match wins" rule
/// `@mention` rendering uses (backlog #80's Open Questions explicitly defer
/// disambiguating two same-named contacts to a future follow-up).
#[cfg(feature = "ssr")]
async fn find_contact_by_name(
    pool: &SqlitePool,
    username: &str,
    name: &str,
) -> Result<Option<i64>, ServerFnError> {
    sqlx::query_scalar(
        "SELECT c.id FROM contacts c JOIN contact_collections cc ON cc.id = c.collection_id
         WHERE (cc.owner_username = ? OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?))
            AND LOWER(TRIM(c.display_name)) = LOWER(TRIM(?))
         ORDER BY c.id ASC LIMIT 1",
    )
    .bind(username)
    .bind(username)
    .bind(name)
    .fetch_optional(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))
}

/// Resolve an `@Name` mention (as typed in a note) to the current user's own
/// matching contact, for the click-to-open popover (backlog #80). `None`
/// when no contact matches — the caller treats that as "nothing to show"
/// rather than an error, since the rendered mention itself already only
/// appears when `render_note_mentions` found a match against the same
/// contact list, so a `None` here would only happen on a race (the contact
/// was renamed/deleted between render and click).
#[server]
pub async fn resolve_contact_mention(name: String) -> Result<Option<ContactDto>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        let Some(id) = find_contact_by_name(&pool, &username, &name).await? else {
            return Ok(None);
        };
        return fetch_contact(&pool, id).await;
    }
    #[allow(unreachable_code)]
    Ok(None)
}

/// Create a contact, auto-placed in the currently-shortest grid column.
#[server(input = server_fn::codec::Json)]
pub async fn create_contact(input: NewContact) -> Result<ContactDto, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        validate_new_contact(&input)?;

        let collection_id = match input.collection_id {
            Some(cid) => {
                if !has_contact_collection_access(&pool, &username, cid).await? {
                    return Err(ServerFnError::new("Not authorized for this collection"));
                }
                cid
            }
            None => ensure_default_contact_collection(&pool, &username).await?,
        };

        let mut tx = pool
            .begin()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // A real estimate from the fields actually being saved, not a
        // hardcoded placeholder (backlog #106 sixth follow-up, found live:
        // `place_contact`'s own shortest-column search -- and the row/span
        // this contact actually gets stored with -- used to always assume
        // span 1 regardless of content, wildly undershooting a real card's
        // rendered height; a client-side height-correction pass fixes this
        // up after the fact, but only once it runs and `contacts_res`
        // eventually refetches to pick it up -- in the meantime,
        // Auto-layout's own masonry pack read the stale, far-too-short
        // span and could stack the very next contact almost on top of
        // this one). Mirrors `vcard_content_lines`'s own field-presence
        // count exactly, over `NewContact`'s matching fields.
        let has_subtitle = input.title.as_deref().is_some_and(|s| !s.is_empty())
            || input.org.as_deref().is_some_and(|s| !s.is_empty());
        let content_lines = i32::from(has_subtitle)
            + i32::from(!input.urls.is_empty())
            + input.emails.len() as i32
            + input.phones.len() as i32
            + i32::from(input.notes.as_deref().is_some_and(|s| !s.is_empty()));
        let initial_span = grid::estimate_span(content_lines);
        let (grid_row, grid_col) = place_contact(&mut tx, &username, None, initial_span).await?;
        let uid = format!("contact:{}", uuid::Uuid::new_v4());
        let etag = format!("etag-{}", uuid::Uuid::new_v4().simple());

        let id: i64 = sqlx::query(
            "INSERT INTO contacts
             (owner_username, collection_id, uid, display_name, given_name, family_name, middle_name,
              name_prefix, name_suffix, nickname, org, title, birthday, anniversary, notes,
              etag, grid_row, grid_col, grid_row_span)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id",
        )
        .bind(&username)
        .bind(collection_id)
        .bind(&uid)
        .bind(input.display_name.trim())
        .bind(&input.given_name)
        .bind(&input.family_name)
        .bind(&input.middle_name)
        .bind(&input.name_prefix)
        .bind(&input.name_suffix)
        .bind(&input.nickname)
        .bind(&input.org)
        .bind(&input.title)
        .bind(&input.birthday)
        .bind(&input.anniversary)
        .bind(&input.notes)
        .bind(&etag)
        .bind(grid_row)
        .bind(grid_col)
        .bind(initial_span)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
        .try_get("id")
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        replace_methods(&mut tx, "contact_emails", id, &input.emails).await?;
        replace_methods(&mut tx, "contact_phones", id, &input.phones).await?;
        replace_methods(&mut tx, "contact_urls", id, &input.urls).await?;
        replace_methods(&mut tx, "contact_impps", id, &input.impps).await?;
        replace_addresses(&mut tx, id, &input.addresses).await?;
        replace_categories(&mut tx, id, &input.categories).await?;

        tx.commit()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        bump_contacts_sync_token(&pool, collection_id, &uid, "modified").await?;

        return fetch_contact(&pool, id)
            .await?
            .ok_or_else(|| ServerFnError::new("Contact not found"));
    }
    #[allow(unreachable_code)]
    Err(ServerFnError::new("unreachable"))
}

/// Update a contact's fields. Owner only. Grid position is left untouched
/// here — the client's height self-correction pass (mirroring Notes')
/// persists any span change separately via `move_contacts`.
#[server(input = server_fn::codec::Json)]
pub async fn update_contact(id: i64, input: NewContact) -> Result<ContactDto, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        if !is_contact_owner(&pool, &username, id).await? {
            return Err(ServerFnError::new("Not authorized for this contact"));
        }
        validate_new_contact(&input)?;

        let etag = format!("etag-{}", uuid::Uuid::new_v4().simple());

        let mut tx = pool
            .begin()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let (uid, collection_id): (String, i64) = sqlx::query_as(
            "UPDATE contacts SET display_name = ?, given_name = ?, family_name = ?,
                    middle_name = ?, name_prefix = ?, name_suffix = ?, nickname = ?, org = ?,
                    title = ?, birthday = ?, anniversary = ?, notes = ?, etag = ?,
                    updated_at = datetime('now')
             WHERE id = ? RETURNING uid, collection_id",
        )
        .bind(input.display_name.trim())
        .bind(&input.given_name)
        .bind(&input.family_name)
        .bind(&input.middle_name)
        .bind(&input.name_prefix)
        .bind(&input.name_suffix)
        .bind(&input.nickname)
        .bind(&input.org)
        .bind(&input.title)
        .bind(&input.birthday)
        .bind(&input.anniversary)
        .bind(&input.notes)
        .bind(&etag)
        .bind(id)
        .fetch_one(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        replace_methods(&mut tx, "contact_emails", id, &input.emails).await?;
        replace_methods(&mut tx, "contact_phones", id, &input.phones).await?;
        replace_methods(&mut tx, "contact_urls", id, &input.urls).await?;
        replace_methods(&mut tx, "contact_impps", id, &input.impps).await?;
        replace_addresses(&mut tx, id, &input.addresses).await?;
        replace_categories(&mut tx, id, &input.categories).await?;

        tx.commit()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        bump_contacts_sync_token(&pool, collection_id, &uid, "modified").await?;

        return fetch_contact(&pool, id)
            .await?
            .ok_or_else(|| ServerFnError::new("Contact not found"));
    }
    #[allow(unreachable_code)]
    Err(ServerFnError::new("unreachable"))
}

/// Max raw (decoded) photo bytes accepted from the web-UI upload path
/// (#86) — mirrors `tidl_carddav::vcard::MAX_PHOTO_BYTES`, the CardDAV-PUT
/// entry point's equivalent cap. This is the *other* way photo bytes enter
/// tidl, and never goes through vCard parsing, so it needs its own
/// independent check.
#[cfg(feature = "ssr")]
const MAX_PHOTO_BYTES: usize = 5 * 1024 * 1024;

/// Validates `content_type` and decodes+size-checks `base64_data`, separate
/// from `upload_contact_photo` so it's testable without the `#[server]`
/// macro's request-context requirements — mirrors `validate_new_contact`'s
/// reasoning for being its own function.
#[cfg(feature = "ssr")]
fn decode_and_validate_photo(
    base64_data: &str,
    content_type: &str,
) -> Result<Vec<u8>, ServerFnError> {
    if !matches!(content_type, "image/jpeg" | "image/png") {
        return Err(ServerFnError::new("Unsupported photo type"));
    }

    use base64::Engine;
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(base64_data.trim())
        .map_err(|_| ServerFnError::new("Invalid photo data"))?;
    if bytes.is_empty() {
        return Err(ServerFnError::new("Photo is empty"));
    }
    if bytes.len() > MAX_PHOTO_BYTES {
        return Err(ServerFnError::new("Photo is too large (max 5 MiB)"));
    }
    Ok(bytes)
}

/// Upload or replace a contact's photo. `content_type` must be
/// `image/jpeg` or `image/png` (vCard `TYPE=JPEG`/`TYPE=PNG` map cleanly to
/// these) — `image/gif` is accepted on the CardDAV *parse* side for
/// photos synced in from another client, but not offered here since
/// nothing in the upload UI produces GIFs. Decoupled from
/// `create_contact`/`update_contact` deliberately (#86 design): editing an
/// unrelated field never re-sends up to 5MB of photo bytes, and uploading a
/// photo never touches other fields.
#[server(input = server_fn::codec::Json)]
pub async fn upload_contact_photo(
    id: i64,
    base64_data: String,
    content_type: String,
) -> Result<ContactDto, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        if !is_contact_owner(&pool, &username, id).await? {
            return Err(ServerFnError::new("Not authorized for this contact"));
        }
        let bytes = decode_and_validate_photo(&base64_data, &content_type)?;

        let (uid, collection_id): (String, i64) = sqlx::query_as(
            "UPDATE contacts SET photo = ?, photo_content_type = ?, updated_at = datetime('now')
             WHERE id = ? RETURNING uid, collection_id",
        )
        .bind(&bytes)
        .bind(&content_type)
        .bind(id)
        .fetch_one(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        bump_contacts_sync_token(&pool, collection_id, &uid, "modified").await?;

        return fetch_contact(&pool, id)
            .await?
            .ok_or_else(|| ServerFnError::new("Contact not found"));
    }
    #[allow(unreachable_code)]
    Err(ServerFnError::new("unreachable"))
}

/// Remove a contact's photo. Owner only.
#[server]
pub async fn delete_contact_photo(id: i64) -> Result<ContactDto, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        if !is_contact_owner(&pool, &username, id).await? {
            return Err(ServerFnError::new("Not authorized for this contact"));
        }

        let (uid, collection_id): (String, i64) = sqlx::query_as(
            "UPDATE contacts SET photo = NULL, photo_content_type = NULL, updated_at = datetime('now')
             WHERE id = ? RETURNING uid, collection_id",
        )
        .bind(id)
        .fetch_one(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        bump_contacts_sync_token(&pool, collection_id, &uid, "modified").await?;

        return fetch_contact(&pool, id)
            .await?
            .ok_or_else(|| ServerFnError::new("Contact not found"));
    }
    #[allow(unreachable_code)]
    Err(ServerFnError::new("unreachable"))
}

/// Delete a contact (its email/phone rows cascade via `ON DELETE CASCADE`).
/// Owner only.
#[server]
pub async fn delete_contact(id: i64) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        if !is_contact_owner(&pool, &username, id).await? {
            return Err(ServerFnError::new("Not authorized for this contact"));
        }
        let (uid, collection_id): (String, i64) =
            sqlx::query_as("DELETE FROM contacts WHERE id = ? RETURNING uid, collection_id")
                .bind(id)
                .fetch_one(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        bump_contacts_sync_token(&pool, collection_id, &uid, "deleted").await?;
        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Delete every contact `username` can access — every contact of theirs
/// (`collection_id = Some(cid)`), or every contact across every collection
/// they own or are a member of (`collection_id = None`) — and log each
/// batch as one CardDAV sync revision per real owner. Factored out of
/// [`clear_contacts`] so it's directly testable without an auth context
/// (mirrors `notes.rs`'s `delete_note_inner`/`clear_notes` split).
///
/// A shared collection can contain contacts created by different members
/// (`contacts.owner_username` is "who created it", not the collection's
/// owner — see this module's `is_contact_owner` doc comment), so the
/// cleared rows are grouped by their own real owner before bumping each
/// one's CardDAV sync token — a single member's "Clear all" click can
/// therefore bump more than one owner's address book.
///
/// The naive version of this (one `bump_contacts_sync_token` call per uid,
/// each doing its own 4-statement autocommit) was the actual root cause of
/// "Clear all seems to need a page refresh": for a real address book of
/// ~1,000 contacts that's ~4,000 sequential round trips — tens of seconds
/// with no loading indicator, easily mistaken for the button doing nothing.
/// Wrapping the whole batch in one transaction, and giving every deleted
/// uid the *same* new token (per owner), cuts that to a handful of
/// statements and one commit. Sharing a token across a batch is safe for
/// `get_contact_sync_changes`: it dedups by uid (not by token) within the
/// `(since, until]` range, so N rows stamped with the same token are all
/// still returned as N distinct deletions.
#[cfg(feature = "ssr")]
async fn clear_contacts_for(
    pool: &SqlitePool,
    username: &str,
    collection_id: Option<i64>,
) -> Result<(), ServerFnError> {
    if let Some(cid) = collection_id {
        if !has_contact_collection_access(pool, username, cid).await? {
            return Err(ServerFnError::new("Not authorized for this collection"));
        }
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let rows: Vec<(i64, String)> = match collection_id {
        Some(cid) => sqlx::query_as("SELECT collection_id, uid FROM contacts WHERE collection_id = ?")
            .bind(cid)
            .fetch_all(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?,
        None => sqlx::query_as(
            "SELECT c.collection_id, c.uid FROM contacts c JOIN contact_collections cc ON cc.id = c.collection_id
             WHERE cc.owner_username = ?
                OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?)",
        )
        .bind(username)
        .bind(username)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?,
    };

    if rows.is_empty() {
        return Ok(());
    }

    match collection_id {
        Some(cid) => {
            sqlx::query("DELETE FROM contacts WHERE collection_id = ?")
                .bind(cid)
                .execute(&mut *tx)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        }
        None => {
            sqlx::query(
                "DELETE FROM contacts WHERE collection_id IN (
                     SELECT id FROM contact_collections WHERE owner_username = ?
                        OR id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?)
                 )",
            )
            .bind(username)
            .bind(username)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        }
    }

    tx.commit()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let mut by_collection: std::collections::HashMap<i64, Vec<String>> =
        std::collections::HashMap::new();
    for (collection_id, uid) in rows {
        by_collection.entry(collection_id).or_default().push(uid);
    }
    let mut conn = pool
        .acquire()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    for (collection_id, uids) in by_collection {
        bump_contacts_sync_token_batch(&mut conn, collection_id, &uids, "deleted").await?;
    }
    Ok(())
}

/// Delete every contact the current user can access, either in one
/// specific collection (`Some(collection_id)`) or across every collection
/// they own/are a member of (`None`) — "Clear all" toolbar button.
/// Mirrors `notes::clear_notes`'s signature. See [`clear_contacts_for`]
/// for why this is one transaction rather than a per-contact loop.
#[server]
pub async fn clear_contacts(collection_id: Option<i64>) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        return clear_contacts_for(&pool, &username, collection_id).await;
    }
    #[allow(unreachable_code)]
    Ok(())
}

// ── Contacts import (.vcf, backlog #87) ───────────────────────────────────────

/// Max raw `.vcf` upload size — matches `import_ics`'s (#10) precedent
/// exactly (`api/calendars.rs::import_ics`'s `MAX_BYTES`).
#[cfg(feature = "ssr")]
const MAX_VCF_BYTES: usize = 10 * 1024 * 1024;

/// Split a text blob containing zero or more `VCARD`s into individual
/// `BEGIN:VCARD…END:VCARD` blocks, each ready to hand to
/// [`vcard::parse_vcard`] as-is. Unlike `api/calendars.rs::split_vcalendar_events`,
/// a `VCARD` needs no surrounding envelope or shared-component copying (no
/// `VTIMEZONE`-style cross-references), so this only has to find the
/// boundaries — line-based, same approach as that function, and safe to run
/// before unfolding since a folded continuation line never equals
/// `BEGIN:VCARD`/`END:VCARD`.
#[cfg(feature = "ssr")]
fn split_vcards(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut current = String::new();
    let mut in_vcard = false;

    for line in text.lines() {
        let trimmed = line.trim_end();
        let upper = trimmed.trim().to_ascii_uppercase();

        if upper == "BEGIN:VCARD" {
            in_vcard = true;
            current.clear();
            current.push_str(trimmed);
            current.push_str("\r\n");
        } else if upper == "END:VCARD" && in_vcard {
            current.push_str(trimmed);
            current.push_str("\r\n");
            out.push(std::mem::take(&mut current));
            in_vcard = false;
        } else if in_vcard {
            current.push_str(trimmed);
            current.push_str("\r\n");
        }
    }
    out
}

/// Maps a parsed `VCardFields` onto `NewContact` — the two shapes are
/// deliberately near-identical (both describe "everything about one
/// contact"), so this is a field-for-field copy rather than any real
/// translation. `photo` is handled separately by the caller since
/// `NewContact` doesn't carry it (#86's photo upload is its own call).
#[cfg(feature = "ssr")]
fn vcard_to_new_contact(fields: &VCardFields) -> NewContact {
    let map_method = |m: &tidl_carddav::vcard::VCardMethod| ContactMethod {
        value: m.value.clone(),
        kind: m.kind.clone(),
        is_primary: m.is_primary,
    };
    NewContact {
        display_name: fields.display_name.clone(),
        given_name: fields.given_name.clone(),
        family_name: fields.family_name.clone(),
        middle_name: fields.middle_name.clone(),
        name_prefix: fields.name_prefix.clone(),
        name_suffix: fields.name_suffix.clone(),
        nickname: fields.nickname.clone(),
        org: fields.org.clone(),
        title: fields.title.clone(),
        birthday: fields.birthday.clone(),
        anniversary: fields.anniversary.clone(),
        notes: fields.notes.clone(),
        categories: fields.categories.clone(),
        emails: fields.emails.iter().map(map_method).collect(),
        phones: fields.phones.iter().map(map_method).collect(),
        urls: fields.urls.iter().map(map_method).collect(),
        impps: fields.impps.iter().map(map_method).collect(),
        addresses: fields
            .addresses
            .iter()
            .map(|a| ContactAddress {
                kind: a.kind.clone(),
                is_primary: a.is_primary,
                street: a.street.clone(),
                city: a.city.clone(),
                region: a.region.clone(),
                postal_code: a.postal_code.clone(),
                country: a.country.clone(),
            })
            .collect(),
        // `create_contact_from_vcard`/`apply_vcard_to_contact` (the only
        // callers) resolve `collection_id` separately, not from this
        // field — see their own signatures.
        collection_id: None,
    }
}

/// The identity signals `looks_like_duplicate` compares — name is always
/// pre-normalized (trimmed, lowercased) by whoever builds this, so the
/// predicate itself never has to think about casing.
#[cfg(feature = "ssr")]
struct DuplicateSignals {
    name_key: String,
    /// Pre-lowercased — email comparison is case-insensitive.
    emails: Vec<String>,
    /// Not lowercased — phone values compare exactly, matching
    /// `find_duplicate_contact`'s original `contact_phones` query.
    phones: Vec<String>,
    /// True if this contact has *any* other identifying data on file —
    /// given/family/middle name, prefix/suffix, nickname, org, title,
    /// birthday, anniversary, notes, categories, URLs, IM handles,
    /// addresses, or a photo. Only used to decide whether the name-only
    /// fallback below is safe (neither side has data that could be lost),
    /// never as a *positive* match signal — none of those fields are
    /// reliable enough to assert "same person," but their mere presence
    /// is enough to say "there's real data here, don't blindly assume a
    /// match on name alone."
    has_other_info: bool,
}

/// Two contacts "look like" duplicates when their names match and either
/// (a) *neither* side has any identifying data at all — no email, phone,
/// or anything in `has_other_info` — so name is the only signal there
/// will ever be (the accepted false-positive risk for common names,
/// unavoidable when both records are this bare), or (b) at least one
/// email or phone value is shared exactly. Two same-named contacts where
/// *either* side has real data on file — even data that doesn't overlap,
/// or isn't email/phone at all — are **not** flagged: a same-named "Alex"
/// with only an email on one side and only a URL on the other side must
/// not be treated as a duplicate, or that URL is silently discarded on
/// import skip. Shared by `find_duplicate_contact` (import, one incoming
/// vCard vs. many DB rows) and `find_possible_duplicates` (existing
/// contacts vs. each other) so the two can never drift onto different
/// rules.
#[cfg(feature = "ssr")]
fn looks_like_duplicate(a: &DuplicateSignals, b: &DuplicateSignals) -> bool {
    if a.name_key != b.name_key {
        return false;
    }
    let a_bare = a.emails.is_empty() && a.phones.is_empty() && !a.has_other_info;
    let b_bare = b.emails.is_empty() && b.phones.is_empty() && !b.has_other_info;
    if a_bare && b_bare {
        return true;
    }
    a.emails.iter().any(|e| b.emails.contains(e)) || a.phones.iter().any(|p| b.phones.contains(p))
}

/// `has_other_info` for an incoming vCard — every field `NewContact`
/// tracks besides display name/emails/phones. A blank-but-present string
/// (`Some("")`, which some vCard producers emit) doesn't count as real
/// data, matching how the rest of this file treats optional text fields
/// (e.g. `contact_card.rs`'s subtitle logic).
#[cfg(feature = "ssr")]
fn vcard_has_other_info(fields: &VCardFields) -> bool {
    fn present(s: &Option<String>) -> bool {
        s.as_deref().is_some_and(|s| !s.trim().is_empty())
    }
    present(&fields.given_name)
        || present(&fields.family_name)
        || present(&fields.middle_name)
        || present(&fields.name_prefix)
        || present(&fields.name_suffix)
        || present(&fields.nickname)
        || present(&fields.org)
        || present(&fields.title)
        || present(&fields.birthday)
        || present(&fields.anniversary)
        || present(&fields.notes)
        || !fields.categories.is_empty()
        || !fields.urls.is_empty()
        || !fields.impps.is_empty()
        || !fields.addresses.is_empty()
        || fields.photo.is_some()
}

/// `has_other_info` for every *existing* contact owned by `username`, in
/// one query (not one query per candidate) — `find_duplicate_contact` is
/// called once per vCard in an import batch of potentially hundreds, so
/// this is computed once by the caller and passed down, the same
/// precompute-outside-the-loop pattern `import_vcf` already uses for grid
/// placement (see `create_contact_from_vcard`'s doc comment on the O(n²)
/// bug that pattern was introduced to fix). Returns the set of contact
/// ids that have anything beyond display_name/email/phone on file.
#[cfg(feature = "ssr")]
async fn contact_ids_with_other_info(
    pool: &SqlitePool,
    username: &str,
) -> Result<std::collections::HashSet<i64>, ServerFnError> {
    let ids: Vec<i64> = sqlx::query_scalar(
        "SELECT c.id FROM contacts c JOIN contact_collections cc ON cc.id = c.collection_id
         WHERE (cc.owner_username = ? OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?)) AND (
            (c.given_name IS NOT NULL AND TRIM(c.given_name) != '') OR
            (c.family_name IS NOT NULL AND TRIM(c.family_name) != '') OR
            (c.middle_name IS NOT NULL AND TRIM(c.middle_name) != '') OR
            (c.name_prefix IS NOT NULL AND TRIM(c.name_prefix) != '') OR
            (c.name_suffix IS NOT NULL AND TRIM(c.name_suffix) != '') OR
            (c.nickname IS NOT NULL AND TRIM(c.nickname) != '') OR
            (c.org IS NOT NULL AND TRIM(c.org) != '') OR
            (c.title IS NOT NULL AND TRIM(c.title) != '') OR
            c.birthday IS NOT NULL OR c.anniversary IS NOT NULL OR
            (c.notes IS NOT NULL AND TRIM(c.notes) != '') OR
            c.photo IS NOT NULL OR
            EXISTS(SELECT 1 FROM contact_urls u WHERE u.contact_id = c.id) OR
            EXISTS(SELECT 1 FROM contact_impps i WHERE i.contact_id = c.id) OR
            EXISTS(SELECT 1 FROM contact_addresses a WHERE a.contact_id = c.id) OR
            EXISTS(SELECT 1 FROM contact_categories g WHERE g.contact_id = c.id)
        )",
    )
    .bind(username)
    .bind(username)
    .fetch_all(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    Ok(ids.into_iter().collect())
}

/// Duplicate-matching heuristic (#87's spec, "full name + a primary
/// email/phone"): an existing contact is a match if its `display_name`
/// equals the incoming vCard's (case/whitespace-insensitive) *and* they
/// share at least one email or phone value — except when *neither* side
/// has any identifying data at all, where a name-only match is the only
/// signal available (the spec's own acknowledged false-positive risk for
/// common names, accepted rather than silently importing a likely dupe).
/// Returns the first matching contact's id, if any. `existing_other_info`
/// is precomputed once per import batch by the caller — see
/// `contact_ids_with_other_info`'s doc comment for why.
///
/// Takes `&mut SqliteConnection` (i.e. `&mut *tx`, not `&pool`) so
/// `import_vcf` can run this in the same transaction as its writes — a
/// contact created earlier in the same `.vcf` batch must be visible to this
/// check for later blocks in that batch, which it wouldn't be through a
/// separate pool connection until the writer's transaction commits. Same
/// convention as `api/notes.rs::sync_mirror`.
#[cfg(feature = "ssr")]
async fn find_duplicate_contact(
    conn: &mut sqlx::SqliteConnection,
    username: &str,
    fields: &VCardFields,
    existing_other_info: &std::collections::HashSet<i64>,
) -> Result<Option<i64>, ServerFnError> {
    let candidates: Vec<i64> = sqlx::query_scalar(
        "SELECT c.id FROM contacts c JOIN contact_collections cc ON cc.id = c.collection_id
         WHERE (cc.owner_username = ? OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?))
            AND LOWER(TRIM(c.display_name)) = LOWER(TRIM(?))",
    )
    .bind(username)
    .bind(username)
    .bind(&fields.display_name)
    .fetch_all(&mut *conn)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    if candidates.is_empty() {
        return Ok(None);
    }

    let incoming = DuplicateSignals {
        name_key: fields.display_name.trim().to_lowercase(),
        emails: fields
            .emails
            .iter()
            .map(|e| e.value.to_lowercase())
            .collect(),
        phones: fields.phones.iter().map(|p| p.value.clone()).collect(),
        has_other_info: vcard_has_other_info(fields),
    };

    for id in candidates {
        let existing_emails: Vec<String> =
            sqlx::query_scalar("SELECT LOWER(value) FROM contact_emails WHERE contact_id = ?")
                .bind(id)
                .fetch_all(&mut *conn)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        let existing_phones: Vec<String> =
            sqlx::query_scalar("SELECT value FROM contact_phones WHERE contact_id = ?")
                .bind(id)
                .fetch_all(&mut *conn)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        let existing = DuplicateSignals {
            name_key: incoming.name_key.clone(),
            emails: existing_emails,
            phones: existing_phones,
            has_other_info: existing_other_info.contains(&id),
        };
        if looks_like_duplicate(&incoming, &existing) {
            return Ok(Some(id));
        }
    }
    Ok(None)
}

/// Counts the optional content lines a vCard's fields will render as on its
/// `ContactCard` (subtitle, primary URL, one line per email/phone, notes
/// body) — see `contact_card.rs` for the exact composition this mirrors.
/// Used only to feed `grid::estimate_span` a real per-contact size for
/// `import_vcf`'s bulk-placement bookkeeping instead of a flat placeholder;
/// doesn't need to be pixel-exact, just proportional to the real card.
#[cfg(feature = "ssr")]
fn vcard_content_lines(fields: &VCardFields) -> i32 {
    let has_subtitle = fields.title.as_deref().is_some_and(|s| !s.is_empty())
        || fields.org.as_deref().is_some_and(|s| !s.is_empty());
    i32::from(has_subtitle)
        + i32::from(!fields.urls.is_empty())
        + fields.emails.len() as i32
        + fields.phones.len() as i32
        + i32::from(fields.notes.as_deref().is_some_and(|s| !s.is_empty()))
}

/// Inserts a brand-new contact from a parsed vCard — same shape as
/// `create_contact`'s body, reusing its `replace_*` helpers, but sourced
/// from `VCardFields` (with `photo`) instead of the web UI's `NewContact`
/// (without one). Takes `grid_row`/`grid_col` from the caller rather than
/// computing them via `place_contact` — `import_vcf` places hundreds of
/// contacts in one loop, and `place_contact` re-fetches and rescans *every*
/// existing contact per call, which made that loop O(n²) (900 contacts took
/// ~30s, see backlog #89's investigation). The caller now scans once and
/// tracks column heights across the loop instead.
///
/// Does **not** bump the address book's sync token, and takes `&mut
/// SqliteConnection` rather than owning its own transaction — `import_vcf`
/// runs the whole `.vcf` batch (every contact's writes, duplicate checks,
/// and the final sync-token bump) as one transaction with one commit,
/// instead of one commit per contact. That per-contact-commit version was
/// the actual root cause of the multi-second import time (~2.5ms per commit
/// × ~1,000 contacts, measured via timing instrumentation — see the `.vcf`
/// import perf investigation); this mirrors `clear_contacts_for`'s fix for
/// the same class of problem on the delete side.
#[cfg(feature = "ssr")]
async fn create_contact_from_vcard(
    conn: &mut sqlx::SqliteConnection,
    username: &str,
    collection_id: i64,
    fields: &VCardFields,
    (grid_row, grid_col): (i32, i32),
) -> Result<String, ServerFnError> {
    let input = vcard_to_new_contact(fields);
    validate_new_contact(&input)?;

    let uid = format!("contact:{}", uuid::Uuid::new_v4());
    let etag = format!("etag-{}", uuid::Uuid::new_v4().simple());
    let photo_bytes = fields.photo.as_ref().map(|p| p.bytes.clone());
    let photo_content_type = fields.photo.as_ref().map(|p| p.content_type.clone());

    let id: i64 = sqlx::query(
        "INSERT INTO contacts
         (owner_username, collection_id, uid, display_name, given_name, family_name, middle_name,
          name_prefix, name_suffix, nickname, org, title, birthday, anniversary, notes,
          photo, photo_content_type, etag, grid_row, grid_col, grid_row_span)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, 1) RETURNING id",
    )
    .bind(username)
    .bind(collection_id)
    .bind(&uid)
    .bind(input.display_name.trim())
    .bind(&input.given_name)
    .bind(&input.family_name)
    .bind(&input.middle_name)
    .bind(&input.name_prefix)
    .bind(&input.name_suffix)
    .bind(&input.nickname)
    .bind(&input.org)
    .bind(&input.title)
    .bind(&input.birthday)
    .bind(&input.anniversary)
    .bind(&input.notes)
    .bind(&photo_bytes)
    .bind(&photo_content_type)
    .bind(&etag)
    .bind(grid_row)
    .bind(grid_col)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
    .try_get("id")
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    replace_methods(conn, "contact_emails", id, &input.emails).await?;
    replace_methods(conn, "contact_phones", id, &input.phones).await?;
    replace_methods(conn, "contact_urls", id, &input.urls).await?;
    replace_methods(conn, "contact_impps", id, &input.impps).await?;
    replace_addresses(conn, id, &input.addresses).await?;
    replace_categories(conn, id, &input.categories).await?;

    Ok(uid)
}

/// Overwrites an existing contact (`id`) with a parsed vCard's fields — same
/// full-replace shape as `update_contact`, including `photo` (absent in the
/// vCard clears any existing photo, consistent with every other field's
/// full-replace semantics here). Like `create_contact_from_vcard`, takes a
/// shared `&mut SqliteConnection` (not its own transaction) and does not
/// bump the sync token itself — returns the `uid` for the caller to batch.
#[cfg(feature = "ssr")]
async fn apply_vcard_to_contact(
    conn: &mut sqlx::SqliteConnection,
    id: i64,
    fields: &VCardFields,
) -> Result<String, ServerFnError> {
    let input = vcard_to_new_contact(fields);
    validate_new_contact(&input)?;

    let etag = format!("etag-{}", uuid::Uuid::new_v4().simple());
    let photo_bytes = fields.photo.as_ref().map(|p| p.bytes.clone());
    let photo_content_type = fields.photo.as_ref().map(|p| p.content_type.clone());

    let uid: String = sqlx::query_scalar(
        "UPDATE contacts SET display_name = ?, given_name = ?, family_name = ?,
                middle_name = ?, name_prefix = ?, name_suffix = ?, nickname = ?, org = ?,
                title = ?, birthday = ?, anniversary = ?, notes = ?,
                photo = ?, photo_content_type = ?, etag = ?, updated_at = datetime('now')
         WHERE id = ? RETURNING uid",
    )
    .bind(input.display_name.trim())
    .bind(&input.given_name)
    .bind(&input.family_name)
    .bind(&input.middle_name)
    .bind(&input.name_prefix)
    .bind(&input.name_suffix)
    .bind(&input.nickname)
    .bind(&input.org)
    .bind(&input.title)
    .bind(&input.birthday)
    .bind(&input.anniversary)
    .bind(&input.notes)
    .bind(&photo_bytes)
    .bind(&photo_content_type)
    .bind(&etag)
    .bind(id)
    .fetch_one(&mut *conn)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    replace_methods(conn, "contact_emails", id, &input.emails).await?;
    replace_methods(conn, "contact_phones", id, &input.phones).await?;
    replace_methods(conn, "contact_urls", id, &input.urls).await?;
    replace_methods(conn, "contact_impps", id, &input.impps).await?;
    replace_addresses(conn, id, &input.addresses).await?;
    replace_categories(conn, id, &input.categories).await?;

    Ok(uid)
}

/// Preview a `.vcf` import before committing: how many `VCARD`s parse and
/// how many look like duplicates, per [`find_duplicate_contact`]'s
/// heuristic. Read-only — writes nothing.
#[server(input = server_fn::codec::Json)]
pub async fn preview_vcf_import(vcf_content: String) -> Result<VcfImportPreview, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        if vcf_content.len() > MAX_VCF_BYTES {
            return Err(ServerFnError::new("File exceeds the 10 MB limit"));
        }
        let (pool, username) = auth().await?;
        let mut conn = pool
            .acquire()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let other_info = contact_ids_with_other_info(&pool, &username).await?;
        let mut total = 0u32;
        let mut duplicates = 0u32;
        for block in split_vcards(&vcf_content) {
            let Ok(fields) = vcard::parse_vcard(&block) else {
                continue;
            };
            total += 1;
            if find_duplicate_contact(&mut conn, &username, &fields, &other_info)
                .await?
                .is_some()
            {
                duplicates += 1;
            }
        }
        return Ok(VcfImportPreview { total, duplicates });
    }
    #[allow(unreachable_code)]
    Ok(VcfImportPreview::default())
}

/// Import every `VCARD` in `vcf_content` into the current user's address
/// book. `overwrite` controls what happens to a duplicate match (per
/// [`find_duplicate_contact`]): replace it when `true`, leave it untouched
/// and count it as skipped when `false` — mirrors `import_ics`'s
/// skip/overwrite-all toggle (#10's precedent) rather than the spec's
/// literal "per match" wording, since a per-contact review step is a much
/// larger UI than this item's scope; see the backlog file's Resolution.
#[server(input = server_fn::codec::Json)]
pub async fn import_vcf(
    vcf_content: String,
    overwrite: bool,
    collection_id: Option<i64>,
) -> Result<ImportResult, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        if vcf_content.len() > MAX_VCF_BYTES {
            return Err(ServerFnError::new("File exceeds the 10 MB limit"));
        }
        let (pool, username) = auth().await?;

        // Resolved once for the whole batch — every imported contact lands
        // in the same collection (backlog #81). No per-contact collection
        // picker in this pass; a natural follow-up, not blocking.
        let target_collection = match collection_id {
            Some(cid) => {
                if !has_contact_collection_access(&pool, &username, cid).await? {
                    return Err(ServerFnError::new("Not authorized for this collection"));
                }
                cid
            }
            None => ensure_default_contact_collection(&pool, &username).await?,
        };

        let mut imported = 0u32;
        let mut skipped = 0u32;
        let mut failed = 0u32;
        // Every uid successfully created or overwritten this run, bumped as
        // one batch at the end instead of once per contact — see
        // `bump_contacts_sync_token_batch`'s doc comment for why.
        let mut changed_uids: Vec<String> = Vec::new();

        // The whole import — every contact's writes, duplicate checks, and
        // the final sync-token bump — runs as one transaction with one
        // commit, not one commit per contact. See
        // `create_contact_from_vcard`'s doc comment for why: that was the
        // dominant cost once the sync-token bump was fixed (measured via
        // timing instrumentation, see the `.vcf` import perf
        // investigation). Running `find_duplicate_contact` on this same
        // transaction (not a separate pool read) also means a contact
        // created earlier in this batch is visible to later duplicate
        // checks in the same batch, same as before this change.
        let mut tx = pool
            .begin()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Scan existing placements once and track column heights in memory
        // across the loop below, rather than having each new contact's
        // `create_contact_from_vcard` call `place_contact` (a full
        // rescan) — see that function's doc comment for why.
        let existing_rows: Vec<(i32, i32, i32)> = sqlx::query_as(
            "SELECT grid_row, grid_col, grid_row_span FROM contacts WHERE owner_username = ?",
        )
        .bind(&username)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        let mut heights = grid::column_heights(&existing_rows, CARD_GRID_COLS, 0);
        // Same one-scan-before-the-loop precompute as `heights` above, for
        // the same O(n²) reason (see `create_contact_from_vcard`'s doc
        // comment) — a snapshot from before this batch, so a contact
        // created earlier in *this* import won't have its own
        // `has_other_info` reflected for a later vCard in the same batch.
        // Accepted as a rare edge case, not a regression: the prior code
        // never consulted the existing side's data for this decision at
        // all.
        let other_info = contact_ids_with_other_info(&pool, &username).await?;

        for block in split_vcards(&vcf_content) {
            let fields = match vcard::parse_vcard(&block) {
                Ok(f) => f,
                Err(_) => {
                    failed += 1;
                    continue;
                }
            };

            let existing_id =
                find_duplicate_contact(&mut tx, &username, &fields, &other_info).await?;
            let result = match existing_id {
                Some(_) if !overwrite => {
                    skipped += 1;
                    continue;
                }
                Some(id) => apply_vcard_to_contact(&mut tx, id, &fields).await,
                None => {
                    let span = grid::estimate_span(vcard_content_lines(&fields));
                    let position = grid::place_in_shortest(&mut heights, span);
                    create_contact_from_vcard(
                        &mut tx,
                        &username,
                        target_collection,
                        &fields,
                        position,
                    )
                    .await
                }
            };

            match result {
                Ok(uid) => {
                    changed_uids.push(uid);
                    imported += 1;
                }
                Err(_) => failed += 1,
            }
        }

        bump_contacts_sync_token_batch(&mut tx, target_collection, &changed_uids, "modified")
            .await?;

        tx.commit()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

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

/// Writes every `(id, grid_row, grid_col)` in `moves` in a temporary,
/// mutually-unique parking position first, then in a second pass to their
/// real final positions — same two-pass technique as
/// `api/notes.rs::apply_positions`, for the same reason (a batch that
/// cascades contacts through each other's cells must never transiently
/// collide with another move in the same batch).
#[cfg(feature = "ssr")]
async fn apply_positions(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    moves: &[GridPosition],
) -> Result<(), ServerFnError> {
    for (i, m) in moves.iter().enumerate() {
        sqlx::query("UPDATE contacts SET grid_row = ?, grid_col = 0 WHERE id = ?")
            .bind(-1000 - i as i32)
            .bind(m.id)
            .execute(&mut **tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }
    for m in moves {
        sqlx::query(
            "UPDATE contacts SET grid_row = ?, grid_col = ?, grid_row_span = ? WHERE id = ?",
        )
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

/// Bulk-applies grid positions computed by the client's height
/// self-correction pass (backlog #48/#79 — no manual drag-reorder for
/// Contacts in v1, unlike Notes; see the position-only corrections this
/// still needs to persist). Requires ownership of every contact in the
/// batch.
#[server(input = server_fn::codec::Json)]
pub async fn move_contacts(moves: Vec<GridPosition>) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        if moves.is_empty() {
            return Ok(());
        }

        // BEGIN IMMEDIATE — see the identical comment in
        // `api/notes.rs::move_notes`. Same shape, same fix (backlog #83):
        // reads before the first write in a deferred transaction let another
        // writer's commit invalidate our snapshot, which WAL mode reports as
        // an instant, non-retried "database is locked".
        let mut tx = pool
            .begin_with("BEGIN IMMEDIATE")
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        for m in &moves {
            if !is_contact_owner(&pool, &username, m.id).await? {
                return Err(ServerFnError::new("Not authorized for this contact"));
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

/// Core of `auto_layout_contacts` — factored out so it's directly testable
/// without a live `#[server]` request context (mirrors this file's
/// `clear_contacts_for`/`clear_contacts` split).
#[cfg(feature = "ssr")]
async fn auto_layout_contacts_for(
    pool: &SqlitePool,
    username: &str,
    view_cols: i32,
) -> Result<(), ServerFnError> {
    let mut tx = pool
        .begin()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    // Every contact `username` can access, not just the ones they own —
    // a contact in a collection shared *to* them (member, not owner) still
    // needs repacking here, same access scope `list_contacts` uses; this
    // was previously owner-only, silently skipping a member's shared
    // contacts on every Auto-layout run.
    let rows: Vec<(i64, i32)> = sqlx::query_as(
        "SELECT c.id, c.grid_row_span FROM contacts c
         JOIN contact_collections cc ON cc.id = c.collection_id
         WHERE cc.owner_username = ?
            OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?)
         ORDER BY c.manual_order IS NULL ASC, c.manual_order ASC, c.display_name COLLATE NOCASE ASC",
    )
    .bind(username)
    .bind(username)
    .fetch_all(&mut *tx)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let moves: Vec<GridPosition> =
        grid::pack_for_view(&rows, view_cols, &vec![0; CARD_GRID_COLS as usize])
            .into_iter()
            .map(|(id, grid_row, grid_col, grid_row_span)| GridPosition {
                id,
                grid_row,
                grid_col,
                grid_row_span,
            })
            .collect();
    apply_positions(&mut tx, &moves).await?;

    tx.commit()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    Ok(())
}

/// Repacks the grid tightly (masonry) for the current user's contacts —
/// unlike Notes' own Auto-layout (which preserves reading order, and is
/// entirely client-computed since backlog #102 — see
/// `components::notes::NotesView`), contacts are placed in alphabetical
/// order by `display_name` (backlog #84) since contacts are looked up by
/// name, not recency; manual drag-to-reorder (#82) is untouched by this
/// and only this explicit action re-sorts. Single flat zone, unlike
/// Notes' pinned/non-pinned pair.
///
/// Only reachable today from the post-VCF-import rebalance pass
/// (`components::contacts::ContactsView`'s `contacts_needs_relayout`
/// effect) — "Keep this layout" persists the client preview directly via
/// `move_contacts` instead (backlog #102 follow-up), so this whole-address-
/// book repack never runs from that button any more.
///
/// `view_cols` is the caller's *currently rendered* column count, not
/// always `CARD_GRID_COLS`: repacking the full 8-column real space and
/// then compacting for display renders unevenly whenever `CARD_GRID_COLS`
/// doesn't divide evenly by however many columns are actually on screen.
#[server]
pub async fn auto_layout_contacts(view_cols: i32) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        return auto_layout_contacts_for(&pool, &username, view_cols).await;
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// Spacing between freshly-backfilled `manual_order` values (backlog
/// #102) -- wide enough that many subsequent midpoint inserts between
/// the same two neighbors stay comfortably clear of floating-point
/// precision before it would matter in practice. Not LexoRank-style
/// string ranks -- an accepted simplification for v1 (see the design
/// spec's Data Model section).
#[cfg(feature = "ssr")]
const MANUAL_ORDER_SPACING: f64 = 1000.0;

/// Reorders `id` to sit between `predecessor_id` and `successor_id` in
/// Auto mode's manual-order sequence (backlog #102) -- either may be
/// `None` (dragged to the very start/end of the list, or the zone has
/// only this one card). If this is the zone's first-ever reorder (no
/// contact the caller can access has a non-null `manual_order` yet),
/// backfills every accessible contact's `manual_order` first, seeded
/// from current alphabetical rank -- see the design spec's
/// `manual_order` section for why a plain null-falls-back-to-
/// alphabetical sort can't support inserting between two still-null
/// neighbors otherwise.
#[server]
pub async fn reorder_contact(
    id: i64,
    predecessor_id: Option<i64>,
    successor_id: Option<i64>,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        return reorder_contact_inner(&pool, &username, id, predecessor_id, successor_id).await;
    }
    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(feature = "ssr")]
async fn reorder_contact_inner(
    pool: &SqlitePool,
    username: &str,
    id: i64,
    predecessor_id: Option<i64>,
    successor_id: Option<i64>,
) -> Result<(), ServerFnError> {
    if !is_contact_owner(pool, username, id).await? {
        return Err(ServerFnError::new("Not authorized for this contact"));
    }

    let mut tx = pool
        .begin()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let any_ordered: Option<i64> = sqlx::query_scalar(
        "SELECT 1 FROM contacts c
         JOIN contact_collections cc ON cc.id = c.collection_id
         WHERE (cc.owner_username = ? OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?))
            AND c.manual_order IS NOT NULL
         LIMIT 1",
    )
    .bind(username)
    .bind(username)
    .fetch_optional(&mut *tx)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    if any_ordered.is_none() {
        let ids: Vec<i64> = sqlx::query_scalar(
            "SELECT c.id FROM contacts c
             JOIN contact_collections cc ON cc.id = c.collection_id
             WHERE cc.owner_username = ? OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?)
             ORDER BY c.display_name COLLATE NOCASE ASC",
        )
        .bind(username)
        .bind(username)
        .fetch_all(&mut *tx)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        for (i, cid) in ids.iter().enumerate() {
            sqlx::query("UPDATE contacts SET manual_order = ? WHERE id = ?")
                .bind((i as f64 + 1.0) * MANUAL_ORDER_SPACING)
                .bind(cid)
                .execute(&mut *tx)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        }
    }

    // Scoped by the same owner-or-member access join as everywhere else
    // in this file -- an inaccessible neighbor id (e.g. another user's
    // private contact) must be treated as absent (falls through to
    // extrapolation), never have its manual_order read and used to
    // compute the dragged card's own value. A bare `WHERE id = ?` here
    // would silently leak a cross-user float, and use it to place this
    // caller's own contact relative to it.
    let predecessor_order: Option<f64> = match predecessor_id {
        Some(nid) => sqlx::query_scalar(
            "SELECT c.manual_order FROM contacts c
             JOIN contact_collections cc ON cc.id = c.collection_id
             WHERE c.id = ?
                AND (cc.owner_username = ? OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?))",
        )
        .bind(nid)
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
            "SELECT c.manual_order FROM contacts c
             JOIN contact_collections cc ON cc.id = c.collection_id
             WHERE c.id = ?
                AND (cc.owner_username = ? OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?))",
        )
        .bind(nid)
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

    sqlx::query("UPDATE contacts SET manual_order = ? WHERE id = ?")
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

/// Core of `merge_contacts` — factored out so it's directly testable
/// without a live `#[server]` request context (mirrors this file's
/// `clear_contacts_for`/`clear_contacts` split).
#[cfg(feature = "ssr")]
async fn merge_contacts_inner(
    pool: &SqlitePool,
    username: &str,
    input: MergeContactsInput,
) -> Result<ContactDto, ServerFnError> {
    if input.merged_ids.is_empty() {
        return Err(ServerFnError::new(
            "Select at least one other contact to merge",
        ));
    }
    if input.merged_ids.contains(&input.primary_id) {
        return Err(ServerFnError::new(
            "Primary contact can't also be merged away",
        ));
    }
    let mut all_ids = vec![input.primary_id];
    all_ids.extend(input.merged_ids.iter().copied());
    let mut dedup_check = all_ids.clone();
    dedup_check.sort_unstable();
    dedup_check.dedup();
    if dedup_check.len() != all_ids.len() {
        return Err(ServerFnError::new("Duplicate contact in merge selection"));
    }
    if let Some(src) = input.keep_photo_from {
        if !all_ids.contains(&src) {
            return Err(ServerFnError::new("Invalid photo source"));
        }
    }

    // Ownership + existence check and pre-merge snapshot in one query —
    // read happens before any write, so this is safe outside a transaction.
    let rows = sqlx::query(
        "SELECT id, uid, display_name, collection_id FROM contacts WHERE owner_username = ?",
    )
    .bind(username)
    .fetch_all(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    let mut owned: std::collections::HashMap<i64, (String, String, i64)> =
        std::collections::HashMap::new();
    for r in rows {
        let id: i64 = r
            .try_get("id")
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        let uid: String = r
            .try_get("uid")
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        let name: String = r
            .try_get("display_name")
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        let collection_id: i64 = r
            .try_get("collection_id")
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        owned.insert(id, (uid, name, collection_id));
    }
    for &id in &all_ids {
        if !owned.contains_key(&id) {
            return Err(ServerFnError::new(
                "Not authorized for one of the selected contacts",
            ));
        }
    }

    let mut loaded: Vec<ContactDto> = Vec::with_capacity(all_ids.len());
    for &id in &all_ids {
        let c = fetch_contact(pool, id)
            .await?
            .ok_or_else(|| ServerFnError::new("Contact not found"))?;
        loaded.push(c);
    }

    let emails = union_methods(loaded.iter().map(|c| c.emails.clone()));
    let phones = union_methods(loaded.iter().map(|c| c.phones.clone()));
    let urls = union_methods(loaded.iter().map(|c| c.urls.clone()));
    let impps = union_methods(loaded.iter().map(|c| c.impps.clone()));
    let addresses = union_addresses(loaded.iter().map(|c| c.addresses.clone()));
    let categories = union_categories(loaded.iter().map(|c| c.categories.clone()));

    let merged = NewContact {
        display_name: input.resolved.display_name,
        given_name: input.resolved.given_name,
        family_name: input.resolved.family_name,
        middle_name: input.resolved.middle_name,
        name_prefix: input.resolved.name_prefix,
        name_suffix: input.resolved.name_suffix,
        nickname: input.resolved.nickname,
        org: input.resolved.org,
        title: input.resolved.title,
        birthday: input.resolved.birthday,
        anniversary: input.resolved.anniversary,
        notes: input.resolved.notes,
        categories,
        emails,
        phones,
        urls,
        impps,
        addresses,
        // The surviving contact keeps its own existing collection — a
        // merge never moves a contact between collections.
        collection_id: None,
    };
    validate_new_contact(&merged)?;

    // Longest-old-name-first so a same-pass rename to a short name can't
    // partially re-match text a longer rename already substituted — see
    // `rewrite_mentions`'s doc comment.
    let mut renames: Vec<(String, String)> = all_ids
        .iter()
        .filter_map(|id| owned.get(id))
        .filter(|(_, name, _)| !name.trim().is_empty() && *name != merged.display_name)
        .map(|(_, name, _)| (name.clone(), merged.display_name.clone()))
        .collect();
    renames.sort_by_key(|(old, _)| std::cmp::Reverse(old.chars().count()));

    let etag = format!("etag-{}", uuid::Uuid::new_v4().simple());
    let mut tx = pool
        .begin()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    sqlx::query(
        "UPDATE contacts SET display_name = ?, given_name = ?, family_name = ?,
                middle_name = ?, name_prefix = ?, name_suffix = ?, nickname = ?, org = ?,
                title = ?, birthday = ?, anniversary = ?, notes = ?, etag = ?,
                updated_at = datetime('now')
         WHERE id = ?",
    )
    .bind(merged.display_name.trim())
    .bind(&merged.given_name)
    .bind(&merged.family_name)
    .bind(&merged.middle_name)
    .bind(&merged.name_prefix)
    .bind(&merged.name_suffix)
    .bind(&merged.nickname)
    .bind(&merged.org)
    .bind(&merged.title)
    .bind(&merged.birthday)
    .bind(&merged.anniversary)
    .bind(&merged.notes)
    .bind(&etag)
    .bind(input.primary_id)
    .execute(&mut *tx)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    replace_methods(&mut tx, "contact_emails", input.primary_id, &merged.emails).await?;
    replace_methods(&mut tx, "contact_phones", input.primary_id, &merged.phones).await?;
    replace_methods(&mut tx, "contact_urls", input.primary_id, &merged.urls).await?;
    replace_methods(&mut tx, "contact_impps", input.primary_id, &merged.impps).await?;
    replace_addresses(&mut tx, input.primary_id, &merged.addresses).await?;
    replace_categories(&mut tx, input.primary_id, &merged.categories).await?;

    if let Some(src) = input.keep_photo_from {
        if src != input.primary_id {
            sqlx::query(
                "UPDATE contacts SET photo = (SELECT photo FROM contacts WHERE id = ?),
                        photo_content_type = (SELECT photo_content_type FROM contacts WHERE id = ?)
                 WHERE id = ?",
            )
            .bind(src)
            .bind(src)
            .bind(input.primary_id)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        }
    }

    if !renames.is_empty() {
        let note_rows: Vec<(i64, String)> =
            sqlx::query_as("SELECT id, text FROM notes WHERE author_username = ?")
                .bind(username)
                .fetch_all(&mut *tx)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        for (note_id, text) in note_rows {
            let rewritten = crate::util::rewrite_mentions(&text, &renames);
            if rewritten != text {
                crate::api::notes::rewrite_note_body(&mut tx, username, note_id, rewritten).await?;
            }
        }
    }

    for &id in &input.merged_ids {
        sqlx::query("DELETE FROM contacts WHERE id = ?")
            .bind(id)
            .execute(&mut *tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    }

    tx.commit()
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let (primary_uid, _, primary_collection) = owned
        .get(&input.primary_id)
        .cloned()
        .ok_or_else(|| ServerFnError::new("Contact not found"))?;
    bump_contacts_sync_token(pool, primary_collection, &primary_uid, "modified").await?;
    for &id in &input.merged_ids {
        if let Some((uid, _, coll)) = owned.get(&id) {
            bump_contacts_sync_token(pool, *coll, uid, "deleted").await?;
        }
    }

    fetch_contact(pool, input.primary_id)
        .await?
        .ok_or_else(|| ServerFnError::new("Contact not found"))
}

/// Merge two or more of the current user's own contacts into one surviving
/// record (backlog #88). See `merge_contacts_inner` for the actual logic —
/// this wrapper only adds the `#[server]` request-context auth lookup.
#[server(input = server_fn::codec::Json)]
pub async fn merge_contacts(input: MergeContactsInput) -> Result<ContactDto, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        return merge_contacts_inner(&pool, &username, input).await;
    }
    #[allow(unreachable_code)]
    Err(ServerFnError::new("unreachable"))
}

/// Groups `(contact_id, signals)` pairs into clusters of mutual duplicates
/// (backlog #88's "iOS Contacts-style" suggestions) — a simple union-find
/// over every pair where [`looks_like_duplicate`] is true. Deliberately
/// *not* "same name ⇒ one cluster": two same-named contacts only join a
/// cluster by actually matching the predicate pairwise, so a third
/// same-named-but-different-person contact (matching email absent, e.g.)
/// stays out of a real pair's cluster — see this fn's tests. Bucketing by
/// name first keeps this O(n) in practice for a personal address book
/// instead of true O(n²): only same-named groups are ever compared.
#[cfg(feature = "ssr")]
fn cluster_duplicates(signals: &[(i64, DuplicateSignals)]) -> Vec<Vec<i64>> {
    let mut by_name: std::collections::HashMap<&str, Vec<usize>> = std::collections::HashMap::new();
    for (i, (_, s)) in signals.iter().enumerate() {
        by_name.entry(s.name_key.as_str()).or_default().push(i);
    }

    // Union-find over signals' indices, restricted to comparisons within
    // the same name bucket.
    let mut parent: Vec<usize> = (0..signals.len()).collect();
    fn find(parent: &mut [usize], x: usize) -> usize {
        if parent[x] != x {
            parent[x] = find(parent, parent[x]);
        }
        parent[x]
    }
    fn union(parent: &mut [usize], a: usize, b: usize) {
        let ra = find(parent, a);
        let rb = find(parent, b);
        if ra != rb {
            parent[ra] = rb;
        }
    }

    for indices in by_name.values() {
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                let (a, b) = (indices[i], indices[j]);
                if looks_like_duplicate(&signals[a].1, &signals[b].1) {
                    union(&mut parent, a, b);
                }
            }
        }
    }

    let mut groups: std::collections::HashMap<usize, Vec<i64>> = std::collections::HashMap::new();
    for (i, (id, _)) in signals.iter().enumerate() {
        let root = find(&mut parent, i);
        groups.entry(root).or_default().push(*id);
    }
    groups.into_values().filter(|g| g.len() >= 2).collect()
}

/// Loads every contact's `DuplicateSignals` for `find_possible_duplicates`
/// — reuses [`methods_by_contact`] (already groups `contact_emails`/
/// `contact_phones` by contact id in one query each, same helper
/// `list_contacts` itself uses) and `contact_ids_with_other_info` (one more
/// query) rather than a bespoke per-contact query for either.
#[cfg(feature = "ssr")]
async fn contact_duplicate_signals(
    pool: &SqlitePool,
    username: &str,
) -> Result<Vec<(i64, DuplicateSignals)>, ServerFnError> {
    let names: Vec<(i64, String)> = sqlx::query_as(
        "SELECT c.id, c.display_name FROM contacts c JOIN contact_collections cc ON cc.id = c.collection_id
         WHERE cc.owner_username = ?
            OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = ?)",
    )
    .bind(username)
    .bind(username)
    .fetch_all(pool)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let emails_by_contact = methods_by_contact(pool, "contact_emails", username).await?;
    let phones_by_contact = methods_by_contact(pool, "contact_phones", username).await?;
    let other_info = contact_ids_with_other_info(pool, username).await?;

    Ok(names
        .into_iter()
        .map(|(id, name)| {
            let signals = DuplicateSignals {
                name_key: name.trim().to_lowercase(),
                emails: emails_by_contact
                    .get(&id)
                    .map(|list| list.iter().map(|m| m.value.to_lowercase()).collect())
                    .unwrap_or_default(),
                phones: phones_by_contact
                    .get(&id)
                    .map(|list| list.iter().map(|m| m.value.clone()).collect())
                    .unwrap_or_default(),
                has_other_info: other_info.contains(&id),
            };
            (id, signals)
        })
        .collect())
}

/// Core of `find_possible_duplicates` — factored out for direct testing,
/// same split as `merge_contacts`/`merge_contacts_inner`.
#[cfg(feature = "ssr")]
async fn find_possible_duplicates_inner(
    pool: &SqlitePool,
    username: &str,
) -> Result<Vec<Vec<i64>>, ServerFnError> {
    let signals = contact_duplicate_signals(pool, username).await?;
    Ok(cluster_duplicates(&signals))
}

/// Scans the current user's whole address book for contacts that look like
/// duplicates of each other (backlog #88's "possible duplicates"
/// suggestion, iOS Contacts-style) — unlike `find_duplicate_contact`
/// (which only ever checks one *incoming* vCard against existing rows at
/// import time), this compares every existing contact against every other
/// one, so it also catches duplicates from manual entry or CardDAV sync,
/// neither of which has any dedup check today. Returns clusters (not
/// flat pairs) — each inner `Vec` is a group of 2+ mutually-duplicate
/// contact ids, directly usable as `ContactMergeModal`'s contact list.
#[server]
pub async fn find_possible_duplicates() -> Result<Vec<Vec<i64>>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let (pool, username) = auth().await?;
        return find_possible_duplicates_inner(&pool, &username).await;
    }
    #[allow(unreachable_code)]
    Ok(Vec::new())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::MergedScalarFields;

    #[sqlx::test(migrations = "../../migrations")]
    async fn has_contact_collection_access_true_for_owner(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'Family', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        assert!(has_contact_collection_access(&pool, "alice", id)
            .await
            .unwrap());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn has_contact_collection_access_true_for_member(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0), ('bob', 'bob@example.com', 'bob', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color, shared) VALUES ('alice', 'Family', '#000000', 1) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO contact_collection_members (collection_id, member_username) VALUES (?, 'bob')")
            .bind(id).execute(&pool).await.unwrap();

        assert!(has_contact_collection_access(&pool, "bob", id)
            .await
            .unwrap());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn has_contact_collection_access_false_for_neither(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0), ('eve', 'eve@example.com', 'eve', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let id: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'Family', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        assert!(!has_contact_collection_access(&pool, "eve", id)
            .await
            .unwrap());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_contacts_query_includes_contacts_from_a_shared_collection(
        pool: sqlx::SqlitePool,
    ) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0), ('bob', 'bob@example.com', 'bob', 'x', 0)"
        ).execute(&pool).await.unwrap();

        // Migrations backfill "My Contacts" for users that existed at
        // migration time — a test-created user has none yet (mirrors a
        // real signup, whose first collection is only lazily created by
        // `ensure_default_contact_collection` on first contact write).
        let alice_default: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        let shared_id: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color, shared) VALUES ('alice', 'Family', '#000000', 1) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO contact_collection_members (collection_id, member_username) VALUES (?, 'bob')")
            .bind(shared_id).execute(&pool).await.unwrap();

        sqlx::query(
            "INSERT INTO contacts (owner_username, collection_id, uid, display_name, etag)
             VALUES ('alice', ?, 'u1', 'Jane', 'etag1')",
        )
        .bind(shared_id)
        .execute(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO contacts (owner_username, collection_id, uid, display_name, etag)
             VALUES ('alice', ?, 'u2', 'John (private)', 'etag2')",
        )
        .bind(alice_default)
        .execute(&pool)
        .await
        .unwrap();

        // The query shape `list_contacts` (a #[server] fn, so not directly
        // callable outside a request context) must use.
        let visible: Vec<String> = sqlx::query_scalar(
            "SELECT c.display_name FROM contacts c JOIN contact_collections cc ON cc.id = c.collection_id
             WHERE cc.owner_username = 'bob'
                OR cc.id IN (SELECT collection_id FROM contact_collection_members WHERE member_username = 'bob')
             ORDER BY c.display_name"
        ).fetch_all(&pool).await.unwrap();

        assert_eq!(visible, vec!["Jane".to_string()]);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn list_contacts_exposes_manual_order(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO contacts (owner_username, collection_id, uid, display_name, etag, manual_order) VALUES ('alice', ?, 'u1', 'Ordered', 'etag1', 42.5)"
        ).bind(collection_id).execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO contacts (owner_username, collection_id, uid, display_name, etag) VALUES ('alice', ?, 'u2', 'Unordered', 'etag2')"
        ).bind(collection_id).execute(&pool).await.unwrap();

        let contacts = list_contacts_for(&pool, "alice").await.unwrap();
        let ordered = contacts
            .iter()
            .find(|c| c.display_name == "Ordered")
            .unwrap();
        let unordered = contacts
            .iter()
            .find(|c| c.display_name == "Unordered")
            .unwrap();
        assert_eq!(ordered.manual_order, Some(42.5));
        assert_eq!(unordered.manual_order, None);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn has_contact_collection_access_gates_a_non_member(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0), ('eve', 'eve@example.com', 'eve', 'x', 0)"
        ).execute(&pool).await.unwrap();

        let alice_default: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        sqlx::query(
            "INSERT INTO contacts (owner_username, collection_id, uid, display_name, etag)
             VALUES ('alice', ?, 'u1', 'Jane', 'etag1')",
        )
        .bind(alice_default)
        .execute(&pool)
        .await
        .unwrap();

        assert!(!has_contact_collection_access(&pool, "eve", alice_default)
            .await
            .unwrap());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn find_contact_by_name_matches_case_and_whitespace_insensitively(
        pool: sqlx::SqlitePool,
    ) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let coll: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        let mut conn = pool.acquire().await.unwrap();
        create_contact_from_vcard(
            &mut conn,
            "alice",
            coll,
            &sample_vcard_fields("Jane Doe", None, None),
            (0, 0),
        )
        .await
        .unwrap();

        let found = find_contact_by_name(&pool, "alice", "  jane doe  ")
            .await
            .unwrap();
        assert!(
            found.is_some(),
            "case/whitespace-insensitive match should hit"
        );

        let missing = find_contact_by_name(&pool, "alice", "Bob").await.unwrap();
        assert!(missing.is_none());

        // Scoped to the calling user's own address book — no cross-user lookup.
        let wrong_user = find_contact_by_name(&pool, "someone-else", "Jane Doe")
            .await
            .unwrap();
        assert!(wrong_user.is_none());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn create_and_list_contact_round_trips_methods(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();

        let input = NewContact {
            display_name: "  Jane Doe  ".to_string(),
            org: Some("Acme".to_string()),
            emails: vec![ContactMethod {
                value: "jane@acme.test".to_string(),
                kind: "work".to_string(),
                is_primary: true,
            }],
            phones: vec![],
            ..Default::default()
        };

        let mut tx = pool.begin().await.unwrap();
        let (grid_row, grid_col) = place_contact(&mut tx, "alice", None, 1).await.unwrap();
        assert_eq!((grid_row, grid_col), (0, 0));
        let id: i64 = sqlx::query(
            "INSERT INTO contacts (owner_username, uid, display_name, org, etag, grid_row, grid_col, grid_row_span)
             VALUES ('alice', 'u1', ?, ?, 'etag', ?, ?, 1) RETURNING id",
        )
        .bind(input.display_name.trim())
        .bind(&input.org)
        .bind(grid_row)
        .bind(grid_col)
        .fetch_one(&mut *tx)
        .await
        .unwrap()
        .try_get("id")
        .unwrap();
        replace_methods(&mut tx, "contact_emails", id, &input.emails)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        let dto = fetch_contact(&pool, id).await.unwrap().unwrap();
        assert_eq!(dto.display_name, "Jane Doe");
        assert_eq!(dto.org.as_deref(), Some("Acme"));
        assert_eq!(dto.emails.len(), 1);
        assert_eq!(dto.emails[0].value, "jane@acme.test");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn bump_contacts_sync_token_creates_row_and_logs_change(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        // No `address_books` row exists yet for a brand-new collection —
        // must be created lazily, not error, so a CardDAV client's first
        // sync-collection REPORT (which reads this table) isn't blocked on
        // some other eager-provisioning step that doesn't exist (see
        // backlog #29's Resolution).
        bump_contacts_sync_token(&pool, collection_id, "uid-1", "modified")
            .await
            .unwrap();

        let token: i64 =
            sqlx::query_scalar("SELECT sync_token FROM address_books WHERE collection_id = ?")
                .bind(collection_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(token, 1);

        let logged: (String, String) =
            sqlx::query_as("SELECT uid, operation FROM contact_sync_log WHERE collection_id = ?")
                .bind(collection_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(logged, ("uid-1".to_string(), "modified".to_string()));

        bump_contacts_sync_token(&pool, collection_id, "uid-2", "deleted")
            .await
            .unwrap();
        let token: i64 =
            sqlx::query_scalar("SELECT sync_token FROM address_books WHERE collection_id = ?")
                .bind(collection_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            token, 2,
            "a second bump must increment, not reset, the token"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn clear_contacts_for_deletes_all_and_logs_one_batch_revision(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('bob', 'bob@example.com', 'bob', 'x', 0)"
        ).execute(&pool).await.unwrap();

        let alice_coll: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let bob_coll: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('bob', 'My Contacts', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        for (owner, coll, uid) in [
            ("alice", alice_coll, "a1"),
            ("alice", alice_coll, "a2"),
            ("bob", bob_coll, "b1"),
        ] {
            sqlx::query(
                "INSERT INTO contacts (owner_username, collection_id, uid, display_name, etag, grid_row, grid_col, grid_row_span)
                 VALUES (?, ?, ?, 'Someone', 'etag', 0, 0, 1)",
            )
            .bind(owner)
            .bind(coll)
            .bind(uid)
            .execute(&pool)
            .await
            .unwrap();
        }

        clear_contacts_for(&pool, "alice", None).await.unwrap();

        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM contacts WHERE owner_username = 'alice'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(remaining, 0, "all of alice's contacts must be deleted");

        let bob_remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM contacts WHERE owner_username = 'bob'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            bob_remaining, 1,
            "clearing alice must not touch bob's contacts"
        );

        let mut logged: Vec<(String, String, i64)> = sqlx::query_as(
            "SELECT uid, operation, token FROM contact_sync_log WHERE collection_id = ? ORDER BY uid",
        )
        .bind(alice_coll)
        .fetch_all(&pool)
        .await
        .unwrap();
        logged.sort_by(|a, b| a.0.cmp(&b.0));
        assert_eq!(
            logged,
            vec![
                ("a1".to_string(), "deleted".to_string(), 1),
                ("a2".to_string(), "deleted".to_string(), 1),
            ],
            "every cleared uid is logged as deleted, sharing the batch's single new token"
        );

        let token: i64 =
            sqlx::query_scalar("SELECT sync_token FROM address_books WHERE collection_id = ?")
                .bind(alice_coll)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            token, 1,
            "the whole batch bumps the address book token exactly once"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn clear_contacts_for_is_a_noop_when_there_are_no_contacts(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        clear_contacts_for(&pool, "alice", None).await.unwrap();

        let token: Option<i64> =
            sqlx::query_scalar("SELECT sync_token FROM address_books WHERE collection_id = ?")
                .bind(collection_id)
                .fetch_optional(&pool)
                .await
                .unwrap();
        assert_eq!(
            token, None,
            "no address_books row should be created for an empty clear"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn auto_layout_contacts_places_alphabetically_by_display_name(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        // Inserted in a deliberately non-alphabetical (and non-lexically-cased)
        // creation order — auto-layout must re-sort by display_name regardless.
        for (uid, name, row) in [("c1", "charlie", 0), ("c2", "Alice", 1), ("c3", "bob", 2)] {
            sqlx::query(
                "INSERT INTO contacts (owner_username, collection_id, uid, display_name, etag, grid_row, grid_col, grid_row_span)
                 VALUES ('alice', ?, ?, ?, 'etag', ?, 0, 1)",
            )
            .bind(collection_id)
            .bind(uid)
            .bind(name)
            .bind(row)
            .execute(&pool)
            .await
            .unwrap();
        }

        // Full real column count so `pack_for_view`'s two-level packing
        // collapses to a single masonry pass and reading order (grid_row
        // then grid_col) is unambiguous — see `grid::pack_for_view`'s doc
        // comment for why `view_cols < CARD_GRID_COLS` spreads same-row
        // items across real columns instead.
        auto_layout_contacts_for(&pool, "alice", CARD_GRID_COLS)
            .await
            .unwrap();

        let ordered: Vec<String> = sqlx::query_scalar(
            "SELECT display_name FROM contacts WHERE owner_username = 'alice'
             ORDER BY grid_row ASC, grid_col ASC",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(
            ordered,
            vec![
                "Alice".to_string(),
                "bob".to_string(),
                "charlie".to_string()
            ],
            "auto-layout must place contacts alphabetically (case-insensitively) by display_name"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn auto_layout_contacts_sorts_manually_ordered_before_alphabetical(
        pool: sqlx::SqlitePool,
    ) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        // Alphabetically "Aardvark" < "Zebra" < "Manual", but "Manual" has
        // an explicit manual_order that should place it first.
        for (uid, name, order) in [
            ("u1", "Aardvark", None),
            ("u2", "Zebra", None),
            ("u3", "Manual", Some(5.0)),
        ] {
            sqlx::query(
                "INSERT INTO contacts (owner_username, collection_id, uid, display_name, etag, manual_order, grid_row, grid_col, grid_row_span)
                 VALUES ('alice', ?, ?, ?, 'etag', ?, 0, 0, 1)",
            )
            .bind(collection_id).bind(uid).bind(name).bind(order)
            .execute(&pool).await.unwrap();
        }

        auto_layout_contacts_for(&pool, "alice", CARD_GRID_COLS)
            .await
            .unwrap();

        let ordered: Vec<String> = sqlx::query_scalar(
            "SELECT display_name FROM contacts WHERE owner_username = 'alice' ORDER BY grid_row ASC, grid_col ASC",
        )
        .fetch_all(&pool).await.unwrap();
        assert_eq!(
            ordered,
            vec!["Manual".to_string(), "Aardvark".to_string(), "Zebra".to_string()],
            "the explicitly manual_order'd contact must sort before the alphabetical ones, which sort among themselves"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn auto_layout_contacts_also_repacks_a_shared_collections_contacts(
        pool: sqlx::SqlitePool,
    ) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0), ('bob', 'bob@example.com', 'bob', 'x', 0)"
        ).execute(&pool).await.unwrap();

        let alice_default: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let shared: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color, shared) VALUES ('alice', 'Shared', '#22c55e', 1) RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        sqlx::query("INSERT INTO contact_collection_members (collection_id, member_username) VALUES (?, 'bob')")
            .bind(shared).execute(&pool).await.unwrap();

        sqlx::query(
            "INSERT INTO contacts (owner_username, collection_id, uid, display_name, etag, grid_row, grid_col, grid_row_span)
             VALUES ('alice', ?, 'c1', 'Owner Contact', 'etag', 0, 0, 1)"
        ).bind(alice_default).execute(&pool).await.unwrap();
        // Created by bob (a member, not the owner) directly in the shared
        // collection — bob has full read-write access to it (backlog #81's
        // single 'member' tier), so a contact he created there has
        // owner_username = 'bob', not 'alice'.
        sqlx::query(
            "INSERT INTO contacts (owner_username, collection_id, uid, display_name, etag, grid_row, grid_col, grid_row_span)
             VALUES ('bob', ?, 'c2', 'Shared Contact', 'etag', 500, 0, 1)"
        ).bind(shared).execute(&pool).await.unwrap();

        // alice runs Auto-layout — must reach bob's contact too, since it's
        // in a collection alice owns, not just her own owner_username rows.
        auto_layout_contacts_for(&pool, "alice", CARD_GRID_COLS)
            .await
            .unwrap();

        let row: i32 = sqlx::query_scalar("SELECT grid_row FROM contacts WHERE uid = 'c2'")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_ne!(
            row, 500,
            "bob's contact in alice's shared collection must be repacked, not skipped"
        );
    }

    #[test]
    fn validate_new_contact_rejects_blank_name() {
        let input = NewContact::default();
        assert!(validate_new_contact(&input).is_err());
    }

    #[test]
    fn validate_new_contact_rejects_bad_method_kind() {
        let input = NewContact {
            display_name: "Jane".to_string(),
            emails: vec![ContactMethod {
                value: "j@x.test".to_string(),
                kind: "carrier-pigeon".to_string(),
                is_primary: false,
            }],
            ..Default::default()
        };
        assert!(validate_new_contact(&input).is_err());
    }

    #[test]
    fn union_methods_dedupes_case_insensitively_and_keeps_first_primary() {
        let a = vec![ContactMethod {
            value: "Jane@Work.com".to_string(),
            kind: "work".to_string(),
            is_primary: true,
        }];
        let b = vec![
            ContactMethod {
                value: "jane@work.com".to_string(),
                kind: "work".to_string(),
                is_primary: true,
            },
            ContactMethod {
                value: "jane@home.com".to_string(),
                kind: "home".to_string(),
                is_primary: false,
            },
        ];
        let merged = union_methods([a, b].into_iter());
        assert_eq!(
            merged.len(),
            2,
            "the duplicate work email must be dropped: {merged:?}"
        );
        assert_eq!(
            merged.iter().filter(|m| m.is_primary).count(),
            1,
            "only one survivor keeps is_primary"
        );
        assert!(
            merged[0].is_primary,
            "the first list's primary wins the tie"
        );
    }

    #[test]
    fn union_methods_skips_blank_values() {
        let a = vec![ContactMethod {
            value: "  ".to_string(),
            kind: "other".to_string(),
            is_primary: false,
        }];
        assert!(union_methods([a].into_iter()).is_empty());
    }

    #[test]
    fn union_addresses_dedupes_identical_addresses() {
        let addr = ContactAddress {
            kind: "home".to_string(),
            is_primary: true,
            street: Some("1 Main St".to_string()),
            city: Some("Springfield".to_string()),
            region: None,
            postal_code: None,
            country: None,
        };
        let merged = union_addresses([vec![addr.clone()], vec![addr]].into_iter());
        assert_eq!(merged.len(), 1);
    }

    #[test]
    fn union_categories_dedupes_case_insensitively() {
        let merged = union_categories(
            [
                vec!["VIP".to_string()],
                vec!["vip".to_string(), "Colleague".to_string()],
            ]
            .into_iter(),
        );
        assert_eq!(merged, vec!["VIP".to_string(), "Colleague".to_string()]);
    }

    fn sig(name: &str, emails: &[&str], phones: &[&str], has_other_info: bool) -> DuplicateSignals {
        DuplicateSignals {
            name_key: name.trim().to_lowercase(),
            emails: emails.iter().map(|s| s.to_lowercase()).collect(),
            phones: phones.iter().map(|s| s.to_string()).collect(),
            has_other_info,
        }
    }

    #[test]
    fn looks_like_duplicate_false_when_contact_info_present_but_non_overlapping() {
        // Same name, but one side has only an email and the other only a
        // phone — no overlapping value on either side, so not a match, even
        // though both sides do have *some* contact info.
        let a = sig("Jane Doe", &["jane@work.test"], &[], false);
        let b = sig("Jane Doe", &[], &["+15551234567"], false);
        assert!(!looks_like_duplicate(&a, &b));
    }

    #[test]
    fn looks_like_duplicate_false_when_same_type_but_different_value() {
        let a = sig("Jane Doe", &["jane@work.test"], &[], false);
        let b = sig("Jane Doe", &["jane@personal.test"], &[], false);
        assert!(!looks_like_duplicate(&a, &b));
    }

    #[test]
    fn looks_like_duplicate_true_when_neither_side_has_any_info_at_all() {
        let a = sig("Jane Doe", &[], &[], false);
        let b = sig("Jane Doe", &[], &[], false);
        assert!(looks_like_duplicate(&a, &b));
    }

    #[test]
    fn looks_like_duplicate_true_on_shared_email_case_insensitive() {
        let a = sig("Jane Doe", &["jane@work.test"], &[], false);
        let b = sig("Jane Doe", &["JANE@WORK.TEST"], &["+15551234567"], false);
        assert!(looks_like_duplicate(&a, &b));
    }

    #[test]
    fn looks_like_duplicate_false_when_names_differ() {
        let a = sig("Jane Doe", &[], &[], false);
        let b = sig("John Doe", &[], &[], false);
        assert!(!looks_like_duplicate(&a, &b));
    }

    /// The exact "Alex" scenario from this session: same name, no email/
    /// phone on either side, but one side has real data (a URL) on file
    /// that isn't email/phone. Must NOT match — that data would be lost
    /// on import skip if it did.
    #[test]
    fn looks_like_duplicate_false_when_one_side_has_other_info_the_other_lacks() {
        let a = sig("Alex", &[], &[], false); // e.g. a totally bare existing contact
        let b = sig("Alex", &[], &[], true); // incoming vCard has a URL, no email/phone
        assert!(!looks_like_duplicate(&a, &b));
    }

    #[test]
    fn looks_like_duplicate_false_when_existing_has_email_incoming_has_only_other_info() {
        // This session's literal example: existing "Alex" has an email;
        // incoming "Alex" vCard has only a URL, no email/phone.
        let a = sig("Alex", &["alex@example.com"], &[], false);
        let b = sig("Alex", &[], &[], true);
        assert!(!looks_like_duplicate(&a, &b));
    }

    #[test]
    fn cluster_duplicates_groups_three_mutually_matching_contacts() {
        let signals = vec![
            (1, sig("Jane Doe", &["jane@work.test"], &[], false)),
            (2, sig("Jane Doe", &["jane@work.test"], &[], false)),
            (
                3,
                sig("Jane Doe", &["jane@work.test"], &["+15551234567"], false),
            ),
        ];
        let clusters = cluster_duplicates(&signals);
        assert_eq!(clusters.len(), 1);
        let mut c = clusters[0].clone();
        c.sort_unstable();
        assert_eq!(c, vec![1, 2, 3]);
    }

    #[test]
    fn cluster_duplicates_groups_three_mutually_bare_contacts_by_name_alone() {
        // All three are genuinely bare (no email/phone/other info) — the
        // only case where a pure name-only match is safe, since none of
        // them has data that could be lost by clustering wrongly.
        let signals = vec![
            (1, sig("Jane Doe", &[], &[], false)),
            (2, sig("Jane Doe", &[], &[], false)),
            (3, sig("Jane Doe", &[], &[], false)),
        ];
        let clusters = cluster_duplicates(&signals);
        assert_eq!(clusters.len(), 1);
        let mut c = clusters[0].clone();
        c.sort_unstable();
        assert_eq!(c, vec![1, 2, 3]);
    }

    #[test]
    fn cluster_duplicates_excludes_a_bare_contact_from_a_real_pairs_cluster() {
        // Contacts 1 and 2 share an email (real dupe). Contact 3 has the
        // same name but is bare — since contact 1/2 have real data on
        // file, the name-only fallback must not pull 3 in with them (that
        // would be exactly the "Alex" data-loss bug this session found).
        let signals = vec![
            (1, sig("Jane Doe", &["jane@work.test"], &[], false)),
            (2, sig("Jane Doe", &["jane@work.test"], &[], false)),
            (3, sig("Jane Doe", &[], &[], false)),
        ];
        let clusters = cluster_duplicates(&signals);
        assert_eq!(
            clusters.len(),
            1,
            "only the two real dupes cluster: {clusters:?}"
        );
        let mut c = clusters[0].clone();
        c.sort_unstable();
        assert_eq!(c, vec![1, 2]);
    }

    #[test]
    fn cluster_duplicates_keeps_a_same_named_non_matching_contact_separate() {
        // Two share an email (real dupe); a third has the same name but a
        // *different* email — genuinely a different person, must not be
        // pulled into the cluster just for sharing the name.
        let signals = vec![
            (1, sig("Jane Doe", &["jane@work.test"], &[], false)),
            (2, sig("Jane Doe", &["jane@work.test"], &[], false)),
            (3, sig("Jane Doe", &["someone.else@test.test"], &[], false)),
        ];
        let clusters = cluster_duplicates(&signals);
        assert_eq!(
            clusters.len(),
            1,
            "only the two real dupes cluster: {clusters:?}"
        );
        let mut c = clusters[0].clone();
        c.sort_unstable();
        assert_eq!(c, vec![1, 2]);
    }

    #[test]
    fn cluster_duplicates_ignores_a_solo_contact() {
        let signals = vec![(1, sig("Jane Doe", &[], &[], false))];
        assert!(cluster_duplicates(&signals).is_empty());
    }

    #[test]
    fn cluster_duplicates_ignores_distinct_names() {
        let signals = vec![
            (1, sig("Jane Doe", &[], &[], false)),
            (2, sig("John Smith", &[], &[], false)),
        ];
        assert!(cluster_duplicates(&signals).is_empty());
    }

    #[test]
    fn cluster_duplicates_keeps_bare_contact_separate_from_one_with_other_info() {
        // Same "Alex" scenario as `looks_like_duplicate`'s test, at the
        // clustering level: a bare contact and one with only a URL must
        // not cluster together.
        let signals = vec![
            (1, sig("Alex", &[], &[], false)),
            (2, sig("Alex", &[], &[], true)),
        ];
        assert!(cluster_duplicates(&signals).is_empty());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn find_possible_duplicates_finds_a_real_pair_and_ignores_solo_contacts(
        pool: sqlx::SqlitePool,
    ) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();

        let a = insert_test_contact(&pool, "alice", "Jane Doe").await;
        let b = insert_test_contact(&pool, "alice", "Jane Doe").await;
        let _solo = insert_test_contact(&pool, "alice", "Unique Person").await;
        sqlx::query("INSERT INTO contact_emails (contact_id, value, type, is_primary) VALUES (?, 'jane@work.test', 'work', 1)")
            .bind(a).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO contact_emails (contact_id, value, type, is_primary) VALUES (?, 'jane@work.test', 'work', 1)")
            .bind(b).execute(&pool).await.unwrap();

        let clusters = find_possible_duplicates_inner(&pool, "alice")
            .await
            .unwrap();
        assert_eq!(clusters.len(), 1);
        let mut c = clusters[0].clone();
        c.sort_unstable();
        assert_eq!(c, vec![a.min(b), a.max(b)]);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn find_possible_duplicates_is_scoped_to_the_calling_user(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0), ('eve', 'eve@example.com', 'eve', 'x', 0)"
        ).execute(&pool).await.unwrap();
        insert_test_contact(&pool, "alice", "Jane Doe").await;
        insert_test_contact(&pool, "eve", "Jane Doe").await;
        insert_test_contact(&pool, "eve", "Jane Doe").await;

        let clusters = find_possible_duplicates_inner(&pool, "alice")
            .await
            .unwrap();
        assert!(
            clusters.is_empty(),
            "eve's duplicate pair must not leak into alice's results"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn find_possible_duplicates_end_to_end_alex_scenario(pool: sqlx::SqlitePool) {
        // This session's exact scenario, exercised through the real DB
        // path (not just the pure `looks_like_duplicate`/`cluster_duplicates`
        // unit tests): a contact with an email and a same-named contact
        // with only a URL must not be suggested as duplicates.
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let with_email = insert_test_contact(&pool, "alice", "Alex").await;
        let with_url = insert_test_contact(&pool, "alice", "Alex").await;
        sqlx::query("INSERT INTO contact_emails (contact_id, value, type, is_primary) VALUES (?, 'alex@example.com', 'work', 1)")
            .bind(with_email).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO contact_urls (contact_id, value, type, is_primary) VALUES (?, 'garage.example.com', 'other', 0)")
            .bind(with_url).execute(&pool).await.unwrap();

        let clusters = find_possible_duplicates_inner(&pool, "alice")
            .await
            .unwrap();
        assert!(
            clusters.is_empty(),
            "email-only vs. URL-only Alex must not be suggested as duplicates: {clusters:?}"
        );
    }

    #[test]
    fn vcard_has_other_info_true_for_a_url_with_no_email_or_phone() {
        let mut fields = sample_vcard_fields("Alex", None, None);
        fields.urls.push(tidl_carddav::vcard::VCardMethod {
            value: "garage.example.com".to_string(),
            kind: "other".to_string(),
            is_primary: false,
        });
        assert!(vcard_has_other_info(&fields));
    }

    #[test]
    fn vcard_has_other_info_false_for_bare_name_only() {
        assert!(!vcard_has_other_info(&sample_vcard_fields(
            "Alex", None, None
        )));
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn contact_ids_with_other_info_finds_a_contact_with_only_a_url(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let bare = insert_test_contact(&pool, "alice", "Bare Contact").await;
        let with_url = insert_test_contact(&pool, "alice", "Alex").await;
        sqlx::query("INSERT INTO contact_urls (contact_id, value, type, is_primary) VALUES (?, 'garage.example.com', 'other', 0)")
            .bind(with_url).execute(&pool).await.unwrap();

        let ids = contact_ids_with_other_info(&pool, "alice").await.unwrap();
        assert!(!ids.contains(&bare));
        assert!(ids.contains(&with_url));
    }

    #[test]
    fn decode_and_validate_photo_rejects_unsupported_content_type() {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"hello");
        assert!(decode_and_validate_photo(&b64, "image/gif").is_err());
    }

    #[test]
    fn decode_and_validate_photo_rejects_invalid_base64() {
        assert!(decode_and_validate_photo("not-valid-base64!!", "image/jpeg").is_err());
    }

    #[test]
    fn decode_and_validate_photo_rejects_over_the_size_cap() {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(vec![0u8; MAX_PHOTO_BYTES + 1]);
        assert!(decode_and_validate_photo(&b64, "image/jpeg").is_err());
    }

    #[test]
    fn decode_and_validate_photo_accepts_jpeg_and_png() {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode(b"hello");
        assert_eq!(
            decode_and_validate_photo(&b64, "image/jpeg").unwrap(),
            b"hello"
        );
        assert_eq!(
            decode_and_validate_photo(&b64, "image/png").unwrap(),
            b"hello"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn photo_upload_and_delete_round_trip_and_bump_sync_token(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        let mut tx = pool.begin().await.unwrap();
        let (grid_row, grid_col) = place_contact(&mut tx, "alice", None, 1).await.unwrap();
        let id: i64 = sqlx::query(
            "INSERT INTO contacts (owner_username, collection_id, uid, display_name, etag, grid_row, grid_col, grid_row_span)
             VALUES ('alice', ?, 'uid-1', 'Jane Doe', 'etag', ?, ?, 1) RETURNING id",
        )
        .bind(collection_id)
        .bind(grid_row)
        .bind(grid_col)
        .fetch_one(&mut *tx)
        .await
        .unwrap()
        .try_get("id")
        .unwrap();
        tx.commit().await.unwrap();

        let dto = fetch_contact(&pool, id).await.unwrap().unwrap();
        assert!(!dto.has_photo);

        // Mirrors upload_contact_photo's SQL directly — the #[server] fn
        // itself needs a request context this test doesn't have (see
        // create_and_list_contact_round_trips_methods for the same
        // work-around, already established in this file).
        sqlx::query("UPDATE contacts SET photo = ?, photo_content_type = ? WHERE id = ?")
            .bind(b"hello".to_vec())
            .bind("image/jpeg")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        bump_contacts_sync_token(&pool, collection_id, "uid-1", "modified")
            .await
            .unwrap();

        let dto = fetch_contact(&pool, id).await.unwrap().unwrap();
        assert!(dto.has_photo);
        let token: i64 =
            sqlx::query_scalar("SELECT sync_token FROM address_books WHERE collection_id = ?")
                .bind(collection_id)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(token, 1);

        sqlx::query("UPDATE contacts SET photo = NULL, photo_content_type = NULL WHERE id = ?")
            .bind(id)
            .execute(&pool)
            .await
            .unwrap();
        let dto = fetch_contact(&pool, id).await.unwrap().unwrap();
        assert!(!dto.has_photo);
    }

    // ── Contacts import (.vcf, backlog #87) ──────────────────────────────

    #[test]
    fn split_vcards_finds_every_vcard_in_a_multi_contact_file() {
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:u1\r\nFN:Ada\r\nEND:VCARD\r\n\
                     BEGIN:VCARD\r\nVERSION:3.0\r\nUID:u2\r\nFN:Grace\r\nEND:VCARD\r\n";
        let blocks = split_vcards(text);
        assert_eq!(blocks.len(), 2);
        assert!(vcard::parse_vcard(&blocks[0]).unwrap().display_name == "Ada");
        assert!(vcard::parse_vcard(&blocks[1]).unwrap().display_name == "Grace");
    }

    #[test]
    fn split_vcards_keeps_folded_continuation_lines_inside_their_block() {
        let text = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:u1\r\nFN:Ada\r\n\
                     NOTE:Line one\r\n continued\r\nEND:VCARD\r\n";
        let blocks = split_vcards(text);
        assert_eq!(blocks.len(), 1);
        let fields = vcard::parse_vcard(&blocks[0]).unwrap();
        assert_eq!(fields.notes.as_deref(), Some("Line onecontinued"));
    }

    #[test]
    fn split_vcards_ignores_text_outside_any_vcard_block() {
        let text = "Some preamble\r\nBEGIN:VCARD\r\nVERSION:3.0\r\nUID:u1\r\nFN:Ada\r\nEND:VCARD\r\ntrailing junk\r\n";
        let blocks = split_vcards(text);
        assert_eq!(blocks.len(), 1);
    }

    fn sample_vcard_fields(name: &str, email: Option<&str>, phone: Option<&str>) -> VCardFields {
        VCardFields {
            uid: format!("uid-{name}"),
            display_name: name.to_string(),
            emails: email
                .map(|e| {
                    vec![tidl_carddav::vcard::VCardMethod {
                        value: e.to_string(),
                        kind: "work".to_string(),
                        is_primary: true,
                    }]
                })
                .unwrap_or_default(),
            phones: phone
                .map(|p| {
                    vec![tidl_carddav::vcard::VCardMethod {
                        value: p.to_string(),
                        kind: "mobile".to_string(),
                        is_primary: false,
                    }]
                })
                .unwrap_or_default(),
            ..Default::default()
        }
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn find_duplicate_contact_matches_on_name_and_shared_email(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let coll: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        let mut conn = pool.acquire().await.unwrap();
        create_contact_from_vcard(
            &mut conn,
            "alice",
            coll,
            &sample_vcard_fields("Jane Doe", Some("jane@acme.test"), None),
            (0, 0),
        )
        .await
        .unwrap();

        let other_info = std::collections::HashSet::new();

        // Same name, same email → duplicate.
        let dup = find_duplicate_contact(
            &mut conn,
            "alice",
            &sample_vcard_fields("Jane Doe", Some("jane@acme.test"), None),
            &other_info,
        )
        .await
        .unwrap();
        assert!(dup.is_some());

        // Same name, different email → not a duplicate (email is the
        // disambiguating signal once one is present).
        let not_dup = find_duplicate_contact(
            &mut conn,
            "alice",
            &sample_vcard_fields("Jane Doe", Some("someone.else@acme.test"), None),
            &other_info,
        )
        .await
        .unwrap();
        assert!(not_dup.is_none());

        // Different name entirely → not a duplicate.
        let not_dup2 = find_duplicate_contact(
            &mut conn,
            "alice",
            &sample_vcard_fields("John Smith", Some("jane@acme.test"), None),
            &other_info,
        )
        .await
        .unwrap();
        assert!(not_dup2.is_none());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn find_duplicate_contact_falls_back_to_name_only_with_no_contact_info(
        pool: sqlx::SqlitePool,
    ) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let coll: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        let mut conn = pool.acquire().await.unwrap();
        create_contact_from_vcard(
            &mut conn,
            "alice",
            coll,
            &sample_vcard_fields("Jane Doe", None, None),
            (0, 0),
        )
        .await
        .unwrap();

        let dup = find_duplicate_contact(
            &mut conn,
            "alice",
            &sample_vcard_fields("Jane Doe", None, None),
            &std::collections::HashSet::new(),
        )
        .await
        .unwrap();
        assert!(
            dup.is_some(),
            "name-only match is the only signal available when neither vCard has an email/phone"
        );
    }

    #[test]
    fn vcard_content_lines_counts_every_optional_card_line() {
        assert_eq!(
            vcard_content_lines(&sample_vcard_fields("Jane Doe", None, None)),
            0,
            "bare name/uid only renders the header, no optional lines"
        );
        assert_eq!(
            vcard_content_lines(&sample_vcard_fields(
                "Jane Doe",
                Some("jane@acme.test"),
                None
            )),
            1
        );
        assert_eq!(
            vcard_content_lines(&sample_vcard_fields(
                "Jane Doe",
                Some("jane@acme.test"),
                Some("+1 555 0100")
            )),
            2,
            "one line per email and per phone"
        );

        let heavy = VCardFields {
            org: Some("Acme".to_string()),
            title: Some("Engineer".to_string()),
            notes: Some("met at a conference".to_string()),
            urls: vec![tidl_carddav::vcard::VCardMethod {
                value: "https://example.com".to_string(),
                kind: "home".to_string(),
                is_primary: true,
            }],
            emails: vec![
                tidl_carddav::vcard::VCardMethod {
                    value: "a@example.com".to_string(),
                    kind: "work".to_string(),
                    is_primary: true,
                },
                tidl_carddav::vcard::VCardMethod {
                    value: "b@example.com".to_string(),
                    kind: "home".to_string(),
                    is_primary: false,
                },
            ],
            ..sample_vcard_fields("Heavy Contact", None, None)
        };
        // title+org share one subtitle line, +1 url, +2 emails, +1 notes = 5.
        assert_eq!(vcard_content_lines(&heavy), 5);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn import_vcf_creates_new_contacts_and_skips_or_overwrites_duplicates(
        pool: sqlx::SqlitePool,
    ) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let coll: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();

        let mut conn = pool.acquire().await.unwrap();
        create_contact_from_vcard(
            &mut conn,
            "alice",
            coll,
            &sample_vcard_fields("Jane Doe", Some("jane@acme.test"), None),
            (0, 0),
        )
        .await
        .unwrap();

        let vcf = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:new-1\r\nFN:New Person\r\n\
                    EMAIL;TYPE=WORK:new@acme.test\r\nEND:VCARD\r\n\
                    BEGIN:VCARD\r\nVERSION:3.0\r\nUID:dup-1\r\nFN:Jane Doe\r\n\
                    EMAIL;TYPE=WORK:jane@acme.test\r\nORG:Acme 2\r\nEND:VCARD\r\n";

        // overwrite = false: the duplicate is skipped, only the new contact is created.
        let rows: Vec<i64> =
            sqlx::query_scalar("SELECT id FROM contacts WHERE owner_username = 'alice'")
                .fetch_all(&pool)
                .await
                .unwrap();
        assert_eq!(rows.len(), 1);

        let other_info = contact_ids_with_other_info(&pool, "alice").await.unwrap();
        for block in split_vcards(vcf) {
            let fields = vcard::parse_vcard(&block).unwrap();
            match find_duplicate_contact(&mut conn, "alice", &fields, &other_info)
                .await
                .unwrap()
            {
                Some(_) => {}
                None => {
                    create_contact_from_vcard(&mut conn, "alice", coll, &fields, (0, 0))
                        .await
                        .unwrap();
                }
            }
        }
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM contacts WHERE owner_username = 'alice'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 2, "duplicate skipped, only the new contact created");

        // overwrite = true: the duplicate's org is updated in place, no new row.
        let other_info = contact_ids_with_other_info(&pool, "alice").await.unwrap();
        for block in split_vcards(vcf) {
            let fields = vcard::parse_vcard(&block).unwrap();
            if let Some(id) = find_duplicate_contact(&mut conn, "alice", &fields, &other_info)
                .await
                .unwrap()
            {
                apply_vcard_to_contact(&mut conn, id, &fields)
                    .await
                    .unwrap();
            }
        }
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM contacts WHERE owner_username = 'alice'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(count, 2, "overwrite updates in place, doesn't add a row");
        let org: Option<String> = sqlx::query_scalar(
            "SELECT org FROM contacts WHERE owner_username = 'alice' AND display_name = 'Jane Doe'",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        assert_eq!(org.as_deref(), Some("Acme 2"));
    }

    /// Inserts a minimal contact row directly (bypassing `create_contact`,
    /// which needs a live `#[server]` request context) and returns its id.
    #[cfg(test)]
    async fn insert_test_contact(pool: &sqlx::SqlitePool, owner: &str, display_name: &str) -> i64 {
        // Idempotent per-owner default collection — mirrors
        // `ensure_default_contact_collection` — so repeated calls for the
        // same `owner` within one test share one collection instead of
        // tripping the `UNIQUE(owner_username, name)` constraint.
        sqlx::query(
            "INSERT INTO contact_collections (owner_username, name, color)
             VALUES (?, 'My Contacts', '#3b82f6')
             ON CONFLICT(owner_username, name) DO NOTHING",
        )
        .bind(owner)
        .execute(pool)
        .await
        .unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "SELECT id FROM contact_collections WHERE owner_username = ? AND name = 'My Contacts'",
        )
        .bind(owner)
        .fetch_one(pool)
        .await
        .unwrap();

        sqlx::query_scalar(
            "INSERT INTO contacts (owner_username, collection_id, uid, display_name, etag)
             VALUES (?, ?, ?, ?, 'etag-x') RETURNING id",
        )
        .bind(owner)
        .bind(collection_id)
        .bind(format!("contact:{}", uuid::Uuid::new_v4()))
        .bind(display_name)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn merge_contacts_unions_emails_and_deletes_merged_away(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();

        let primary = insert_test_contact(&pool, "alice", "Jane Doe").await;
        let other = insert_test_contact(&pool, "alice", "Jane D.").await;
        sqlx::query("INSERT INTO contact_emails (contact_id, value, type, is_primary) VALUES (?, 'jane@work.com', 'work', 1)")
            .bind(primary).execute(&pool).await.unwrap();
        sqlx::query("INSERT INTO contact_emails (contact_id, value, type, is_primary) VALUES (?, 'jane@home.com', 'home', 0)")
            .bind(other).execute(&pool).await.unwrap();

        let input = MergeContactsInput {
            primary_id: primary,
            merged_ids: vec![other],
            resolved: MergedScalarFields {
                display_name: "Jane Doe".to_string(),
                ..Default::default()
            },
            keep_photo_from: None,
        };
        let result = merge_contacts_inner(&pool, "alice", input).await.unwrap();

        assert_eq!(result.id, primary);
        assert_eq!(
            result.emails.len(),
            2,
            "both emails must survive: {:?}",
            result.emails
        );

        let remaining: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM contacts WHERE owner_username = 'alice'")
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(remaining, 1, "the merged-away contact must be deleted");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn merge_contacts_repoints_mentions_in_own_notes(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color) VALUES ('alice', 'Inbox', '#000000') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let note_id: i64 = sqlx::query_scalar(
            "INSERT INTO notes (collection_id, author_username, text) VALUES (?, 'alice', 'Ping @Jane D. about lunch') RETURNING id"
        ).bind(collection_id).fetch_one(&pool).await.unwrap();

        let primary = insert_test_contact(&pool, "alice", "Jane Doe").await;
        let other = insert_test_contact(&pool, "alice", "Jane D.").await;

        let input = MergeContactsInput {
            primary_id: primary,
            merged_ids: vec![other],
            resolved: MergedScalarFields {
                display_name: "Jane Doe".to_string(),
                ..Default::default()
            },
            keep_photo_from: None,
        };
        merge_contacts_inner(&pool, "alice", input).await.unwrap();

        let text: String = sqlx::query_scalar("SELECT text FROM notes WHERE id = ?")
            .bind(note_id)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(text, "Ping @Jane Doe about lunch");
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn merge_contacts_rejects_contact_owned_by_another_user(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0), ('eve', 'eve@example.com', 'eve', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let primary = insert_test_contact(&pool, "alice", "Jane Doe").await;
        let eves = insert_test_contact(&pool, "eve", "Someone Else").await;

        let input = MergeContactsInput {
            primary_id: primary,
            merged_ids: vec![eves],
            resolved: MergedScalarFields {
                display_name: "Jane Doe".to_string(),
                ..Default::default()
            },
            keep_photo_from: None,
        };
        assert!(merge_contacts_inner(&pool, "alice", input).await.is_err());

        // Neither contact must have been touched.
        let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM contacts")
            .fetch_one(&pool)
            .await
            .unwrap();
        assert_eq!(count, 2);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn merge_contacts_carries_over_photo_from_a_merged_away_contact(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let primary = insert_test_contact(&pool, "alice", "Jane Doe").await;
        let other = insert_test_contact(&pool, "alice", "Jane D.").await;
        sqlx::query("UPDATE contacts SET photo = ?, photo_content_type = 'image/png' WHERE id = ?")
            .bind(vec![1u8, 2, 3])
            .bind(other)
            .execute(&pool)
            .await
            .unwrap();

        let input = MergeContactsInput {
            primary_id: primary,
            merged_ids: vec![other],
            resolved: MergedScalarFields {
                display_name: "Jane Doe".to_string(),
                ..Default::default()
            },
            keep_photo_from: Some(other),
        };
        let result = merge_contacts_inner(&pool, "alice", input).await.unwrap();
        assert!(result.has_photo);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn merge_contacts_bumps_sync_log_for_survivor_and_deletions(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let primary = insert_test_contact(&pool, "alice", "Jane Doe").await;
        let other = insert_test_contact(&pool, "alice", "Jane D.").await;

        let input = MergeContactsInput {
            primary_id: primary,
            merged_ids: vec![other],
            resolved: MergedScalarFields {
                display_name: "Jane Doe".to_string(),
                ..Default::default()
            },
            keep_photo_from: None,
        };
        merge_contacts_inner(&pool, "alice", input).await.unwrap();

        let ops: Vec<String> = sqlx::query_scalar(
            "SELECT operation FROM contact_sync_log
             WHERE collection_id = (SELECT id FROM contact_collections WHERE owner_username = 'alice' AND name = 'My Contacts')
             ORDER BY id ASC",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(ops, vec!["modified".to_string(), "deleted".to_string()]);
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn delete_contact_collection_removes_its_contacts_without_fk_violation(
        pool: sqlx::SqlitePool,
    ) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        // insert_test_contact seeds alice's default "My Contacts" collection
        // — needed so the "can't delete your only collection" guard doesn't
        // trip on the second collection below.
        insert_test_contact(&pool, "alice", "Jane Doe").await;

        let second_collection: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'Work', '#22c55e') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();
        sqlx::query(
            "INSERT INTO contacts (owner_username, collection_id, uid, display_name, etag) VALUES ('alice', ?, 'u-work-1', 'Work Contact', 'etag1')",
        )
        .bind(second_collection)
        .execute(&pool)
        .await
        .unwrap();

        delete_contact_collection_inner(&pool, "alice", second_collection)
            .await
            .unwrap();

        let remaining_collection: Option<i64> =
            sqlx::query_scalar("SELECT id FROM contact_collections WHERE id = ?")
                .bind(second_collection)
                .fetch_optional(&pool)
                .await
                .unwrap();
        assert_eq!(
            remaining_collection, None,
            "the collection itself must be gone"
        );

        let remaining_contact: Option<i64> =
            sqlx::query_scalar("SELECT id FROM contacts WHERE collection_id = ?")
                .bind(second_collection)
                .fetch_optional(&pool)
                .await
                .unwrap();
        assert_eq!(
            remaining_contact, None,
            "its contact must be gone too, not orphaned"
        );
    }

    async fn insert_test_contact_no_default_collection(
        pool: &sqlx::SqlitePool,
        owner: &str,
        collection_id: i64,
        uid: &str,
        display_name: &str,
    ) -> i64 {
        sqlx::query_scalar(
            "INSERT INTO contacts (owner_username, collection_id, uid, display_name, etag) VALUES (?, ?, ?, ?, 'etag') RETURNING id",
        )
        .bind(owner)
        .bind(collection_id)
        .bind(uid)
        .bind(display_name)
        .fetch_one(pool)
        .await
        .unwrap()
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reorder_contact_first_call_backfills_the_whole_zone(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let a = insert_test_contact_no_default_collection(
            &pool,
            "alice",
            collection_id,
            "u1",
            "Aardvark",
        )
        .await;
        let b =
            insert_test_contact_no_default_collection(&pool, "alice", collection_id, "u2", "Bear")
                .await;
        let c =
            insert_test_contact_no_default_collection(&pool, "alice", collection_id, "u3", "Camel")
                .await;

        // Drag "Camel" to sit between "Aardvark" and "Bear" -- their first
        // reorder ever, so all three must get backfilled first.
        reorder_contact_inner(&pool, "alice", c, Some(a), Some(b))
            .await
            .unwrap();

        let orders: Vec<(String, Option<f64>)> = sqlx::query_as(
            "SELECT display_name, manual_order FROM contacts WHERE owner_username = 'alice' ORDER BY manual_order",
        )
        .fetch_all(&pool)
        .await
        .unwrap();
        assert_eq!(orders.len(), 3);
        assert!(
            orders.iter().all(|(_, o)| o.is_some()),
            "every contact in the zone must be backfilled, not just the dragged one: {orders:?}"
        );
        let order_of = |name: &str| orders.iter().find(|(n, _)| n == name).unwrap().1.unwrap();
        assert!(
            order_of("Aardvark") < order_of("Camel"),
            "Camel must land after Aardvark"
        );
        assert!(
            order_of("Camel") < order_of("Bear"),
            "Camel must land before Bear"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reorder_contact_second_call_does_not_rebackfill(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let a = insert_test_contact_no_default_collection(
            &pool,
            "alice",
            collection_id,
            "u1",
            "Aardvark",
        )
        .await;
        let b =
            insert_test_contact_no_default_collection(&pool, "alice", collection_id, "u2", "Bear")
                .await;
        let c =
            insert_test_contact_no_default_collection(&pool, "alice", collection_id, "u3", "Camel")
                .await;

        reorder_contact_inner(&pool, "alice", c, Some(a), Some(b))
            .await
            .unwrap();
        let a_order_after_first: f64 =
            sqlx::query_scalar("SELECT manual_order FROM contacts WHERE id = ?")
                .bind(a)
                .fetch_one(&pool)
                .await
                .unwrap();

        // A second, unrelated reorder must not touch Aardvark's already-set
        // order -- only a fresh, never-before-ordered zone gets backfilled.
        let d =
            insert_test_contact_no_default_collection(&pool, "alice", collection_id, "u4", "Deer")
                .await;
        reorder_contact_inner(&pool, "alice", d, Some(b), None)
            .await
            .unwrap();

        let a_order_after_second: f64 =
            sqlx::query_scalar("SELECT manual_order FROM contacts WHERE id = ?")
                .bind(a)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(
            a_order_after_first, a_order_after_second,
            "an already-ordered contact must not be re-backfilled by a later, unrelated reorder"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reorder_contact_to_the_very_start_extrapolates_below_the_successor(
        pool: sqlx::SqlitePool,
    ) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let a = insert_test_contact_no_default_collection(
            &pool,
            "alice",
            collection_id,
            "u1",
            "Aardvark",
        )
        .await;
        let b =
            insert_test_contact_no_default_collection(&pool, "alice", collection_id, "u2", "Bear")
                .await;

        reorder_contact_inner(&pool, "alice", b, None, Some(a))
            .await
            .unwrap();

        let (a_order, b_order): (Option<f64>, Option<f64>) = (
            sqlx::query_scalar("SELECT manual_order FROM contacts WHERE id = ?")
                .bind(a)
                .fetch_one(&pool)
                .await
                .unwrap(),
            sqlx::query_scalar("SELECT manual_order FROM contacts WHERE id = ?")
                .bind(b)
                .fetch_one(&pool)
                .await
                .unwrap(),
        );
        assert!(
            b_order.unwrap() < a_order.unwrap(),
            "Bear dragged before Aardvark must sort before it"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reorder_contact_to_the_very_end_extrapolates_above_the_predecessor(
        pool: sqlx::SqlitePool,
    ) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let a = insert_test_contact_no_default_collection(
            &pool,
            "alice",
            collection_id,
            "u1",
            "Aardvark",
        )
        .await;
        let b =
            insert_test_contact_no_default_collection(&pool, "alice", collection_id, "u2", "Bear")
                .await;

        reorder_contact_inner(&pool, "alice", b, Some(a), None)
            .await
            .unwrap();

        let (a_order, b_order): (Option<f64>, Option<f64>) = (
            sqlx::query_scalar("SELECT manual_order FROM contacts WHERE id = ?")
                .bind(a)
                .fetch_one(&pool)
                .await
                .unwrap(),
            sqlx::query_scalar("SELECT manual_order FROM contacts WHERE id = ?")
                .bind(b)
                .fetch_one(&pool)
                .await
                .unwrap(),
        );
        assert!(
            b_order.unwrap() > a_order.unwrap(),
            "Bear dragged after Aardvark must sort after it"
        );
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reorder_contact_the_only_card_in_its_zone_does_not_panic(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let a = insert_test_contact_no_default_collection(
            &pool,
            "alice",
            collection_id,
            "u1",
            "Aardvark",
        )
        .await;

        let result = reorder_contact_inner(&pool, "alice", a, None, None).await;
        assert!(result.is_ok());
    }

    #[sqlx::test(migrations = "../../migrations")]
    async fn reorder_contact_rejects_a_non_member(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0), ('eve', 'eve@example.com', 'eve', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let collection_id: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let a = insert_test_contact_no_default_collection(
            &pool,
            "alice",
            collection_id,
            "u1",
            "Aardvark",
        )
        .await;

        let result = reorder_contact_inner(&pool, "eve", a, None, None).await;
        assert!(result.is_err());
    }

    /// Found in final review: `predecessor_id`/`successor_id` weren't
    /// scope-checked -- only the dragged `id` itself was. A caller could
    /// reference another user's private contact's id as a neighbor and
    /// have its `manual_order` read (and used to compute the dragged
    /// card's own value) with no access check at all.
    #[sqlx::test(migrations = "../../migrations")]
    async fn reorder_contact_ignores_a_predecessor_outside_the_callers_access(
        pool: sqlx::SqlitePool,
    ) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin) VALUES ('alice', 'alice@example.com', 'alice', 'x', 0), ('eve', 'eve@example.com', 'eve', 'x', 0)"
        ).execute(&pool).await.unwrap();
        let alice_collection: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('alice', 'My Contacts', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let eve_collection: i64 = sqlx::query_scalar(
            "INSERT INTO contact_collections (owner_username, name, color) VALUES ('eve', 'Eve Private', '#3b82f6') RETURNING id"
        ).fetch_one(&pool).await.unwrap();
        let alice_contact = insert_test_contact_no_default_collection(
            &pool,
            "alice",
            alice_collection,
            "u1",
            "Alice's Contact",
        )
        .await;
        // Eve's own private contact, given a deliberately extreme
        // manual_order -- if this leaked into alice's computation, her
        // contact's new order would land near this extreme value.
        let eve_contact = insert_test_contact_no_default_collection(
            &pool,
            "eve",
            eve_collection,
            "u2",
            "Eve's Private Contact",
        )
        .await;
        sqlx::query("UPDATE contacts SET manual_order = 999999.0 WHERE id = ?")
            .bind(eve_contact)
            .execute(&pool)
            .await
            .unwrap();

        // Alice references eve's private contact as her own predecessor --
        // must be treated as if no predecessor were given (extrapolated
        // from MANUAL_ORDER_SPACING), never as eve's real order value.
        reorder_contact_inner(&pool, "alice", alice_contact, Some(eve_contact), None)
            .await
            .unwrap();

        let alice_order: f64 = sqlx::query_scalar("SELECT manual_order FROM contacts WHERE id = ?")
            .bind(alice_contact)
            .fetch_one(&pool)
            .await
            .unwrap();
        assert!(
            alice_order < 999999.0,
            "alice's contact must not be ordered relative to eve's inaccessible private contact's manual_order, got {alice_order}"
        );
    }
}
