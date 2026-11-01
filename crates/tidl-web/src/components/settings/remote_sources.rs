use leptos::prelude::*;

use super::CalsResource;
use crate::api::{
    add_remote_source, delete_remote_source, discover_remote_calendars, list_remote_sources,
    trigger_remote_caldav_sync, update_remote_source, NewRemoteCalDavSource, RemoteCalDavSource,
    UpdateRemoteCalDavSource, MIN_SYNC_INTERVAL_SECS,
};
use crate::util::format_server_fn_error;

#[component]
pub(super) fn RemoteSourcesPanel(cals_res: CalsResource) -> impl IntoView {
    let sources_list = RwSignal::new(Vec::<RemoteCalDavSource>::new());
    let sources_loading = RwSignal::new(true);
    let src_name = RwSignal::new(String::new());
    let src_url = RwSignal::new(String::new());
    let src_user = RwSignal::new(String::new());
    let src_token = RwSignal::new(String::new());
    let src_remote_cal = RwSignal::new(String::new());
    let src_discovered = RwSignal::new(Vec::<String>::new());
    let src_discovering = RwSignal::new(false);
    let src_disc_error = RwSignal::new(Option::<String>::None);
    let src_cal_pick = RwSignal::new(String::new());
    let src_cal_new = RwSignal::new(String::new());
    let src_direction = RwSignal::new("both".to_string());
    let src_interval = RwSignal::new(300u32);
    let src_adding = RwSignal::new(false);
    let src_error = RwSignal::new(Option::<String>::None);
    let src_syncing_id: RwSignal<Option<String>> = RwSignal::new(None);
    let editing_id: RwSignal<Option<String>> = RwSignal::new(None);
    // The local calendar the source was already mapped to when the edit
    // dialog opened. A save that leaves `src_cal_pick` equal to this value
    // doesn't change `local_calendar`, so the backend's `resync_needed`
    // check (api/calendars.rs `update_remote_source_tx`) stays false and no
    // wipe happens — the "will be deleted" warning would be a false alarm.
    let editing_orig_local_cal: RwSignal<Option<String>> = RwSignal::new(None);

    let reload_sources = move || {
        sources_loading.set(true);
        leptos::task::spawn_local(async move {
            if let Ok(list) = list_remote_sources().await {
                sources_list.set(list);
            }
            sources_loading.set(false);
        });
    };

    // Spawn a polling loop that reloads the sources list every 3 s while any
    // source has never completed a sync.  Gated to the WASM target because the
    // sleep helper is JS-based; on SSR there is nothing to poll.
    let start_sync_poll = move || {
        #[cfg(feature = "hydrate")]
        leptos::task::spawn_local(async move {
            loop {
                // Sleep ~3 s via a JS Promise so we don't block the WASM thread.
                let promise = js_sys::Promise::new(&mut |resolve, _| {
                    let _ = web_sys::window()
                        .unwrap()
                        .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 3_000);
                });
                let _ = wasm_bindgen_futures::JsFuture::from(promise).await;

                // Fetch the latest list.
                if let Ok(list) = list_remote_sources().await {
                    let still_syncing = list
                        .iter()
                        .any(|s| s.last_synced_at.is_none() && s.last_error.is_none());
                    sources_list.set(list);
                    sources_loading.set(false);
                    if !still_syncing {
                        break;
                    }
                } else {
                    break;
                }
            }
        });
    };

    // Load sources on mount; start poll if any are in progress.
    leptos::task::spawn_local(async move {
        if let Ok(list) = list_remote_sources().await {
            let needs_poll = list
                .iter()
                .any(|s| s.last_synced_at.is_none() && s.last_error.is_none());
            sources_list.set(list);
            if needs_poll {
                start_sync_poll();
            }
        }
        sources_loading.set(false);
    });

    // Poll every 3 s until `last_synced_at` changes for the triggered source,
    // then clear src_syncing_id. Capped at 10 iterations (~30 s).
    #[cfg(not(feature = "hydrate"))]
    let start_caldav_sync_poll = |_: String| {};
    #[cfg(feature = "hydrate")]
    let start_caldav_sync_poll = move |poll_id: String| {
        leptos::task::spawn_local(async move {
            let old_ts = sources_list
                .get_untracked()
                .into_iter()
                .find(|s| s.id == poll_id)
                .and_then(|s| s.last_synced_at.clone());

            for _ in 0..10u8 {
                let promise = js_sys::Promise::new(&mut |resolve, _| {
                    let _ = web_sys::window()
                        .unwrap()
                        .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, 3_000);
                });
                let _ = wasm_bindgen_futures::JsFuture::from(promise).await;

                if let Ok(list) = list_remote_sources().await {
                    let new_ts = list
                        .iter()
                        .find(|s| s.id == poll_id)
                        .and_then(|s| s.last_synced_at.clone());
                    sources_list.set(list);
                    sources_loading.set(false);
                    if new_ts != old_ts {
                        break;
                    }
                } else {
                    break;
                }
            }
            src_syncing_id.set(None);
        });
    };

    // Setting `src_error` alone is invisible if the dialog is scrolled past
    // the top of the form — the banner renders above the fold. Scroll the
    // dialog's scrollable body back to the top whenever an error appears so
    // it's always in view.
    let set_src_error = move |msg: String| {
        src_error.set(Some(msg));
        #[cfg(feature = "hydrate")]
        {
            use wasm_bindgen::JsCast;
            if let Some(el) = web_sys::window()
                .and_then(|w| w.document())
                .and_then(|d| d.query_selector(".dialog-form").ok().flatten())
                .and_then(|el| el.dyn_into::<web_sys::HtmlElement>().ok())
            {
                el.set_scroll_top(0);
            }
        }
    };

    let on_discover = move |_: leptos::ev::MouseEvent| {
        if src_discovering.get_untracked() {
            return;
        }
        let url = src_url.get_untracked();
        let user = src_user.get_untracked();
        let tok = src_token.get_untracked();
        if url.trim().is_empty() || tok.trim().is_empty() {
            return;
        }
        src_discovering.set(true);
        src_disc_error.set(None);
        leptos::task::spawn_local(async move {
            match discover_remote_calendars(url, user, tok).await {
                Ok(cals) => {
                    if cals.len() == 1 {
                        src_remote_cal.set(cals[0].clone());
                    }
                    src_discovered.set(cals);
                }
                Err(e) => src_disc_error.set(Some(format_server_fn_error(&e.to_string()))),
            }
            src_discovering.set(false);
        });
    };

    let reset_form = move || {
        src_name.set(String::new());
        src_url.set(String::new());
        src_user.set(String::new());
        src_token.set(String::new());
        src_remote_cal.set(String::new());
        src_discovered.set(Vec::new());
        src_disc_error.set(None);
        src_cal_pick.set(String::new());
        src_cal_new.set(String::new());
        src_direction.set("both".to_string());
        src_interval.set(300);
        editing_id.set(None);
        editing_orig_local_cal.set(None);
    };

    let on_edit_source = move |src: RemoteCalDavSource| {
        src_error.set(None);
        src_name.set(src.name);
        src_url.set(src.remote_base_url);
        src_user.set(src.remote_username);
        src_token.set(String::new());
        src_remote_cal.set(src.remote_calendar);
        src_discovered.set(Vec::new());
        src_disc_error.set(None);
        src_cal_pick.set(src.local_calendar.clone());
        src_cal_new.set(String::new());
        src_direction.set(src.sync_direction);
        src_interval.set(src.sync_interval_secs);
        editing_id.set(Some(src.id));
        editing_orig_local_cal.set(Some(src.local_calendar));
    };

    let on_submit = move |_: leptos::ev::MouseEvent| {
        if src_adding.get_untracked() {
            return;
        }
        let name = src_name.get_untracked();
        let url = src_url.get_untracked();
        let user = src_user.get_untracked();
        let tok = src_token.get_untracked();
        let remote_cal = src_remote_cal.get_untracked();
        let direction = src_direction.get_untracked();
        let interval = src_interval.get_untracked();
        let editing = editing_id.get_untracked();

        let pick = src_cal_pick.get_untracked();

        if name.trim().is_empty() {
            set_src_error("Name is required".into());
            return;
        }
        if url.trim().is_empty() {
            set_src_error("Remote server URL is required".into());
            return;
        }
        // The token is required to create a source, but optional when
        // editing one — blank means "keep the existing token".
        if editing.is_none() && tok.trim().is_empty() {
            set_src_error("App token is required".into());
            return;
        }
        if remote_cal.trim().is_empty() {
            set_src_error("Remote calendar is required".into());
            return;
        }
        if pick.trim().is_empty() {
            set_src_error("Local calendar is required".into());
            return;
        }
        if pick == "__new__" && src_cal_new.get_untracked().trim().is_empty() {
            set_src_error("New calendar slug is required".into());
            return;
        }
        if interval < MIN_SYNC_INTERVAL_SECS {
            set_src_error(format!(
                "Sync interval must be at least {MIN_SYNC_INTERVAL_SECS} seconds"
            ));
            return;
        }

        src_adding.set(true);
        src_error.set(None);

        let cal = if pick == "__new__" {
            src_cal_new.get_untracked()
        } else {
            pick
        };

        if let Some(id) = editing {
            let update = UpdateRemoteCalDavSource {
                name,
                remote_base_url: url,
                remote_username: user,
                remote_token: Some(tok).filter(|t| !t.is_empty()),
                remote_calendar: remote_cal,
                local_calendar: cal,
                sync_direction: direction,
                sync_interval_secs: interval,
            };
            leptos::task::spawn_local(async move {
                match update_remote_source(id.clone(), update).await {
                    Ok(()) => {
                        reset_form();
                        reload_sources();
                        // Re-exercise the edited settings right away, same
                        // inline-spinner treatment as adding/manually syncing.
                        src_syncing_id.set(Some(id.clone()));
                        start_caldav_sync_poll(id);
                    }
                    Err(e) => {
                        set_src_error(format_server_fn_error(&e.to_string()));
                    }
                }
                src_adding.set(false);
            });
            return;
        }

        let source = NewRemoteCalDavSource {
            name,
            remote_base_url: url,
            remote_username: user,
            remote_token: tok,
            remote_calendar: remote_cal,
            local_calendar: cal,
            sync_direction: direction,
            sync_interval_secs: interval,
        };
        leptos::task::spawn_local(async move {
            match add_remote_source(source).await {
                Ok(new_id) => {
                    reset_form();
                    reload_sources();
                    // Track the new source's first sync the same way a manually
                    // triggered one is tracked — an inline spinner on its row —
                    // instead of a separate top-level "syncing now" banner.
                    src_syncing_id.set(Some(new_id.clone()));
                    start_caldav_sync_poll(new_id);
                }
                Err(e) => {
                    set_src_error(format_server_fn_error(&e.to_string()));
                }
            }
            src_adding.set(false);
        });
    };

    view! {
        <h3 class="settings-section-heading">
            {move || if editing_id.get().is_some() { "Edit remote CalDAV source" } else { "Add remote CalDAV source" }}
        </h3>
        <p class="sharing-empty">
            "Sync with another tidl instance. Generate an app token on the remote server first, then paste it here."
        </p>

        {move || src_error.get().map(|e| view! { <div class="form-error">{e}</div> })}

        <div class="form-group">
            <label class="form-label">"Name"</label>
            <input
                type="text"
                class="form-input"
                placeholder="e.g. Personal server"
                prop:value=move || src_name.get()
                on:input=move |ev| src_name.set(event_target_value(&ev))
            />
        </div>
        <div class="form-group">
            <label class="form-label">"Remote server URL"</label>
            <input
                type="url"
                class="form-input"
                placeholder="https://personal.example.com"
                prop:value=move || src_url.get()
                on:input=move |ev| src_url.set(event_target_value(&ev))
            />
        </div>
        <div class="form-group">
            <label class="form-label">"Remote username"</label>
            <input
                type="text"
                class="form-input"
                placeholder="alice"
                prop:value=move || src_user.get()
                on:input=move |ev| src_user.set(event_target_value(&ev))
            />
        </div>
        <div class="form-group">
            <label class="form-label">"App token (from remote server)"</label>
            <input
                type="password"
                class="form-input"
                placeholder=move || if editing_id.get().is_some() {
                    "Leave blank to keep the existing token"
                } else {
                    "tidl_…"
                }
                prop:value=move || src_token.get()
                on:input=move |ev| src_token.set(event_target_value(&ev))
            />
        </div>
        <div class="form-group">
            <label class="form-label">"Remote calendar"</label>
            <div class="form-row">
                {move || {
                    let discovered = src_discovered.get();
                    if discovered.is_empty() {
                        view! {
                            <input
                                type="text"
                                class="form-input"
                                placeholder="e.g. personal"
                                prop:value=move || src_remote_cal.get()
                                on:input=move |ev| src_remote_cal.set(event_target_value(&ev))
                            />
                        }.into_any()
                    } else {
                        view! {
                            <select
                                class="form-input"
                                prop:value=move || src_remote_cal.get()
                                on:change=move |ev| src_remote_cal.set(event_target_value(&ev))
                            >
                                <option value="" disabled=true>"— select —"</option>
                                {discovered.into_iter().map(|c| {
                                    let v = c.clone();
                                    view! { <option value=c>{v}</option> }
                                }).collect_view()}
                            </select>
                        }.into_any()
                    }
                }}
                <button
                    class="btn-secondary"
                    type="button"
                    disabled=move || src_discovering.get()
                        || src_url.get().trim().is_empty()
                        || src_token.get().trim().is_empty()
                    on:click=on_discover
                >
                    {move || if src_discovering.get() { "Discovering…" } else { "Discover" }}
                </button>
            </div>
            {move || src_disc_error.get().map(|e| view! {
                <div class="form-error">{e}</div>
            })}
        </div>
        <div class="form-group">
            <label class="form-label">"Local calendar (on this server)"</label>
            <select
                class="form-input"
                prop:value=move || src_cal_pick.get()
                on:change=move |ev| src_cal_pick.set(event_target_value(&ev))
            >
                <option value="" disabled=true>"— pick a calendar —"</option>
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
                <option value="__new__">"New calendar…"</option>
            </select>
            <Show when=move || {
                let p = src_cal_pick.get();
                if p.is_empty() || p == "__new__" { return false; }
                // In edit mode, picking back the calendar the source already
                // owned is a no-op on the backend (see `editing_orig_local_cal`
                // above) — only warn if the pick actually changed.
                match editing_orig_local_cal.get() {
                    Some(orig) => p != orig,
                    None => true,
                }
            }>
                <div class="form-warning">
                    "Calendar already exists — "
                    <strong>"all of its existing events will be deleted"</strong>
                    " and replaced by this sync. This calendar then behaves like a \
                     fresh one going forward."
                </div>
            </Show>
            <Show when=move || editing_id.get().is_some()>
                <p class="dialog-hint">
                    "Events left in the previous local calendar (if you're changing it) \
                     are untouched — they just stop being synced."
                </p>
            </Show>
        </div>
        <Show when=move || src_cal_pick.get() == "__new__">
            <div class="form-group">
                <label class="form-label">"New calendar slug"</label>
                <input
                    type="text"
                    class="form-input"
                    placeholder="e.g. personal"
                    prop:value=move || src_cal_new.get()
                    on:input=move |ev| src_cal_new.set(event_target_value(&ev))
                />
            </div>
        </Show>
        <div class="form-group">
            <label class="form-label">"Sync direction"</label>
            <select
                class="form-input"
                prop:value=move || src_direction.get()
                on:change=move |ev| src_direction.set(event_target_value(&ev))
            >
                <option value="both">"Both (pull and push)"</option>
                <option value="pull">"Pull only (read from remote)"</option>
                <option value="push">"Push only (write to remote)"</option>
            </select>
        </div>
        <div class="form-group">
            <label class="form-label">"Sync interval (seconds)"</label>
            <input
                type="number"
                class="form-input"
                min=MIN_SYNC_INTERVAL_SECS.to_string()
                prop:value=move || src_interval.get().to_string()
                on:input=move |ev| {
                    if let Ok(v) = event_target_value(&ev).parse::<u32>() {
                        src_interval.set(v);
                    }
                }
            />
            <Show when=move || src_interval.get() < MIN_SYNC_INTERVAL_SECS>
                <div class="form-error">
                    "Minimum " {MIN_SYNC_INTERVAL_SECS} " seconds"
                </div>
            </Show>
        </div>
        <button
            class="btn-primary"
            disabled=move || src_adding.get()
            on:click=on_submit
        >
            {move || match (editing_id.get().is_some(), src_adding.get()) {
                (true, true) => "Saving…",
                (true, false) => "Save changes",
                (false, true) => "Adding…",
                (false, false) => "Add source",
            }}
        </button>
        <Show when=move || editing_id.get().is_some()>
            <button
                class="btn-secondary"
                type="button"
                on:click=move |_| reset_form()
            >"Cancel"</button>
        </Show>

        <h3 class="settings-section-heading">"Configured sources"</h3>
        {move || {
            let list = sources_list.get();
            if sources_loading.get() {
                return view! { <p class="sharing-empty">"Loading…"</p> }.into_any();
            }
            if list.is_empty() {
                return view! { <p class="sharing-empty">"No remote sources configured."</p> }.into_any();
            }
            list.into_iter().map(|src| {
                let id = src.id.clone();
                let id_spinner = id.clone();
                let rc = src.remote_calendar.clone();
                let lc = src.local_calendar.clone();
                // Always show both sides of the mapping.
                let remote_label = if rc.is_empty() { lc.clone() } else { rc };
                let local_label  = lc;
                // Encode sync direction as an arrow between the two calendar names.
                // Display is "remote  arrow  local", so:
                //   pull  = remote ⟶ local  (data flows in from remote)
                //   push  = remote ⟵ local  (data flows out to remote)
                //   both  = remote ⟷ local
                let dir_arrow = match src.sync_direction.as_str() {
                    "push" => "⟵",
                    "pull" => "⟶",
                    _      => "⟷", // "both" or anything else
                };
                view! {
                    <div class="sharing-row caldav-source-row">
                        <div class="caldav-source-info">
                            <div class="caldav-source-name">
                                {src.name.clone()}
                                <span class="caldav-source-url">{src.remote_base_url.clone()}</span>
                            </div>
                            <div class="caldav-source-meta">
                                <span class="sharing-perm caldav-cal-map">
                                    <span class="caldav-cal-remote">{remote_label}</span>
                                    {format!(" {dir_arrow} ")}
                                    <span class="caldav-cal-local">{local_label}</span>
                                </span>
                                <span class="caldav-source-url">"every " {src.sync_interval_secs} "s"</span>
                                {src.last_error.clone().map(|e| view! {
                                    <span class="form-error">"⚠ " {e}</span>
                                })}
                                {src.last_synced_at.clone().map(|t| view! {
                                    <span class="sharing-perm">"last sync: " {t}</span>
                                })}
                                {move || (src_syncing_id.get().as_deref() == Some(&id_spinner)).then(|| view! {
                                    <span class="sync-badge">
                                        <span class="sync-spinner"></span>
                                        "Syncing…"
                                    </span>
                                })}
                            </div>
                        </div>
                        {
                            let id_sync  = id.clone();
                            let id_sync2 = id.clone();
                            let id_label = id.clone();
                            view! {
                                <button
                                    class="btn-secondary-small"
                                    disabled=move || src_syncing_id.get().as_deref() == Some(&id_sync)
                                    on:click=move |_| {
                                        let trigger_id = id_sync2.clone();
                                        src_syncing_id.set(Some(trigger_id.clone()));
                                        leptos::task::spawn_local(async move {
                                            let _ = trigger_remote_caldav_sync(trigger_id.clone()).await;
                                            start_caldav_sync_poll(trigger_id);
                                        });
                                    }
                                >
                                    {move || {
                                        if src_syncing_id.get().as_deref() == Some(&id_label) {
                                            "Syncing…"
                                        } else {
                                            "Sync now"
                                        }
                                    }}
                                </button>
                            }
                        }
                        <button
                            class="btn-secondary-small"
                            on:click={
                                let src_for_edit = src.clone();
                                move |_| on_edit_source(src_for_edit.clone())
                            }
                        >"Edit"</button>
                        <button
                            class="btn-danger-small"
                            on:click=move |_| {
                                let id2 = id.clone();
                                leptos::task::spawn_local(async move {
                                    let _ = delete_remote_source(id2).await;
                                    reload_sources();
                                });
                            }
                        >"Remove"</button>
                    </div>
                }
            }).collect_view().into_any()
        }}
    }
}
