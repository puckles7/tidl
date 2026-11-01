//! Profile modal — opened by clicking the username in the navbar.
//!
//! Unlike `SettingsModal`, this has no tabs: each section (display name,
//! username, email, password) saves independently, matching how
//! `PreferencesPanel` etc. already work.

use leptos::prelude::*;

use crate::api::{
    change_password, confirm_email_change, get_profile, rename_username_api, request_email_change,
    save_display_name, ProfileInfo,
};
use crate::app::CurrentUsernameCtx;
use crate::util::{format_server_fn_error, mouse_event_targets_self};

#[component]
pub fn ProfileModal(on_close: impl Fn() + Send + Sync + 'static) -> impl IntoView {
    let on_close =
        StoredValue::new(std::sync::Arc::new(on_close) as std::sync::Arc<dyn Fn() + Send + Sync>);
    let profile_res: LocalResource<Result<ProfileInfo, ServerFnError>> =
        LocalResource::new(get_profile);
    let backdrop_mousedown = RwSignal::new(false);
    let esc_id = crate::modal_esc::use_escape_close(move || on_close.with_value(|f| f()));

    view! {
        <div class="dialog-backdrop"
            data-esc-id=esc_id.to_string()
            on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
            on:click=move |_| if backdrop_mousedown.get() { on_close.with_value(|f| f()) }
        >
            <div class="dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                <div class="dialog-header">
                    <h2 class="dialog-title">"Profile"</h2>
                    <button class="dialog-close" on:click=move |_| on_close.with_value(|f| f())>"✕"</button>
                </div>

                <div class="dialog-form">
                    <Suspense fallback=|| view! { <p class="dialog-hint">"Loading…"</p> }>
                        {move || Suspend::new(async move {
                            match profile_res.await {
                                Ok(profile) => view! {
                                    <ProfileForm profile=profile on_close=on_close />
                                }.into_any(),
                                Err(e) => view! {
                                    <div class="form-error">{format_server_fn_error(&e.to_string())}</div>
                                }.into_any(),
                            }
                        })}
                    </Suspense>
                </div>
            </div>
        </div>
    }
}

#[component]
fn ProfileForm(
    profile: ProfileInfo,
    on_close: StoredValue<std::sync::Arc<dyn Fn() + Send + Sync>>,
) -> impl IntoView {
    let current_username_ctx = use_context::<CurrentUsernameCtx>();

    view! {
        <DisplayNameSection initial=profile.display_name.clone() />
        <UsernameSection initial=profile.username.clone() current_username_ctx=current_username_ctx />
        <EmailSection initial=profile.email.clone() />
        <PasswordSection />

        <div class="dialog-actions">
            <button class="btn-cancel" on:click=move |_| on_close.with_value(|f| f())>"Close"</button>
        </div>
    }
}

// ── Display name ────────────────────────────────────────────────────────────────

#[component]
fn DisplayNameSection(initial: String) -> impl IntoView {
    let display_name = RwSignal::new(initial);
    let is_saving = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);
    let saved = RwSignal::new(false);

    let on_save = move |_: leptos::ev::MouseEvent| {
        if is_saving.get_untracked() {
            return;
        }
        is_saving.set(true);
        error.set(None);
        saved.set(false);
        let name = display_name.get_untracked();
        leptos::task::spawn_local(async move {
            match save_display_name(name).await {
                Ok(()) => {
                    is_saving.set(false);
                    saved.set(true);
                }
                Err(e) => {
                    is_saving.set(false);
                    error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
        });
    };

    view! {
        <div class="form-group">
            <label class="form-label">"Display Name"</label>
            <input
                type="text"
                class="form-input"
                prop:value=move || display_name.get()
                on:input=move |ev| { display_name.set(event_target_value(&ev)); saved.set(false); }
            />
            {move || error.get().map(|e| view! { <div class="form-error">{e}</div> })}
            {move || saved.get().then(|| view! { <div class="dialog-ok">"Saved"</div> })}
            <div class="form-actions">
                <button class="btn-primary" on:click=on_save disabled=move || is_saving.get()>
                    {move || if is_saving.get() { "Saving…" } else { "Save" }}
                </button>
            </div>
        </div>
    }
}

// ── Username ────────────────────────────────────────────────────────────────────

#[component]
fn UsernameSection(
    initial: String,
    current_username_ctx: Option<CurrentUsernameCtx>,
) -> impl IntoView {
    let loaded_username = RwSignal::new(initial.clone());
    let username = RwSignal::new(initial);
    let is_saving = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);
    let saved = RwSignal::new(false);

    let changed = move || username.get() != loaded_username.get();

    let on_save = move |_: leptos::ev::MouseEvent| {
        if is_saving.get_untracked() {
            return;
        }
        is_saving.set(true);
        error.set(None);
        saved.set(false);
        let new_username = username.get_untracked();
        leptos::task::spawn_local(async move {
            match rename_username_api(new_username).await {
                Ok(applied) => {
                    is_saving.set(false);
                    saved.set(true);
                    loaded_username.set(applied.clone());
                    username.set(applied.clone());
                    if let Some(ctx) = current_username_ctx {
                        ctx.0.set(applied);
                    }
                }
                Err(e) => {
                    is_saving.set(false);
                    error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
        });
    };

    view! {
        <div class="form-group">
            <label class="form-label">"Username"</label>
            <input
                type="text"
                class="form-input"
                prop:value=move || username.get()
                on:input=move |ev| { username.set(event_target_value(&ev)); saved.set(false); }
            />
            {move || changed().then(|| view! {
                <div class="note-sharing-warning">
                    "Renaming changes your CalDAV and CardDAV addresses (/caldav/"
                    {move || username.get()}
                    "/…, /carddav/"
                    {move || username.get()}
                    "/…) — connected apps and devices will need reconfiguring."
                </div>
            })}
            {move || error.get().map(|e| view! { <div class="form-error">{e}</div> })}
            {move || saved.get().then(|| view! { <div class="dialog-ok">"Saved"</div> })}
            <div class="form-actions">
                <button
                    class="btn-primary"
                    on:click=on_save
                    disabled=move || is_saving.get() || !changed()
                >
                    {move || if is_saving.get() { "Saving…" } else { "Save" }}
                </button>
            </div>
        </div>
    }
}

// ── Email ───────────────────────────────────────────────────────────────────────

#[component]
fn EmailSection(initial: String) -> impl IntoView {
    let current_email = RwSignal::new(initial);
    let new_email = RwSignal::new(String::new());
    let current_password = RwSignal::new(String::new());
    let code = RwSignal::new(String::new());
    let awaiting_code = RwSignal::new(false);
    let is_busy = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);
    let saved = RwSignal::new(false);

    let on_request = move |_: leptos::ev::MouseEvent| {
        if is_busy.get_untracked() {
            return;
        }
        is_busy.set(true);
        error.set(None);
        let email = new_email.get_untracked();
        let password = current_password.get_untracked();
        leptos::task::spawn_local(async move {
            match request_email_change(email, password).await {
                Ok(()) => {
                    is_busy.set(false);
                    awaiting_code.set(true);
                }
                Err(e) => {
                    is_busy.set(false);
                    error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
        });
    };

    let on_confirm = move |_: leptos::ev::MouseEvent| {
        if is_busy.get_untracked() {
            return;
        }
        is_busy.set(true);
        error.set(None);
        let c = code.get_untracked();
        leptos::task::spawn_local(async move {
            match confirm_email_change(c).await {
                Ok(email) => {
                    is_busy.set(false);
                    saved.set(true);
                    awaiting_code.set(false);
                    current_email.set(email);
                    new_email.set(String::new());
                    current_password.set(String::new());
                    code.set(String::new());
                }
                Err(e) => {
                    is_busy.set(false);
                    error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
        });
    };

    let on_back = move |_: leptos::ev::MouseEvent| {
        awaiting_code.set(false);
        error.set(None);
        code.set(String::new());
    };

    view! {
        <div class="form-group">
            <label class="form-label">"Email"</label>
            <p class="dialog-hint">"Current: " {move || current_email.get()}</p>

            {move || saved.get().then(|| view! { <div class="dialog-ok">"Email updated"</div> })}
            {move || error.get().map(|e| view! { <div class="form-error">{e}</div> })}

            <Show
                when=move || !awaiting_code.get()
                fallback=move || view! {
                    <p class="dialog-hint">
                        "We sent a code to " {move || new_email.get()} ". Enter it below to confirm."
                    </p>
                    <input
                        type="text"
                        class="form-input"
                        placeholder="6-digit code"
                        prop:value=move || code.get()
                        on:input=move |ev| code.set(event_target_value(&ev))
                    />
                    <div class="form-actions">
                        <button class="btn-cancel" on:click=on_back disabled=move || is_busy.get()>"Back"</button>
                        <button class="btn-primary" on:click=on_confirm disabled=move || is_busy.get()>
                            {move || if is_busy.get() { "Confirming…" } else { "Confirm" }}
                        </button>
                    </div>
                }
            >
                <input
                    type="email"
                    class="form-input"
                    placeholder="New email address"
                    prop:value=move || new_email.get()
                    on:input=move |ev| new_email.set(event_target_value(&ev))
                />
                <input
                    type="password"
                    class="form-input"
                    placeholder="Current password"
                    prop:value=move || current_password.get()
                    on:input=move |ev| current_password.set(event_target_value(&ev))
                />
                <div class="form-actions">
                    <button
                        class="btn-primary"
                        on:click=on_request
                        disabled=move || is_busy.get() || new_email.get().is_empty() || current_password.get().is_empty()
                    >
                        {move || if is_busy.get() { "Sending…" } else { "Send code" }}
                    </button>
                </div>
            </Show>
        </div>
    }
}

// ── Password ────────────────────────────────────────────────────────────────────

#[component]
fn PasswordSection() -> impl IntoView {
    let current_password = RwSignal::new(String::new());
    let new_password = RwSignal::new(String::new());
    let confirm_password = RwSignal::new(String::new());
    let is_saving = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);
    let saved = RwSignal::new(false);

    let on_save = move |_: leptos::ev::MouseEvent| {
        if is_saving.get_untracked() {
            return;
        }
        error.set(None);
        saved.set(false);
        let current = current_password.get_untracked();
        let new_pw = new_password.get_untracked();
        let confirm = confirm_password.get_untracked();
        if new_pw != confirm {
            error.set(Some(
                "New password and confirmation do not match".to_string(),
            ));
            return;
        }
        is_saving.set(true);
        leptos::task::spawn_local(async move {
            match change_password(current, new_pw).await {
                Ok(()) => {
                    is_saving.set(false);
                    saved.set(true);
                    current_password.set(String::new());
                    new_password.set(String::new());
                    confirm_password.set(String::new());
                }
                Err(e) => {
                    is_saving.set(false);
                    error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
        });
    };

    view! {
        <div class="form-group">
            <label class="form-label">"Password"</label>
            <input
                type="password"
                class="form-input"
                placeholder="Current password"
                prop:value=move || current_password.get()
                on:input=move |ev| current_password.set(event_target_value(&ev))
            />
            <input
                type="password"
                class="form-input"
                placeholder="New password"
                prop:value=move || new_password.get()
                on:input=move |ev| new_password.set(event_target_value(&ev))
            />
            <input
                type="password"
                class="form-input"
                placeholder="Confirm new password"
                prop:value=move || confirm_password.get()
                on:input=move |ev| confirm_password.set(event_target_value(&ev))
            />
            <p class="dialog-hint">"Changing your password signs you out of all other devices."</p>
            {move || error.get().map(|e| view! { <div class="form-error">{e}</div> })}
            {move || saved.get().then(|| view! { <div class="dialog-ok">"Password changed"</div> })}
            <div class="form-actions">
                <button
                    class="btn-primary"
                    on:click=on_save
                    disabled=move || is_saving.get()
                        || current_password.get().is_empty()
                        || new_password.get().is_empty()
                >
                    {move || if is_saving.get() { "Saving…" } else { "Change Password" }}
                </button>
            </div>
        </div>
    }
}
