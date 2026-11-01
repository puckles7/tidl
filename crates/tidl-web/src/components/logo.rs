//! tidl wordmark — the monoline rounded "tidl" with an amber dot on the i,
//! matching `~/dev/tidl-site/assets/brand/logo.svg` (minus its embedded C2PA
//! metadata). Ink and dot colours come from CSS (`.tidl-mark` in style.css):
//! blue in light mode, white in dark mode, with the amber dot in both.

use leptos::prelude::*;

#[component]
pub fn Logo() -> impl IntoView {
    view! {
        <svg
            class="tidl-mark"
            viewBox="0 0 80 48"
            role="img"
            aria-label="tidl"
        >
            <g
                class="tidl-ink"
                fill="none"
                stroke-width="7"
                stroke-linecap="round"
                stroke-linejoin="round"
            >
                <path d="M10 8V33Q10 42 19 42"/>
                <path d="M3 18H19"/>
                <path d="M30 19V42"/>
                <circle cx="50" cy="30.5" r="11.5"/>
                <path d="M61.5 4V42"/>
                <path d="M74 4V42"/>
            </g>
            <circle class="tidl-dot" cx="30" cy="7" r="4.8"/>
        </svg>
    }
}
