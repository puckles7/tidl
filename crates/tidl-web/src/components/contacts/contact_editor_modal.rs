//! Contact create/edit modal — mirrors `notes::NoteEditorModal`'s structure
//! (same `.dialog-backdrop`/`.dialog` chrome, same save/cancel/delete
//! button row) with repeatable email/phone/url/IM rows and a repeatable
//! address block in place of Notes' task/await fields.

use leptos::prelude::*;

use crate::api::{
    create_contact, delete_contact, delete_contact_photo, update_contact, upload_contact_photo,
    ContactAddress, ContactDto, ContactMethod, NewContact,
};
use crate::util::{format_server_fn_error, mouse_event_targets_self};

const METHOD_KINDS: [&str; 4] = ["home", "work", "mobile", "other"];
const ADDRESS_KINDS: [&str; 3] = ["home", "work", "other"];

/// Split a comma-separated tag list into trimmed, non-empty entries —
/// simplest UI for `CATEGORIES` (a flat vCard tag list); a fancier
/// chip-input isn't worth building for a field this secondary.
fn split_categories(s: &str) -> Vec<String> {
    s.split(',')
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect()
}

#[component]
pub fn ContactEditorModal(
    /// `None` to create; `Some(contact)` to edit.
    contact: Option<ContactDto>,
    on_close: impl Fn() + Send + Sync + 'static,
    on_refresh: Callback<()>,
) -> impl IntoView {
    let on_close =
        StoredValue::new(std::sync::Arc::new(on_close) as std::sync::Arc<dyn Fn() + Send + Sync>);

    let is_edit = contact.is_some();
    let contact_id = StoredValue::new(contact.as_ref().map(|c| c.id));

    let display_name_val = RwSignal::new(
        contact
            .as_ref()
            .map(|c| c.display_name.clone())
            .unwrap_or_default(),
    );
    let given_name_val = RwSignal::new(
        contact
            .as_ref()
            .and_then(|c| c.given_name.clone())
            .unwrap_or_default(),
    );
    let middle_name_val = RwSignal::new(
        contact
            .as_ref()
            .and_then(|c| c.middle_name.clone())
            .unwrap_or_default(),
    );
    let family_name_val = RwSignal::new(
        contact
            .as_ref()
            .and_then(|c| c.family_name.clone())
            .unwrap_or_default(),
    );
    let name_prefix_val = RwSignal::new(
        contact
            .as_ref()
            .and_then(|c| c.name_prefix.clone())
            .unwrap_or_default(),
    );
    let name_suffix_val = RwSignal::new(
        contact
            .as_ref()
            .and_then(|c| c.name_suffix.clone())
            .unwrap_or_default(),
    );
    let nickname_val = RwSignal::new(
        contact
            .as_ref()
            .and_then(|c| c.nickname.clone())
            .unwrap_or_default(),
    );
    let org_val = RwSignal::new(
        contact
            .as_ref()
            .and_then(|c| c.org.clone())
            .unwrap_or_default(),
    );
    let title_val = RwSignal::new(
        contact
            .as_ref()
            .and_then(|c| c.title.clone())
            .unwrap_or_default(),
    );
    let birthday_val = RwSignal::new(
        contact
            .as_ref()
            .and_then(|c| c.birthday.clone())
            .unwrap_or_default(),
    );
    let anniversary_val = RwSignal::new(
        contact
            .as_ref()
            .and_then(|c| c.anniversary.clone())
            .unwrap_or_default(),
    );
    let notes_val = RwSignal::new(
        contact
            .as_ref()
            .and_then(|c| c.notes.clone())
            .unwrap_or_default(),
    );
    let categories_val = RwSignal::new(
        contact
            .as_ref()
            .map(|c| c.categories.join(", "))
            .unwrap_or_default(),
    );

    let emails_val: RwSignal<Vec<ContactMethod>> = RwSignal::new(
        contact
            .as_ref()
            .map(|c| c.emails.clone())
            .unwrap_or_default(),
    );
    let phones_val: RwSignal<Vec<ContactMethod>> = RwSignal::new(
        contact
            .as_ref()
            .map(|c| c.phones.clone())
            .unwrap_or_default(),
    );
    let urls_val: RwSignal<Vec<ContactMethod>> =
        RwSignal::new(contact.as_ref().map(|c| c.urls.clone()).unwrap_or_default());
    let impps_val: RwSignal<Vec<ContactMethod>> = RwSignal::new(
        contact
            .as_ref()
            .map(|c| c.impps.clone())
            .unwrap_or_default(),
    );
    let addresses_val: RwSignal<Vec<ContactAddress>> = RwSignal::new(
        contact
            .as_ref()
            .map(|c| c.addresses.clone())
            .unwrap_or_default(),
    );

    let saving = RwSignal::new(false);
    let error_msg = RwSignal::<Option<String>>::new(None);

    // Photo (#86) — decoupled from the rest of the form's fields, both here
    // and on the wire (see upload_contact_photo/delete_contact_photo's doc
    // comments): a picked file is staged client-side as (base64, mime) and
    // only sent on Save, in its own call, so editing an unrelated field
    // never re-sends photo bytes and a photo change never touches the rest.
    let existing_has_photo = contact.as_ref().map(|c| c.has_photo).unwrap_or(false);
    let staged_photo = RwSignal::new(Option::<(String, String)>::None);
    let photo_removed = RwSignal::new(false);
    let photo_error = RwSignal::<Option<String>>::new(None);

    let photo_preview_src = move || -> Option<String> {
        if let Some((b64, content_type)) = staged_photo.get() {
            Some(format!("data:{content_type};base64,{b64}"))
        } else if existing_has_photo && !photo_removed.get() {
            contact_id
                .get_value()
                .map(|id| format!("/api/contacts/{id}/photo"))
        } else {
            None
        }
    };

    let on_photo_change = move |ev: leptos::ev::Event| {
        #[cfg(feature = "hydrate")]
        {
            use wasm_bindgen::JsCast as _;
            let input = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok());
            let Some(file) = input.and_then(|i| i.files()).and_then(|f| f.get(0)) else {
                return;
            };
            let content_type = file.type_();
            if !matches!(content_type.as_str(), "image/jpeg" | "image/png") {
                photo_error.set(Some("Please choose a JPEG or PNG image.".to_string()));
                return;
            }
            // Mirrors upload_contact_photo's server-side cap — checked here
            // too so a too-large file is rejected before spending time
            // reading and base64-encoding it.
            if file.size() > 5.0 * 1024.0 * 1024.0 {
                photo_error.set(Some("Photo is too large (max 5 MiB).".to_string()));
                return;
            }
            photo_error.set(None);
            leptos::task::spawn_local(async move {
                let promise = file.array_buffer();
                match wasm_bindgen_futures::JsFuture::from(promise).await {
                    Ok(buf) => {
                        let bytes = js_sys::Uint8Array::new(&buf).to_vec();
                        use base64::Engine;
                        let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                        staged_photo.set(Some((b64, content_type)));
                        photo_removed.set(false);
                    }
                    Err(_) => {
                        photo_error.set(Some("Could not read photo.".to_string()));
                    }
                }
            });
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = ev;
    };

    let remove_photo = move |_: leptos::ev::MouseEvent| {
        staged_photo.set(None);
        photo_removed.set(true);
    };

    /// Fires the staged photo change (upload/delete/none) after
    /// create_contact/update_contact resolves, for the id it returns.
    async fn apply_staged_photo(
        id: i64,
        staged: Option<(String, String)>,
        removed: bool,
    ) -> Result<(), server_fn::ServerFnError> {
        if let Some((b64, content_type)) = staged {
            upload_contact_photo(id, b64, content_type).await?;
        } else if removed {
            delete_contact_photo(id).await?;
        }
        Ok(())
    }

    // ── Collection picker (backlog #81) ───────────────────────────────────────
    // Defaults to the contact's existing collection when editing, or the
    // last-used collection when creating — mirrors `NoteEditorModal`'s
    // `LastUsedCollectionCtx` pattern. Editing never moves a contact
    // between collections in this pass (see `NewContact::collection_id`'s
    // doc comment), so the picker is disabled once `is_edit` is true.
    let last_used_collection =
        use_context::<crate::components::contacts::LastUsedContactCollectionCtx>().map(|ctx| ctx.0);
    let collection_id_val: RwSignal<Option<i64>> = RwSignal::new(
        contact
            .as_ref()
            .map(|c| c.collection_id)
            .or_else(|| last_used_collection.and_then(|s| s.get_untracked())),
    );
    let collections_res = LocalResource::new(|| async move {
        crate::api::list_contact_collections()
            .await
            .unwrap_or_default()
    });

    let build_input = move || -> NewContact {
        let opt = |s: String| (!s.trim().is_empty()).then(|| s.trim().to_string());
        NewContact {
            display_name: display_name_val.get_untracked(),
            given_name: opt(given_name_val.get_untracked()),
            middle_name: opt(middle_name_val.get_untracked()),
            family_name: opt(family_name_val.get_untracked()),
            name_prefix: opt(name_prefix_val.get_untracked()),
            name_suffix: opt(name_suffix_val.get_untracked()),
            nickname: opt(nickname_val.get_untracked()),
            org: opt(org_val.get_untracked()),
            title: opt(title_val.get_untracked()),
            birthday: opt(birthday_val.get_untracked()),
            anniversary: opt(anniversary_val.get_untracked()),
            notes: opt(notes_val.get_untracked()),
            categories: split_categories(&categories_val.get_untracked()),
            emails: emails_val.get_untracked(),
            phones: phones_val.get_untracked(),
            urls: urls_val.get_untracked(),
            impps: impps_val.get_untracked(),
            addresses: addresses_val.get_untracked(),
            collection_id: collection_id_val.get_untracked(),
        }
    };

    let save = move |_: leptos::ev::MouseEvent| {
        if display_name_val.get_untracked().trim().is_empty() {
            error_msg.set(Some("Name is required.".into()));
            return;
        }
        saving.set(true);
        error_msg.set(None);
        let input = build_input();
        let on_close = on_close;
        let on_refresh = on_refresh;

        if is_edit {
            let id = contact_id.get_value().unwrap_or_default();
            let staged = staged_photo.get_untracked();
            let removed = photo_removed.get_untracked();
            leptos::task::spawn_local(async move {
                match update_contact(id, input).await {
                    Ok(_) => match apply_staged_photo(id, staged, removed).await {
                        Ok(()) => {
                            saving.set(false);
                            on_close.get_value()();
                            on_refresh.run(());
                        }
                        Err(e) => {
                            error_msg.set(Some(format_server_fn_error(&e.to_string())));
                            saving.set(false);
                        }
                    },
                    Err(e) => {
                        error_msg.set(Some(format_server_fn_error(&e.to_string())));
                        saving.set(false);
                    }
                }
            });
        } else {
            let staged = staged_photo.get_untracked();
            leptos::task::spawn_local(async move {
                match create_contact(input).await {
                    Ok(dto) => match apply_staged_photo(dto.id, staged, false).await {
                        Ok(()) => {
                            if let Some(sig) = last_used_collection {
                                sig.set(Some(dto.collection_id));
                            }
                            saving.set(false);
                            on_close.get_value()();
                            on_refresh.run(());
                        }
                        Err(e) => {
                            error_msg.set(Some(format_server_fn_error(&e.to_string())));
                            saving.set(false);
                        }
                    },
                    Err(e) => {
                        error_msg.set(Some(format_server_fn_error(&e.to_string())));
                        saving.set(false);
                    }
                }
            });
        }
    };

    let do_delete = move |_: leptos::ev::MouseEvent| {
        let id = contact_id.get_value().unwrap_or_default();
        saving.set(true);
        error_msg.set(None);
        let on_close = on_close;
        let on_refresh = on_refresh;
        leptos::task::spawn_local(async move {
            match delete_contact(id).await {
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
        "Edit contact"
    } else {
        "New contact"
    };
    let backdrop_mousedown = RwSignal::new(false);
    let esc_id = crate::modal_esc::use_escape_close(move || on_close.get_value()());

    view! {
        <div class="dialog-backdrop"
            data-esc-id=esc_id.to_string()
            on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
            on:click=move |_| if backdrop_mousedown.get() { on_close.get_value()() }
        >
            <div class="dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                <div class="dialog-header">
                    <h2 class="dialog-title">{title}</h2>
                    <div class="dialog-header-actions">
                        <button class="dialog-close" on:click=move |_| on_close.get_value()()>"✕"</button>
                    </div>
                </div>

                <div class="dialog-form">
                    <div class="form-group">
                        <label class="form-label">"Photo"</label>
                        <div class="contact-photo-picker">
                            {move || match photo_preview_src() {
                                Some(src) => view! { <img class="contact-photo-preview" src=src /> }.into_any(),
                                None => view! { <div class="contact-photo-preview--empty"></div> }.into_any(),
                            }}
                            <div class="contact-photo-picker-controls">
                                <input type="file" accept="image/jpeg,image/png" class="form-input"
                                    on:change=on_photo_change
                                    disabled=move || saving.get() />
                                {move || photo_preview_src().is_some().then(|| view! {
                                    <button type="button" class="btn-secondary"
                                        disabled=move || saving.get()
                                        on:click=remove_photo
                                    >"Remove photo"</button>
                                })}
                            </div>
                        </div>
                        {move || photo_error.get().map(|e| view! { <p class="dialog-error">{e}</p> })}
                    </div>

                    <div class="form-group">
                        <input type="text" class="form-input" placeholder="Name"
                            prop:value=move || display_name_val.get()
                            on:input=move |ev| display_name_val.set(event_target_value(&ev))
                            disabled=move || saving.get() />
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Collection"</label>
                        <Transition fallback=|| view! { <p class="dialog-hint">"Loading…"</p> }>
                            {move || {
                                Suspend::new(async move {
                                    let collections = collections_res.await;
                                    view! {
                                        <select
                                            class="form-input"
                                            disabled=move || saving.get() || is_edit
                                            on:change=move |ev| {
                                                let v = event_target_value(&ev);
                                                collection_id_val.set(v.parse().ok());
                                            }
                                        >
                                            {collections.into_iter().map(|c| {
                                                let selected = collection_id_val.get_untracked() == Some(c.id);
                                                view! {
                                                    <option value=c.id.to_string() selected=selected>{c.name}</option>
                                                }
                                            }).collect_view()}
                                        </select>
                                    }
                                })
                            }}
                        </Transition>
                    </div>

                    <div class="contact-field-row">
                        <input type="text" class="form-input" placeholder="Given name"
                            prop:value=move || given_name_val.get()
                            on:input=move |ev| given_name_val.set(event_target_value(&ev))
                            disabled=move || saving.get() />
                        <input type="text" class="form-input" placeholder="Middle name"
                            prop:value=move || middle_name_val.get()
                            on:input=move |ev| middle_name_val.set(event_target_value(&ev))
                            disabled=move || saving.get() />
                        <input type="text" class="form-input" placeholder="Family name"
                            prop:value=move || family_name_val.get()
                            on:input=move |ev| family_name_val.set(event_target_value(&ev))
                            disabled=move || saving.get() />
                    </div>

                    <div class="contact-field-row">
                        <input type="text" class="form-input" placeholder="Name prefix (Dr., …)"
                            prop:value=move || name_prefix_val.get()
                            on:input=move |ev| name_prefix_val.set(event_target_value(&ev))
                            disabled=move || saving.get() />
                        <input type="text" class="form-input" placeholder="Name suffix (Jr., …)"
                            prop:value=move || name_suffix_val.get()
                            on:input=move |ev| name_suffix_val.set(event_target_value(&ev))
                            disabled=move || saving.get() />
                        <input type="text" class="form-input" placeholder="Nickname"
                            prop:value=move || nickname_val.get()
                            on:input=move |ev| nickname_val.set(event_target_value(&ev))
                            disabled=move || saving.get() />
                    </div>

                    <div class="contact-field-row">
                        <input type="text" class="form-input" placeholder="Organization"
                            prop:value=move || org_val.get()
                            on:input=move |ev| org_val.set(event_target_value(&ev))
                            disabled=move || saving.get() />
                        <input type="text" class="form-input" placeholder="Title"
                            prop:value=move || title_val.get()
                            on:input=move |ev| title_val.set(event_target_value(&ev))
                            disabled=move || saving.get() />
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Emails"</label>
                        <ContactMethodRows methods=emails_val saving=saving placeholder="Email address" kinds=&METHOD_KINDS />
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Phones"</label>
                        <ContactMethodRows methods=phones_val saving=saving placeholder="Phone number" kinds=&METHOD_KINDS />
                    </div>

                    <div class="form-group">
                        <label class="form-label">"URLs"</label>
                        <ContactMethodRows methods=urls_val saving=saving placeholder="https://…" kinds=&METHOD_KINDS />
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Instant messaging"</label>
                        <ContactMethodRows methods=impps_val saving=saving placeholder="e.g. xmpp:user@example.com" kinds=&METHOD_KINDS />
                    </div>

                    <div class="contact-field-row">
                        <div class="form-group">
                            <label class="form-label">"Birthday"</label>
                            <input type="date" class="form-input"
                                prop:value=move || birthday_val.get()
                                on:input=move |ev| birthday_val.set(event_target_value(&ev))
                                disabled=move || saving.get() />
                        </div>
                        <div class="form-group">
                            <label class="form-label">"Anniversary"</label>
                            <input type="date" class="form-input"
                                prop:value=move || anniversary_val.get()
                                on:input=move |ev| anniversary_val.set(event_target_value(&ev))
                                disabled=move || saving.get() />
                        </div>
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Addresses"</label>
                        <ContactAddressRows addresses=addresses_val saving=saving />
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Categories"</label>
                        <input type="text" class="form-input" placeholder="Comma-separated, e.g. Colleague, VIP"
                            prop:value=move || categories_val.get()
                            on:input=move |ev| categories_val.set(event_target_value(&ev))
                            disabled=move || saving.get() />
                    </div>

                    <div class="form-group">
                        <textarea class="form-input note-editor-textarea" rows=3 placeholder="Notes…"
                            prop:value=move || notes_val.get()
                            on:input=move |ev| notes_val.set(event_target_value(&ev))
                            disabled=move || saving.get()
                        ></textarea>
                    </div>

                    {move || error_msg.get().map(|e| view! { <p class="dialog-error">{e}</p> })}
                </div>

                <div class="dialog-actions">
                    <button class="btn-primary" disabled=move || saving.get() on:click=save>
                        {move || if saving.get() {
                            if is_edit { "Saving…" } else { "Creating…" }
                        } else if is_edit { "Save" } else { "Create" }}
                    </button>

                    {if is_edit {
                        view! {
                            <button class="btn-danger" disabled=move || saving.get() on:click=do_delete>"Delete"</button>
                        }.into_any()
                    } else { ().into_any() }}

                    <button class="btn-secondary" disabled=move || saving.get() on:click=move |_| on_close.get_value()()>
                        "Cancel"
                    </button>
                </div>
            </div>
        </div>
    }
}

/// Repeatable add/remove rows for one method list (emails/phones/urls/IM) —
/// a text input, a kind picker, and a "primary" radio (`is_primary` is
/// scoped to this one list — an email and a phone may each have their own
/// primary). `kinds` lets callers reuse this for lists with different kind
/// vocabularies (currently all four contact-method lists share the same
/// home/work/mobile/other set, but the parameter keeps that a choice, not
/// an assumption baked into the component).
#[component]
fn ContactMethodRows(
    methods: RwSignal<Vec<ContactMethod>>,
    saving: RwSignal<bool>,
    placeholder: &'static str,
    kinds: &'static [&'static str],
) -> impl IntoView {
    // Built before the `view!` block below — like `week_grid.rs`'s
    // `event_child`, so the macro parser never has to swallow a deeply
    // nested closure body inline as a `<For>` prop value.
    let each_method = move || methods.get().into_iter().enumerate().collect::<Vec<_>>();
    // Keyed on index alone — not the row's content. These rows have no
    // stable id of their own (they don't exist as DB rows until saved), so
    // position is the only real identity. Keying on `m.value` too (as an
    // earlier version did) made every keystroke change the key, which
    // makes `<For>` treat the row as a brand-new item and remount its
    // `<input>` — losing focus after every single character typed.
    let method_key = |(i, _): &(usize, ContactMethod)| *i;
    let method_child = move |(i, m): (usize, ContactMethod)| {
        let kind = m.kind.clone();
        view! {
            <div class="contact-method-row">
                <input type="text" class="form-input" placeholder=placeholder
                    prop:value=m.value.clone()
                    on:input=move |ev| methods.update(|list| {
                        if let Some(entry) = list.get_mut(i) { entry.value = event_target_value(&ev); }
                    })
                    disabled=move || saving.get() />
                <select class="form-input contact-method-kind"
                    prop:value=kind.clone()
                    on:change=move |ev| methods.update(|list| {
                        if let Some(entry) = list.get_mut(i) { entry.kind = event_target_value(&ev); }
                    })
                    disabled=move || saving.get()
                >
                    {kinds.iter().map(|k| {
                        let selected = *k == kind;
                        view! { <option value=*k selected=selected>{*k}</option> }
                    }).collect_view()}
                </select>
                <label class="form-check-label contact-method-primary">
                    <input type="checkbox"
                        prop:checked=m.is_primary
                        on:change=move |ev| {
                            let checked = event_target_checked(&ev);
                            methods.update(|list| {
                                for (j, entry) in list.iter_mut().enumerate() {
                                    entry.is_primary = checked && j == i;
                                }
                            });
                        }
                        disabled=move || saving.get() />
                    "Primary"
                </label>
                <button type="button" class="contact-method-remove"
                    disabled=move || saving.get()
                    on:click=move |_| methods.update(|list| { if i < list.len() { list.remove(i); } })
                >"✕"</button>
            </div>
        }
    };

    view! {
        <div class="contact-method-rows">
            <For each=each_method key=method_key children=method_child />
            <button type="button" class="note-chip note-chip--new"
                disabled=move || saving.get()
                on:click=move |_| methods.update(|list| list.push(ContactMethod {
                    // Not `ContactMethod::default()` — its `kind` would be
                    // "" (String::default()), which matches none of the
                    // `<select>`'s options. The browser then *visually*
                    // falls back to showing the first option ("home")
                    // without ever firing `on:change`, so the underlying
                    // signal is left holding "" — passing client-side but
                    // failing the server's CHECK-constraint-matching
                    // validation with an opaque error. Seed a real value
                    // that matches an actual `<option>` instead.
                    kind: "other".to_string(),
                    ..Default::default()
                }))
            >"+ Add"</button>
        </div>
    }
}

/// Repeatable add/remove rows for the address list — same
/// add/remove/`<For>`-keyed-by-index structure as [`ContactMethodRows`],
/// but each row is a mini-form (street/city/region/postal/country) instead
/// of a single value, and the kind picker is the narrower home/work/other
/// set (see [`ADDRESS_KINDS`] — no "mobile", it doesn't apply to a postal
/// address).
#[component]
fn ContactAddressRows(
    addresses: RwSignal<Vec<ContactAddress>>,
    saving: RwSignal<bool>,
) -> impl IntoView {
    let each_address = move || addresses.get().into_iter().enumerate().collect::<Vec<_>>();
    let address_key = |(i, _): &(usize, ContactAddress)| *i;
    let address_child = move |(i, a): (usize, ContactAddress)| {
        let kind = a.kind.clone();
        // Same rationale as ContactMethodRows' per-field `on:input` closures:
        // each field updates just its own entry in the list by index.
        let set_field = move |f: fn(&mut ContactAddress, String)| {
            move |ev: leptos::ev::Event| {
                let value = event_target_value(&ev);
                addresses.update(|list| {
                    if let Some(entry) = list.get_mut(i) {
                        f(entry, value);
                    }
                });
            }
        };
        view! {
            <div class="contact-address-row">
                <div class="contact-field-row">
                    <select class="form-input contact-method-kind"
                        prop:value=kind.clone()
                        on:change=move |ev| addresses.update(|list| {
                            if let Some(entry) = list.get_mut(i) { entry.kind = event_target_value(&ev); }
                        })
                        disabled=move || saving.get()
                    >
                        {ADDRESS_KINDS.iter().map(|k| {
                            let selected = *k == kind;
                            view! { <option value=*k selected=selected>{*k}</option> }
                        }).collect_view()}
                    </select>
                    <label class="form-check-label contact-method-primary">
                        <input type="checkbox"
                            prop:checked=a.is_primary
                            on:change=move |ev| {
                                let checked = event_target_checked(&ev);
                                addresses.update(|list| {
                                    for (j, entry) in list.iter_mut().enumerate() {
                                        entry.is_primary = checked && j == i;
                                    }
                                });
                            }
                            disabled=move || saving.get() />
                        "Primary"
                    </label>
                    <button type="button" class="contact-method-remove"
                        disabled=move || saving.get()
                        on:click=move |_| addresses.update(|list| { if i < list.len() { list.remove(i); } })
                    >"✕"</button>
                </div>
                <input type="text" class="form-input contact-address-field" placeholder="Street"
                    prop:value=a.street.clone().unwrap_or_default()
                    on:input=set_field(|e, v| e.street = (!v.trim().is_empty()).then_some(v))
                    disabled=move || saving.get() />
                <div class="contact-field-row">
                    <input type="text" class="form-input" placeholder="City"
                        prop:value=a.city.clone().unwrap_or_default()
                        on:input=set_field(|e, v| e.city = (!v.trim().is_empty()).then_some(v))
                        disabled=move || saving.get() />
                    <input type="text" class="form-input" placeholder="Region"
                        prop:value=a.region.clone().unwrap_or_default()
                        on:input=set_field(|e, v| e.region = (!v.trim().is_empty()).then_some(v))
                        disabled=move || saving.get() />
                </div>
                <div class="contact-field-row">
                    <input type="text" class="form-input" placeholder="Postal code"
                        prop:value=a.postal_code.clone().unwrap_or_default()
                        on:input=set_field(|e, v| e.postal_code = (!v.trim().is_empty()).then_some(v))
                        disabled=move || saving.get() />
                    <input type="text" class="form-input" placeholder="Country"
                        prop:value=a.country.clone().unwrap_or_default()
                        on:input=set_field(|e, v| e.country = (!v.trim().is_empty()).then_some(v))
                        disabled=move || saving.get() />
                </div>
            </div>
        }
    };

    view! {
        <div class="contact-method-rows">
            <For each=each_address key=address_key children=address_child />
            <button type="button" class="note-chip note-chip--new"
                disabled=move || saving.get()
                on:click=move |_| addresses.update(|list| list.push(ContactAddress {
                    kind: "home".to_string(),
                    ..Default::default()
                }))
            >"+ Add address"</button>
        </div>
    }
}
