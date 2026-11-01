use leptos::prelude::*;

use crate::api::{
    add_trusted_sender, create_inbound_email_token, get_calendars, list_inbound_email_tokens,
    list_trusted_senders, remove_trusted_sender, revoke_inbound_email_token, CalendarInfo,
    InboundEmailTokenInfo, TrustedSenderInfo,
};
use crate::util::format_server_fn_error;

#[component]
pub(super) fn InboundEmailPanel() -> impl IntoView {
    // ── Inbound email tokens ──────────────────────────────────────────────────

    let tokens_list = RwSignal::new(Vec::<InboundEmailTokenInfo>::new());
    let tokens_loading = RwSignal::new(true);
    let token_label = RwSignal::new(String::new());
    let token_cal_id = RwSignal::new(0i64);
    let token_creating = RwSignal::new(false);
    let token_error = RwSignal::new(Option::<String>::None);
    let new_token_info = RwSignal::new(Option::<InboundEmailTokenInfo>::None);
    let copy_hint = RwSignal::new(Option::<String>::None);

    // ── Calendar list for the dropdown ────────────────────────────────────────

    let cals_list = RwSignal::new(Vec::<CalendarInfo>::new());
    let cals_loading = RwSignal::new(true);

    leptos::task::spawn_local(async move {
        if let Ok(list) = get_calendars().await {
            // Pre-select the first calendar so the form is valid by default.
            if let Some(first) = list.first() {
                token_cal_id.set(first.id);
            }
            cals_list.set(list);
        }
        cals_loading.set(false);
    });

    // ── Trusted senders ───────────────────────────────────────────────────────

    let senders_list = RwSignal::new(Vec::<TrustedSenderInfo>::new());
    let senders_loading = RwSignal::new(true);
    let sender_email = RwSignal::new(String::new());
    let sender_adding = RwSignal::new(false);
    let sender_error = RwSignal::new(Option::<String>::None);

    // ── Initial loads ─────────────────────────────────────────────────────────

    leptos::task::spawn_local(async move {
        if let Ok(list) = list_inbound_email_tokens().await {
            tokens_list.set(list);
        }
        tokens_loading.set(false);
    });

    leptos::task::spawn_local(async move {
        if let Ok(list) = list_trusted_senders().await {
            senders_list.set(list);
        }
        senders_loading.set(false);
    });

    // ── Reload helpers ────────────────────────────────────────────────────────

    let reload_tokens = move || {
        tokens_loading.set(true);
        leptos::task::spawn_local(async move {
            if let Ok(list) = list_inbound_email_tokens().await {
                tokens_list.set(list);
            }
            tokens_loading.set(false);
        });
    };

    let reload_senders = move || {
        senders_loading.set(true);
        leptos::task::spawn_local(async move {
            if let Ok(list) = list_trusted_senders().await {
                senders_list.set(list);
            }
            senders_loading.set(false);
        });
    };

    // ── Create inbound email token ────────────────────────────────────────────

    let on_create_token = move |_: leptos::ev::MouseEvent| {
        let label = token_label.get_untracked();
        if label.trim().is_empty() {
            return;
        }
        let cal_id = token_cal_id.get_untracked();
        if cal_id == 0 {
            return;
        }
        if token_creating.get_untracked() {
            return;
        }
        token_creating.set(true);
        token_error.set(None);
        new_token_info.set(None);
        leptos::task::spawn_local(async move {
            match create_inbound_email_token(cal_id, label).await {
                Ok(info) => {
                    new_token_info.set(Some(info));
                    token_label.set(String::new());
                    reload_tokens();
                }
                Err(e) => {
                    token_error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
            token_creating.set(false);
        });
    };

    // ── Add trusted sender ────────────────────────────────────────────────────

    let on_add_sender = move |_: leptos::ev::MouseEvent| {
        let email = sender_email.get_untracked();
        if email.trim().is_empty() {
            return;
        }
        if sender_adding.get_untracked() {
            return;
        }
        sender_adding.set(true);
        sender_error.set(None);
        leptos::task::spawn_local(async move {
            match add_trusted_sender(email).await {
                Ok(()) => {
                    sender_email.set(String::new());
                    reload_senders();
                }
                Err(e) => {
                    sender_error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
            sender_adding.set(false);
        });
    };

    view! {
        // ── Panel 1: Inbound Email Tokens ─────────────────────────────────────
        <h3 class="settings-section-heading">"Inbound Email Tokens"</h3>
        <p class="sharing-empty">
            "Forward meeting invitations to your personal inbound address to import them automatically."
        </p>

        {move || token_error.get().map(|e| view! { <div class="form-error">{e}</div> })}

        {move || new_token_info.get().map(|info| {
            let addr = info.address.clone();
            view! {
                <div class="form-group">
                    <label class="form-label">"Your new inbound address (copy now — shown once):"</label>
                    <div class="token-display">
                        <code class="token-value">{addr.clone()}</code>
                        <button
                            class="btn-secondary"
                            on:click={
                                let _addr2 = addr.clone();
                                move |_| {
                                    #[cfg(feature = "hydrate")]
                                    {
                                        let _addr3 = _addr2.clone();
                                        leptos::task::spawn_local(async move {
                                            if crate::util::copy_to_clipboard(&_addr3).await {
                                                copy_hint.set(None);
                                            } else {
                                                copy_hint.set(Some(
                                                    "Couldn't copy automatically — \
                                                     select and copy the address above manually.".to_string(),
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
            }
        })}

        <div class="form-group">
            <label class="form-label">"Label"</label>
            <input
                type="text"
                class="form-input"
                placeholder="e.g. Work invites"
                prop:value=move || token_label.get()
                on:input=move |ev| token_label.set(event_target_value(&ev))
            />
        </div>
        <div class="form-group">
            <label class="form-label">"Calendar"</label>
            <select
                class="form-input"
                prop:value=move || token_cal_id.get().to_string()
                on:change=move |ev| {
                    if let Ok(v) = event_target_value(&ev).parse::<i64>() {
                        token_cal_id.set(v);
                    }
                }
            >
                <option value="0" disabled=true>"— pick a calendar —"</option>
                {move || {
                    cals_list.get().into_iter().map(|cal| {
                        let id_str = cal.id.to_string();
                        let display = cal.display_name.clone();
                        view! { <option value=id_str>{display}</option> }
                    }).collect_view()
                }}
            </select>
        </div>
        <button
            class="btn-primary"
            disabled=move || {
                token_creating.get()
                    || token_label.get().trim().is_empty()
                    || token_cal_id.get() == 0
                    || cals_loading.get()
            }
            on:click=on_create_token
        >
            {move || if token_creating.get() { "Creating…" } else { "Create" }}
        </button>

        <h3 class="settings-section-heading">"Existing addresses"</h3>
        {move || {
            let list = tokens_list.get();
            if tokens_loading.get() {
                return view! { <p class="sharing-empty">"Loading…"</p> }.into_any();
            }
            if list.is_empty() {
                return view! { <p class="sharing-empty">"No inbound email addresses yet."</p> }.into_any();
            }
            list.into_iter().map(|tok| {
                let id = tok.id.clone();
                let address = tok.address.clone();
                view! {
                    <div class="sharing-row">
                        <span class="sharing-grantee">
                            {tok.label.clone()}
                            " → "
                            {tok.calendar_name.clone()}
                        </span>
                        <code
                            class="token-value"
                            title=address.clone()
                            style="max-width:20ch;overflow:hidden;text-overflow:ellipsis;white-space:nowrap"
                        >{address.clone()}</code>
                        <span class="sharing-perm">
                            {tok.last_used_at.map(|t| format!("last used {}", t))
                                .unwrap_or_else(|| format!("created {}", tok.created_at))}
                        </span>
                        <button
                            class="btn-danger-small"
                            on:click=move |_| {
                                let id2 = id.clone();
                                leptos::task::spawn_local(async move {
                                    let _ = revoke_inbound_email_token(id2).await;
                                    reload_tokens();
                                });
                            }
                        >"Revoke"</button>
                    </div>
                }
            }).collect_view().into_any()
        }}

        // ── Panel 2: Trusted Sender Addresses ─────────────────────────────────
        <h3 class="settings-section-heading">"Trusted Sender Addresses"</h3>
        <p class="sharing-empty">
            "Only emails from these addresses will be accepted."
        </p>

        {move || sender_error.get().map(|e| view! { <div class="form-error">{e}</div> })}

        <div class="form-group">
            <label class="form-label">"Email address"</label>
            <input
                type="email"
                class="form-input"
                placeholder="you@example.com"
                prop:value=move || sender_email.get()
                on:input=move |ev| sender_email.set(event_target_value(&ev))
            />
        </div>
        <button
            class="btn-primary"
            disabled=move || sender_adding.get() || sender_email.get().trim().is_empty()
            on:click=on_add_sender
        >
            {move || if sender_adding.get() { "Adding…" } else { "Add" }}
        </button>

        <h3 class="settings-section-heading">"Trusted senders"</h3>
        {move || {
            let list = senders_list.get();
            if senders_loading.get() {
                return view! { <p class="sharing-empty">"Loading…"</p> }.into_any();
            }
            if list.is_empty() {
                return view! { <p class="sharing-empty">"No trusted senders yet."</p> }.into_any();
            }
            list.into_iter().map(|sender| {
                let id = sender.id.clone();
                view! {
                    <div class="sharing-row">
                        <span class="sharing-grantee">{sender.email.clone()}</span>
                        <span class="sharing-perm">"added " {sender.created_at.clone()}</span>
                        <button
                            class="btn-danger-small"
                            on:click=move |_| {
                                let id2 = id.clone();
                                leptos::task::spawn_local(async move {
                                    let _ = remove_trusted_sender(id2).await;
                                    reload_senders();
                                });
                            }
                        >"Remove"</button>
                    </div>
                }
            }).collect_view().into_any()
        }}
    }
}
