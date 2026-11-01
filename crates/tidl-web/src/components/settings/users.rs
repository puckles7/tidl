use leptos::prelude::*;

use crate::api::{
    admin_delete_user, admin_reset_password, create_user_admin, get_user_deletion_summary,
    list_users, set_user_active, set_user_admin, UserAdminInfo,
};
use crate::app::CurrentUsernameCtx;
use crate::util::format_server_fn_error;

#[component]
pub(super) fn UsersPanel() -> impl IntoView {
    let current_username = use_context::<CurrentUsernameCtx>()
        .map(|c| c.0.get_untracked())
        .unwrap_or_default();

    let users_list = RwSignal::new(Vec::<UserAdminInfo>::new());
    let users_loading = RwSignal::new(true);
    let list_error = RwSignal::new(Option::<String>::None);

    // Create-user form state.
    let new_username = RwSignal::new(String::new());
    let new_email = RwSignal::new(String::new());
    let new_display_name = RwSignal::new(String::new());
    let new_password = RwSignal::new(String::new());
    let creating = RwSignal::new(false);
    let create_error = RwSignal::new(Option::<String>::None);

    // At most one row's reset-password panel open at a time.
    let reset_open_for = RwSignal::new(Option::<String>::None);
    let reset_password = RwSignal::new(String::new());
    let reset_saving = RwSignal::new(false);
    let reset_error = RwSignal::new(Option::<String>::None);

    // At most one row's delete-confirmation panel open at a time.
    let delete_open_for = RwSignal::new(Option::<String>::None);
    let delete_summary = RwSignal::new(Option::<crate::api::UserDeletionSummary>::None);
    let delete_confirm_text = RwSignal::new(String::new());
    let delete_error = RwSignal::new(Option::<String>::None);
    let deleting = RwSignal::new(false);

    let reload_users = move || {
        users_loading.set(true);
        leptos::task::spawn_local(async move {
            match list_users().await {
                Ok(list) => {
                    users_list.set(list);
                    list_error.set(None);
                }
                Err(e) => list_error.set(Some(format_server_fn_error(&e.to_string()))),
            }
            users_loading.set(false);
        });
    };

    // Load on mount.
    reload_users();

    let on_create = move |_: leptos::ev::MouseEvent| {
        if creating.get_untracked() {
            return;
        }
        creating.set(true);
        create_error.set(None);
        let username = new_username.get_untracked();
        let email = new_email.get_untracked();
        let display_name = new_display_name.get_untracked();
        let password = new_password.get_untracked();
        leptos::task::spawn_local(async move {
            match create_user_admin(username, email, display_name, password).await {
                Ok(()) => {
                    new_username.set(String::new());
                    new_email.set(String::new());
                    new_display_name.set(String::new());
                    new_password.set(String::new());
                    reload_users();
                }
                Err(e) => create_error.set(Some(format_server_fn_error(&e.to_string()))),
            }
            creating.set(false);
        });
    };

    view! {
        <h3 class="settings-section-heading">"Create new user"</h3>
        {move || create_error.get().map(|e| view! { <div class="form-error">{e}</div> })}
        <div class="form-group">
            <label class="form-label">"Username"</label>
            <input class="form-input" type="text" autocomplete="off"
                prop:value=move || new_username.get()
                on:input=move |ev| new_username.set(event_target_value(&ev)) />
        </div>
        <div class="form-group">
            <label class="form-label">"Email"</label>
            <input class="form-input" type="email" autocomplete="off"
                prop:value=move || new_email.get()
                on:input=move |ev| new_email.set(event_target_value(&ev)) />
        </div>
        <div class="form-group">
            <label class="form-label">"Display name"</label>
            <input class="form-input" type="text"
                prop:value=move || new_display_name.get()
                on:input=move |ev| new_display_name.set(event_target_value(&ev)) />
        </div>
        <div class="form-group">
            <label class="form-label">"Password"</label>
            <input class="form-input" type="password" autocomplete="new-password"
                prop:value=move || new_password.get()
                on:input=move |ev| new_password.set(event_target_value(&ev)) />
        </div>
        <button class="btn-primary"
            disabled=move || creating.get()
            on:click=on_create>
            {move || if creating.get() { "Creating…" } else { "Create user" }}
        </button>

        <h3 class="settings-section-heading">"Existing users"</h3>
        {move || list_error.get().map(|e| view! { <div class="form-error">{e}</div> })}
        {move || {
            if users_loading.get() {
                return view! { <p class="sharing-empty">"Loading…"</p> }.into_any();
            }
            let list = users_list.get();
            if list.is_empty() {
                return view! { <p class="sharing-empty">"No users."</p> }.into_any();
            }
            let current_username = current_username.clone();
            list.into_iter().map(|u| {
                let is_self = u.username == current_username;
                // Copy the two flags out to plain `bool`s before building the
                // closures below — `u` itself is a `UserAdminInfo` (not
                // `Copy`), and pulling the fields out up front avoids relying
                // on precise-capture inference to figure out that each
                // `move` closure only touches one field of `u`.
                let is_admin = u.is_admin;
                let is_active = u.is_active;
                let username = u.username.clone();
                let username_admin = username.clone();
                let username_active = username.clone();
                let username_reset = username.clone();
                let username_delete = username.clone();
                let username_delete_summary = username.clone();

                view! {
                    <div class="sharing-row user-row">
                        <span class="sharing-grantee">
                            {u.display_name.clone()} " (" {u.username.clone()} ") — " {u.email.clone()}
                            {if is_admin { " · admin" } else { "" }}
                            {if !is_active { " · deactivated" } else { "" }}
                        </span>

                        <button class="btn-secondary-small" disabled=is_self
                            on:click=move |_| {
                                let username = username_admin.clone();
                                let next = !is_admin;
                                leptos::task::spawn_local(async move {
                                    let _ = set_user_admin(username, next).await;
                                    reload_users();
                                });
                            }
                        >{if is_admin { "Revoke admin" } else { "Make admin" }}</button>

                        <button class="btn-secondary-small" disabled=is_self
                            on:click=move |_| {
                                let username = username_active.clone();
                                let next = !is_active;
                                leptos::task::spawn_local(async move {
                                    let _ = set_user_active(username, next).await;
                                    reload_users();
                                });
                            }
                        >{if is_active { "Deactivate" } else { "Activate" }}</button>

                        <button class="btn-secondary-small"
                            on:click=move |_| {
                                reset_open_for.set(Some(username_reset.clone()));
                                reset_password.set(String::new());
                                reset_error.set(None);
                            }
                        >"Reset password"</button>

                        <button class="btn-danger-small" disabled=is_self
                            on:click=move |_| {
                                let target = username_delete_summary.clone();
                                delete_open_for.set(Some(target.clone()));
                                delete_summary.set(None);
                                delete_confirm_text.set(String::new());
                                delete_error.set(None);
                                leptos::task::spawn_local(async move {
                                    if let Ok(summary) = get_user_deletion_summary(target).await {
                                        delete_summary.set(Some(summary));
                                    }
                                });
                            }
                        >"Delete"</button>
                    </div>

                    {move || (reset_open_for.get().as_deref() == Some(username.as_str())).then(|| {
                        let username = username.clone();
                        view! {
                            <div class="form-group">
                                {move || reset_error.get().map(|e| view! { <div class="form-error">{e}</div> })}
                                <input class="form-input" type="password" autocomplete="new-password"
                                    placeholder="New password"
                                    prop:value=move || reset_password.get()
                                    on:input=move |ev| reset_password.set(event_target_value(&ev)) />
                                <button class="btn-primary"
                                    disabled=move || reset_saving.get()
                                    on:click=move |_| {
                                        if reset_saving.get_untracked() { return; }
                                        reset_saving.set(true);
                                        reset_error.set(None);
                                        let username = username.clone();
                                        let password = reset_password.get_untracked();
                                        leptos::task::spawn_local(async move {
                                            match admin_reset_password(username, password).await {
                                                Ok(()) => reset_open_for.set(None),
                                                Err(e) => reset_error.set(Some(format_server_fn_error(&e.to_string()))),
                                            }
                                            reset_saving.set(false);
                                        });
                                    }
                                >{move || if reset_saving.get() { "Resetting…" } else { "Confirm reset" }}</button>
                                <button class="btn-cancel" on:click=move |_| reset_open_for.set(None)>"Cancel"</button>
                            </div>
                        }
                    })}

                    {move || (delete_open_for.get().as_deref() == Some(username_delete.as_str())).then(|| {
                        let username = username_delete.clone();
                        let username_confirm = username_delete.clone();
                        let username_for_disabled = username.clone();
                        view! {
                            <div class="form-group">
                                {move || delete_error.get().map(|e| view! { <div class="form-error">{e}</div> })}
                                {move || delete_summary.get().map(|s| view! {
                                    <p class="dialog-hint">
                                        {username_confirm.clone()} " owns " {s.calendars} " calendar(s), "
                                        {s.collections} " note collection(s), " {s.notes} " note(s), "
                                        {s.shares_owned} " share(s) granted, and " {s.app_tokens}
                                        " app token(s) — all will be permanently deleted."
                                    </p>
                                })}
                                <input class="form-input" type="text"
                                    placeholder="Type the username to confirm"
                                    prop:value=move || delete_confirm_text.get()
                                    on:input=move |ev| delete_confirm_text.set(event_target_value(&ev)) />
                                <button class="btn-danger-small"
                                    disabled=move || deleting.get() || delete_confirm_text.get() != username_for_disabled
                                    on:click=move |_| {
                                        if deleting.get_untracked() { return; }
                                        deleting.set(true);
                                        delete_error.set(None);
                                        let username = username.clone();
                                        let confirm = delete_confirm_text.get_untracked();
                                        leptos::task::spawn_local(async move {
                                            match admin_delete_user(username, confirm).await {
                                                Ok(()) => { delete_open_for.set(None); reload_users(); }
                                                Err(e) => delete_error.set(Some(format_server_fn_error(&e.to_string()))),
                                            }
                                            deleting.set(false);
                                        });
                                    }
                                >{move || if deleting.get() { "Deleting…" } else { "Delete permanently" }}</button>
                                <button class="btn-cancel" on:click=move |_| delete_open_for.set(None)>"Cancel"</button>
                            </div>
                        }
                    })}
                }
            }).collect_view().into_any()
        }}
    }
}
