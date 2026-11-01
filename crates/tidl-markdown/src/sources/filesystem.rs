//! Filesystem-based markdown source

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

use crate::error::MarkdownError;
use crate::source::{MarkdownFile, MarkdownSource, SourceCallback};

type Result<T> = std::result::Result<T, MarkdownError>;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesystemSource {
    pub name: String,
    pub path: PathBuf,
    pub recursive: bool,
}

impl FilesystemSource {
    pub fn new(name: String, path: PathBuf, recursive: bool) -> Self {
        Self {
            name,
            path,
            recursive,
        }
    }

    fn is_markdown_file(path: &Path) -> bool {
        path.extension()
            .and_then(|e| e.to_str())
            .map(|e| e == "md" || e == "markdown")
            .unwrap_or(false)
    }

    fn compute_hash(content: &str) -> String {
        let mut hasher = Sha256::new();
        hasher.update(content.as_bytes());
        hex::encode(hasher.finalize())
    }

    async fn read_markdown_file(&self, path: &Path) -> Result<MarkdownFile> {
        let content = tokio::fs::read_to_string(path).await?;
        let metadata = tokio::fs::metadata(path).await?;

        let modified = metadata.modified().ok().and_then(|t| {
            chrono::DateTime::from_timestamp(
                t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64,
                0,
            )
        });

        let relative_path = path
            .strip_prefix(&self.path)
            .unwrap_or(path)
            .to_string_lossy()
            .to_string();

        Ok(MarkdownFile {
            path: relative_path,
            content: content.clone(),
            modified,
            hash: Self::compute_hash(&content),
        })
    }
}

#[async_trait]
impl MarkdownSource for FilesystemSource {
    async fn list_files(&self) -> Result<Vec<MarkdownFile>> {
        let mut files = Vec::new();

        let walker = if self.recursive {
            WalkDir::new(&self.path)
        } else {
            WalkDir::new(&self.path).max_depth(1)
        };

        for entry in walker.into_iter().filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_file() && Self::is_markdown_file(path) {
                match self.read_markdown_file(path).await {
                    Ok(file) => files.push(file),
                    Err(e) => {
                        tracing::warn!("Failed to read {}: {}", path.display(), e);
                    }
                }
            }
        }

        Ok(files)
    }

    async fn read_file(&self, relative_path: &str) -> Result<MarkdownFile> {
        let full_path = self.path.join(relative_path);
        self.read_markdown_file(&full_path).await
    }

    async fn watch(&self, callback: SourceCallback) -> Result<()> {
        use notify::{Event, RecursiveMode, Watcher};
        use std::sync::mpsc::channel;

        let (tx, rx) = channel();
        let mut watcher = notify::recommended_watcher(tx)?;

        let mode = if self.recursive {
            RecursiveMode::Recursive
        } else {
            RecursiveMode::NonRecursive
        };

        watcher.watch(&self.path, mode)?;

        // Spawn a task to handle file system events
        let source = self.clone();
        tokio::spawn(async move {
            while let Ok(event) = rx.recv() {
                if let Ok(Event { paths, .. }) = event {
                    for path in paths {
                        if path.is_file() && Self::is_markdown_file(&path) {
                            if let Ok(file) = source.read_markdown_file(&path).await {
                                if let Err(e) = callback(file) {
                                    tracing::error!("Callback error: {}", e);
                                }
                            }
                        }
                    }
                }
            }
        });

        Ok(())
    }

    fn name(&self) -> &str {
        &self.name
    }

    fn source_type(&self) -> &str {
        "filesystem"
    }
}
