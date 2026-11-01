//! Pure date/time helpers — no Leptos dependencies.

pub const WEEKDAY_SHORT: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
pub const MONTH_SHORT: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];
pub const MONTH_LONG: [&str; 12] = [
    "January",
    "February",
    "March",
    "April",
    "May",
    "June",
    "July",
    "August",
    "September",
    "October",
    "November",
    "December",
];

pub fn days_in_month(year: i32, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 => {
            if (year % 4 == 0 && year % 100 != 0) || (year % 400 == 0) {
                29
            } else {
                28
            }
        }
        _ => 30,
    }
}

pub fn weekday_of_first(year: i32, month: u32) -> u32 {
    let t = [0i32, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    let dow = (y + y / 4 - y / 100 + y / 400 + t[(month as usize) - 1] + 1) % 7;
    ((dow + 6) % 7) as u32
}

pub fn parse_dt(s: &str) -> Option<(i32, u32, u32, u32, u32)> {
    let s = s.trim();
    if s.len() < 16 {
        return None;
    }
    Some((
        s[0..4].parse().ok()?,
        s[5..7].parse().ok()?,
        s[8..10].parse().ok()?,
        s[11..13].parse().ok()?,
        s[14..16].parse().ok()?,
    ))
}

pub fn month_label(year: i32, month: u32) -> String {
    format!("{} {}", MONTH_LONG[(month as usize) - 1], year)
}

pub fn today_ymd() -> (i32, u32, u32) {
    #[cfg(feature = "hydrate")]
    {
        let d = js_sys::Date::new_0();
        (
            d.get_full_year() as i32,
            d.get_month() as u32 + 1,
            d.get_date() as u32,
        )
    }
    #[cfg(not(feature = "hydrate"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        let days = (SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
            / 86400) as i32;
        let z = days + 719468;
        let era = if z >= 0 { z } else { z - 146096 } / 146097;
        let doe = z - era * 146097;
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
        let y = yoe + era * 400;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let d = doy - (153 * mp + 2) / 5 + 1;
        let m = if mp < 10 { mp + 3 } else { mp - 9 };
        let y = if m <= 2 { y + 1 } else { y };
        (y, m as u32, d as u32)
    }
}

/// Minutes since midnight (0-1439) in `tz` (an IANA timezone name) — for
/// positioning the current-time indicator line in the week/day time grid,
/// which is laid out 1px per minute (`.week-day-body` is 1440px tall).
///
/// Uses `Intl.DateTimeFormat` with the caller-supplied timezone rather than
/// `js_sys::Date`'s local getters, which read the browser's system clock —
/// wrong whenever that differs from the user's chosen timezone preference,
/// or when the browser spoofs it (e.g. LibreWolf's resistFingerprinting
/// reports UTC). Same reasoning as `clock_time` in `calendar/mod.rs`; pass
/// it `UserPrefsCtx`'s `timezone` field (defaulting to `"UTC"`).
///
/// SSR fallback uses UTC epoch seconds rather than true local time, same as
/// `today_ymd` above and for the same reason: it's never actually rendered,
/// since real interactivity only exists post-hydration.
pub fn local_now_minutes(tz: &str) -> u32 {
    #[cfg(feature = "hydrate")]
    {
        let code = format!(
            "new Intl.DateTimeFormat('en-GB',\
             {{timeZone:'{}',hour:'2-digit',minute:'2-digit',hour12:false}}).format(new Date())",
            tz.replace('\'', "\\'")
        );
        js_sys::eval(&code)
            .ok()
            .and_then(|v| v.as_string())
            .and_then(|s| {
                let (h, m) = s.split_once(':')?;
                // Some engines format midnight as "24:00" under hour12:false
                // instead of "00:00" — mod 24 normalizes either form.
                Some((h.parse::<u32>().ok()? % 24) * 60 + m.parse::<u32>().ok()?)
            })
            .unwrap_or(0)
    }
    #[cfg(not(feature = "hydrate"))]
    {
        let _ = tz;
        use std::time::{SystemTime, UNIX_EPOCH};
        let secs = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        ((secs % 86400) / 60) as u32
    }
}

pub fn add_days(year: i32, month: u32, day: u32, delta: i32) -> (i32, u32, u32) {
    jdn_to_ymd(ymd_to_jdn(year, month, day) + delta)
}

pub fn ymd_to_jdn(y: i32, m: u32, d: u32) -> i32 {
    let m = m as i32;
    let d = d as i32;
    (1461 * (y + 4800 + (m - 14) / 12)) / 4 + (367 * (m - 2 - 12 * ((m - 14) / 12))) / 12
        - (3 * ((y + 4900 + (m - 14) / 12) / 100)) / 4
        + d
        - 32075
}

pub fn jdn_to_ymd(jdn: i32) -> (i32, u32, u32) {
    let l = jdn + 68569;
    let n = (4 * l) / 146097;
    let l = l - (146097 * n + 3) / 4;
    let i = (4000 * (l + 1)) / 1461001;
    let l = l - (1461 * i) / 4 + 31;
    let j = (80 * l) / 2447;
    let d = l - (2447 * j) / 80;
    let l = j / 11;
    let m = j + 2 - 12 * l;
    let y = 100 * (n - 49) + i + l;
    (y, m as u32, d as u32)
}

pub fn week_monday_of(year: i32, month: u32, day: u32) -> (i32, u32, u32) {
    let t = [0i32, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    let dow = (y + y / 4 - y / 100 + y / 400 + t[(month as usize) - 1] + day as i32) % 7;
    add_days(year, month, day, -((dow + 6) % 7))
}

/// Return the inclusive date span the main grid is currently displaying —
/// single day / work-week / week / month, depending on `cal_mode`.
///
/// No longer unions this with the mini-calendar's own displayed month (see
/// `mini_cal_range`) except for `CalMode::Month`, where `mini_month` *is*
/// the signal the main grid itself pages (see `WeekGrid`'s `do_prev`/
/// `do_next`: in `Month` mode they update `mini_month` directly, not
/// `week_monday`) — main grid and mini-cal are the same view there, so
/// `mini_month` is exactly right, not a stand-in for something wider.
///
/// For the other three modes, `week_monday`/`mobile_day` are the sole
/// drivers, and `mini_month` is a separate signal that can drift arbitrarily
/// far from them: paging the mini-cal's own prev/next buttons (`mini_cal.rs`)
/// moves only `mini_month`, not `week_monday`. An earlier version still
/// unioned the mini-cal's span in here for those modes too, so the mini-cal's
/// event dots stayed correct even then — but a union of two far-apart spans
/// covers everything *between* them as well, including months neither view
/// shows, and each further mini-cal page stretched `get_events`'s range (and
/// its response payload) a little more, unbounded short of `MAX_RANGE_DAYS`.
/// The mini-cal now fetches its own events independently via
/// `mini_cal_range` (see `CalendarView`'s `mini_events_res`), so this only
/// ever needs to cover what the main grid itself renders.
///
/// Used to bound server-side RRULE expansion (`get_events(range_start,
/// range_end)`) to what's actually visible, instead of a fixed horizon.
pub fn visible_range(
    mini_month: (i32, u32),
    week_monday: (i32, u32, u32),
    cal_mode: super::CalMode,
    mobile_day: (i32, u32, u32),
) -> ((i32, u32, u32), (i32, u32, u32)) {
    use super::CalMode;

    match cal_mode {
        CalMode::Day => (mobile_day, mobile_day),
        CalMode::WorkWeek => {
            let (y, m, d) = week_monday;
            (week_monday, add_days(y, m, d, 4))
        }
        CalMode::Week => {
            let (y, m, d) = week_monday;
            (week_monday, add_days(y, m, d, 6))
        }
        CalMode::Month => mini_cal_range(mini_month),
    }
}

/// Return the inclusive 6-row-grid date span the mini-calendar needs event
/// data for: Monday on/before `month`'s 1st, through 41 days later.
///
/// `MonthMiniCal` (mini_cal.rs) only ever renders `[1, days_in_month]`, with
/// blank placeholder cells before the 1st — it never shows real
/// adjacent-month dates or pads to a full 6-row grid. Fetching the wider
/// 42-day span anyway is always safe (it never hides events, it just isn't
/// the mini-cal's literal displayed span) and matches `CalMode::Month`'s own
/// main-grid span, which reuses this function.
pub fn mini_cal_range(month: (i32, u32)) -> ((i32, u32, u32), (i32, u32, u32)) {
    let (y, m) = month;
    let first_dow = weekday_of_first(y, m) as i32;
    let first = add_days(y, m, 1, -first_dow);
    let last = add_days(first.0, first.1, first.2, 41);
    (first, last)
}

/// Returns 0=Mon … 6=Sun for any calendar date.
pub fn weekday_idx(year: i32, month: u32, day: u32) -> usize {
    let t = [0i32, 3, 2, 5, 0, 3, 5, 1, 4, 6, 2, 4];
    let y = if month < 3 { year - 1 } else { year };
    let dow = (y + y / 4 - y / 100 + y / 400 + t[(month as usize) - 1] + day as i32) % 7;
    ((dow + 6) % 7) as usize
}

/// Returns the current time in milliseconds via the browser Performance API.
/// Only available in WASM; returns 0.0 on the server.
#[cfg(target_arch = "wasm32")]
pub fn perf_now() -> f64 {
    leptos::web_sys::window()
        .and_then(|w| w.performance())
        .map(|p: leptos::web_sys::Performance| p.now())
        .unwrap_or(0.0)
}

pub fn snap15(total_mins: i32) -> i32 {
    let snapped = (total_mins + 7) / 15 * 15;
    snapped.clamp(0, 23 * 60 + 59)
}

pub fn mins_to_hhmm(mins: i32) -> (u32, u32) {
    let mins = mins.clamp(0, 1439);
    (mins as u32 / 60, mins as u32 % 60)
}

pub fn dt_to_server(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> String {
    format!("{:04}-{:02}-{:02} {:02}:{:02}", y, mo, d, h, mi)
}

pub fn to_input_dt(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> String {
    format!("{:04}-{:02}-{:02}T{:02}:{:02}", y, mo, d, h, mi)
}

pub fn input_to_server_dt(s: &str) -> String {
    s.replace('T', " ")
}

/// "2026-03-20 09:30:00" or "2026-03-20 09:30" → "2026-03-20T09:30"
pub fn input_dt_from_server(s: &str) -> String {
    let s = s.trim();
    if s.len() < 16 {
        return s.to_string();
    }
    format!("{}T{}", &s[0..10], &s[11..16])
}

pub fn snap30(h: u32, m: u32) -> (u32, u32) {
    (h, if m < 30 { 0 } else { 30 })
}

/// Parse a "HH:MM" time-of-day string into `(hour, minute)`. Used by the
/// hour/minute `<select>` pair in `DateTimeInput` to read the current value
/// out of the `"YYYY-MM-DDTHH:MM"` datetime-local string it's bound to.
pub fn parse_hhmm(s: &str) -> Option<(u32, u32)> {
    let (h, m) = s.split_once(':')?;
    Some((h.trim().parse().ok()?, m.trim().parse().ok()?))
}

/// Format hours and minutes according to the user's time-format preference.
/// fmt: "12h" | "24h" | "zulu"
pub fn format_hm(h: u32, m: u32, fmt: &str) -> String {
    match fmt {
        "24h" => format!("{:02}:{:02}", h, m),
        "zulu" => format!("{:02}:{:02}Z", h, m),
        _ => {
            let (period, h12) = if h < 12 {
                ("am", if h == 0 { 12 } else { h })
            } else {
                ("pm", if h == 12 { 12 } else { h - 12 })
            };
            if m == 0 {
                format!("{}{}", h12, period)
            } else {
                format!("{}:{:02}{}", h12, m, period)
            }
        }
    }
}

/// Label shown in the time gutter for a given hour.
pub fn format_hour_label(h: u32, fmt: &str) -> String {
    if h == 0 {
        return String::new();
    }
    match fmt {
        "24h" => format!("{:02}", h),
        "zulu" => format!("{:02}Z", h),
        _ => {
            if h < 12 {
                format!("{} am", h)
            } else if h == 12 {
                "12 pm".into()
            } else {
                format!("{} pm", h - 12)
            }
        }
    }
}

pub fn event_position_from(start: &str, end: &str) -> (f64, f64) {
    let (_, _, _, sh, sm) = parse_dt(start).unwrap_or((0, 0, 0, 0, 0));
    let (_, _, _, eh, em) = parse_dt(end).unwrap_or((0, 0, 0, 1, 0));
    let start_m = sh as f64 * 60.0 + sm as f64;
    let end_m = (eh as f64 * 60.0 + em as f64)
        .max(start_m + 15.0)
        .min(1440.0);
    (start_m / 1440.0 * 100.0, (end_m - start_m) / 1440.0 * 100.0)
}

pub fn short_time(s: &str, fmt: &str) -> String {
    parse_dt(s)
        .map(|(_, _, _, h, m)| format_hm(h, m, fmt))
        .unwrap_or_default()
}

/// Parse a date-only string "YYYY-MM-DD" into `(year, month, day)`.
pub fn parse_date(s: &str) -> Option<(i32, u32, u32)> {
    let s = s.trim();
    if s.len() < 10 {
        return None;
    }
    Some((
        s[0..4].parse().ok()?,
        s[5..7].parse().ok()?,
        s[8..10].parse().ok()?,
    ))
}

/// Shift `old_end`'s date by however many days `new_start` moved from
/// `old_start`, preserving `old_end`'s time-of-day suffix (if any) — so
/// moving an event's start date keeps its length instead of silently
/// collapsing or inverting it (buglist item 2: "End date should be bumped
/// up when start date changes"). `old_end` may be date-only ("YYYY-MM-DD")
/// or date+time ("YYYY-MM-DDTHH:MM"); either way only its first 10 chars
/// are touched. Returns `old_end` unchanged if any input fails to parse as
/// at least a date.
pub fn bump_end_date(old_start: &str, new_start: &str, old_end: &str) -> String {
    let (Some((oy, om, od)), Some((ny, nm, nd))) = (parse_date(old_start), parse_date(new_start))
    else {
        return old_end.to_string();
    };
    let delta = ymd_to_jdn(ny, nm, nd) - ymd_to_jdn(oy, om, od);
    let Some((ey, em, ed)) = old_end.get(..10).and_then(parse_date) else {
        return old_end.to_string();
    };
    let (fy, fm, fd) = jdn_to_ymd(ymd_to_jdn(ey, em, ed) + delta);
    let rest = old_end.get(10..).unwrap_or("");
    format!("{:04}-{:02}-{:02}{}", fy, fm, fd, rest)
}

/// "YYYY-MM-DDTHH:MM" → minutes since the JDN epoch, or `None` if either the
/// date or time portion fails to parse. Shared by `bump_end_time` to work in
/// a flat minutes domain instead of juggling date/time components — a shift
/// that crosses midnight (e.g. Start moving from 23:50 to 00:10 the next
/// day) then falls out of the arithmetic for free.
fn datetime_to_minutes(s: &str) -> Option<i64> {
    let (y, mo, d) = s.get(..10).and_then(parse_date)?;
    let (h, mi) = s.get(11..16).and_then(parse_hhmm)?;
    Some(ymd_to_jdn(y, mo, d) as i64 * 1440 + h as i64 * 60 + mi as i64)
}

fn minutes_to_datetime(total: i64) -> String {
    let days = total.div_euclid(1440) as i32;
    let rem = total.rem_euclid(1440);
    let (y, mo, d) = jdn_to_ymd(days);
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}",
        y,
        mo,
        d,
        rem / 60,
        rem % 60
    )
}

/// Shift `old_end` (a full `"YYYY-MM-DDTHH:MM"` datetime) by however many
/// minutes `new_start` moved from `old_start` — the timed counterpart of
/// `bump_end_date`, which only ever preserved whole-day shifts and left the
/// time-of-day untouched. Used when Start's *time* changes (typing or
/// picking from the time popup), so the event's length survives a start
/// time edit the same way it already survives a start date edit. Handles a
/// shift crossing midnight the same as any other, since both inputs are
/// compared in one continuous minutes domain. Returns `old_end` unchanged if
/// any input fails to parse as a full timed datetime.
pub fn bump_end_time(old_start: &str, new_start: &str, old_end: &str) -> String {
    let (Some(old_total), Some(new_total), Some(end_total)) = (
        datetime_to_minutes(old_start),
        datetime_to_minutes(new_start),
        datetime_to_minutes(old_end),
    ) else {
        return old_end.to_string();
    };
    minutes_to_datetime(end_total + (new_total - old_total))
}

#[cfg(test)]
mod tests {
    use super::*;

    // ── days_in_month ─────────────────────────────────────────────────────────

    #[test]
    fn days_31_months() {
        for m in [1u32, 3, 5, 7, 8, 10, 12] {
            assert_eq!(days_in_month(2024, m), 31, "month {m}");
        }
    }

    #[test]
    fn days_30_months() {
        for m in [4u32, 6, 9, 11] {
            assert_eq!(days_in_month(2024, m), 30, "month {m}");
        }
    }

    #[test]
    fn days_feb_common_year() {
        assert_eq!(days_in_month(2023, 2), 28);
    }

    #[test]
    fn days_feb_divisible_by_4() {
        assert_eq!(days_in_month(2024, 2), 29);
    }

    #[test]
    fn days_feb_century_non_leap() {
        // 1900 is divisible by 100 but not 400 → not a leap year
        assert_eq!(days_in_month(1900, 2), 28);
    }

    #[test]
    fn days_feb_400_year_leap() {
        assert_eq!(days_in_month(2000, 2), 29);
    }

    // ── weekday_of_first ──────────────────────────────────────────────────────

    #[test]
    fn weekday_of_first_known_dates() {
        // 2024-01-01 is a Monday (0)
        assert_eq!(weekday_of_first(2024, 1), 0);
        // 2024-04-01 is a Monday (0)
        assert_eq!(weekday_of_first(2024, 4), 0);
        // 2024-07-01 is a Monday (0)
        assert_eq!(weekday_of_first(2024, 7), 0);
        // 2025-01-01 is a Wednesday (2)
        assert_eq!(weekday_of_first(2025, 1), 2);
        // 2026-05-01 is a Friday (4)
        assert_eq!(weekday_of_first(2026, 5), 4);
    }

    // ── parse_dt ──────────────────────────────────────────────────────────────

    #[test]
    fn parse_dt_valid() {
        assert_eq!(parse_dt("2026-05-29 14:30"), Some((2026, 5, 29, 14, 30)));
    }

    #[test]
    fn parse_dt_with_seconds() {
        // seconds portion is ignored — only first 16 chars matter
        assert_eq!(parse_dt("2026-05-29 14:30:00"), Some((2026, 5, 29, 14, 30)));
    }

    #[test]
    fn parse_dt_too_short() {
        assert_eq!(parse_dt("2026-05-29"), None);
        assert_eq!(parse_dt(""), None);
        assert_eq!(parse_dt("2026-05-2"), None);
    }

    #[test]
    fn parse_dt_invalid_numbers() {
        assert_eq!(parse_dt("XXXX-05-29 14:30"), None);
    }

    #[test]
    fn parse_dt_trims_whitespace() {
        assert_eq!(
            parse_dt("  2026-05-29 14:30  "),
            Some((2026, 5, 29, 14, 30))
        );
    }

    // ── month_label ───────────────────────────────────────────────────────────

    #[test]
    fn month_label_january() {
        assert_eq!(month_label(2026, 1), "January 2026");
    }

    #[test]
    fn month_label_december() {
        assert_eq!(month_label(2025, 12), "December 2025");
    }

    // ── ymd_to_jdn / jdn_to_ymd ──────────────────────────────────────────────

    #[test]
    fn jdn_roundtrip_known_dates() {
        for (y, m, d) in [(2024, 1, 1), (2026, 5, 29), (2000, 2, 29), (1970, 1, 1)] {
            assert_eq!(
                jdn_to_ymd(ymd_to_jdn(y, m, d)),
                (y, m, d),
                "{y}-{m:02}-{d:02}"
            );
        }
    }

    #[test]
    fn jdn_epoch_is_known() {
        // Unix epoch 1970-01-01 = JDN 2440588
        assert_eq!(ymd_to_jdn(1970, 1, 1), 2440588);
    }

    // ── add_days ──────────────────────────────────────────────────────────────

    #[test]
    fn add_days_forward_across_month() {
        assert_eq!(add_days(2026, 1, 31, 1), (2026, 2, 1));
    }

    #[test]
    fn add_days_backward_across_year() {
        assert_eq!(add_days(2026, 1, 1, -1), (2025, 12, 31));
    }

    #[test]
    fn add_days_zero() {
        assert_eq!(add_days(2026, 5, 15, 0), (2026, 5, 15));
    }

    #[test]
    fn add_days_leap_day() {
        assert_eq!(add_days(2024, 2, 28, 1), (2024, 2, 29));
        assert_eq!(add_days(2024, 2, 29, 1), (2024, 3, 1));
    }

    // ── week_monday_of ────────────────────────────────────────────────────────

    #[test]
    fn week_monday_of_already_monday() {
        // 2026-05-25 is a Monday
        assert_eq!(week_monday_of(2026, 5, 25), (2026, 5, 25));
    }

    #[test]
    fn week_monday_of_sunday_gives_prior_monday() {
        // 2026-05-31 is a Sunday → Monday is 2026-05-25
        assert_eq!(week_monday_of(2026, 5, 31), (2026, 5, 25));
    }

    #[test]
    fn week_monday_of_crosses_month_boundary() {
        // 2026-06-01 is a Monday → same day
        assert_eq!(week_monday_of(2026, 6, 1), (2026, 6, 1));
        // 2026-06-03 is a Wednesday → Monday is 2026-06-01
        assert_eq!(week_monday_of(2026, 6, 3), (2026, 6, 1));
    }

    // ── weekday_idx ───────────────────────────────────────────────────────────

    #[test]
    fn weekday_idx_known() {
        // 2026-05-25 is Monday → 0
        assert_eq!(weekday_idx(2026, 5, 25), 0);
        // 2026-05-26 Tuesday → 1
        assert_eq!(weekday_idx(2026, 5, 26), 1);
        // 2026-05-31 Sunday → 6
        assert_eq!(weekday_idx(2026, 5, 31), 6);
    }

    // ── snap15 ────────────────────────────────────────────────────────────────

    #[test]
    fn snap15_exact_multiples() {
        assert_eq!(snap15(0), 0);
        assert_eq!(snap15(15), 15);
        assert_eq!(snap15(60), 60);
    }

    #[test]
    fn snap15_rounds_to_nearest() {
        // (n+7)/15*15 — rounds to nearest 15 using integer arithmetic
        assert_eq!(snap15(1), 0); // (1+7)/15=0 → 0
        assert_eq!(snap15(7), 0); // (7+7)/15=0 → 0
        assert_eq!(snap15(8), 15); // (8+7)/15=1 → 15
        assert_eq!(snap15(22), 15); // (22+7)/15=1 → 15
        assert_eq!(snap15(23), 30); // (23+7)/15=2 → 30
    }

    #[test]
    fn snap15_clamps_to_day() {
        // 23*60+59 = 1439 — never exceeds it
        assert_eq!(snap15(1439), 1439);
        assert_eq!(snap15(1440), 1439);
    }

    #[test]
    fn snap15_negative_clamps_to_zero() {
        assert_eq!(snap15(-1), 0);
    }

    // ── mins_to_hhmm ─────────────────────────────────────────────────────────

    #[test]
    fn mins_to_hhmm_basic() {
        assert_eq!(mins_to_hhmm(0), (0, 0));
        assert_eq!(mins_to_hhmm(60), (1, 0));
        assert_eq!(mins_to_hhmm(90), (1, 30));
        assert_eq!(mins_to_hhmm(1439), (23, 59));
    }

    #[test]
    fn mins_to_hhmm_clamps() {
        assert_eq!(mins_to_hhmm(-1), (0, 0));
        assert_eq!(mins_to_hhmm(1440), (23, 59));
    }

    // ── snap30 ────────────────────────────────────────────────────────────────

    #[test]
    fn snap30_below_30_gives_0() {
        assert_eq!(snap30(10, 0), (10, 0));
        assert_eq!(snap30(10, 29), (10, 0));
    }

    #[test]
    fn snap30_at_or_above_30_gives_30() {
        assert_eq!(snap30(10, 30), (10, 30));
        assert_eq!(snap30(10, 59), (10, 30));
    }

    // ── parse_hhmm ────────────────────────────────────────────────────────────

    #[test]
    fn parse_hhmm_valid() {
        assert_eq!(parse_hhmm("09:05"), Some((9, 5)));
        assert_eq!(parse_hhmm("00:00"), Some((0, 0)));
        assert_eq!(parse_hhmm("23:59"), Some((23, 59)));
    }

    #[test]
    fn parse_hhmm_trims_whitespace() {
        assert_eq!(parse_hhmm(" 09 : 05 "), Some((9, 5)));
    }

    #[test]
    fn parse_hhmm_missing_separator() {
        assert_eq!(parse_hhmm("0905"), None);
        assert_eq!(parse_hhmm(""), None);
    }

    #[test]
    fn parse_hhmm_invalid_numbers() {
        assert_eq!(parse_hhmm("ab:cd"), None);
    }

    // ── dt_to_server / to_input_dt ───────────────────────────────────────────

    #[test]
    fn dt_to_server_pads_zeros() {
        assert_eq!(dt_to_server(2026, 1, 5, 9, 3), "2026-01-05 09:03");
    }

    #[test]
    fn to_input_dt_uses_t_separator() {
        assert_eq!(to_input_dt(2026, 5, 29, 14, 30), "2026-05-29T14:30");
    }

    // ── input_to_server_dt ────────────────────────────────────────────────────

    #[test]
    fn input_to_server_dt_replaces_t() {
        assert_eq!(input_to_server_dt("2026-05-29T14:30"), "2026-05-29 14:30");
    }

    // ── input_dt_from_server ─────────────────────────────────────────────────

    #[test]
    fn input_dt_from_server_converts() {
        assert_eq!(
            input_dt_from_server("2026-05-29 14:30:00"),
            "2026-05-29T14:30"
        );
        assert_eq!(input_dt_from_server("2026-05-29 14:30"), "2026-05-29T14:30");
    }

    #[test]
    fn input_dt_from_server_too_short_passthrough() {
        assert_eq!(input_dt_from_server("short"), "short");
    }

    // ── format_hm ────────────────────────────────────────────────────────────

    #[test]
    fn format_hm_24h() {
        assert_eq!(format_hm(9, 5, "24h"), "09:05");
        assert_eq!(format_hm(0, 0, "24h"), "00:00");
        assert_eq!(format_hm(23, 59, "24h"), "23:59");
    }

    #[test]
    fn format_hm_zulu() {
        assert_eq!(format_hm(14, 30, "zulu"), "14:30Z");
        assert_eq!(format_hm(0, 0, "zulu"), "00:00Z");
    }

    #[test]
    fn format_hm_12h_am() {
        assert_eq!(format_hm(0, 0, "12h"), "12am"); // midnight
        assert_eq!(format_hm(1, 0, "12h"), "1am");
        assert_eq!(format_hm(11, 30, "12h"), "11:30am");
    }

    #[test]
    fn format_hm_12h_pm() {
        assert_eq!(format_hm(12, 0, "12h"), "12pm"); // noon
        assert_eq!(format_hm(13, 0, "12h"), "1pm");
        assert_eq!(format_hm(23, 45, "12h"), "11:45pm");
    }

    #[test]
    fn format_hm_12h_no_minutes_omits_colon() {
        assert_eq!(format_hm(9, 0, "12h"), "9am");
        assert_eq!(format_hm(14, 0, "12h"), "2pm");
    }

    // ── format_hour_label ────────────────────────────────────────────────────

    #[test]
    fn format_hour_label_zero_is_empty() {
        assert_eq!(format_hour_label(0, "24h"), "");
        assert_eq!(format_hour_label(0, "12h"), "");
        assert_eq!(format_hour_label(0, "zulu"), "");
    }

    #[test]
    fn format_hour_label_24h() {
        assert_eq!(format_hour_label(9, "24h"), "09");
        assert_eq!(format_hour_label(23, "24h"), "23");
    }

    #[test]
    fn format_hour_label_zulu() {
        assert_eq!(format_hour_label(14, "zulu"), "14Z");
    }

    #[test]
    fn format_hour_label_12h() {
        assert_eq!(format_hour_label(1, "12h"), "1 am");
        assert_eq!(format_hour_label(12, "12h"), "12 pm");
        assert_eq!(format_hour_label(13, "12h"), "1 pm");
        assert_eq!(format_hour_label(23, "12h"), "11 pm");
    }

    // ── event_position_from ───────────────────────────────────────────────────

    #[test]
    fn event_position_midnight_start() {
        // 00:00–01:00 → top=0%, height=1/24*100≈4.167%
        let (top, height) = event_position_from("2026-05-29 00:00", "2026-05-29 01:00");
        assert!((top - 0.0).abs() < 0.01);
        assert!((height - 100.0 / 24.0).abs() < 0.01);
    }

    #[test]
    fn event_position_minimum_15min_height() {
        // Start == end → enforced minimum of 15 minutes
        let (_, height) = event_position_from("2026-05-29 09:00", "2026-05-29 09:00");
        assert!((height - 15.0 / 1440.0 * 100.0).abs() < 0.01);
    }

    #[test]
    fn event_position_invalid_falls_back() {
        // Invalid strings parse as (0,0,0,0,0) and (0,0,0,1,0)
        let (top, height) = event_position_from("bad", "also-bad");
        assert!((top - 0.0).abs() < 0.01);
        assert!(height > 0.0);
    }

    // ── parse_date ───────────────────────────────────────────────────────────

    #[test]
    fn parse_date_valid() {
        assert_eq!(parse_date("2026-05-29"), Some((2026, 5, 29)));
    }

    #[test]
    fn parse_date_too_short() {
        assert_eq!(parse_date("2026-05"), None);
        assert_eq!(parse_date(""), None);
    }

    #[test]
    fn parse_date_ignores_trailing() {
        // Extra chars after position 10 are ignored
        assert_eq!(parse_date("2026-05-29 extra"), Some((2026, 5, 29)));
    }

    // ── bump_end_date ─────────────────────────────────────────────────────────

    #[test]
    fn bump_end_date_shifts_by_the_same_delta() {
        // Start moved forward 3 days → end (originally 1 day after start)
        // moves forward 3 days too, preserving the 1-day span.
        assert_eq!(
            bump_end_date("2026-05-01", "2026-05-04", "2026-05-02"),
            "2026-05-05"
        );
    }

    #[test]
    fn bump_end_date_shifts_backward() {
        assert_eq!(
            bump_end_date("2026-05-10", "2026-05-08", "2026-05-12"),
            "2026-05-10"
        );
    }

    #[test]
    fn bump_end_date_preserves_time_suffix() {
        assert_eq!(
            bump_end_date("2026-05-01", "2026-05-03", "2026-05-01T14:30"),
            "2026-05-03T14:30"
        );
    }

    #[test]
    fn bump_end_date_crosses_month_boundary() {
        assert_eq!(
            bump_end_date("2026-05-30", "2026-06-02", "2026-05-31"),
            "2026-06-03"
        );
    }

    #[test]
    fn bump_end_date_invalid_start_leaves_end_unchanged() {
        assert_eq!(
            bump_end_date("not-a-date", "2026-05-04", "2026-05-02"),
            "2026-05-02"
        );
    }

    #[test]
    fn bump_end_date_invalid_end_leaves_it_unchanged() {
        assert_eq!(
            bump_end_date("2026-05-01", "2026-05-04", "garbage"),
            "garbage"
        );
    }

    // ── bump_end_time ─────────────────────────────────────────────────────────

    #[test]
    fn bump_end_time_shifts_forward() {
        // 7:00 → 7:15 start (a +15m move) shifts a 7:15 end to 7:30.
        assert_eq!(
            bump_end_time("2026-05-01T07:00", "2026-05-01T07:15", "2026-05-01T07:15"),
            "2026-05-01T07:30"
        );
    }

    #[test]
    fn bump_end_time_shifts_backward() {
        assert_eq!(
            bump_end_time("2026-05-01T20:00", "2026-05-01T19:00", "2026-05-01T20:30"),
            "2026-05-01T19:30"
        );
    }

    #[test]
    fn bump_end_time_crosses_midnight_forward() {
        // Start moves from 23:50 to 00:10 the next day (+20m) — end shifts
        // by the same 20 minutes, crossing midnight itself if needed.
        assert_eq!(
            bump_end_time("2026-05-01T23:50", "2026-05-02T00:10", "2026-05-01T23:55"),
            "2026-05-02T00:15"
        );
    }

    #[test]
    fn bump_end_time_date_only_shift_matches_bump_end_date() {
        // A pure date shift (same time-of-day) behaves like bump_end_date.
        assert_eq!(
            bump_end_time("2026-05-01T09:00", "2026-05-04T09:00", "2026-05-02T10:00"),
            "2026-05-05T10:00"
        );
    }

    #[test]
    fn bump_end_time_invalid_start_leaves_end_unchanged() {
        assert_eq!(
            bump_end_time("garbage", "2026-05-04T09:00", "2026-05-02T10:00"),
            "2026-05-02T10:00"
        );
    }

    #[test]
    fn bump_end_time_invalid_end_leaves_it_unchanged() {
        assert_eq!(
            bump_end_time("2026-05-01T09:00", "2026-05-04T09:00", "garbage"),
            "garbage"
        );
    }

    // ── visible_range ─────────────────────────────────────────────────────────

    #[test]
    fn visible_range_day_mode_ignores_mobile_day_month() {
        // Day mode is just the single mobile_day, regardless of mini_month.
        let (start, end) = visible_range(
            (2026, 5),
            (2026, 5, 25),
            super::super::CalMode::Day,
            (2026, 5, 29),
        );
        assert_eq!(start, (2026, 5, 29));
        assert_eq!(end, (2026, 5, 29));
    }

    #[test]
    fn visible_range_workweek_mode() {
        // week_monday=2026-05-25, WorkWeek span is Mon..Fri (2026-05-25..05-29).
        let (start, end) = visible_range(
            (2026, 5),
            (2026, 5, 25),
            super::super::CalMode::WorkWeek,
            (2026, 5, 29),
        );
        assert_eq!(start, (2026, 5, 25));
        assert_eq!(end, (2026, 5, 29));
    }

    #[test]
    fn visible_range_week_mode() {
        // Week span is Mon..Sun (2026-05-25..05-31).
        let (start, end) = visible_range(
            (2026, 5),
            (2026, 5, 25),
            super::super::CalMode::Week,
            (2026, 5, 29),
        );
        assert_eq!(start, (2026, 5, 25));
        assert_eq!(end, (2026, 5, 31));
    }

    #[test]
    fn visible_range_month_mode_matches_mini_cal_grid() {
        // Month mode is driven by mini_month directly (see visible_range's
        // doc comment: WeekGrid's do_prev/do_next update mini_month, not
        // week_monday, when cal_mode is Month), so it's exactly
        // mini_cal_range(mini_month).
        let (start, end) = visible_range(
            (2026, 5),
            (2026, 7, 6),
            super::super::CalMode::Month,
            (2026, 5, 29),
        );
        assert_eq!(start, (2026, 4, 27));
        assert_eq!(end, (2026, 6, 7));
    }

    /// Regression test: paging the mini-cal's own prev/next buttons (which
    /// move only `mini_month`, not `week_monday`) must not widen the main
    /// grid's fetched range at all in Week/Day/WorkWeek modes — that was the
    /// bug behind `get_events`' response payload growing on every mini-cal
    /// click regardless of the actual visible week. `mini_cal_range` is what
    /// the mini-cal now fetches independently for its own display.
    #[test]
    fn visible_range_week_mode_ignores_a_far_away_mini_month() {
        let (start, end) = visible_range(
            (2027, 11), // mini-cal paged many months ahead
            (2026, 7, 6),
            super::super::CalMode::Week,
            (2026, 5, 29),
        );
        assert_eq!(start, (2026, 7, 6));
        assert_eq!(end, (2026, 7, 12));
    }

    // ── mini_cal_range ────────────────────────────────────────────────────────

    #[test]
    fn mini_cal_range_covers_a_full_6_row_grid() {
        let (start, end) = mini_cal_range((2026, 5));
        assert_eq!(start, (2026, 4, 27));
        assert_eq!(end, (2026, 6, 7));
    }
}
