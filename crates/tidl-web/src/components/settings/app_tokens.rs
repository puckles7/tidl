use leptos::prelude::*;

use crate::api::{create_app_token, list_app_tokens, revoke_app_token, AppTokenInfo};
use crate::util::format_server_fn_error;

#[component]
pub(super) fn AppTokensPanel() -> impl IntoView {
    let tokens_list = RwSignal::new(Vec::<AppTokenInfo>::new());
    let tokens_loading = RwSignal::new(true);
    let token_name = RwSignal::new(String::new());
    let token_readonly = RwSignal::new(false);
    let token_creating = RwSignal::new(false);
    let token_error = RwSignal::new(Option::<String>::None);
    let new_token_value = RwSignal::new(Option::<String>::None);
    let copy_hint = RwSignal::new(Option::<String>::None);

    // Load tokens on mount.
    leptos::task::spawn_local(async move {
        if let Ok(list) = list_app_tokens().await {
            tokens_list.set(list);
        }
        tokens_loading.set(false);
    });

    let reload_tokens = move || {
        tokens_loading.set(true);
        leptos::task::spawn_local(async move {
            if let Ok(list) = list_app_tokens().await {
                tokens_list.set(list);
            }
            tokens_loading.set(false);
        });
    };

    let on_create_token = move |_: leptos::ev::MouseEvent| {
        let name = token_name.get_untracked();
        if name.trim().is_empty() {
            return;
        }
        if token_creating.get_untracked() {
            return;
        }
        token_creating.set(true);
        token_error.set(None);
        new_token_value.set(None);
        let ro = token_readonly.get_untracked();
        leptos::task::spawn_local(async move {
            match create_app_token(name, ro).await {
                Ok(tok) => {
                    new_token_value.set(Some(tok));
                    token_name.set(String::new());
                    reload_tokens();
                }
                Err(e) => {
                    token_error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
            token_creating.set(false);
        });
    };

    view! {
        <h3 class="settings-section-heading">"Create new token"</h3>
        <p class="sharing-empty">
            "App tokens let other apps (or another tidl instance) access your CalDAV calendars without using your password. The token is shown only once — copy it immediately."
        </p>

        {move || token_error.get().map(|e| view! { <div class="form-error">{e}</div> })}
        {move || new_token_value.get().map(|tok| view! {
            <div class="form-group">
                <label class="form-label">"Your new token (copy now — shown once):"</label>
                <div class="token-display">
                    <code class="token-value">{tok.clone()}</code>
                    <button
                        class="btn-secondary"
                        on:click={
                            let _tok2 = tok.clone();
                            move |_| {
                                #[cfg(feature = "hydrate")]
                                {
                                    let _tok3 = _tok2.clone();
                                    leptos::task::spawn_local(async move {
                                        if crate::util::copy_to_clipboard(&_tok3).await {
                                            copy_hint.set(None);
                                        } else {
                                            copy_hint.set(Some(
                                                "Couldn't copy automatically — \
                                                 select and copy the token above manually.".to_string(),
                                            ));
                                        }
                                    });
                                }
                            }
                        }
                    >"Copy"</button>
                </div>
                {move || copy_hint.get().map(|h| view! { <p class="dialog-hint">{h}</p> })}
            </div>
        })}

        <div class="form-group">
            <label class="form-label">"Token name"</label>
            <input
                type="text"
                class="form-input"
                placeholder="e.g. Work Laptop Sync"
                prop:value=move || token_name.get()
                on:input=move |ev| token_name.set(event_target_value(&ev))
            />
        </div>
        <div class="form-group">
            <label class="settings-toggle-label">
                <input
                    type="checkbox"
                    prop:checked=move || token_readonly.get()
                    on:change=move |_ev| {
                        #[cfg(feature = "hydrate")]
                        {
                            use wasm_bindgen::JsCast as _;
                            if let Some(input) = _ev.target()
                                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                            {
                                token_readonly.set(input.checked());
                            }
                        }
                    }
                />
                " Read-only (no writes)"
            </label>
        </div>
        <button
            class="btn-primary"
            disabled=move || token_creating.get() || token_name.get().trim().is_empty()
            on:click=on_create_token
        >
            {move || if token_creating.get() { "Creating…" } else { "Create token" }}
        </button>

        <h3 class="settings-section-heading">"Existing tokens"</h3>
        {move || {
            let list = tokens_list.get();
            if tokens_loading.get() {
                return view! { <p class="sharing-empty">"Loading…"</p> }.into_any();
            }
            if list.is_empty() {
                return view! { <p class="sharing-empty">"No tokens yet."</p> }.into_any();
            }
            list.into_iter().map(|tok| {
                let id = tok.id.clone();
                view! {
                    <div class="sharing-row">
                        <span class="sharing-grantee">
                            {tok.name.clone()}
                            {if tok.read_only { " (read-only)" } else { " (read-write)" }}
                        </span>
                        <span class="sharing-perm">"created " {tok.created_at.clone()}</span>
                        <button
                            class="btn-danger-small"
                            on:click=move |_| {
                                let id2 = id.clone();
                                leptos::task::spawn_local(async move {
                                    let _ = revoke_app_token(id2).await;
                                    reload_tokens();
                                });
                            }
                        >"Revoke"</button>
                    </div>
                }
            }).collect_view().into_any()
        }}
    }
}
