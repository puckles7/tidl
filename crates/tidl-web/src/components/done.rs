//! Done view (backlog #124, #125) — the central list of completed items across
//! markdown sources, notes and calendars. Laid out like Focus: the same filter
//! sidebar, the same card tile, the same content-type pill row, with the search
//! box right-aligned. Cards show the completion date where Focus shows urgency;
//! the checkbox is checked (un-ticking re-opens) and archived items render
//! greyed with a disabled checkbox.
//!
//! Design: `docs/superpowers/specs/2026-10-08-done-item-lifecycle-design.md`.

use leptos::prelude::*;
use std::collections::HashSet;

use crate::api::{
    list_done_items, purge_done_ids, purge_done_items, reopen_done_item, DoneFilter, DoneItem,
    Retention,
};
use crate::app::AllCalendarsCtx;
use crate::components::filter_context::{HiddenMdSourcesCtx, HiddenNoteCollectionsCtx};
use crate::components::focus::{
    note_token_locator, source_pill_style, AdHocEdit, NoteGotoModal, NoteTarget, PreviewOverlay,
    PreviewState, TodoKind, TodoModalState,
};
use crate::components::focus_sidebar::FocusSidebar;
use crate::types::MarkdownUid;
use crate::util::mouse_event_targets_self;

#[derive(Clone, Copy, PartialEq, Eq)]
enum OriginFilter {
    All,
    Markdown,
    Note,
    Caldav,
}

impl OriginFilter {
    fn as_str(self) -> Option<&'static str> {
        match self {
            OriginFilter::All => None,
            OriginFilter::Markdown => Some("markdown"),
            OriginFilter::Note => Some("note"),
            OriginFilter::Caldav => Some("caldav"),
        }
    }
    fn label(self) -> &'static str {
        match self {
            OriginFilter::All => "All",
            OriginFilter::Markdown => "Markdown",
            OriginFilter::Note => "Notes",
            OriginFilter::Caldav => "Calendars",
        }
    }
}

/// Live / Archived pill — replaces the old "Show archived" checkbox so it sits
/// with the other pill row.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ViewMode {
    All,
    Archived,
}

#[component]
pub fn DoneView() -> impl IntoView {
    let refresh = RwSignal::new(0u32);
    let search = RwSignal::new(String::new());
    let origin = RwSignal::new(OriginFilter::All);
    let view = RwSignal::new(ViewMode::All);
    let confirm_purge = RwSignal::new(false);
    let done_refresh = crate::app::use_done_refresh();
    // Single-item archiving confirm: the target id and its source label.
    let confirm_archive: RwSignal<Option<(i64, String)>> = RwSignal::new(None);
    // Live count, set inside the Transition so the toolbar's bulk action can
    // show it (the toolbar renders outside the Transition).
    let n_live_sig = RwSignal::new(0usize);
    // Go-to-source plumbing, mirroring Focus.
    let preview: RwSignal<Option<PreviewState>> = RwSignal::new(None);
    let note_target: RwSignal<Option<NoteTarget>> = RwSignal::new(None);
    let todo_modal = crate::app::use_todo_modal();

    // Sidebar visibility (shared with Focus) filters done items by source.
    let hidden_md = use_context::<HiddenMdSourcesCtx>()
        .unwrap_or_else(|| HiddenMdSourcesCtx(RwSignal::new(HashSet::new())))
        .0;
    let hidden_notes = use_context::<HiddenNoteCollectionsCtx>()
        .unwrap_or_else(|| HiddenNoteCollectionsCtx(RwSignal::new(HashSet::new())))
        .0;
    let all_calendars = use_context::<AllCalendarsCtx>().unwrap_or_else(|| RwSignal::new(vec![]));

    let filter = move || DoneFilter {
        origin: origin.get().as_str().map(|s| s.to_string()),
        scope_id: None,
        since: None,
        until: None,
        query: {
            let q = search.get().trim().to_lowercase();
            if q.is_empty() {
                None
            } else {
                Some(q)
            }
        },
        include_archived: true,
    };

    let items_res = LocalResource::new(move || {
        let _ = refresh.get();
        // Also track the shared trigger so a clear/purge from another component
        // (e.g. Settings → Done's "Clear archived items now") refreshes this list.
        let _ = done_refresh.get();
        let f = filter();
        async move { list_done_items(f).await.unwrap_or_default() }
    });

    let do_purge = move || {
        leptos::task::spawn_local(async move {
            let _ = purge_done_items(filter(), Retention::None).await;
            confirm_purge.set(false);
            refresh.update(|n| *n += 1);
            done_refresh.update(|n| *n += 1);
        });
    };
    let reopen = move |id: i64| {
        leptos::task::spawn_local(async move {
            let _ = reopen_done_item(id).await;
            refresh.update(|n| *n += 1);
            done_refresh.update(|n| *n += 1);
        });
    };

    view! {
        <div class="done-view">
            <div class="sidebar-layout">
                <FocusSidebar/>
                <div class="filter-main">
                    <div class="screen-toolbar">
                        <div class="focus-add-btns">
                            {move || (n_live_sig.get() > 0).then(|| view! {
                                <button
                                    class="tasks-add-btn"
                                    on:click=move |_| confirm_purge.set(true)
                                >
                                    {move || format!("Clean up all matching ({})", n_live_sig.get())}
                                </button>
                            })}
                        </div>
                        <div class="focus-right-controls">
                            <div class="focus-horizon-ctrl focus-filter-ctrl">
                                {[
                                    OriginFilter::All,
                                    OriginFilter::Markdown,
                                    OriginFilter::Note,
                                    OriginFilter::Caldav,
                                ]
                                .into_iter()
                                .map(|value| view! {
                                    <button
                                        class=move || if origin.get() == value {
                                            "horizon-btn horizon-btn--active"
                                        } else {
                                            "horizon-btn"
                                        }
                                        on:click=move |_| origin.set(value)
                                    >
                                        {value.label()}
                                    </button>
                                })
                                .collect_view()}
                            </div>
                            <div class="focus-horizon-ctrl">
                                {[(ViewMode::All, "All"), (ViewMode::Archived, "Archived")]
                                    .into_iter()
                                    .map(|(value, label)| view! {
                                        <button
                                            class=move || if view.get() == value {
                                                "horizon-btn horizon-btn--active"
                                            } else {
                                                "horizon-btn"
                                            }
                                            on:click=move |_| view.set(value)
                                        >
                                            {label}
                                        </button>
                                    })
                                    .collect_view()}
                            </div>
                            <input
                                class="form-input notes-search"
                                type="text"
                                placeholder="Search done items…"
                                prop:value=move || search.get()
                                on:input=move |ev| search.set(event_target_value(&ev))
                            />
                        </div>
                    </div>

                    <Transition fallback=|| view! { <p class="focus-loading">"Loading…"</p> }>
                        {move || {
                            let _ = refresh.get();
                            let items = items_res.get().unwrap_or_default();
                            let mode = view.get();
                            let hidden_md = hidden_md.get();
                            let hidden_notes = hidden_notes.get();
                            let hidden_cals: HashSet<String> = all_calendars
                                .get()
                                .into_iter()
                                .filter(|(_, visible)| !*visible)
                                .map(|(e, _)| e.calendar_name)
                                .collect();

                            let mut rows: Vec<DoneItem> = items
                                .into_iter()
                                .filter(|item| {
                                    let sidebar_hidden = match item.origin.as_str() {
                                        "markdown" => hidden_md.contains(&item.scope_id),
                                        "note" => hidden_notes.contains(&item.scope_id),
                                        _ => hidden_cals.contains(&item.scope_id),
                                    };
                                    if sidebar_hidden {
                                        return false;
                                    }
                                    match mode {
                                        ViewMode::All => true,
                                        ViewMode::Archived => item.archived_at.is_some(),
                                    }
                                })
                                .collect();

                            // Live first, then archived; each newest-first.
                            rows.sort_by(|a, b| {
                                a.archived_at
                                    .is_some()
                                    .cmp(&b.archived_at.is_some())
                                    .then(b.completed_at.cmp(&a.completed_at))
                            });
                            let n_live = rows.iter().filter(|i| i.archived_at.is_none()).count();
                            n_live_sig.set(n_live);

                            view! {
                                {if rows.is_empty() {
                                    view! { <p class="focus-empty">"No done items."</p> }.into_any()
                                } else {
                                    view! {
                                        <ul class="focus-cards">
                                            {rows.into_iter().map(|item| {
                                                let archived = item.archived_at.is_some();
                                                let id = item.id;
                                                let color = item.color.clone();
                                                let stripe = format!("background:{};", color);
                                                let dot = format!("background:{};", color);
                                                let pill_style = source_pill_style(&color);
                                                let label = source_label(&item);
                                                let li_class = if archived {
                                                    "card-tile focus-scored-card done-card--archived"
                                                } else {
                                                    "card-tile focus-scored-card"
                                                };
                                                let goto_title = if item.origin == "caldav" {
                                                    "Edit or delete"
                                                } else {
                                                    "Go to source"
                                                };
                                                // Same "go to source" as Focus: markdown opens
                                                // the preview at the tag, notes open the note,
                                                // an ad-hoc CalDAV item opens its edit dialog.
                                                let on_goto = {
                                                    let item = item.clone();
                                                    move |_| match item.origin.as_str() {
                                                        "markdown" => {
                                                            if let Some(u) =
                                                                MarkdownUid::parse(&item.origin_key)
                                                            {
                                                                preview.set(Some(PreviewState::Markdown {
                                                                    uid: item.origin_key.clone(),
                                                                    summary: item.title.clone(),
                                                                    source_file: item
                                                                        .source_file
                                                                        .clone()
                                                                        .unwrap_or_default(),
                                                                    source_line: u.line_u32(),
                                                                    source_ordinal: u.ordinal_u32(),
                                                                }));
                                                            }
                                                        }
                                                        "note" => {
                                                            if let Some(nid) = item.note_id {
                                                                note_target.set(Some(NoteTarget {
                                                                    note_id: nid,
                                                                    token: note_token_locator(
                                                                        &item.origin_key,
                                                                    ),
                                                                }));
                                                            }
                                                        }
                                                        _ => {
                                                            todo_modal.set(Some(TodoModalState::Edit(
                                                                AdHocEdit {
                                                                    uid: item.origin_key.clone(),
                                                                    kind: if item.kind == "await" {
                                                                        TodoKind::Await
                                                                    } else {
                                                                        TodoKind::Task
                                                                    },
                                                                    summary: item.title.clone(),
                                                                    priority: item.priority.unwrap_or(50),
                                                                    date: item
                                                                        .due_date
                                                                        .clone()
                                                                        .unwrap_or_default(),
                                                                    calendar: item
                                                                        .calendar
                                                                        .clone()
                                                                        .unwrap_or_default(),
                                                                    completed: true,
                                                                },
                                                            )));
                                                        }
                                                    }
                                                };
                                                view! {
                                                    <li class=li_class>
                                                        <div class="card-tile-stripe" style=stripe></div>
                                                        <div class="fsc-rank">"·"</div>
                                                        <input
                                                            type="checkbox"
                                                            class="fsc-check"
                                                            prop:checked=!archived
                                                            disabled=archived
                                                            on:change=move |_| reopen(id)
                                                        />
                                                        <div class="fsc-body">
                                                            <div class="fsc-title">{item.title.clone()}</div>
                                                            <div class="fsc-meta">
                                                                <span
                                                                    class="pill fsc-source-pill"
                                                                    style=pill_style
                                                                >
                                                                    <span class="pill-dot" style=dot></span>
                                                                    {label.clone()}
                                                                </span>
                                                            </div>
                                                        </div>
                                                        <div class="fsc-urgency">
                                                            <span
                                                                class="fsc-urgency-label done-completed-date"
                                                            >
                                                                {item.completed_at.clone()}
                                                            </span>
                                                        </div>
                                                        <div class="fsc-actions">
                                                            <button
                                                                class="fsc-action-btn"
                                                                title=goto_title
                                                                on:click=on_goto
                                                            >
                                                                "↗"
                                                            </button>
                                                            {(!archived).then(|| {
                                                                let label = label.clone();
                                                                view! {
                                                                    <button
                                                                        class="fsc-action-btn fsc-action-btn--danger"
                                                                        title="Archive"
                                                                        on:click=move |_| confirm_archive
                                                                            .set(Some((id, label.clone())))
                                                                    >
                                                                        "✕"
                                                                    </button>
                                                                }
                                                            })}
                                                        </div>
                                                    </li>
                                                }
                                            }).collect_view()}
                                        </ul>
                                    }.into_any()
                                }}
                            }
                        }}
                    </Transition>
                </div>
            </div>

            {move || preview.get().map(|ps| view! {
                <PreviewOverlay state=ps on_close=move || preview.set(None)/>
            })}
            {move || note_target.get().map(|t| view! {
                <NoteGotoModal
                    note_id=t.note_id
                    token=t.token
                    on_close=move || note_target.set(None)
                    on_refresh=Callback::new(move |_| refresh.update(|n| *n += 1))
                />
            })}

            {move || confirm_purge.get().then(|| view! {
                <ConfirmModal
                    message="Archive the listed done items? They will be removed from their source (note, markdown file, calendar). The items will be kept only as read-only history.".to_string()
                    on_confirm=Callback::new(move |_: ()| do_purge())
                    on_cancel=Callback::new(move |_: ()| confirm_purge.set(false))
                />
            })}

            {move || confirm_archive.get().map(|(id, label)| view! {
                <ConfirmModal
                    message=format!(
                        "Archive this done item? Its source in “{label}” is deleted; the item is kept as read-only history.",
                    )
                    on_confirm=Callback::new(move |_: ()| {
                        leptos::task::spawn_local(async move {
                            let _ = purge_done_ids(vec![id]).await;
                            confirm_archive.set(None);
                            refresh.update(|n| *n += 1);
                            done_refresh.update(|n| *n += 1);
                        });
                    })
                    on_cancel=Callback::new(move |_: ()| confirm_archive.set(None))
                />
            })}
        </div>
    }
}

/// `"{source} · {file}"` for markdown items, else the source label.
fn source_label(item: &DoneItem) -> String {
    if item.origin == "markdown" {
        if let Some(file) = &item.source_file {
            let base = file.rsplit('/').next().unwrap_or(file);
            return format!("{} · {}", item.source_label, base);
        }
    }
    item.source_label.clone()
}

#[component]
fn ConfirmModal(
    message: String,
    on_confirm: Callback<()>,
    on_cancel: Callback<()>,
) -> impl IntoView {
    let esc_id = crate::modal_esc::use_escape_close(move || on_cancel.run(()));
    let backdrop_mousedown = RwSignal::new(false);
    view! {
        <div
            class="dialog-backdrop"
            data-esc-id=esc_id.to_string()
            on:mousedown=move |ev: leptos::web_sys::MouseEvent| {
                backdrop_mousedown.set(mouse_event_targets_self(&ev))
            }
            on:click=move |_| if backdrop_mousedown.get() { on_cancel.run(()) }
        >
            <div class="dialog dialog-small" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                <div class="dialog-header">
                    <h2 class="dialog-title">"Archive done items?"</h2>
                </div>
                <div class="dialog-form">
                    <p class="dialog-hint">{message}</p>
                </div>
                <div class="dialog-actions">
                    <button class="btn-danger" on:click=move |_| on_confirm.run(())>"Archive"</button>
                    <button class="btn-secondary" on:click=move |_| on_cancel.run(())>"Cancel"</button>
                </div>
            </div>
        </div>
    }
}
