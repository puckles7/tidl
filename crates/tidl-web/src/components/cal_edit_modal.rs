//! Calendar and Markdown-source edit modals.
//!
//! Both modals follow the same design pattern:
//! - Create mode: all fields blank, single "Create" action.
//! - Edit mode:   fields pre-filled; additionally exposes Share, Delete,
//!   Refresh (webcal) and Unsubscribe actions.

use leptos::prelude::*;
use std::sync::Arc;

use crate::api::{
    add_git_source, add_markdown_source, clear_shared_calendar, create_calendar_api,
    delete_calendar_api, delete_markdown_source, get_calendar_shares, is_enc_key_configured,
    refresh_webcal_calendar_api, rename_calendar_api, revoke_calendar_share,
    set_calendar_sync_url_api, set_shared_calendar_color, share_calendar,
    update_calendar_color_api, update_git_source, update_markdown_source, CalendarInfo,
    MarkdownSourceInfo, NewGitSource, NewMarkdownSource, ShareInfo, MIN_SYNC_INTERVAL_SECS,
};
use crate::components::dir_picker::DirPickerModal;
use crate::util::{format_server_fn_error, mouse_event_targets_self};

/// Calendar colour palette (backlog #127) — the base hue set (6 hues, 60°
/// apart). Every other entity type uses the same base hue-rotated by 15°, so
/// calendars have full internal contrast without matching another type.
///
/// Values are CSS variable references so light/dark palette variants are
/// applied automatically via the `--pal-cal-N` custom properties in style.css.
pub const CALENDAR_PALETTE: &[&str] = &[
    "var(--pal-cal-1)",
    "var(--pal-cal-2)",
    "var(--pal-cal-3)",
    "var(--pal-cal-4)",
    "var(--pal-cal-5)",
    "var(--pal-cal-6)",
];

/// Markdown-source colour palette (backlog #127) — the base palette rotated
/// +15°. Disjoint from every other type's rotated set.
pub const MARKDOWN_PALETTE: &[&str] = &[
    "var(--pal-md-1)",
    "var(--pal-md-2)",
    "var(--pal-md-3)",
    "var(--pal-md-4)",
    "var(--pal-md-5)",
    "var(--pal-md-6)",
];

// ── CalendarEditModal ─────────────────────────────────────────────────────────

/// A calendar shared *with* the current user by someone else — the other
/// shape `CalendarEditModal` can edit, alongside `CalendarInfo` for owned
/// calendars. Kept separate from `CalendarInfo` (rather than reusing it with
/// blank/sentinel fields) because the two have almost no overlapping
/// semantics: `id`/`read_only`/`sync_url` etc. describe the owner's actual
/// row and don't apply to a grantee's view of it.
#[derive(Clone)]
pub struct SharedCalendarEditInfo {
    pub owner_username: String,
    pub calendar_name: String,
    pub display_name: String,
    pub color: String,
    /// "read" or "read-write", as stored in `calendar_shares.permission`.
    pub permission: String,
}

/// Combined create / edit / subscribe modal for calendars.
///
/// - `cal = None, shared = None`    → create mode (blank fields; webcal URL optional).
/// - `cal = Some`                   → edit an owned calendar (rename, color, webcal
///   URL, Share, Delete/Unsubscribe).
/// - `shared = Some`                → edit a calendar shared *with* the current
///   user: only their own display color is actually theirs to change — name
///   and Share are shown but disabled (they belong to the owner), and
///   "Clear all events" only appears under a `read-write` grant, since that
///   deletes the owner's real events, not just this grantee's view.
#[component]
pub fn CalendarEditModal(
    /// `None` to create; `Some(cal)` to edit an existing owned calendar.
    cal: Option<CalendarInfo>,
    /// `Some(info)` to edit a calendar shared with the current user instead.
    /// Mutually exclusive with `cal` — at most one of the two is `Some`.
    shared: Option<SharedCalendarEditInfo>,
    on_close: impl Fn() + Send + Sync + 'static,
    on_refresh: Callback<()>,
) -> impl IntoView {
    let on_close = StoredValue::new(Arc::new(on_close) as Arc<dyn Fn() + Send + Sync>);

    let is_shared = shared.is_some();
    let is_edit = cal.is_some() || is_shared;
    let is_webcal = cal
        .as_ref()
        .map(|c| c.read_only && c.sync_url.is_some())
        .unwrap_or(false);
    let is_personal = cal.as_ref().map(|c| c.name == "personal").unwrap_or(false);
    let can_write_shared = shared
        .as_ref()
        .map(|s| s.permission == "read-write")
        .unwrap_or(false);

    let cal_name_sv = StoredValue::new(
        cal.as_ref()
            .map(|c| c.name.clone())
            .or_else(|| shared.as_ref().map(|s| s.calendar_name.clone()))
            .unwrap_or_default(),
    );
    let shared_owner_sv = StoredValue::new(shared.as_ref().map(|s| s.owner_username.clone()));

    // ── Form state ────────────────────────────────────────────────────────────
    let default_color = cal
        .as_ref()
        .map(|c| c.color.clone())
        .or_else(|| shared.as_ref().map(|s| s.color.clone()))
        .unwrap_or_else(|| CALENDAR_PALETTE[0].to_string());
    let name_val = RwSignal::new(
        cal.as_ref()
            .map(|c| c.display_name.clone())
            .or_else(|| shared.as_ref().map(|s| s.display_name.clone()))
            .unwrap_or_default(),
    );
    let color_val = RwSignal::new(default_color);
    let url_val = RwSignal::new(
        cal.as_ref()
            .and_then(|c| c.sync_url.clone())
            .unwrap_or_default(),
    );

    let saving = RwSignal::new(false);
    let error_msg = RwSignal::<Option<String>>::new(None);

    // Webcal-specific state (edit only).
    let refreshing = RwSignal::new(false);
    let sync_error_sv = StoredValue::new(cal.as_ref().and_then(|c| c.sync_error.clone()));
    let last_synced_sv = StoredValue::new(cal.as_ref().and_then(|c| c.last_synced_at.clone()));

    // ── Clear-all-events dialog — shown in place of Delete for a personal
    // calendar, or for a shared one under a `read-write` grant. ──────────────
    let clear_all_open = RwSignal::new(false);
    let clear_all_busy = RwSignal::new(false);

    // ── Sub-modal: share dialog ───────────────────────────────────────────────
    let share_open: RwSignal<bool> = RwSignal::new(false);

    // ── Save handler ──────────────────────────────────────────────────────────
    let save = {
        move |_: leptos::ev::MouseEvent| {
            let name = name_val.get_untracked();
            let color = color_val.get_untracked();
            let url = url_val.get_untracked();

            if name.trim().is_empty() {
                error_msg.set(Some("Name is required.".into()));
                return;
            }
            saving.set(true);
            error_msg.set(None);

            let on_close = on_close;
            let on_refresh = on_refresh;

            if is_shared {
                // Name/URL aren't ours to change — only the grantee's own
                // display color, via `set_shared_calendar_color`.
                let owner = shared_owner_sv.get_value().unwrap_or_default();
                let cal_name = cal_name_sv.get_value();
                leptos::task::spawn_local(async move {
                    match set_shared_calendar_color(owner, cal_name, color).await {
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
            } else if is_edit {
                // Editing existing calendar: rename + color + (optional) sync URL.
                let cal_name = cal_name_sv.get_value();
                leptos::task::spawn_local(async move {
                    let r1 = rename_calendar_api(cal_name.clone(), name.trim().to_string()).await;
                    let r2 = update_calendar_color_api(cal_name.clone(), color.clone()).await;
                    // For webcal calendars, also update the sync URL.
                    let r3 = if is_webcal {
                        let url_opt = if url.trim().is_empty() {
                            None
                        } else {
                            Some(url.trim().to_string())
                        };
                        set_calendar_sync_url_api(cal_name, url_opt).await
                    } else {
                        Ok(())
                    };
                    saving.set(false);
                    if let Err(e) = r1.or(r2).or(r3) {
                        error_msg.set(Some(format_server_fn_error(&e.to_string())));
                        return;
                    }
                    on_close.get_value()();
                    on_refresh.run(());
                });
            } else if !url.trim().is_empty() {
                // Subscribe to webcal feed.
                let on_refresh2 = on_refresh;
                leptos::task::spawn_local(async move {
                    match create_calendar_api(name.trim().to_string(), color).await {
                        Ok(slug) => {
                            match set_calendar_sync_url_api(
                                slug.clone(),
                                Some(url.trim().to_string()),
                            )
                            .await
                            {
                                Ok(()) => {
                                    let _ = refresh_webcal_calendar_api(slug).await;
                                    saving.set(false);
                                    on_close.get_value()();
                                    on_refresh2.run(());
                                }
                                Err(e) => {
                                    error_msg.set(Some(format_server_fn_error(&e.to_string())));
                                    saving.set(false);
                                }
                            }
                        }
                        Err(e) => {
                            error_msg.set(Some(format_server_fn_error(&e.to_string())));
                            saving.set(false);
                        }
                    }
                });
            } else {
                // Create local calendar.
                leptos::task::spawn_local(async move {
                    match create_calendar_api(name.trim().to_string(), color).await {
                        Ok(_) => {
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
            }
        }
    };

    // ── Refresh-now handler (webcal edit) ─────────────────────────────────────
    let refresh_now = {
        move |_: leptos::ev::MouseEvent| {
            let name = cal_name_sv.get_value();
            refreshing.set(true);
            error_msg.set(None);
            let on_refresh = on_refresh;
            leptos::task::spawn_local(async move {
                match refresh_webcal_calendar_api(name).await {
                    Ok(()) => on_refresh.run(()),
                    Err(e) => error_msg.set(Some(format_server_fn_error(&e.to_string()))),
                }
                refreshing.set(false);
            });
        }
    };

    // ── Delete / unsubscribe handler ──────────────────────────────────────────
    let do_delete = {
        move |_: leptos::ev::MouseEvent| {
            let name = cal_name_sv.get_value();
            saving.set(true);
            error_msg.set(None);
            let on_close = on_close;
            let on_refresh = on_refresh;
            if is_webcal {
                leptos::task::spawn_local(async move {
                    match set_calendar_sync_url_api(name, None).await {
                        Ok(()) => {
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
                    match delete_calendar_api(name).await {
                        Ok(()) => {
                            on_close.get_value()();
                            on_refresh.run(());
                        }
                        Err(e) => {
                            error_msg.set(Some(format_server_fn_error(&e.to_string())));
                            saving.set(false);
                        }
                    }
                });
            }
        }
    };

    let title = if is_edit {
        "Edit Calendar"
    } else {
        "New Calendar"
    };
    let backdrop_mousedown = RwSignal::new(false);
    let esc_id = crate::modal_esc::use_escape_close(move || {
        // Escape dismisses the clear-all sub-dialog first (unless it's busy),
        // then the calendar editor itself.
        if clear_all_open.get_untracked() {
            if !clear_all_busy.get_untracked() {
                clear_all_open.set(false);
            }
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
                    <button class="dialog-close"
                        on:click=move |_| on_close.get_value()()
                    >"✕"</button>
                </div>

                <div class="dialog-form">
                    // ── Name ──────────────────────────────────────────────────
                    <div class="form-group">
                        <label class="form-label">"Name"</label>
                        <input
                            class="form-input"
                            type="text"
                            placeholder="Calendar name"
                            prop:value=move || name_val.get()
                            on:input=move |ev| name_val.set(event_target_value(&ev))
                            disabled=move || is_shared || saving.get() || refreshing.get()
                        />
                        {is_shared.then(|| view! {
                            <p class="dialog-hint">"Shared calendars can only be renamed by the owner."</p>
                        })}
                    </div>

                    // ── Colour palette ────────────────────────────────────────
                    <div class="form-group">
                        <label class="form-label">"Colour"</label>
                        <div class="cal-palette">
                            {CALENDAR_PALETTE.iter().map(|&c| {
                                let color  = c.to_string();
                                let color2 = c.to_string();
                                view! {
                                    <button
                                        type="button"
                                        class="cal-palette-swatch"
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

                    // ── Webcal URL ────────────────────────────────────────────
                    // Always shown for webcal edits; shown as optional hint for new.
                    {if is_edit && !is_webcal {
                        ().into_any()
                    } else {
                        view! {
                            <div class="form-group">
                                <label class="form-label">
                                    {if is_webcal { "Sync URL" } else { "Subscribe to feed (optional)" }}
                                </label>
                                <input
                                    class="form-input"
                                    type="url"
                                    placeholder="webcal:// or https://"
                                    prop:value=move || url_val.get()
                                    on:input=move |ev| url_val.set(event_target_value(&ev))
                                    disabled=move || saving.get() || refreshing.get()
                                />
                            </div>
                        }.into_any()
                    }}

                    // ── Webcal sync status ────────────────────────────────────
                    {move || sync_error_sv.with_value(|e| e.clone()).map(|e| view! {
                        <p class="webcal-sync-error">"Last sync error: " {e}</p>
                    })}
                    {move || last_synced_sv.with_value(|t| t.clone()).map(|t| view! {
                        <p class="webcal-sync-ts">"Last synced: " {t}</p>
                    })}

                    // ── Errors ────────────────────────────────────────────────
                    {move || error_msg.get().map(|e| view! {
                        <p class="dialog-error">{e}</p>
                    })}
                </div>

                <div class="dialog-actions">
                    // Save / Create
                    <button
                        class="btn-primary"
                        disabled=move || saving.get() || refreshing.get()
                        on:click=save
                    >
                        {move || if saving.get() {
                            if is_edit { "Saving…" } else { "Creating…" }
                        } else if is_edit { "Save" } else { "Create" }}
                    </button>

                    // Refresh now (webcal edit only)
                    {if is_edit && is_webcal {
                        view! {
                            <button
                                class="btn-secondary"
                                disabled=move || saving.get() || refreshing.get()
                                on:click=refresh_now
                            >
                                {move || if refreshing.get() { "Refreshing…" } else { "Refresh now" }}
                            </button>
                        }.into_any()
                    } else { ().into_any() }}

                    // Share (edit, non-webcal). Shown but disabled for a
                    // shared calendar — sharing it further belongs to the
                    // owner, not this grantee (see backlog #57).
                    {if is_edit {
                        view! {
                            <button
                                class="btn-secondary"
                                disabled=move || is_shared || saving.get() || refreshing.get()
                                on:click=move |_| share_open.set(true)
                            >"Share"</button>
                        }.into_any()
                    } else { ().into_any() }}

                    // Delete / Unsubscribe / Clear-all-events. A shared
                    // calendar only gets "Clear all events", and only under
                    // a `read-write` grant — the grantee never deletes or
                    // unsubscribes the owner's calendar.
                    {if is_shared {
                        can_write_shared.then(|| view! {
                            <button
                                class="btn-danger"
                                disabled=move || saving.get() || refreshing.get()
                                on:click=move |_| clear_all_open.set(true)
                            >"Clear all events"</button>
                        }).into_any()
                    } else if is_edit {
                        if is_personal {
                            view! {
                                <button
                                    class="btn-danger"
                                    disabled=move || saving.get() || refreshing.get()
                                    on:click=move |_| clear_all_open.set(true)
                                >"Clear all events"</button>
                            }.into_any()
                        } else {
                            let delete_label = if is_webcal { "Unsubscribe" } else { "Delete" };
                            view! {
                                <button
                                    class="btn-danger"
                                    disabled=move || saving.get() || refreshing.get()
                                    on:click=do_delete
                                >{delete_label}</button>
                            }.into_any()
                        }
                    } else { ().into_any() }}

                    <button
                        class="btn-secondary"
                        disabled=move || saving.get() || refreshing.get()
                        on:click=move |_| on_close.get_value()()
                    >"Cancel"</button>
                </div>
            </div>
        </div>

        // ── Clear-all-events sub-dialog (personal calendar, or a shared one
        // under a `read-write` grant) ─────────────────────────────────────────
        {move || clear_all_open.get().then(|| {
            let name_for_dialog = name_val.get_untracked();
            let backdrop_mousedown = RwSignal::new(false);
            view! {
                <div class="dialog-backdrop"
                    on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
                    on:click=move |_| {
                        if backdrop_mousedown.get() && !clear_all_busy.get_untracked() { clear_all_open.set(false); }
                    }
                >
                    <div class="dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                        <div class="dialog-header">
                            <h2 class="dialog-title">
                                {if is_shared { "Clear Shared Calendar" } else { "Clear Personal Calendar" }}
                            </h2>
                            <button class="dialog-close"
                                disabled=move || clear_all_busy.get()
                                on:click=move |_| clear_all_open.set(false)
                            >"✕"</button>
                        </div>
                        <div class="dialog-form">
                            {if is_shared {
                                let owner = shared_owner_sv.get_value().unwrap_or_default();
                                view! {
                                    <p>
                                        "This deletes every event in \"" {name_for_dialog} "\" — "
                                        "not just from your view. " {owner}
                                        " and anyone else this calendar is shared with will lose "
                                        "these events too. This cannot be undone."
                                    </p>
                                }.into_any()
                            } else {
                                view! {
                                    <p>"Personal is your default calendar and cannot be deleted."</p>
                                    <p>"Delete all events from it?"</p>
                                }.into_any()
                            }}
                        </div>
                        <div class="dialog-actions">
                            <button
                                class="btn-danger"
                                disabled=move || clear_all_busy.get()
                                on:click=move |_| {
                                    clear_all_busy.set(true);
                                    let on_close   = on_close;
                                    let on_refresh = on_refresh;
                                    if is_shared {
                                        let owner = shared_owner_sv.get_value().unwrap_or_default();
                                        let cal   = cal_name_sv.get_value();
                                        leptos::task::spawn_local(async move {
                                            match clear_shared_calendar(owner, cal).await {
                                                Ok(()) => {
                                                    clear_all_busy.set(false);
                                                    clear_all_open.set(false);
                                                    on_close.get_value()();
                                                    on_refresh.run(());
                                                }
                                                Err(e) => {
                                                    error_msg.set(Some(format_server_fn_error(&e.to_string())));
                                                    clear_all_busy.set(false);
                                                    clear_all_open.set(false);
                                                }
                                            }
                                        });
                                    } else {
                                        leptos::task::spawn_local(async move {
                                            let _ = crate::api::clear_personal_calendar().await;
                                            clear_all_busy.set(false);
                                            clear_all_open.set(false);
                                            on_refresh.run(());
                                        });
                                    }
                                }
                            >
                                {move || if clear_all_busy.get() { "Deleting…" } else { "Yes, delete all" }}
                            </button>
                            <button
                                class="btn-secondary"
                                disabled=move || clear_all_busy.get()
                                on:click=move |_| clear_all_open.set(false)
                            >"Cancel"</button>
                        </div>
                    </div>
                </div>
            }
        })}

        // ── Share sub-dialog ──────────────────────────────────────────────────
        {move || share_open.get().then(|| {
            let cal_name_for_share = cal_name_sv.get_value();
            let cal_dn_for_share   = name_val.get_untracked();
            view! {
                <CalendarShareDialog
                    calendar_name=cal_name_for_share
                    calendar_display=cal_dn_for_share
                    on_close=move || share_open.set(false)
                />
            }
        })}
    }
}

// ── CalendarShareDialog ───────────────────────────────────────────────────────

/// Inline share management dialog (extracted from the old ShareDialog).
/// `pub(crate)` so `settings::sharing` can reuse it directly instead of
/// duplicating the invite/revoke form there (see backlog note in that module).
#[component]
pub(crate) fn CalendarShareDialog(
    calendar_name: String,
    calendar_display: String,
    on_close: impl Fn() + Send + Sync + 'static,
) -> impl IntoView {
    let on_close = StoredValue::new(Arc::new(on_close) as Arc<dyn Fn() + Send + Sync>);
    let cal_name_sv = StoredValue::new(calendar_name.clone());

    let shares: RwSignal<Vec<ShareInfo>> = RwSignal::new(vec![]);
    let shares_loading = RwSignal::new(true);
    let shares_error = RwSignal::new(Option::<String>::None);

    let refresh_sv: StoredValue<Arc<dyn Fn() + Send + Sync>> =
        StoredValue::new(Arc::new(move || {
            let cal = cal_name_sv.get_value();
            shares_loading.set(true);
            leptos::task::spawn_local(async move {
                match get_calendar_shares(cal).await {
                    Ok(list) => {
                        shares.set(list);
                        shares_error.set(None);
                    }
                    Err(e) => shares_error.set(Some(format_server_fn_error(&e.to_string()))),
                }
                shares_loading.set(false);
            });
        }));
    leptos::task::spawn_local(async move {
        refresh_sv.with_value(|f| f());
    });

    let invite_email = RwSignal::new(String::new());
    let invite_perm = RwSignal::new("read".to_string());
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
        let cal = cal_name_sv.get_value();
        let perm = invite_perm.get_untracked();
        leptos::task::spawn_local(async move {
            match share_calendar(cal, email.clone(), perm).await {
                Ok(()) => {
                    invite_ok.set(Some(format!("Invitation sent to {email}.")));
                    invite_email.set(String::new());
                    refresh_sv.with_value(|f| f());
                }
                Err(e) => {
                    invite_error.set(Some(format_server_fn_error(&e.to_string())));
                }
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
                    <h2 class="dialog-title">{format!("Share \"{}\"", calendar_display)}</h2>
                    <button class="dialog-close"
                        on:click=move |_| on_close.with_value(|f| f())>"✕"</button>
                </div>
                <div class="dialog-form">
                    // ── Current grants ────────────────────────────────────────
                    <div class="form-group">
                        <label class="form-label">"Shared with"</label>
                        {move || if shares_loading.get() {
                            view! { <p class="dialog-hint">"Loading…"</p> }.into_any()
                        } else if let Some(e) = shares_error.get() {
                            view! { <p class="dialog-error">{e}</p> }.into_any()
                        } else {
                            let list = shares.get();
                            if list.is_empty() {
                                view! { <p class="dialog-hint">"Not shared with anyone yet."</p> }.into_any()
                            } else {
                                view! {
                                    <ul class="share-list">
                                        {list.into_iter().map(|s| {
                                            let grantee = s.grantee_username.clone();
                                            let grantee2 = grantee.clone();
                                            let perm    = s.permission.clone();
                                            view! {
                                                <li class="share-list-item">
                                                    <span class="share-email">{grantee}</span>
                                                    <span class="share-perm">{perm}</span>
                                                    <button
                                                        class="share-revoke-btn"
                                                        on:click=move |_| {
                                                            let cal = cal_name_sv.get_value();
                                                            let u   = grantee2.clone();
                                                            leptos::task::spawn_local(async move {
                                                                let _ = revoke_calendar_share(cal, u).await;
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

                    // ── Invite form ───────────────────────────────────────────
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
                            <select
                                class="form-input"
                                prop:value=move || invite_perm.get()
                                on:change=move |ev| invite_perm.set(event_target_value(&ev))
                                disabled=move || inviting.get()
                            >
                                <option value="read">"Read"</option>
                                <option value="read-write">"Read/Write"</option>
                            </select>
                        </div>
                    </div>
                    {move || invite_ok.get().map(|m| view! { <p class="dialog-ok">{m}</p> })}
                    {move || invite_error.get().map(|e| view! { <p class="dialog-error">{e}</p> })}
                </div>
                <div class="dialog-actions">
                    <button
                        class="btn-primary"
                        disabled=move || inviting.get()
                        on:click=on_invite
                    >
                        {move || if inviting.get() { "Inviting…" } else { "Invite" }}
                    </button>
                    <button
                        class="btn-secondary"
                        on:click=move |_| on_close.with_value(|f| f())
                    >"Done"</button>
                </div>
            </div>
        </div>
    }
}

// ── MdSourceEditModal ─────────────────────────────────────────────────────────

/// Combined create / edit modal for markdown sources.
///
/// This is the single place that owns markdown-source configuration (name,
/// type, location, colour) for both source types — the
/// sidebar's "+"/pencil and the Settings panel's "Add source"/edit actions
/// all open this same component so their capabilities can't drift apart.
///
/// - `src = None`  → create mode (choose filesystem or git).
/// - `src = Some`  → edit mode (pre-filled; shows Delete unless config-managed).
#[component]
pub fn MdSourceEditModal(
    /// `None` to create; `Some(src)` to edit an existing source.
    src: Option<MarkdownSourceInfo>,
    on_close: impl Fn() + Send + Sync + 'static,
    on_refresh: Callback<()>,
) -> impl IntoView {
    let on_close = StoredValue::new(Arc::new(on_close) as Arc<dyn Fn() + Send + Sync>);

    let is_edit = src.is_some();
    let is_git_fixed = src
        .as_ref()
        .map(|s| s.source_type == "git")
        .unwrap_or(false);
    let config_managed = src.as_ref().map(|s| s.config_managed).unwrap_or(false);
    let src_id = StoredValue::new(src.as_ref().map(|s| s.id));

    let default_color = src
        .as_ref()
        .map(|s| s.color.clone())
        .unwrap_or_else(|| MARKDOWN_PALETTE[0].to_string());

    let name_val = RwSignal::new(src.as_ref().map(|s| s.name.clone()).unwrap_or_default());
    let path_val = RwSignal::new(src.as_ref().map(|s| s.path.clone()).unwrap_or_default());
    let url_val = RwSignal::new(src.as_ref().and_then(|s| s.url.clone()).unwrap_or_default());
    let branch_val = RwSignal::new(
        src.as_ref()
            .and_then(|s| s.branch.clone())
            .unwrap_or_default(),
    );
    // PAT is never sent to the client, so this always starts blank ("leave blank to keep current").
    let pat_val = RwSignal::new(String::new());
    let color_val = RwSignal::new(default_color);
    let interval_val = RwSignal::new(src.as_ref().map(|s| s.sync_interval_secs).unwrap_or(60));

    // Create-mode-only: which type of source is being added. Fixed to the
    // existing source's type in edit mode (changing type after creation
    // would mean a different config shape entirely — not supported).
    let create_type = RwSignal::new("filesystem".to_string());
    let is_git = move || {
        if is_edit {
            is_git_fixed
        } else {
            create_type.get() == "git"
        }
    };

    // Whether the server has an encryption key configured — gates the PAT
    // field for newly created git sources. None = still loading.
    let enc_key_ok: RwSignal<Option<bool>> = RwSignal::new(None);
    leptos::task::spawn_local(async move {
        let _ = is_enc_key_configured()
            .await
            .map(|ok| enc_key_ok.set(Some(ok)));
    });

    let saving = RwSignal::new(false);
    let error_msg = RwSignal::<Option<String>>::new(None);

    // Folder browser (filesystem sources only) — lets the user pick the path
    // interactively instead of typing it.
    let browse_open = RwSignal::new(false);

    // ── Save handler ──────────────────────────────────────────────────────────
    let save = {
        move |_: leptos::ev::MouseEvent| {
            let name = name_val.get_untracked();
            let color = color_val.get_untracked();
            let sync_interval_secs = interval_val.get_untracked();

            if name.trim().is_empty() {
                error_msg.set(Some("Name is required.".into()));
                return;
            }
            if sync_interval_secs < MIN_SYNC_INTERVAL_SECS {
                error_msg.set(Some(format!(
                    "Sync interval must be at least {MIN_SYNC_INTERVAL_SECS} seconds."
                )));
                return;
            }

            let on_close = on_close;
            let on_refresh = on_refresh;

            if is_git() {
                let url = url_val.get_untracked();
                if url.trim().is_empty() {
                    error_msg.set(Some("URL is required.".into()));
                    return;
                }
                let branch = branch_val.get_untracked();
                let pat = pat_val.get_untracked();

                if is_edit {
                    saving.set(true);
                    error_msg.set(None);
                    let id = src_id.get_value().unwrap_or_default();
                    leptos::task::spawn_local(async move {
                        match update_git_source(
                            id,
                            name.trim().to_string(),
                            url.trim().to_string(),
                            branch.trim().to_string(),
                            color,
                            pat.trim().to_string(),
                            sync_interval_secs,
                        )
                        .await
                        {
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
                    if pat.trim().is_empty() {
                        error_msg.set(Some("Personal access token is required.".into()));
                        return;
                    }
                    saving.set(true);
                    error_msg.set(None);
                    leptos::task::spawn_local(async move {
                        match add_git_source(NewGitSource {
                            name: name.trim().to_string(),
                            url: url.trim().to_string(),
                            branch: branch.trim().to_string(),
                            pat: pat.trim().to_string(),
                            color,
                            sync_interval_secs,
                        })
                        .await
                        {
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
                }
                return;
            }

            let path = path_val.get_untracked();
            if path.trim().is_empty() {
                error_msg.set(Some("Path is required.".into()));
                return;
            }
            saving.set(true);
            error_msg.set(None);

            if is_edit {
                let id = src_id.get_value().unwrap_or_default();
                leptos::task::spawn_local(async move {
                    match update_markdown_source(
                        id,
                        name.trim().to_string(),
                        path.trim().to_string(),
                        color,
                        sync_interval_secs,
                    )
                    .await
                    {
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
                    match add_markdown_source(NewMarkdownSource {
                        name: name.trim().to_string(),
                        path: path.trim().to_string(),
                        color,
                        sync_interval_secs,
                    })
                    .await
                    {
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
            }
        }
    };

    // ── Delete handler ────────────────────────────────────────────────────────
    let do_delete = {
        move |_: leptos::ev::MouseEvent| {
            let id = src_id.get_value().unwrap_or_default();
            saving.set(true);
            error_msg.set(None);
            let on_close = on_close;
            let on_refresh = on_refresh;
            leptos::task::spawn_local(async move {
                match delete_markdown_source(id).await {
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
        }
    };

    let title = if !is_edit {
        "Add Markdown Source"
    } else if config_managed {
        "Markdown Source (config-managed)"
    } else if is_git_fixed {
        "Edit Git Source"
    } else {
        "Edit Markdown Source"
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
                    <button class="dialog-close"
                        on:click=move |_| on_close.get_value()()
                    >"✕"</button>
                </div>

                <div class="dialog-form">
                    // ── Name ──────────────────────────────────────────────────
                    <div class="form-group">
                        <label class="form-label">"Name"</label>
                        <input
                            class="form-input"
                            type="text"
                            placeholder="e.g. Work notes"
                            prop:value=move || name_val.get()
                            on:input=move |ev| name_val.set(event_target_value(&ev))
                            disabled=move || saving.get()
                        />
                    </div>

                    // ── Source type (create mode only) ──────────────────────────
                    {(!is_edit).then(|| view! {
                        <div class="form-group">
                            <label class="form-label">"Source type"</label>
                            <select
                                class="form-input"
                                on:change=move |ev| create_type.set(event_target_value(&ev))
                            >
                                <option value="filesystem" selected=move || create_type.get() == "filesystem">
                                    "Local directory"
                                </option>
                                <option value="git" selected=move || create_type.get() == "git">
                                    "Git repository"
                                </option>
                            </select>
                        </div>
                    })}

                    {move || if config_managed {
                        view! {
                            <p class="dialog-hint">
                                "Provisioned via the server's NixOS config — its URL, branch, \
                                 and access token can only be changed there."
                            </p>
                        }.into_any()
                    } else if is_git() {
                        view! {
                            <>
                                // ── URL ───────────────────────────────────────────
                                <div class="form-group">
                                    <label class="form-label">"Remote URL"</label>
                                    <input
                                        class="form-input"
                                        type="text"
                                        placeholder="https://forgejo.example.com/user/notes.git"
                                        prop:value=move || url_val.get()
                                        on:input=move |ev| url_val.set(event_target_value(&ev))
                                        disabled=move || saving.get()
                                    />
                                </div>

                                // ── Branch ────────────────────────────────────────
                                <div class="form-group">
                                    <label class="form-label">"Branch"</label>
                                    <input
                                        class="form-input"
                                        type="text"
                                        placeholder="Leave blank to auto-detect the remote's default"
                                        prop:value=move || branch_val.get()
                                        on:input=move |ev| branch_val.set(event_target_value(&ev))
                                        disabled=move || saving.get()
                                    />
                                </div>

                                // ── PAT ───────────────────────────────────────────
                                {move || match (is_edit, enc_key_ok.get()) {
                                    (false, None) => view! {
                                        <p class="form-help">"Checking encryption…"</p>
                                    }.into_any(),
                                    (false, Some(false)) => view! {
                                        <p class="form-error">
                                            "Git sources store a personal access token, which must be encrypted at rest. Set the "
                                            <code>"TIDL_ENCRYPTION_KEY_FILE"</code>
                                            " environment variable to a path containing a 32-byte key to enable this."
                                        </p>
                                    }.into_any(),
                                    _ => view! {
                                        <div class="form-group">
                                            <label class="form-label">"Personal access token"</label>
                                            <input
                                                class="form-input"
                                                type="password"
                                                placeholder=if is_edit { "Leave blank to keep the current token" } else { "ghp_…" }
                                                prop:value=move || pat_val.get()
                                                on:input=move |ev| pat_val.set(event_target_value(&ev))
                                                disabled=move || saving.get()
                                            />
                                        </div>
                                    }.into_any(),
                                }}
                            </>
                        }.into_any()
                    } else {
                        view! {
                            // ── Path ──────────────────────────────────────────────
                            <div class="form-group">
                                <label class="form-label">"Path on server"</label>
                                <div class="md-source-path-row">
                                    <input
                                        class="form-input"
                                        type="text"
                                        placeholder="/home/user/notes"
                                        prop:value=move || path_val.get()
                                        on:input=move |ev| path_val.set(event_target_value(&ev))
                                        disabled=move || saving.get()
                                    />
                                    <button
                                        type="button"
                                        class="btn-secondary"
                                        disabled=move || saving.get()
                                        on:click=move |_| browse_open.set(true)
                                    >"Browse…"</button>
                                </div>
                            </div>
                        }.into_any()
                    }}

                    // ── Sync interval ─────────────────────────────────────────
                    <div class="form-group">
                        <label class="form-label">"Sync interval (seconds)"</label>
                        <input
                            class="form-input"
                            type="number"
                            min=MIN_SYNC_INTERVAL_SECS.to_string()
                            prop:value=move || interval_val.get().to_string()
                            on:input=move |ev| {
                                if let Ok(v) = event_target_value(&ev).parse::<u32>() {
                                    interval_val.set(v);
                                }
                            }
                            disabled=move || saving.get()
                        />
                        {move || (interval_val.get() < MIN_SYNC_INTERVAL_SECS).then(|| view! {
                            <div class="form-error">
                                "Minimum " {MIN_SYNC_INTERVAL_SECS} " seconds"
                            </div>
                        })}
                    </div>

                    // ── Colour palette ────────────────────────────────────────
                    <div class="form-group">
                        <label class="form-label">"Colour"</label>
                        <div class="cal-palette">
                            {MARKDOWN_PALETTE.iter().map(|&c| {
                                let color  = c.to_string();
                                let color2 = c.to_string();
                                view! {
                                    <button
                                        type="button"
                                        class="cal-palette-swatch"
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

                    // ── Errors ────────────────────────────────────────────────
                    {move || error_msg.get().map(|e| view! {
                        <p class="dialog-error">{e}</p>
                    })}
                </div>

                <div class="dialog-actions">
                    <button
                        class="btn-primary"
                        disabled=move || saving.get()
                        on:click=save
                    >
                        {move || if saving.get() {
                            if is_edit { "Saving…" } else { "Adding…" }
                        } else if is_edit { "Save" } else { "Add" }}
                    </button>

                    {if is_edit && !config_managed {
                        view! {
                            <button
                                class="btn-danger"
                                disabled=move || saving.get()
                                on:click=do_delete
                            >"Delete"</button>
                        }.into_any()
                    } else { ().into_any() }}

                    <button
                        class="btn-secondary"
                        disabled=move || saving.get()
                        on:click=move |_| on_close.get_value()()
                    >"Cancel"</button>
                </div>
            </div>

            {move || browse_open.get().then(|| view! {
                <DirPickerModal
                    start=Some(path_val.get_untracked()).filter(|p| !p.trim().is_empty())
                    on_close=move || browse_open.set(false)
                    on_pick=move |picked| {
                        path_val.set(picked);
                        browse_open.set(false);
                    }
                />
            })}
        </div>
    }
}
