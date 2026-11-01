//! "Clear all" confirmation for the Notes toolbar (backlog #90). Unlike
//! Contacts' flat list — a bare confirm via `DeleteConfirmModal` — notes
//! live in collections (some shared with other users), so clearing needs a
//! scope picker rather than a single yes/no: a dropdown defaulting to "All
//! collections", or any one collection the viewer can see. Same dialog
//! chrome as `DeleteConfirmModal`, just with a `<select>` in place of a
//! pluralized title.
//!
//! The "also delete the collection(s)" checkbox reuses `delete_note_collection`
//! (which already cascades to a collection's notes) rather than clearing
//! contents and then deleting — one call instead of two for the single-
//! collection case. For "All collections" a shared-with-the-viewer
//! collection can't be deleted by them (owner-only), so that scope still
//! runs `clear_notes(None)` first to empty everything visible, then deletes
//! every collection the viewer actually owns.

use leptos::prelude::*;

use crate::api::{clear_notes, delete_note_collection};
use crate::types::NoteCollectionInfo;
use crate::util::{format_server_fn_error, mouse_event_targets_self};

/// Runs the confirmed action against the server. A free function (not a
/// closure) so the `?`-per-call error mapping doesn't have to be repeated
/// at each call site in the button handler.
async fn run_clear(
    collection_id: Option<i64>,
    delete_collections: bool,
    owned_collection_ids: Vec<i64>,
) -> Result<(), String> {
    if !delete_collections {
        return clear_notes(collection_id)
            .await
            .map_err(|e| format_server_fn_error(&e.to_string()));
    }

    match collection_id {
        Some(id) => delete_note_collection(id)
            .await
            .map_err(|e| format_server_fn_error(&e.to_string())),
        None => {
            clear_notes(None)
                .await
                .map_err(|e| format_server_fn_error(&e.to_string()))?;
            for id in owned_collection_ids {
                delete_note_collection(id)
                    .await
                    .map_err(|e| format_server_fn_error(&e.to_string()))?;
            }
            Ok(())
        }
    }
}

#[component]
pub fn ClearNotesModal(
    armed: RwSignal<bool>,
    collections: Signal<Vec<NoteCollectionInfo>>,
    on_refresh: Callback<()>,
) -> impl IntoView {
    // "" means "All collections" — the default option's value, so a viewer
    // who opens the modal and immediately clicks "Clear" gets the
    // broadest scope rather than an arbitrary first collection.
    let selected = RwSignal::new(String::new());
    let delete_collections = RwSignal::new(false);
    let saving = RwSignal::new(false);
    let error_msg: RwSignal<Option<String>> = RwSignal::new(None);

    // "All collections" always permits the checkbox (it silently only
    // touches owned collections); a specific selection only permits it when
    // the viewer owns that collection.
    let checkbox_allowed = move || {
        let sel = selected.get();
        sel.is_empty()
            || sel
                .parse::<i64>()
                .ok()
                .and_then(|id| collections.get().into_iter().find(|c| c.id == id))
                .map(|c| c.is_owner)
                .unwrap_or(false)
    };

    // Switching to a non-owned collection silently drops a checked box
    // rather than leaving it checked-but-disabled and confusing.
    Effect::new(move |_| {
        if !checkbox_allowed() {
            delete_collections.set(false);
        }
    });

    let checkbox_label = move || {
        if selected.get().is_empty() {
            "Also delete these collections"
        } else {
            "Also delete this collection"
        }
    };

    let do_clear = move |_: leptos::ev::MouseEvent| {
        let collection_id = selected.get_untracked().parse::<i64>().ok();
        let delete = delete_collections.get_untracked();
        let owned_ids: Vec<i64> = collections
            .get_untracked()
            .into_iter()
            .filter(|c| c.is_owner)
            .map(|c| c.id)
            .collect();
        saving.set(true);
        error_msg.set(None);
        leptos::task::spawn_local(async move {
            let result = run_clear(collection_id, delete, owned_ids).await;
            saving.set(false);
            match result {
                Ok(()) => {
                    armed.set(false);
                    on_refresh.run(());
                }
                Err(msg) => error_msg.set(Some(msg)),
            }
        });
    };

    let esc_id = crate::modal_esc::use_escape_close(move || {
        if !saving.get_untracked() {
            armed.set(false);
        }
    });

    move || {
        armed.get().then(|| {
            let backdrop_mousedown = RwSignal::new(false);
            view! {
                <div class="dialog-backdrop"
                    data-esc-id=esc_id.to_string()
                    on:pointerdown=|ev: leptos::web_sys::PointerEvent| ev.stop_propagation()
                    on:mousedown=move |ev: leptos::web_sys::MouseEvent| {
                        ev.stop_propagation();
                        backdrop_mousedown.set(mouse_event_targets_self(&ev));
                    }
                    on:click=move |ev: leptos::ev::MouseEvent| {
                        ev.stop_propagation();
                        if backdrop_mousedown.get() && !saving.get_untracked() { armed.set(false); }
                    }
                >
                    <div class="dialog dialog-small" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                        <div class="dialog-header">
                            <h2 class="dialog-title">"Clear notes"</h2>
                        </div>
                        <div class="dialog-form">
                            <div class="form-group">
                                <select
                                    class="form-input"
                                    prop:value=move || selected.get()
                                    on:change=move |ev| selected.set(event_target_value(&ev))
                                >
                                    <option value="">"All collections"</option>
                                    {move || collections.get().into_iter().map(|c| {
                                        let id = c.id.to_string();
                                        view! { <option value=id>{c.name}</option> }
                                    }).collect_view()}
                                </select>
                            </div>
                            <label class="form-check-label">
                                <input
                                    type="checkbox"
                                    prop:checked=move || delete_collections.get()
                                    disabled=move || !checkbox_allowed()
                                    on:change=move |ev| delete_collections.set(event_target_checked(&ev))
                                />
                                {checkbox_label}
                            </label>
                            {move || (!checkbox_allowed()).then(|| view! {
                                <p class="dialog-hint">"(you can only delete collections you own)"</p>
                            })}
                            <p class="dialog-hint">"This can't be undone."</p>
                            {move || error_msg.get().map(|e| view! { <p class="dialog-error">{e}</p> })}
                        </div>
                        <div class="dialog-actions">
                            <button
                                class="btn-danger"
                                disabled=move || saving.get()
                                on:click=do_clear
                            >{move || if saving.get() { "Clearing…" } else { "Clear" }}</button>
                            <button
                                class="btn-secondary"
                                disabled=move || saving.get()
                                on:click=move |_| armed.set(false)
                            >"Cancel"</button>
                        </div>
                    </div>
                </div>
            }
        })
    }
}
