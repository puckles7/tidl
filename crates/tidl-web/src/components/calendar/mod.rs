//! Calendar view — month mini-calendar + week grid + new-event dialog

// pub(crate) so focus.rs (a sibling component, not a descendant of this
// module) can reuse today_ymd()/add_days() for its fixed lookahead window.
pub(crate) mod date_helpers;
use date_helpers::*;

mod layout;

mod mini_cal;
use mini_cal::MonthMiniCal;

mod event_form;
pub(crate) use event_form::EventFormDialog;
use event_form::LastUsedCalendarCtx;

mod week_grid;
use week_grid::WeekGrid;

use leptos::prelude::*;
use leptos_router::hooks::use_query_map;

use crate::api::{get_events, Event};
use crate::app::{
    use_events_refresh, use_sidebar_refresh, AllCalendarsCtx, CachedEventsCtx, CalendarLoadedCtx,
};
use crate::components::calendar_list_panel::CalendarListPanel;
use crate::components::collapsible_panel::CollapsiblePanel;
use crate::components::view_header::{format_today_heading, heading_with_clock, use_live_clock};

// ── Calendar display mode ─────────────────────────────────────────────────────

// pub(crate) (not pub(super)) so date_helpers::visible_range, which is used
// by focus.rs outside this module's parent, can expose it in its signature.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum CalMode {
    Day,
    WorkWeek,
    Week,
    Month,
}

// ── Top-level CalendarView ────────────────────────────────────────────────────

#[component]
pub fn CalendarView() -> impl IntoView {
    let (today_y, today_m, today_d) = today_ymd();
    let today_heading = chrono::NaiveDate::from_ymd_opt(today_y, today_m, today_d)
        .map(format_today_heading)
        .unwrap_or_default();

    // Deep-link target from Focus' "Show in calendar" (backlog #157):
    // `?highlight=<event uid>&date=<YYYY-MM-DD>` on a fresh full-page
    // navigation. `date` seeds the visible week/day/month so the event is
    // actually in the fetched range; `highlight` is pulsed once it renders.
    let (highlight_uid, deep_link_date) = {
        let q = use_query_map();
        let q = q.get_untracked();
        (
            q.get("highlight").filter(|s| !s.is_empty()),
            q.get("date").and_then(|s| parse_date(&s)),
        )
    };
    let (nav_y, nav_m, nav_d) = deep_link_date.unwrap_or((today_y, today_m, today_d));

    let week_monday = RwSignal::new(week_monday_of(nav_y, nav_m, nav_d));
    let mini_month = RwSignal::new((nav_y, nav_m));
    let cal_mode: RwSignal<CalMode> = RwSignal::new(CalMode::Week);
    // Free-text filter over the events shown in the main grid.
    let search: RwSignal<String> = RwSignal::new(String::new());
    let dialog_times: RwSignal<Option<(String, String)>> = RwSignal::new(None);
    // Some(event_id) when the edit dialog is open for an existing event.
    let edit_event_id: RwSignal<Option<String>> = RwSignal::new(None);
    // The day currently shown in mobile single-day view. Declared here
    // (rather than further down with the rest of the mobile-detection state)
    // because events_res below needs it to compute the visible date range.
    let mobile_day: RwSignal<(i32, u32, u32)> = RwSignal::new((nav_y, nav_m, nav_d));

    // ── Calendars ─────────────────────────────────────────────────────────────

    // Persisted in AuthenticatedApp context so this survives route unmount/remount,
    // and shared with Focus/Todos/Tickler — CalendarListPanel (rendered below) owns
    // the fetch and population of this context; this component only reads it to
    // filter events.
    let all_calendars_ctx =
        use_context::<AllCalendarsCtx>().expect("AllCalendarsCtx provided by AuthenticatedApp");

    // Tracks the last calendar the user added an event to so NewEventDialog
    // can re-use it as the default on the next open.
    let last_used_calendar = LastUsedCalendarCtx(RwSignal::new(String::new()));
    provide_context(last_used_calendar);

    let sidebar_refresh = use_sidebar_refresh();
    let events_refresh = use_events_refresh();

    // ── Events ────────────────────────────────────────────────────────────────

    // LocalResource skips SSR and only loads after WASM hydration, which
    // ensures SSR output and the initial hydration tree are identical (both
    // see an empty/None resource). The client then fills in reactively.
    //
    // NOTE: Switching to Resource::new (SSR-included) would require wrapping
    // the event-rendering content in a <Suspense> so the server streams events
    // into the initial HTML; without that boundary the server blocks the entire
    // response waiting for get_events() and events still only appear after WASM
    // anyway. That refactor is tracked as a future improvement.
    // Reactive to calendar navigation: reading these tracked signals inside
    // the resource closure means switching cal_mode or navigating the main
    // grid naturally triggers a refetch bound to the new visible range — no
    // extra plumbing needed. `cached_events` (below) keeps the previous
    // range's events on screen while the new range loads.
    //
    // `mini_month` is read only inside the `Month` branch, not
    // unconditionally: Leptos tracks whichever signals a reactive closure
    // actually reads *on a given run*, so this resource only depends on
    // (and refetches for) `mini_month` while `cal_mode` is `Month` — the one
    // case where `mini_month` really is the main grid's own driver (see
    // `visible_range`'s doc comment). Reading it unconditionally here would
    // re-run this fetch on every mini-cal page even in Week/Day/WorkWeek
    // mode, where the mini-cal's own month is irrelevant to what this
    // fetches; that used to also inflate the *range itself* (see
    // `mini_events_res` below and `visible_range`'s doc comment for the
    // full history of that bug).
    let events_res = LocalResource::new(move || {
        let _ = events_refresh.get(); // reactive: re-fetch when events change
        let mode = cal_mode.get();
        let mm = if mode == CalMode::Month {
            mini_month.get()
        } else {
            (0, 0)
        };
        let (start, end) = visible_range(mm, week_monday.get(), mode, mobile_day.get());
        let range_start = format!("{:04}-{:02}-{:02}", start.0, start.1, start.2);
        let range_end = format!("{:04}-{:02}-{:02}", end.0, end.1, end.2);
        async move { get_events(range_start, range_end).await.unwrap_or_default() }
    });

    // Cached events — persisted in AuthenticatedApp context so events stay visible
    // during any refetch (no blank while the new fetch is in flight).
    let cached_events =
        use_context::<CachedEventsCtx>().expect("CachedEventsCtx provided by AuthenticatedApp");

    // Independent fetch for the mini-calendar's own event dots/bars, bound
    // only to `mini_month` — deliberately *not* merged into `events_res`
    // above. The mini-cal can be paged (via its own prev/next buttons in
    // `mini_cal.rs`) to a month arbitrarily far from whatever the main grid
    // is showing; giving it its own small, independent request keeps that
    // navigation fast and its response size bounded, instead of widening
    // `events_res`'s range to cover the growing gap between the two (see
    // `visible_range`'s doc comment for the bug this replaced).
    let mini_events_res = LocalResource::new(move || {
        let _ = events_refresh.get();
        let (start, end) = mini_cal_range(mini_month.get());
        let range_start = format!("{:04}-{:02}-{:02}", start.0, start.1, start.2);
        let range_end = format!("{:04}-{:02}-{:02}", end.0, end.1, end.2);
        async move { get_events(range_start, range_end).await.unwrap_or_default() }
    });

    // Gate — persisted in AuthenticatedApp context so it stays true across
    // CalendarView unmount/remount cycles.  Only false on the very first ever
    // visit to the Calendar page; never reverts to false after that.
    let calendar_loaded = use_context::<CalendarLoadedCtx>()
        .expect("CalendarLoadedCtx provided by AuthenticatedApp")
        .0;

    // Calendar-list fetching/population now lives entirely in CalendarListPanel
    // (rendered below), shared with Focus/Todos/Tickler via AllCalendarsCtx —
    // this component only reacts to events_res.
    #[cfg(target_arch = "wasm32")]
    Effect::new(move |_| {
        if let Some(evs) = events_res.get() {
            // Only notify subscribers when events actually changed — prevents
            // spurious DOM rebuilds when a double-refetch returns identical data.
            if evs != cached_events.get_untracked() {
                cached_events.set(evs);
            }
            if !calendar_loaded.get_untracked() {
                calendar_loaded.set(true);
            }
        }
    });

    // Filter events to only those in visible calendars.
    // Memo so the filter only re-runs when all_calendars_ctx or cached_events
    // actually changes — Signal::derive would recompute on every .get() call, causing
    // a full 6000-event scan for each event card that reads the signal.
    let events_memo: Memo<Vec<Event>> = Memo::new(move |_| {
        #[cfg(target_arch = "wasm32")]
        let t0 = perf_now();
        // Keyed by (owner_username, calendar_name) — not bare calendar name —
        // so an owned and a shared calendar sharing a name never alias each
        // other's visibility (see CalendarEntry's doc comment).
        // `Event.owner_username` is populated for both owned and shared rows
        // (get_events sets it to the viewer's own username for owned rows),
        // so no server change is needed to match against it here.
        let visible: std::collections::HashSet<(String, String)> = all_calendars_ctx
            .get()
            .into_iter()
            .filter(|(_, v)| *v)
            .map(|(c, _)| (c.owner_username, c.calendar_name))
            .collect();
        let result = if visible.is_empty() {
            cached_events.get()
        } else {
            cached_events
                .get()
                .into_iter()
                .filter(|e| {
                    let key = (e.owner_username.clone(), e.calendar.clone());
                    visible.contains(&key) || e.calendar.is_empty()
                })
                .collect()
        };
        // Free-text filter: keep only events whose title/description match.
        let q = search.get().trim().to_lowercase();
        let mut result = result;
        if !q.is_empty() {
            result.retain(|e| {
                e.summary.to_lowercase().contains(&q)
                    || e.description
                        .as_deref()
                        .is_some_and(|d| d.to_lowercase().contains(&q))
            });
        }
        #[cfg(target_arch = "wasm32")]
        leptos::logging::log!(
            "[perf] events-filter: {:.2}ms ({} events)",
            perf_now() - t0,
            result.len()
        );
        result
    });
    let events: Signal<Vec<Event>> = events_memo.into();

    // One-shot pulse for the deep-linked event (backlog #157) — mirrors
    // Notes' `?highlight=` effect: wait until the event is actually in the
    // fetched/filtered set, then flash whichever grid element carries its
    // `data-event-uid` (timed block, all-day pill, or month line/bar).
    // Degrades to a plain navigation (no flash) when the event stays hidden
    // (deselected calendar, search filter, or out of range).
    let highlight_uid: RwSignal<Option<String>> = RwSignal::new(highlight_uid);
    #[cfg(target_arch = "wasm32")]
    Effect::new(move |_| {
        let Some(uid) = highlight_uid.get() else {
            return;
        };
        if !events.get().iter().any(|e| e.id == uid) {
            return;
        }
        use leptos::wasm_bindgen::prelude::Closure;
        use leptos::wasm_bindgen::JsCast;

        // Defer to the next animation frame so Leptos has patched the DOM for
        // an event that only just entered the rendered range.
        let flash = Closure::once(move || {
            let Some(doc) = leptos::web_sys::window().and_then(|w| w.document()) else {
                return;
            };
            let Ok(nodes) = doc.query_selector_all("[data-event-uid]") else {
                return;
            };
            for i in 0..nodes.length() {
                let Some(node) = nodes.item(i) else {
                    continue;
                };
                let Ok(node_el) = node.dyn_into::<leptos::web_sys::Element>() else {
                    continue;
                };
                if node_el.get_attribute("data-event-uid").as_deref() != Some(uid.as_str()) {
                    continue;
                }
                let el: leptos::web_sys::HtmlElement = node_el.unchecked_into();
                // Center the deep-linked event in the grid's scroll container
                // rather than pinning it to the top edge, so it lands in the
                // middle of the visible window when possible.
                let opts = leptos::web_sys::ScrollIntoViewOptions::new();
                opts.set_block(leptos::web_sys::ScrollLogicalPosition::Center);
                el.scroll_into_view_with_scroll_into_view_options(&opts);
                let _ = el.class_list().add_1("cal-event--highlight");
                // Self-terminating animation: strip the class on `animationend`
                // so a later grid re-render can't replay the pulse.
                let cleanup_el = el.clone();
                let end_closure = Closure::once(move || {
                    let _ = cleanup_el.class_list().remove_1("cal-event--highlight");
                });
                let _ = el.add_event_listener_with_callback(
                    "animationend",
                    end_closure.as_ref().unchecked_ref(),
                );
                end_closure.forget();
                break;
            }
        });
        if let Some(win) = leptos::web_sys::window() {
            let _ = win.request_animation_frame(flash.as_ref().unchecked_ref());
        }
        flash.forget();
        highlight_uid.set(None);
    });
    #[cfg(not(target_arch = "wasm32"))]
    let _ = highlight_uid;

    // Same visible-calendar filter as `events_memo`, applied to the
    // mini-cal's independent `mini_events_res` fetch instead of the main
    // grid's `cached_events` — see `mini_events_res`'s doc comment for why
    // it's a separate resource. No `cached_events`-style persistence here:
    // the mini-cal briefly showing no dots while `mini_month` changes is a
    // fine trade for not sharing state with the main grid's cache.
    let mini_events_memo: Memo<Vec<Event>> = Memo::new(move |_| {
        let visible: std::collections::HashSet<(String, String)> = all_calendars_ctx
            .get()
            .into_iter()
            .filter(|(_, v)| *v)
            .map(|(c, _)| (c.owner_username, c.calendar_name))
            .collect();
        let evs = mini_events_res.get().unwrap_or_default();
        if visible.is_empty() {
            evs
        } else {
            evs.into_iter()
                .filter(|e| {
                    let key = (e.owner_username.clone(), e.calendar.clone());
                    visible.contains(&key) || e.calendar.is_empty()
                })
                .collect()
        }
    });
    let mini_events: Signal<Vec<Event>> = mini_events_memo.into();

    // Browser-side alarm scheduler: start once on mount, then re-check and
    // refresh the event cache on every resource update.
    let notifications_blocked: RwSignal<bool> = RwSignal::new(false);
    let notif_banner_dismissed: RwSignal<bool> = RwSignal::new(false);
    #[cfg(feature = "hydrate")]
    {
        use crate::alarm_scheduler::{
            check_alarms, notification_permission_denied, start_alarm_scheduler, update_events,
        };
        Effect::new(move |prev: Option<()>| {
            if prev.is_none() {
                start_alarm_scheduler();
                notifications_blocked.set(notification_permission_denied());
            }
            if let Some(evs) = events_res.get() {
                update_events(evs.clone());
                check_alarms(&evs);
            }
        });
    }

    // Live clock — ticks every second in the browser, empty string on SSR.
    let clock_time = use_live_clock();

    // Mobile-only: each sidebar section independently collapsible.
    let mini_open: RwSignal<bool> = RwSignal::new(false);
    let cals_open: RwSignal<bool> = RwSignal::new(false);

    // Whether the viewport is in single-day mobile mode (≤767 px).
    let is_mobile: RwSignal<bool> = RwSignal::new(false);

    // Set up mobile detection: check once after hydration and on every resize.
    //
    // IMPORTANT: do NOT call measure() during the component body.  Doing so
    // sets is_mobile=true on mobile *before* the view! macro runs, which
    // produces a week grid with 1 column — but the server rendered 7 columns.
    // Tachys detects the structural mismatch and panics with
    // "Unrecoverable hydration error".
    //
    // Deferring to Effect::new means the initial hydration always uses the
    // same is_mobile=false that the server used, and the reactive update to
    // true fires immediately after hydration completes (causing a fast
    // client-side re-render to mobile layout, with no flicker in practice
    // because it happens before the browser paints).
    #[cfg(target_arch = "wasm32")]
    {
        use leptos::wasm_bindgen::prelude::Closure;
        use leptos::wasm_bindgen::JsCast;

        let measure = move || {
            let w = leptos::web_sys::window()
                .and_then(|w| w.inner_width().ok())
                .and_then(|v| v.as_f64())
                .unwrap_or(1024.0);
            is_mobile.set(w <= 767.0);
        };

        // Deferred initial check — runs after hydration, safe from SSR mismatch.
        Effect::new(move |_: Option<()>| {
            measure();
        });

        // Keep updating on resize so the view can switch back to week mode.
        let closure = Closure::<dyn Fn()>::new(measure);
        if let Some(win) = leptos::web_sys::window() {
            let _ =
                win.add_event_listener_with_callback("resize", closure.as_ref().unchecked_ref());
        }
        // The component lives for the app's lifetime, so we intentionally leak
        // the closure rather than managing a cleanup handle.
        closure.forget();
    }

    view! {
        <div class="calendar-view">
            {move || if calendar_loaded.get() {
                view! {
                // Flex row so the sidebar's top edge sits level with the
                // date/mode pills, matching how .notes-sidebar lines up with
                // the Starred toggle on the Notes tab — a grid row above the
                // sidebar would push it down by that row's own height
                // instead of sitting level with it.
                <div class="sidebar-layout calendar-layout">
                <aside class="filter-sidebar calendar-sidebar">
                    // ── Month panel ───────────────────────────────────────────
                    <CollapsiblePanel title="Month" open=mini_open>
                        <div class="sidebar-section">
                            <MonthMiniCal
                                events=mini_events month=mini_month
                                week_monday=week_monday today=(today_y, today_m, today_d)
                                mobile_day=mobile_day
                            />
                        </div>
                    </CollapsiblePanel>
                    // ── Calendars panel ───────────────────────────────────────
                    <CollapsiblePanel title="Calendars" open=cals_open>
                        <div class="sidebar-section">
                            <CalendarListPanel
                                on_refresh=Callback::new(move |_| {
                                    // CalendarListPanel owns cals_res/shared_cals_res
                                    // internally and refetches them off sidebar_refresh —
                                    // same signal-bump idiom focus_sidebar.rs already
                                    // uses, rather than this component reaching into a
                                    // child's private resources.
                                    sidebar_refresh.update(|n| *n += 1);
                                    events_refresh.update(|n| *n += 1);
                                })
                            />
                        </div>
                    </CollapsiblePanel>
                </aside>
                <main class="calendar-main">
                    {move || (notifications_blocked.get() && !notif_banner_dismissed.get()).then(|| view! {
                        <div class="notif-warning-banner">
                            <span>
                                "Browser notifications are blocked, so calendar alarms won't show a desktop alert. \
                                 This usually happens when tidl is served over plain HTTP from anything other than \
                                 localhost — browsers only grant notification permission on HTTPS or localhost origins. \
                                 Serve tidl behind an HTTPS reverse proxy, or open it via http://localhost, to fix this."
                            </span>
                            <button
                                class="notif-warning-dismiss"
                                on:click=move |_| notif_banner_dismissed.set(true)
                            >"×"</button>
                        </div>
                    })}
                    <div class="cal-pill-row">
                        <p class="focus-date">
                            {
                                let h = today_heading.clone();
                                move || heading_with_clock(&h, clock_time)
                            }
                        </p>
                        <div class="focus-right-controls">
                            <div class="focus-horizon-ctrl cal-mode-ctrl">
                                <button
                                    class=move || if cal_mode.get()==CalMode::Day {"horizon-btn horizon-btn--active"} else {"horizon-btn"}
                                    on:click=move |_| cal_mode.set(CalMode::Day)
                                >"Day"</button>
                                <button
                                    class=move || if cal_mode.get()==CalMode::WorkWeek {"horizon-btn horizon-btn--active"} else {"horizon-btn"}
                                    on:click=move |_| cal_mode.set(CalMode::WorkWeek)
                                >"Work week"</button>
                                <button
                                    class=move || if cal_mode.get()==CalMode::Week {"horizon-btn horizon-btn--active"} else {"horizon-btn"}
                                    on:click=move |_| cal_mode.set(CalMode::Week)
                                >"Week"</button>
                                <button
                                    class=move || if cal_mode.get()==CalMode::Month {"horizon-btn horizon-btn--active"} else {"horizon-btn"}
                                    on:click=move |_| cal_mode.set(CalMode::Month)
                                >"Month"</button>
                            </div>
                            <input
                                class="form-input notes-search"
                                type="text"
                                placeholder="Search events…"
                                prop:value=move || search.get()
                                on:input=move |ev| search.set(event_target_value(&ev))
                            />
                        </div>
                    </div>
                    <WeekGrid
                        events=events
                        cached_events=cached_events
                        on_refetch=Callback::new(move |_| events_res.refetch())
                        week_monday=week_monday
                        mini_month=mini_month today=(today_y, today_m, today_d)
                        cal_mode=cal_mode
                        dialog_times=dialog_times
                        edit_event_id=edit_event_id
                        is_mobile=is_mobile
                        mobile_day=mobile_day
                    />
                </main>
            </div>
                }.into_any()
            } else {
                view! { <p class="calendar-loading">"Loading…"</p> }.into_any()
            }}
            {move || dialog_times.get().map(|(start, end)| view! {
                <EventFormDialog
                    event=None
                    start=start end=end
                    on_close=move || dialog_times.set(None)
                    on_saved=move || { dialog_times.set(None); events_res.refetch(); }
                />
            })}
            {move || edit_event_id.get().and_then(|uid| {
                let ev = events.get()
                    .into_iter().find(|e| e.id == uid)?;
                // All-day: start/end are already "YYYY-MM-DD"; timed: convert to "YYYY-MM-DDTHH:MM".
                let start = if ev.all_day { ev.start.clone() } else { input_dt_from_server(&ev.start) };
                let end   = if ev.all_day { ev.end.clone()   } else { input_dt_from_server(&ev.end)   };
                Some(view! {
                    <EventFormDialog
                        event=Some(ev)
                        start=start
                        end=end
                        on_close=move || edit_event_id.set(None)
                        on_saved=move || { edit_event_id.set(None); events_res.refetch(); }
                    />
                })
            })}
        </div>
    }
}
