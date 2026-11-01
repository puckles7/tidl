//! GET /api/md-images/{source_id}/{*rel_path} — serve an image file from a
//! markdown source (backlog #122).
//!
//! The rendered markdown preview rewrites local relative `![](…)` destinations
//! to this route, because a browser resolves a plain relative path against the
//! page URL and 404s. Same shape as `note_image.rs`: a plain Axum GET route
//! (not a `#[server]` fn) so an `<img src>` can point straight at it and lean
//! on normal HTTP caching. Session-cookie auth mirrors `export.rs`.
//!
//! Only image-typed paths are served, and the resolved path is canonicalised
//! and required to stay inside the source root — the markdown editor's own
//! `get_source_file_content` reads arbitrary text files by relative path, but
//! this route is reachable directly from the browser, so it is deliberately
//! narrower.

use axum::{
    extract::Path,
    http::{header, HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    Extension,
};
use sqlx::{Row, SqlitePool};

use tidl_web::api::resolve_markdown_source_dir;
use tidl_web::ssr_types::{DataDir, SessionCookieName};

use crate::session::session_token_from_headers;

/// Extensions for which a content type is served. Anything else 404s, so this
/// route can't be used to exfiltrate arbitrary files from the source tree.
const ALLOWED_IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp"];

fn content_type_for_extension(ext: &str) -> &'static str {
    match ext {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        _ => "application/octet-stream",
    }
}

pub async fn get_md_source_image(
    Extension(pool): Extension<SqlitePool>,
    Extension(DataDir(data_dir)): Extension<DataDir>,
    Extension(cookie_name): Extension<SessionCookieName>,
    headers: HeaderMap,
    Path((source_id, rel_path)): Path<(i64, String)>,
) -> Response {
    let token = match session_token_from_headers(&headers, &cookie_name) {
        Some(t) => t,
        None => return (StatusCode::UNAUTHORIZED, "Not authenticated").into_response(),
    };
    let username: Option<String> = sqlx::query_scalar(
        "SELECT username FROM sessions WHERE token = ? AND expires_at > datetime('now')",
    )
    .bind(&token)
    .fetch_optional(&pool)
    .await
    .ok()
    .flatten();
    let Some(username) = username else {
        return (StatusCode::UNAUTHORIZED, "Session invalid or expired").into_response();
    };

    // Resolve the source directory (owner-scoped).
    let row =
        sqlx::query("SELECT type, config FROM markdown_sources WHERE id = ? AND username = ?")
            .bind(source_id)
            .bind(&username)
            .fetch_optional(&pool)
            .await;
    let Ok(Some(row)) = row else {
        return (StatusCode::NOT_FOUND, "Source not found").into_response();
    };
    let source_type: String = row.try_get("type").unwrap_or_default();
    let config_json: String = row.try_get("config").unwrap_or_default();
    let config: serde_json::Value = serde_json::from_str(&config_json).unwrap_or_default();
    let Some(base_dir) = resolve_markdown_source_dir(&source_type, &config, source_id, &data_dir)
    else {
        return (StatusCode::NOT_FOUND, "Invalid source").into_response();
    };

    // Canonicalise and require containment so `..` can't escape the source.
    let (Ok(canon_base), Ok(canon_file)) = (
        tokio::fs::canonicalize(&base_dir).await,
        tokio::fs::canonicalize(base_dir.join(&rel_path)).await,
    ) else {
        return (StatusCode::NOT_FOUND, "No such image").into_response();
    };
    if !canon_file.starts_with(&canon_base) {
        return (StatusCode::NOT_FOUND, "No such image").into_response();
    }
    let ext = canon_file
        .extension()
        .and_then(|e| e.to_str())
        .map(|e| e.to_ascii_lowercase())
        .unwrap_or_default();
    if !ALLOWED_IMAGE_EXTENSIONS.contains(&ext.as_str()) {
        return (StatusCode::NOT_FOUND, "Not an image").into_response();
    }

    let bytes = match tokio::fs::read(&canon_file).await {
        Ok(b) => b,
        Err(_) => return (StatusCode::NOT_FOUND, "No such image").into_response(),
    };

    (
        StatusCode::OK,
        [
            (
                header::CONTENT_TYPE,
                content_type_for_extension(&ext).to_string(),
            ),
            (header::CACHE_CONTROL, "private, max-age=86400".to_string()),
        ],
        bytes,
    )
        .into_response()
}
