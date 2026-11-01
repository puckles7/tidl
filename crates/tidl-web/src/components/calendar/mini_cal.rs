//! Month mini-calendar sidebar widget.

use super::date_helpers::{
    add_days, days_in_month, month_label, week_monday_of, weekday_of_first, ymd_to_jdn,
    WEEKDAY_SHORT,
};
use super::layout::{compute_month_day_marks, MonthDayMark};
use crate::api::Event;
use leptos::prelude::*;

/// Corner cut depth (px) for the chamfered bevel drawn at a multi-day event's
/// true start/end. Kept in sync with `.mini-cal-line` height in style.css.
const MINI_CAL_BEVEL_PX: f64 = 2.0;
const MINI_CAL_LINE_H_PX: f64 = 4.0;

/// `clip-path` value that chamfers the corners on the beveled side(s) of a
/// multi-day bar; `None` means "draw a plain rectangle" (a join or a flat
/// edge clipped by the month/row boundary — neither should be rounded).
fn bevel_clip_path(bevel_left: bool, bevel_right: bool) -> Option<String> {
    if !bevel_left && !bevel_right {
        return None;
    }
    let bl = if bevel_left { MINI_CAL_BEVEL_PX } else { 0.0 };
    let br = if bevel_right { MINI_CAL_BEVEL_PX } else { 0.0 };
    let h = MINI_CAL_LINE_H_PX;
    Some(format!(
        "polygon({bl}px 0, calc(100% - {br}px) 0, 100% {br}px, 100% {h_minus_br}px, \
         calc(100% - {br}px) {h}px, {bl}px {h}px, 0 {h_minus_bl}px, 0 {bl}px)",
        h_minus_br = h - br,
        h_minus_bl = h - bl,
    ))
}

#[component]
pub fn MonthMiniCal(
    events: Signal<Vec<Event>>,
    month: RwSignal<(i32, u32)>,
    week_monday: RwSignal<(i32, u32, u32)>,
    today: (i32, u32, u32),
    mobile_day: RwSignal<(i32, u32, u32)>,
) -> impl IntoView {
    let prev_month = move |_| {
        month.update(|(y, m)| {
            if *m == 1 {
                *y -= 1;
                *m = 12;
            } else {
                *m -= 1;
            }
        });
    };
    let next_month = move |_| {
        month.update(|(y, m)| {
            if *m == 12 {
                *y += 1;
                *m = 1;
            } else {
                *m += 1;
            }
        });
    };

    view! {
        <div class="mini-cal">
            <div class="mini-cal-header">
                <button class="mini-cal-nav" on:click=prev_month>"‹"</button>
                <span class="mini-cal-title">
                    {move || { let (y,m) = month.get(); month_label(y,m) }}
                </span>
                <button class="mini-cal-nav" on:click=next_month>"›"</button>
            </div>
            <div class="mini-cal-grid">
                {WEEKDAY_SHORT.iter().map(|n| view!{<div class="mini-cal-dow">{*n}</div>}).collect_view()}
                    {move || {
                        let (y,m) = month.get();
                        let first_dow = weekday_of_first(y,m);
                        let num_days  = days_in_month(y,m);

                        let evs = events.get();
                        let marks: Vec<Option<MonthDayMark>> = compute_month_day_marks(&evs, y, m);

                        let mut cells: Vec<AnyView> = Vec::new();
                        for _ in 0..first_dow {
                            cells.push(view!{<div class="mini-cal-cell mini-cal-empty"></div>}.into_any());
                        }
                        for d in 1..=num_days {
                            let is_today = today==(y,m,d);
                            let mark = marks[d as usize].clone();

                            let (dy,dm,dd) = (y,m,d);
                            let on_click = move |_| {
                                week_monday.set(week_monday_of(dy,dm,dd));
                                mobile_day.set((dy,dm,dd));
                            };
                            let in_week = move || {
                                let (my,mm,md) = week_monday.get();
                                let (sy,sm,sd) = add_days(my,mm,md,6);
                                let jd = ymd_to_jdn(dy,dm,dd);
                                jd >= ymd_to_jdn(my,mm,md) && jd <= ymd_to_jdn(sy,sm,sd)
                            };
                            let base_cls = {
                                let mut c = String::from("mini-cal-cell");
                                if is_today { c.push_str(" mini-cal-today"); }
                                c
                            };
                            let line = mark.map(|mk| {
                                if !mk.multiday {
                                    // Single-day event: a short pill, always centered — never
                                    // merges with a neighbor even if it also has an event.
                                    let style = format!(
                                        "background:{};left:22%;right:22%;border-radius:2px",
                                        mk.color
                                    );
                                    view! { <span class="mini-cal-line mini-cal-line-single" style=style></span> }
                                } else {
                                    let left  = if mk.join_left  { "-1px" } else { "0" };
                                    let right = if mk.join_right { "-1px" } else { "0" };
                                    let clip  = bevel_clip_path(mk.bevel_left, mk.bevel_right)
                                        .map(|p| format!("clip-path:{p};"))
                                        .unwrap_or_default();
                                    let style = format!(
                                        "background:{};left:{left};right:{right};{clip}",
                                        mk.color
                                    );
                                    view! { <span class="mini-cal-line mini-cal-line-multi" style=style></span> }
                                }
                            });
                            cells.push(view!{
                                <div
                                    class=move || {
                                        let mut c = base_cls.clone();
                                        if in_week() { c.push_str(" mini-cal-in-week"); }
                                        c
                                    }
                                    on:click=on_click
                                >
                                    {line}
                                    <span class="mini-cal-daynum">{d}</span>
                                </div>
                            }.into_any());
                        }
                        cells
                    }}
            </div>
        </div>
    }
}
