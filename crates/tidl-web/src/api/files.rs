//! Server functions for reading and writing markdown files via the in-app editor.

use leptos::prelude::*;

#[cfg(feature = "ssr")]
use super::auth;
#[cfg(feature = "ssr")]
use crate::types::DirEntryInfo;
use crate::types::{DirListing, MarkdownFileEntry};
#[cfg(feature = "ssr")]
use sqlx::Row;

/// Resolve the directory a markdown source's files live in.
///
/// Git sources are always cloned to `<data_dir>/repos/<source_id>/`, regardless
/// of what's in `config` (which only carries `url`/`branch` for git sources —
/// there is no `path` key). Filesystem sources carry their directory in
/// `config.path`.
#[cfg(feature = "ssr")]
pub fn resolve_markdown_source_dir(
    source_type: &str,
    config: &serde_json::Value,
    source_id: i64,
    data_dir: &std::path::Path,
) -> Option<std::path::PathBuf> {
    if source_type == "git" {
        Some(data_dir.join("repos").join(source_id.to_string()))
    } else {
        config
            .get("path")
            .and_then(|p| p.as_str())
            .filter(|p| !p.is_empty())
            .map(std::path::PathBuf::from)
    }
}

/// Read the raw content of a markdown file from a markdown source.
/// Verifies that the source belongs to the authenticated user.
#[server]
pub async fn get_source_file_content(
    source_id: i64,
    rel_path: String,
) -> Result<String, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::DataDir;
        use axum::Extension;
        use leptos_axum::extract;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;

        let row =
            sqlx::query("SELECT type, config FROM markdown_sources WHERE id = ? AND username = ?")
                .bind(source_id)
                .bind(&username)
                .fetch_optional(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
                .ok_or_else(|| ServerFnError::new("Source not found"))?;

        let source_type: String = row.try_get("type").unwrap_or_default();
        let config: String = row.try_get("config").unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&config).unwrap_or_default();

        let base_dir = resolve_markdown_source_dir(&source_type, &v, source_id, &data_dir)
            .ok_or_else(|| ServerFnError::new("Invalid source config"))?;
        let abs_path = base_dir.join(&rel_path);

        let content = tokio::fs::read_to_string(&abs_path)
            .await
            .map_err(|e| ServerFnError::new(format!("Cannot read file: {e}")))?;

        return Ok(content);
    }
    #[allow(unreachable_code)]
    Ok(String::new())
}

/// Write markdown content and (for git sources) commit + push.
///
/// File write is atomic (write to a hidden .tmp file, then rename).
/// On push failure the local write is NOT rolled back — data safety first.
#[allow(unused_variables)]
#[server]
pub async fn save_markdown_file(
    source_id: i64,
    rel_path: String,
    content: String,
) -> Result<(), ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::{DataDir, GitCommitPusher, MarkdownSyncTrigger};
        use axum::Extension;
        use leptos_axum::extract;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;
        let Extension(git_pusher): Extension<GitCommitPusher> = extract().await?;
        let Extension(md_trigger): Extension<MarkdownSyncTrigger> = extract().await?;

        // Auth check: verify source belongs to user.
        let row =
            sqlx::query("SELECT type, config FROM markdown_sources WHERE id = ? AND username = ?")
                .bind(source_id)
                .bind(&username)
                .fetch_optional(&pool)
                .await
                .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?
                .ok_or_else(|| ServerFnError::new("Source not found"))?;

        let source_type: String = row.try_get("type").unwrap_or_default();
        let config: String = row.try_get("config").unwrap_or_default();
        let v: serde_json::Value = serde_json::from_str(&config).unwrap_or_default();

        let base_dir = resolve_markdown_source_dir(&source_type, &v, source_id, &data_dir)
            .ok_or_else(|| ServerFnError::new("Invalid source config"))?;
        let abs_path = base_dir.join(&rel_path);

        // Atomic write: hidden .tmp next to the target, then rename.
        let tmp_path = {
            let mut p = abs_path.clone();
            let name = p
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            p.set_file_name(format!(".{name}.tmp"));
            p
        };
        tokio::fs::write(&tmp_path, content.as_bytes())
            .await
            .map_err(|e| ServerFnError::new(format!("Write failed: {e}")))?;
        tokio::fs::rename(&tmp_path, &abs_path)
            .await
            .map_err(|e| ServerFnError::new(format!("Rename failed: {e}")))?;

        // For git sources: commit + push via the injected closure (which lives in tidl-server
        // and can use git2 — it never touches the WASM compilation boundary).
        if source_type == "git" {
            git_pusher.0(source_id, vec![rel_path.clone()])
                .await
                .map_err(|e| ServerFnError::new(format!("Git push failed: {e}")))?;
        }

        // Trigger immediate re-sync so the rendered output updates without delay.
        let _ = md_trigger.0.send(source_id);

        return Ok(());
    }
    #[allow(unreachable_code)]
    Ok(())
}

/// List repo-relative paths with uncommitted changes in a git-backed source's
/// server-side clone. Empty for filesystem sources.
#[server]
pub async fn get_markdown_source_git_status(source_id: i64) -> Result<Vec<String>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::GitStatusReader;
        use axum::Extension;
        use leptos_axum::extract;

        let (pool, username) = auth().await?;
        let owned: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM markdown_sources
             WHERE id = ? AND username = ? AND type = 'git'",
        )
        .bind(source_id)
        .bind(&username)
        .fetch_one(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        if owned == 0 {
            return Err(ServerFnError::new("Git source not found"));
        }

        let Extension(reader): Extension<GitStatusReader> = extract().await?;
        return reader.0(source_id).await.map_err(ServerFnError::new);
    }
    #[allow(unreachable_code)]
    Ok(vec![])
}

/// Commit and push every uncommitted change in a git-backed source's
/// server-side clone. Returns the number of paths committed (0 when the clone
/// was already clean) and wakes the markdown sync loop afterwards.
#[server]
pub async fn commit_and_push_markdown_source(source_id: i64) -> Result<usize, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::{GitCommitPusher, GitStatusReader, MarkdownSyncTrigger};
        use axum::Extension;
        use leptos_axum::extract;

        let (pool, username) = auth().await?;
        let owned: i64 = sqlx::query_scalar(
            "SELECT COUNT(*) FROM markdown_sources
             WHERE id = ? AND username = ? AND type = 'git'",
        )
        .bind(source_id)
        .bind(&username)
        .fetch_one(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;
        if owned == 0 {
            return Err(ServerFnError::new("Git source not found"));
        }

        let Extension(reader): Extension<GitStatusReader> = extract().await?;
        let paths = reader.0(source_id).await.map_err(ServerFnError::new)?;
        if paths.is_empty() {
            return Ok(0);
        }

        let Extension(pusher): Extension<GitCommitPusher> = extract().await?;
        pusher.0(source_id, paths.clone())
            .await
            .map_err(|e| ServerFnError::new(format!("Git commit/push failed: {e}")))?;

        if let Ok(Extension(md_trigger)) = extract::<Extension<MarkdownSyncTrigger>>().await {
            let _ = md_trigger.0.send(source_id);
        }

        return Ok(paths.len());
    }
    #[allow(unreachable_code)]
    Ok(0)
}

/// True for files the markdown sync loop treats as markdown — same extension
/// check as `tidl_markdown`'s filesystem source, duplicated here since this
/// walk skips reading file content entirely (unlike
/// `MarkdownSource::list_files`, which reads every file just to hash it).
#[cfg(feature = "ssr")]
pub(crate) fn is_markdown_path(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .map(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
        .unwrap_or(false)
}

/// Recursively collect markdown file paths under `dir` (relative to `base`)
/// into `out`, skipping `.git` (present in git-backed sources' clone).
/// Boxes the recursive call's future rather than pulling in an
/// async-recursion crate — `async fn` can't otherwise call itself, since
/// its desugared state machine would need to contain its own size.
#[cfg(feature = "ssr")]
pub(crate) fn walk_markdown_files<'a>(
    dir: &'a std::path::Path,
    base: &'a std::path::Path,
    out: &'a mut Vec<String>,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = ()> + Send + 'a>> {
    Box::pin(async move {
        let Ok(mut entries) = tokio::fs::read_dir(dir).await else {
            return;
        };
        while let Ok(Some(entry)) = entries.next_entry().await {
            if entry.file_name() == ".git" {
                continue;
            }
            let path = entry.path();
            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            if file_type.is_dir() {
                walk_markdown_files(&path, base, out).await;
            } else if file_type.is_file() && is_markdown_path(&path) {
                if let Ok(rel) = path.strip_prefix(base) {
                    out.push(rel.to_string_lossy().replace('\\', "/"));
                }
            }
        }
    })
}

/// List markdown files across all of the current user's *enabled* markdown
/// sources, for the file finder modal (backlog #75). Unlike
/// `tidl_markdown::MarkdownSource::list_files`, this only reads directory
/// entries — never file content — since the finder only needs paths.
#[server]
pub async fn list_all_markdown_files() -> Result<Vec<MarkdownFileEntry>, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        use crate::ssr_types::DataDir;
        use axum::Extension;
        use leptos_axum::extract;

        let (pool, username) = auth().await?;
        let Extension(DataDir(data_dir)): Extension<DataDir> = extract().await?;

        let rows = sqlx::query(
            "SELECT id, name, type, config, COALESCE(color, 'var(--pal-md-1)') AS color
             FROM markdown_sources WHERE username = ? AND enabled = 1",
        )
        .bind(&username)
        .fetch_all(&pool)
        .await
        .map_err(|e: sqlx::Error| ServerFnError::new(e.to_string()))?;

        let mut out = Vec::new();
        for row in rows {
            let source_id: i64 = row.try_get("id").unwrap_or_default();
            let source_name: String = row.try_get("name").unwrap_or_default();
            let source_color: String = row
                .try_get("color")
                .unwrap_or_else(|_| "var(--pal-md-1)".to_string());
            let source_type: String = row.try_get("type").unwrap_or_default();
            let config_str: String = row.try_get("config").unwrap_or_default();
            let config: serde_json::Value = serde_json::from_str(&config_str).unwrap_or_default();

            let Some(base_dir) =
                resolve_markdown_source_dir(&source_type, &config, source_id, &data_dir)
            else {
                continue;
            };

            let mut rel_paths = Vec::new();
            walk_markdown_files(&base_dir, &base_dir, &mut rel_paths).await;
            for rel_path in rel_paths {
                out.push(MarkdownFileEntry {
                    source_id,
                    source_name: source_name.clone(),
                    source_color: source_color.clone(),
                    rel_path,
                });
            }
        }

        return Ok(out);
    }
    #[allow(unreachable_code)]
    Ok(Vec::new())
}

/// List the subdirectories of `path` on the server, for the folder browser in
/// the markdown-source setup form.
///
/// `path` blank or `None` starts at the server process's current working
/// directory. Only directories are returned (never file contents or file
/// names), sorted case-insensitively. The path is canonicalised so the browser
/// always shows a stable absolute location and can walk back up via `parent`.
#[server]
pub async fn list_server_directories(path: Option<String>) -> Result<DirListing, ServerFnError> {
    #[cfg(feature = "ssr")]
    {
        let _ = auth().await?;

        let requested = path.as_deref().map(str::trim).filter(|p| !p.is_empty());

        let start = match requested {
            Some(p) => std::path::PathBuf::from(p),
            None => std::env::current_dir().map_err(|e| {
                ServerFnError::new(format!("Cannot determine start directory: {e}"))
            })?,
        };

        let canonical = tokio::fs::canonicalize(&start)
            .await
            .map_err(|e| ServerFnError::new(format!("Cannot open folder: {e}")))?;

        let meta = tokio::fs::metadata(&canonical)
            .await
            .map_err(|e: std::io::Error| ServerFnError::new(e.to_string()))?;
        if !meta.is_dir() {
            return Err(ServerFnError::new("Not a directory"));
        }

        let mut dirs = Vec::new();
        let mut entries = tokio::fs::read_dir(&canonical)
            .await
            .map_err(|e| ServerFnError::new(format!("Cannot read folder: {e}")))?;
        while let Ok(Some(entry)) = entries.next_entry().await {
            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            if !file_type.is_dir() {
                continue;
            }
            dirs.push(DirEntryInfo {
                name: entry.file_name().to_string_lossy().to_string(),
                path: entry.path().to_string_lossy().to_string(),
            });
        }
        dirs.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

        let parent = canonical.parent().map(|p| p.to_string_lossy().to_string());

        return Ok(DirListing {
            path: canonical.to_string_lossy().to_string(),
            parent,
            dirs,
        });
    }
    #[allow(unreachable_code)]
    Err(ServerFnError::new("unreachable"))
}

#[cfg(all(test, feature = "ssr"))]
mod resolve_markdown_source_dir_tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn git_source_resolves_to_data_dir_repos_id() {
        let data_dir = Path::new("/data");
        let config = serde_json::json!({"url": "https://example.com/r.git", "branch": "main"});
        assert_eq!(
            resolve_markdown_source_dir("git", &config, 7, data_dir),
            Some(PathBuf::from("/data/repos/7"))
        );
    }

    #[test]
    fn filesystem_source_uses_config_path() {
        let data_dir = Path::new("/data");
        let config = serde_json::json!({"path": "/notes", "recursive": true});
        assert_eq!(
            resolve_markdown_source_dir("filesystem", &config, 3, data_dir),
            Some(PathBuf::from("/notes"))
        );
    }

    #[test]
    fn filesystem_source_without_path_is_none() {
        let data_dir = Path::new("/data");
        let config = serde_json::json!({"path": "", "recursive": true});
        assert_eq!(
            resolve_markdown_source_dir("filesystem", &config, 3, data_dir),
            None
        );
    }
}

#[cfg(all(test, feature = "ssr"))]
mod walk_markdown_files_tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    /// A fresh, empty scratch directory — unique per call so parallel test
    /// threads never collide. No `tempfile` dependency: this repo avoids
    /// adding crate dependencies without sign-off, and cleanup here is just
    /// a manual `remove_dir_all` at the end of each test.
    fn scratch_dir() -> std::path::PathBuf {
        static COUNTER: AtomicU64 = AtomicU64::new(0);
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "tidl-walk-markdown-test-{}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[tokio::test]
    async fn finds_markdown_files_recursively_and_skips_non_markdown() {
        let base = scratch_dir();
        std::fs::write(base.join("root.md"), "# root").unwrap();
        std::fs::write(base.join("notes.txt"), "not markdown").unwrap();
        std::fs::create_dir_all(base.join("sub")).unwrap();
        std::fs::write(base.join("sub").join("child.markdown"), "# child").unwrap();

        let mut out = Vec::new();
        walk_markdown_files(&base, &base, &mut out).await;
        out.sort();

        assert_eq!(
            out,
            vec!["root.md".to_string(), "sub/child.markdown".to_string()]
        );

        std::fs::remove_dir_all(&base).ok();
    }

    #[tokio::test]
    async fn skips_dot_git_directory() {
        let base = scratch_dir();
        std::fs::create_dir_all(base.join(".git")).unwrap();
        std::fs::write(base.join(".git").join("hidden.md"), "nope").unwrap();
        std::fs::write(base.join("visible.md"), "# visible").unwrap();

        let mut out = Vec::new();
        walk_markdown_files(&base, &base, &mut out).await;

        assert_eq!(out, vec!["visible.md".to_string()]);

        std::fs::remove_dir_all(&base).ok();
    }

    #[tokio::test]
    async fn missing_directory_returns_empty_without_panicking() {
        let base = std::env::temp_dir().join("tidl-walk-markdown-test-does-not-exist");
        let mut out = Vec::new();
        walk_markdown_files(&base, &base, &mut out).await;
        assert!(out.is_empty());
    }
}
