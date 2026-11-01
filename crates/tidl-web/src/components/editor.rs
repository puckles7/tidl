//! In-app CodeMirror 6 editor for markdown files.

#[cfg(any(feature = "hydrate", test))]
use crate::api::ContactDto;
use crate::api::{get_source_file_content, list_contacts, save_markdown_file};
use crate::components::priority_slider::PrioritySlider;
use leptos::prelude::*;

/// Filters `contacts_json` (a JSON `Vec<ContactDto>`) against `query` using
/// the same ordering as the Notes editor's typeahead (backlog #80, #91).
/// Compiled under `hydrate` (for the `#[wasm_bindgen]` export below) or
/// `test` (so it's testable via a normal `cargo test` run without also
/// needing `hydrate`) — never under a plain feature-less build, where it
/// would otherwise be genuinely dead code. Returns `"[]"` on malformed
/// input rather than panicking, since the real caller is a JS callback
/// with no good way to surface a Rust panic.
#[cfg(any(feature = "hydrate", test))]
fn filter_mentions_json_impl(contacts_json: &str, query: &str) -> String {
    let contacts: Vec<ContactDto> = serde_json::from_str(contacts_json).unwrap_or_default();
    let filtered = crate::util::filter_contacts_for_mention(&contacts, query);
    serde_json::to_string(&filtered).unwrap_or_else(|_| "[]".to_string())
}

// JS bridge — only compiled for WASM target.
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen(inline_js = r#"
export function tidl_editor_init(el, content) {
    return window.__tidlEditorInit(el, content);
}
export function tidl_editor_get_content(id) {
    return window.__tidlEditorGetContent(id);
}
export function tidl_editor_set_contacts(id, json) {
    window.__tidlEditorSetContacts(id, json);
}
export function tidl_editor_set_handlers(id, on_tag, on_menu) {
    window.__tidlEditorSetHandlers(id, on_tag, on_menu);
}
export function tidl_editor_insert(id, text) {
    window.__tidlEditorInsert(id, text);
}
export function tidl_editor_replace(id, from, to, text) {
    window.__tidlEditorReplace(id, from, to, text);
}
export function tidl_editor_destroy(id) {
    window.__tidlEditorDestroy(id);
}
export function tidl_load_editor() {
    return window.__tidlLoadEditor();
}
"#)]
extern "C" {
    fn tidl_editor_init(el: &web_sys::HtmlElement, content: &str) -> f64;
    fn tidl_editor_get_content(id: f64) -> String;
    fn tidl_editor_set_contacts(id: f64, contacts_json: &str);
    // Registers the Rust callbacks the CM6 instance invokes: `on_tag` on a
    // mousedown inside an existing task/await tag, `on_menu` when the `#`
    // chooser picks Task…/Await… (backlog #129).
    fn tidl_editor_set_handlers(id: f64, on_tag: &js_sys::Function, on_menu: &js_sys::Function);
    fn tidl_editor_insert(id: f64, text: &str);
    fn tidl_editor_replace(id: f64, from: f64, to: f64, text: &str);
    fn tidl_editor_destroy(id: f64);
    // Resolves once assets/editor.js (CodeMirror) has been dynamically
    // imported; memoized in JS so repeated calls are free.
    fn tidl_load_editor() -> js_sys::Promise;
}

/// Converts a CodeMirror document position (UTF-16 code units) to a byte offset
/// safe to slice a Rust `String` at (backlog #129).
#[cfg(feature = "hydrate")]
fn utf16_to_byte(s: &str, utf16_pos: usize) -> usize {
    let mut u = 0usize;
    let mut b = 0usize;
    for c in s.chars() {
        if u >= utf16_pos {
            break;
        }
        u += c.len_utf16();
        b += c.len_utf8();
    }
    b
}

/// Inverse of [`utf16_to_byte`]: a byte offset (on a char boundary) as a
/// CodeMirror document position.
#[cfg(feature = "hydrate")]
fn byte_to_utf16(s: &str, byte_pos: usize) -> usize {
    s[..byte_pos.min(s.len())].encode_utf16().count()
}

/// The JS-visible export `assets/editor.js`'s CM6 `autocompletion()` source
/// calls directly (imported from the generated `/pkg/tidl.js` bindings, the
/// same module `assets/index.html` boots — see backlog #91's design doc).
/// Thin wrapper around [`filter_mentions_json_impl`], which carries the
/// actual logic so it stays testable without the `hydrate` feature.
#[cfg(feature = "hydrate")]
#[wasm_bindgen::prelude::wasm_bindgen]
pub fn filter_mentions_json(contacts_json: &str, query: &str) -> String {
    filter_mentions_json_impl(contacts_json, query)
}

/// The browser's local date as `YYYY-MM-DD`, used to stamp `#done` records
/// (backlog #162). Matches the user's calendar day rather than UTC so a task
/// completed late in the evening is not dated tomorrow.
fn local_today() -> String {
    let (year, month, day) = crate::components::calendar::date_helpers::today_ymd();
    format!("{year:04}-{month:02}-{day:02}")
}

/// Full-page editor overlay for a markdown file in a markdown source.
///
/// Props:
/// - `source_id` — the markdown_sources.id row
/// - `rel_path`  — file path relative to the source root
/// - `on_close`  — called when the user closes the editor (Cancel or after Save)
#[component]
pub fn MarkdownFileEditor(
    source_id: i64,
    rel_path: String,
    on_close: Callback<()>,
    /// Called once a save actually succeeds (not on Cancel), so the caller
    /// can refetch any file content/render it's showing elsewhere — the
    /// editor writes straight to disk via its own resource/action and has
    /// no way to invalidate a sibling view's cache itself.
    on_saved: Callback<()>,
) -> impl IntoView {
    let rel_path = StoredValue::new(rel_path);

    // Load raw file content from server.
    let content_resource = Resource::new(
        move || (source_id, rel_path.get_value()),
        move |(sid, path)| get_source_file_content(sid, path),
    );

    // Contact list for the `@mention` typeahead (backlog #91) — same
    // fetch pattern as note_editor_modal.rs's own `contacts_res`.
    let contacts_res =
        LocalResource::new(|| async move { list_contacts().await.unwrap_or_default() });

    // Save action.
    let save_action = Action::new(move |content: &String| {
        let content = content.clone();
        let path = rel_path.get_value();
        async move { save_markdown_file(source_id, path, content).await }
    });

    let save_status: RwSignal<Option<Result<(), String>>> = RwSignal::new(None);

    // Track editor ID (numeric handle to the JS CM6 instance).
    // Only read/written inside #[cfg(feature = "hydrate")] blocks; suppress the
    // unused-variable lint that fires when building without that feature.
    #[allow(unused_variables)]
    let editor_id: RwSignal<Option<f64>> = RwSignal::new(None);

    // Set in on_cleanup so a still-pending editor load (see tidl_load_editor
    // below) doesn't attach CM6 to a container that's already been unmounted.
    #[allow(unused_variables)]
    let destroyed = RwSignal::new(false);

    // Keeps the JS-invoked Rust callbacks alive for the component's lifetime
    // (backlog #129). `Closure` is neither `Send` nor `Sync`, so these use
    // local-storage signals; only available under `hydrate`.
    #[cfg(feature = "hydrate")]
    #[allow(unused_variables)]
    let on_tag_handler: RwSignal<
        Option<wasm_bindgen::prelude::Closure<dyn FnMut(String, f64)>>,
        leptos::prelude::LocalStorage,
    > = RwSignal::new_local(None);
    #[cfg(feature = "hydrate")]
    #[allow(unused_variables)]
    let on_menu_handler: RwSignal<
        Option<wasm_bindgen::prelude::Closure<dyn FnMut(String)>>,
        leptos::prelude::LocalStorage,
    > = RwSignal::new_local(None);

    // ── Inline task/await authoring/edit dialog (backlog #129) ───────────────
    // Local to this editor: it rewrites the in-memory document (create inserts
    // at the caret, edit replaces the tag), never the file, so the editor's own
    // Save stays the single writer.
    let task_modal_mode: RwSignal<Option<&'static str>> = RwSignal::new(None);
    let task_name_val = RwSignal::new(String::new());
    let task_priority_val = RwSignal::new(50u32);
    let task_date_val = RwSignal::new(
        (chrono::Utc::now().date_naive() + chrono::Duration::days(7))
            .format("%Y-%m-%d")
            .to_string(),
    );
    // `Some(form)` while editing an existing tag; `None` while creating one.
    let task_edit_form: RwSignal<Option<crate::markdown_tokens::MdTagForm>> = RwSignal::new(None);
    // "Mark as done" checkbox in the edit dialog (backlog #162).
    let task_done_val = RwSignal::new(false);
    // "Remove await tag" checkbox in the edit dialog — awaits have no
    // completion, so an edit can instead strip the tag entirely.
    let task_remove_val = RwSignal::new(false);
    // True while editing a rich `#done` record; keeps its original completion
    // date when re-saving and keeps the "Mark as done" box visible even when
    // the record was originally an await (so it can be restored).
    let task_done_origin = RwSignal::new(false);
    let task_done_completed = RwSignal::new(Option::<String>::None);
    // UTF-16 `[start, end)` of the tag being edited (CodeMirror positions).
    #[allow(unused_variables)]
    let task_edit_range: RwSignal<Option<(usize, usize)>> = RwSignal::new(None);

    // Escape closes this editor (backlog #34) without also closing the preview
    // modal it was opened from: its `.md-editor-overlay` carries the id, and
    // the window-level listener picks the last visible backdrop-like surface
    // in document order — which is this overlay while it's mounted. The inline
    // task/await dialog (a nested backdrop with no id of its own) is closed
    // first when it's open (backlog #129).
    let esc_id = crate::modal_esc::use_escape_close(move || {
        if task_modal_mode.get_untracked().is_some() {
            task_modal_mode.set(None);
            task_edit_form.set(None);
            task_edit_range.set(None);
            task_done_val.set(false);
            task_remove_val.set(false);
            task_done_origin.set(false);
            task_done_completed.set(None);
        } else {
            on_close.run(());
        }
    });

    // Container node ref — CM6 mounts here.
    let container_ref = NodeRef::<leptos::html::Div>::new();

    // When save_action completes, update status.
    Effect::new(move |_| {
        if let Some(result) = save_action.value().get() {
            save_status.set(Some(result.map_err(|e| e.to_string())));
        }
    });

    // Cleanup: destroy CM6 instance when component unmounts.
    on_cleanup(move || {
        #[cfg(feature = "hydrate")]
        {
            destroyed.set(true);
            if let Some(id) = editor_id.get_untracked() {
                tidl_editor_destroy(id);
            }
        }
    });

    // Independent of CM6 init timing (backlog #91): pushes the contact list
    // into the already-initialized editor whenever either this or `editor_id`
    // resolves, in either order — a contacts fetch that completes after
    // tidl_editor_init has already run must still reach the live instance,
    // not be silently dropped because the init effect below only runs once.
    Effect::new(move |_| {
        let id = editor_id.get();
        let contacts = contacts_res.get();
        #[cfg(feature = "hydrate")]
        if let (Some(id), Some(contacts)) = (id, contacts) {
            let json = serde_json::to_string(&contacts).unwrap_or_else(|_| "[]".to_string());
            tidl_editor_set_contacts(id, &json);
        }
        #[cfg(not(feature = "hydrate"))]
        {
            let _ = (id, contacts);
        }
    });

    let handle_save = move |_: leptos::ev::MouseEvent| {
        #[cfg(feature = "hydrate")]
        {
            if let Some(id) = editor_id.get_untracked() {
                let content = tidl_editor_get_content(id);
                save_status.set(None);
                save_action.dispatch(content);
            }
        }
        #[cfg(not(feature = "hydrate"))]
        {
            let _ = save_action.dispatch(String::new());
        }
    };

    let handle_cancel = move |_: leptos::ev::MouseEvent| {
        on_close.run(());
    };

    // Apply the create/edit dialog: insert a new token at the caret, or rewrite
    // the remembered tag range, in the in-memory document (backlog #129).
    let submit_task = move |_: leptos::ev::MouseEvent| {
        let Some(mode) = task_modal_mode.get_untracked() else {
            return;
        };
        let removing = task_remove_val.get_untracked();
        let slug = crate::note_tokens::slugify(&task_name_val.get_untracked());
        if slug.is_empty() && !removing {
            return;
        }
        if let Some(form) = task_edit_form.get_untracked() {
            if form != crate::markdown_tokens::MdTagForm::PriorityTask
                && task_date_val.get_untracked().is_empty()
                && !removing
            {
                return;
            }
            let completed = task_done_completed
                .get_untracked()
                .unwrap_or_else(local_today);
            let token = if removing {
                // Remove the await tag from the document; Save persists it.
                String::new()
            } else if task_done_val.get_untracked() {
                crate::markdown_tokens::format_done_token(
                    form,
                    &slug,
                    Some(task_priority_val.get_untracked()),
                    Some(&task_date_val.get_untracked()),
                    &completed,
                )
            } else {
                crate::markdown_tokens::format_md_token(
                    form,
                    &slug,
                    Some(task_priority_val.get_untracked()),
                    Some(&task_date_val.get_untracked()),
                )
            };
            #[cfg(feature = "hydrate")]
            if let (Some(id), Some((from, to))) = (
                editor_id.get_untracked(),
                task_edit_range.get_untracked(),
            ) {
                tidl_editor_replace(id, from as f64, to as f64, &token);
            }
            #[cfg(not(feature = "hydrate"))]
            {
                let _ = (form, token);
            }
        } else {
            let form = if mode == "task" {
                crate::markdown_tokens::MdTagForm::PriorityTask
            } else {
                crate::markdown_tokens::MdTagForm::Await
            };
            if form == crate::markdown_tokens::MdTagForm::Await
                && task_date_val.get_untracked().is_empty()
            {
                return;
            }
            let token = crate::markdown_tokens::format_md_token(
                form,
                &slug,
                Some(task_priority_val.get_untracked()),
                Some(&task_date_val.get_untracked()),
            );
            #[cfg(feature = "hydrate")]
            if let Some(id) = editor_id.get_untracked() {
                tidl_editor_insert(id, &token);
            }
            #[cfg(not(feature = "hydrate"))]
            {
                let _ = token;
            }
        }
        task_modal_mode.set(None);
        task_edit_form.set(None);
        task_edit_range.set(None);
        task_done_val.set(false);
        task_remove_val.set(false);
        task_done_origin.set(false);
        task_done_completed.set(None);
        task_name_val.set(String::new());
    };

    // After save succeeds, notify the caller (so it can refetch stale views)
    // and close the editor.
    let on_close_after_save = on_close;
    Effect::new(move |_| {
        if let Some(Ok(())) = save_status.get() {
            on_saved.run(());
            on_close_after_save.run(());
        }
    });

    view! {
        <div class="md-editor-overlay" data-esc-id=esc_id.to_string()>
            // ── Toolbar ──────────────────────────────────────────────────────
            <div class="md-editor-toolbar">
                <span class="md-editor-path">{rel_path.get_value()}</span>
                <div class="md-editor-actions">
                    <button class="btn-secondary-small" on:click=handle_cancel>"Cancel"</button>
                    <button
                        class="btn-primary"
                        disabled=move || save_action.pending().get()
                        on:click=handle_save
                    >
                        {move || if save_action.pending().get() { "Saving…" } else { "Save" }}
                    </button>
                </div>
            </div>

            // ── Status bar ────────────────────────────────────────────────────
            {move || save_status.get().map(|r| match r {
                Ok(()) => view! { <div class="md-editor-status md-editor-status--ok">"Saved"</div> }.into_any(),
                Err(e) => view! { <div class="md-editor-status md-editor-status--err">{e}</div> }.into_any(),
            })}

            // ── Editor area ───────────────────────────────────────────────────
            <Suspense fallback=move || view! { <p class="fp-loading">"Loading…"</p> }>
                {move || {
                    content_resource.get().map(|result| {
                        let initial_content = match result {
                            Ok(c) => c,
                            Err(e) => return view! {
                                <p class="form-error">{e.to_string()}</p>
                            }.into_any(),
                        };
                        // Init CM6 once content is available.
                        // Only used inside #[cfg(feature = "hydrate")] — suppress lint.
                        #[allow(unused_variables)]
                        let content_for_effect = initial_content.clone();
                        Effect::new(move |_| {
                            #[cfg(feature = "hydrate")]
                            if let Some(el) = container_ref.get() {
                                if editor_id.get_untracked().is_none() {
                                    use wasm_bindgen::JsCast;
                                    let html_el: web_sys::HtmlElement = el.unchecked_into();
                                    let content = content_for_effect.clone();
                                    // tidl_load_editor() only resolves once editor.js
                                    // (CodeMirror) has loaded — see assets/editor-loader.js.
                                    leptos::task::spawn_local(async move {
                                        let _ = wasm_bindgen_futures::JsFuture::from(tidl_load_editor()).await;
                                        if destroyed.get_untracked() || editor_id.get_untracked().is_some() {
                                            return;
                                        }
                                        let id = tidl_editor_init(&html_el, &content);
                                        editor_id.set(Some(id));

                                        // Register the Rust callbacks CM6 invokes
                                        // (backlog #129). `on_tag` opens the dialog
                                        // for an existing tag under the click;
                                        // `on_menu` opens the create dialog.
                                        let on_tag =
                                            wasm_bindgen::prelude::Closure::<dyn FnMut(String, f64)>::new(
                                                move |doc: String, pos: f64| {
                                                    let pos = pos.max(0.0) as usize;
                                                    let byte = utf16_to_byte(&doc, pos);
                                                    let Some(hit) =
                                                        crate::markdown_tokens::tag_at_caret(&doc, byte)
                                                    else {
                                                        return;
                                                    };
                                                    let from = byte_to_utf16(&doc, hit.start);
                                                    let to = byte_to_utf16(&doc, hit.end);
                                                    task_done_val.set(false);
                                                    task_remove_val.set(false);
                                                    task_done_origin.set(false);
                                                    task_done_completed.set(None);
                                                    use crate::markdown_tokens::{
                                                        MdTagForm, MdTagKind,
                                                    };
                                                    // A rich `#done` record opens the
                                                    // same dialog with "Mark as done"
                                                    // checked, so unchecking it and
                                                    // saving restores the original tag.
                                                    if let Some(done) = hit.done {
                                                        task_done_val.set(true);
                                                        task_done_origin.set(true);
                                                        task_done_completed.set(Some(
                                                            done.completed().to_string(),
                                                        ));
                                                        task_name_val.set(
                                                            crate::note_tokens::slug_to_title(
                                                                done.slug(),
                                                            ),
                                                        );
                                                        if let Some(date) = done.date() {
                                                            task_date_val.set(date.to_string());
                                                        }
                                                        task_priority_val.set(done.priority());
                                                        task_edit_form.set(Some(done.form()));
                                                        task_edit_range.set(Some((from, to)));
                                                        task_modal_mode.set(Some(match done.form() {
                                                            MdTagForm::Await => "await",
                                                            _ => "task",
                                                        }));
                                                        return;
                                                    }
                                                    let Some(kind) = hit.kind else { return };
                                                    match &kind {
                                                        MdTagKind::Priority { priority, slug } => {
                                                            task_name_val.set(
                                                                crate::note_tokens::slug_to_title(slug),
                                                            );
                                                            task_priority_val.set(*priority);
                                                            task_edit_form
                                                                .set(Some(MdTagForm::PriorityTask));
                                                        }
                                                        MdTagKind::Dated { date, slug } => {
                                                            task_name_val.set(
                                                                crate::note_tokens::slug_to_title(slug),
                                                            );
                                                            task_date_val.set(date.clone());
                                                            task_edit_form
                                                                .set(Some(MdTagForm::DatedTask));
                                                        }
                                                        MdTagKind::Await { date, slug } => {
                                                            task_name_val.set(
                                                                crate::note_tokens::slug_to_title(slug),
                                                            );
                                                            task_date_val.set(date.clone());
                                                            task_edit_form.set(Some(MdTagForm::Await));
                                                        }
                                                    }
                                                    task_edit_range.set(Some((from, to)));
                                                    task_modal_mode.set(Some(match kind.form() {
                                                        MdTagForm::Await => "await",
                                                        _ => "task",
                                                    }));
                                                },
                                            );
                                        let on_menu =
                                            wasm_bindgen::prelude::Closure::<dyn FnMut(String)>::new(
                                                move |kind: String| {
                                                    task_edit_form.set(None);
                                                    task_edit_range.set(None);
                                                    task_done_val.set(false);
                                                    task_remove_val.set(false);
                                                    task_done_origin.set(false);
                                                    task_done_completed.set(None);
                                                    task_name_val.set(String::new());
                                                    task_modal_mode.set(Some(if kind == "await" {
                                                        "await"
                                                    } else {
                                                        "task"
                                                    }));
                                                },
                                            );
                                        {
                                            use wasm_bindgen::JsCast;
                                            tidl_editor_set_handlers(
                                                id,
                                                on_tag.as_ref().unchecked_ref(),
                                                on_menu.as_ref().unchecked_ref(),
                                            );
                                        }
                                        on_tag_handler.set(Some(on_tag));
                                        on_menu_handler.set(Some(on_menu));
                                    });
                                }
                            }
                        });
                        view! {
                            <div node_ref=container_ref class="md-editor-content"></div>
                        }.into_any()
                    })
                }}
            </Suspense>

            // ── Inline task/await create/edit dialog (backlog #129) ──────────
            // A nested backdrop with no `data-esc-id` of its own: the overlay's
            // registered Escape callback closes this first.
            {move || task_modal_mode.get().map(|mode| {
                let is_task = mode == "task";
                let slug_preview = move || crate::note_tokens::slugify(&task_name_val.get());
                let close = move |_: leptos::web_sys::MouseEvent| {
                    task_modal_mode.set(None);
                    task_edit_form.set(None);
                    task_edit_range.set(None);
                    task_done_val.set(false);
                    task_remove_val.set(false);
                    task_done_origin.set(false);
                    task_done_completed.set(None);
                };
                view! {
                    <div class="dialog-backdrop" on:click=close>
                        <div class="dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                            <div class="dialog-header">
                                <h2 class="dialog-title">{move || {
                                    let editing = task_edit_form.get().is_some();
                                    match (editing, is_task) {
                                        (false, true) => "New task",
                                        (false, false) => "New await",
                                        (true, true) => "Edit task",
                                        (true, false) => "Edit await",
                                    }
                                }}</h2>
                                <button class="dialog-close" on:click=close>"✕"</button>
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
                                            let form = task_edit_form.get();
                                            if task_remove_val.get() && form.is_some() {
                                                "(tag removed)".to_string()
                                            } else {
                                                let slug = slug_preview();
                                                if slug.is_empty() { "…".to_string() }
                                                else if task_done_val.get() && form.is_some() {
                                                    let completed = task_done_completed
                                                        .get()
                                                        .unwrap_or_else(local_today);
                                                    crate::markdown_tokens::format_done_token(
                                                        form.unwrap(),
                                                        &slug,
                                                        Some(task_priority_val.get()),
                                                        Some(&task_date_val.get()),
                                                        &completed,
                                                    )
                                                } else if is_task {
                                                    if form == Some(crate::markdown_tokens::MdTagForm::DatedTask) {
                                                        format!("#task/{}/{}", task_date_val.get(), slug)
                                                    } else {
                                                        format!("#task/p{}/{}", task_priority_val.get(), slug)
                                                    }
                                                } else {
                                                    format!("#await/{}/{}", task_date_val.get(), slug)
                                                }
                                            }
                                        }}
                                    </p>
                                </div>
                                {move || {
                                    let dated = task_edit_form.get() == Some(crate::markdown_tokens::MdTagForm::DatedTask);
                                    (is_task && !dated).then(|| view! {
                                        <PrioritySlider priority=task_priority_val disabled=Signal::derive(|| false) />
                                    })
                                }}
                                {move || {
                                    let dated = task_edit_form.get() == Some(crate::markdown_tokens::MdTagForm::DatedTask);
                                    (!is_task || dated).then(|| view! {
                                        <div class="form-group">
                                            <label class="form-label">"Date"</label>
                                            <input
                                                type="date"
                                                class="form-input"
                                                prop:value=move || task_date_val.get()
                                                on:input=move |ev| task_date_val.set(event_target_value(&ev))
                                            />
                                        </div>
                                    })
                                }}
                                {move || ((is_task || task_done_origin.get()) && task_edit_form.get().is_some()).then(|| view! {
                                    <label class="form-check-label">
                                        <input
                                            type="checkbox"
                                            prop:checked=move || task_done_val.get()
                                            on:change=move |ev| task_done_val.set(event_target_checked(&ev))
                                        />
                                        "Mark as done"
                                    </label>
                                })}
                                {move || (!is_task && !task_done_origin.get() && task_edit_form.get().is_some()).then(|| view! {
                                    <label class="form-check-label">
                                        <input
                                            type="checkbox"
                                            prop:checked=move || task_remove_val.get()
                                            on:change=move |ev| task_remove_val.set(event_target_checked(&ev))
                                        />
                                        "Remove await tag"
                                    </label>
                                })}
                            </div>
                            <div class="dialog-actions">
                                <button class="btn-primary" on:click=submit_task>
                                    {move || if task_edit_form.get().is_some() { "Save" } else { "Add" }}
                                </button>
                                <button class="btn-secondary" on:click=close>"Cancel"</button>
                            </div>
                        </div>
                    </div>
                }
            })}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ContactDto;

    #[test]
    fn filter_mentions_json_round_trips_through_the_shared_filter() {
        let contacts = vec![
            ContactDto {
                display_name: "Jane Doe".to_string(),
                ..Default::default()
            },
            ContactDto {
                display_name: "Bob".to_string(),
                ..Default::default()
            },
        ];
        let json = serde_json::to_string(&contacts).unwrap();
        let result_json = filter_mentions_json_impl(&json, "jane");
        let result: Vec<ContactDto> = serde_json::from_str(&result_json).unwrap();
        assert_eq!(result.len(), 1);
        assert_eq!(result[0].display_name, "Jane Doe");
    }

    #[test]
    fn filter_mentions_json_returns_empty_array_for_invalid_input() {
        let result_json = filter_mentions_json_impl("not json", "jane");
        let result: Vec<ContactDto> = serde_json::from_str(&result_json).unwrap();
        assert!(result.is_empty());
    }
}
