//! "Consolidate to Markdown" modal (backlog #122) — write a note out as a new
//! markdown file in a chosen source/folder. Reached from the note editor.

use leptos::prelude::*;
use std::sync::Arc;

use crate::api::{
    consolidate_note_to_markdown, list_markdown_sources, MarkdownSourceInfo, NoteInfo,
};
use crate::components::dir_picker::SourceDirPickerModal;
use crate::util::{format_server_fn_error, mouse_event_targets_self};

/// Default file name for a note: a slugified title, or `"note"` when it has
/// none.
fn default_filename(note: &NoteInfo) -> String {
    let slug = note
        .title
        .as_deref()
        .map(crate::note_tokens::slugify)
        .unwrap_or_default();
    if slug.is_empty() {
        "note".to_string()
    } else {
        slug
    }
}

#[component]
pub fn ConsolidateNoteModal(
    note: NoteInfo,
    on_close: impl Fn() + Send + Sync + 'static,
    on_refresh: Callback<()>,
) -> impl IntoView {
    let on_close = StoredValue::new(Arc::new(on_close) as Arc<dyn Fn() + Send + Sync>);

    let note = StoredValue::new(note);
    let has_images = note.with_value(|n| !n.images.is_empty());

    let sources_res: LocalResource<Vec<MarkdownSourceInfo>> =
        LocalResource::new(|| async move { list_markdown_sources().await.unwrap_or_default() });

    let selected_source: RwSignal<Option<i64>> = RwSignal::new(None);
    let folder = RwSignal::new(String::new());
    let filename = RwSignal::new(note.with_value(default_filename));
    let delete_note = RwSignal::new(false);
    let show_picker = RwSignal::new(false);
    let saving = RwSignal::new(false);
    let error_msg = RwSignal::<Option<String>>::new(None);

    // Default to the first source once the list lands.
    Effect::new(move |_| {
        if selected_source.get_untracked().is_none() {
            if let Some(first) = sources_res.get().and_then(|s| s.first().map(|x| x.id)) {
                selected_source.set(Some(first));
            }
        }
    });

    let save = move |_: leptos::ev::MouseEvent| {
        let note = note.get_value();
        let Some(source_id) = selected_source.get_untracked() else {
            error_msg.set(Some("Choose a target source.".into()));
            return;
        };
        let name = filename.get_untracked();
        if name.trim().is_empty() {
            error_msg.set(Some("Enter a file name.".into()));
            return;
        }
        saving.set(true);
        error_msg.set(None);
        let folder = folder.get_untracked();
        let delete = delete_note.get_untracked();
        let on_close = on_close;
        let on_refresh = on_refresh;
        leptos::task::spawn_local(async move {
            match consolidate_note_to_markdown(note.id, source_id, folder, name, delete).await {
                Ok(_) => {
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

    let backdrop_mousedown = RwSignal::new(false);

    view! {
        <div
            class="dialog-backdrop"
            on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
            on:click=move |_| if backdrop_mousedown.get() { on_close.get_value()() }
        >
            <div class="dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                <div class="dialog-header">
                    <h2 class="dialog-title">"Consolidate to Markdown"</h2>
                    <button class="dialog-close" on:click=move |_| on_close.get_value()()>"✕"</button>
                </div>

                <div class="dialog-form">
                    <div class="form-group">
                        <label class="form-label">"Target source"</label>
                        <Suspense fallback=|| view! { <p class="dialog-hint">"Loading sources…"</p> }>
                            {move || Suspend::new(async move {
                                let sources: Vec<MarkdownSourceInfo> = sources_res.await;
                                if sources.is_empty() {
                                    return view! {
                                        <p class="dialog-hint">
                                            "No markdown sources configured. Add one in Settings first."
                                        </p>
                                    }.into_any();
                                }
                                view! {
                                    <div class="note-chip-row">
                                        {sources.into_iter().map(|s| {
                                            let id = s.id;
                                            let color = s.color.clone();
                                            let color2 = color.clone();
                                            let name = s.name.clone();
                                            view! {
                                                <button
                                                    type="button"
                                                    class="note-chip"
                                                    style=move || if selected_source.get() == Some(id) {
                                                        format!("background:{color}1a;border-color:{color}55;color:{color};")
                                                    } else {
                                                        String::new()
                                                    }
                                                    on:click=move |_| {
                                                        selected_source.set(Some(id));
                                                        folder.set(String::new());
                                                    }
                                                >
                                                    <span class="note-chip-dot" style=format!("background:{color2};")></span>
                                                    {name}
                                                </button>
                                            }
                                        }).collect_view()}
                                    </div>
                                }.into_any()
                            })}
                        </Suspense>
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Folder in source"</label>
                        <div class="dir-picker-bar">
                            <input
                                type="text"
                                class="form-input dir-picker-input"
                                placeholder="(source root)"
                                prop:value=move || folder.get()
                                on:input=move |ev| folder.set(event_target_value(&ev))
                                disabled=move || saving.get()
                            />
                            <button
                                type="button"
                                class="btn-secondary"
                                disabled=move || selected_source.get().is_none() || saving.get()
                                on:click=move |_| show_picker.set(true)
                            >"Browse…"</button>
                        </div>
                    </div>

                    <div class="form-group">
                        <label class="form-label">"File name"</label>
                        <input
                            type="text"
                            class="form-input"
                            placeholder="my-note"
                            prop:value=move || filename.get()
                            on:input=move |ev| filename.set(event_target_value(&ev))
                            disabled=move || saving.get()
                        />
                        <p class="dialog-hint">"A .md extension is added if you omit it."</p>
                    </div>

                    {has_images.then(|| view! {
                        <p class="dialog-hint">
                            "This note has inline images — they'll be written alongside the file and linked from it."
                        </p>
                    })}

                    <div class="form-group">
                        <label class="form-label">"After consolidating"</label>
                        <label class="dialog-hint" style="display:flex;align-items:center;gap:.5rem;">
                            <input
                                type="checkbox"
                                prop:checked=move || delete_note.get()
                                on:change=move |ev| delete_note.set(event_target_checked(&ev))
                                disabled=move || saving.get()
                            />
                            "Delete the note (its tasks/awaits move with the file)"
                        </label>
                    </div>

                    {move || error_msg.get().map(|e| view! { <p class="dialog-error">{e}</p> })}
                </div>

                <div class="dialog-actions">
                    <button class="btn-primary" disabled=move || saving.get() on:click=save>
                        {move || if saving.get() { "Consolidating…" } else { "Consolidate" }}
                    </button>
                    <button class="btn-secondary" disabled=move || saving.get() on:click=move |_| on_close.get_value()()>
                        "Cancel"
                    </button>
                </div>
            </div>
        </div>

        {move || show_picker.get().then(|| {
            let source_id = selected_source.get_untracked();
            source_id.map(|source_id| view! {
                <SourceDirPickerModal
                    source_id=source_id
                    start=Some(folder.get_untracked())
                    on_close=move || show_picker.set(false)
                    on_pick=move |picked: String| {
                        folder.set(picked);
                        show_picker.set(false);
                    }
                />
            })
        })}
    }
}
