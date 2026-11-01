use leptos::prelude::*;

use super::CalsResource;
use crate::api::{
    get_calendar_shares, get_shared_calendars, leave_contact_collection, leave_note_collection,
    leave_shared_calendar, list_contact_collections, list_note_collections,
    remove_contact_collection_member, remove_note_collection_member, revoke_calendar_share,
    unshare_contact_collection, unshare_note_collection, CalendarInfo, ContactCollectionInfo,
    NoteCollectionInfo, ShareInfo, SharedCalendar,
};
use crate::components::cal_edit_modal::CalendarShareDialog;
use crate::components::contacts::contact_collection_editor_modal::ContactCollectionShareDialog;
use crate::components::notes::collection_editor_modal::NoteCollectionShareDialog;

#[component]
pub(super) fn SharingPanel(cals_res: CalsResource) -> impl IntoView {
    let share_cal = RwSignal::new(String::new());
    let cal_share_dialog_open = RwSignal::new(false);
    let shared_with_me = RwSignal::new(Vec::<SharedCalendar>::new());

    // All grants the current user has extended, across every calendar they
    // own — not scoped to whichever calendar happens to be picked in the
    // "Share a calendar" dropdown above, so revoking access doesn't require
    // first re-selecting the calendar from the share dialog.
    let my_shares = RwSignal::new(Vec::<(CalendarInfo, ShareInfo)>::new());
    let my_shares_tick = RwSignal::new(0u32);

    let note_collections = RwSignal::new(Vec::<NoteCollectionInfo>::new());
    let selected_coll_id = RwSignal::new(Option::<i64>::None);
    let coll_share_dialog_open = RwSignal::new(false);

    let contact_collections = RwSignal::new(Vec::<ContactCollectionInfo>::new());
    let selected_contact_coll_id = RwSignal::new(Option::<i64>::None);
    let contact_coll_share_dialog_open = RwSignal::new(false);

    // Load shared-with-me on mount.
    leptos::task::spawn_local(async move {
        if let Ok(list) = get_shared_calendars().await {
            shared_with_me.set(list);
        }
    });

    let reload_shared_with_me = move || {
        leptos::task::spawn_local(async move {
            if let Ok(list) = get_shared_calendars().await {
                shared_with_me.set(list);
            }
        });
    };

    // Load the current user's note collections on mount.
    leptos::task::spawn_local(async move {
        if let Ok(list) = list_note_collections().await {
            note_collections.set(list);
        }
    });

    // Every collection the user owns — not just already-shared ones, so a
    // fresh collection can be picked here and shared for the first time via
    // the Share button (mirrors the calendar picker above, which likewise
    // isn't filtered to "already shared" calendars).
    let owned_collections = move || {
        note_collections
            .get()
            .into_iter()
            .filter(|c| c.is_owner)
            .collect::<Vec<_>>()
    };

    let reload_note_collections = move || {
        leptos::task::spawn_local(async move {
            if let Ok(list) = list_note_collections().await {
                note_collections.set(list);
            }
        });
    };

    // Load the current user's contact collections on mount.
    leptos::task::spawn_local(async move {
        if let Ok(list) = list_contact_collections().await {
            contact_collections.set(list);
        }
    });

    let owned_contact_collections = move || {
        contact_collections
            .get()
            .into_iter()
            .filter(|c| c.is_owner)
            .collect::<Vec<_>>()
    };

    let reload_contact_collections = move || {
        leptos::task::spawn_local(async move {
            if let Ok(list) = list_contact_collections().await {
                contact_collections.set(list);
            }
        });
    };

    // Re-fetch the flat "who has access to what" list whenever the owned
    // calendar list resolves/changes or `my_shares_tick` is bumped (after a
    // revoke) — mirrors the `caldav_sources` Effect pattern used elsewhere
    // to bridge a resource into a plain signal without racing suspense.
    Effect::new(move |_| {
        let _ = my_shares_tick.get();
        if let Some(Ok(cals)) = cals_res.get() {
            leptos::task::spawn_local(async move {
                let mut all = Vec::new();
                for cal in cals {
                    if let Ok(list) = get_calendar_shares(cal.name.clone()).await {
                        all.extend(list.into_iter().map(|share| (cal.clone(), share)));
                    }
                }
                my_shares.set(all);
            });
        }
    });

    view! {
        <h3 class="settings-section-heading">"Share a calendar"</h3>
        <p class="dialog-hint">"Share a calendar with another tidl user on this server"</p>

        <div class="form-group">
            <div class="form-row form-row--picker">
                <select
                    class="form-input"
                    prop:value=move || share_cal.get()
                    on:change=move |ev| share_cal.set(event_target_value(&ev))
                >
                    <option value="" disabled=true>
                        "— pick a calendar —"
                    </option>
                    {move || {
                        cals_res.get()
                            .and_then(Result::ok)
                            .unwrap_or_default()
                            .into_iter()
                            .map(|cal| {
                                let name    = cal.name.clone();
                                let display = cal.display_name.clone();
                                view! { <option value=name>{display}</option> }
                            })
                            .collect_view()
                    }}
                </select>
                <button
                    class="btn-secondary"
                    disabled=move || share_cal.get().is_empty()
                    on:click=move |_| cal_share_dialog_open.set(true)
                >"Share"</button>
            </div>
        </div>

        <h3 class="settings-section-heading">"Your shared calendars"</h3>
        {move || {
            let list = my_shares.get();
            if list.is_empty() {
                view! { <p class="sharing-empty">"You haven't shared any calendars yet."</p> }.into_any()
            } else {
                list.into_iter().map(|(cal, share)| {
                    let grantee  = share.grantee_username.clone();
                    let cal_name = cal.name.clone();
                    view! {
                        <div class="sharing-row">
                            <span
                                class="sharing-color-dot"
                                style=format!("background:{}", cal.color)
                            />
                            <span class="sharing-grantee">
                                {cal.display_name.clone()} " → " {share.grantee_username.clone()}
                            </span>
                            <span class="sharing-perm">{share.permission.clone()}</span>
                            <button
                                class="btn-danger-small"
                                on:click=move |_| {
                                    let g = grantee.clone();
                                    let c = cal_name.clone();
                                    leptos::task::spawn_local(async move {
                                        let _ = revoke_calendar_share(c, g).await;
                                        my_shares_tick.update(|n| *n += 1);
                                    });
                                }
                            >"Revoke"</button>
                        </div>
                    }
                }).collect_view().into_any()
            }
        }}

        <h3 class="settings-section-heading">"Calendars shared with me"</h3>
        {move || {
            let list = shared_with_me.get();
            if list.is_empty() {
                view! { <p class="sharing-empty">"No calendars shared with you yet."</p> }.into_any()
            } else {
                list.into_iter().map(|sc| {
                    let owner = sc.owner_username.clone();
                    let cal_name = sc.calendar_name.clone();
                    view! {
                        <div class="sharing-row">
                            <span
                                class="sharing-color-dot"
                                style=format!("background:{}", sc.color)
                            />
                            <span class="sharing-grantee">
                                {sc.display_name.clone()} " (" {sc.owner_username.clone()} ")"
                            </span>
                            <span class="sharing-perm">{sc.permission.clone()}</span>
                            <button
                                class="btn-danger-small"
                                on:click=move |_| {
                                    let o = owner.clone();
                                    let c = cal_name.clone();
                                    leptos::task::spawn_local(async move {
                                        let _ = leave_shared_calendar(o, c).await;
                                        reload_shared_with_me();
                                    });
                                }
                            >"Remove"</button>
                        </div>
                    }
                }).collect_view().into_any()
            }
        }}

        <h3 class="settings-section-heading">"Note Collections"</h3>
        <p class="dialog-hint">"Share a note collection with another tidl user on this server"</p>

        <div class="form-group">
            <div class="form-row form-row--picker">
                <select
                    class="form-input"
                    prop:value=move || selected_coll_id.get().map(|id| id.to_string()).unwrap_or_default()
                    on:change=move |ev| {
                        let val = event_target_value(&ev);
                        selected_coll_id.set(val.parse::<i64>().ok());
                    }
                >
                    <option value="" disabled=true>
                        "— pick a collection —"
                    </option>
                    {move || {
                        owned_collections()
                            .into_iter()
                            .map(|c| {
                                let id = c.id.to_string();
                                view! { <option value=id>{c.name.clone()}</option> }
                            })
                            .collect_view()
                    }}
                </select>
                <button
                    class="btn-secondary"
                    disabled=move || selected_coll_id.get().is_none()
                    on:click=move |_| coll_share_dialog_open.set(true)
                >"Share"</button>
            </div>
        </div>

        // Bulk unshare for the picked collection — kept alongside the picker
        // rather than per-row in "Your shared collections" below, since it
        // also flips `shared` back off (not just clearing members one by
        // one). Only shown once there's actually something to unshare.
        {move || {
            let sel = selected_coll_id.get();
            let has_members = sel
                .and_then(|id| owned_collections().into_iter().find(|c| c.id == id))
                .map(|c| !c.members.is_empty())
                .unwrap_or(false);
            has_members.then(|| view! {
                <div class="dialog-actions">
                    <button
                        class="btn-danger-small"
                        on:click=move |_| {
                            let Some(id) = selected_coll_id.get_untracked() else { return; };
                            leptos::task::spawn_local(async move {
                                let _ = unshare_note_collection(id).await;
                                selected_coll_id.set(None);
                                reload_note_collections();
                            });
                        }
                    >"Unshare this collection"</button>
                </div>
            })
        }}

        <h3 class="settings-section-heading">"Your shared collections"</h3>
        {move || {
            let list: Vec<(NoteCollectionInfo, String)> = owned_collections()
                .into_iter()
                .flat_map(|c| c.members.clone().into_iter().map(move |m| (c.clone(), m)))
                .collect();
            if list.is_empty() {
                view! { <p class="sharing-empty">"You haven't shared any collections yet."</p> }.into_any()
            } else {
                list.into_iter().map(|(coll, member)| {
                    let coll_id = coll.id;
                    let m = member.clone();
                    view! {
                        <div class="sharing-row">
                            <span
                                class="sharing-color-dot"
                                style=format!("background:{}", coll.color)
                            />
                            <span class="sharing-grantee">
                                {coll.name.clone()} " → " {member.clone()}
                            </span>
                            <button
                                class="btn-danger-small"
                                on:click=move |_| {
                                    let m = m.clone();
                                    leptos::task::spawn_local(async move {
                                        let _ = remove_note_collection_member(coll_id, m).await;
                                        reload_note_collections();
                                    });
                                }
                            >"Revoke"</button>
                        </div>
                    }
                }).collect_view().into_any()
            }
        }}

        <h3 class="settings-section-heading">"Collections shared with me"</h3>
        {move || {
            let list: Vec<NoteCollectionInfo> = note_collections
                .get()
                .into_iter()
                .filter(|c| !c.is_owner)
                .collect();
            if list.is_empty() {
                view! { <p class="sharing-empty">"No collections shared with you yet."</p> }.into_any()
            } else {
                list.into_iter().map(|coll| {
                    let coll_id = coll.id;
                    view! {
                        <div class="sharing-row">
                            <span
                                class="sharing-color-dot"
                                style=format!("background:{}", coll.color)
                            />
                            <span class="sharing-grantee">
                                {coll.name.clone()} " (" {coll.owner_username.clone()} ")"
                            </span>
                            <button
                                class="btn-danger-small"
                                on:click=move |_| {
                                    leptos::task::spawn_local(async move {
                                        let _ = leave_note_collection(coll_id).await;
                                        reload_note_collections();
                                    });
                                }
                            >"Remove"</button>
                        </div>
                    }
                }).collect_view().into_any()
            }
        }}

        <h3 class="settings-section-heading">"Contact Collections"</h3>
        <p class="dialog-hint">"Share a contact collection with another tidl user on this server"</p>

        <div class="form-group">
            <div class="form-row form-row--picker">
                <select
                    class="form-input"
                    prop:value=move || selected_contact_coll_id.get().map(|id| id.to_string()).unwrap_or_default()
                    on:change=move |ev| {
                        let val = event_target_value(&ev);
                        selected_contact_coll_id.set(val.parse::<i64>().ok());
                    }
                >
                    <option value="" disabled=true>
                        "— pick a collection —"
                    </option>
                    {move || {
                        owned_contact_collections()
                            .into_iter()
                            .map(|c| {
                                let id = c.id.to_string();
                                view! { <option value=id>{c.name.clone()}</option> }
                            })
                            .collect_view()
                    }}
                </select>
                <button
                    class="btn-secondary"
                    disabled=move || selected_contact_coll_id.get().is_none()
                    on:click=move |_| contact_coll_share_dialog_open.set(true)
                >"Share"</button>
            </div>
        </div>

        // Bulk unshare for the picked collection — mirrors the note-collection
        // block above (also flips `shared` back off, not just clearing
        // members one by one).
        {move || {
            let sel = selected_contact_coll_id.get();
            let has_members = sel
                .and_then(|id| owned_contact_collections().into_iter().find(|c| c.id == id))
                .map(|c| !c.members.is_empty())
                .unwrap_or(false);
            has_members.then(|| view! {
                <div class="dialog-actions">
                    <button
                        class="btn-danger-small"
                        on:click=move |_| {
                            let Some(id) = selected_contact_coll_id.get_untracked() else { return; };
                            leptos::task::spawn_local(async move {
                                let _ = unshare_contact_collection(id).await;
                                selected_contact_coll_id.set(None);
                                reload_contact_collections();
                            });
                        }
                    >"Unshare this collection"</button>
                </div>
            })
        }}

        <h3 class="settings-section-heading">"Your shared contact collections"</h3>
        {move || {
            let list: Vec<(ContactCollectionInfo, String)> = owned_contact_collections()
                .into_iter()
                .flat_map(|c| c.members.clone().into_iter().map(move |m| (c.clone(), m)))
                .collect();
            if list.is_empty() {
                view! { <p class="sharing-empty">"You haven't shared any contact collections yet."</p> }.into_any()
            } else {
                list.into_iter().map(|(coll, member)| {
                    let coll_id = coll.id;
                    let m = member.clone();
                    view! {
                        <div class="sharing-row">
                            <span
                                class="sharing-color-dot"
                                style=format!("background:{}", coll.color)
                            />
                            <span class="sharing-grantee">
                                {coll.name.clone()} " → " {member.clone()}
                            </span>
                            <button
                                class="btn-danger-small"
                                on:click=move |_| {
                                    let m = m.clone();
                                    leptos::task::spawn_local(async move {
                                        let _ = remove_contact_collection_member(coll_id, m).await;
                                        reload_contact_collections();
                                    });
                                }
                            >"Revoke"</button>
                        </div>
                    }
                }).collect_view().into_any()
            }
        }}

        <h3 class="settings-section-heading">"Contact collections shared with me"</h3>
        {move || {
            let list: Vec<ContactCollectionInfo> = contact_collections
                .get()
                .into_iter()
                .filter(|c| !c.is_owner)
                .collect();
            if list.is_empty() {
                view! { <p class="sharing-empty">"No contact collections shared with you yet."</p> }.into_any()
            } else {
                list.into_iter().map(|coll| {
                    let coll_id = coll.id;
                    view! {
                        <div class="sharing-row">
                            <span
                                class="sharing-color-dot"
                                style=format!("background:{}", coll.color)
                            />
                            <span class="sharing-grantee">
                                {coll.name.clone()} " (" {coll.owner_username.clone()} ")"
                            </span>
                            <button
                                class="btn-danger-small"
                                on:click=move |_| {
                                    leptos::task::spawn_local(async move {
                                        let _ = leave_contact_collection(coll_id).await;
                                        reload_contact_collections();
                                    });
                                }
                            >"Remove"</button>
                        </div>
                    }
                }).collect_view().into_any()
            }
        }}

        // ── Calendar share sub-dialog ─────────────────────────────────────────
        {move || cal_share_dialog_open.get().then(|| {
            let name = share_cal.get_untracked();
            let display = cals_res.get()
                .and_then(Result::ok)
                .unwrap_or_default()
                .into_iter()
                .find(|c| c.name == name)
                .map(|c| c.display_name)
                .unwrap_or_else(|| name.clone());
            view! {
                <CalendarShareDialog
                    calendar_name=name
                    calendar_display=display
                    on_close=move || {
                        cal_share_dialog_open.set(false);
                        my_shares_tick.update(|n| *n += 1);
                    }
                />
            }
        })}

        // ── Note collection share sub-dialog ──────────────────────────────────
        {move || coll_share_dialog_open.get().then(|| {
            let id = selected_coll_id.get_untracked().unwrap_or_default();
            // Untracked: `on_change` below calls `reload_note_collections()`,
            // which writes `note_collections` — reading it tracked here (e.g.
            // via `owned_collections()`) would make this whole closure a
            // dependent of that write, remounting `NoteCollectionShareDialog`
            // on every refresh and re-triggering its own initial fetch +
            // `on_change` forever (the "Loading…"/"Not shared" flicker).
            let name = note_collections
                .get_untracked()
                .into_iter()
                .find(|c| c.id == id)
                .map(|c| c.name)
                .unwrap_or_default();
            view! {
                <NoteCollectionShareDialog
                    collection_id=id
                    collection_name=name
                    on_close=move || coll_share_dialog_open.set(false)
                    on_change=Callback::new(move |_: Vec<String>| reload_note_collections())
                />
            }
        })}

        // ── Contact collection share sub-dialog ───────────────────────────────
        {move || contact_coll_share_dialog_open.get().then(|| {
            let id = selected_contact_coll_id.get_untracked().unwrap_or_default();
            // Untracked for the same reason as the note-collection dialog
            // above — reading `contact_collections` tracked here would make
            // this closure a dependent of `reload_contact_collections()`'s
            // own write, remounting the dialog on every refresh.
            let name = contact_collections
                .get_untracked()
                .into_iter()
                .find(|c| c.id == id)
                .map(|c| c.name)
                .unwrap_or_default();
            view! {
                <ContactCollectionShareDialog
                    collection_id=id
                    collection_name=name
                    on_close=move || contact_coll_share_dialog_open.set(false)
                    on_change=Callback::new(move |_: Vec<String>| reload_contact_collections())
                />
            }
        })}
    }
}
