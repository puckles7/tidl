//! Shared sidebar toggle row — calendars, markdown sources, note collections.
//!
//! A single row: colour dot, bold coloured name, then icon buttons (sync,
//! share, edit — in that fixed order) shown only when the caller supplies
//! the matching callback, then an on/off toggle switch at the right end.
//! `on_edit`/`on_sync`/`on_share` being `None` only omits that specific
//! button. Edit is anchored immediately before the toggle (rather than
//! sync/edit/share in a fixed left-to-right order) because every entry has
//! edit but not every entry has sync/share — anchoring edit to the toggle
//! (both always present) keeps it in a visually stable spot without
//! reserving empty space for the icons that vary entry to entry.

use leptos::prelude::*;

#[component]
pub fn SidebarPill(
    display_name: String,
    color: String,
    visible: bool,
    on_toggle: Callback<()>,
    /// Show the coloured dot at the start of the row.
    #[prop(default = true)]
    show_dot: bool,
    /// If `Some`, show an edit icon button that calls this callback.
    #[prop(default = None)]
    on_edit: Option<Callback<()>>,
    /// If `Some`, show a sync/refresh icon button that calls this callback.
    #[prop(default = None)]
    on_sync: Option<Callback<()>>,
    /// While `true`, the sync button shows a spinner instead of its icon and
    /// is disabled — mirrors the `.sync-badge`/`.sync-spinner` treatment used
    /// in the settings modal's source lists. A `Signal` (not a plain `bool`)
    /// so toggling it only re-renders this button, not the whole row.
    #[prop(default = Signal::derive(|| false))]
    syncing: Signal<bool>,
    /// If `Some`, show a share icon button that calls this callback (opens
    /// Settings on the Sharing tab — backlog #65).
    #[prop(default = None)]
    on_share: Option<Callback<()>>,
    /// Tooltip for the share button — describes who it's shared with/by.
    #[prop(default = String::new())]
    share_title: String,
) -> impl IntoView {
    // The toggle switch's on-colour, and the row's own custom property so
    // CSS can read it without a static per-kind colour living in the
    // stylesheet (mirrors the `--accent` pattern used elsewhere in the app).
    let accent_style = format!("--accent:{};", color);
    let name_style = format!("color:{};", color);
    let dot_style = format!("background:{};", color);

    let sync_btn = on_sync.map(|sync_cb| {
        view! {
            <button
                type="button"
                class="sidebar-pill-icon-btn"
                title=move || if syncing.get() { "Syncing…" } else { "Sync now" }
                disabled=move || syncing.get()
                on:click=move |ev: leptos::web_sys::MouseEvent| {
                    ev.stop_propagation();
                    sync_cb.run(());
                }
            >
                {move || if syncing.get() {
                    view! { <span class="sync-spinner"></span> }.into_any()
                } else {
                    view! {
                        <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                            <polyline points="23 4 23 10 17 10"></polyline>
                            <polyline points="1 20 1 14 7 14"></polyline>
                            <path d="M3.51 9a9 9 0 0 1 14.85-3.36L23 10M1 14l4.64 4.36A9 9 0 0 0 20.49 15"></path>
                        </svg>
                    }.into_any()
                }}
            </button>
        }
    });
    let share_btn = on_share.map(|share_cb| {
        view! {
            <button
                type="button"
                class="sidebar-pill-icon-btn"
                title=share_title.clone()
                on:click=move |ev: leptos::web_sys::MouseEvent| {
                    ev.stop_propagation();
                    share_cb.run(());
                }
            >
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <circle cx="18" cy="5" r="3"></circle>
                    <circle cx="6" cy="12" r="3"></circle>
                    <circle cx="18" cy="19" r="3"></circle>
                    <line x1="8.59" y1="13.51" x2="15.42" y2="17.49"></line>
                    <line x1="15.41" y1="6.51" x2="8.59" y2="10.49"></line>
                </svg>
            </button>
        }
    });
    let edit_btn = on_edit.map(|edit_cb| {
        view! {
            <button
                type="button"
                class="sidebar-pill-icon-btn"
                title="Edit"
                on:click=move |ev: leptos::web_sys::MouseEvent| {
                    ev.stop_propagation();
                    edit_cb.run(());
                }
            >
                <svg width="15" height="15" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M17 3a2.85 2.83 0 1 1 4 4L7.5 20.5 2 22l1.5-5.5Z"></path>
                </svg>
            </button>
        }
    });

    view! {
        <div class="sidebar-pill" style=accent_style>
            {show_dot.then(|| view! {
                <span class="sidebar-pill-dot" style=dot_style></span>
            })}
            <span class="sidebar-pill-name" style=name_style title=display_name.clone()>{display_name.clone()}</span>
            // Icon buttons + toggle sit in their own tightly-packed cluster
            // (rather than spread across the row's own gap) so the name gets
            // as much of the row's width as possible.
            <div class="sidebar-pill-controls">
                {sync_btn}
                {share_btn}
                {edit_btn}
                <label class="sidebar-pill-toggle">
                    <input
                        type="checkbox"
                        class="sidebar-pill-toggle-input"
                        prop:checked=visible
                        aria-label=format!("Show {display_name}")
                        on:change=move |_| on_toggle.run(())
                    />
                    <span class="sidebar-pill-track"><span class="sidebar-pill-thumb"></span></span>
                </label>
            </div>
        </div>
    }
}
