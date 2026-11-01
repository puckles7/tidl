//! Unified "Calendars" sidebar section — shared by the Calendar page and
//! Focus's sidebar. Replaces the two independent
//! implementations that used to live in `calendar/calendar_list.rs` and
//! inline in `focus_sidebar.rs`, each with its own fetch and its own
//! visibility model (see backlog #41 Follow-up 1/4 for how that duplication
//! caused the same shared-calendar bug to need fixing twice).
//!
//! Owns its own data: fetches `get_calendars()` + `get_shared_calendars()`,
//! merges them into `AllCalendarsCtx` via `merge_calendar_entries`, and
//! renders one `SidebarPill` per entry. Every entry gets an edit button —
//! owned ones open `CalendarEditModal` with `cal` set; shared ones open the
//! same modal with `shared` set instead, which narrows it to what's
//! actually the grantee's to change.

use leptos::prelude::*;
use std::collections::HashSet;

use crate::api::{
    get_calendars, get_shared_calendars, list_remote_sources, refresh_webcal_calendar_api,
    trigger_remote_caldav_sync, RemoteCalDavSource,
};
use crate::app::{use_events_refresh, use_sidebar_refresh, AllCalendarsCtx, CurrentUsernameCtx};
use crate::components::cal_edit_modal::{CalendarEditModal, SharedCalendarEditInfo};
use crate::components::settings::OpenSettingsCtx;
use crate::components::sidebar_pill::SidebarPill;
use crate::types::{merge_calendar_entries, CalendarEntry, CalendarInfo};

/// What `CalendarEditModal` is currently open for — the single source of
/// truth this panel maps onto that modal's `cal`/`shared` props, so there's
/// only ever one modal instantiation site regardless of which kind of entry
/// was clicked.
#[derive(Clone)]
enum EditTarget {
    Create,
    Owned(CalendarInfo),
    Shared(CalendarEntry),
}

#[component]
pub fn CalendarListPanel(on_refresh: Callback<()>) -> impl IntoView {
    let all_calendars = use_context::<AllCalendarsCtx>()
        .expect("AllCalendarsCtx must be provided by AuthenticatedApp");
    let my_username = use_context::<CurrentUsernameCtx>()
        .expect("CurrentUsernameCtx must be provided by AuthenticatedApp");
    let open_settings = use_context::<OpenSettingsCtx>()
        .expect("OpenSettingsCtx must be provided by AuthenticatedApp");

    let sidebar_refresh = use_sidebar_refresh();
    let events_refresh = use_events_refresh();

    let cals_res = LocalResource::new(move || {
        let _ = sidebar_refresh.get();
        async move { get_calendars().await.unwrap_or_default() }
    });
    let shared_cals_res = LocalResource::new(move || {
        let _ = sidebar_refresh.get();
        async move { get_shared_calendars().await.unwrap_or_default() }
    });
    let caldav_sources_res = LocalResource::new(move || {
        let _ = sidebar_refresh.get();
        async move { list_remote_sources().await.unwrap_or_default() }
    });

    // Mirror caldav_sources_res into a plain signal via an Effect, same
    // reasoning as the merge Effect below: keeps LocalResource loading state
    // out of the rendering tree so it never registers a suspension with an
    // ancestor <Transition>/<Suspense>.
    let caldav_sources: RwSignal<Vec<RemoteCalDavSource>> = RwSignal::new(vec![]);
    Effect::new(move |_| {
        if let Some(sources) = caldav_sources_res.get() {
            caldav_sources.set(sources.to_vec());
        }
    });

    // Not wasm32-gated: harmless no-op during SSR since LocalResource never
    // resolves server-side, and this keeps `cals_res`/`shared_cals_res` from
    // looking unused on that target (matches the ungated `caldav_sources`
    // Effect above).
    Effect::new(move |_| {
        if let (Some(cals), Some(shared)) = (cals_res.get(), shared_cals_res.get()) {
            let existing = all_calendars.get_untracked();
            let updated = merge_calendar_entries(
                &my_username.0.get_untracked(),
                cals.to_vec(),
                shared.to_vec(),
                &existing,
            );
            // Only notify subscribers when the data actually changed — avoids
            // tearing down and rebuilding every SidebarPill on each refetch.
            if updated != existing {
                all_calendars.set(updated);
            }
        }
    });

    let editing: RwSignal<Option<EditTarget>> = RwSignal::new(None);

    // Calendar names (not calendar_name+idx) currently mid-sync — keyed by
    // name since that's the stable identity `on_sync` closes over. A set
    // (not a single `Option`) so syncing one calendar's pill doesn't clear
    // the spinner on another mid-flight.
    let syncing_cals: RwSignal<HashSet<String>> = RwSignal::new(HashSet::new());

    view! {
        <div class="fs-section">
            <div class="fs-section-header">
                <span class="fs-section-title">"Calendars"</span>
                <button
                    class="fs-add-btn"
                    title="New calendar / subscribe"
                    on:click=move |_| editing.set(Some(EditTarget::Create))
                >"+"</button>
            </div>
            {move || crate::components::settings::show_sidebar_hints().then(|| view! {
                <p class="fs-hint">"Toggle a calendar on/off to show/hide its items"</p>
            })}
            {move || {
                let entries = all_calendars.get();
                if entries.is_empty() {
                    view! { <p class="fs-empty">"No calendars yet."</p> }.into_any()
                } else {
                    let sources = caldav_sources.get();
                    view! {
                        <ul class="fs-list">
                            {entries.into_iter().enumerate().map(|(idx, (entry, visible))| {
                                let is_owned  = entry.is_owned();
                                let is_webcal = entry.sync_url.is_some();
                                let cal_name  = entry.calendar_name.clone();
                                let label = entry.display_name.clone();
                                // Share icon only appears on entries that are
                                // actually shared — a shared-with-me entry
                                // always qualifies; an owned one only if it
                                // has grantees. Tooltip carries what used to
                                // be the "(from <owner>)" suffix in the pill
                                // name itself — see backlog #65.
                                let is_shared_either_way = !is_owned || entry.shared;
                                let share_title = if !is_owned {
                                    format!("Shared by {}", entry.owner_username)
                                } else {
                                    "Manage sharing".to_string()
                                };
                                let on_share: Option<Callback<()>> = is_shared_either_way.then(|| Callback::new(move |_| {
                                    open_settings.0.set(Some("sharing"));
                                }));
                                // Collect IDs of all CalDAV sync tasks whose local_calendar
                                // matches this (owned-only — shared calendars aren't synced
                                // by the grantee, that's the owner's responsibility) entry.
                                let caldav_ids: Vec<String> = if is_owned {
                                    sources.iter()
                                        .filter(|s| s.local_calendar == entry.calendar_name)
                                        .map(|s| s.id.clone())
                                        .collect()
                                } else {
                                    vec![]
                                };
                                let has_sync = is_owned && (is_webcal || !caldav_ids.is_empty());
                                let syncing: Signal<bool> = {
                                    let name = cal_name.clone();
                                    Signal::derive(move || syncing_cals.get().contains(&name))
                                };
                                let on_sync: Option<Callback<()>> = has_sync.then(|| Callback::new(move |_| {
                                    let name = cal_name.clone();
                                    let ids  = caldav_ids.clone();
                                    syncing_cals.update(|s| { s.insert(name.clone()); });
                                    leptos::task::spawn_local(async move {
                                        if is_webcal {
                                            let _ = refresh_webcal_calendar_api(name.clone()).await;
                                        }
                                        for id in ids {
                                            let _ = trigger_remote_caldav_sync(id).await;
                                        }
                                        // Sync imports events only — no calendar metadata changed.
                                        events_refresh.update(|n| *n += 1);
                                        syncing_cals.update(|s| { s.remove(&name); });
                                    });
                                }));
                                // Owned entries open the modal via `to_calendar_info()`
                                // (`cal` set); shared entries (no `id`, so that call
                                // returns `None`) open the same modal with `shared` set.
                                let on_edit: Option<Callback<()>> = if let Some(cal_info) = entry.to_calendar_info() {
                                    Some(Callback::new(move |_| {
                                        editing.set(Some(EditTarget::Owned(cal_info.clone())));
                                    }))
                                } else {
                                    let entry_for_edit = entry.clone();
                                    Some(Callback::new(move |_| {
                                        editing.set(Some(EditTarget::Shared(entry_for_edit.clone())));
                                    }))
                                };
                                view! {
                                    <li class="fs-list-item">
                                        <SidebarPill
                                            display_name=label
                                            color=entry.color.clone()
                                            visible
                                            on_toggle=Callback::new(move |_| {
                                                all_calendars.update(|entries| {
                                                    if let Some((_, v)) = entries.get_mut(idx) { *v = !*v; }
                                                });
                                            })
                                            on_edit=on_edit
                                            on_sync=on_sync
                                            syncing=syncing
                                            on_share=on_share
                                            share_title=share_title
                                        />
                                    </li>
                                }
                            }).collect_view()}
                        </ul>
                    }.into_any()
                }
            }}
        </div>

        // ── Calendar edit / create modal ──────────────────────────────────
        {move || editing.get().map(|target| {
            let (cal, shared) = match target {
                EditTarget::Create => (None, None),
                EditTarget::Owned(cal_info) => (Some(cal_info), None),
                EditTarget::Shared(entry) => (None, Some(SharedCalendarEditInfo {
                    owner_username: entry.owner_username,
                    calendar_name:  entry.calendar_name,
                    display_name:   entry.display_name,
                    color:          entry.color,
                    permission:     entry.shared_permission.unwrap_or_default(),
                })),
            };
            view! {
                <CalendarEditModal
                    cal=cal
                    shared=shared
                    on_close=move || editing.set(None)
                    on_refresh=on_refresh
                />
            }
        })}
    }
}
