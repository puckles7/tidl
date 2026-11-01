//! Inline delete-with-confirm for a card in a grid (Notes backlog #78,
//! Contacts backlog #79) — clicking the trash icon in the card's header
//! opens a small modal dialog styled like every other dialog in the app
//! (`.dialog-backdrop`/`.dialog`, e.g. `notes::NoteEditorModal`), instead
//! of a native `confirm()` or an in-card affordance.
//!
//! Two earlier designs were tried and rejected:
//! - A single button that toggled its own icon into a checkmark read as
//!   "already done" rather than "click to confirm", and its `:hover` rule
//!   outranked the confirm state's own text color (unreadable while the
//!   cursor was still over the button from the first click).
//! - An inline confirm bar at the bottom of the card let a Cancel click's
//!   propagation reach the card's own "open editor" click handler once the
//!   bar's DOM was removed mid-dispatch (`armed.set(false)` detaches the
//!   bar from the tree synchronously, and browsers don't reliably keep
//!   delivering the bubble phase to a handler on a node removed during
//!   dispatch) — canceling a delete opened the editor instead.
//!
//! A real modal sidesteps both: it's an unambiguous, familiar pattern, and
//! since every click handler inside it (backdrop included) stops
//! propagation, there's no path back to the card's own click handling
//! regardless of DOM mutation timing.

use leptos::prelude::*;

use crate::components::icons::TrashIcon;
use crate::util::mouse_event_targets_self;

#[component]
pub fn DeleteTriggerButton(armed: RwSignal<bool>) -> impl IntoView {
    view! {
        <span
            class="card-delete-btn"
            title="Delete"
            on:pointerdown=|ev: leptos::web_sys::PointerEvent| ev.stop_propagation()
            on:click=move |ev: leptos::ev::MouseEvent| {
                ev.stop_propagation();
                armed.set(true);
            }
        ><TrashIcon size=16 /></span>
    }
}

#[component]
pub fn DeleteConfirmModal(
    armed: RwSignal<bool>,
    /// e.g. "note" / "contact" — used as "Delete this {noun}?", or, when
    /// `count` is more than 1, "Delete all {count} {noun}s?" (a "Clear all"
    /// toolbar action rather than one card's own delete button).
    noun: &'static str,
    /// Called when the modal's own "Delete" button is clicked.
    on_confirm: Callback<()>,
    #[prop(default = 1)] count: usize,
) -> impl IntoView {
    let esc_id = crate::modal_esc::use_escape_close(move || armed.set(false));
    move || {
        armed.get().then(|| {
            let backdrop_mousedown = RwSignal::new(false);
            let title = if count == 1 {
                format!("Delete this {noun}?")
            } else {
                format!("Delete all {count} {noun}s?")
            };
            view! {
                <div class="dialog-backdrop"
                    data-esc-id=esc_id.to_string()
                    on:pointerdown=|ev: leptos::web_sys::PointerEvent| ev.stop_propagation()
                    on:mousedown=move |ev: leptos::web_sys::MouseEvent| {
                        ev.stop_propagation();
                        backdrop_mousedown.set(mouse_event_targets_self(&ev));
                    }
                    on:click=move |ev: leptos::ev::MouseEvent| {
                        ev.stop_propagation();
                        if backdrop_mousedown.get() { armed.set(false); }
                    }
                >
                    <div class="dialog dialog-small" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                        <div class="dialog-header">
                            <h2 class="dialog-title">{title}</h2>
                        </div>
                        <div class="dialog-form">
                            <p class="dialog-hint">"This can't be undone."</p>
                        </div>
                        <div class="dialog-actions">
                            <button
                                class="btn-danger"
                                on:click=move |_| {
                                    armed.set(false);
                                    on_confirm.run(());
                                }
                            >"Delete"</button>
                            <button class="btn-secondary" on:click=move |_| armed.set(false)>"Cancel"</button>
                        </div>
                    </div>
                </div>
            }
        })
    }
}
