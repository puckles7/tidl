//! Shared outlined SVG icons for toolbar/action buttons (backlog #95) —
//! replaces the previous mix of Unicode glyphs/emoji (`+`, `⊞`, `★`, `🔗`,
//! `🗑`) with one consistent stroke style (`stroke-width="2"`, round
//! caps/joins), matching the `~/Downloads/tidl-notes.html` mockup. Merge has
//! no mockup reference, so it borrows Feather/Lucide's `git-merge` glyph to
//! stay in the same icon family.

use leptos::prelude::*;

#[component]
pub fn PlusIcon(#[prop(default = 14)] size: u32) -> impl IntoView {
    let size = size.to_string();
    view! {
        <svg width=size.clone() height=size viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round" stroke-linejoin="round">
            <line x1="12" y1="5" x2="12" y2="19"></line>
            <line x1="5" y1="12" x2="19" y2="12"></line>
        </svg>
    }
}

#[component]
pub fn GridIcon(#[prop(default = 14)] size: u32) -> impl IntoView {
    let size = size.to_string();
    view! {
        <svg width=size.clone() height=size viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="3" width="7" height="7"></rect>
            <rect x="14" y="3" width="7" height="7"></rect>
            <rect x="14" y="14" width="7" height="7"></rect>
            <rect x="3" y="14" width="7" height="7"></rect>
        </svg>
    }
}

/// The one shared trash icon used by every delete/clear-all affordance in
/// the app (toolbar "Clear all" buttons and the per-card `DeleteTriggerButton`).
#[component]
pub fn TrashIcon(#[prop(default = 14)] size: u32) -> impl IntoView {
    let size = size.to_string();
    view! {
        <svg width=size.clone() height=size viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polyline points="3 6 5 6 21 6"></polyline>
            <path d="M19 6l-1 14a2 2 0 0 1-2 2H8a2 2 0 0 1-2-2L5 6"></path>
            <path d="M10 11v6"></path>
            <path d="M14 11v6"></path>
            <path d="M8 6V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"></path>
        </svg>
    }
}

#[component]
pub fn StarIcon(#[prop(default = 14)] size: u32) -> impl IntoView {
    let size = size.to_string();
    view! {
        <svg width=size.clone() height=size viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2"></polygon>
        </svg>
    }
}

#[component]
pub fn SearchIcon(#[prop(default = 14)] size: u32) -> impl IntoView {
    let size = size.to_string();
    view! {
        <svg width=size.clone() height=size viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="11" cy="11" r="8"></circle>
            <line x1="21" y1="21" x2="16.65" y2="16.65"></line>
        </svg>
    }
}

#[component]
pub fn MergeIcon(#[prop(default = 14)] size: u32) -> impl IntoView {
    let size = size.to_string();
    view! {
        <svg width=size.clone() height=size viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="18" cy="18" r="3"></circle>
            <circle cx="6" cy="6" r="3"></circle>
            <path d="M6 21V9a9 9 0 0 0 9 9"></path>
        </svg>
    }
}
