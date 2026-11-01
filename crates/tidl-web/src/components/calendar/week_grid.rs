//! Week time-grid and month grid views with drag-and-drop support.

#[cfg(target_arch = "wasm32")]
use super::date_helpers::perf_now;
use super::date_helpers::{
    add_days, dt_to_server, event_position_from, format_hm, format_hour_label, local_now_minutes,
    mins_to_hhmm, month_label, parse_dt, short_time, snap15, snap30, to_input_dt, week_monday_of,
    weekday_idx, weekday_of_first, MONTH_SHORT, WEEKDAY_SHORT,
};
use super::layout::{compute_allday_layout, compute_layout};
use super::CalMode;
use crate::api::{delete_event, update_event_times, Event};
use crate::components::settings::UserPrefsCtx;
use leptos::prelude::*;

// ── Drag state ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum DragKind {
    Move,
    Resize,
}

#[derive(Clone, Debug)]
pub struct DragState {
    pub uid: String,
    pub kind: DragKind,
    /// Day the event currently occupies (updated live for cross-column move).
    pub day: (i32, u32, u32),
    /// Original event start, minutes from midnight.
    pub orig_start_m: i32,
    /// Original event duration, minutes (held constant during Move).
    pub orig_dur_m: i32,
    /// Pointer position at the moment pointerdown fired (client coords).
    pub origin_x: f64,
    pub origin_y: f64,
    /// (scrollContainer.getBoundingClientRect().top - scrollTop) at drag start.
    pub col_y_offset: f64,
    /// Live start/end in minutes — updated on every pointermove.
    pub live_start_m: i32,
    pub live_end_m: i32,
    /// Maximum distance (px) the pointer has travelled from the origin.
    /// A click is < 5px; anything more is treated as an intentional drag.
    pub drag_px: f64,
}

// ── Vertical zoom (backlog #96) ───────────────────────────────────────────────

/// Default grid scale: 60px per hour (the historical 1440px day). The scale is
/// exposed to CSS as `--hour-h`, so every fixed-pixel assumption in the grid
/// (hour label/slot heights, `.week-day-body` height, the now-line's `top`)
/// scales with it; event blocks already use percentages and scale for free.
const HOUR_H_DEFAULT: f64 = 60.0;
const HOUR_H_MIN: f64 = 24.0;
const HOUR_H_MAX: f64 = 120.0;
/// Discrete zoom levels — `zoom_in`/`zoom_out` step through these rather than
/// free-scrolling, so the grid always lands on a tidy, reproducible scale.
const HOUR_H_STEPS: [f64; 8] = [24.0, 30.0, 36.0, 48.0, 60.0, 72.0, 96.0, 120.0];
/// Hour placed at the top of the viewport on first mount (backlog #96): a fixed
/// early-morning window catches most events, where centering "now" did not.
/// Only referenced by the wasm-only mount effect, so gate the const to match.
#[cfg(target_arch = "wasm32")]
const DEFAULT_VIEW_START_HOUR: f64 = 7.0;
/// localStorage key for the persisted zoom level (per-browser, not per-user).
#[cfg(feature = "hydrate")]
const HOUR_H_LS_KEY: &str = "tidl-cal-hour-h";

// ── WeekGrid ──────────────────────────────────────────────────────────────────

#[component]
// col_idx is only read inside `#[cfg(target_arch = "wasm32")]` perf-logging
// calls; clippy's native-target pass sees it as unused and would strip the
// `.enumerate()` needed for wasm32 builds.
#[allow(clippy::unused_enumerate_index)]
pub fn WeekGrid(
    events: Signal<Vec<Event>>,
    cached_events: RwSignal<Vec<Event>>,
    on_refetch: Callback<()>,
    week_monday: RwSignal<(i32, u32, u32)>,
    mini_month: RwSignal<(i32, u32)>,
    today: (i32, u32, u32),
    cal_mode: RwSignal<CalMode>,
    dialog_times: RwSignal<Option<(String, String)>>,
    edit_event_id: RwSignal<Option<String>>,
    is_mobile: RwSignal<bool>,
    mobile_day: RwSignal<(i32, u32, u32)>,
) -> impl IntoView {
    let do_prev = move || match cal_mode.get() {
        CalMode::Month => {
            mini_month.update(|(y, m)| {
                if *m == 1 {
                    *y -= 1;
                    *m = 12;
                } else {
                    *m -= 1;
                }
            });
        }
        CalMode::Day => {
            let (y, m, d) = mobile_day.get();
            let p = add_days(y, m, d, -1);
            mobile_day.set(p);
            week_monday.set(week_monday_of(p.0, p.1, p.2));
            mini_month.set((p.0, p.1));
        }
        CalMode::WorkWeek | CalMode::Week => {
            if is_mobile.get() && cal_mode.get() == CalMode::Week {
                let (y, m, d) = mobile_day.get();
                let p = add_days(y, m, d, -1);
                mobile_day.set(p);
                week_monday.set(week_monday_of(p.0, p.1, p.2));
                mini_month.set((p.0, p.1));
            } else {
                week_monday.update(|(y, m, d)| {
                    let p = add_days(*y, *m, *d, -7);
                    (*y, *m, *d) = (p.0, p.1, p.2);
                    mini_month.set((*y, *m));
                });
            }
        }
    };
    let do_next = move || match cal_mode.get() {
        CalMode::Month => {
            mini_month.update(|(y, m)| {
                if *m == 12 {
                    *y += 1;
                    *m = 1;
                } else {
                    *m += 1;
                }
            });
        }
        CalMode::Day => {
            let (y, m, d) = mobile_day.get();
            let n = add_days(y, m, d, 1);
            mobile_day.set(n);
            week_monday.set(week_monday_of(n.0, n.1, n.2));
            mini_month.set((n.0, n.1));
        }
        CalMode::WorkWeek | CalMode::Week => {
            if is_mobile.get() && cal_mode.get() == CalMode::Week {
                let (y, m, d) = mobile_day.get();
                let n = add_days(y, m, d, 1);
                mobile_day.set(n);
                week_monday.set(week_monday_of(n.0, n.1, n.2));
                mini_month.set((n.0, n.1));
            } else {
                week_monday.update(|(y, m, d)| {
                    let n = add_days(*y, *m, *d, 7);
                    (*y, *m, *d) = (n.0, n.1, n.2);
                    mini_month.set((*y, *m));
                });
            }
        }
    };
    let prev_week = move |_| {
        do_prev();
    };
    let next_week = move |_| {
        do_next();
    };

    // Touch-swipe state: records the (x, y) at touchstart so touchend can
    // measure the displacement and decide whether it was a horizontal swipe.
    let touch_start: RwSignal<Option<(f64, f64)>> = RwSignal::new(None);
    let go_today = move |_| {
        let (ty, tm, td) = today;
        if cal_mode.get() == CalMode::Month {
            mini_month.set((ty, tm));
        } else {
            mobile_day.set((ty, tm, td));
            let mon = week_monday_of(ty, tm, td);
            week_monday.set(mon);
            mini_month.set((mon.0, mon.1));
        }
    };

    let time_fmt = move || {
        use_context::<UserPrefsCtx>()
            .map(|s| s.get().time_format.clone())
            .unwrap_or_else(|| "12h".to_string())
    };

    let week_days = move || -> Vec<(i32, u32, u32)> {
        match cal_mode.get() {
            CalMode::Day => vec![mobile_day.get()],
            CalMode::WorkWeek => {
                let (y, m, d) = week_monday.get();
                (0..5i32).map(|i| add_days(y, m, d, i)).collect()
            }
            CalMode::Week => {
                if is_mobile.get() {
                    vec![mobile_day.get()]
                } else {
                    let (y, m, d) = week_monday.get();
                    (0..7i32).map(|i| add_days(y, m, d, i)).collect()
                }
            }
            CalMode::Month => vec![],
        }
    };

    let week_label = move || match cal_mode.get() {
        CalMode::Month => {
            let (y, m) = mini_month.get();
            month_label(y, m)
        }
        CalMode::Day => {
            let (dy, dm, dd) = mobile_day.get();
            let wi = weekday_idx(dy, dm, dd);
            let yr = if dy != today.0 {
                format!(" {}", dy)
            } else {
                String::new()
            };
            format!(
                "{} {} {}{}",
                WEEKDAY_SHORT[wi],
                dd,
                MONTH_SHORT[(dm as usize) - 1],
                yr
            )
        }
        _ => {
            let days = week_days();
            if days.is_empty() {
                return String::new();
            }
            if days.len() == 1 {
                let (dy, dm, dd) = days[0];
                let wi = weekday_idx(dy, dm, dd);
                let yr = if dy != today.0 {
                    format!(" {}", dy)
                } else {
                    String::new()
                };
                format!(
                    "{} {} {}{}",
                    WEEKDAY_SHORT[wi],
                    dd,
                    MONTH_SHORT[(dm as usize) - 1],
                    yr
                )
            } else {
                let (sy, sm, sd) = days[0];
                let (ey, em, ed) = days[days.len() - 1];
                let wi_s = weekday_idx(sy, sm, sd);
                let wi_e = weekday_idx(ey, em, ed);
                let yr = if sy != today.0 {
                    format!(" {}", sy)
                } else {
                    String::new()
                };
                if sm == em {
                    format!(
                        "{} {} – {} {} {}{}",
                        WEEKDAY_SHORT[wi_s],
                        sd,
                        WEEKDAY_SHORT[wi_e],
                        ed,
                        MONTH_SHORT[(em as usize) - 1],
                        yr
                    )
                } else {
                    format!(
                        "{} {} {} – {} {} {}{}",
                        WEEKDAY_SHORT[wi_s],
                        sd,
                        MONTH_SHORT[(sm as usize) - 1],
                        WEEKDAY_SHORT[wi_e],
                        ed,
                        MONTH_SHORT[(em as usize) - 1],
                        yr
                    )
                }
            }
        }
    };

    let drag: RwSignal<Option<DragState>> = RwSignal::new(None);
    // Set to true when a real drag ends so the column body on:click ignores
    // the synthetic click that the browser fires right after pointerup.
    let just_dragged: RwSignal<bool> = RwSignal::new(false);
    // Context menu: Some((event_uid, x_px, y_px)) when open.
    let ctx_menu: RwSignal<Option<(String, f64, f64)>> = RwSignal::new(None);

    // Memoized drag identity: only changes when the dragged event UID or its
    // target column changes, NOT on every live-position update. Subscribing to
    // this instead of `drag` directly prevents the event list from being torn
    // down and rebuilt on every pointermove tick.
    type DragTarget = Option<(String, (i32, u32, u32))>;
    let drag_target: Memo<DragTarget> =
        Memo::new(move |_| drag.get().map(|ds| (ds.uid.clone(), ds.day)));

    // NodeRef on the scroll container — used to read scrollTop and bounding rect.
    let scroll_ref: NodeRef<leptos::html::Div> = NodeRef::new();

    // Ticks every 60s so the current-time indicator line advances without
    // needing any other state to change — same pattern as Focus's `now_tick`.
    let now_tick: RwSignal<u32> = RwSignal::new(0);
    #[cfg(feature = "hydrate")]
    {
        use leptos::wasm_bindgen::prelude::*;
        let tick = Closure::<dyn FnMut()>::new(move || {
            now_tick.update(|n| *n = n.wrapping_add(1));
        });
        if let Some(win) = leptos::web_sys::window() {
            let _ = win.set_interval_with_callback_and_timeout_and_arguments_0(
                tick.as_ref().unchecked_ref(),
                60_000,
            );
        }
        tick.forget();
    }

    // ── Vertical zoom (backlog #96) ──────────────────────────────────────────
    // px-per-hour scale, exposed to CSS as `--hour-h`. Persisted per-browser in
    // localStorage: read once after hydration, written back on every change.
    let hour_h: RwSignal<f64> = RwSignal::new(HOUR_H_DEFAULT);

    // Set by `zoom_to` to the time (minutes from midnight) that should stay at
    // the top of the viewport across a scale change; applied by the effect
    // below once the new height has actually been rendered. Browser-only.
    #[cfg(feature = "hydrate")]
    let pending_scroll: RwSignal<Option<f64>> = RwSignal::new(None);

    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        if let Some(win) = leptos::web_sys::window() {
            if let Ok(Some(ls)) = win.local_storage() {
                if let Ok(Some(v)) = ls.get_item(HOUR_H_LS_KEY) {
                    if let Ok(h) = v.parse::<f64>() {
                        if h.is_finite() {
                            hour_h.set(h.clamp(HOUR_H_MIN, HOUR_H_MAX));
                        }
                    }
                }
            }
        }
    });

    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        let h = hour_h.get();
        if let Some(win) = leptos::web_sys::window() {
            if let Ok(Some(ls)) = win.local_storage() {
                let _ = ls.set_item(HOUR_H_LS_KEY, &h.to_string());
            }
        }
    });

    // Keeps the time currently at the viewport top fixed while the scale
    // changes, so zooming doesn't fling the user to a different part of the
    // day. Runs after render (effects are post-DOM-update), so the scroll
    // container already has its new height when we set `scroll_top`.
    #[cfg(feature = "hydrate")]
    Effect::new(move |_| {
        let h = hour_h.get();
        if let Some(mins) = pending_scroll.get() {
            if let Some(el) = scroll_ref.get() {
                use leptos::wasm_bindgen::JsCast as _;
                let el: &web_sys::Element = el.unchecked_ref();
                el.set_scroll_top((mins / 60.0 * h) as i32);
            }
            pending_scroll.set(None);
        }
    });

    let zoom_to = move |new_h: f64| {
        let old_h = hour_h.get_untracked();
        if (new_h - old_h).abs() < 0.01 {
            return;
        }
        #[cfg(feature = "hydrate")]
        if let Some(el) = scroll_ref.get() {
            use leptos::wasm_bindgen::JsCast as _;
            let el: &web_sys::Element = el.unchecked_ref();
            pending_scroll.set(Some(el.scroll_top() as f64 / old_h * 60.0));
        }
        hour_h.set(new_h);
    };
    let zoom_in = move || {
        let cur = hour_h.get_untracked();
        if let Some(next) = HOUR_H_STEPS.iter().copied().find(|&s| s > cur + 0.5) {
            zoom_to(next.min(HOUR_H_MAX));
        }
    };
    let zoom_out = move || {
        let cur = hour_h.get_untracked();
        if let Some(prev) = HOUR_H_STEPS.iter().copied().rev().find(|&s| s < cur - 0.5) {
            zoom_to(prev.max(HOUR_H_MIN));
        }
    };
    // Defined outside the `view!` block: a `>=` / `<=` comparison inline in an
    // attribute would be mis-parsed by the macro's HTML-ish parser (the `>` is
    // read as the end of the `<button>` tag, dumping the rest into the label).
    let zoom_out_disabled = move || hour_h.get() <= HOUR_H_MIN + 0.5;
    let zoom_in_disabled = move || hour_h.get() >= HOUR_H_MAX - 0.5;

    // Scroll to a fixed early-morning window once, when the grid first mounts —
    // not on every later mode switch back to Week/Day, so it doesn't fight a
    // scroll position the user chose. Runs once: it reads nothing reactive
    // besides `scroll_ref` (which goes from `None` to `Some` exactly once) and
    // `hour_h` untracked, so a zoom change never re-triggers it.
    Effect::new(move |_| {
        #[cfg(target_arch = "wasm32")]
        if let Some(el) = scroll_ref.get() {
            use leptos::wasm_bindgen::JsCast;
            let el: &web_sys::Element = el.unchecked_ref();
            let target = DEFAULT_VIEW_START_HOUR * hour_h.get_untracked();
            el.set_scroll_top(target.max(0.0) as i32);
        }
    });

    // ── Shared helper: client_y → minutes from midnight in the column body ─────
    // The column body is `24 * hour_h` px tall (see `--hour-h`) and starts just
    // after the sticky day-header (2.75 rem = 44 px, see .week-day-header height
    // in CSS). The scroll container has its own scrollTop. We measure everything
    // from client coordinates.
    //
    // col_y_offset = container_rect.top - scrollTop + header_height
    // px_per_min  = hour_h / 60  (1px/min at the default 60px-per-hour scale)
    let client_y_to_mins = move |client_y: f64, col_y_offset: f64| -> i32 {
        let h = hour_h.get_untracked();
        let px = (client_y - col_y_offset).clamp(0.0, 24.0 * h);
        ((px / h * 60.0) as i32).min(1439)
    };

    // ── Shared helper: client_x → which column day ─────────────────────────────
    // Computed lazily at pointermove time from the scroll container's live rect.
    // The grid has: 3rem gutter + 7 equal columns.
    // gutter_px ≈ 48px (3rem at 16px base). We measure the actual container
    // width and subtract the gutter to get the column zone.
    let client_x_to_day =
        move |client_x: f64, days: &[(i32, u32, u32)]| -> Option<(i32, u32, u32)> {
            #[cfg(target_arch = "wasm32")]
            {
                // Single-column mobile: always the only day.
                if days.len() == 1 {
                    return Some(days[0]);
                }
                let el = scroll_ref.get()?;
                use leptos::wasm_bindgen::JsCast;
                let el: &web_sys::Element = el.unchecked_ref();
                let rect = el.get_bounding_client_rect();
                let gutter_px = 48.0_f64; // 3rem
                let col_zone_left = rect.left() + gutter_px;
                let col_zone_width = rect.width() - gutter_px;
                if col_zone_width <= 0.0 {
                    return None;
                }
                let frac = (client_x - col_zone_left) / col_zone_width;
                if frac < 0.0 || frac >= 1.0 {
                    return None;
                }
                let col_idx = (frac * days.len() as f64) as usize;
                days.get(col_idx).copied()
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                let _ = (client_x, days);
                None
            }
        };

    // ── pointermove on the scroll container ────────────────────────────────────
    let on_pointer_move = move |ev: leptos::web_sys::PointerEvent| {
        let Some(ds) = drag.get() else { return };

        let client_y = ev.client_y() as f64;
        let client_x = ev.client_x() as f64;

        let raw_mins = client_y_to_mins(client_y, ds.col_y_offset);

        let (new_start_m, new_end_m) = match ds.kind {
            DragKind::Move => {
                let delta = raw_mins - client_y_to_mins(ds.origin_y, ds.col_y_offset);
                let s = snap15(ds.orig_start_m + delta);
                (s, (s + ds.orig_dur_m).min(1439))
            }
            DragKind::Resize => {
                let e = snap15(raw_mins).max(ds.orig_start_m + 15);
                (ds.orig_start_m, e.min(1439))
            }
        };

        let current_days: Vec<(i32, u32, u32)> = week_days();
        let new_day = if ds.kind == DragKind::Move {
            client_x_to_day(client_x, &current_days).unwrap_or(ds.day)
        } else {
            ds.day
        };

        let cx = ev.client_x() as f64;
        let cy = ev.client_y() as f64;
        drag.update(|opt| {
            if let Some(d) = opt {
                let dist = ((cx - d.origin_x).powi(2) + (cy - d.origin_y).powi(2)).sqrt();
                d.drag_px = d.drag_px.max(dist);
                d.live_start_m = new_start_m;
                d.live_end_m = new_end_m;
                d.day = new_day;
            }
        });
    };

    // ── pointerup: click vs drag ───────────────────────────────────────────────
    let on_pointer_up = move |_: leptos::web_sys::PointerEvent| {
        let Some(ds) = drag.get() else { return };
        let is_click = ds.drag_px < 5.0;
        let uid = ds.uid.clone();

        if is_click {
            drag.set(None);
            edit_event_id.set(Some(uid));
            return;
        }

        let (dy, dm, dd) = ds.day;
        let (sh, sm_) = mins_to_hhmm(ds.live_start_m);
        let (eh, em_) = mins_to_hhmm(ds.live_end_m);
        let start_str = dt_to_server(dy, dm, dd, sh, sm_);
        let end_str = dt_to_server(dy, dm, dd, eh, em_);

        // Batch drag clear + optimistic cache update into one reactive flush so
        // the event never snaps back to its old position between the two writes.
        {
            let uid2 = uid.clone();
            let s2 = start_str.clone();
            let e2 = end_str.clone();
            batch(move || {
                drag.set(None);
                just_dragged.set(true);
                cached_events.update(|evs| {
                    if let Some(ev) = evs.iter_mut().find(|e| e.id == uid2) {
                        ev.start = s2;
                        ev.end = e2;
                    }
                });
            });
        }

        leptos::task::spawn_local(async move {
            // Only refetch on failure — the optimistic cache update above is
            // already correct, so a successful round-trip needs no re-render.
            if update_event_times(uid, start_str, end_str).await.is_err() {
                on_refetch.run(());
            }
        });
    };

    // ── Helper: read col_y_offset from the DOM at pointerdown time ─────────────
    // col_y_offset = rect.top - scrollTop + day_header_height (44px)
    let read_col_y_offset = move || -> f64 {
        #[cfg(target_arch = "wasm32")]
        {
            scroll_ref
                .get()
                .map(|el| {
                    use leptos::wasm_bindgen::JsCast;
                    let el: &web_sys::Element = el.unchecked_ref();
                    let rect = el.get_bounding_client_rect();
                    let scroll_top = el.scroll_top() as f64;
                    rect.top() - scroll_top + 44.0
                })
                .unwrap_or(44.0)
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            44.0
        }
    };

    view! {
        <div class="week-grid-wrap">
            <div class="week-nav">
                <button class="week-nav-btn" on:click=go_today>"Today"</button>
                <button class="week-nav-btn" on:click=prev_week>"‹"</button>
                <button class="week-nav-btn" on:click=next_week>"›"</button>
                <span class="cal-date-label">{week_label}</span>
                <div class="week-zoom-ctrl">
                    <button
                        class="week-nav-btn week-zoom-btn"
                        title="Zoom out (compress hours)"
                        aria-label="Zoom out"
                        disabled=zoom_out_disabled
                        on:click=move |_| zoom_out()
                    >"−"</button>
                    <button
                        class="week-nav-btn week-zoom-btn"
                        title="Zoom in (expand hours)"
                        aria-label="Zoom in"
                        disabled=zoom_in_disabled
                        on:click=move |_| zoom_in()
                    >"+"</button>
                </div>
            </div>

            // ── Month grid / time grid ────────────────────────────────────────
            {move || if cal_mode.get() == CalMode::Month {
                view! {
                    <MonthGrid
                        events=events
                        month=mini_month
                        today=today
                        on_edit=Callback::new(move |id: String| edit_event_id.set(Some(id)))
                        on_new=Callback::new(move |(s, e): (String, String)| dialog_times.set(Some((s, e))))
                    />
                }.into_any()
            } else { view! { <>

            // ── All-day banner ────────────────────────────────────────────────
            // Sits outside the scroll container so it stays visible while the
            // time-grid scrolls.  Uses the same column widths as .week-grid.
            <div
                class="week-allday-section"
                style=move || {
                    let n = week_days().len();
                    format!("grid-template-columns: 3rem repeat({n}, 1fr)")
                }
            >
                <div class="week-allday-gutter">"all-day"</div>
                {move || {
                    #[cfg(target_arch = "wasm32")]
                    let t0 = perf_now();
                    let days = week_days();
                    let n = days.len();

                    let allday_evs: Vec<Event> = if n > 0 {
                        let day_s = |y: i32, m: u32, d: u32| format!("{:04}-{:02}-{:02}", y, m, d);
                        let first_str = day_s(days[0].0, days[0].1, days[0].2);
                        let last_str  = { let (y,m,d) = days[n-1]; day_s(y,m,d) };
                        events.get().into_iter()
                            .filter(|e| {
                                // All-day events that overlap the visible week.
                                let is_allday = e.all_day
                                    && e.start.as_str() <= last_str.as_str()
                                    && e.end  .as_str() >= first_str.as_str();
                                // Multi-day timed events: start-date ≠ end-date,
                                // overlapping the visible week.
                                let start_date = &e.start[..10.min(e.start.len())];
                                let end_date   = &e.end  [..10.min(e.end  .len())];
                                let is_multiday_timed = !e.all_day
                                    && start_date != end_date
                                    && start_date <= last_str.as_str()
                                    && end_date   >= first_str.as_str();
                                is_allday || is_multiday_timed
                            })
                            .collect()
                    } else {
                        vec![]
                    };

                    let layout   = compute_allday_layout(&allday_evs, &days);
                    #[cfg(target_arch = "wasm32")]
                    leptos::logging::log!("[perf] allday-banner: {:.2}ms ({} allday/multiday evs)", perf_now() - t0, allday_evs.len());
                    let max_row  = layout.iter().map(|s| s.row).max();
                    // Each row is 18 px tall; 4 px top/bottom padding.
                    let banner_h = max_row.map_or(0usize, |r| (r + 1) * 18 + 4);

                    let pills = allday_evs.iter().zip(layout.iter()).map(|(ev, slot)| {
                        let color   = ev.calendar_color.clone().unwrap_or_else(|| "#3b82f6".to_string());
                        let left_f  = if n > 0 { slot.left_col / n as f64 * 100.0 } else { 0.0 };
                        let wid_f   = if n > 0 { (slot.right_col - slot.left_col) / n as f64 * 100.0 } else { 100.0 };
                        let top_px  = slot.row * 18 + 2;
                        let summary = ev.summary.clone();
                        let ev_id   = ev.id.clone();
                        let ev_uid  = ev.id.clone();
                        let cl      = slot.clips_left;
                        let cr      = slot.clips_right;
                        // Pills bleed to the edge when clipped; otherwise inset 2 px per side.
                        let pill_left = if cl { format!("{:.3}%", left_f) }
                                        else  { format!("calc({:.3}% + 2px)", left_f) };
                        let pill_width = match (cl, cr) {
                            (false, false) => format!("calc({:.3}% - 4px)", wid_f),
                            (true,  false) | (false, true) => format!("calc({:.3}% - 2px)", wid_f),
                            (true,  true)  => format!("{:.3}%", wid_f),
                        };
                        let brad = match (cl, cr) {
                            (false, false) => "border-radius:0.25rem;",
                            (true,  false) => "border-radius:0 0.25rem 0.25rem 0;",
                            (false, true)  => "border-radius:0.25rem 0 0 0.25rem;",
                            (true,  true)  => "border-radius:0;",
                        };
                        // Same tint fill + rail as .week-event, for the same reason (adjacent
                        // same-calendar pills shouldn't fuse). No rail when clipped on the left —
                        // the pill doesn't actually start at its visible edge there, so a rail
                        // would misleadingly mark a false start.
                        let bg = if cl {
                            format!("color-mix(in srgb, {color} 22%, var(--surface))")
                        } else {
                            format!(
                                "linear-gradient(to right, {color} 3px, transparent 3px),\
                                 color-mix(in srgb, {color} 22%, var(--surface))"
                            )
                        };
                        view! {
                            <div
                                class="week-allday-event"
                                data-event-uid=ev_uid
                                style=format!(
                                    "top:{top}px;left:{l};width:{w};background:{bg};{br}",
                                    top=top_px, l=pill_left, w=pill_width, bg=bg, br=brad,
                                )
                                on:click=move |_| edit_event_id.set(Some(ev_id.clone()))
                            >
                                {if cl { "‹ " } else { "" }}
                                {summary}
                                {if cr { " ›" } else { "" }}
                            </div>
                        }
                    }).collect_view();

                    view! {
                        <div
                            class="week-allday-banner"
                            style=format!("height:{}px;", banner_h)
                        >
                            {pills}
                        </div>
                    }
                }}
            </div>

            <div
                class="week-grid-scroll"
                node_ref=scroll_ref
                style=move || format!("--hour-h:{}px;", hour_h.get())
                on:wheel=move |ev: leptos::web_sys::WheelEvent| {
                    // Ctrl/⌘ + wheel zooms the time scale; a plain wheel keeps
                    // its normal scroll behaviour (so the page can't be
                    // browser-zoomed over the grid by accident).
                    if ev.ctrl_key() || ev.meta_key() {
                        ev.prevent_default();
                        if ev.delta_y() < 0.0 { zoom_in(); } else { zoom_out(); }
                    }
                }
                on:pointermove=on_pointer_move
                on:pointerup=on_pointer_up
                on:pointercancel=move |_: leptos::web_sys::PointerEvent| { drag.set(None); }
                on:touchstart=move |ev: leptos::web_sys::TouchEvent| {
                    if drag.get_untracked().is_some() { return; }
                    if let Some(t) = ev.touches().get(0) {
                        touch_start.set(Some((t.client_x() as f64, t.client_y() as f64)));
                    }
                }
                on:touchend=move |ev: leptos::web_sys::TouchEvent| {
                    let start = touch_start.get_untracked();
                    touch_start.set(None);
                    if drag.get_untracked().is_some() { return; }
                    if let Some((sx, sy)) = start {
                        if let Some(t) = ev.changed_touches().get(0) {
                            let dx = t.client_x() as f64 - sx;
                            let dy = t.client_y() as f64 - sy;
                            // Horizontal swipe: must travel ≥50px horizontally
                            // and be more horizontal than vertical.
                            if dx.abs() > 50.0 && dx.abs() > dy.abs() * 1.5 {
                                if dx < 0.0 { do_next(); } else { do_prev(); }
                            }
                        }
                    }
                }
            >
                <div
                    class="week-grid"
                    style=move || {
                        let n = week_days().len();
                        format!("grid-template-columns: 3rem repeat({n}, 1fr)")
                    }
                >
                    // Time gutter
                    <div class="week-time-gutter">
                        <div class="week-day-header-spacer"></div>
                        {(0..24u32).map(|h| view!{
                            <div class="week-hour-label">
                                {move || format_hour_label(h, &time_fmt())}
                            </div>
                        }).collect_view()}
                    </div>

                    // Day columns (7 on desktop, 1 on mobile)
                    {move || week_days().into_iter().enumerate().map(|(_col_idx,(dy,dm,dd))| {
                        let is_today = today==(dy,dm,dd);

                        // Timed events that belong to this column (all-day events are
                        // rendered in the banner above, not in the time grid).
                        let day_events = move || -> Vec<Event> {
                            // Use drag_target (memoized uid+day) so this closure
                            // only re-runs when the drag column changes, not on
                            // every live-position tick.
                            #[cfg(target_arch = "wasm32")]
                            let t0 = perf_now();
                            let drag_key = drag_target.get();
                            let dragged_uid = drag_key.as_ref().and_then(|(uid, day)| {
                                if *day == (dy,dm,dd) { Some(uid.clone()) } else { None }
                            });
                            let result: Vec<Event> = events.get().into_iter()
                                .filter(|e| {
                                    if e.all_day { return false; }
                                    // Multi-day timed events go in the banner, not the time grid.
                                    let start_date = &e.start[..10.min(e.start.len())];
                                    let end_date   = &e.end  [..10.min(e.end  .len())];
                                    if start_date != end_date { return false; }
                                    // Don't show in original column if dragged elsewhere.
                                    if let Some((uid, day)) = &drag_key {
                                        if uid == &e.id && *day != (dy,dm,dd) {
                                            return false;
                                        }
                                    }
                                    // Show if: originally belongs here, OR dragged here.
                                    let native = parse_dt(&e.start)
                                        .is_some_and(|(ey,em,ed,_,_)| ey==dy&&em==dm&&ed==dd);
                                    let dragged_here = dragged_uid.as_deref() == Some(&e.id);
                                    native || dragged_here
                                })
                                .collect();
                            #[cfg(target_arch = "wasm32")]
                            leptos::logging::log!("[perf] day-events col={}: {:.2}ms ({} events)", _col_idx, perf_now() - t0, result.len());
                            result
                        };

                        // Memoised (id, slot_left, slot_width) for this column. The keyed
                        // <For> below reads from this so layout recomputes once per signal
                        // change rather than once per child item.
                        let events_with_layout: Memo<Vec<(String, f64, f64)>> =
                            Memo::new(move |_| {
                                #[cfg(target_arch = "wasm32")]
                                let t0 = perf_now();
                                let evs = day_events();
                                let layout = compute_layout(&evs);
                                let result = evs.into_iter().zip(layout)
                                    .map(|(e, (sl, sw))| (e.id.clone(), sl, sw))
                                    .collect();
                                #[cfg(target_arch = "wasm32")]
                                leptos::logging::log!("[perf] layout col={}: {:.2}ms", _col_idx, perf_now() - t0);
                                result
                            });

                        // Both closures are defined outside the `view!` block below so
                        // the view! macro parser never encounters turbofish syntax (which
                        // has `<>` that the macro would mis-read as HTML tags) or complex
                        // closure bodies.

                        // `each` for <For>: returns the ordered event-ID list for this column.
                        let for_each = move || -> Vec<String> {
                            events_with_layout.get().into_iter().map(|(id, _, _)| id).collect()
                        };

                        // Render function for each event card.
                        let event_child = move |uid: String| {
                            let uid_ev     = uid.clone();
                            let uid_slot   = uid.clone();
                            let uid_live   = uid.clone();
                            let uid_drag   = uid.clone();
                            let uid_drag2  = uid.clone();
                            let uid_ctx    = uid.clone();
                            let uid_time   = uid.clone();
                            let uid_move   = uid.clone();
                            let uid_resize = uid.clone();

                            // Reactive event data — updates when the server refetches.
                            let ev = Signal::derive(move || {
                                #[cfg(target_arch = "wasm32")]
                                let t0 = perf_now();
                                let all = events.get();
                                let result = all.iter().find(|e| e.id == uid_ev).cloned();
                                #[cfg(target_arch = "wasm32")]
                                leptos::logging::log!("[perf] ev-lookup: {:.2}ms (scanned {} events)", perf_now() - t0, all.len());
                                result
                            });

                            // Reactive slot position — updates when layout changes
                            // (e.g. an overlapping event is added or removed).
                            let slot = Signal::derive(move || {
                                events_with_layout.get().iter()
                                    .find(|(id, _, _)| *id == uid_slot)
                                    .map(|(_, sl, sw)| (*sl, *sw))
                                    .unwrap_or((0.0, 100.0))
                            });

                            let is_dragging  = move || drag.get().as_ref().is_some_and(|ds| ds.uid == uid_drag);
                            let is_dragging2 = move || drag.get().as_ref().is_some_and(|ds| ds.uid == uid_drag2);

                            // Whether the time is folded onto the title line or shown on its
                            // own line beneath is a CSS container-query decision (see
                            // .week-event-time / .week-event-time-inline in style.css) driven by
                            // the event block's actual rendered size, not its duration — a block
                            // can end up short or narrow regardless of how long the event runs
                            // (zoomed-out grid, overlapping-event columns, live resize).
                            let event_time_text = Signal::derive(move || -> String {
                                let fmt = time_fmt();
                                if let Some(ds) = drag.get() {
                                    if ds.uid == uid_time {
                                        let (sh,sm_) = mins_to_hhmm(ds.live_start_m);
                                        let (eh,em_) = mins_to_hhmm(ds.live_end_m);
                                        return format!("{}–{}", format_hm(sh,sm_,&fmt), format_hm(eh,em_,&fmt));
                                    }
                                }
                                let (start, end) = ev.get()
                                    .map(|e| (e.start, e.end))
                                    .unwrap_or_default();
                                format!("{}–{}", short_time(&start, &fmt), short_time(&end, &fmt))
                            });

                            let live_pos = move || -> (f64, f64) {
                                if let Some(ds) = drag.get() {
                                    if ds.uid == uid_live && ds.day == (dy,dm,dd) {
                                        let top = ds.live_start_m as f64 / 1440.0 * 100.0;
                                        let ht  = (ds.live_end_m - ds.live_start_m) as f64 / 1440.0 * 100.0;
                                        return (top, ht.max(15.0/1440.0*100.0));
                                    }
                                }
                                let (start, end) = ev.get()
                                    .map(|e| (e.start, e.end))
                                    .unwrap_or_default();
                                event_position_from(&start, &end)
                            };

                            view! {
                                <div
                                    class=move || if is_dragging() {"week-event week-event-dragging"} else {"week-event"}
                                    data-event-uid=uid.clone()
                                    style=move || {
                                        let (top,ht) = live_pos();
                                        let color = ev.get().as_ref()
                                            .and_then(|e| e.calendar_color.clone())
                                            .unwrap_or_else(|| "#3b82f6".to_string());
                                        // Tint fill + accent rail (not text_color_for_event's
                                        // white/black pick) — the rail carries the calendar
                                        // identity. The rail is a hard-stop background gradient,
                                        // not a border: a `border-left` paints on top of the
                                        // block's own box-shadow (see .week-event's inset bottom
                                        // shadow in style.css), so consecutive same-calendar
                                        // events would show one continuous, uncut rail even
                                        // where the tint fill has a visible seam. As a background
                                        // layer, the rail sits *below* that inset shadow like the
                                        // tint fill does, so the seam cuts through both uniformly.
                                        if is_dragging2() {
                                            format!(
                                                "top:{top:.3}%;height:{ht:.3}%;\
                                                 min-height:1.25rem;\
                                                 left:0;width:calc(100% - 4px);z-index:10;\
                                                 background:linear-gradient(to right, {color} 3px, transparent 3px),\
                                                 color-mix(in srgb, {color} 22%, var(--surface));",
                                                top=top, ht=ht, color=color,
                                            )
                                        } else {
                                            let (l, w) = slot.get();
                                            format!(
                                                "top:{top:.3}%;height:{ht:.3}%;\
                                                 min-height:1.25rem;\
                                                 left:calc({l:.3}% + 2px);\
                                                 width:calc({w:.3}% - 4px);\
                                                 background:linear-gradient(to right, {color} 3px, transparent 3px),\
                                                 color-mix(in srgb, {color} 22%, var(--surface));",
                                                top=top, ht=ht, l=l, w=w, color=color,
                                            )
                                        }
                                    }
                                    on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()
                                    on:contextmenu=move |ev: leptos::web_sys::MouseEvent| {
                                        ev.prevent_default();
                                        ev.stop_propagation();
                                        drag.set(None);
                                        ctx_menu.set(Some((uid_ctx.clone(), ev.client_x() as f64, ev.client_y() as f64)));
                                    }
                                    on:pointerdown=move |ev: leptos::web_sys::PointerEvent| {
                                        ev.stop_propagation();
                                        ev.prevent_default();
                                        let current = events.get()
                                            .into_iter().find(|x| x.id == uid_move);
                                        let (osh,osm,oeh,oem) = current.as_ref()
                                            .and_then(|x| {
                                                let s = parse_dt(&x.start)?;
                                                let e = parse_dt(&x.end)?;
                                                Some((s.3,s.4,e.3,e.4))
                                            })
                                            .unwrap_or((0,0,1,0));
                                        let orig_start_m = osh as i32 * 60 + osm as i32;
                                        let orig_end_m   = oeh as i32 * 60 + oem as i32;
                                        let orig_dur_m   = (orig_end_m - orig_start_m).max(15);
                                        let col_y_offset = read_col_y_offset();
                                        drag.set(Some(DragState {
                                            uid:          uid_move.clone(),
                                            kind:         DragKind::Move,
                                            day:          (dy,dm,dd),
                                            orig_start_m,
                                            orig_dur_m,
                                            origin_x:     ev.client_x() as f64,
                                            origin_y:     ev.client_y() as f64,
                                            col_y_offset,
                                            live_start_m: orig_start_m,
                                            live_end_m:   orig_end_m,
                                            drag_px:      0.0,
                                        }));
                                    }
                                >
                                    <div class="week-event-content">
                                        <div class="week-event-summary">
                                            {move || ev.get().map(|e| e.summary).unwrap_or_default()}
                                            <span class="week-event-time-inline">", "{move || event_time_text.get()}</span>
                                        </div>
                                        <div class="week-event-time">{move || event_time_text.get()}</div>
                                    </div>

                                    // Resize handle
                                    <div
                                        class="week-event-resize-handle"
                                        on:pointerdown=move |ev: leptos::web_sys::PointerEvent| {
                                            ev.stop_propagation();
                                            ev.prevent_default();
                                            let current = events.get()
                                                .into_iter().find(|x| x.id == uid_resize);
                                            let (osh,osm,oeh,oem) = current.as_ref()
                                                .and_then(|x| {
                                                    let s = parse_dt(&x.start)?;
                                                    let e = parse_dt(&x.end)?;
                                                    Some((s.3,s.4,e.3,e.4))
                                                })
                                                .unwrap_or((0,0,1,0));
                                            let orig_start_m = osh as i32 * 60 + osm as i32;
                                            let orig_end_m   = oeh as i32 * 60 + oem as i32;
                                            let orig_dur_m   = (orig_end_m - orig_start_m).max(15);
                                            let col_y_offset = read_col_y_offset();
                                            drag.set(Some(DragState {
                                                uid:          uid_resize.clone(),
                                                kind:         DragKind::Resize,
                                                day:          (dy,dm,dd),
                                                orig_start_m,
                                                orig_dur_m,
                                                origin_x:     ev.client_x() as f64,
                                                origin_y:     ev.client_y() as f64,
                                                col_y_offset,
                                                live_start_m: orig_start_m,
                                                live_end_m:   orig_end_m,
                                                drag_px:      0.0,
                                            }));
                                        }
                                        on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()
                                    ></div>
                                </div>
                            }
                        };

                        view! {
                            <div class=if is_today {"week-day-col week-day-today"} else {"week-day-col"}>
                                <div class="week-day-header">
                                    <div class=if is_today{"week-day-name today-badge"}else{"week-day-name"}>
                                        <span class="week-day-abbr">{WEEKDAY_SHORT[weekday_idx(dy,dm,dd)]}</span>
                                        <span class="week-day-num">{dd}</span>
                                    </div>
                                </div>
                                <div
                                    class="week-day-body"
                                    on:click=move |ev: leptos::web_sys::MouseEvent| {
                                        if drag.get().is_some() { return; }
                                        // Consume the synthetic click that follows a drag-end.
                                        if just_dragged.get_untracked() {
                                            just_dragged.set(false);
                                            return;
                                        }
                                        // Use current_target (always .week-day-body) rather than
                                        // offset_y, which resets to 0 at the top of whichever
                                        // child element was actually clicked.
                                        use leptos::wasm_bindgen::JsCast as _;
                                        let Some(target) = ev.current_target() else { return; };
                                        let rect = target
                                            .unchecked_into::<web_sys::Element>()
                                            .get_bounding_client_rect();
                                        let y_offset   = (ev.client_y() as f64 - rect.top()).max(0.0);
                                        let total_mins = (y_offset / hour_h.get_untracked() * 60.0) as u32;
                                        let h=(total_mins/60).min(23); let m=total_mins%60;
                                        let (sh,sm_)=snap30(h,m);
                                        let (eh,em_)=if sh==23&&sm_==30{(23,59)}else if sm_==0{(sh,30)}else{(sh+1,0)};
                                        dialog_times.set(Some((to_input_dt(dy,dm,dd,sh,sm_),to_input_dt(dy,dm,dd,eh,em_))));
                                    }
                                >
                                    {(0..24u32).map(|h| view!{
                                        <div
                                            class="week-hour-slot"
                                            style=format!("top:calc({h} * var(--hour-h, 60px));")
                                        ></div>
                                    }).collect_view()}

                                    // Current-time indicator — only in today's column, wherever
                                    // it appears (Day/Week/Work Week). Re-positions every 60s via
                                    // `now_tick`; `.week-day-body` is `24 * --hour-h` px tall, so
                                    // `local_now_minutes()` maps to a `top` offset through the
                                    // current scale (`minutes / 60 * --hour-h`).
                                    // Reads the timezone preference fresh on every tick so the line
                                    // responds immediately if the user changes it.
                                    {is_today.then(|| view! {
                                        <div
                                            class="week-current-time-line"
                                            style=move || {
                                                let _ = now_tick.get();
                                                let tz = use_context::<UserPrefsCtx>()
                                                    .map(|s| s.get().timezone)
                                                    .unwrap_or_else(|| "UTC".to_string());
                                                format!("top:calc({} * var(--hour-h, 60px) / 60);", local_now_minutes(&tz))
                                            }
                                        >
                                            <div class="week-current-time-dot"></div>
                                        </div>
                                    })}

                                    // Keyed list: <For> preserves DOM nodes for events whose
                                    // ID hasn't changed, so a remote sync that adds or removes
                                    // one event does not tear down and rebuild every event card.
                                    // Reactive signals inside children handle updates to existing
                                    // events (time, title, colour) without full re-mounts.
                                    // `event_child` is defined before this `view!` block so the
                                    // macro parser never sees the complex closure body.
                                    <For
                                        each=for_each
                                        key=|uid: &String| uid.clone()
                                        children=event_child
                                    />
                                </div>
                            </div>
                        }
                    }).collect_view()}
                </div>
            </div>

            </> }.into_any() }}

            // ── Event context menu ────────────────────────────────────────────
            {move || ctx_menu.get().map(|(uid, x, y)| {
                let uid_edit   = uid.clone();
                let uid_delete = uid.clone();
                view! {
                    // Invisible backdrop — click anywhere outside to close.
                    <div
                        class="ctx-menu-backdrop"
                        on:click=move |_| ctx_menu.set(None)
                        on:contextmenu=move |ev: leptos::web_sys::MouseEvent| {
                            ev.prevent_default();
                            ctx_menu.set(None);
                        }
                    />
                    <div
                        class="ctx-menu"
                        style=format!("left:{x}px;top:{y}px;")
                        on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()
                    >
                        <button
                            class="ctx-menu-item"
                            on:click=move |_| {
                                ctx_menu.set(None);
                                edit_event_id.set(Some(uid_edit.clone()));
                            }
                        >"Edit"</button>
                        <button
                            class="ctx-menu-item ctx-menu-item-danger"
                            on:click=move |_| {
                                let uid = uid_delete.clone();
                                ctx_menu.set(None);
                                // Optimistic removal from cached events.
                                cached_events.update(|evs| evs.retain(|e| e.id != uid));
                                let uid2 = uid_delete.clone();
                                leptos::task::spawn_local(async move {
                                    if delete_event(uid2).await.is_err() {
                                        on_refetch.run(());
                                    }
                                });
                            }
                        >"Delete"</button>
                    </div>
                }
            })}
        </div>
    }
}

// ── MonthGrid ─────────────────────────────────────────────────────────────────

#[component]
pub fn MonthGrid(
    events: Signal<Vec<Event>>,
    month: RwSignal<(i32, u32)>,
    today: (i32, u32, u32),
    on_edit: Callback<String>,
    on_new: Callback<(String, String)>,
) -> impl IntoView {
    let time_fmt = move || {
        use_context::<UserPrefsCtx>()
            .map(|s| s.get().time_format.clone())
            .unwrap_or_else(|| "12h".to_string())
    };

    view! {
        <div class="month-grid-wrap">
            <div class="month-col-headers">
                {WEEKDAY_SHORT.iter().map(|&d| view! {
                    <div class="month-col-header">{d}</div>
                }).collect_view()}
            </div>
            <div class="month-grid-body">
                {move || {
                    #[cfg(target_arch = "wasm32")]
                    let t0 = perf_now();
                    let (y, m) = month.get();
                    let evs    = events.get();
                    #[cfg(target_arch = "wasm32")]
                    let evs_count = evs.len();
                    let fmt    = time_fmt();
                    // First cell: Monday on or before the 1st of the displayed month.
                    let first_dow  = weekday_of_first(y, m) as i32; // 0=Mon … 6=Sun
                    let first_cell = add_days(y, m, 1, -first_dow);

                    let rows_iter = (0..6usize).map(|row| {
                        let row_start = add_days(
                            first_cell.0, first_cell.1, first_cell.2,
                            (row * 7) as i32,
                        );
                        let row_days: Vec<(i32,u32,u32)> = (0..7i32)
                            .map(|i| add_days(row_start.0, row_start.1, row_start.2, i))
                            .collect();

                        // Multiday / all-day events touching this row.
                        let first_str = { let (ry,rm,rd)=row_days[0]; format!("{:04}-{:02}-{:02}",ry,rm,rd) };
                        let last_str  = { let (ry,rm,rd)=row_days[6]; format!("{:04}-{:02}-{:02}",ry,rm,rd) };

                        let mut multiday_evs: Vec<Event> = evs.iter()
                            .filter(|e| {
                                let es = &e.start[..10.min(e.start.len())];
                                let ee = &e.end  [..10.min(e.end  .len())];
                                let is_allday = e.all_day
                                    && es <= last_str.as_str()
                                    && ee >= first_str.as_str();
                                let is_multi  = !e.all_day && es != ee
                                    && es <= last_str.as_str()
                                    && ee >= first_str.as_str();
                                is_allday || is_multi
                            })
                            .cloned()
                            .collect();
                        // All-day events first, then multi-day timed events.
                        multiday_evs.sort_by(|a, b| b.all_day.cmp(&a.all_day).then(a.start.cmp(&b.start)));

                        let bar_layout  = compute_allday_layout(&multiday_evs, &row_days);
                        let max_bar_row = bar_layout.iter().map(|s| s.row).max();
                        let bars_h      = max_bar_row.map_or(0usize, |r| (r + 1) * 18 + 4);

                        // Per-cell single-day timed events.
                        let day_cells: Vec<_> = row_days.iter().enumerate().map(|(day_idx, &(dy,dm,dd))| {
                            let is_today = today == (dy,dm,dd);
                            let in_month = dm == m;
                            let day_str  = format!("{:04}-{:02}-{:02}", dy, dm, dd);

                            let mut day_evs: Vec<Event> = evs.iter()
                                .filter(|e| {
                                    if e.all_day { return false; }
                                    let es = &e.start[..10.min(e.start.len())];
                                    let ee = &e.end  [..10.min(e.end  .len())];
                                    es == day_str.as_str() && es == ee
                                })
                                .cloned()
                                .collect();
                            day_evs.sort_by(|a, b| a.start.cmp(&b.start));

                            let cell_cls = {
                                let mut s = "month-day-cell".to_string();
                                if !in_month { s.push_str(" month-day-cell--outside"); }
                                s
                            };
                            let fmt2 = fmt.clone();
                            // Use the max bar-row that actually spans this day, not the
                            // week-wide maximum — so days with fewer bars don't waste space.
                            let cell_max_row = bar_layout.iter()
                                .filter(|s| s.left_col < (day_idx + 1) as f64
                                         && s.right_col > day_idx as f64)
                                .map(|s| s.row)
                                .max();
                            let bars_h_cell = cell_max_row.map_or(0usize, |r| (r + 1) * 18 + 4);
                            let on_new2 = on_new;

                            view! {
                                <div
                                    class=cell_cls
                                    style=format!("padding-top:{}px", bars_h_cell + 26)
                                    on:click=move |_| {
                                        on_new2.run((
                                            to_input_dt(dy, dm, dd, 9, 0),
                                            to_input_dt(dy, dm, dd, 9, 30),
                                        ));
                                    }
                                >
                                    <div class="month-day-num">
                                        <span class=if is_today {
                                            "month-day-badge month-day-badge--today"
                                        } else {
                                            "month-day-badge"
                                        }>{dd}</span>
                                    </div>
                                    {day_evs.into_iter().map(|e| {
                                        let color  = e.calendar_color.clone()
                                            .unwrap_or_else(|| "#3b82f6".to_string());
                                        let ev_id  = e.id.clone();
                                        let ev_uid = e.id.clone();
                                        let on_e   = on_edit;
                                        let t      = short_time(&e.start, &fmt2);
                                        let summ   = e.summary.clone();
                                        view! {
                                            <div
                                                class="month-event-line"
                                                data-event-uid=ev_uid
                                                on:click=move |mev: leptos::web_sys::MouseEvent| {
                                                    mev.stop_propagation();
                                                    on_e.run(ev_id.clone());
                                                }
                                            >
                                                <span
                                                    class="month-event-dot"
                                                    style=format!("background:{color}")
                                                ></span>
                                                <span class="month-event-time">{t}</span>
                                                <span class="month-event-summary">{summ}</span>
                                            </div>
                                        }
                                    }).collect_view()}
                                </div>
                            }
                        }).collect::<Vec<_>>();

                        // Multiday bars overlay.
                        let bars_h2   = bars_h;
                        let on_edit2  = on_edit;
                        let bar_items = multiday_evs.into_iter().zip(bar_layout.into_iter())
                            .map(|(ev, slot)| {
                                let color      = ev.calendar_color.clone()
                                    .unwrap_or_else(|| "#3b82f6".to_string());
                                let n_f        = 7.0f64;
                                let left_f     = slot.left_col / n_f * 100.0;
                                let wid_f      = (slot.right_col - slot.left_col) / n_f * 100.0;
                                let top_px     = slot.row * 18 + 2;
                                let summary    = ev.summary.clone();
                                let ev_id      = ev.id.clone();
                                let ev_uid     = ev.id.clone();
                                let on_e       = on_edit2;
                                let cl         = slot.clips_left;
                                let cr         = slot.clips_right;
                                let pill_left  = if cl {
                                    format!("{:.3}%", left_f)
                                } else {
                                    format!("calc({:.3}% + 2px)", left_f)
                                };
                                let pill_width = match (cl, cr) {
                                    (false, false) => format!("calc({:.3}% - 4px)", wid_f),
                                    _              => format!("calc({:.3}% - 2px)", wid_f),
                                };
                                let brad = match (cl, cr) {
                                    (false, false) => "border-radius:0.25rem;",
                                    (true,  false) => "border-radius:0 0.25rem 0.25rem 0;",
                                    (false, true)  => "border-radius:0.25rem 0 0 0.25rem;",
                                    (true,  true)  => "border-radius:0;",
                                };
                                let bg = if cl {
                                    format!("color-mix(in srgb, {color} 22%, var(--surface))")
                                } else {
                                    format!(
                                        "linear-gradient(to right, {color} 3px, transparent 3px),\
                                         color-mix(in srgb, {color} 22%, var(--surface))"
                                    )
                                };
                                view! {
                                    <div
                                        class="month-bar-event"
                                        data-event-uid=ev_uid
                                        style=format!(
                                            "top:{top}px;left:{l};width:{w};background:{bg};{br}",
                                            top=top_px, l=pill_left, w=pill_width, bg=bg, br=brad,
                                        )
                                        on:click=move |mev: leptos::web_sys::MouseEvent| {
                                            mev.stop_propagation();
                                            on_e.run(ev_id.clone());
                                        }
                                    >
                                        {if cl { "‹ " } else { "" }}
                                        {summary}
                                        {if cr { " ›" } else { "" }}
                                    </div>
                                }
                            })
                            .collect_view();

                        view! {
                            <div class="month-grid-row">
                                <div
                                    class="month-row-bars"
                                    style=format!("height:{}px", bars_h2)
                                >
                                    {bar_items}
                                </div>
                                {day_cells}
                            </div>
                        }
                    }).collect_view();
                    #[cfg(target_arch = "wasm32")]
                    leptos::logging::log!("[perf] month-grid: {:.2}ms ({} events)", perf_now() - t0, evs_count);
                    rows_iter
                }}
            </div>
        </div>
    }
}
