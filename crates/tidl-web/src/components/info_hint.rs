//! A small, tap-friendly ⓘ affordance for long form hints (backlog #126).
//!
//! Renders an optional always-visible short hint plus an `ⓘ` button; clicking
//! the button opens a `position:fixed` popover with the full text. A
//! transparent full-viewport click-catcher dismisses it on an outside tap —
//! the same house pattern as `mention_popover.rs`, so touch users get a real
//! disclosure instead of a hover tooltip. Deliberately no hover behaviour.

use leptos::prelude::*;

use crate::util::mouse_event_targets_self;

#[component]
pub fn InfoHint(
    /// Always-visible short hint rendered before the icon.
    #[prop(optional, into)]
    short: Option<String>,
    /// Full text shown in the popover.
    #[prop(into)]
    text: String,
) -> impl IntoView {
    let open = RwSignal::new(false);
    let backdrop_mousedown = RwSignal::new(false);
    // Top-left of the popover, read from the button's rect on open.
    let pos = RwSignal::new((0.0_f64, 0.0_f64));
    let btn_ref: NodeRef<leptos::html::Button> = NodeRef::new();
    let short = StoredValue::new(short);
    let text = StoredValue::new(text);

    let toggle = move |_| {
        if open.get_untracked() {
            open.set(false);
            return;
        }
        #[cfg(feature = "hydrate")]
        if let Some(btn) = btn_ref.get_untracked() {
            let r = btn.get_bounding_client_rect();
            let mut x = r.left();
            let y = r.bottom() + 6.0;
            // Keep a ~280px popover on-screen when the icon sits near the
            // right edge.
            if let Some(win) = web_sys::window() {
                let vw = win
                    .inner_width()
                    .ok()
                    .and_then(|w| w.as_f64())
                    .unwrap_or(0.0);
                x = x.min(vw - 296.0).max(8.0);
            }
            pos.set((x, y));
        }
        open.set(true);
    };

    view! {
        <span class="info-hint">
            {move || short.get_value().map(|s| view! { <span class="info-hint-short">{s}</span> })}
            <button
                type="button"
                class="info-hint-btn"
                aria-label="More information"
                aria-expanded=move || if open.get() { "true" } else { "false" }
                node_ref=btn_ref
                on:click=toggle
            >"ⓘ"</button>
            {move || open.get().then(|| {
                let (x, y) = pos.get();
                view! {
                    <div
                        class="info-hint-backdrop"
                        on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
                        on:click=move |_| if backdrop_mousedown.get() { open.set(false) }
                    >
                        <div
                            class="info-hint-popover"
                            style=format!("left:{x}px;top:{y}px;")
                            on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()
                        >
                            {text.get_value()}
                        </div>
                    </div>
                }
            })}
        </span>
    }
}
