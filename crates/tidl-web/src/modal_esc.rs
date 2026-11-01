//! Shared "Escape closes the topmost modal" behaviour (backlog #123).
//!
//! The app has many modals sharing the `.dialog-backdrop` / `.dialog` chrome
//! but no shared base component, so each surface wires its own close callback.
//! This module lets every one of them opt in with a single call and a
//! `data-esc-id` attribute on its backdrop:
//!
//! ```ignore
//! let esc_id = use_escape_close(move || on_close.get_value()());
//! view! { <div class="dialog-backdrop" data-esc-id=esc_id.to_string()>…</div> }
//! ```
//!
//! A single window-level Escape listener (installed in `app.rs` alongside the
//! Ctrl/Cmd-K handler) then finds the *last visible* backdrop-like surface in
//! the DOM — `.dialog-backdrop`, or the in-app markdown editor's
//! `.md-editor-overlay` — which, because a closed modal's backdrop isn't
//! rendered at all, is necessarily the topmost open one — reads its
//! `data-esc-id`, and runs the registered close callback. That reuses each
//! modal's existing close path (the same one its ✕/Cancel/backdrop click uses)
//! instead of synthesising events.
//!
//! Nested choosers that already consume Escape (the note editor's `@mention`
//! popup, the file finder's own handler) call `prevent_default()`; the global
//! listener skips already-consumed events, so the first Escape dismisses the
//! chooser and the second closes the modal.

use leptos::prelude::*;

#[derive(Clone, Copy)]
pub struct ModalEsc {
    entries: StoredValue<Vec<(u64, Callback<()>)>>,
    next_id: StoredValue<u64>,
}

/// Provides the shared modal-Escape registry at the app root.
pub fn provide_modal_esc() -> ModalEsc {
    let ctx = ModalEsc {
        entries: StoredValue::new(Vec::new()),
        next_id: StoredValue::new(0),
    };
    provide_context(ctx);
    ctx
}

/// Registers `on_close` for the calling component's lifetime and returns an id
/// to place on that modal's `.dialog-backdrop` as `data-esc-id`.
///
/// Call this at component setup (not inside a reactive closure); cleanup is
/// tied to the component. For a modal whose backdrop is only rendered while
/// open (the usual `visible.get().then(|| …)` shape), registering
/// unconditionally is correct: the backdrop only enters the DOM when shown, so
/// the id is only ever selected then.
pub fn use_escape_close(on_close: impl Fn() + Send + Sync + 'static) -> u64 {
    let ctx = expect_context::<ModalEsc>();
    ctx.register(Callback::new(move |_: ()| on_close()))
}

#[allow(dead_code)]
impl ModalEsc {
    fn register(&self, close: Callback<()>) -> u64 {
        let id = self
            .next_id
            .try_update_value(|n| {
                let id = *n;
                *n += 1;
                id
            })
            .unwrap_or_default();
        self.entries.update_value(|v| v.push((id, close)));
        let this = *self;
        on_cleanup(move || this.entries.update_value(|v| v.retain(|(i, _)| *i != id)));
        id
    }

    fn close(&self, id: u64) -> bool {
        let close = self
            .entries
            .with_value(|v| v.iter().find(|(i, _)| *i == id).map(|(_, cb)| *cb));
        match close {
            Some(cb) => {
                cb.run(());
                true
            }
            None => false,
        }
    }

    #[cfg(target_arch = "wasm32")]
    pub(crate) fn close_topmost(&self) -> bool {
        match topmost_backdrop_id() {
            Some(id) => self.close(id),
            None => false,
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn close_topmost(&self) -> bool {
        false
    }
}

/// The `data-esc-id` of the last visible backdrop-like surface in document
/// order. Backdrops nested inside a modal (sub-dialogs) carry no id of their
/// own, so the enclosing modal's id is used and its callback decides what to
/// close first. The in-app markdown editor (`.md-editor-overlay`) also opts in
/// (backlog #34): it renders *inside* the preview modal's `.dialog-backdrop`,
/// and because document order puts that ancestor backdrop first, the editor's
/// id wins while it's open — so Escape closes the editor, not the preview.
#[cfg(target_arch = "wasm32")]
fn topmost_backdrop_id() -> Option<u64> {
    use wasm_bindgen::JsCast as _;

    let doc = web_sys::window()?.document()?;
    let nodes = doc.query_selector_all(".dialog-backdrop, .md-editor-overlay").ok()?;
    let mut found = None;
    for i in 0..nodes.length() {
        let Some(el) = nodes
            .item(i)
            .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        let visible = el.client_width() > 0 || el.client_height() > 0;
        if !visible {
            continue;
        }
        if let Some(id) = el
            .get_attribute("data-esc-id")
            .and_then(|s| s.parse::<u64>().ok())
        {
            found = Some(id);
        }
    }
    found
}
