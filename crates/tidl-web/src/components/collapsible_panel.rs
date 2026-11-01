//! Shared mobile-collapsible panel: a header button (name + chevron) that
//! toggles visibility of its content below. On desktop the button is hidden
//! and the content always shows (see `.fs-collapse-btn`/`.fs-content` in
//! style.css); on mobile the two render as one bordered `.fs-panel` card.

use leptos::prelude::*;

#[component]
pub fn CollapsiblePanel(
    title: &'static str,
    open: RwSignal<bool>,
    children: Children,
) -> impl IntoView {
    view! {
        <div class="fs-panel">
            <button
                class="fs-collapse-btn"
                on:click=move |_| open.update(|b| *b = !*b)
            >
                {title}
                <span class=move || if open.get() {
                    "fs-chevron fs-chevron--open"
                } else {
                    "fs-chevron"
                }>"›"</span>
            </button>
            <div class=move || if open.get() {
                "fs-content"
            } else {
                "fs-content fs-content--collapsed"
            }>
                {children()}
            </div>
        </div>
    }
}
