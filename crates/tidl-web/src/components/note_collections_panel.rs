//! The "Note collections" box — the collections and labels lists with their
//! show/hide toggles, the inline "new collection" button, and the collection /
//! label edit modals.
//!
//! Shared verbatim by the Focus sidebar and the Notes tab's own sidebar
//! (backlog #114 follow-up) so the two can't drift: previously Focus rendered
//! collections and labels as two separate collapsible panels while Notes put
//! both in one card, from two copies of the same pill list. Now one component
//! renders both lists in a single `.fs-section` box; each host decides the
//! surrounding chrome (a `CollapsiblePanel` on Focus, the `.notes-sidebar`
//! card on Notes).

use leptos::prelude::*;

use crate::api::{list_note_collections, list_note_labels, NoteCollectionInfo, NoteLabelInfo};
use crate::components::filter_context::{HiddenNoteCollectionsCtx, HiddenNoteLabelsCtx};
use crate::components::notes::collection_editor_modal::CollectionEditorModal;
use crate::components::notes::NoteLabelEditorModal;
use crate::components::settings::{show_sidebar_hints, OpenSettingsCtx};
use crate::components::sidebar_pill::SidebarPill;

#[component]
pub fn NoteCollectionsPanel(
    /// Reactive trigger the panel's collection/label resources refetch on.
    /// Hosts derive it from whatever signals mean "notes/collections/labels
    /// may have changed" (e.g. Notes folds in its Keep-import refresh too).
    refresh: Signal<u32>,
    /// Called after a collection/label is saved or deleted, so the host can
    /// bump `refresh` (and refetch its own note list).
    on_refresh: Callback<()>,
) -> impl IntoView {
    let hidden_collections = use_context::<HiddenNoteCollectionsCtx>()
        .expect("HiddenNoteCollectionsCtx must be provided by AuthenticatedApp")
        .0;
    let hidden_labels = use_context::<HiddenNoteLabelsCtx>()
        .expect("HiddenNoteLabelsCtx must be provided by AuthenticatedApp")
        .0;
    let open_settings = use_context::<OpenSettingsCtx>()
        .expect("OpenSettingsCtx must be provided by AuthenticatedApp");

    let collections_res = LocalResource::new(move || {
        let _ = refresh.get();
        async move { list_note_collections().await.unwrap_or_default() }
    });
    let labels_res = LocalResource::new(move || {
        let _ = refresh.get();
        async move { list_note_labels().await.unwrap_or_default() }
    });

    // Modal state: None = closed; Some(None) = create; Some(Some(x)) = edit.
    let editing_collection: RwSignal<Option<Option<NoteCollectionInfo>>> = RwSignal::new(None);
    let editing_label: RwSignal<Option<Option<NoteLabelInfo>>> = RwSignal::new(None);

    view! {
        <div class="fs-section">
            // ── Collections ───────────────────────────────────────────────────
            <div class="fs-section-header">
                <span class="fs-section-title">"Note collections"</span>
                <button
                    class="fs-add-btn"
                    title="New note collection"
                    on:click=move |_| editing_collection.set(Some(None))
                >"+"</button>
            </div>
            {move || show_sidebar_hints().then(|| view! {
                <p class="fs-hint">"Click a collection to show/hide its items."</p>
            })}
            <Transition fallback=|| view! { <p class="dialog-hint">"Loading…"</p> }>
                {move || {
                    let hidden = hidden_collections.get();
                    Suspend::new(async move {
                        let collections = collections_res.await;
                        if collections.is_empty() {
                            return view! { <p class="fs-empty">"No note collections yet."</p> }.into_any();
                        }
                        view! {
                            <ul class="fs-list">
                                {collections.into_iter().map(|coll| {
                                    let key       = coll.id.to_string();
                                    let n         = key.clone();
                                    let visible   = !hidden.contains(&key);
                                    let coll_edit = coll.clone();
                                    // Share icon only for collections that are actually
                                    // shared — see backlog #65 (same rule as the calendar sidebar).
                                    let is_shared_either_way = !coll.is_owner || coll.shared;
                                    let share_title = if !coll.is_owner {
                                        format!("Shared by {}", coll.owner_username)
                                    } else {
                                        "Manage sharing".to_string()
                                    };
                                    let on_share: Option<Callback<()>> = is_shared_either_way.then(|| Callback::new(move |_| {
                                        open_settings.0.set(Some("sharing"));
                                    }));
                                    view! {
                                        <li class="fs-list-item">
                                            <SidebarPill
                                                display_name=coll.name.clone()
                                                color=coll.color.clone()
                                                visible=visible
                                                on_toggle=Callback::new(move |_| {
                                                    hidden_collections.update(|h| {
                                                        if h.contains(&n) { h.remove(&n); }
                                                        else { h.insert(n.clone()); }
                                                    });
                                                })
                                                on_edit=Some(Callback::new(move |_| {
                                                    editing_collection.set(Some(Some(coll_edit.clone())));
                                                }))
                                                on_share=on_share
                                                share_title=share_title
                                            />
                                        </li>
                                    }
                                }).collect_view()}
                            </ul>
                        }.into_any()
                    })
                }}
            </Transition>

            // ── Labels ────────────────────────────────────────────────────────
            <div class="fs-section-header" style="margin-top:1rem">
                <span class="fs-section-title fs-section-title--sub">"Labels"</span>
                <button
                    class="fs-add-btn"
                    title="New label"
                    on:click=move |_| editing_label.set(Some(None))
                >"+"</button>
            </div>
            {move || show_sidebar_hints().then(|| view! {
                <p class="fs-hint">"Click a label to show/hide the notes carrying it."</p>
            })}
            <Transition fallback=|| view! { <p class="dialog-hint">"Loading…"</p> }>
                {move || {
                    let hidden = hidden_labels.get();
                    Suspend::new(async move {
                        let labels = labels_res.await;
                        if labels.is_empty() {
                            return view! { <p class="fs-empty">"No labels yet."</p> }.into_any();
                        }
                        view! {
                            <ul class="fs-list">
                                {labels.into_iter().map(|label| {
                                    let key        = label.id.to_string();
                                    let n          = key.clone();
                                    let visible    = !hidden.contains(&key);
                                    let label_edit = label.clone();
                                    view! {
                                        <li class="fs-list-item">
                                            <SidebarPill
                                                display_name=label.name.clone()
                                                color=label.color.clone()
                                                visible=visible
                                                on_toggle=Callback::new(move |_| {
                                                    hidden_labels.update(|h| {
                                                        if h.contains(&n) { h.remove(&n); }
                                                        else { h.insert(n.clone()); }
                                                    });
                                                })
                                                on_edit=Some(Callback::new(move |_| {
                                                    editing_label.set(Some(Some(label_edit.clone())));
                                                }))
                                            />
                                        </li>
                                    }
                                }).collect_view()}
                            </ul>
                        }.into_any()
                    })
                }}
            </Transition>
        </div>

        {move || editing_collection.get().map(|coll_opt| view! {
            <CollectionEditorModal
                collection=coll_opt
                on_close=move || editing_collection.set(None)
                on_refresh=on_refresh
            />
        })}

        {move || editing_label.get().map(|label_opt| view! {
            <NoteLabelEditorModal
                label=label_opt
                on_close=move || editing_label.set(None)
                on_refresh=on_refresh
            />
        })}
    }
}
