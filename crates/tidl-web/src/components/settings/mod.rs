//! Settings modal — tab shell and panel sub-components.

mod preferences;
use preferences::PreferencesPanel;

mod this_device;
use this_device::ThisDevicePanel;

mod import;
use import::ImportPanel;

mod export;
use export::ExportPanel;

mod smtp;
use smtp::SmtpConfigPanel;

mod users;
use users::UsersPanel;

mod sharing;
use sharing::SharingPanel;

mod app_tokens;
use app_tokens::AppTokensPanel;

mod inbound_email;
use inbound_email::InboundEmailPanel;

mod remote_sources;
use remote_sources::RemoteSourcesPanel;

mod markdown_sources;
use markdown_sources::MarkdownSourcesPanel;

mod curve_editor;
use curve_editor::CurveEditor;

mod priority_curves;
use priority_curves::PriorityCurvesPanel;

mod done;
use done::DonePanel;

use leptos::prelude::*;

use crate::api::{get_calendars, UserPrefs};
use crate::util::mouse_event_targets_self;

/// Context type: a writable signal holding the current user preferences.
pub type UserPrefsCtx = RwSignal<UserPrefs>;

/// Context type: whether the current user has admin privileges.
///
/// Newtype, not a bare `RwSignal<bool>` alias — see `CurrentUsernameCtx`'s doc
/// comment in `app.rs` for why. This one collided with `CalendarLoadedCtx`
/// (also a bare `RwSignal<bool>` alias, provided earlier in the same
/// `AuthenticatedApp` scope), so this signal's later `provide_context` call
/// silently overwrote the loading gate for every descendant.
#[derive(Clone, Copy)]
pub struct IsAdminCtx(pub RwSignal<bool>);

/// Context type: whether `SettingsModal` is open and, if so, which tab id
/// (e.g. `"general"`, `"sharing"`) it should open on. `None` = closed.
///
/// Lets any descendant of `AuthenticatedApp` open Settings directly on a
/// specific tab (backlog #65's sidebar share icon) without threading a
/// callback prop through every intermediate component.
#[derive(Clone, Copy)]
pub struct OpenSettingsCtx(pub RwSignal<Option<&'static str>>);

/// Whether sidebar hint text ("Click a collection to show/hide its …") should
/// be shown — reads the reactive `UserPrefsCtx`, so callers wrapping this in
/// a `move ||` closure re-render when the preference changes. Defaults to
/// `true` when no context is provided (e.g. in tests).
pub fn show_sidebar_hints() -> bool {
    use_context::<UserPrefsCtx>()
        .map(|s| s.get().show_sidebar_hints)
        .unwrap_or(true)
}

pub(super) type CalsResource =
    LocalResource<Result<Vec<crate::api::CalendarInfo>, leptos::server_fn::ServerFnError>>;

#[component]
pub fn SettingsModal(
    on_close: impl Fn() + Send + Sync + 'static,
    /// Tab id to open on, e.g. `"sharing"` — see `OpenSettingsCtx`.
    #[prop(default = "general")]
    initial_tab: &'static str,
) -> impl IntoView {
    let on_close =
        StoredValue::new(std::sync::Arc::new(on_close) as std::sync::Arc<dyn Fn() + Send + Sync>);
    let is_admin = use_context::<IsAdminCtx>()
        .map(|s| s.0.get_untracked())
        .unwrap_or(false);

    let cals_res: CalsResource = LocalResource::new(get_calendars);

    let active_tab = RwSignal::new(initial_tab);
    let backdrop_mousedown = RwSignal::new(false);
    let esc_id = crate::modal_esc::use_escape_close(move || on_close.with_value(|f| f()));

    view! {
        <div class="dialog-backdrop"
            data-esc-id=esc_id.to_string()
            on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
            on:click=move |_| if backdrop_mousedown.get() { on_close.with_value(|f| f()) }
        >
            <div class="dialog settings-dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                <div class="dialog-header">
                    <h2 class="dialog-title">"Settings"</h2>
                    <button class="dialog-close" on:click=move |_| on_close.with_value(|f| f())>"✕"</button>
                </div>

                // ── Tab bar ───────────────────────────────────────────────────
                {
                    let tab_scroll_ref: NodeRef<leptos::html::Div> = NodeRef::new();
                    let scroll_left = {
                        let _r = tab_scroll_ref;
                        move |_: leptos::ev::MouseEvent| {
                            #[cfg(target_arch = "wasm32")]
                            if let Some(el) = _r.get() {
                                let _ = el.scroll_by_with_x_and_y(-120.0, 0.0);
                            }
                        }
                    };
                    let scroll_right = {
                        let _r = tab_scroll_ref;
                        move |_: leptos::ev::MouseEvent| {
                            #[cfg(target_arch = "wasm32")]
                            if let Some(el) = _r.get() {
                                let _ = el.scroll_by_with_x_and_y(120.0, 0.0);
                            }
                        }
                    };
                    // Deep-linking to a non-default tab (e.g. the sidebar
                    // share icon opening straight to "sharing") lands on a
                    // tab button that may be scrolled out of view in this
                    // horizontally-scrolling strip — scroll it into view
                    // once the strip has mounted. Runs once (not on every
                    // later tab click): it doesn't read `active_tab`, only
                    // `tab_scroll_ref`, which goes from `None` to `Some`
                    // exactly once.
                    Effect::new(move |_| {
                        #[cfg(target_arch = "wasm32")]
                        if let Some(container) = tab_scroll_ref.get() {
                            if let Ok(Some(active_el)) = container.query_selector(".settings-tab.active") {
                                let opts = web_sys::ScrollIntoViewOptions::new();
                                opts.set_inline(web_sys::ScrollLogicalPosition::Center);
                                opts.set_block(web_sys::ScrollLogicalPosition::Nearest);
                                active_el.scroll_into_view_with_scroll_into_view_options(&opts);
                            }
                        }
                    });
                    view! {
                        <div class="settings-tabs-wrapper">
                            <button class="settings-tabs-arrow" on:click=scroll_left>"‹"</button>
                            <div class="settings-tabs" node_ref=tab_scroll_ref>
                                <button
                                    class="settings-tab"
                                    class=("active", move || active_tab.get() == "general")
                                    on:click=move |_| active_tab.set("general")
                                >"General"</button>
                                <button
                                    class="settings-tab"
                                    class=("active", move || active_tab.get() == "this_device")
                                    on:click=move |_| active_tab.set("this_device")
                                >"This Device"</button>
                                <button
                                    class="settings-tab"
                                    class=("active", move || active_tab.get() == "priority_curves")
                                    on:click=move |_| active_tab.set("priority_curves")
                                >"Priority Curves"</button>
                                <button
                                    class="settings-tab"
                                    class=("active", move || active_tab.get() == "import")
                                    on:click=move |_| active_tab.set("import")
                                >"Import"</button>
                                <button
                                    class="settings-tab"
                                    class=("active", move || active_tab.get() == "export")
                                    on:click=move |_| active_tab.set("export")
                                >"Export"</button>
                                {is_admin.then(|| view! {
                                    <button
                                        class="settings-tab"
                                        class=("active", move || active_tab.get() == "email")
                                        on:click=move |_| active_tab.set("email")
                                    >"Email"</button>
                                })}
                                {is_admin.then(|| view! {
                                    <button
                                        class="settings-tab"
                                        class=("active", move || active_tab.get() == "users")
                                        on:click=move |_| active_tab.set("users")
                                    >"Users"</button>
                                })}
                                <button
                                    class="settings-tab"
                                    class=("active", move || active_tab.get() == "inbound_email")
                                    on:click=move |_| active_tab.set("inbound_email")
                                >"Inbound Email"</button>
                                <button
                                    class="settings-tab"
                                    class=("active", move || active_tab.get() == "sharing")
                                    on:click=move |_| active_tab.set("sharing")
                                >"Sharing"</button>
                                <button
                                    class="settings-tab"
                                    class=("active", move || active_tab.get() == "tokens")
                                    on:click=move |_| active_tab.set("tokens")
                                >"App Tokens"</button>
                                <button
                                    class="settings-tab"
                                    class=("active", move || active_tab.get() == "remote")
                                    on:click=move |_| active_tab.set("remote")
                                >"CalDAV Sync"</button>
                                <button
                                    class="settings-tab"
                                    class=("active", move || active_tab.get() == "markdown")
                                    on:click=move |_| active_tab.set("markdown")
                                >"Markdown Sources"</button>
                                <button
                                    class="settings-tab"
                                    class=("active", move || active_tab.get() == "done")
                                    on:click=move |_| active_tab.set("done")
                                >"Done"</button>
                            </div>
                            <button class="settings-tabs-arrow" on:click=scroll_right>"›"</button>
                        </div>
                    }
                }

                <div class="dialog-form">

                    // ── General tab ───────────────────────────────────────────
                    <Show when=move || active_tab.get() == "general">
                        <PreferencesPanel on_close=move || on_close.with_value(|f| f()) />
                    </Show>

                    // ── This Device tab ───────────────────────────────────────
                    <Show when=move || active_tab.get() == "this_device">
                        <ThisDevicePanel on_close=move || on_close.with_value(|f| f()) />
                    </Show>

                    // ── Priority Curves tab ──────────────────────────────────
                    <Show when=move || active_tab.get() == "priority_curves">
                        <PriorityCurvesPanel on_close=move || on_close.with_value(|f| f()) />
                    </Show>

                    // ── Import tab ────────────────────────────────────────────
                    <Show when=move || active_tab.get() == "import">
                        <ImportPanel cals_res=cals_res />
                    </Show>

                    // ── Export tab ────────────────────────────────────────────
                    <Show when=move || active_tab.get() == "export">
                        <ExportPanel cals_res=cals_res />
                    </Show>

                    // ── Email tab (admin only) ────────────────────────────────
                    <Show when=move || is_admin && active_tab.get() == "email">
                        <SmtpConfigPanel on_close=move || on_close.with_value(|f| f()) />
                    </Show>

                    // ── Users tab (admin only) ────────────────────────────────
                    <Show when=move || is_admin && active_tab.get() == "users">
                        <UsersPanel />
                    </Show>

                    // ── Inbound Email tab ─────────────────────────────────────
                    <Show when=move || active_tab.get() == "inbound_email">
                        <InboundEmailPanel />
                    </Show>

                    // ── Sharing tab ───────────────────────────────────────────
                    <Show when=move || active_tab.get() == "sharing">
                        <SharingPanel cals_res=cals_res />
                    </Show>

                    // ── App Tokens tab ────────────────────────────────────────
                    <Show when=move || active_tab.get() == "tokens">
                        <AppTokensPanel />
                    </Show>

                    // ── Remote Sources tab ────────────────────────────────────
                    <Show when=move || active_tab.get() == "remote">
                        <RemoteSourcesPanel cals_res=cals_res />
                    </Show>

                    // ── Markdown Sources tab ──────────────────────────────────
                    <Show when=move || active_tab.get() == "markdown">
                        <MarkdownSourcesPanel />
                    </Show>

                    // ── Done tab ──────────────────────────────────────────────
                    <Show when=move || active_tab.get() == "done">
                        <DonePanel on_close=move || on_close.with_value(|f| f()) />
                    </Show>

                </div>
            </div>
        </div>
    }
}
