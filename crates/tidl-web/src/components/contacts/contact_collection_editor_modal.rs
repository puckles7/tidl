//! Collection create/edit modal for the Contacts tab (backlog #81).
//!
//! Mechanical port of `notes::collection_editor_modal::CollectionEditorModal`
//! — structurally mirrors `MdSourceEditModal` in `cal_edit_modal.rs`
//! (create/edit, color palette, delete), but uses the warm collection palette
//! (backlog #127). Sharing
//! mirrors `CalendarEditModal`'s Share button + `CalendarShareDialog`
//! sub-modal pattern, including the email-invite/accept-link flow.

use leptos::prelude::*;
use std::sync::Arc;

use crate::api::{
    create_contact_collection, delete_contact_collection, list_contact_collections,
    remove_contact_collection_member, share_contact_collection, update_contact_collection,
    ContactCollectionInfo, NewContactCollection,
};
use crate::util::{format_server_fn_error, mouse_event_targets_self};

/// Contact-collection colour palette — shares the note-collection palette
/// (base hue set rotated +30°; both are "collections"; backlog #127). Kept as
/// its own constant so the two can diverge later without a cross-module
/// dependency.
pub const CONTACT_COLLECTION_PALETTE: &[&str] = &[
    "var(--pal-coll-1)",
    "var(--pal-coll-2)",
    "var(--pal-coll-3)",
    "var(--pal-coll-4)",
    "var(--pal-coll-5)",
    "var(--pal-coll-6)",
];

#[component]
pub fn ContactCollectionEditorModal(
    /// `None` to create; `Some(collection)` to edit an existing one.
    collection: Option<ContactCollectionInfo>,
    on_close: impl Fn() + Send + Sync + 'static,
    on_refresh: Callback<()>,
    /// Fired with the new row's id on a successful *create* only (never on
    /// edit) — lets a caller nesting this modal auto-select the collection
    /// it just created without needing its own separate creation UI.
    #[prop(optional)]
    on_created: Option<Callback<i64>>,
) -> impl IntoView {
    let on_close = StoredValue::new(Arc::new(on_close) as Arc<dyn Fn() + Send + Sync>);

    let is_edit = collection.is_some();
    let coll_id = StoredValue::new(collection.as_ref().map(|c| c.id));

    let name_val = RwSignal::new(
        collection
            .as_ref()
            .map(|c| c.name.clone())
            .unwrap_or_default(),
    );
    let color_val = RwSignal::new(
        collection
            .as_ref()
            .map(|c| c.color.clone())
            .unwrap_or_else(|| CONTACT_COLLECTION_PALETTE[0].to_string()),
    );
    // `shared`/`members` are no longer edited inline here — they're kept in
    // sync with the server via `ContactCollectionShareDialog`'s `on_change`
    // (see the Share sub-dialog below), so Save (name/color only) always
    // resends the latest values instead of clobbering a share made while
    // this modal was open.
    let shared_val = RwSignal::new(collection.as_ref().map(|c| c.shared).unwrap_or(false));
    let members_val: RwSignal<Vec<String>> = RwSignal::new(
        collection
            .as_ref()
            .map(|c| c.members.clone())
            .unwrap_or_default(),
    );

    let saving = RwSignal::new(false);
    let error_msg = RwSignal::<Option<String>>::new(None);

    // ── Sub-modal: share dialog ───────────────────────────────────────────────
    let share_open = RwSignal::new(false);

    let save = move |_: leptos::ev::MouseEvent| {
        let name = name_val.get_untracked();
        if name.trim().is_empty() {
            error_msg.set(Some("Name is required.".into()));
            return;
        }
        saving.set(true);
        error_msg.set(None);

        let input = NewContactCollection {
            name: name.trim().to_string(),
            color: color_val.get_untracked(),
            shared: shared_val.get_untracked(),
            members: members_val.get_untracked(),
        };

        let on_close = on_close;
        let on_refresh = on_refresh;

        if is_edit {
            let id = coll_id.get_value().unwrap_or_default();
            leptos::task::spawn_local(async move {
                match update_contact_collection(id, input).await {
                    Ok(()) => {
                        saving.set(false);
                        on_close.get_value()();
                        on_refresh.run(());
                    }
                    Err(e) => {
                        error_msg.set(Some(format_server_fn_error(&e.to_string())));
                        saving.set(false);
                    }
                }
            });
        } else {
            leptos::task::spawn_local(async move {
                match create_contact_collection(input).await {
                    Ok(id) => {
                        saving.set(false);
                        on_close.get_value()();
                        on_refresh.run(());
                        if let Some(cb) = on_created {
                            cb.run(id);
                        }
                    }
                    Err(e) => {
                        error_msg.set(Some(format_server_fn_error(&e.to_string())));
                        saving.set(false);
                    }
                }
            });
        }
    };

    let delete_open = RwSignal::new(false);

    let do_delete = move |_: leptos::ev::MouseEvent| {
        let id = coll_id.get_value().unwrap_or_default();
        saving.set(true);
        error_msg.set(None);
        let on_close = on_close;
        let on_refresh = on_refresh;
        leptos::task::spawn_local(async move {
            match delete_contact_collection(id).await {
                Ok(()) => {
                    saving.set(false);
                    on_close.get_value()();
                    on_refresh.run(());
                }
                Err(e) => {
                    error_msg.set(Some(format_server_fn_error(&e.to_string())));
                    saving.set(false);
                }
            }
        });
    };

    let title = if is_edit {
        "Edit collection"
    } else {
        "New collection"
    };

    let backdrop_mousedown = RwSignal::new(false);
    let esc_id = crate::modal_esc::use_escape_close(move || {
        // Escape dismisses the nested delete-confirm before the modal itself;
        // an open share dialog is a separate component and wins on its own.
        if delete_open.get_untracked() {
            delete_open.set(false);
        } else {
            on_close.get_value()();
        }
    });
    view! {
        <div class="dialog-backdrop"
            data-esc-id=esc_id.to_string()
            on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
            on:click=move |_| if backdrop_mousedown.get() { on_close.get_value()() }
        >
            <div class="dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                <div class="dialog-header">
                    <h2 class="dialog-title">{title}</h2>
                    <button class="dialog-close" on:click=move |_| on_close.get_value()()>"✕"</button>
                </div>

                <div class="dialog-form">
                    <div class="form-group">
                        <label class="form-label">"Name"</label>
                        <input
                            class="form-input"
                            type="text"
                            placeholder="Collection name"
                            prop:value=move || name_val.get()
                            on:input=move |ev| name_val.set(event_target_value(&ev))
                            disabled=move || saving.get()
                        />
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Color"</label>
                        <div class="note-palette">
                            {CONTACT_COLLECTION_PALETTE.iter().map(|&c| {
                                let color = c.to_string();
                                let color2 = c.to_string();
                                view! {
                                    <button
                                        type="button"
                                        class="note-palette-swatch"
                                        style=move || format!(
                                            "background:{};{}",
                                            color,
                                            if color_val.get() == color {
                                                "outline:2px solid var(--text,#111);outline-offset:2px;"
                                            } else { "" }
                                        )
                                        on:click=move |ev| {
                                            ev.prevent_default();
                                            color_val.set(color2.clone());
                                        }
                                    ></button>
                                }
                            }).collect_view()}
                        </div>
                    </div>

                    {move || error_msg.get().map(|e| view! { <p class="dialog-error">{e}</p> })}
                </div>

                <div class="dialog-actions">
                    <button class="btn-primary" disabled=move || saving.get() on:click=save>
                        {move || if saving.get() {
                            if is_edit { "Saving…" } else { "Creating…" }
                        } else if is_edit { "Save" } else { "Create" }}
                    </button>

                    // Share (edit only — a collection can't be shared before
                    // it exists, matching `CalendarEditModal`'s Share button).
                    {if is_edit {
                        view! {
                            <button
                                class="btn-secondary"
                                disabled=move || saving.get()
                                on:click=move |_| share_open.set(true)
                            >"Share"</button>
                        }.into_any()
                    } else { ().into_any() }}

                    {if is_edit {
                        view! {
                            <button
                                class="btn-danger"
                                disabled=move || saving.get()
                                on:click=move |_| delete_open.set(true)
                            >"Delete"</button>
                        }.into_any()
                    } else { ().into_any() }}

                    <button class="btn-secondary" disabled=move || saving.get() on:click=move |_| on_close.get_value()()>
                        "Cancel"
                    </button>
                </div>
            </div>
        </div>

        {move || delete_open.get().then(|| {
            let backdrop_mousedown = RwSignal::new(false);
            view! {
            <div class="dialog-backdrop"
                on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
                on:click=move |_| if backdrop_mousedown.get() { delete_open.set(false) }
            >
                <div class="dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                    <div class="dialog-header">
                        <h2 class="dialog-title">"Delete collection?"</h2>
                    </div>
                    <div class="dialog-form">
                        <p>"This deletes the collection and all of its contacts. This cannot be undone."</p>
                    </div>
                    <div class="dialog-actions">
                        <button class="btn-danger" disabled=move || saving.get() on:click=do_delete>
                            {move || if saving.get() { "Deleting…" } else { "Yes, delete all" }}
                        </button>
                        <button class="btn-secondary" on:click=move |_| delete_open.set(false)>"Cancel"</button>
                    </div>
                </div>
            </div>
        }})}

        // ── Share sub-dialog ──────────────────────────────────────────────────
        {move || share_open.get().then(|| {
            let id = coll_id.get_value().unwrap_or_default();
            let name_for_share = name_val.get_untracked();
            view! {
                <ContactCollectionShareDialog
                    collection_id=id
                    collection_name=name_for_share
                    on_close=move || share_open.set(false)
                    on_change=Callback::new(move |members: Vec<String>| {
                        shared_val.set(!members.is_empty());
                        members_val.set(members);
                    })
                />
            }
        })}
    }
}

// ── ContactCollectionShareDialog ──────────────────────────────────────────────

/// Inline share management dialog for contact collections — mirrors
/// `notes::collection_editor_modal::NoteCollectionShareDialog`'s
/// invite/revoke pattern, but with no permission `<select>` (a member
/// always gets full read/write on the collection; there's only one tier).
/// `pub(crate)` so `settings::sharing` can reuse it directly instead of
/// duplicating the invite/revoke form there.
#[component]
pub(crate) fn ContactCollectionShareDialog(
    collection_id: i64,
    collection_name: String,
    on_close: impl Fn() + Send + Sync + 'static,
    /// Fired with the fresh member list after every add/revoke, so the
    /// parent modal's `shared_val`/`members_val` never go stale while this
    /// dialog is open.
    on_change: Callback<Vec<String>>,
) -> impl IntoView {
    let on_close = StoredValue::new(Arc::new(on_close) as Arc<dyn Fn() + Send + Sync>);

    let members: RwSignal<Vec<String>> = RwSignal::new(vec![]);
    let members_loading = RwSignal::new(true);
    let members_error = RwSignal::new(Option::<String>::None);

    let refresh_sv: StoredValue<Arc<dyn Fn() + Send + Sync>> =
        StoredValue::new(Arc::new(move || {
            members_loading.set(true);
            leptos::task::spawn_local(async move {
                match list_contact_collections().await {
                    Ok(list) => {
                        let m = list
                            .into_iter()
                            .find(|c| c.id == collection_id)
                            .map(|c| c.members)
                            .unwrap_or_default();
                        on_change.run(m.clone());
                        members.set(m);
                        members_error.set(None);
                    }
                    Err(e) => members_error.set(Some(format_server_fn_error(&e.to_string()))),
                }
                members_loading.set(false);
            });
        }));
    leptos::task::spawn_local(async move {
        refresh_sv.with_value(|f| f());
    });

    let invite_email = RwSignal::new(String::new());
    let inviting = RwSignal::new(false);
    let invite_ok = RwSignal::new(Option::<String>::None);
    let invite_error = RwSignal::new(Option::<String>::None);

    let on_invite = move |_: leptos::ev::MouseEvent| {
        let email = invite_email.get_untracked();
        if email.trim().is_empty() || inviting.get_untracked() {
            return;
        }
        inviting.set(true);
        invite_ok.set(None);
        invite_error.set(None);
        leptos::task::spawn_local(async move {
            match share_contact_collection(collection_id, email.clone()).await {
                Ok(()) => {
                    invite_ok.set(Some(format!("Invitation sent to {email}.")));
                    invite_email.set(String::new());
                    refresh_sv.with_value(|f| f());
                }
                Err(e) => invite_error.set(Some(format_server_fn_error(&e.to_string()))),
            }
            inviting.set(false);
        });
    };

    let backdrop_mousedown = RwSignal::new(false);
    let esc_id = crate::modal_esc::use_escape_close(move || on_close.with_value(|f| f()));
    view! {
        <div class="dialog-backdrop"
            data-esc-id=esc_id.to_string()
            on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
            on:click=move |_| if backdrop_mousedown.get() { on_close.with_value(|f| f()) }
        >
            <div class="dialog share-dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                <div class="dialog-header">
                    <h2 class="dialog-title">{format!("Share \"{}\"", collection_name)}</h2>
                    <button class="dialog-close"
                        on:click=move |_| on_close.with_value(|f| f())>"✕"</button>
                </div>
                <div class="dialog-form">
                    // ── Current members ───────────────────────────────────────
                    <div class="form-group">
                        <label class="form-label">"Shared with"</label>
                        {move || if members_loading.get() {
                            view! { <p class="dialog-hint">"Loading…"</p> }.into_any()
                        } else if let Some(e) = members_error.get() {
                            view! { <p class="dialog-error">{e}</p> }.into_any()
                        } else {
                            let list = members.get();
                            if list.is_empty() {
                                view! { <p class="dialog-hint">"Not shared with anyone yet."</p> }.into_any()
                            } else {
                                view! {
                                    <ul class="share-list">
                                        {list.into_iter().map(|m| {
                                            let m2 = m.clone();
                                            view! {
                                                <li class="share-list-item">
                                                    <span class="share-email">{m.clone()}</span>
                                                    <button
                                                        class="share-revoke-btn"
                                                        on:click=move |_| {
                                                            let u = m2.clone();
                                                            leptos::task::spawn_local(async move {
                                                                let _ = remove_contact_collection_member(collection_id, u).await;
                                                                refresh_sv.with_value(|f| f());
                                                            });
                                                        }
                                                    >"✕"</button>
                                                </li>
                                            }
                                        }).collect_view()}
                                    </ul>
                                }.into_any()
                            }
                        }}
                    </div>

                    // ── Invite by email ───────────────────────────────────────
                    <div class="form-group">
                        <label class="form-label">"Invite by email"</label>
                        <div class="form-row">
                            <input
                                class="form-input"
                                type="email"
                                placeholder="user@example.com"
                                prop:value=move || invite_email.get()
                                on:input=move |ev| invite_email.set(event_target_value(&ev))
                                disabled=move || inviting.get()
                            />
                            <button
                                type="button"
                                class="btn-secondary"
                                disabled=move || inviting.get()
                                on:click=on_invite
                            >
                                {move || if inviting.get() { "Inviting…" } else { "Invite" }}
                            </button>
                        </div>
                    </div>
                    {move || invite_ok.get().map(|m| view! { <p class="dialog-ok">{m}</p> })}
                    {move || invite_error.get().map(|e| view! { <p class="dialog-error">{e}</p> })}
                </div>
                <div class="dialog-actions">
                    <button
                        class="btn-secondary"
                        on:click=move |_| on_close.with_value(|f| f())
                    >"Done"</button>
                </div>
            </div>
        </div>
    }
}
