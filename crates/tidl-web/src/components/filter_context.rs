//! Per-viewer visibility filters shared across screens (backlog #114).
//!
//! These signals are provided once in `AuthenticatedApp` (see `app.rs`) and
//! outlive any single route's mount, so hiding a markdown source, note
//! collection, or note label from one screen's sidebar is remembered while the
//! user switches tabs and back. Each screen renders its own sidebar from them:
//! Focus's [`crate::components::focus_sidebar::FocusSidebar`] and the Notes
//! tab's own collection/label sidebar both read these.
//!
//! Newtypes, not bare `RwSignal<HashSet<String>>` aliases — see
//! `CurrentUsernameCtx`'s doc comment in `app.rs` for why. A bare alias isn't a
//! distinct type, so `HiddenMdSourcesCtx`/`HiddenNoteCollectionsCtx` (identical
//! aliases, provided in the same `AuthenticatedApp` scope) collided and the
//! later `provide_context` silently overwrote the earlier one for every
//! descendant — hiding a markdown source and hiding a note collection were
//! secretly toggling the very same hidden-ID set.

use leptos::prelude::*;
use std::collections::HashSet;

/// Markdown source IDs (as strings) hidden by the user. Empty = show all.
#[derive(Clone, Copy)]
pub struct HiddenMdSourcesCtx(pub RwSignal<HashSet<String>>);

/// Note collection IDs (as strings) hidden by the user. Empty = show all.
#[derive(Clone, Copy)]
pub struct HiddenNoteCollectionsCtx(pub RwSignal<HashSet<String>>);

/// Note label IDs (as strings) hidden by the user. Empty = show all.
#[derive(Clone, Copy)]
pub struct HiddenNoteLabelsCtx(pub RwSignal<HashSet<String>>);
