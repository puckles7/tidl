//! Consolidation between notes and markdown source files (backlog #122).
//!
//! Two directions, sharing one grammar (`#task`/`#await`/`#done` tokens are the
//! same in notes and markdown — see `crate::note_tokens`):
//!
//! - [`consolidate_note_to_markdown`] — write a note out as a new `.md` file
//!   (with any inline images written as sidecar files next to it), optionally
//!   deleting the note.
//! - [`pull_markdown_into_note`] — import a markdown file as a new note
//!   (importing local relative images as note images), optionally deleting the
//!   file.
//!
//! The pure text transforms live at the top so they're unit-testable without a
//! live server context.

// The pure helpers are exercised by the SSR bodies and unit tests; in a plain
// hydrate-only build they exist only to be dead code.
#![cfg_attr(not(any(feature = "ssr", test)), allow(dead_code))]

use leptos::prelude::*;
use server_fn::ServerFnError;

use crate::types::{note_image_token_at, DirListing};

#[cfg(feature = "ssr")]
use crate::types::{DirEntryInfo, NewNote};

#[cfg(feature = "ssr")]
use super::auth;
#[cfg(feature = "ssr")]
use crate::types::{note_image_ids_in_text, rewrite_note_image_temp_tokens};
#[cfg(feature = "ssr")]
use sqlx::Row;

// ── Pure helpers ──────────────────────────────────────────────────────────────

/// Content types accepted for images moved into a markdown source, mapped to
/// the on-disk extension used for their sidecar file.
fn image_extension_for_content_type(content_type: &str) -> Option<&'static str> {
    match content_type {
        "image/png" => Some("png"),
        "image/jpeg" => Some("jpg"),
        "image/gif" => Some("gif"),
        "image/webp" => Some("webp"),
        _ => None,
    }
}

/// Content type for a local image path, from its extension. `None` for
/// extensions we don't import (external or unknown assets are left alone).
fn content_type_for_image_path(path: &str) -> Option<&'static str> {
    let ext = std::path::Path::new(path)
        .extension()
        .and_then(|e| e.to_str())?
        .to_ascii_lowercase();
    match ext.as_str() {
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        _ => None,
    }
}

/// Trim a user-supplied file name down to a single safe path component.
/// Strips any directory part, control characters, and leading/trailing dots
/// and spaces. Empty when nothing usable remains.
fn sanitize_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base.chars().filter(|c| !c.is_control()).collect();
    cleaned.trim_matches([' ', '.']).to_string()
}

/// Normalise a user-supplied relative folder into `/`-separated components,
/// dropping empties and `.`/`..` so the result can never escape the source
/// root. An empty result means the source root itself.
fn normalize_rel_folder(folder: &str) -> String {
    folder
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|p| !p.is_empty() && *p != "." && *p != "..")
        .collect::<Vec<_>>()
        .join("/")
}

/// Join two relative path fragments with a single `/`.
fn join_rel(a: &str, b: &str) -> String {
    match (a.is_empty(), b.is_empty()) {
        (true, _) => b.to_string(),
        (_, true) => a.to_string(),
        (false, false) => format!("{a}/{b}"),
    }
}

/// Replace every stored `@img:<id>` token in `text` with a markdown image
/// link. Tokens with no entry in `links` (e.g. a stale id with no row) are
/// left untouched. `links` is `(image_id, relative_path)`.
fn rewrite_note_image_tokens(text: &str, links: &[(i64, String)]) -> String {
    if links.is_empty() {
        return text.to_string();
    }
    let chars: Vec<char> = text.chars().collect();
    let mut out = String::with_capacity(text.len());
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '@' {
            if let Some((end, body)) = note_image_token_at(&chars, i) {
                if !body.starts_with('t') {
                    if let Ok(id) = body.parse::<i64>() {
                        if let Some((_, rel)) = links.iter().find(|(i, _)| *i == id) {
                            let alt = std::path::Path::new(rel)
                                .file_stem()
                                .and_then(|s| s.to_str())
                                .unwrap_or("image");
                            out.push_str(&format!("![{alt}](<{rel}>)"));
                            i = end;
                            continue;
                        }
                    }
                }
            }
        }
        out.push(chars[i]);
        i += 1;
    }
    out
}

/// True when a line already ends in a CommonMark hard break (two spaces or a
/// trailing backslash).
fn has_hard_break(line: &str) -> bool {
    line.ends_with("  ") || line.ends_with('\\')
}

/// The fence character when `line` opens a fenced code block, i.e. starts a
/// run of at least three backticks or tildes (optionally with an info string).
fn fence_open(line: &str) -> Option<char> {
    let trimmed = line.trim_start();
    let c = trimmed.chars().next()?;
    if c != '`' && c != '~' {
        return None;
    }
    (trimmed.chars().take_while(|&x| x == c).count() >= 3).then_some(c)
}

/// True when `line` closes a fence opened with `fence` (a run of at least
/// three of the same character, with only whitespace after it).
fn fence_close(line: &str, fence: char) -> bool {
    let trimmed = line.trim();
    let count = trimmed.len() - trimmed.trim_start_matches(fence).len();
    count >= 3 && trimmed[count..].trim().is_empty()
}

/// Preserve a note's visual line structure when it becomes markdown. A single
/// newline is a CommonMark *soft* break that renders as a space, so mark each
/// one as a hard break. Blank lines (paragraph breaks), lines already ending
/// in a hard break, and fenced code blocks are left untouched.
fn preserve_line_breaks(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + text.len() / 8);
    let mut in_fence: Option<char> = None;
    for line in text.split_inclusive('\n') {
        let (content, newline) = match line.strip_suffix('\n') {
            Some(rest) if rest.ends_with('\r') => (&rest[..rest.len() - 1], "\r\n"),
            Some(rest) => (rest, "\n"),
            None => (line, ""),
        };
        if let Some(fence) = in_fence {
            if fence_close(content, fence) {
                in_fence = None;
            }
        } else if let Some(fence) = fence_open(content) {
            in_fence = Some(fence);
        } else if !newline.is_empty() && !content.trim().is_empty() && !has_hard_break(content) {
            out.push_str(content);
            out.push_str("  ");
            out.push_str(newline);
            continue;
        }
        out.push_str(content);
        out.push_str(newline);
    }
    out
}

/// Build the markdown file body for a note: an optional `# Title` heading,
/// blank line, then the (image-rewritten) body with its newlines preserved as
/// hard breaks.
fn build_markdown_document(title: Option<&str>, text: &str, links: &[(i64, String)]) -> String {
    let body = preserve_line_breaks(&rewrite_note_image_tokens(text, links));
    match title.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => format!("# {t}\n\n{}", body.trim_start_matches(['\n', '\r'])),
        None => body,
    }
}

/// Split a markdown file into `(title, body)` for import: if the first
/// non-blank line is an ATX level-1 heading (`# Title`), that becomes the
/// note title and everything after it is the body. Leading blank lines are
/// dropped either way.
fn split_leading_h1(content: &str) -> (Option<String>, String) {
    let mut offset = 0usize;
    for line in content.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.trim().is_empty() {
            offset += line.len();
            continue;
        }
        if let Some(rest) = trimmed.trim_start().strip_prefix("# ") {
            let title = rest.trim().to_string();
            let body = content[offset + line.len()..]
                .trim_start_matches(['\n', '\r'])
                .to_string();
            return (Some(title), body);
        }
        break;
    }
    (None, content.trim_start_matches(['\n', '\r']).to_string())
}

/// One markdown image reference `![alt](dest)` and its byte span.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MarkdownImageRef {
    start: usize,
    end: usize,
    alt: String,
    dest: String,
}

/// Find every `![alt](dest)` reference. `dest` is stripped of a surrounding
/// `<…>` (CommonMark's angle-bracket form for paths with spaces).
fn find_markdown_image_refs(content: &str) -> Vec<MarkdownImageRef> {
    use std::sync::LazyLock;
    static RE: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"!\[([^\]]*)\]\(([^)]*)\)").unwrap());
    RE.captures_iter(content)
        .filter_map(|cap| {
            let m = cap.get(0)?;
            let alt = cap.get(1)?.as_str();
            let mut dest = cap.get(2)?.as_str().trim();
            if dest.starts_with('<') && dest.ends_with('>') && dest.len() >= 2 {
                dest = &dest[1..dest.len() - 1];
            }
            Some(MarkdownImageRef {
                start: m.start(),
                end: m.end(),
                alt: alt.to_string(),
                dest: dest.to_string(),
            })
        })
        .collect()
}

/// True for a destination that should be imported from disk (a plain relative
/// path). External URLs, data URIs and absolute paths are left as-is.
fn is_local_image_dest(dest: &str) -> bool {
    let d = dest.trim();
    if d.is_empty() || d.starts_with('/') || d.starts_with('#') {
        return false;
    }
    if d.contains("://") || d.starts_with("data:") {
        return false;
    }
    content_type_for_image_path(d).is_some()
}

/// Resolve a relative destination against a containing directory, honouring
/// `.` and `..` (popping, never escaping above the root) — used to turn a
/// markdown image's file-relative path into a source-relative one.
fn resolve_rel(base_dir: &str, dest: &str) -> String {
    let mut parts: Vec<&str> = base_dir.split('/').filter(|p| !p.is_empty()).collect();
    for seg in dest.split('/') {
        match seg {
            "" | "." => {}
            ".." => {
                parts.pop();
            }
            s => parts.push(s),
        }
    }
    parts.join("/")
}

/// Percent-encode a relative path for use in a URL, leaving `/` and the
/// RFC 3986 unreserved set alone.
fn percent_encode_path(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' | b'/' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// Inverse of [`percent_encode_path`]. Malformed escapes are left verbatim.
fn percent_decode_path(s: &str) -> String {
    fn hex(b: u8) -> Option<u8> {
        match b {
            b'0'..=b'9' => Some(b - b'0'),
            b'a'..=b'f' => Some(b - b'a' + 10),
            b'A'..=b'F' => Some(b - b'A' + 10),
            _ => None,
        }
    }
    let bytes = s.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let (Some(h), Some(l)) = (hex(bytes[i + 1]), hex(bytes[i + 2])) {
                out.push(h * 16 + l);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// The authenticated URL a markdown source image is served from (see the
/// `GET /api/md-images/{source_id}/{*rel_path}` route in `tidl-server`).
fn md_image_url(source_id: i64, image_rel_path: &str) -> String {
    format!(
        "/api/md-images/{source_id}/{}",
        percent_encode_path(image_rel_path)
    )
}

/// Rewrite every local relative markdown image destination in `content` to its
/// serving URL so images render in the preview. `file_dir_rel` is the
/// containing file's folder relative to the source root, so a destination is
/// resolved exactly as a browser would resolve it against the file. External
/// and absolute destinations are left untouched.
pub(crate) fn rewrite_markdown_image_dests(
    content: &str,
    source_id: i64,
    file_dir_rel: &str,
) -> String {
    let mut out = String::with_capacity(content.len());
    let mut cursor = 0usize;
    for img in find_markdown_image_refs(content) {
        out.push_str(&content[cursor..img.start]);
        if is_local_image_dest(&img.dest) {
            let image_rel = resolve_rel(file_dir_rel, &percent_decode_path(&img.dest));
            out.push_str(&format!(
                "![{}]({})",
                img.alt,
                md_image_url(source_id, &image_rel)
            ));
        } else {
            out.push_str(&content[img.start..img.end]);
        }
        cursor = img.end;
    }
    out.push_str(&content[cursor..]);
    out
}

// ── Source-scoped folder browser ──────────────────────────────────────────────

/// List the subdirectories of `rel_path` *within* a markdown source (backlog
/// #122), for the "folder inside the repo" picker. Paths are relative to the
/// source root, never absolute — unlike [`list_server_directories`], which
/// walks the whole server filesystem.
#[server]
pub async fn list_source_directories(
    source_id: i64,
    rel_path: Option<String>,
) -> Result<DirListing, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::DataDir;
        use axum::Extension;
        use leptos_axum::extract;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;

        let (source_type, config) = load_source(&pool, source_id, &username).await?;
        let base_dir =
            super::files::resolve_markdown_source_dir(&source_type, &config, source_id, &data_dir)
                .ok_or_else(|| ServerFnError::new("Invalid source config"))?;

        let rel = normalize_rel_folder(rel_path.as_deref().unwrap_or(""));
        let dir = if rel.is_empty() {
            base_dir.clone()
        } else {
            base_dir.join(&rel)
        };

        let mut dirs = Vec::new();
        let mut entries = tokio::fs::read_dir(&dir)
            .await
            .map_err(|e| ServerFnError::new(format!("Cannot read folder: {e}")))?;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            if !file_type.is_dir() || entry.file_name() == ".git" {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_string();
            dirs.push(DirEntryInfo {
                path: join_rel(&rel, &name),
                name,
            });
        }
        dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        let parent = if rel.is_empty() {
            None
        } else {
            Some(
                rel.rsplit_once('/')
                    .map(|(p, _)| p.to_string())
                    .unwrap_or_default(),
            )
        };

        return Ok(DirListing {
            path: rel,
            parent,
            dirs,
        });
    }
    #[allow(unreachable_code)]
    Err(ServerFnError::new("unreachable"))
}

// ── Server-side helpers ───────────────────────────────────────────────────────

#[cfg(feature = "ssr")]
async fn load_source(
    pool: &sqlx::SqlitePool,
    source_id: i64,
    username: &str,
) -> Result<(String, serde_json::Value), ServerFnError> {
    let row =
        sqlx::query("SELECT type, config FROM markdown_sources WHERE id = ? AND username = ?")
            .bind(source_id)
            .bind(username)
            .fetch_optional(pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
            .ok_or_else(|| ServerFnError::new("Source not found"))?;
    let source_type: String = row.try_get("type").unwrap_or_default();
    let config: String = row.try_get("config").unwrap_or_default();
    let config: serde_json::Value = serde_json::from_str(&config).unwrap_or_default();
    Ok((source_type, config))
}

/// Atomic write (hidden `.tmp` then rename), creating parent directories.
#[cfg(feature = "ssr")]
async fn atomic_write(abs_path: &std::path::Path, bytes: &[u8]) -> Result<(), ServerFnError> {
    if let Some(parent) = abs_path.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| ServerFnError::new(format!("Cannot create folder: {e}")))?;
    }
    let tmp_path = {
        let mut p = abs_path.to_path_buf();
        let name = p
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .to_string();
        p.set_file_name(format!(".{name}.tmp"));
        p
    };
    tokio::fs::write(&tmp_path, bytes)
        .await
        .map_err(|e| ServerFnError::new(format!("Write failed: {e}")))?;
    tokio::fs::rename(&tmp_path, abs_path)
        .await
        .map_err(|e| ServerFnError::new(format!("Rename failed: {e}")))?;
    Ok(())
}

// ── Note → markdown ───────────────────────────────────────────────────────────

/// Write a note out as a new markdown file in `source_id`, under `folder`,
/// named `filename` (a `.md` extension is added when missing). Inline images
/// are written as sidecar files in a sibling `<stem>.assets/` folder and the
/// note's `@img:<id>` tokens rewritten to markdown links. Errors if a file of
/// that name already exists. When `delete_note` is set, the note (and its
/// task/await events) is deleted after the file is written successfully.
/// Returns the new file's path relative to the source root.
#[server]
pub async fn consolidate_note_to_markdown(
    note_id: i64,
    source_id: i64,
    folder: String,
    filename: String,
    delete_note: bool,
) -> Result<String, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::api::notes::{delete_note_inner, has_collection_access};
        use crate::ssr_types::{DataDir, GitCommitPusher, MarkdownSyncTrigger};
        use axum::Extension;
        use leptos_axum::extract;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;
        let Extension(git_pusher): Extension<GitCommitPusher> = extract().await?;
        let Extension(md_trigger): Extension<MarkdownSyncTrigger> = extract().await?;

        // Load + authorise the note.
        let note_row = sqlx::query("SELECT collection_id, title, text FROM notes WHERE id = ?")
            .bind(note_id)
            .fetch_optional(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
            .ok_or_else(|| ServerFnError::new("Note not found"))?;
        let collection_id: i64 = note_row.try_get("collection_id").unwrap_or_default();
        if !has_collection_access(&pool, &username, collection_id).await? {
            return Err(ServerFnError::new("Not authorized for this collection"));
        }
        let title: Option<String> = note_row.try_get("title").unwrap_or_default();
        let text: String = note_row.try_get("text").unwrap_or_default();

        // Resolve source + target paths.
        let (source_type, config) = load_source(&pool, source_id, &username).await?;
        let base_dir =
            super::files::resolve_markdown_source_dir(&source_type, &config, source_id, &data_dir)
                .ok_or_else(|| ServerFnError::new("Invalid source config"))?;

        let folder = normalize_rel_folder(&folder);
        let mut filename = sanitize_filename(&filename);
        if filename.is_empty() {
            return Err(ServerFnError::new("Enter a file name"));
        }
        let ext = std::path::Path::new(&filename)
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.to_ascii_lowercase());
        if !matches!(ext.as_deref(), Some("md") | Some("markdown")) {
            filename.push_str(".md");
        }
        let stem = filename
            .rsplit_once('.')
            .map(|(s, _)| s)
            .unwrap_or(&filename)
            .to_string();
        let rel_path = join_rel(&folder, &filename);
        if tokio::fs::try_exists(base_dir.join(&rel_path))
            .await
            .unwrap_or(false)
        {
            return Err(ServerFnError::new(format!(
                "A file named \"{filename}\" already exists in that folder"
            )));
        }

        // Write inline images as sidecars under `<stem>.assets/`, building the
        // id → relative-path map the body rewrite uses.
        let assets_rel_dir = join_rel(&folder, &format!("{stem}.assets"));
        let mut links: Vec<(i64, String)> = Vec::new();
        let mut extra_files: Vec<String> = Vec::new();
        for image_id in note_image_ids_in_text(&text) {
            let img = sqlx::query(
                "SELECT data, content_type, filename FROM note_images WHERE id = ? AND note_id = ?",
            )
            .bind(image_id)
            .bind(note_id)
            .fetch_optional(&pool)
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
            let Some(img) = img else { continue };
            let content_type: String = img.try_get("content_type").unwrap_or_default();
            let Some(ext) = image_extension_for_content_type(&content_type) else {
                continue;
            };
            let data: Vec<u8> = img.try_get("data").unwrap_or_default();
            let stored_name: Option<String> = img.try_get("filename").unwrap_or_default();
            let mut name = stored_name
                .as_deref()
                .map(sanitize_filename)
                .unwrap_or_default();
            if name.is_empty() {
                name = format!("image-{image_id}.{ext}");
            } else if std::path::Path::new(&name).extension().is_none() {
                name.push('.');
                name.push_str(ext);
            }
            // Keep names unique within this batch.
            let mut candidate = name.clone();
            let mut n = 2;
            while links.iter().any(|(_, p)| {
                std::path::Path::new(p).file_name().and_then(|f| f.to_str()) == Some(&candidate)
            }) {
                candidate = format!("{n}-{name}");
                n += 1;
            }
            // On disk the sidecar lives next to the file (under `folder`);
            // the markdown link is relative to the file itself, so it omits
            // `folder`.
            let asset_rel = join_rel(&assets_rel_dir, &candidate);
            let asset_link = format!("{stem}.assets/{candidate}");
            atomic_write(&base_dir.join(&asset_rel), &data).await?;
            extra_files.push(asset_rel);
            links.push((image_id, asset_link));
        }

        // Write the markdown file last, so a failure above leaves no partial doc.
        let content = build_markdown_document(title.as_deref(), &text, &links);
        atomic_write(&base_dir.join(&rel_path), content.as_bytes()).await?;

        if source_type == "git" {
            let mut paths = vec![rel_path.clone()];
            paths.extend(extra_files);
            git_pusher.0(source_id, paths)
                .await
                .map_err(|e| ServerFnError::new(format!("Git push failed: {e}")))?;
        }
        let _ = md_trigger.0.send(source_id);

        if delete_note {
            delete_note_inner(&pool, &username, note_id).await?;
        }
        return Ok(rel_path);
    }
    #[allow(unreachable_code)]
    Err(ServerFnError::new("unreachable"))
}

// ── Markdown → note ───────────────────────────────────────────────────────────

/// Import a markdown file as a new note in `collection_id`. A leading `# Title`
/// becomes the note's title; local relative images are imported as note images
/// (rewritten to `@img:<id>`), external URLs are left as text. When
/// `delete_file` is set, the file is removed (and committed for git sources)
/// after the note is created. Returns the new note's id.
#[server]
pub async fn pull_markdown_into_note(
    source_id: i64,
    rel_path: String,
    collection_id: i64,
    delete_file: bool,
) -> Result<i64, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::api::notes::{
            has_collection_access, insert_note_image, insert_note_row, rewrite_note_body,
            validate_note_image_bytes,
        };
        use crate::ssr_types::{DataDir, GitCommitPusher, MarkdownSyncTrigger};
        use axum::Extension;
        use leptos_axum::extract;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;
        let Extension(git_pusher): Extension<GitCommitPusher> = extract().await?;
        let Extension(md_trigger): Extension<MarkdownSyncTrigger> = extract().await?;

        if !has_collection_access(&pool, &username, collection_id).await? {
            return Err(ServerFnError::new("Not authorized for this collection"));
        }

        let (source_type, config) = load_source(&pool, source_id, &username).await?;
        let base_dir =
            super::files::resolve_markdown_source_dir(&source_type, &config, source_id, &data_dir)
                .ok_or_else(|| ServerFnError::new("Invalid source config"))?;
        let abs_path = base_dir.join(&rel_path);
        let content = tokio::fs::read_to_string(&abs_path)
            .await
            .map_err(|e| ServerFnError::new(format!("Cannot read file: {e}")))?;

        let (title, body) = split_leading_h1(&content);
        if body.trim().is_empty() {
            return Err(ServerFnError::new("The file has no content to import"));
        }

        // Resolve local image refs to (temp-key, bytes, content-type, name) and
        // rewrite the body with `@img:t<key>` placeholders, mirroring the
        // note editor's own staging flow.
        let mut staged: Vec<(String, Vec<u8>, String, Option<String>)> = Vec::new();
        let mut rewritten = String::with_capacity(body.len());
        let mut cursor = 0usize;
        for img in find_markdown_image_refs(&body) {
            rewritten.push_str(&body[cursor..img.start]);
            cursor = img.end;
            if !is_local_image_dest(&img.dest) {
                rewritten.push_str(&body[img.start..img.end]);
                continue;
            }
            let Some(content_type) = content_type_for_image_path(&img.dest) else {
                rewritten.push_str(&body[img.start..img.end]);
                continue;
            };
            // Resolve relative to the markdown file's own directory, and keep
            // the result inside the source root (no `..` escape).
            let file_dir = abs_path.parent().unwrap_or(&base_dir);
            let candidate = file_dir.join(&img.dest);
            let Ok(canon_base) = tokio::fs::canonicalize(&base_dir).await else {
                rewritten.push_str(&body[img.start..img.end]);
                continue;
            };
            let Ok(canon_img) = tokio::fs::canonicalize(&candidate).await else {
                rewritten.push_str(&body[img.start..img.end]);
                continue;
            };
            if !canon_img.starts_with(&canon_base) {
                rewritten.push_str(&body[img.start..img.end]);
                continue;
            }
            let Ok(bytes) = tokio::fs::read(&canon_img).await else {
                rewritten.push_str(&body[img.start..img.end]);
                continue;
            };
            if validate_note_image_bytes(&bytes, content_type).is_err() {
                rewritten.push_str(&body[img.start..img.end]);
                continue;
            }
            let key = staged.len().to_string();
            let filename = canon_img
                .file_name()
                .map(|f| f.to_string_lossy().to_string());
            rewritten.push_str(&format!(" @img:t{key} "));
            staged.push((key, bytes, content_type.to_string(), filename));
        }
        rewritten.push_str(&body[cursor..]);

        let input = NewNote {
            collection_id,
            title,
            text: rewritten,
            pinned: false,
            calendar_name: None,
            labels: Vec::new(),
        };

        let mut tx = pool
            .begin()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        let note_id = insert_note_row(&mut tx, &username, &input).await?;
        let mut map: Vec<(String, i64)> = Vec::with_capacity(staged.len());
        for (key, bytes, content_type, filename) in &staged {
            let image_id =
                insert_note_image(&mut tx, note_id, bytes, content_type, filename.as_deref())
                    .await?;
            map.push((key.clone(), image_id));
        }
        if !map.is_empty() {
            let final_text = rewrite_note_image_temp_tokens(&input.text, &map);
            rewrite_note_body(&mut tx, &username, note_id, final_text).await?;
        }
        tx.commit()
            .await
            .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        if delete_file {
            tokio::fs::remove_file(&abs_path)
                .await
                .map_err(|e| ServerFnError::new(format!("Delete failed: {e}")))?;
            if source_type == "git" {
                git_pusher.0(source_id, vec![rel_path.clone()])
                    .await
                    .map_err(|e| ServerFnError::new(format!("Git push failed: {e}")))?;
            }
            let _ = md_trigger.0.send(source_id);
        }

        return Ok(note_id);
    }
    #[allow(unreachable_code)]
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitize_filename_strips_paths_and_dots() {
        assert_eq!(sanitize_filename("my note.md"), "my note.md");
        assert_eq!(sanitize_filename("../evil.md"), "evil.md");
        assert_eq!(sanitize_filename("a/b/c.md"), "c.md");
        assert_eq!(sanitize_filename("..."), "");
    }

    #[test]
    fn normalize_rel_folder_drops_escapes() {
        assert_eq!(normalize_rel_folder("a/b"), "a/b");
        assert_eq!(normalize_rel_folder("/a//b/"), "a/b");
        assert_eq!(normalize_rel_folder("../.."), "");
        assert_eq!(normalize_rel_folder("a/../b"), "a/b");
    }

    #[test]
    fn build_markdown_document_prepends_h1_and_rewrites_images() {
        let links = vec![(7i64, "note.assets/pic.png".to_string())];
        let out = build_markdown_document(Some("My Title"), "a @img:7 b", &links);
        assert_eq!(out, "# My Title\n\na ![pic](<note.assets/pic.png>) b");
    }

    #[test]
    fn build_markdown_document_without_title() {
        let out = build_markdown_document(None, "just body", &[]);
        assert_eq!(out, "just body");
    }

    #[test]
    fn build_markdown_document_preserves_single_newlines_as_hard_breaks() {
        let out = build_markdown_document(None, "one\ntwo\n\nthree", &[]);
        assert_eq!(out, "one  \ntwo  \n\nthree");
    }

    #[test]
    fn build_markdown_document_keeps_existing_breaks_and_code_fences() {
        let text = "a  \nb\\\n\n```\ncode\nmore\n```\nafter";
        let out = build_markdown_document(None, text, &[]);
        assert_eq!(out, "a  \nb\\\n\n```\ncode\nmore\n```\nafter");
    }

    #[test]
    fn build_markdown_document_preserves_crlf_newlines() {
        let out = build_markdown_document(None, "one\r\ntwo\r\n", &[]);
        assert_eq!(out, "one  \r\ntwo  \r\n");
    }

    #[test]
    fn split_leading_h1_extracts_title() {
        let (title, body) = split_leading_h1("\n\n# Hello World\n\nrest of body\n");
        assert_eq!(title.as_deref(), Some("Hello World"));
        assert_eq!(body, "rest of body\n");
    }

    #[test]
    fn split_leading_h1_without_title_keeps_body() {
        let (title, body) = split_leading_h1("no heading here\nsecond line");
        assert_eq!(title, None);
        assert_eq!(body, "no heading here\nsecond line");
    }

    #[test]
    fn split_leading_h1_ignores_h2() {
        let (title, body) = split_leading_h1("## not a title\nbody");
        assert_eq!(title, None);
        assert_eq!(body, "## not a title\nbody");
    }

    #[test]
    fn find_markdown_image_refs_handles_angles() {
        let refs = find_markdown_image_refs("x ![alt](<a b/c.png>) y ![z](http://e/x.png)");
        assert_eq!(refs.len(), 2);
        assert_eq!(refs[0].dest, "a b/c.png");
        assert_eq!(refs[1].dest, "http://e/x.png");
    }

    #[test]
    fn is_local_image_dest_filters_remote_and_absolute() {
        assert!(is_local_image_dest("a/pic.png"));
        assert!(!is_local_image_dest("http://x/y.png"));
        assert!(!is_local_image_dest("https://x/y.png"));
        assert!(!is_local_image_dest("/abs/y.png"));
        assert!(!is_local_image_dest("data:image/png;base64,zz"));
        assert!(!is_local_image_dest("a/pic.txt"));
    }

    #[test]
    fn resolve_rel_honours_dotdot_without_escaping() {
        assert_eq!(resolve_rel("a/b", "img.png"), "a/b/img.png");
        assert_eq!(resolve_rel("a/b", "../c.png"), "a/c.png");
        assert_eq!(resolve_rel("a/b", "./c.png"), "a/b/c.png");
        assert_eq!(resolve_rel("", "../../x.png"), "x.png");
    }

    #[test]
    fn percent_round_trips_spaces_and_specials() {
        let encoded = percent_encode_path("a b/c#d.png");
        assert_eq!(encoded, "a%20b/c%23d.png");
        assert_eq!(percent_decode_path(&encoded), "a b/c#d.png");
        // Malformed escapes survive verbatim.
        assert_eq!(percent_decode_path("a%zz"), "a%zz");
    }

    #[test]
    fn rewrite_markdown_image_dests_serves_local_and_leaves_remote() {
        let content = "![pic](<img/my pic.png>) and ![x](https://e/x.png)";
        let out = rewrite_markdown_image_dests(content, 7, "notes");
        assert_eq!(
            out,
            "![pic](/api/md-images/7/notes/img/my%20pic.png) and ![x](https://e/x.png)"
        );
    }
}
