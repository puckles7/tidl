use leptos::prelude::StoredValue;
use leptos::prelude::*;

use crate::api::{get_smtp_config, save_smtp_config, test_smtp_send, SmtpConfig};
use crate::util::format_server_fn_error;

#[component]
pub(super) fn SmtpConfigPanel(on_close: impl Fn() + Send + Sync + 'static) -> impl IntoView {
    let on_close =
        StoredValue::new(std::sync::Arc::new(on_close) as std::sync::Arc<dyn Fn() + Send + Sync>);

    let smtp_host = RwSignal::new(String::new());
    let smtp_port = RwSignal::new(587u16);
    let smtp_tls = RwSignal::new("starttls".to_string());
    let smtp_user = RwSignal::new(String::new());
    let smtp_pass = RwSignal::new(String::new());
    let smtp_from_addr = RwSignal::new(String::new());
    let smtp_from_name = RwSignal::new(String::new());
    let smtp_saving = RwSignal::new(false);
    let smtp_error = RwSignal::new(Option::<String>::None);
    let smtp_ok = RwSignal::new(false);
    let smtp_test_addr = RwSignal::new(String::new());
    let smtp_testing = RwSignal::new(false);
    let smtp_test_ok = RwSignal::new(Option::<bool>::None);

    // Load existing SMTP config on mount.
    leptos::task::spawn_local(async move {
        if let Ok(Some(cfg)) = get_smtp_config().await {
            smtp_host.set(cfg.host);
            smtp_port.set(cfg.port);
            smtp_tls.set(cfg.tls_mode);
            smtp_user.set(cfg.username);
            smtp_from_addr.set(cfg.from_address);
            smtp_from_name.set(cfg.from_name);
        }
    });

    let on_save_smtp = move |_: leptos::ev::MouseEvent| {
        if smtp_saving.get_untracked() {
            return;
        }
        smtp_saving.set(true);
        smtp_error.set(None);
        smtp_ok.set(false);
        let cfg = SmtpConfig {
            host: smtp_host.get_untracked(),
            port: smtp_port.get_untracked(),
            tls_mode: smtp_tls.get_untracked(),
            username: smtp_user.get_untracked(),
            password: smtp_pass.get_untracked(),
            from_address: smtp_from_addr.get_untracked(),
            from_name: smtp_from_name.get_untracked(),
        };
        leptos::task::spawn_local(async move {
            match save_smtp_config(cfg).await {
                Ok(()) => {
                    smtp_ok.set(true);
                    smtp_pass.set(String::new());
                }
                Err(e) => {
                    smtp_error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
            smtp_saving.set(false);
        });
    };

    let on_test_smtp = move |_: leptos::ev::MouseEvent| {
        if smtp_testing.get_untracked() {
            return;
        }
        let addr = smtp_test_addr.get_untracked();
        if addr.trim().is_empty() {
            return;
        }
        smtp_testing.set(true);
        smtp_test_ok.set(None);
        smtp_error.set(None);
        leptos::task::spawn_local(async move {
            match test_smtp_send(addr).await {
                Ok(()) => smtp_test_ok.set(Some(true)),
                Err(e) => {
                    smtp_test_ok.set(Some(false));
                    smtp_error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
            smtp_testing.set(false);
        });
    };

    view! {
        {move || smtp_error.get().map(|e| view! { <div class="form-error">{e}</div> })}
        {move || smtp_ok.get().then(|| view! {
            <div class="form-success">"SMTP settings saved."</div>
        })}

        <div class="form-group">
            <label class="form-label">"SMTP Host"</label>
            <input class="form-input" type="text" placeholder="smtp.example.com"
                prop:value=move || smtp_host.get()
                on:input=move |ev| smtp_host.set(event_target_value(&ev)) />
        </div>
        <div class="form-row">
            <div class="form-group form-group-narrow">
                <label class="form-label">"Port"</label>
                <input class="form-input" type="number" min="1" max="65535"
                    prop:value=move || smtp_port.get().to_string()
                    on:input=move |ev| {
                        if let Ok(p) = event_target_value(&ev).parse::<u16>() {
                            smtp_port.set(p);
                        }
                    } />
            </div>
            <div class="form-group form-group-grow">
                <label class="form-label">"Encryption"</label>
                <select class="form-input"
                    prop:value=move || smtp_tls.get()
                    on:change=move |ev| smtp_tls.set(event_target_value(&ev))>
                    <option value="starttls">"STARTTLS (port 587)"</option>
                    <option value="tls">"TLS (port 465)"</option>
                    <option value="none">"None (dev only)"</option>
                </select>
            </div>
        </div>
        <div class="form-group">
            <label class="form-label">"Username"</label>
            <input class="form-input" type="text" autocomplete="off"
                prop:value=move || smtp_user.get()
                on:input=move |ev| smtp_user.set(event_target_value(&ev)) />
        </div>
        <div class="form-group">
            <label class="form-label">"Password"</label>
            <input class="form-input" type="password" autocomplete="new-password"
                placeholder="Leave blank to keep existing"
                prop:value=move || smtp_pass.get()
                on:input=move |ev| smtp_pass.set(event_target_value(&ev)) />
        </div>
        <div class="form-group">
            <label class="form-label">"From Address"</label>
            <input class="form-input" type="email" placeholder="noreply@example.com"
                prop:value=move || smtp_from_addr.get()
                on:input=move |ev| smtp_from_addr.set(event_target_value(&ev)) />
        </div>
        <div class="form-group">
            <label class="form-label">"From Name"</label>
            <input class="form-input" type="text" placeholder="tidl"
                prop:value=move || smtp_from_name.get()
                on:input=move |ev| smtp_from_name.set(event_target_value(&ev)) />
        </div>

        <div class="dialog-actions">
            <button class="btn-cancel" on:click=move |_| on_close.with_value(|f| f())>"Cancel"</button>
            <button class="btn-primary"
                disabled=move || smtp_saving.get()
                on:click=on_save_smtp>
                {move || if smtp_saving.get() { "Saving…" } else { "Save" }}
            </button>
        </div>

        <div class="smtp-test-section">
            <span class="smtp-test-label">"Test:"</span>
            <input class="form-input smtp-test-input" type="email"
                placeholder="you@example.com"
                prop:value=move || smtp_test_addr.get()
                on:input=move |ev| smtp_test_addr.set(event_target_value(&ev)) />
            <button class="btn-secondary"
                disabled=move || smtp_testing.get()
                on:click=on_test_smtp>
                {move || if smtp_testing.get() { "Sending…" } else { "Send test" }}
            </button>
            {move || smtp_test_ok.get().map(|ok| if ok {
                view! { <span class="smtp-test-ok">"Sent!"</span> }.into_any()
            } else {
                view! { <span class="smtp-test-fail">"Failed"</span> }.into_any()
            })}
        </div>
    }
}
