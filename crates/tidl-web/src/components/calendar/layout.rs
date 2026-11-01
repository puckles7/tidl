//! Event layout algorithms — all-day banner placement and timed-event overlap.

use super::date_helpers::{days_in_month, parse_dt, weekday_of_first, ymd_to_jdn};
use crate::api::Event;

pub struct AllDaySlot {
    pub row: usize,
    pub left_col: f64,
    pub right_col: f64,
    pub clips_left: bool,
    pub clips_right: bool,
}

/// For each event in the all-day banner (all-day events and multi-day timed
/// events, pre-filtered to those touching the visible week), return an
/// [`AllDaySlot`] describing which row and horizontal extent to render.
pub fn compute_allday_layout(events: &[Event], week_days: &[(i32, u32, u32)]) -> Vec<AllDaySlot> {
    let n = week_days.len();
    if n == 0 || events.is_empty() {
        return vec![];
    }

    let day_s = |&(y, m, d): &(i32, u32, u32)| -> String { format!("{:04}-{:02}-{:02}", y, m, d) };
    let first_str = day_s(&week_days[0]);
    let last_str = day_s(&week_days[n - 1]);

    let spans: Vec<(f64, f64, bool, bool)> = events
        .iter()
        .map(|e| {
            let ev_start_date = &e.start[..10.min(e.start.len())];
            let ev_end_date = &e.end[..10.min(e.end.len())];

            let clips_left = ev_start_date < first_str.as_str();
            let clips_right = ev_end_date > last_str.as_str();

            let start_frac: f64 = if e.all_day {
                0.0
            } else {
                parse_dt(&e.start)
                    .map(|(_, _, _, h, m)| (h * 60 + m) as f64 / 1440.0)
                    .unwrap_or(0.0)
            };
            let end_frac: f64 = if e.all_day {
                1.0
            } else {
                parse_dt(&e.end)
                    .map(|(_, _, _, h, m)| (h * 60 + m) as f64 / 1440.0)
                    .unwrap_or(1.0)
            };

            let start_col_idx: f64 = if clips_left {
                0.0
            } else {
                week_days
                    .iter()
                    .position(|d| day_s(d) == ev_start_date)
                    .map(|i| i as f64 + start_frac)
                    .unwrap_or(0.0)
            };
            let end_col_idx: f64 = if clips_right {
                n as f64
            } else if e.all_day {
                week_days
                    .iter()
                    .position(|d| day_s(d) == ev_end_date)
                    .map(|i| i as f64 + 1.0)
                    .unwrap_or(n as f64)
            } else {
                week_days
                    .iter()
                    .position(|d| day_s(d) == ev_end_date)
                    .map(|i| i as f64 + end_frac)
                    .unwrap_or(n as f64)
            };

            (start_col_idx, end_col_idx, clips_left, clips_right)
        })
        .collect();

    let mut rows: Vec<usize> = vec![0; events.len()];
    let mut row_right: Vec<f64> = Vec::new();
    let mut order: Vec<usize> = (0..events.len()).collect();
    order.sort_by(|&a, &b| {
        spans[a]
            .0
            .partial_cmp(&spans[b].0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(
                spans[a]
                    .1
                    .partial_cmp(&spans[b].1)
                    .unwrap_or(std::cmp::Ordering::Equal),
            )
    });

    for &i in &order {
        let (lc, rc, _, _) = spans[i];
        let row = row_right
            .iter()
            .position(|&end| end <= lc)
            .unwrap_or_else(|| {
                row_right.push(-1.0);
                row_right.len() - 1
            });
        rows[i] = row;
        row_right[row] = rc;
    }

    spans
        .into_iter()
        .zip(rows)
        .map(|((lc, rc, cl, cr), row)| AllDaySlot {
            row,
            left_col: lc,
            right_col: rc,
            clips_left: cl,
            clips_right: cr,
        })
        .collect()
}

/// For each timed event in `events` (already filtered to one day), return
/// `(left_pct, width_pct)` — the horizontal slot within the column.
pub fn compute_layout(events: &[Event]) -> Vec<(f64, f64)> {
    let n = events.len();
    if n == 0 {
        return vec![];
    }

    let s_m = |e: &Event| -> i32 {
        let (_, _, _, h, m) = parse_dt(&e.start).unwrap_or((0, 0, 0, 0, 0));
        h as i32 * 60 + m as i32
    };
    let e_m = |e: &Event| -> i32 {
        let (_, _, _, h, m) = parse_dt(&e.end).unwrap_or((0, 0, 0, 1, 0));
        (h as i32 * 60 + m as i32).max(s_m(e) + 15)
    };

    let mut order: Vec<usize> = (0..n).collect();
    order.sort_by_key(|&i| s_m(&events[i]));

    let mut lanes: Vec<i32> = vec![0; n];
    let mut lane_end: Vec<i32> = Vec::new();

    for &i in &order {
        let start = s_m(&events[i]);
        let lane = lane_end
            .iter()
            .position(|&end| end <= start)
            .unwrap_or_else(|| {
                lane_end.push(0);
                lane_end.len() - 1
            });
        lanes[i] = lane as i32;
        lane_end[lane] = e_m(&events[i]);
    }

    let overlaps = |a: usize, b: usize| -> bool {
        s_m(&events[a]) < e_m(&events[b]) && s_m(&events[b]) < e_m(&events[a])
    };

    let mut parent: Vec<usize> = (0..n).collect();
    fn find(p: &mut Vec<usize>, x: usize) -> usize {
        if p[x] != x {
            p[x] = find(p, p[x]);
        }
        p[x]
    }
    for a in 0..n {
        for b in (a + 1)..n {
            if overlaps(a, b) {
                let ra = find(&mut parent, a);
                let rb = find(&mut parent, b);
                if ra != rb {
                    parent[ra] = rb;
                }
            }
        }
    }

    let mut cluster_max: std::collections::HashMap<usize, i32> = std::collections::HashMap::new();
    for (i, &lane) in lanes.iter().enumerate() {
        let root = find(&mut parent, i);
        let entry = cluster_max.entry(root).or_insert(0);
        if lane > *entry {
            *entry = lane;
        }
    }

    lanes
        .iter()
        .enumerate()
        .map(|(i, &lane)| {
            let root = find(&mut parent, i);
            let num_lanes = cluster_max[&root] + 1;
            let left_pct = lane as f64 / num_lanes as f64 * 100.0;
            let width_pct = 100.0 / num_lanes as f64;
            (left_pct, width_pct)
        })
        .collect()
}

/// Per-day render info for the month mini-calendar's event-line indicator.
#[derive(Clone, Debug, PartialEq)]
pub struct MonthDayMark {
    pub color: String,
    /// False for a single-day event — rendered as a short centered pill,
    /// never merged with a neighboring day's mark even if adjacent.
    pub multiday: bool,
    /// True: this day's bar should merge with the day to its left (same
    /// event continues from there) — straight edge, no rounding.
    pub join_left: bool,
    pub join_right: bool,
    /// True: this day is the event's real start/end date (not just where
    /// the visible month or grid row happens to cut it off) — chamfered edge.
    pub bevel_left: bool,
    pub bevel_right: bool,
}

#[derive(Clone)]
struct DaySpan {
    color: String,
    start_jdn: i32,
    end_jdn: i32,
}

/// For each day of `year`-`month`, the render info for the first event
/// covering that day (`None` = no event). Index 0 is unused padding so
/// callers can index directly by day-of-month (1..=num_days).
///
/// Two adjacent days only merge into one continuous bar when they're
/// covered by the *same* event span (same start/end date) — this is what
/// distinguishes a real multi-day event from two different single-day (or
/// multi-day) events that simply happen to fall on neighboring days.
pub fn compute_month_day_marks(
    events: &[Event],
    year: i32,
    month: u32,
) -> Vec<Option<MonthDayMark>> {
    let first_dow = weekday_of_first(year, month) as usize;
    let num_days = days_in_month(year, month);
    let month_start_jdn = ymd_to_jdn(year, month, 1);
    let month_end_jdn = ymd_to_jdn(year, month, num_days);

    let mut day_span: Vec<Option<DaySpan>> = vec![None; num_days as usize + 2];

    // Multi-day all-day events are laid down first so a long-running span
    // always wins a day's color/continuity — a single-day event (all-day or
    // timed) landing in the middle of it shouldn't break up the bar — then
    // single-day all-day events, then timed events fill in whatever's left.
    struct Candidate<'a> {
        event: &'a Event,
        start_jdn: i32,
        end_jdn: i32,
    }
    let mut candidates: Vec<Candidate> = events
        .iter()
        .filter_map(|e| {
            let s = e.start.trim();
            let t = e.end.trim();
            if s.len() < 10 || t.len() < 10 {
                return None;
            }
            let (Ok(sy), Ok(sm), Ok(sd)): (Result<i32, _>, Result<u32, _>, Result<u32, _>) =
                (s[0..4].parse(), s[5..7].parse(), s[8..10].parse())
            else {
                return None;
            };
            let (Ok(ey), Ok(em), Ok(ed)): (Result<i32, _>, Result<u32, _>, Result<u32, _>) =
                (t[0..4].parse(), t[5..7].parse(), t[8..10].parse())
            else {
                return None;
            };
            Some(Candidate {
                event: e,
                start_jdn: ymd_to_jdn(sy, sm, sd),
                end_jdn: ymd_to_jdn(ey, em, ed),
            })
        })
        .collect();

    // Category first (multi-day all-day > single-day all-day > timed), then
    // within a category the longer-running event wins — so e.g. two
    // overlapping multi-day all-day events don't hand a continuous bar off
    // mid-week to whichever is shorter, breaking the longer one's join.
    let priority = |c: &Candidate| -> (u8, i32) {
        let category = match (c.event.all_day, c.end_jdn > c.start_jdn) {
            (true, true) => 0,  // multi-day all-day
            (true, false) => 1, // single-day all-day
            (false, _) => 2,    // timed
        };
        (category, c.start_jdn - c.end_jdn) // negative duration: longest first
    };
    candidates.sort_by_key(priority);

    for c in &candidates {
        let first_jdn = c.start_jdn.max(month_start_jdn);
        let last_jdn = c.end_jdn.min(month_end_jdn);
        if first_jdn > last_jdn {
            continue;
        }

        let first_d = (first_jdn - month_start_jdn + 1) as usize;
        let last_d = (last_jdn - month_start_jdn + 1) as usize;
        let color = c
            .event
            .calendar_color
            .clone()
            .unwrap_or_else(|| "var(--primary-color)".to_string());

        for slot in day_span.iter_mut().take(last_d + 1).skip(first_d) {
            if slot.is_none() {
                *slot = Some(DaySpan {
                    color: color.clone(),
                    start_jdn: c.start_jdn,
                    end_jdn: c.end_jdn,
                });
            }
        }
    }

    let same_span = |a: &DaySpan, b: &Option<DaySpan>| -> bool {
        b.as_ref()
            .is_some_and(|b| b.start_jdn == a.start_jdn && b.end_jdn == a.end_jdn)
    };

    let mut marks: Vec<Option<MonthDayMark>> = vec![None; num_days as usize + 1];
    for d in 1..=num_days as usize {
        let Some(span) = &day_span[d] else { continue };
        let multiday = span.end_jdn > span.start_jdn;
        let day_jdn = month_start_jdn + d as i32 - 1;
        let row = (first_dow + d - 1) / 7;

        let left_in_row = d > 1 && (first_dow + d - 2) / 7 == row;
        let right_in_row = d < num_days as usize && (first_dow + d) / 7 == row;

        let join_left = multiday && left_in_row && same_span(span, &day_span[d - 1]);
        let join_right = multiday && right_in_row && same_span(span, &day_span[d + 1]);

        let bevel_left = multiday && !join_left && day_jdn == span.start_jdn;
        let bevel_right = multiday && !join_right && day_jdn == span.end_jdn;

        marks[d] = Some(MonthDayMark {
            color: span.color.clone(),
            multiday,
            join_left,
            join_right,
            bevel_left,
            bevel_right,
        });
    }
    marks
}

#[cfg(test)]
mod tests {
    use super::super::date_helpers::add_days;
    use super::*;

    fn make_event(start: &str, end: &str, all_day: bool) -> Event {
        Event {
            id: "uid1".into(),
            summary: "Test".into(),
            start: start.into(),
            end: end.into(),
            all_day,
            description: None,
            priority: None,
            alarms: vec![],
            calendar: "personal".into(),
            calendar_color: None,
            rrule: None,
            occurrence_start: None,
            is_exception: false,
            start_epoch_ms: 0,
            owner_username: String::new(),
            shared_permission: None,
        }
    }

    fn week_mon() -> Vec<(i32, u32, u32)> {
        (0..7i32).map(|i| add_days(2025, 4, 14, i)).collect()
    }

    #[test]
    fn timed_multiday_start_frac_noon() {
        let ev = make_event("2025-04-15 12:00", "2025-04-17 18:00", false);
        let days = week_mon();
        let slots = compute_allday_layout(&[ev], &days);
        assert_eq!(slots.len(), 1);
        let s = &slots[0];
        assert!(
            (s.left_col - 1.5).abs() < 1e-9,
            "expected left_col=1.5, got {}",
            s.left_col
        );
        assert!(
            (s.right_col - 3.75).abs() < 1e-9,
            "expected right_col=3.75, got {}",
            s.right_col
        );
        assert!(!s.clips_left);
        assert!(!s.clips_right);
    }

    #[test]
    fn timed_multiday_start_frac_midnight() {
        let ev = make_event("2025-04-14 00:00", "2025-04-16 06:00", false);
        let days = week_mon();
        let slots = compute_allday_layout(&[ev], &days);
        assert_eq!(slots.len(), 1);
        let s = &slots[0];
        assert!(
            (s.left_col - 0.0).abs() < 1e-9,
            "expected left_col=0.0 (midnight Mon), got {}",
            s.left_col
        );
    }

    #[test]
    fn timed_multiday_clips_left() {
        let ev = make_event("2025-04-10 12:00", "2025-04-15 18:00", false);
        let days = week_mon();
        let slots = compute_allday_layout(&[ev], &days);
        assert_eq!(slots.len(), 1);
        let s = &slots[0];
        assert!(s.clips_left, "should clip left");
        assert!((s.left_col - 0.0).abs() < 1e-9);
    }

    // ── compute_month_day_marks ─────────────────────────────────────────────
    // All cases use April 2025 (2025-04-01 is a Tuesday, so weekday_of_first
    // is 1 and days 14-20 fall in one grid row, day 21 starts the next row —
    // matches week_mon()'s assumption that 2025-04-14 is a Monday).

    #[test]
    fn single_day_event_is_not_multiday() {
        let ev = make_event("2025-04-15", "2025-04-15", true);
        let marks = compute_month_day_marks(&[ev], 2025, 4);

        let m15 = marks[15].as_ref().expect("day 15 should have a mark");
        assert!(!m15.multiday);
        assert!(!m15.join_left && !m15.join_right);
        assert!(!m15.bevel_left && !m15.bevel_right);
        assert!(marks[14].is_none());
        assert!(marks[16].is_none());
    }

    #[test]
    fn two_adjacent_single_day_events_do_not_merge() {
        let a = make_event("2025-04-14", "2025-04-14", true);
        let b = make_event("2025-04-15", "2025-04-15", true);
        let marks = compute_month_day_marks(&[a, b], 2025, 4);

        assert!(!marks[14].as_ref().unwrap().multiday);
        assert!(!marks[15].as_ref().unwrap().multiday);
    }

    #[test]
    fn three_day_event_fully_visible_bevels_both_ends() {
        let ev = make_event("2025-04-14", "2025-04-16", true);
        let marks = compute_month_day_marks(&[ev], 2025, 4);

        let start = marks[14].as_ref().unwrap();
        assert!(start.multiday);
        assert!(!start.join_left && start.bevel_left);
        assert!(start.join_right && !start.bevel_right);

        let mid = marks[15].as_ref().unwrap();
        assert!(mid.join_left && mid.join_right);
        assert!(!mid.bevel_left && !mid.bevel_right);

        let end = marks[16].as_ref().unwrap();
        assert!(end.join_left && !end.join_right);
        assert!(!end.bevel_left && end.bevel_right);
    }

    #[test]
    fn event_clipped_at_month_start_is_flat_not_beveled() {
        let ev = make_event("2025-03-30", "2025-04-02", true);
        let marks = compute_month_day_marks(&[ev], 2025, 4);

        let d1 = marks[1].as_ref().unwrap();
        assert!(d1.multiday);
        assert!(
            !d1.join_left && !d1.bevel_left,
            "clipped edge must stay flat, not beveled"
        );
        assert!(d1.join_right);

        let d2 = marks[2].as_ref().unwrap();
        assert!(d2.join_left);
        assert!(
            !d2.join_right && d2.bevel_right,
            "true event end should be beveled"
        );
    }

    #[test]
    fn adjacent_distinct_multiday_events_do_not_merge() {
        let a = make_event("2025-04-14", "2025-04-15", true);
        let b = make_event("2025-04-16", "2025-04-17", true);
        let marks = compute_month_day_marks(&[a, b], 2025, 4);

        let a_end = marks[15].as_ref().unwrap();
        assert!(
            !a_end.join_right && a_end.bevel_right,
            "A's real end must bevel, not join B"
        );

        let b_start = marks[16].as_ref().unwrap();
        assert!(
            !b_start.join_left && b_start.bevel_left,
            "B's real start must bevel, not join A"
        );
    }

    #[test]
    fn longer_multiday_event_wins_over_shorter_overlapping_one() {
        // Both start 4/14 (one grid row: 14th-20th, see module test comment).
        // Short event (14th-16th) listed first so it would win the naive
        // "first event wins" slot fill on the days they both cover; the
        // longer event (14th-19th) should claim those days instead so its
        // bar doesn't get cut at day 16 where the short one ends.
        let short = Event {
            calendar_color: Some("blue".into()),
            ..make_event("2025-04-14", "2025-04-16", true)
        };
        let long = Event {
            calendar_color: Some("blue".into()),
            ..make_event("2025-04-14", "2025-04-19", true)
        };
        let marks = compute_month_day_marks(&[short, long], 2025, 4);

        let start = marks[14].as_ref().unwrap();
        assert!(start.bevel_left, "day 14 is the long span's real start");
        let end = marks[19].as_ref().unwrap();
        assert!(end.bevel_right, "day 19 is the long span's real end");

        // interior days must all join continuously, including day 16 where
        // the shorter event's own end would otherwise have beveled the bar.
        for (i, mark) in marks[15..=18].iter().enumerate() {
            let d = i + 15;
            let m = mark.as_ref().unwrap();
            assert!(
                m.join_left && m.join_right,
                "day {d} should stay joined to the long span, not cut at the short one's end"
            );
        }
    }

    #[test]
    fn all_day_event_color_trumps_timed_event_regardless_of_order() {
        let timed = Event {
            calendar_color: Some("purple".into()),
            ..make_event("2025-04-15 09:00", "2025-04-15 10:00", false)
        };
        let allday = Event {
            calendar_color: Some("blue".into()),
            ..make_event("2025-04-15", "2025-04-16", true)
        };

        let marks = compute_month_day_marks(&[timed.clone(), allday.clone()], 2025, 4);
        assert_eq!(
            marks[15].as_ref().unwrap().color,
            "blue",
            "all-day event should win when listed after the timed event"
        );

        let marks = compute_month_day_marks(&[allday, timed], 2025, 4);
        assert_eq!(
            marks[15].as_ref().unwrap().color,
            "blue",
            "all-day event should win when listed before the timed event"
        );
    }

    #[test]
    fn single_day_event_does_not_break_a_multiday_events_continuity() {
        // A long-running all-day event (14th-17th) with a same-calendar
        // single-day all-day event landing in the middle (15th) — listed
        // *first* so it would win the naive "first event wins" slot fill.
        let single = make_event("2025-04-15", "2025-04-15", true);
        let multi = make_event("2025-04-14", "2025-04-17", true);
        let marks = compute_month_day_marks(&[single, multi], 2025, 4);

        let mid = marks[15].as_ref().unwrap();
        assert!(
            mid.multiday,
            "the multi-day span should win, not the single-day one"
        );
        assert!(
            mid.join_left && mid.join_right,
            "bar must stay continuous through day 15"
        );
    }

    #[test]
    fn timed_single_day_event_does_not_break_a_multiday_events_continuity() {
        let timed = make_event("2025-04-15 09:00", "2025-04-15 10:00", false);
        let multi = make_event("2025-04-14", "2025-04-17", true);
        let marks = compute_month_day_marks(&[timed, multi], 2025, 4);

        let mid = marks[15].as_ref().unwrap();
        assert!(mid.multiday);
        assert!(mid.join_left && mid.join_right);
    }

    #[test]
    fn multiday_event_wrapping_grid_row_breaks_flat_at_the_wrap() {
        let ev = make_event("2025-04-20", "2025-04-22", true);
        let marks = compute_month_day_marks(&[ev], 2025, 4);

        let last_of_row = marks[20].as_ref().unwrap();
        assert!(
            !last_of_row.join_right && !last_of_row.bevel_right,
            "row wrap is not the event's real end — must stay flat, not beveled"
        );

        let first_of_next_row = marks[21].as_ref().unwrap();
        assert!(
            !first_of_next_row.join_left && !first_of_next_row.bevel_left,
            "row wrap is not the event's real start — must stay flat, not beveled"
        );
    }
}
