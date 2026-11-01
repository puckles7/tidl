//! tidl-markdown - Markdown integration for calendar and task management
//!
//! This crate provides pluggable markdown source support with tag parsing
//! for todo and pending items.

pub mod error;
pub mod parser;
pub mod source;
pub mod sources;
pub mod watcher;

pub use error::MarkdownError;
pub use parser::*;
pub use source::*;
pub use watcher::*;
