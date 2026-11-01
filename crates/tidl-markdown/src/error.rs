//! Typed errors for the pluggable markdown source abstraction
//! ([`crate::source::MarkdownSource`] and its filesystem implementation).
//!
//! Replaces the `anyhow::Result` these modules used to return - library
//! crates shouldn't leak `anyhow::Error` into callers (CLAUDE.md §11).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum MarkdownError {
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
    #[error("file watch error: {0}")]
    Watch(#[from] notify::Error),
    #[error("invalid source configuration: {0}")]
    Config(#[from] serde_json::Error),
    #[error("unknown source type: {0}")]
    UnknownSourceType(String),
}
