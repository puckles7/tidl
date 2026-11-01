//! Fullscreen lightbox for inline note images (backlog #140). Opened by
//! clicking an inline image in a note card (see `render_note_content`'s
//! `data-note-image` wrapper and `NoteCard`'s click delegation). Rendered at
//! the app root so a card's transformed drag wrapper can't clip a
//! `position: fixed` overlay.

use leptos::prelude::*;

use crate::modal_esc::use_escape_close;
use crate::util::mouse_event_targets_self;

#[component]
pub fn ImageLightbox(
    /// The image URL to display, taken from the clicked `data-note-image`.
    src: String,
    on_close: impl Fn() + Send + Sync + 'static,
) -> impl IntoView {
    let on_close =
        StoredValue::new(std::sync::Arc::new(on_close) as std::sync::Arc<dyn Fn() + Send + Sync>);
    let esc_id = use_escape_close(move || on_close.with_value(|f| f()));
    let backdrop_mousedown = RwSignal::new(false);

    view! {
        <div
            class="dialog-backdrop image-lightbox"
            data-esc-id=esc_id.to_string()
            on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
            on:click=move |_| if backdrop_mousedown.get() { on_close.with_value(|f| f()) }
        >
            <button
                class="image-lightbox-close"
                title="Close"
                on:click=move |_| on_close.with_value(|f| f())
            >"✕"</button>
            <img class="image-lightbox-img" src=src.clone() alt="" />
        </div>
    }
}
