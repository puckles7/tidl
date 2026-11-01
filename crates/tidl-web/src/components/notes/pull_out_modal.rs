//! "Pull out into Note" modal (backlog #122) — import a markdown file as a new
//! note in a chosen collection. Reached from the rendered markdown preview.

use leptos::prelude::*;
use std::sync::Arc;

use crate::api::{list_note_collections, pull_markdown_into_note, NoteCollectionInfo};
use crate::util::{format_server_fn_error, mouse_event_targets_self};

#[component]
pub fn PullOutNoteModal(
    source_id: i64,
    rel_path: String,
    on_close: impl Fn() + Send + Sync + 'static,
    /// Called with the new note's id once the import succeeds.
    on_refresh: Callback<i64>,
) -> impl IntoView {
    let on_close = StoredValue::new(Arc::new(on_close) as Arc<dyn Fn() + Send + Sync>);
    let rel_path = StoredValue::new(rel_path);
    let display_name = rel_path.with_value(|p| p.rsplit('/').next().unwrap_or(p).to_string());

    let collections_res: LocalResource<Vec<NoteCollectionInfo>> =
        LocalResource::new(|| async move { list_note_collections().await.unwrap_or_default() });

    let selected_collection: RwSignal<Option<i64>> = RwSignal::new(None);
    let delete_file = RwSignal::new(false);
    let saving = RwSignal::new(false);
    let error_msg = RwSignal::<Option<String>>::new(None);

    Effect::new(move |_| {
        if selected_collection.get_untracked().is_none() {
            if let Some(first) = collections_res.get().and_then(|c| c.first().map(|x| x.id)) {
                selected_collection.set(Some(first));
            }
        }
    });

    let save = move |_: leptos::ev::MouseEvent| {
        let Some(collection_id) = selected_collection.get_untracked() else {
            error_msg.set(Some("Choose a collection.".into()));
            return;
        };
        saving.set(true);
        error_msg.set(None);
        let rel_path = rel_path.get_value();
        let delete = delete_file.get_untracked();
        let on_close = on_close;
        let on_refresh = on_refresh;
        leptos::task::spawn_local(async move {
            match pull_markdown_into_note(source_id, rel_path, collection_id, delete).await {
                Ok(note_id) => {
                    saving.set(false);
                    on_close.get_value()();
                    on_refresh.run(note_id);
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
                    <h2 class="dialog-title">"Pull out into Note"</h2>
                    <button class="dialog-close" on:click=move |_| on_close.get_value()()>"✕"</button>
                </div>

                <div class="dialog-form">
                    <p class="dialog-hint">
                        "Import " {display_name} " as a new note. A leading # heading becomes the note title."
                    </p>

                    <div class="form-group">
                        <label class="form-label">"Collection"</label>
                        <Suspense fallback=|| view! { <p class="dialog-hint">"Loading collections…"</p> }>
                            {move || Suspend::new(async move {
                                let collections: Vec<NoteCollectionInfo> = collections_res.await;
                                if collections.is_empty() {
                                    return view! {
                                        <p class="dialog-hint">
                                            "No note collections yet. Create one in the Notes tab first."
                                        </p>
                                    }.into_any();
                                }
                                view! {
                                    <div class="note-chip-row">
                                        {collections.into_iter().map(|c| {
                                            let id = c.id;
                                            let color = c.color.clone();
                                            let color2 = color.clone();
                                            let name = c.name.clone();
                                            view! {
                                                <button
                                                    type="button"
                                                    class="note-chip"
                                                    style=move || if selected_collection.get() == Some(id) {
                                                        format!("background:{color}1a;border-color:{color}55;color:{color};")
                                                    } else {
                                                        String::new()
                                                    }
                                                    on:click=move |_| selected_collection.set(Some(id))
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
                        <label class="form-label">"After importing"</label>
                        <label class="dialog-hint" style="display:flex;align-items:center;gap:.5rem;">
                            <input
                                type="checkbox"
                                prop:checked=move || delete_file.get()
                                on:change=move |ev| delete_file.set(event_target_checked(&ev))
                                disabled=move || saving.get()
                            />
                            "Delete the markdown file"
                        </label>
                    </div>

                    {move || error_msg.get().map(|e| view! { <p class="dialog-error">{e}</p> })}
                </div>

                <div class="dialog-actions">
                    <button class="btn-primary" disabled=move || saving.get() on:click=save>
                        {move || if saving.get() { "Importing…" } else { "Pull out" }}
                    </button>
                    <button class="btn-secondary" disabled=move || saving.get() on:click=move |_| on_close.get_value()()>
                        "Cancel"
                    </button>
                </div>
            </div>
        </div>
    }
}
