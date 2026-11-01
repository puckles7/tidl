//! Focus view sidebar — left panel for the Focus screen.
//!
//! This was once a "shared filter sidebar" for Focus/Tasks/Await/Calendar, but
//! Tasks and Await were merged into Focus (backlog #115) and Calendar has its
//! own bespoke sidebar (`calendar/mod.rs`). Only Focus renders this component
//! now, so it — like the Calendar and Notes sidebars — is screen-specific. The
//! `.fs-*` classes and `CollapsiblePanel` are the shared pieces; which boxes a
//! screen shows is its own decision.
//!
//! The Calendars section is the shared `CalendarListPanel` component (also
//! used directly by the Calendar page) — see its doc comment for why that's
//! a separate component with its own context (`AllCalendarsCtx`) rather than
//! following the pattern below.
//!
//! Markdown sources, Note collections, and Note labels each expose their own
//! context type (defined in [`crate::components::filter_context`]) so sibling
//! views can filter their content:
//! - [`HiddenMdSourcesCtx`]        — markdown source IDs (as strings) toggled off
//! - [`HiddenNoteCollectionsCtx`]  — note collection IDs (as strings) toggled off
//! - [`HiddenNoteLabelsCtx`]       — note label IDs (as strings) toggled off
//!
//! On desktop the sidebar is a fixed-width left panel.
//! On mobile each section collapses independently behind its own toggle button.
//!
//! All sections include full management (create / edit / delete) via modal
//! dialogs.

use leptos::prelude::*;
use std::collections::HashSet;

use crate::api::{list_markdown_sources, trigger_markdown_sync, MarkdownSourceInfo};
use crate::app::{use_sidebar_refresh, ShowMarkdownFinderCtx};
use crate::components::cal_edit_modal::MdSourceEditModal;
use crate::components::calendar_list_panel::CalendarListPanel;
use crate::components::collapsible_panel::CollapsiblePanel;
use crate::components::filter_context::HiddenMdSourcesCtx;
use crate::components::icons::SearchIcon;
use crate::components::note_collections_panel::NoteCollectionsPanel;
use crate::components::sidebar_pill::SidebarPill;

/// Focus view sidebar listing calendars, markdown sources, and the shared
/// note-collections/labels panel with visibility toggles and full
/// create/edit/delete management.
///
/// Must be rendered inside a `<div class="sidebar-layout">` alongside a
/// `<div class="filter-main">` that holds the page content.
#[component]
pub fn FocusSidebar() -> impl IntoView {
    let hidden_md = use_context::<HiddenMdSourcesCtx>()
        .expect("HiddenMdSourcesCtx must be provided by AuthenticatedApp")
        .0;
    let show_file_finder = use_context::<ShowMarkdownFinderCtx>()
        .expect("ShowMarkdownFinderCtx must be provided by AuthenticatedApp")
        .0;
    let sidebar_refresh = use_sidebar_refresh();

    // Mobile: each section independently collapsible.
    let cals_open = RwSignal::new(false);
    let md_open = RwSignal::new(false);
    let note_open = RwSignal::new(false);

    // LocalResource (not Resource) so the sidebar does not participate in SSR
    // serialisation. Both SSR and the initial WASM pass render the empty
    // fallback, so tachys sees identical DOM node counts on both sides and
    // hydration succeeds. Data loads client-side after hydration completes.
    let sources_res = LocalResource::new(move || {
        let _ = sidebar_refresh.get(); // markdown source ops go through the on_refresh callback
        async move { list_markdown_sources().await.unwrap_or_default() }
    });

    // ── Markdown source edit modal state ──────────────────────────────────────
    let editing_src: RwSignal<Option<Option<MarkdownSourceInfo>>> = RwSignal::new(None);

    // Markdown source IDs currently mid-sync — a set (not a single `Option`,
    // same reasoning as `syncing_cals` in `calendar_list_panel.rs`) so
    // syncing one source's pill doesn't clear the spinner on another
    // mid-flight.
    let syncing_md: RwSignal<HashSet<i64>> = RwSignal::new(HashSet::new());

    // on_refresh callback triggers a calendars refresh (calendar, source, and
    // note-collection ops all share this one signal).
    let on_refresh = Callback::new(move |_| sidebar_refresh.update(|n| *n += 1));

    // The shared panel refetches its collections/labels off this.
    let collections_refresh: Signal<u32> = Signal::derive(move || sidebar_refresh.get());

    view! {
        <aside class="filter-sidebar">
            // ── Calendars panel ────────────────────────────────────────────────
            <CollapsiblePanel title="Calendars" open=cals_open>
                <div class="sidebar-section">
                    <CalendarListPanel on_refresh=on_refresh />
                </div>
            </CollapsiblePanel>

            // ── Markdown sources panel ─────────────────────────────────────────
            <CollapsiblePanel title="Markdown Sources" open=md_open>
                <Transition>
                    {move || {
                        let hidden = hidden_md.get();
                        Suspend::new(async move {
                            let sources = sources_res.await;
                            view! {
                                <div class="fs-section">
                                    <div class="fs-section-header">
                                        <span class="fs-section-title">"Markdown"</span>
                                        <div class="fs-header-actions">
                                            <button
                                                class="fs-search-btn"
                                                title="Find markdown file (Ctrl/Cmd+K)"
                                                on:click=move |_| show_file_finder.set(true)
                                            ><SearchIcon size=13/></button>
                                            <button
                                                class="fs-add-btn"
                                                title="Add markdown source"
                                                on:click=move |_| editing_src.set(Some(None))
                                            >"+"</button>
                                        </div>
                                    </div>
                                    {move || crate::components::settings::show_sidebar_hints().then(|| view! {
                                        <p class="fs-hint">"Click a source to show/hide its items."</p>
                                    })}
                                    {if sources.is_empty() {
                                        view! { <p class="fs-empty">"No markdown sources yet."</p> }.into_any()
                                    } else {
                                        view! {
                                            <ul class="fs-list">
                                                {sources.into_iter().map(|src| {
                                                    let key      = src.id.to_string();
                                                    let n        = key.clone();
                                                    let src_id   = src.id;
                                                    let visible  = !hidden.contains(&key);
                                                    let src_edit = src.clone();
                                                    let syncing: Signal<bool> = Signal::derive(move || syncing_md.get().contains(&src_id));
                                                    let on_sync: Option<Callback<()>> = Some(Callback::new(move |_| {
                                                        syncing_md.update(|s| { s.insert(src_id); });
                                                        leptos::task::spawn_local(async move {
                                                            let _ = trigger_markdown_sync(src_id).await;
                                                            syncing_md.update(|s| { s.remove(&src_id); });
                                                        });
                                                    }));
                                                    view! {
                                                        <li class="fs-list-item">
                                                            <SidebarPill
                                                                display_name=src.name.clone()
                                                                color=src.color.clone()
                                                                visible
                                                                on_toggle=Callback::new(move |_| {
                                                                    hidden_md.update(|h| {
                                                                        if h.contains(&n) { h.remove(&n); }
                                                                        else { h.insert(n.clone()); }
                                                                    });
                                                                })
                                                                on_edit=Some(Callback::new(move |_| {
                                                                    editing_src.set(Some(Some(src_edit.clone())));
                                                                }))
                                                                on_sync=on_sync
                                                                syncing=syncing
                                                            />
                                                        </li>
                                                    }
                                                }).collect_view()}
                                            </ul>
                                        }.into_any()
                                    }}
                                </div>
                            }.into_any()
                        })
                    }}
                </Transition>
            </CollapsiblePanel>

            // ── Note collections + labels panel ────────────────────────────────
            // Both live in one box (the shared `NoteCollectionsPanel`), matching
            // the Notes tab; previously they were two separate panels here.
            <CollapsiblePanel title="Note collections" open=note_open>
                <NoteCollectionsPanel refresh=collections_refresh on_refresh=on_refresh />
            </CollapsiblePanel>
        </aside>

        // ── Markdown source edit / create modal ───────────────────────────────
        {move || editing_src.get().map(|src_opt| {
            view! {
                <MdSourceEditModal
                    src=src_opt
                    on_close=move || editing_src.set(None)
                    on_refresh=on_refresh
                />
            }
        })}
    }
}
