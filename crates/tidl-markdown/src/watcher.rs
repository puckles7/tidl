//! File watcher for monitoring markdown sources

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use crate::error::MarkdownError;
use crate::parser::{AwaitItem, MarkdownParser, TaskItem};
use crate::source::{MarkdownFile, MarkdownSource};

type Result<T> = std::result::Result<T, MarkdownError>;

/// Watcher state
pub struct WatcherState {
    /// File hash tracking for change detection
    pub file_hashes: HashMap<String, String>,
    /// Parsed todos
    pub todos: Vec<(String, TaskItem)>,
    /// Parsed pending items
    pub pending: Vec<(String, AwaitItem)>,
}

impl WatcherState {
    pub fn new() -> Self {
        Self {
            file_hashes: HashMap::new(),
            todos: Vec::new(),
            pending: Vec::new(),
        }
    }
}

impl Default for WatcherState {
    fn default() -> Self {
        Self::new()
    }
}

/// Markdown file watcher
pub struct MarkdownWatcher {
    sources: Vec<Box<dyn MarkdownSource>>,
    parser: MarkdownParser,
    state: Arc<RwLock<WatcherState>>,
}

impl MarkdownWatcher {
    pub fn new(sources: Vec<Box<dyn MarkdownSource>>) -> Self {
        Self {
            sources,
            parser: MarkdownParser::new(),
            state: Arc::new(RwLock::new(WatcherState::new())),
        }
    }

    /// Perform initial scan of all sources
    pub async fn initial_scan(&self) -> Result<()> {
        for source in &self.sources {
            let files = source.list_files().await?;
            for file in files {
                self.process_file(file, source.name()).await?;
            }
        }
        Ok(())
    }

    /// Process a single markdown file
    async fn process_file(&self, file: MarkdownFile, source_name: &str) -> Result<()> {
        let mut state = self.state.write().await;

        // Check if file has changed
        if let Some(old_hash) = state.file_hashes.get(&file.path) {
            if old_hash == &file.hash {
                return Ok(()); // No changes
            }
        }

        // Update hash
        state
            .file_hashes
            .insert(file.path.clone(), file.hash.clone());

        // Remove old entries for this file
        state.todos.retain(|(path, _)| path != &file.path);
        state.pending.retain(|(path, _)| path != &file.path);

        // Parse new items
        let (todos, pending) = self.parser.parse_all(&file.content);

        // Add new items
        for todo in todos {
            state.todos.push((file.path.clone(), todo));
        }

        for pend in pending {
            state.pending.push((file.path.clone(), pend));
        }

        tracing::info!(
            "Processed {} from {}: {} todos, {} pending",
            file.path,
            source_name,
            state.todos.len(),
            state.pending.len()
        );

        Ok(())
    }

    /// Get current todos
    pub async fn get_todos(&self) -> Vec<(String, TaskItem)> {
        let state = self.state.read().await;
        state.todos.clone()
    }

    /// Get current pending items
    pub async fn get_pending(&self) -> Vec<(String, AwaitItem)> {
        let state = self.state.read().await;
        state.pending.clone()
    }

    /// Start watching all sources
    pub async fn start_watching(&self) -> Result<()> {
        for source in &self.sources {
            let parser = self.parser.clone();
            let state = self.state.clone();
            let source_name = source.name().to_string();

            source
                .watch(Box::new(move |file| {
                    let parser = parser.clone();
                    let state = state.clone();
                    let source_name = source_name.clone();

                    tokio::spawn(async move {
                        let mut state = state.write().await;

                        // Update hash
                        state
                            .file_hashes
                            .insert(file.path.clone(), file.hash.clone());

                        // Remove old entries
                        state.todos.retain(|(path, _)| path != &file.path);
                        state.pending.retain(|(path, _)| path != &file.path);

                        // Parse and add new items
                        let (todos, pending) = parser.parse_all(&file.content);

                        for todo in todos {
                            state.todos.push((file.path.clone(), todo));
                        }

                        for pend in pending {
                            state.pending.push((file.path.clone(), pend));
                        }

                        tracing::info!(
                            "Updated {} from {}: {} todos, {} pending",
                            file.path,
                            source_name,
                            state.todos.len(),
                            state.pending.len()
                        );
                    });

                    Ok(())
                }))
                .await?;
        }

        Ok(())
    }
}
