//! "Clear all" confirmation for the Contacts toolbar (backlog #90/#81) —
//! mirrors `notes::clear_notes_modal::ClearNotesModal` now that contacts
//! also live in collections: a scope picker (all collections, or one),
//! plus a checkbox to delete the collection(s) themselves rather than just
//! their contacts.
//!
//! The "also delete" checkbox reuses `delete_contact_collection` (which
//! already cascades to a collection's contacts) for a single collection.
//! For "All collections" a shared-with-the-viewer collection can't be
//! deleted by them (owner-only), so that scope still runs
//! `clear_contacts(None)` first to empty everything visible, then deletes
//! every collection the viewer owns except one (whichever sorts last in
//! `owned_collection_ids`, itself alphabetical per `list_contact_collections`
//! — not necessarily "My Contacts"), since `delete_contact_collection`
//! refuses to remove an owner's only collection (a contact must always
//! have somewhere to live). A text hint surfaces that constraint up front
//! — disabling the checkbox outright when the single selected collection
//! is the only one the viewer owns, and noting that one collection is
//! kept when the "All collections" scope is checked — rather than letting
//! either case round-trip to a server error.

use leptos::prelude::*;

use crate::api::{clear_contacts, delete_contact_collection};
use crate::types::ContactCollectionInfo;
use crate::util::{format_server_fn_error, mouse_event_targets_self};

async fn run_clear(
    collection_id: Option<i64>,
    delete_collections: bool,
    owned_collection_ids: Vec<i64>,
) -> Result<(), String> {
    if !delete_collections {
        return clear_contacts(collection_id)
            .await
            .map_err(|e| format_server_fn_error(&e.to_string()));
    }

    match collection_id {
        Some(id) => delete_contact_collection(id)
            .await
            .map_err(|e| format_server_fn_error(&e.to_string())),
        None => {
            clear_contacts(None)
                .await
                .map_err(|e| format_server_fn_error(&e.to_string()))?;
            // The owner must always keep at least one collection — leave the
            // last one in place (now empty) rather than attempt a delete the
            // server would reject anyway.
            let keep = owned_collection_ids.len().saturating_sub(1);
            for id in owned_collection_ids.into_iter().take(keep) {
                delete_contact_collection(id)
                    .await
                    .map_err(|e| format_server_fn_error(&e.to_string()))?;
            }
            Ok(())
        }
    }
}

#[component]
pub fn ClearContactsModal(
    armed: RwSignal<bool>,
    collections: Signal<Vec<ContactCollectionInfo>>,
    on_refresh: Callback<()>,
) -> impl IntoView {
    // "" means "All collections" — see `ClearNotesModal` for why this is
    // the default option's value rather than an arbitrary first collection.
    let selected = RwSignal::new(String::new());
    let delete_collections = RwSignal::new(false);
    let saving = RwSignal::new(false);
    let error_msg: RwSignal<Option<String>> = RwSignal::new(None);

    let owned_count = move || collections.get().into_iter().filter(|c| c.is_owner).count();

    let selected_collection = move || {
        selected
            .get()
            .parse::<i64>()
            .ok()
            .and_then(|id| collections.get().into_iter().find(|c| c.id == id))
    };

    // The viewer's only owned collection — `delete_contact_collection`
    // refuses to delete it server-side (a contact must always have
    // somewhere to live), so disable the checkbox here rather than let the
    // attempt round-trip to a server error.
    let is_mandatory_selection =
        move || selected_collection().is_some_and(|c| c.is_owner) && owned_count() <= 1;

    let checkbox_allowed = move || {
        let sel = selected.get();
        sel.is_empty()
            || (selected_collection().is_some_and(|c| c.is_owner) && !is_mandatory_selection())
    };

    Effect::new(move |_| {
        if !checkbox_allowed() {
            delete_collections.set(false);
        }
    });

    // Explains a disabled checkbox; `None` once it's enabled again.
    let disabled_hint = move || {
        if checkbox_allowed() {
            None
        } else if is_mandatory_selection() {
            Some("(this is your only collection, so it can't be deleted)")
        } else {
            Some("(you can only delete collections you own)")
        }
    };

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
                            <h2 class="dialog-title">"Clear contacts"</h2>
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
                            {move || disabled_hint().map(|msg| view! {
                                <p class="dialog-hint">{msg}</p>
                            })}
                            {move || (selected.get().is_empty() && delete_collections.get()).then(|| view! {
                                <p class="dialog-hint">"(one of your collections will be kept, not deleted)"</p>
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
