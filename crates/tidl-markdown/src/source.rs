//! Markdown source abstraction
//!
//! This module defines the trait that all markdown sources must implement,
//! allowing for pluggable backends (filesystem, Dropbox, OneDrive, etc.)

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use crate::error::MarkdownError;

type Result<T> = std::result::Result<T, MarkdownError>;

/// A markdown file from a source
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkdownFile {
    /// Relative path within the source
    pub path: String,
    /// File content
    pub content: String,
    /// Last modified time (if available)
    pub modified: Option<chrono::DateTime<chrono::Utc>>,
    /// Content hash for change detection
    pub hash: String,
}

/// Callback for file changes
pub type SourceCallback = Box<dyn Fn(MarkdownFile) -> Result<()> + Send + Sync>;

/// Trait that all markdown sources must implement
#[async_trait]
pub trait MarkdownSource: Send + Sync {
    /// List all markdown files in the source
    async fn list_files(&self) -> Result<Vec<MarkdownFile>>;

    /// Read a specific file by path
    async fn read_file(&self, path: &str) -> Result<MarkdownFile>;

    /// Watch for changes (optional, return Ok if not supported)
    async fn watch(&self, callback: SourceCallback) -> Result<()>;

    /// Get the source name/identifier
    fn name(&self) -> &str;

    /// Get the source type
    fn source_type(&self) -> &str;
}

/// Factory for creating markdown sources from configuration
pub fn create_source(
    source_type: &str,
    config: serde_json::Value,
) -> Result<Box<dyn MarkdownSource>> {
    match source_type {
        "filesystem" => {
            let fs_source: crate::sources::filesystem::FilesystemSource =
                serde_json::from_value(config)?;
            Ok(Box::new(fs_source))
        }
        _ => Err(MarkdownError::UnknownSourceType(source_type.to_string())),
    }
}
