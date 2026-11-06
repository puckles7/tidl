use leptos::prelude::StoredValue;
use leptos::prelude::*;

use super::UserPrefsCtx;
use crate::api::{save_preferences, UserPrefs};
use crate::util::format_server_fn_error;

pub(super) const TIMEZONES: &[(&str, &str)] = &[
    ("UTC", "UTC"),
    ("America/New_York", "Eastern Time (ET)"),
    ("America/Chicago", "Central Time (CT)"),
    ("America/Denver", "Mountain Time (MT)"),
    ("America/Los_Angeles", "Pacific Time (PT)"),
    ("America/Anchorage", "Alaska Time"),
    ("Pacific/Honolulu", "Hawaii Time"),
    ("America/Sao_Paulo", "Brazil (São Paulo)"),
    ("America/Argentina/Buenos_Aires", "Argentina"),
    ("America/Toronto", "Toronto (ET)"),
    ("America/Vancouver", "Vancouver (PT)"),
    ("Europe/London", "London (GMT/BST)"),
    ("Europe/Paris", "Paris (CET/CEST)"),
    ("Europe/Berlin", "Berlin (CET/CEST)"),
    ("Europe/Rome", "Rome (CET/CEST)"),
    ("Europe/Amsterdam", "Amsterdam (CET/CEST)"),
    ("Europe/Moscow", "Moscow (MSK)"),
    ("Africa/Cairo", "Cairo (EET)"),
    ("Africa/Johannesburg", "Johannesburg (SAST)"),
    ("Asia/Dubai", "Dubai (GST)"),
    ("Asia/Kolkata", "India (IST)"),
    ("Asia/Karachi", "Pakistan (PKT)"),
    ("Asia/Dhaka", "Bangladesh (BST)"),
    ("Asia/Bangkok", "Bangkok (ICT)"),
    ("Asia/Singapore", "Singapore (SGT)"),
    ("Asia/Tokyo", "Tokyo (JST)"),
    ("Asia/Seoul", "Seoul (KST)"),
    ("Asia/Shanghai", "China (CST)"),
    ("Australia/Sydney", "Sydney (AEST/AEDT)"),
    ("Australia/Melbourne", "Melbourne (AEST/AEDT)"),
    ("Pacific/Auckland", "Auckland (NZST/NZDT)"),
];

#[component]
pub(super) fn PreferencesPanel(on_close: impl Fn() + Send + Sync + 'static) -> impl IntoView {
    let on_close =
        StoredValue::new(std::sync::Arc::new(on_close) as std::sync::Arc<dyn Fn() + Send + Sync>);

    let prefs_ctx = use_context::<UserPrefsCtx>();
    let current = prefs_ctx.map(|s| s.get_untracked()).unwrap_or_default();

    let timezone = RwSignal::new(current.timezone.clone());
    let time_format = RwSignal::new(current.time_format.clone());
    let dark_mode = RwSignal::new(current.dark_mode);
    let open_in_editor_cmd = RwSignal::new(current.open_in_editor_cmd.clone().unwrap_or_default());
    let show_sidebar_hints = RwSignal::new(current.show_sidebar_hints);
    let is_saving = RwSignal::new(false);
    let error = RwSignal::new(Option::<String>::None);

    let on_save = move |_: leptos::ev::MouseEvent| {
        if is_saving.get_untracked() {
            return;
        }
        is_saving.set(true);
        error.set(None);
        let tz = timezone.get_untracked();
        let fmt = time_format.get_untracked();
        let dm = dark_mode.get_untracked();
        let oie = open_in_editor_cmd.get_untracked();
        let ssh = show_sidebar_hints.get_untracked();
        let cb = on_close.get_value();
        leptos::task::spawn_local(async move {
            match save_preferences(
                tz.clone(),
                fmt.clone(),
                dm,
                if oie.is_empty() {
                    None
                } else {
                    Some(oie.clone())
                },
                ssh,
            )
            .await
            {
                Ok(()) => {
                    if let Some(ctx) = prefs_ctx {
                        ctx.set(UserPrefs {
                            timezone: tz,
                            time_format: fmt,
                            dark_mode: dm,
                            open_in_editor_cmd: if oie.is_empty() { None } else { Some(oie) },
                            show_sidebar_hints: ssh,
                        });
                    }
                    is_saving.set(false);
                    cb();
                }
                Err(e) => {
                    is_saving.set(false);
                    error.set(Some(format_server_fn_error(&e.to_string())));
                }
            }
        });
    };

    view! {
        {move || error.get().map(|e| view! { <div class="form-error">{e}</div> })}

        <div class="form-group">
            <label class="form-label">"Timezone"</label>
            <select
                class="form-input"
                prop:value=move || timezone.get()
                on:change=move |ev| timezone.set(event_target_value(&ev))
            >
                {TIMEZONES.iter().map(|(id, label)| {
                    let id    = *id;
                    let label = *label;
                    view! { <option value=id>{label}</option> }
                }).collect_view()}
            </select>
        </div>

        <div class="form-group">
            <label class="form-label">"Time Format"</label>
            <div class="settings-radio-group">
                {[
                    ("12h", "12-hour  (1:00 pm)"),
                    ("24h", "24-hour  (13:00)"),
                ].into_iter().map(|(val, label)| {
                    let v1 = val.to_string();
                    let v2 = val.to_string();
                    view! {
                        <label class="settings-radio-label">
                            <input
                                type="radio"
                                name="time_format"
                                value=v1.clone()
                                prop:checked=move || time_format.get() == v1
                                on:change=move |_| time_format.set(v2.clone())
                            />
                            {label}
                        </label>
                    }
                }).collect_view()}
            </div>
        </div>

        <div class="form-group">
            <label class="form-label">"Appearance"</label>
            <label class="settings-toggle-label">
                <input
                    type="checkbox"
                    prop:checked=move || dark_mode.get()
                    on:change=move |ev| {
                        #[cfg(feature = "hydrate")]
                        {
                            use wasm_bindgen::JsCast as _;
                            if let Some(input) = ev.target()
                                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                            {
                                dark_mode.set(input.checked());
                            }
                        }
                        #[cfg(not(feature = "hydrate"))]
                        let _ = ev;
                    }
                />
                "Dark mode"
            </label>
        </div>

        <div class="form-group">
            <label class="form-label">"Sidebar hints"</label>
            <label class="settings-toggle-label">
                <input
                    type="checkbox"
                    prop:checked=move || show_sidebar_hints.get()
                    on:change=move |ev| {
                        #[cfg(feature = "hydrate")]
                        {
                            use wasm_bindgen::JsCast as _;
                            if let Some(input) = ev.target()
                                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
                            {
                                show_sidebar_hints.set(input.checked());
                            }
                        }
                        #[cfg(not(feature = "hydrate"))]
                        let _ = ev;
                    }
                />
                "Show \"click to show/hide\" hint text in sidebars"
            </label>
        </div>

        <div class="dialog-actions">
            <button class="btn-cancel" on:click=move |_| on_close.with_value(|f| f())>"Cancel"</button>
            <button
                class="btn-primary"
                on:click=on_save
                disabled=move || is_saving.get()
            >
                {move || if is_saving.get() { "Saving…" } else { "Save" }}
            </button>
        </div>
    }
}
