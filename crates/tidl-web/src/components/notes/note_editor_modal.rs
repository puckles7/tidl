//! Note create/edit modal — shared between "+ Take a note…" and click-to-edit.

use leptos::prelude::*;
use std::collections::BTreeSet;
use std::sync::Arc;

use super::collection_editor_modal::CollectionEditorModal;
use super::consolidate_modal::ConsolidateNoteModal;
use crate::api::{
    delete_note, get_calendar_shares, get_calendars, list_contacts, list_note_collections,
    list_note_labels, save_note, CalendarInfo, ContactDto, NewNote, NewNoteImage,
    NoteCollectionInfo, NoteInfo, NoteLabelInfo, ShareInfo,
};
use crate::components::info_hint::InfoHint;
use crate::components::priority_slider::PrioritySlider;
use crate::types::note_image_token;
use crate::util::{format_server_fn_error, mouse_event_targets_self};

// ── `@mention` typeahead (backlog #80) ────────────────────────────────────────

/// The in-progress `@mention` being typed, if any — drives both the popup's
/// visibility/position and, on selection, which span of `text_val` to
/// replace.
#[derive(Clone)]
struct MentionState {
    query: String,
    /// Byte offset of the triggering `@` within `text_val`'s current value.
    /// Safe to use as a slice index later only because it (and `query`)
    /// were derived from the exact same string that's still in `text_val`
    /// when a selection commits — see `select_mention`'s guard.
    start: usize,
    /// Viewport coordinates of the caret (`position: fixed` anchor).
    x: f64,
    y: f64,
}

/// Mirror-div technique for caret screen coordinates (a plain `<textarea>`
/// has no API for this): builds an offscreen, invisibility-styled div that
/// copies the textarea's font/box metrics, fills it with the text up to the
/// caret plus a marker span, and reads the marker's `getBoundingClientRect()`
/// — see backlog #80's Context/Background for why this is needed (Slack/
/// GitHub-style caret-anchored popup, not a fixed position below the
/// textarea).
#[cfg(feature = "hydrate")]
fn caret_client_coords(
    textarea: &web_sys::HtmlTextAreaElement,
    caret_utf16: u32,
) -> Option<(f64, f64)> {
    use wasm_bindgen::JsCast;

    let window = web_sys::window()?;
    let document = window.document()?;
    let body = document.body()?;

    let mirror: web_sys::HtmlElement = document.create_element("div").ok()?.dyn_into().ok()?;
    let computed = window.get_computed_style(textarea).ok()??;
    let style = mirror.style();
    for prop in [
        // CSSStyleDeclaration's getPropertyValue()/setProperty() take CSS
        // syntax (hyphen-case) property names, not the JS IDL camelCase
        // names — passing e.g. "whiteSpace" silently matches nothing, so
        // every property in this list must stay hyphen-case or the mirror
        // falls back to the browser's default `<div>` styling (notably
        // `white-space: normal`, which collapses the textarea's newlines
        // and breaks caret positioning on any wrapped/multi-line note).
        "font-family",
        "font-size",
        "font-weight",
        "font-style",
        "line-height",
        "letter-spacing",
        "padding-top",
        "padding-right",
        "padding-bottom",
        "padding-left",
        "border-top-width",
        "border-right-width",
        "border-bottom-width",
        "border-left-width",
        "box-sizing",
        "word-wrap",
        "white-space",
        "word-break",
    ] {
        if let Ok(v) = computed.get_property_value(prop) {
            let _ = style.set_property(prop, &v);
        }
    }
    let _ = style.set_property("position", "absolute");
    let _ = style.set_property("visibility", "hidden");
    let _ = style.set_property("top", "0");
    let _ = style.set_property("left", "-9999px");
    let _ = style.set_property("width", &format!("{}px", textarea.client_width()));

    body.append_child(&mirror).ok()?;

    let utf16: Vec<u16> = textarea.value().encode_utf16().collect();
    let caret = (caret_utf16 as usize).min(utf16.len());
    let before = String::from_utf16_lossy(&utf16[..caret]);
    let after = String::from_utf16_lossy(&utf16[caret..]);

    let _ = mirror.append_child(&document.create_text_node(&before));
    let marker = document.create_element("span").ok()?;
    marker.set_text_content(Some("\u{200b}"));
    let _ = mirror.append_child(&marker);
    let _ = mirror.append_child(&document.create_text_node(&after));

    let marker_rect = marker.get_bounding_client_rect();
    let mirror_rect = mirror.get_bounding_client_rect();
    let ta_rect = textarea.get_bounding_client_rect();

    let x =
        ta_rect.left() + (marker_rect.left() - mirror_rect.left()) - textarea.scroll_left() as f64;
    let y =
        ta_rect.top() + (marker_rect.bottom() - mirror_rect.top()) - textarea.scroll_top() as f64;

    let _ = body.remove_child(&mirror);
    Some((x, y))
}

/// Recomputes `mention_state` from the textarea's current value + caret —
/// called on every `input`/`click` so the popup opens, updates, or closes
/// in step with typing and caret movement. No-ops (closes the popup) when
/// there's an active selection, since "typing a mention" and "selecting
/// text" are mutually exclusive states for this feature's purposes.
#[cfg(feature = "hydrate")]
fn update_mention_state(
    textarea: &web_sys::HtmlTextAreaElement,
    mention_state: RwSignal<Option<MentionState>>,
    mention_index: RwSignal<usize>,
) {
    let (Ok(Some(start)), Ok(Some(end))) = (textarea.selection_start(), textarea.selection_end())
    else {
        mention_state.set(None);
        return;
    };
    if start != end {
        mention_state.set(None);
        return;
    }

    let utf16: Vec<u16> = textarea.value().encode_utf16().collect();
    let caret = (start as usize).min(utf16.len());
    let before = String::from_utf16_lossy(&utf16[..caret]);

    if crate::util::caret_in_note_image_token(&textarea.value(), before.len()) {
        mention_state.set(None);
        return;
    }

    let Some((at_idx, query)) = crate::util::find_active_mention(&before) else {
        mention_state.set(None);
        return;
    };
    let Some((x, y)) = caret_client_coords(textarea, start) else {
        mention_state.set(None);
        return;
    };

    mention_state.set(Some(MentionState {
        query: query.to_string(),
        start: at_idx,
        x,
        y,
    }));
    mention_index.set(0);
}

/// `#` token chooser trigger (backlog #113), parallel to `update_mention_state`.
/// Closed while a mention popup is active (the two never overlap), and while
/// the caret sits inside an `@img` token.
#[cfg(feature = "hydrate")]
fn update_hash_state(
    textarea: &web_sys::HtmlTextAreaElement,
    mention_state: RwSignal<Option<MentionState>>,
    hash_state: RwSignal<Option<MentionState>>,
) {
    if mention_state.get_untracked().is_some() {
        hash_state.set(None);
        return;
    }
    let (Ok(Some(start)), Ok(Some(end))) = (textarea.selection_start(), textarea.selection_end())
    else {
        hash_state.set(None);
        return;
    };
    if start != end {
        hash_state.set(None);
        return;
    }

    let utf16: Vec<u16> = textarea.value().encode_utf16().collect();
    let caret = (start as usize).min(utf16.len());
    let before = String::from_utf16_lossy(&utf16[..caret]);

    if crate::util::caret_in_note_image_token(&textarea.value(), before.len()) {
        hash_state.set(None);
        return;
    }
    // A caret inside an existing token must not re-open the create chooser
    // (backlog #129): otherwise `open_task_modal` deletes the token's keyword
    // prefix and `submit_task` appends a new token over the top of it. Clicking
    // such a token opens its edit dialog instead (the textarea's `on:click`).
    if crate::note_tokens::token_at_caret(&textarea.value(), before.len()).is_some() {
        hash_state.set(None);
        return;
    }
    let Some((hash_idx, partial)) = crate::util::find_active_hash(&before) else {
        hash_state.set(None);
        return;
    };
    let Some((x, y)) = caret_client_coords(textarea, start) else {
        hash_state.set(None);
        return;
    };

    hash_state.set(Some(MentionState {
        query: partial.to_string(),
        start: hash_idx,
        x,
        y,
    }));
}

/// Byte offset of the textarea's caret (collapsed selection), or `None` when
/// there's an active selection or no DOM yet. Converts the browser's UTF-16
/// selection units to a byte offset safe to slice a Rust `String` at.
#[cfg(feature = "hydrate")]
fn caret_byte_offset(ta: &web_sys::HtmlTextAreaElement) -> Option<usize> {
    let (Ok(Some(start)), Ok(Some(end))) = (ta.selection_start(), ta.selection_end()) else {
        return None;
    };
    if start != end {
        return None;
    }
    let utf16: Vec<u16> = ta.value().encode_utf16().collect();
    let caret = (start as usize).min(utf16.len());
    Some(String::from_utf16_lossy(&utf16[..caret]).len())
}

/// Inserts `insertion` at the caret (or appends when there's no usable caret)
/// and restores focus/caret just after it, deferred one tick so the queued
/// value update lands first — same technique as `select_mention` below.
#[cfg(feature = "hydrate")]
fn insert_at_caret(ta: web_sys::HtmlTextAreaElement, text_val: RwSignal<String>, insertion: &str) {
    use wasm_bindgen::prelude::*;

    let current = text_val.get_untracked();
    let start = caret_byte_offset(&ta).unwrap_or(current.len());
    let mut new_text = String::with_capacity(current.len() + insertion.len());
    new_text.push_str(&current[..start]);
    new_text.push_str(insertion);
    new_text.push_str(&current[start..]);
    let caret_byte = start + insertion.len();
    text_val.set(new_text.clone());

    let caret_utf16 = new_text[..caret_byte].encode_utf16().count() as u32;
    let closure = Closure::once(move || {
        let _ = ta.focus();
        let _ = ta.set_selection_range(caret_utf16, caret_utf16);
    });
    if let Some(win) = web_sys::window() {
        let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
            closure.as_ref().unchecked_ref(),
            0,
        );
    }
    closure.forget();
}

// ── LastUsedCollectionCtx (defined here, exported for NotesView to provide) ──
// Newtype, not a bare `RwSignal<Option<i64>>` alias — see the project's
// context-shadowing rule (CLAUDE.md §15 / `LastUsedCalendarCtx`'s own doc
// comment in `calendar/event_form.rs`): Leptos keys context on `TypeId`, so
// a plain alias would collide with any other context sharing the same inner
// type.
#[derive(Clone, Copy)]
pub struct LastUsedCollectionCtx(pub RwSignal<Option<i64>>);

#[component]
pub fn NoteEditorModal(
    /// `None` to create (via "+ Take a note…"); `Some(note)` to edit.
    note: Option<NoteInfo>,
    /// Collections the user has access to, for the collection chip row.
    collections: Vec<NoteCollectionInfo>,
    /// Pre-selected collection when creating from a specific collection context.
    default_collection_id: Option<i64>,
    on_close: impl Fn() + Send + Sync + 'static,
    on_refresh: Callback<()>,
    /// When set, renders a "Show in collection" button that navigates to the
    /// Notes tab and highlights this note there. Only passed by
    /// `NoteGotoModal` (backlog #63) — the Notes tab's own usage of this
    /// modal leaves it `None` since you're already looking at the note.
    #[prop(optional)]
    on_show_in_collection: Option<Callback<()>>,
) -> impl IntoView {
    let on_close = StoredValue::new(Arc::new(on_close) as Arc<dyn Fn() + Send + Sync>);

    let is_edit = note.is_some();
    let note_id = StoredValue::new(note.as_ref().map(|n| n.id));
    // Owned snapshot of the note for the "Consolidate to Markdown" modal
    // (backlog #122) — only reachable in edit mode, when the note is saved.
    let note_for_consolidate = StoredValue::new(note.clone());
    let show_consolidate = RwSignal::new(false);

    let last_used_coll_ctx = use_context::<LastUsedCollectionCtx>();

    let title_val = RwSignal::new(
        note.as_ref()
            .and_then(|n| n.title.clone())
            .unwrap_or_default(),
    );
    let text_val = RwSignal::new(note.as_ref().map(|n| n.text.clone()).unwrap_or_default());

    // ── Labels (backlog #114) ─────────────────────────────────────────────
    // The note carries label *names*; new ones typed here are created on save.
    // `all_labels_res` supplies the caller's existing labels for the suggestion
    // chips and the `datalist`.
    let labels_val: RwSignal<Vec<String>> = RwSignal::new(
        note.as_ref()
            .map(|n| n.labels.iter().map(|l| l.name.clone()).collect())
            .unwrap_or_default(),
    );
    let label_input = RwSignal::new(String::new());
    let all_labels_res: LocalResource<Vec<NoteLabelInfo>> =
        LocalResource::new(|| async move { list_note_labels().await.unwrap_or_default() });

    // ── `@mention` typeahead (backlog #80) ────────────────────────────────
    let textarea_ref: NodeRef<leptos::html::Textarea> = NodeRef::new();
    let contacts_res =
        LocalResource::new(|| async move { list_contacts().await.unwrap_or_default() });
    let mention_state: RwSignal<Option<MentionState>> = RwSignal::new(None);
    let mention_index = RwSignal::new(0usize);
    // Plain reactive `.get()`, not `Suspend::new` — a slow contacts fetch
    // must never suspend the whole editor dialog just because the user
    // typed `@` (leptos-suspense skill).
    let filtered_contacts = Signal::derive(move || {
        let Some(state) = mention_state.get() else {
            return Vec::new();
        };
        crate::util::filter_contacts_for_mention(
            &contacts_res.get().unwrap_or_default(),
            &state.query,
        )
    });

    // ── Inline images (backlog #110) ──────────────────────────────────────
    // A picked file is staged client-side (base64 + MIME) and referenced in
    // the text by a `@img:t<key>` placeholder; `save_note` inserts the rows
    // and rewrites those placeholders to real `@img:<id>` tokens in one
    // transaction. Existing images arrive with the note.
    let existing_images: Vec<crate::api::NoteImageInfo> =
        note.as_ref().map(|n| n.images.clone()).unwrap_or_default();
    let existing_images = StoredValue::new(existing_images);
    let staged_images: RwSignal<Vec<NewNoteImage>> = RwSignal::new(Vec::new());
    #[cfg(feature = "hydrate")]
    let image_key_seq = RwSignal::new(0u32);
    let image_error = RwSignal::<Option<String>>::new(None);

    let on_image_pick = move |ev: leptos::ev::Event| {
        #[cfg(feature = "hydrate")]
        {
            use wasm_bindgen::JsCast as _;
            let Some(input) = ev
                .target()
                .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
            else {
                return;
            };
            let Some(files) = input.files() else { return };
            let ta_opt = textarea_ref.get_untracked();
            for i in 0..files.length() {
                let Some(file) = files.get(i) else { continue };
                let content_type = file.type_();
                if !matches!(
                    content_type.as_str(),
                    "image/jpeg" | "image/png" | "image/gif" | "image/webp"
                ) {
                    image_error.set(Some(
                        "Only JPEG, PNG, GIF or WebP images are supported.".into(),
                    ));
                    continue;
                }
                if file.size() > 5.0 * 1024.0 * 1024.0 {
                    image_error.set(Some("Image is too large (max 5 MiB).".into()));
                    continue;
                }
                image_error.set(None);
                let temp_key = image_key_seq.get_untracked().to_string();
                image_key_seq.update(|n| *n += 1);
                let filename = file.name();
                let ta_opt = ta_opt.clone();
                leptos::task::spawn_local(async move {
                    use wasm_bindgen_futures::JsFuture;
                    match JsFuture::from(file.array_buffer()).await {
                        Ok(buf) => {
                            let bytes = js_sys::Uint8Array::new(&buf).to_vec();
                            use base64::Engine;
                            let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                            staged_images.update(|v| {
                                v.push(NewNoteImage {
                                    temp_key: temp_key.clone(),
                                    base64_data: b64,
                                    content_type,
                                    filename: Some(filename),
                                })
                            });
                            let token = format!(" @img:t{temp_key} ");
                            if let Some(ta) = ta_opt {
                                insert_at_caret(ta, text_val, &token);
                            } else {
                                text_val.update(|t| t.push_str(&token));
                            }
                        }
                        Err(_) => image_error.set(Some("Could not read image.".into())),
                    }
                });
            }
            // Reset so picking the same file again still fires `change`.
            input.set_value("");
        }
        #[cfg(not(feature = "hydrate"))]
        let _ = ev;
    };

    // Removes an image's token from the text (which is what actually detaches
    // it — `save_note` prunes any row whose token is gone). Staged entries are
    // also dropped from `staged_images` so they aren't uploaded at all.
    let remove_image = move |token: String, temp_key: Option<String>| {
        text_val.update(|t| {
            *t = t.replace(&token, " ");
        });
        if let Some(key) = temp_key {
            staged_images.update(|v| v.retain(|img| img.temp_key != key));
        }
    };

    // Replaces the `@query` span in `text_val` with `@{display_name} ` and
    // restores focus/caret right after it. Shared by both Enter-to-select
    // and click-to-select below.
    let select_mention = move |contact: ContactDto| {
        let Some(state) = mention_state.get_untracked() else {
            return;
        };
        let current = text_val.get_untracked();
        let end = state.start + 1 + state.query.len();
        if state.start > current.len() || end > current.len() {
            // Text changed under us in some unexpected way — bail safely
            // rather than risk slicing at a non-char-boundary.
            mention_state.set(None);
            return;
        }
        let insertion = format!("@{} ", contact.display_name);
        #[cfg_attr(not(feature = "hydrate"), allow(unused_variables))]
        let new_caret_byte = state.start + insertion.len();
        let mut new_text = String::with_capacity(current.len() + insertion.len());
        new_text.push_str(&current[..state.start]);
        new_text.push_str(&insertion);
        new_text.push_str(&current[end..]);
        text_val.set(new_text.clone());
        mention_state.set(None);

        #[cfg(feature = "hydrate")]
        {
            use wasm_bindgen::prelude::*;
            let caret_utf16 = new_text[..new_caret_byte].encode_utf16().count() as u32;
            if let Some(ta) = textarea_ref.get_untracked() {
                // Deferred one tick: `text_val.set` above only queues the
                // DOM's `.value` update, which the browser hasn't applied
                // yet when this closure runs — `set_selection_range` right
                // now would clamp against the *old* value. Same technique
                // (and same reason) as the wikilink pill-flash timer in
                // `focus.rs`.
                let closure = Closure::once(move || {
                    let _ = ta.focus();
                    let _ = ta.set_selection_range(caret_utf16, caret_utf16);
                });
                if let Some(win) = web_sys::window() {
                    let _ = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                        closure.as_ref().unchecked_ref(),
                        0,
                    );
                }
                closure.forget();
            }
        }
    };
    // `None` is a real, representable state here (no collection chosen yet) —
    // not the old `0` sentinel, which never corresponded to an actual row
    // and caused a confusing "Not authorized" error on save whenever the
    // user had no collections yet (backlog #58).
    let collection_val: RwSignal<Option<i64>> = RwSignal::new(
        note.as_ref()
            .map(|n| n.collection_id)
            .or(default_collection_id)
            .or_else(|| {
                last_used_coll_ctx
                    .and_then(|ctx| ctx.0.get_untracked())
                    .filter(|id| collections.iter().any(|c| c.id == *id))
            })
            .or_else(|| collections.first().map(|c| c.id)),
    );
    // Reactive (not a `StoredValue` snapshot) so the chip row can grow when
    // the inline "+ New" flow below creates a collection without closing
    // this modal.
    let collections_state: RwSignal<Vec<NoteCollectionInfo>> = RwSignal::new(collections);
    let show_new_collection = RwSignal::new(false);

    let pinned_val = RwSignal::new(note.as_ref().map(|n| n.pinned).unwrap_or(false));
    // Per-note calendar applied to every inline task/await event (backlog
    // #112) — one note, one calendar, matching the pre-#112 behaviour.
    let calendar_val = RwSignal::new(
        note.as_ref()
            .and_then(|n| n.calendar_name.clone())
            .unwrap_or_else(|| "personal".to_string()),
    );

    // ── Inline task/await authoring (backlog #113) ────────────────────────
    // `task_modal_mode`: `None` closed, `Some("task")` / `Some("await")` open.
    let task_modal_mode: RwSignal<Option<&'static str>> = RwSignal::new(None);
    let task_name_val = RwSignal::new(String::new());
    // `.max(1)` guards a stray legacy 0 (the old 0-100 scale's "highest");
    // the slider's domain is 1-100 (backlog #70).
    let task_priority_val = RwSignal::new(50u32);
    let task_date_val = RwSignal::new(
        (chrono::Utc::now().date_naive() + chrono::Duration::days(7))
            .format("%Y-%m-%d")
            .to_string(),
    );
    // `Some((line, ordinal))` while the task modal is editing an existing token
    // in the body (backlog #129); `None` when creating a new one.
    let task_edit_target: RwSignal<Option<(usize, usize)>> = RwSignal::new(None);
    // Caret-anchored `#` token popup (task/await chooser), parallel to the
    // `@mention` popup.
    let hash_state: RwSignal<Option<MentionState>> = RwSignal::new(None);

    let saving = RwSignal::new(false);
    let error_msg = RwSignal::<Option<String>>::new(None);

    // Opens the task/await modal, consuming any typed `#`-triggered partial
    // (backlog #113). Shared by the chooser popup's buttons.
    let open_task_modal = move |mode: &'static str| {
        if let Some(state) = hash_state.get_untracked() {
            let current = text_val.get_untracked();
            let end = state.start + 1 + state.query.len();
            if end <= current.len() {
                let mut new_text = String::with_capacity(current.len());
                new_text.push_str(&current[..state.start]);
                new_text.push_str(&current[end..]);
                text_val.set(new_text);
            }
        }
        hash_state.set(None);
        task_edit_target.set(None);
        task_name_val.set(String::new());
        task_modal_mode.set(Some(mode));
    };

    // Prefills the task/await modal from an existing body token and marks it as
    // an edit (backlog #129). A free fn (all-Copy signals) so the textarea's
    // click handler and the chip row can both call it.
    fn prefill_task_edit(
        kind: &crate::note_tokens::ScannedTokenKind,
        line: usize,
        ordinal: usize,
        task_modal_mode: RwSignal<Option<&'static str>>,
        task_name_val: RwSignal<String>,
        task_priority_val: RwSignal<u32>,
        task_date_val: RwSignal<String>,
        task_edit_target: RwSignal<Option<(usize, usize)>>,
    ) {
        use crate::note_tokens::ScannedTokenKind;
        match kind {
            ScannedTokenKind::Task { priority, slug } => {
                task_name_val.set(crate::note_tokens::slug_to_title(slug));
                task_priority_val.set(*priority);
                task_edit_target.set(Some((line, ordinal)));
                task_modal_mode.set(Some("task"));
            }
            ScannedTokenKind::Await { date, slug } => {
                task_name_val.set(crate::note_tokens::slug_to_title(slug));
                task_date_val.set(date.format("%Y-%m-%d").to_string());
                task_edit_target.set(Some((line, ordinal)));
                task_modal_mode.set(Some("await"));
            }
            ScannedTokenKind::Done { .. } => {}
        }
    }

    let calendars_res =
        LocalResource::new(|| async move { get_calendars().await.unwrap_or_default() });

    // Refetches whenever the selected calendar changes, to compare its
    // grantees against the note collection's members below (backlog #39).
    let calendar_shares_res = LocalResource::new(move || {
        let cal = calendar_val.get();
        async move { get_calendar_shares(cal).await.unwrap_or_default() }
    });

    let build_input = move |collection_id: i64| -> NewNote {
        let title = title_val.get_untracked();
        NewNote {
            collection_id,
            title: (!title.trim().is_empty()).then_some(title),
            text: text_val.get_untracked(),
            pinned: pinned_val.get_untracked(),
            calendar_name: Some(calendar_val.get_untracked()),
            labels: labels_val.get_untracked(),
        }
    };

    let save = move |_: leptos::ev::MouseEvent| {
        let text = text_val.get_untracked();
        if text.trim().is_empty() {
            error_msg.set(Some("Note text is required.".into()));
            return;
        }
        let Some(cid) = collection_val.get_untracked() else {
            error_msg.set(Some("Please choose or create a collection.".into()));
            return;
        };
        saving.set(true);
        error_msg.set(None);
        let input = build_input(cid);
        // Only send staged images whose token still survives in the text —
        // one whose token the user deleted shouldn't be uploaded at all.
        let staged: Vec<NewNoteImage> = staged_images
            .get_untracked()
            .into_iter()
            .filter(|img| text.contains(&format!("@img:t{}", img.temp_key)))
            .collect();
        let note_id_opt = note_id.get_value();
        let on_close = on_close;
        let on_refresh = on_refresh;

        leptos::task::spawn_local(async move {
            match save_note(note_id_opt, input, staged).await {
                Ok(_) => {
                    saving.set(false);
                    if let Some(ctx) = last_used_coll_ctx {
                        ctx.0.set(Some(cid));
                    }
                    on_close.get_value()();
                    on_refresh.run(());
                }
                Err(e) => {
                    error_msg.set(Some(format_server_fn_error(&e.to_string())));
                    saving.set(false);
                }
            }
        });
    };

    let do_delete = move |_: leptos::ev::MouseEvent| {
        let id = note_id.get_value().unwrap_or_default();
        saving.set(true);
        error_msg.set(None);
        let on_close = on_close;
        let on_refresh = on_refresh;
        leptos::task::spawn_local(async move {
            match delete_note(id).await {
                Ok(()) => {
                    saving.set(false);
                    on_close.get_value()();
                    on_refresh.run(());
                }
                Err(e) => {
                    error_msg.set(Some(format_server_fn_error(&e.to_string())));
                    saving.set(false);
                }
            }
        });
    };

    let title = if is_edit { "Edit note" } else { "New note" };
    let backdrop_mousedown = RwSignal::new(false);
    let esc_id = crate::modal_esc::use_escape_close(move || {
        // Innermost surface first: the `@mention` popup (usually already
        // consumed by the textarea's own handler), then the `#` task/await
        // chooser, then the inline task/await authoring modal (backlog #113),
        // then the editor itself.
        if mention_state.get_untracked().is_some() {
            mention_state.set(None);
        } else if hash_state.get_untracked().is_some() {
            hash_state.set(None);
        } else if task_modal_mode.get_untracked().is_some() {
            task_modal_mode.set(None);
        } else {
            on_close.get_value()();
        }
    });

    // Appends the token built by the task modal to the body (simple and robust
    // versus caret insertion, whose DOM value lags a tick) — or, when editing an
    // existing token (backlog #129), rewrites it in place preserving its kind.
    let submit_task = move |_: leptos::ev::MouseEvent| {
        let Some(mode) = task_modal_mode.get_untracked() else {
            return;
        };
        let slug = crate::note_tokens::slugify(&task_name_val.get_untracked());
        if slug.is_empty() {
            return;
        }
        if let Some((line, ordinal)) = task_edit_target.get_untracked() {
            if mode == "task" {
                let priority = Some(task_priority_val.get_untracked());
                if let Some(new_text) = crate::note_tokens::edit_token_at(
                    &text_val.get_untracked(),
                    line,
                    ordinal,
                    &slug,
                    priority,
                    None,
                ) {
                    text_val.set(new_text);
                }
            } else {
                match chrono::NaiveDate::parse_from_str(&task_date_val.get_untracked(), "%Y-%m-%d")
                {
                    Ok(date) => {
                        if let Some(new_text) = crate::note_tokens::edit_token_at(
                            &text_val.get_untracked(),
                            line,
                            ordinal,
                            &slug,
                            None,
                            Some(date),
                        ) {
                            text_val.set(new_text);
                        }
                    }
                    Err(_) => return,
                }
            }
            task_edit_target.set(None);
            task_modal_mode.set(None);
            task_name_val.set(String::new());
            return;
        }
        let token = if mode == "task" {
            crate::note_tokens::format_task_token(task_priority_val.get_untracked(), &slug)
        } else {
            match chrono::NaiveDate::parse_from_str(&task_date_val.get_untracked(), "%Y-%m-%d") {
                Ok(date) => crate::note_tokens::format_await_token(date, &slug),
                Err(_) => return,
            }
        };
        text_val.update(|t| {
            if !t.is_empty() && !t.ends_with(char::is_whitespace) {
                t.push(' ');
            }
            t.push_str(&token);
            t.push(' ');
        });
        task_modal_mode.set(None);
        task_name_val.set(String::new());
    };

    // Add a label name to `labels_val` (case-insensitively deduped); blank is
    // a no-op. A free function (not a closure) so the same logic can be
    // called from both the Enter handler and the suggestion chips without
    // moving a closure.
    fn add_label_name(labels_val: RwSignal<Vec<String>>, name: &str) {
        let name = name.trim();
        if name.is_empty() {
            return;
        }
        labels_val.update(|v| {
            if !v.iter().any(|x| x.eq_ignore_ascii_case(name)) {
                v.push(name.to_string());
            }
        });
    }

    // Colour a selected label chip from the caller's owned-label list; a label
    // only present on a shared note (not owned) falls back to muted text.
    fn label_color(all: &[NoteLabelInfo], name: &str) -> String {
        all.iter()
            .find(|l| l.name.eq_ignore_ascii_case(name))
            .map(|l| l.color.clone())
            .unwrap_or_else(|| "var(--text-muted, #6b7280)".to_string())
    }

    view! {
        <div class="dialog-backdrop"
            data-esc-id=esc_id.to_string()
            on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
            on:click=move |_| if backdrop_mousedown.get() { on_close.get_value()() }
        >
            <div class="dialog dialog--note-editor" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                <div class="dialog-header">
                    <h2 class="dialog-title">{title}</h2>
                    <div class="dialog-header-actions">
                        <span
                            class=move || if pinned_val.get() { "note-pin note-pin--active" } else { "note-pin" }
                            title=move || if pinned_val.get() { "Unstar note" } else { "Star note" }
                            on:click=move |_| pinned_val.update(|v| *v = !*v)
                        >"★"</span>
                        <button class="dialog-close" on:click=move |_| on_close.get_value()()>"✕"</button>
                    </div>
                </div>

                <div class="dialog-form note-editor-form">
                    <div class="form-group">
                        <input
                            type="text"
                            class="form-input"
                            placeholder="Title (optional)"
                            prop:value=move || title_val.get()
                            on:input=move |ev| title_val.set(event_target_value(&ev))
                            disabled=move || saving.get()
                        />
                    </div>

                    <div class="form-group form-group--grow note-editor-body-group">
                        <textarea
                            class="form-input note-editor-textarea"
                            rows=6
                            placeholder="Take a note…"
                            node_ref=textarea_ref
                            prop:value=move || text_val.get()
                            on:input=move |ev| {
                                text_val.set(event_target_value(&ev));
                                #[cfg(feature = "hydrate")]
                                {
                                    use wasm_bindgen::JsCast as _;
                                    if let Some(ta) = ev
                                        .target()
                                        .and_then(|t| t.dyn_into::<web_sys::HtmlTextAreaElement>().ok())
                                    {
                                        update_mention_state(&ta, mention_state, mention_index);
                                        update_hash_state(&ta, mention_state, hash_state);
                                    }
                                }
                            }
                            on:click=move |_ev: leptos::web_sys::MouseEvent| {
                                #[cfg(feature = "hydrate")]
                                if let Some(ta) = textarea_ref.get_untracked() {
                                    // A click that lands inside an existing token
                                    // opens its edit dialog instead of re-opening
                                    // the `#` chooser (backlog #129).
                                    if let Some(offset) = caret_byte_offset(&ta) {
                                        if let Some((tok, ordinal)) =
                                            crate::note_tokens::token_at_caret_ordinal(&ta.value(), offset)
                                        {
                                            mention_state.set(None);
                                            hash_state.set(None);
                                            prefill_task_edit(
                                                &tok.kind,
                                                tok.line,
                                                ordinal,
                                                task_modal_mode,
                                                task_name_val,
                                                task_priority_val,
                                                task_date_val,
                                                task_edit_target,
                                            );
                                            return;
                                        }
                                    }
                                    update_mention_state(&ta, mention_state, mention_index);
                                    update_hash_state(&ta, mention_state, hash_state);
                                }
                            }
                            on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                                if mention_state.get_untracked().is_none() {
                                    return;
                                }
                                let list = filtered_contacts.get_untracked();
                                match ev.key().as_str() {
                                    "ArrowDown" => {
                                        ev.prevent_default();
                                        if !list.is_empty() {
                                            mention_index.update(|i| *i = (*i + 1) % list.len());
                                        }
                                    }
                                    "ArrowUp" => {
                                        ev.prevent_default();
                                        if !list.is_empty() {
                                            mention_index.update(|i| *i = if *i == 0 { list.len() - 1 } else { *i - 1 });
                                        }
                                    }
                                    "Enter" => {
                                        if let Some(contact) = list.get(mention_index.get_untracked()) {
                                            ev.prevent_default();
                                            select_mention(contact.clone());
                                        }
                                    }
                                    "Escape" => {
                                        ev.prevent_default();
                                        mention_state.set(None);
                                    }
                                    _ => {}
                                }
                            }
                            on:blur=move |_| mention_state.set(None)
                            disabled=move || saving.get()
                        ></textarea>

                        <span class="note-editor-body-hint">
                            <InfoHint text="Type @ to mention a contact, and # for a task or await. Start a line with - for a bullet list. You can also use the + Task / + Await buttons below." />
                        </span>

                        {move || mention_state.get().map(|state| {
                            let x = state.x;
                            let y = state.y;
                            view! {
                                <div class="mention-popup" style=format!("left:{x}px;top:{y}px;")>
                                    {move || {
                                        let list = filtered_contacts.get();
                                        if list.is_empty() {
                                            view! { <div class="mention-popup-empty">"No matching contacts"</div> }.into_any()
                                        } else {
                                            let active = mention_index.get();
                                            list.into_iter().enumerate().map(|(idx, c)| {
                                                let name = c.display_name.clone();
                                                let subtitle = c.org.clone().filter(|o| !o.is_empty());
                                                let for_select = c.clone();
                                                view! {
                                                    <div
                                                        class=if idx == active { "mention-popup-item mention-popup-item--active" } else { "mention-popup-item" }
                                                        on:mousedown=move |ev: leptos::web_sys::MouseEvent| {
                                                            ev.prevent_default();
                                                            select_mention(for_select.clone());
                                                        }
                                                    >
                                                        <span class="mention-popup-item-name">{name}</span>
                                                        {subtitle.map(|s| view! { <span class="mention-popup-item-org">{s}</span> })}
                                                    </div>
                                                }
                                            }).collect_view().into_any()
                                        }
                                    }}
                                </div>
                            }
                        })}
                    </div>

                    {move || hash_state.get().map(|state| {
                        let x = state.x;
                        let y = state.y;
                        view! {
                            <div class="mention-popup" style=format!("left:{x}px;top:{y}px;")>
                                <div
                                    class="mention-popup-item"
                                    on:mousedown=move |ev: leptos::web_sys::MouseEvent| {
                                        ev.prevent_default();
                                        open_task_modal("task");
                                    }
                                ><span class="mention-popup-item-name">"Task…"</span></div>
                                <div
                                    class="mention-popup-item"
                                    on:mousedown=move |ev: leptos::web_sys::MouseEvent| {
                                        ev.prevent_default();
                                        open_task_modal("await");
                                    }
                                ><span class="mention-popup-item-name">"Await…"</span></div>
                            </div>
                        }
                    })}

                    <div class="note-editor-details">
                        <div class="form-group">
                            <label class="form-label">"Images"</label>
                            <input
                                type="file"
                                class="form-input note-image-input"
                                accept="image/jpeg,image/png,image/gif,image/webp"
                                multiple
                                on:change=on_image_pick
                                disabled=move || saving.get()
                            />
                            <div class="note-image-chips">
                                {move || existing_images.with_value(|imgs| {
                                    let text = text_val.get();
                                    imgs.iter().filter_map(|img| {
                                        let token = note_image_token(img.id);
                                        if !text.contains(&token) {
                                            return None;
                                        }
                                        let label = img
                                            .filename
                                            .clone()
                                            .unwrap_or_else(|| format!("image {}", img.id));
                                        let token_for_remove = token.clone();
                                        Some(view! {
                                            <span class="note-image-chip">
                                                <span class="note-image-chip-name">{label}</span>
                                                <button
                                                    type="button"
                                                    class="note-image-chip-remove"
                                                    title="Remove image"
                                                    on:click=move |_| remove_image(token_for_remove.clone(), None)
                                                >"✕"</button>
                                            </span>
                                        })
                                    }).collect_view()
                                })}
                                {move || staged_images.get().into_iter().filter_map(|img| {
                                    let token = format!("@img:t{}", img.temp_key);
                                    if !text_val.get().contains(&token) {
                                        return None;
                                    }
                                    let label = img
                                        .filename
                                        .clone()
                                        .unwrap_or_else(|| "image".to_string());
                                    let temp_key = img.temp_key.clone();
                                    let token_for_remove = token.clone();
                                    Some(view! {
                                        <span class="note-image-chip note-image-chip--staged">
                                            <span class="note-image-chip-name">{label}</span>
                                            <button
                                                type="button"
                                                class="note-image-chip-remove"
                                                title="Remove image"
                                                on:click=move |_| remove_image(token_for_remove.clone(), Some(temp_key.clone()))
                                            >"✕"</button>
                                        </span>
                                    })
                                }).collect_view()}
                            </div>
                            {move || image_error.get().map(|e| view! { <p class="dialog-error">{e}</p> })}
                        </div>

                        <div class="form-group">
                            <label class="form-label">"Collection"</label>
                            <div class="note-chip-row">
                                {move || collections_state.get().into_iter().map(|c| {
                                    let id = c.id;
                                    let color = c.color.clone();
                                    let color2 = color.clone();
                                    let name = c.name.clone();
                                    view! {
                                        <button
                                            type="button"
                                            class="note-chip"
                                            style=move || if collection_val.get() == Some(id) {
                                                format!("background:{color}1a;border-color:{color}55;color:{color};")
                                            } else {
                                                String::new()
                                            }
                                            on:click=move |_| collection_val.set(Some(id))
                                        >
                                            <span class="note-chip-dot" style=format!("background:{color2};")></span>
                                            {name}
                                        </button>
                                    }
                                }).collect_view()}
                                <button
                                    type="button"
                                    class="note-chip note-chip--new"
                                    disabled=move || saving.get()
                                    on:click=move |_| show_new_collection.set(true)
                                >"+ New"</button>
                            </div>
                        </div>

                        <div class="form-group">
                            <label class="form-label">"Labels"</label>
                            <div class="note-chip-row">
                                {move || labels_val.get().into_iter().map(|name| {
                                    let color = label_color(&all_labels_res.get().unwrap_or_default(), &name);
                                    let name_rm = name.clone();
                                    view! {
                                        <span
                                            class="note-chip"
                                            style=format!("background:{color}1a;border-color:{color}55;color:{color};")
                                        >
                                            <span class="note-chip-dot" style=format!("background:{color};")></span>
                                            {name}
                                            <button
                                                type="button"
                                                class="note-chip-remove"
                                                title="Remove label"
                                                on:click=move |_| labels_val.update(|v| v.retain(|x| x != &name_rm))
                                            >"✕"</button>
                                        </span>
                                    }
                                }).collect_view()}
                                {move || {
                                    let selected = labels_val.get();
                                    all_labels_res.get().unwrap_or_default().into_iter()
                                        .filter(|l| !selected.iter().any(|s| s.eq_ignore_ascii_case(&l.name)))
                                        .map(|l| {
                                            let color = l.color.clone();
                                            let name = l.name.clone();
                                            let name_add = name.clone();
                                            view! {
                                                <button
                                                    type="button"
                                                    class="note-chip note-chip--new"
                                                    disabled=move || saving.get()
                                                    on:click=move |_| add_label_name(labels_val, &name_add)
                                                >
                                                    <span class="note-chip-dot" style=format!("background:{color};")></span>
                                                    {name}
                                                </button>
                                            }
                                        }).collect_view()
                                }}
                            </div>
                            <input
                                type="text"
                                class="form-input"
                                placeholder="New label…"
                                list="note-label-suggestions"
                                prop:value=move || label_input.get()
                                on:input=move |ev| label_input.set(event_target_value(&ev))
                                on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                                    if ev.key() == "Enter" {
                                        ev.prevent_default();
                                        add_label_name(labels_val, &label_input.get_untracked());
                                        label_input.set(String::new());
                                    }
                                }
                                disabled=move || saving.get()
                            />
                            <datalist id="note-label-suggestions">
                                {move || all_labels_res.get().unwrap_or_default().into_iter().map(|l| view! {
                                    <option value=l.name></option>
                                }).collect_view()}
                            </datalist>
                        </div>

                        <div class="form-group">
                            <label class="form-label">"Tasks & awaits"</label>
                                <InfoHint
                                    short="Each token becomes its own Tasks/Await item."
                                    text="Each #task/pN/name or #await/date/name token becomes its own Tasks/Await item. Use the buttons, type # for the chooser, or type a token directly."
                                />
                            <div class="note-chip-row">
                                {move || {
                                    use crate::note_tokens::{scan_note_tokens, DoneOriginal, ScannedTokenKind};
                                    let text = text_val.get();
                                    let mut line_ord: std::collections::HashMap<usize, usize> =
                                        std::collections::HashMap::new();
                                    scan_note_tokens(&text)
                                        .into_iter()
                                        .map(|t| {
                                            let ord = {
                                                let e = line_ord.entry(t.line).or_insert(0);
                                                let this = *e;
                                                *e += 1;
                                                this
                                            };
                                            let line = t.line;
                                            let raw = t.raw(&text).to_string();
                                            let label = t.title();
                                            let kind_cls = match &t.kind {
                                                ScannedTokenKind::Task { .. } => "task",
                                                ScannedTokenKind::Await { .. } => "await",
                                                ScannedTokenKind::Done { .. } => "done",
                                            };
                                            let reopen = match &t.kind {
                                                ScannedTokenKind::Done { original, slug, .. } => {
                                                    let back = match original {
                                                        DoneOriginal::Task { priority } => {
                                                            crate::note_tokens::format_task_token(*priority, slug)
                                                        }
                                                        DoneOriginal::Await { date } => {
                                                            crate::note_tokens::format_await_token(*date, slug)
                                                        }
                                                    };
                                                    Some((raw.clone(), back))
                                                }
                                                _ => None,
                                            };
                                            let remove_raw = raw.clone();
                                            // Body click opens the local edit dialog
                                            // (backlog #129); a done chip opens
                                            // nothing (the reopen control below is
                                            // its affordance).
                                            let chip_kind = t.kind.clone();
                                            let on_chip_click = move |ev: leptos::ev::MouseEvent| {
                                                ev.stop_propagation();
                                                prefill_task_edit(
                                                    &chip_kind,
                                                    line,
                                                    ord,
                                                    task_modal_mode,
                                                    task_name_val,
                                                    task_priority_val,
                                                    task_date_val,
                                                    task_edit_target,
                                                );
                                            };
                                            view! {
                                                <span
                                                    class=format!("note-chip note-chip--{kind_cls}")
                                                    on:click=on_chip_click
                                                >
                                                    {label}
                                                    {reopen.map(|(from, to)| view! {
                                                        <button
                                                            type="button"
                                                            class="note-chip-action"
                                                            title="Reopen"
                                                            on:click=move |ev: leptos::ev::MouseEvent| {
                                                                ev.stop_propagation();
                                                                text_val.update(|t| { *t = t.replacen(&from, &to, 1); });
                                                            }
                                                        >"↺"</button>
                                                    })}
                                                    <button
                                                        type="button"
                                                        class="note-chip-remove"
                                                        title="Remove"
                                                        on:click=move |ev: leptos::ev::MouseEvent| {
                                                            ev.stop_propagation();
                                                            text_val.update(|t| { *t = t.replacen(&remove_raw, "", 1); });
                                                        }
                                                    >"✕"</button>
                                                </span>
                                            }
                                        })
                                        .collect_view()
                                }}
                                <button
                                    type="button"
                                    class="note-chip note-chip--new"
                                    disabled=move || saving.get()
                                    on:click=move |_| { task_edit_target.set(None); task_name_val.set(String::new()); task_modal_mode.set(Some("task")); }
                                >"+ Task"</button>
                                <button
                                    type="button"
                                    class="note-chip note-chip--new"
                                    disabled=move || saving.get()
                                    on:click=move |_| { task_edit_target.set(None); task_name_val.set(String::new()); task_modal_mode.set(Some("await")); }
                                >"+ Await"</button>
                            </div>
                        </div>

                        <div class="form-group">
                            <label class="form-label">"Calendar"</label>
                                <InfoHint
                                    short="Where this note's tasks and awaits are exposed."
                                    text="The calendar this note's tasks and awaits are exposed in — it decides which CalDAV clients and shared users can see them."
                                />
                                <Suspense fallback=|| view! { <p class="dialog-hint">"Loading calendars…"</p> }>
                                    {move || Suspend::new(async move {
                                        let cals: Vec<CalendarInfo> = calendars_res.await;
                                        view! {
                                            <div class="note-chip-row">
                                                {cals.into_iter().map(|c| {
                                                    let name = c.name.clone();
                                                    let name2 = name.clone();
                                                    let display = c.display_name.clone();
                                                    view! {
                                                        <button
                                                            type="button"
                                                            class="note-chip"
                                                            style=move || if calendar_val.get() == name {
                                                                "background:var(--surface-alt);border-color:var(--primary-color);".to_string()
                                                            } else { String::new() }
                                                            on:click=move |_| calendar_val.set(name2.clone())
                                                        >
                                                            {display}
                                                        </button>
                                                    }
                                                }).collect_view()}
                                            </div>
                                        }
                                    })}
                                </Suspense>
                                // Local Transition (not Suspense): this resource refetches on
                                // every calendar chip click, and we don't want that to blank
                                // the modal or bubble into the root boundary — just swap the
                                // hint in place, keeping stale text visible mid-fetch.
                                <Transition fallback=|| ()>
                                    {move || Suspend::new(async move {
                                        let shares: Vec<ShareInfo> = calendar_shares_res.await;
                                        let cal_grantees: BTreeSet<String> = shares
                                            .into_iter()
                                            .map(|s| s.grantee_username)
                                            .collect();
                                        let coll_members: BTreeSet<String> = collections_state
                                            .get_untracked()
                                            .into_iter()
                                            .find(|c| Some(c.id) == collection_val.get())
                                            .map(|c| if c.shared { c.members } else { vec![] })
                                            .unwrap_or_default()
                                            .into_iter()
                                            .collect();
                                        (cal_grantees != coll_members).then(|| view! {
                                            <p class="dialog-hint note-sharing-warning">
                                                "⚠ This calendar isn't shared with the same people as this note's collection — some people may see one but not the other."
                                            </p>
                                        })
                                    })}
                                </Transition>
                            </div>

                        {move || error_msg.get().map(|e| view! { <p class="dialog-error">{e}</p> })}
                    </div>
                </div>

                <div class="dialog-actions note-editor-actions">
                    <div class="note-editor-actions-start">
                        {if is_edit {
                            view! {
                                <button class="btn-danger" disabled=move || saving.get() on:click=do_delete>"Delete"</button>
                            }.into_any()
                        } else { ().into_any() }}

                        {if is_edit {
                            on_show_in_collection.map(|cb| view! {
                                <button
                                    class="btn-secondary"
                                    disabled=move || saving.get()
                                    on:click=move |_| cb.run(())
                                >
                                    "Show in collection"
                                </button>
                            }).into_any()
                        } else { ().into_any() }}

                        {if is_edit {
                            view! {
                                <button
                                    class="btn-secondary"
                                    title="Consolidate to Markdown"
                                    disabled=move || saving.get()
                                    on:click=move |_| show_consolidate.set(true)
                                >
                                    "Consolidate"
                                </button>
                            }.into_any()
                        } else { ().into_any() }}

                        // Toggle between tokens and bullets (backlog #130): every
                        // run of token-only lines becomes `- Title` (so a Google
                        // Keep packing/shopping list stops surfacing in
                        // Tasks/Focus/CalDAV), and a plain bullet list comes back
                        // as `#task/p50/…` tasks. The label names the direction
                        // that will actually run; it's disabled when neither
                        // applies.
                        <button
                            class="btn-secondary"
                            title=move || if crate::note_tokens::checklist_to_bullets(&text_val.get()).is_some() {
                                "Turn this note's checklist items into plain bullets"
                            } else {
                                "Turn this note's bullet list into tasks"
                            }
                            disabled=move || {
                                saving.get()
                                    || (crate::note_tokens::checklist_to_bullets(&text_val.get()).is_none()
                                        && crate::note_tokens::bullets_to_checklist(&text_val.get()).is_none())
                            }
                            on:click=move |_| {
                                let text = text_val.get_untracked();
                                if let Some(converted) = crate::note_tokens::checklist_to_bullets(&text) {
                                    text_val.set(converted);
                                } else if let Some(converted) = crate::note_tokens::bullets_to_checklist(&text) {
                                    text_val.set(converted);
                                }
                            }
                        >
                            {move || if crate::note_tokens::checklist_to_bullets(&text_val.get()).is_some() {
                                "Bulletize"
                            } else {
                                "Taskify"
                            }}
                        </button>
                    </div>

                    <div class="note-editor-actions-end">
                        <button class="btn-secondary" disabled=move || saving.get() on:click=move |_| on_close.get_value()()>
                            "Cancel"
                        </button>
                        <button class="btn-primary" disabled=move || saving.get() on:click=save>
                            {move || if saving.get() {
                                if is_edit { "Saving…" } else { "Creating…" }
                            } else if is_edit { "Save" } else { "Create" }}
                        </button>
                    </div>
                </div>
            </div>
        </div>

        // Stacked on top of the dialog above (both use `.dialog-backdrop`,
        // same layering as the recurrence scope dialog in `event_form.rs`) —
        // lets "+ New" create a collection without leaving the note editor.
        {move || show_new_collection.get().then(|| {
            let on_created = Callback::new(move |new_id: i64| {
                collection_val.set(Some(new_id));
                leptos::task::spawn_local(async move {
                    collections_state.set(list_note_collections().await.unwrap_or_default());
                });
            });
            view! {
                <CollectionEditorModal
                    collection=None
                    on_close=move || show_new_collection.set(false)
                    on_refresh=on_refresh
                    on_created=on_created
                />
            }
        })}

        // Inline task/await authoring modal (backlog #113), stacked like the
        // collection editor above.
        {move || task_modal_mode.get().map(|mode| {
            let is_task = mode == "task";
            let slug_preview = move || crate::note_tokens::slugify(&task_name_val.get());
            view! {
                <div class="dialog-backdrop" on:click=move |_| task_modal_mode.set(None)>
                    <div class="dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                        <div class="dialog-header">
                            <h2 class="dialog-title">{move || {
                                let editing = task_edit_target.get().is_some();
                                match (editing, is_task) {
                                    (false, true) => "New task",
                                    (false, false) => "New await",
                                    (true, true) => "Edit task",
                                    (true, false) => "Edit await",
                                }
                            }}</h2>
                            <button class="dialog-close" on:click=move |_| task_modal_mode.set(None)>"✕"</button>
                        </div>
                        <div class="dialog-form">
                            <div class="form-group">
                                <label class="form-label">"Name"</label>
                                <input
                                    type="text"
                                    class="form-input"
                                    placeholder=if is_task { "Check production now" } else { "Renew passport" }
                                    prop:value=move || task_name_val.get()
                                    on:input=move |ev| task_name_val.set(event_target_value(&ev))
                                />
                                <p class="dialog-hint">
                                    {"Token: "}
                                    {move || {
                                        let slug = slug_preview();
                                        if slug.is_empty() { "…".to_string() }
                                        else if is_task {
                                            format!("#task/p{}/{slug}", task_priority_val.get())
                                        } else {
                                            format!("#await/{}/{slug}", task_date_val.get())
                                        }
                                    }}
                                </p>
                            </div>
                            {is_task.then(|| view! {
                                <PrioritySlider priority=task_priority_val disabled=Signal::derive(|| false) />
                            })}
                            {(!is_task).then(|| view! {
                                <div class="form-group">
                                    <label class="form-label">"Date"</label>
                                    <input
                                        type="date"
                                        class="form-input"
                                        prop:value=move || task_date_val.get()
                                        on:input=move |ev| task_date_val.set(event_target_value(&ev))
                                    />
                                </div>
                            })}
                        </div>
                        <div class="dialog-actions">
                            <button class="btn-primary" on:click=submit_task>
                                {move || if task_edit_target.get().is_some() { "Save" } else { "Add" }}
                            </button>
                            <button class="btn-secondary" on:click=move |_| task_modal_mode.set(None)>"Cancel"</button>
                        </div>
                    </div>
                </div>
            }
        })}

        // "Consolidate to Markdown" (backlog #122), stacked like the other
        // sub-modals. On success, close the whole editor (the note may have
        // just been deleted) and refresh the Notes list.
        {move || show_consolidate.get().then(|| {
            note_for_consolidate.get_value().map(|n| {
                view! {
                    <ConsolidateNoteModal
                        note=n
                        on_close=move || show_consolidate.set(false)
                        on_refresh=Callback::new(move |_: ()| {
                            on_refresh.run(());
                            on_close.get_value()();
                        })
                    />
                }
            })
        })}
    }
}
