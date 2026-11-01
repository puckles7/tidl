//! Google Keep (Takeout) import — backlog #109.
//!
//! Takeout exports Keep as a `.zip` holding, per note, a `<note>.json` (the
//! faithful structured form) plus a rendered `<note>.html`, and the actual
//! attachment image files alongside them. This module parses the JSON — the
//! `.html` files carry no data the JSON lacks — and writes each note into a
//! tidl note collection, re-attaching any image bytes it finds in the same
//! archive as note images (backlog #110's `note_images`).
//!
//! Everything here is server-only: the `zip` dependency is `ssr`-gated and
//! the `#[server]` fn's body never runs on the wasm target.

use leptos::prelude::*;

use crate::types::KeepImportResult;

#[cfg(feature = "ssr")]
use super::auth;
#[cfg(feature = "ssr")]
use super::notes::{
    has_collection_access, insert_note_image, insert_note_row, set_note_labels,
    validate_note_image_bytes,
};
#[cfg(feature = "ssr")]
use crate::components::notes::collection_editor_modal::NOTE_PALETTE;
#[cfg(feature = "ssr")]
use crate::types::{note_image_token, NewNote};
#[cfg(feature = "ssr")]
use std::collections::HashMap;
#[cfg(feature = "ssr")]
use std::io::Read;

/// Decoded-size cap for the uploaded Takeout `.zip` (backlog #109). Generous
/// because Takeout bundles the attachment images inside the same archive; a
/// personal Keep library of a few hundred notes is far below this.
#[cfg(feature = "ssr")]
const MAX_KEEP_ZIP_BYTES: usize = 100 * 1024 * 1024;

/// Cap on the *uncompressed* bytes read out of the zip, so a malicious or
/// corrupt archive (a "zip bomb") can't exhaust server memory before the
/// decoded-size cap above is ever consulted.
#[cfg(feature = "ssr")]
const MAX_KEEP_UNCOMPRESSED_BYTES: u64 = 512 * 1024 * 1024;

// ── Parsed model ──────────────────────────────────────────────────────────────

/// A Keep note after parsing, before it's written to the database.
#[cfg(feature = "ssr")]
#[derive(Debug, Default, Clone, PartialEq)]
struct KeepNote {
    title: Option<String>,
    text: String,
    pinned: bool,
    is_archived: bool,
    is_trashed: bool,
    /// Unix seconds, when the JSON carried a timestamp.
    created_secs: Option<i64>,
    updated_secs: Option<i64>,
    /// The raw `createdTimestampUsec`, the stable identity a re-import matches
    /// on (it survives edits, unlike the title). `None` only if the archive
    /// omitted it, in which case the note can't de-dup.
    created_usec: Option<i64>,
    /// Emails in the note's `sharees[]`, deduped and in archive order. Empty
    /// for a private note; non-empty means the note was shared (in either
    /// direction). The exporter's own address is never listed, so every entry
    /// is *another* person the note was shared with.
    sharee_emails: Vec<String>,
    /// Keep labels attached to this note (backlog #114), trimmed, deduped and
    /// in archive order.
    labels: Vec<String>,
    attachments: Vec<KeepAttachmentRef>,
}

/// One entry of a Keep note's `attachments[]` — `path` is the file name inside
/// the Takeout `Keep/` folder (not a URL), resolved against the zip's own
/// entries at import time.
#[cfg(feature = "ssr")]
#[derive(Debug, Clone, PartialEq)]
struct KeepAttachmentRef {
    path: String,
    mimetype: Option<String>,
}

/// Raw shape of one Keep note's `.json` (only the fields this importer uses;
/// Keep carries more, e.g. `color`, which tidl has no per-note slot for).
#[cfg(feature = "ssr")]
#[derive(serde::Deserialize)]
struct KeepRaw {
    #[serde(default)]
    title: Option<String>,
    #[serde(default, rename = "textContent")]
    text_content: Option<String>,
    #[serde(default, rename = "listContent")]
    list_content: Vec<KeepRawListItem>,
    #[serde(default)]
    sharees: Vec<KeepRawSharee>,
    #[serde(default)]
    labels: Vec<KeepRawLabel>,
    #[serde(default, rename = "isTrashed")]
    is_trashed: bool,
    #[serde(default, rename = "isArchived")]
    is_archived: bool,
    #[serde(default, rename = "isPinned")]
    is_pinned: bool,
    #[serde(default, rename = "createdTimestampUsec")]
    created_usec: Option<i64>,
    #[serde(default, rename = "userEditedTimestampUsec")]
    edited_usec: Option<i64>,
    #[serde(default)]
    attachments: Vec<KeepRawAttachment>,
}

#[cfg(feature = "ssr")]
#[derive(serde::Deserialize)]
struct KeepRawListItem {
    #[serde(default)]
    text: String,
    #[serde(default, rename = "isChecked")]
    is_checked: bool,
}

#[cfg(feature = "ssr")]
#[derive(serde::Deserialize)]
struct KeepRawSharee {
    #[serde(default)]
    email: String,
}

#[cfg(feature = "ssr")]
#[derive(serde::Deserialize)]
struct KeepRawLabel {
    #[serde(default)]
    name: String,
}

#[cfg(feature = "ssr")]
#[derive(serde::Deserialize)]
struct KeepRawAttachment {
    #[serde(default, rename = "filePath")]
    file_path: String,
    #[serde(default)]
    mimetype: Option<String>,
}

/// Parses one Keep `.json` document into a [`KeepNote`]. `Err` is the serde
/// message, so a stray non-note `.json` in the archive surfaces as one failed
/// row rather than aborting the whole import.
#[cfg(feature = "ssr")]
fn parse_keep_note_json(json: &str) -> Result<KeepNote, String> {
    let raw: KeepRaw = serde_json::from_str(json).map_err(|e| e.to_string())?;
    Ok(keep_note_from_raw(raw))
}

#[cfg(feature = "ssr")]
fn keep_note_from_raw(raw: KeepRaw) -> KeepNote {
    let title = raw
        .title
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(str::to_string);

    let mut sharee_emails: Vec<String> = Vec::new();
    for s in raw.sharees {
        let email = s.email.trim().to_string();
        if !email.is_empty() && !sharee_emails.contains(&email) {
            sharee_emails.push(email);
        }
    }

    let mut labels: Vec<String> = Vec::new();
    for l in raw.labels {
        let name = l.name.trim().to_string();
        if !name.is_empty() && !labels.iter().any(|n| n.eq_ignore_ascii_case(&name)) {
            labels.push(name);
        }
    }

    let attachments = raw
        .attachments
        .into_iter()
        .filter(|a| !a.file_path.trim().is_empty())
        .map(|a| KeepAttachmentRef {
            path: a.file_path,
            mimetype: a.mimetype,
        })
        .collect();

    let done_date = raw
        .edited_usec
        .map(usec_to_date)
        .unwrap_or_else(|| chrono::Utc::now().date_naive());

    KeepNote {
        title,
        text: build_note_text(raw.text_content.as_deref(), &raw.list_content, done_date),
        pinned: raw.is_pinned,
        is_archived: raw.is_archived,
        is_trashed: raw.is_trashed,
        created_secs: raw.created_usec.map(usec_to_secs),
        updated_secs: raw.edited_usec.map(usec_to_secs),
        created_usec: raw.created_usec,
        sharee_emails,
        labels,
        attachments,
    }
}

/// Default priority for a task imported from a Keep checkbox item — the middle
/// of the 1-100 scale (Keep has no priority concept).
#[cfg(feature = "ssr")]
const KEEP_ITEM_PRIORITY: u32 = 50;

/// Combines a Keep note's free text and its checkbox list into one tidl note
/// body. Each list item becomes an inline token (backlog #113): an unchecked
/// item a `#task/p50/<slug>`, a checked one a `#done/<date>/p50/<slug>` record
/// — so imported items surface in Tasks and stay reopenable, rather than being
/// flattened to literal `[ ]`/`[x]` text. An item whose text has no
/// alphanumerics falls back to `item-<n>`.
#[cfg(feature = "ssr")]
fn build_note_text(
    text_content: Option<&str>,
    list: &[KeepRawListItem],
    done_date: chrono::NaiveDate,
) -> String {
    use crate::note_tokens::{format_done, format_task_token, slugify, DoneOriginal};

    let mut parts: Vec<String> = Vec::new();

    let body = text_content.unwrap_or("").trim();
    if !body.is_empty() {
        parts.push(body.to_string());
    }

    if !list.is_empty() {
        let items: Vec<String> = list
            .iter()
            .enumerate()
            .map(|(i, item)| {
                let slug = {
                    let s = slugify(item.text.trim());
                    if s.is_empty() {
                        format!("item-{}", i + 1)
                    } else {
                        s
                    }
                };
                if item.is_checked {
                    format_done(
                        done_date,
                        &DoneOriginal::Task {
                            priority: KEEP_ITEM_PRIORITY,
                        },
                        &slug,
                    )
                } else {
                    format_task_token(KEEP_ITEM_PRIORITY, &slug)
                }
            })
            .collect();
        parts.push(items.join("\n"));
    }

    parts.join("\n\n")
}

/// Keep timestamps are microseconds since the Unix epoch.
#[cfg(feature = "ssr")]
fn usec_to_secs(usec: i64) -> i64 {
    usec.div_euclid(1_000_000)
}

/// The UTC date a Keep microsecond timestamp falls on (falls back to today).
#[cfg(feature = "ssr")]
fn usec_to_date(usec: i64) -> chrono::NaiveDate {
    chrono::DateTime::from_timestamp(usec_to_secs(usec), 0)
        .map(|dt| dt.date_naive())
        .unwrap_or_else(|| chrono::Utc::now().date_naive())
}

// ── Zip reading ───────────────────────────────────────────────────────────────

/// Attachment bytes pulled out of the Takeout zip, plus the content type
/// inferred from the file extension (the JSON's own `mimetype`, when present,
/// takes precedence at import time).
#[cfg(feature = "ssr")]
struct KeepAttachmentFile {
    bytes: Vec<u8>,
    mime: Option<String>,
}

/// The parsed contents of a Takeout archive: each note in order (an `Err`
/// being one malformed `.json`) plus the attachment files keyed by file name.
#[cfg(feature = "ssr")]
type KeepArchive = (
    Vec<Result<KeepNote, String>>,
    HashMap<String, KeepAttachmentFile>,
);

/// Reads every Keep note and attachment out of a Takeout `.zip`. Returns the
/// parsed notes in archive order (each `Err` being one malformed `.json`) and
/// the attachment files keyed by file name.
#[cfg(feature = "ssr")]
fn read_keep_zip(bytes: &[u8]) -> Result<KeepArchive, String> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
        .map_err(|e| format!("Not a valid Keep .zip archive: {e}"))?;

    let mut notes: Vec<Result<KeepNote, String>> = Vec::new();
    let mut files: HashMap<String, KeepAttachmentFile> = HashMap::new();
    let mut total: u64 = 0;

    for i in 0..archive.len() {
        let mut entry = archive
            .by_index(i)
            .map_err(|e| format!("Failed to read a zip entry: {e}"))?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().to_string();
        if !path_is_in_keep_folder(&name) {
            continue;
        }

        total = total.saturating_add(entry.size());
        if total > MAX_KEEP_UNCOMPRESSED_BYTES {
            return Err("Keep archive is too large to import".to_string());
        }

        let mut buf = Vec::with_capacity(entry.size().min(8 * 1024 * 1024) as usize);
        entry
            .read_to_end(&mut buf)
            .map_err(|e| format!("Failed to read {name}: {e}"))?;

        let lower = name.to_ascii_lowercase();
        if lower.ends_with(".json") {
            notes.push(parse_keep_note_json(&String::from_utf8_lossy(&buf)));
        } else if !lower.ends_with(".html") {
            files.insert(
                basename(&name).to_string(),
                KeepAttachmentFile {
                    mime: mime_from_extension(&name),
                    bytes: buf,
                },
            );
        }
    }

    if notes.is_empty() {
        return Err("No Google Keep notes (.json) found in the archive".to_string());
    }

    Ok((notes, files))
}

/// Whether a zip entry lives in the export's `Keep/` folder. Accepts the
/// canonical `Takeout/Keep/...` layout, a zip of the `Keep/` folder itself,
/// and a flat zip of `.json` files — while rejecting other Takeout apps'
/// folders (e.g. `Takeout/Calendar/`).
#[cfg(feature = "ssr")]
fn path_is_in_keep_folder(name: &str) -> bool {
    let normalized = name.replace('\\', "/");
    normalized.contains("/Keep/") || normalized.starts_with("Keep/") || !normalized.contains('/')
}

#[cfg(feature = "ssr")]
fn basename(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

#[cfg(feature = "ssr")]
fn mime_from_extension(name: &str) -> Option<String> {
    let ext = name.rsplit('.').next()?.to_ascii_lowercase();
    match ext.as_str() {
        "jpg" | "jpeg" => Some("image/jpeg".to_string()),
        "png" => Some("image/png".to_string()),
        "gif" => Some("image/gif".to_string()),
        "webp" => Some("image/webp".to_string()),
        _ => None,
    }
}

// ── Collection resolution ─────────────────────────────────────────────────────

/// Finds (or creates, owned by the user) the note collection with the given
/// name, cached across the batch so repeated names touch the DB once. Used for
/// both the "Add new collection…" target and each per-sharee "Shared with …"
/// collection. A name matching an existing *owned* collection reuses it; a
/// per-owner uniqueness constraint means a collection shared to the user by
/// someone else never blocks creating their own.
#[cfg(feature = "ssr")]
async fn resolve_named_collection(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    username: &str,
    name: &str,
    cache: &mut HashMap<String, i64>,
) -> Result<i64, ServerFnError> {
    if let Some(id) = cache.get(name) {
        return Ok(*id);
    }

    let existing: Option<i64> =
        sqlx::query_scalar("SELECT id FROM note_collections WHERE owner_username = ? AND name = ?")
            .bind(username)
            .bind(name)
            .fetch_optional(&mut **tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

    let id = match existing {
        Some(id) => id,
        None => {
            // Stable per-name colour, so an import doesn't paint every new
            // collection identically.
            let hash = name.bytes().fold(0usize, |acc, b| {
                acc.wrapping_mul(31).wrapping_add(b as usize)
            });
            let color = NOTE_PALETTE[hash % NOTE_PALETTE.len()];
            sqlx::query_scalar(
                "INSERT INTO note_collections (owner_username, name, color)
                 VALUES (?, ?, ?) RETURNING id",
            )
            .bind(username)
            .bind(name)
            .bind(color)
            .fetch_one(&mut **tx)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
        }
    };

    cache.insert(name.to_string(), id);
    Ok(id)
}

/// The collection name a shared note is grouped under. The archive carries only
/// emails (no display names), so the name is built from them directly; a note
/// shared with several people gets one combined collection rather than being
/// duplicated across collections. Emails are sorted for a stable name.
#[cfg(feature = "ssr")]
fn shared_collection_name(emails: &[String]) -> String {
    let mut sorted: Vec<&str> = emails.iter().map(String::as_str).collect();
    sorted.sort_unstable();
    format!("Shared with {}", sorted.join(", "))
}

/// The collection a Keep note lands in. A shared note is grouped into a
/// `Shared with …` collection (created on demand, cached across the batch) only
/// when `split_shared` is set; otherwise — and for every unshared note — it
/// falls through to `fallback_id`.
#[cfg(feature = "ssr")]
async fn resolve_note_collection(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    username: &str,
    sharee_emails: &[String],
    split_shared: bool,
    fallback_id: i64,
    cache: &mut HashMap<String, i64>,
) -> Result<i64, ServerFnError> {
    if !split_shared || sharee_emails.is_empty() {
        return Ok(fallback_id);
    }
    let name = shared_collection_name(sharee_emails);
    resolve_named_collection(tx, username, &name, cache).await
}

/// The `import_source` stamped on every note this importer creates.
#[cfg(feature = "ssr")]
const KEEP_IMPORT_SOURCE: &str = "keep";

/// Finds a note already imported from the same Keep note, so a re-import drops
/// it instead of duplicating. First tries the persisted identity
/// (`import_source`/`import_external_id`); when that misses it *adopts* a note
/// imported before the key existed — matched by `created_at` (the note's
/// second-precision creation instant) plus title, since Keep's creation
/// timestamp is stable across edits. Matching title rather than the body keeps
/// adoption working for notes whose body gained appended `@img:` tokens. Only
/// the caller's own notes are considered.
#[cfg(feature = "ssr")]
async fn find_import_duplicate(
    tx: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    username: &str,
    created_usec: i64,
    title: Option<&str>,
) -> Result<Option<i64>, ServerFnError> {
    let external_id = created_usec.to_string();
    let exact: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM notes
         WHERE author_username = ? AND import_source = ? AND import_external_id = ?",
    )
    .bind(username)
    .bind(KEEP_IMPORT_SOURCE)
    .bind(&external_id)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    if exact.is_some() {
        return Ok(exact);
    }

    let created_secs = created_usec.div_euclid(1_000_000);
    let adopted: Option<i64> = sqlx::query_scalar(
        "SELECT id FROM notes
         WHERE author_username = ? AND import_source IS NULL
           AND created_at = datetime(?, 'unixepoch')
           AND COALESCE(title, '') = COALESCE(?, '')
         ORDER BY id LIMIT 1",
    )
    .bind(username)
    .bind(created_secs)
    .bind(title)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
    Ok(adopted)
}

// ── Server function ───────────────────────────────────────────────────────────

/// Import a Google Keep Takeout `.zip`. `zip_base64` is the raw archive
/// (base64-encoded, matching the note-image transport). A **shared** note (one
/// whose archive entry carried non-empty `sharees[]`) is grouped into a
/// `Shared with <email(s)>` collection, created on demand — the archive names
/// only the other people, never the caller. `split_shared` controls this: when
/// false, shared notes fall through to the fallback collection like any other
/// note and no `Shared with …` collection is created. Every other note lands in
/// `target_collection_id`; as an alternative, `new_collection_name`
/// finds-or-creates an owned collection of that name and uses it as the
/// fallback — so an import works even when the user has no collections yet.
/// Trashed notes are always skipped; `include_archived` decides the archived
/// ones. A note already imported from the same export (matched by its stable
/// `createdTimestampUsec`, persisted per #109's `import_source`/
/// `import_external_id` columns) is dropped rather than duplicated — never
/// overwritten, so a shared note another user may have edited is left alone.
/// Nothing is written unless the whole archive parses, and all writes
/// share one transaction.
#[server(input = server_fn::codec::Json)]
pub async fn import_keep(
    zip_base64: String,
    target_collection_id: Option<i64>,
    include_archived: bool,
    split_shared: bool,
    new_collection_name: Option<String>,
) -> Result<KeepImportResult, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use base64::Engine;

        let bytes = base64::engine::general_purpose::STANDARD
            .decode(zip_base64.trim())
            .map_err(|_| ServerFnError::new("Invalid zip data"))?;
        if bytes.is_empty() {
            return Err(ServerFnError::new("The uploaded file is empty"));
        }
        if bytes.len() > MAX_KEEP_ZIP_BYTES {
            return Err(ServerFnError::new("File exceeds the 100 MB limit"));
        }

        let (pool, username) = auth().await?;

        let new_collection_name = new_collection_name
            .as_deref()
            .map(str::trim)
            .filter(|name| !name.is_empty());

        // A brand-new collection is authorized by creation itself; an existing
        // one must be owned by, or shared with, the caller.
        let target_id = if new_collection_name.is_none() {
            let target = target_collection_id
                .ok_or_else(|| ServerFnError::new("Pick a collection to import into"))?;
            if !has_collection_access(&pool, &username, target).await? {
                return Err(ServerFnError::new("Not authorized for this collection"));
            }
            Some(target)
        } else {
            None
        };

        let (notes, files) = read_keep_zip(&bytes).map_err(ServerFnError::new)?;

        let mut result = KeepImportResult::default();
        let mut collection_ids: HashMap<String, i64> = HashMap::new();

        let mut tx = pool
            .begin()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        // Resolve the fallback collection inside the transaction, so a failed
        // import never leaves an orphaned collection behind.
        let fallback_id = match new_collection_name {
            Some(name) => {
                resolve_named_collection(&mut tx, &username, name, &mut collection_ids).await?
            }
            None => {
                target_id.ok_or_else(|| ServerFnError::new("Pick a collection to import into"))?
            }
        };

        for parsed in notes {
            let note = match parsed {
                Ok(note) => note,
                Err(_) => {
                    result.failed += 1;
                    continue;
                }
            };

            if note.is_trashed || (note.is_archived && !include_archived) {
                result.skipped += 1;
                continue;
            }

            // Drop a note already imported from the same Keep export (i.e. a
            // re-import). `created_usec` is the stable key; a note without one
            // can't be matched and imports as usual.
            if let Some(usec) = note.created_usec {
                if let Some(existing_id) =
                    find_import_duplicate(&mut tx, &username, usec, note.title.as_deref()).await?
                {
                    // Stamp the identity onto a note adopted from a
                    // pre-migration import, so later re-imports hit the fast
                    // exact-key path (`import_source IS NULL` keeps this a
                    // no-op for an already-stamped row).
                    sqlx::query(
                        "UPDATE notes SET import_source = ?, import_external_id = ?
                         WHERE id = ? AND import_source IS NULL",
                    )
                    .bind(KEEP_IMPORT_SOURCE)
                    .bind(usec.to_string())
                    .bind(existing_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
                    result.duplicates += 1;
                    continue;
                }
            }

            // Shared notes are grouped per person (or per-person-set) unless
            // the caller opted out; every other note falls to the chosen
            // target collection.
            let collection_id = match resolve_note_collection(
                &mut tx,
                &username,
                &note.sharee_emails,
                split_shared,
                fallback_id,
                &mut collection_ids,
            )
            .await
            {
                Ok(id) => id,
                Err(_) => {
                    result.failed += 1;
                    continue;
                }
            };

            let input = NewNote {
                collection_id,
                title: note.title.clone(),
                text: note.text.clone(),
                pinned: note.pinned,
                calendar_name: None,
                labels: Vec::new(),
            };

            let note_id = match insert_note_row(&mut tx, &username, &input).await {
                Ok(id) => id,
                Err(_) => {
                    result.failed += 1;
                    continue;
                }
            };

            if note.created_secs.is_some() || note.updated_secs.is_some() {
                let created = note.created_secs.or(note.updated_secs).unwrap_or_default();
                let updated = note.updated_secs.or(note.created_secs).unwrap_or(created);
                sqlx::query(
                    "UPDATE notes SET created_at = datetime(?, 'unixepoch'),
                            updated_at = datetime(?, 'unixepoch')
                     WHERE id = ?",
                )
                .bind(created)
                .bind(updated)
                .bind(note_id)
                .execute(&mut *tx)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            }

            if let Some(usec) = note.created_usec {
                sqlx::query(
                    "UPDATE notes SET import_source = ?, import_external_id = ? WHERE id = ?",
                )
                .bind(KEEP_IMPORT_SOURCE)
                .bind(usec.to_string())
                .bind(note_id)
                .execute(&mut *tx)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            }

            // Attach the note's Keep labels as this user's labels (backlog
            // #114) — resolved/created inside the import transaction so a
            // failed import leaves no orphan labels.
            if let Ok(n) = set_note_labels(&mut tx, note_id, &note.labels).await {
                result.labels += n;
            }

            let mut tokens: Vec<String> = Vec::new();
            for attachment in &note.attachments {
                let name = basename(&attachment.path).to_string();
                let Some(file) = files.get(&name) else {
                    continue;
                };
                let content_type = attachment
                    .mimetype
                    .clone()
                    .or_else(|| file.mime.clone())
                    .unwrap_or_default();
                if validate_note_image_bytes(&file.bytes, &content_type).is_err() {
                    continue;
                }
                if let Ok(image_id) =
                    insert_note_image(&mut tx, note_id, &file.bytes, &content_type, Some(&name))
                        .await
                {
                    tokens.push(note_image_token(image_id));
                    result.images += 1;
                }
            }

            if !tokens.is_empty() {
                let suffix = tokens.join("\n");
                let base = note.text.trim();
                let text = if base.is_empty() {
                    suffix
                } else {
                    format!("{base}\n\n{suffix}")
                };
                sqlx::query("UPDATE notes SET text = ? WHERE id = ?")
                    .bind(&text)
                    .bind(note_id)
                    .execute(&mut *tx)
                    .await
                    .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            }

            result.imported += 1;
        }

        tx.commit()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        return Ok(result);
    }
    #[allow(unreachable_code)]
    Ok(KeepImportResult::default())
}

#[cfg(all(test, feature = "ssr"))]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn parses_text_note_title_body_and_timestamps() {
        let json = r#"{
            "title": "Shopping",
            "textContent": "milk",
            "isPinned": true,
            "createdTimestampUsec": 1600000000000000,
            "userEditedTimestampUsec": 1600000060000000
        }"#;
        let note = parse_keep_note_json(json).unwrap();
        assert_eq!(note.title.as_deref(), Some("Shopping"));
        assert_eq!(note.text, "milk");
        assert!(note.pinned);
        assert!(note.sharee_emails.is_empty());
        assert_eq!(note.created_secs, Some(1_600_000_000));
        assert_eq!(note.updated_secs, Some(1_600_000_060));
        // The raw microseconds are kept for de-duplication.
        assert_eq!(note.created_usec, Some(1_600_000_000_000_000));
    }

    #[test]
    fn sharees_are_parsed_deduped_and_blank_dropped() {
        let json = r#"{
            "textContent": "shared",
            "sharees": [
                {"email": "a@example.com", "isOwner": false, "type": "USER"},
                {"email": "  b@example.com  ", "isOwner": true, "type": "USER"},
                {"email": "a@example.com", "isOwner": false, "type": "USER"},
                {"email": "", "isOwner": true, "type": "USER"}
            ]
        }"#;
        let note = parse_keep_note_json(json).unwrap();
        assert_eq!(
            note.sharee_emails,
            vec!["a@example.com".to_string(), "b@example.com".to_string()]
        );
    }

    #[test]
    fn a_private_note_has_no_sharees() {
        let json = r#"{"textContent": "x"}"#;
        let note = parse_keep_note_json(json).unwrap();
        assert!(note.sharee_emails.is_empty());
    }

    #[test]
    fn labels_are_parsed_trimmed_and_deduped_case_insensitively() {
        let json = r#"{
            "textContent": "x",
            "labels": [{"name": "Work"}, {"name": "  work "}, {"name": "Home"}, {"name": ""}]
        }"#;
        let note = parse_keep_note_json(json).unwrap();
        assert_eq!(note.labels, vec!["Work".to_string(), "Home".to_string()]);
    }

    #[test]
    fn a_note_without_labels_parses_empty() {
        let note = parse_keep_note_json(r#"{"textContent": "x"}"#).unwrap();
        assert!(note.labels.is_empty());
    }

    #[test]
    fn shared_collection_name_joins_sorted_emails() {
        assert_eq!(
            shared_collection_name(&["b@example.com".to_string(), "a@example.com".to_string()]),
            "Shared with a@example.com, b@example.com"
        );
        assert_eq!(
            shared_collection_name(&["only@example.com".to_string()]),
            "Shared with only@example.com"
        );
    }

    #[test]
    fn list_note_becomes_task_tokens_preserving_checked_state() {
        let json = r#"{
            "title": "Groceries",
            "userEditedTimestampUsec": 1600000060000000,
            "listContent": [
                {"text": "apples", "isChecked": false},
                {"text": "bread", "isChecked": true}
            ]
        }"#;
        let note = parse_keep_note_json(json).unwrap();
        assert_eq!(note.text, "#task/p50/apples\n#done/2020-09-13/p50/bread");
    }

    #[test]
    fn list_note_with_body_joins_paragraphs() {
        let json = r#"{
            "textContent": "before you go",
            "listContent": [{"text": "one", "isChecked": false}]
        }"#;
        let note = parse_keep_note_json(json).unwrap();
        assert_eq!(note.text, "before you go\n\n#task/p50/one");
    }

    #[test]
    fn list_item_without_alphanumerics_gets_numbered_slug() {
        let json = r#"{"listContent": [{"text": "!!!", "isChecked": false}]}"#;
        let note = parse_keep_note_json(json).unwrap();
        assert_eq!(note.text, "#task/p50/item-1");
    }

    #[test]
    fn trashed_and_archived_flags_are_read() {
        let json = r#"{"textContent": "x", "isTrashed": true, "isArchived": true}"#;
        let note = parse_keep_note_json(json).unwrap();
        assert!(note.is_trashed);
        assert!(note.is_archived);
    }

    #[test]
    fn attachments_and_mime_inference() {
        assert_eq!(
            mime_from_extension("Takeout/Keep/IMG.JPG"),
            Some("image/jpeg".to_string())
        );
        assert_eq!(
            mime_from_extension("a.webp"),
            Some("image/webp".to_string())
        );
        assert_eq!(mime_from_extension("notes.txt"), None);

        let json = r#"{"attachments": [{"filePath": "IMG_1.jpg", "mimetype": "image/jpeg"}]}"#;
        let note = parse_keep_note_json(json).unwrap();
        assert_eq!(note.attachments.len(), 1);
        assert_eq!(note.attachments[0].path, "IMG_1.jpg");
        assert_eq!(note.attachments[0].mimetype.as_deref(), Some("image/jpeg"));
    }

    #[test]
    fn keep_folder_detection() {
        assert!(path_is_in_keep_folder("Takeout/Keep/note.json"));
        assert!(path_is_in_keep_folder("Keep/note.json"));
        assert!(path_is_in_keep_folder("note.json"));
        assert!(!path_is_in_keep_folder("Takeout/Calendar/event.ics"));
    }

    #[test]
    fn malformed_json_is_an_error_not_a_panic() {
        assert!(parse_keep_note_json("{ not json").is_err());
    }

    fn zip_with(entries: &[(&str, &[u8])]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        for (name, bytes) in entries {
            writer.start_file(*name, options).unwrap();
            writer.write_all(bytes).unwrap();
        }
        writer.finish().unwrap().into_inner()
    }

    #[test]
    fn reads_notes_and_attachment_files_from_a_zip() {
        let zip = zip_with(&[
            (
                "Takeout/Keep/note1.json",
                br#"{"title":"A","textContent":"body"}"#.as_slice(),
            ),
            (
                "Takeout/Keep/note2.json",
                br#"{"listContent":[{"text":"x","isChecked":false}]}"#.as_slice(),
            ),
            ("Takeout/Keep/IMG_1.jpg", b"\xff\xd8\xffbytes".as_slice()),
            ("Takeout/Keep/note1.html", b"<html></html>".as_slice()),
            ("Takeout/Calendar/event.ics", b"BEGIN:VCALENDAR".as_slice()),
        ]);
        let (notes, files) = read_keep_zip(&zip).unwrap();
        assert_eq!(notes.len(), 2);
        assert_eq!(notes[0].as_ref().unwrap().text, "body");
        assert_eq!(notes[1].as_ref().unwrap().text, "#task/p50/x");
        assert!(files.contains_key("IMG_1.jpg"));
        // The calendar folder and the rendered HTML are ignored.
        assert_eq!(files.len(), 1);
    }

    #[test]
    fn empty_archive_of_notes_is_an_error() {
        let zip = zip_with(&[("Takeout/Calendar/event.ics", b"x".as_slice())]);
        assert!(read_keep_zip(&zip).is_err());
    }

    /// The "Add new collection…" import path resolves its fallback through
    /// `resolve_named_collection`: the same name must reuse the one collection
    /// (never insert a duplicate) so a re-import can't trip the per-owner
    /// uniqueness constraint. The per-sharee "Shared with …" collections use
    /// the same path.
    #[sqlx::test(migrations = "../../migrations")]
    async fn resolving_a_new_collection_name_reuses_the_created_collection(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin)
             VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();

        let mut tx = pool.begin().await.unwrap();
        let mut cache = HashMap::new();
        let name = shared_collection_name(&["a@example.com".to_string()]);
        let first = resolve_named_collection(&mut tx, "alice", &name, &mut cache)
            .await
            .unwrap();
        let second = resolve_named_collection(&mut tx, "alice", &name, &mut cache)
            .await
            .unwrap();
        assert_eq!(first, second);

        let count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM note_collections
             WHERE owner_username = 'alice' AND name = ?",
        )
        .bind(&name)
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        assert_eq!(count, 1);
    }

    /// De-dup matching: a persisted key wins; otherwise a pre-migration import
    /// is adopted by created_at + title; another user's identical note never
    /// matches.
    #[sqlx::test(migrations = "../../migrations")]
    async fn duplicate_lookup_matches_key_and_adopts_a_legacy_import(pool: sqlx::SqlitePool) {
        for who in ["alice", "bob"] {
            sqlx::query(
                "INSERT INTO users (username, email, display_name, password_hash, is_admin)
                 VALUES (?, ?, ?, 'x', 0)",
            )
            .bind(who)
            .bind(format!("{who}@example.com"))
            .bind(who)
            .execute(&pool)
            .await
            .unwrap();
        }
        let coll: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color)
             VALUES ('alice', 'Inbox', '#000000') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        // A legacy import: no key, created_at derived from its timestamp.
        let usec: i64 = 1_600_000_000_000_000;
        let legacy: i64 = sqlx::query_scalar(
            "INSERT INTO notes (collection_id, author_username, text, title, created_at)
             VALUES (?, 'alice', 'body', 'Shopping', datetime(?, 'unixepoch')) RETURNING id",
        )
        .bind(coll)
        .bind(usec.div_euclid(1_000_000))
        .fetch_one(&pool)
        .await
        .unwrap();

        let mut tx = pool.begin().await.unwrap();
        // Adopted by created_at + title, even before any key is stamped.
        assert_eq!(
            find_import_duplicate(&mut tx, "alice", usec, Some("Shopping"))
                .await
                .unwrap(),
            Some(legacy)
        );
        // A different title is a different note.
        assert_eq!(
            find_import_duplicate(&mut tx, "alice", usec, Some("Other"))
                .await
                .unwrap(),
            None
        );
        // Another user's matching note is invisible.
        assert_eq!(
            find_import_duplicate(&mut tx, "bob", usec, Some("Shopping"))
                .await
                .unwrap(),
            None
        );

        // Once stamped, the exact key matches regardless of a renamed title.
        sqlx::query("UPDATE notes SET import_source = 'keep', import_external_id = ? WHERE id = ?")
            .bind(usec.to_string())
            .bind(legacy)
            .execute(&mut *tx)
            .await
            .unwrap();
        assert_eq!(
            find_import_duplicate(&mut tx, "alice", usec, Some("Renamed"))
                .await
                .unwrap(),
            Some(legacy)
        );
    }

    /// `split_shared` gates the per-sharee collection: when off, a shared note
    /// lands in the fallback just like an unshared one and no `Shared with …`
    /// collection is created; when on, the grouping collection is created on
    /// demand.
    #[sqlx::test(migrations = "../../migrations")]
    async fn split_shared_gates_the_shared_collection(pool: sqlx::SqlitePool) {
        sqlx::query(
            "INSERT INTO users (username, email, display_name, password_hash, is_admin)
             VALUES ('alice', 'alice@example.com', 'alice', 'x', 0)",
        )
        .execute(&pool)
        .await
        .unwrap();
        let fallback: i64 = sqlx::query_scalar(
            "INSERT INTO note_collections (owner_username, name, color)
             VALUES ('alice', 'Inbox', '#000000') RETURNING id",
        )
        .fetch_one(&pool)
        .await
        .unwrap();

        let mut tx = pool.begin().await.unwrap();
        let mut cache = HashMap::new();
        let sharees = vec!["a@example.com".to_string()];

        // Opted out: the shared note falls to the fallback and no collection
        // is created.
        let id = resolve_note_collection(&mut tx, "alice", &sharees, false, fallback, &mut cache)
            .await
            .unwrap();
        assert_eq!(id, fallback);
        let shared_count: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM note_collections WHERE owner_username = 'alice' AND name LIKE 'Shared with %'",
        )
        .fetch_one(&mut *tx)
        .await
        .unwrap();
        assert_eq!(shared_count, 0);

        // Opted in: the "Shared with …" collection is created and used.
        let id = resolve_note_collection(&mut tx, "alice", &sharees, true, fallback, &mut cache)
            .await
            .unwrap();
        assert_ne!(id, fallback);
        let name: String = sqlx::query_scalar("SELECT name FROM note_collections WHERE id = ?")
            .bind(id)
            .fetch_one(&mut *tx)
            .await
            .unwrap();
        assert_eq!(name, "Shared with a@example.com");

        // An unshared note always uses the fallback, even when split is on.
        let id = resolve_note_collection(&mut tx, "alice", &[], true, fallback, &mut cache)
            .await
            .unwrap();
        assert_eq!(id, fallback);
    }
}
