//! Shared header pieces reused by the Calendar and Focus top-level views.
//!
//! Both screens put a single `.screen-toolbar` row at the top (see that class
//! in `assets/style.css` containing their controls, with the search box last.
//! Focus follows it with a one-line `SectionHeader` describing the list.

use chrono::NaiveDate;
use leptos::prelude::*;

/// Shared one-line list summary under Focus's toolbar, e.g.
/// `"All items · Ranked by urgency · 5 remaining"`. The whole line uses the
/// same normal (non-uppercase) font as the count, rather than the old
/// all-caps label — see backlog #115/#119.
#[component]
pub fn SectionHeader(
    #[prop(into)] label: Signal<String>,
    #[prop(into)] count: Signal<usize>,
) -> impl IntoView {
    view! {
        <p class="focus-result-line">
            {move || format!("{} · Ranked by urgency · {} remaining", label.get(), count.get())}
        </p>
    }
}

/// Format a date as "Friday, 15 May 2026".
pub(crate) fn format_today_heading(today: NaiveDate) -> String {
    use chrono::Datelike;
    const WEEKDAYS: [&str; 7] = [
        "Monday",
        "Tuesday",
        "Wednesday",
        "Thursday",
        "Friday",
        "Saturday",
        "Sunday",
    ];
    const MONTHS: [&str; 12] = [
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
    let wd = today.weekday().num_days_from_monday() as usize;
    let mo = (today.month0()) as usize;
    format!(
        "{}, {} {} {}",
        WEEKDAYS[wd],
        today.day(),
        MONTHS[mo],
        today.year()
    )
}

/// Live HH:MM:SS clock in the user's configured timezone (from
/// `UserPrefsCtx`), ticking every second in the browser. Empty string on SSR
/// and before hydration completes.
///
/// Uses `Intl.DateTimeFormat` rather than the client's raw local clock so
/// the displayed time is correct even when the browser spoofs its local
/// timezone (e.g. LibreWolf with resistFingerprinting=true).
pub(crate) fn use_live_clock() -> RwSignal<String> {
    let clock_time: RwSignal<String> = RwSignal::new(String::new());
    #[cfg(feature = "hydrate")]
    {
        use crate::components::settings::UserPrefsCtx;
        use leptos::wasm_bindgen::prelude::*;

        let prefs_ctx = use_context::<UserPrefsCtx>();

        // Read timezone from the signal on every tick so the clock responds
        // immediately when preferences load (async) or the user changes their
        // timezone setting — and so we never capture the stale "UTC" default.
        let time_str = move || {
            let tz = prefs_ctx
                .map(|p| p.get_untracked().timezone)
                .unwrap_or_else(|| "UTC".to_string());
            let code = format!(
                "new Intl.DateTimeFormat('en-GB',\
                 {{timeZone:'{}',hour:'2-digit',minute:'2-digit',\
                   second:'2-digit',hour12:false}}).format(new Date())",
                tz.replace('\'', "\\'")
            );
            js_sys::eval(&code)
                .ok()
                .and_then(|v| v.as_string())
                .unwrap_or_default()
        };
        clock_time.set(time_str());
        let tick = Closure::<dyn FnMut()>::new(move || clock_time.set(time_str()));
        if let Some(win) = leptos::web_sys::window() {
            let _ = win.set_interval_with_callback_and_timeout_and_arguments_0(
                tick.as_ref().unchecked_ref(),
                1_000,
            );
        }
        tick.forget();
    }
    clock_time
}

/// Combine a date heading with the live clock: "Friday, 15 May 2026  ·  14:03:21",
/// falling back to just the date while the clock is empty (SSR/pre-hydration).
pub(crate) fn heading_with_clock(heading: &str, clock_time: RwSignal<String>) -> String {
    let t = clock_time.get();
    if t.is_empty() {
        heading.to_string()
    } else {
        format!("{}  ·  {}", heading, t)
    }
}

#[cfg(test)]
mod tests {
    use super::{format_today_heading, NaiveDate};

    fn date(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn format_today_heading_known_date() {
        assert_eq!(
            format_today_heading(date("2026-05-16")),
            "Saturday, 16 May 2026"
        );
        assert_eq!(
            format_today_heading(date("2026-01-01")),
            "Thursday, 1 January 2026"
        );
    }
}
