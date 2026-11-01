//! Fuzzy file finder modal (backlog #75) — a telescope-style "quick open"
//! across every enabled markdown source's files. Opening a file reuses the
//! existing rendered markdown preview (`PreviewOverlay`/`PreviewState`,
//! defined in `components::focus`) rather than a new viewer, by
//! synthesizing the same `"md:{source_id}:{rel_path}:{line}:{ordinal}"` uid the
//! Focus/Tasks/Await cards already build for their own "go to source".

use leptos::prelude::*;

use crate::api::list_all_markdown_files;
use crate::components::focus::{source_pill_style, PreviewState};
use crate::types::MarkdownFileEntry;
use crate::util::{fuzzy_match_score, mouse_event_targets_self};

type FileKey = (i64, String);

fn file_key(f: &MarkdownFileEntry) -> FileKey {
    (f.source_id, f.rel_path.clone())
}

/// Builds the same uid shape `md_rel_path`/`md_source_id_i64` (focus.rs)
/// parse — `"md:{source_id}:{rel_path}:{line}:{ordinal}"` — pointed at line 1,
/// since a file opened from the finder has no specific task/await line to jump
/// to.
fn preview_state_for(f: &MarkdownFileEntry) -> PreviewState {
    let uid = format!("md:{}:{}:1:0", f.source_id, f.rel_path);
    let display_name = f
        .rel_path
        .rsplit('/')
        .next()
        .unwrap_or(&f.rel_path)
        .to_string();
    PreviewState::Markdown {
        uid,
        summary: display_name,
        source_file: f.rel_path.clone(),
        source_line: 1,
        source_ordinal: 0,
    }
}

#[allow(private_interfaces, private_bounds)]
#[component]
pub(crate) fn MarkdownFileFinder(
    on_close: impl Fn() + Send + Sync + 'static,
    on_open: impl Fn(PreviewState) + Send + Sync + 'static,
) -> impl IntoView {
    let on_close =
        StoredValue::new(std::sync::Arc::new(on_close) as std::sync::Arc<dyn Fn() + Send + Sync>);
    let on_open = StoredValue::new(
        std::sync::Arc::new(on_open) as std::sync::Arc<dyn Fn(PreviewState) + Send + Sync>
    );

    let files_res: LocalResource<Vec<MarkdownFileEntry>> =
        LocalResource::new(|| async move { list_all_markdown_files().await.unwrap_or_default() });

    let query = RwSignal::new(String::new());
    let selected: RwSignal<Option<FileKey>> = RwSignal::new(None);

    // Focus the search box as soon as the modal mounts so Ctrl/Cmd-K and the
    // 🔎 button land the caret in it — the static `autofocus` attribute is
    // unreliable for an element inserted after the initial page load.
    let search_ref: NodeRef<leptos::html::Input> = NodeRef::new();
    search_ref.on_load(move |input| {
        let _ = input.focus();
    });

    let filtered = Memo::new(move |_| {
        let q = query.get();
        let mut scored: Vec<(i32, MarkdownFileEntry)> = files_res
            .get()
            .unwrap_or_default()
            .into_iter()
            .filter_map(|f| fuzzy_match_score(&q, &f.rel_path).map(|score| (score, f)))
            .collect();
        scored.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.rel_path.cmp(&b.1.rel_path)));
        scored.into_iter().map(|(_, f)| f).collect::<Vec<_>>()
    });

    // Keep `selected` pointing at a row still present in the filtered list —
    // it stops matching whenever the query changes underneath it, and starts
    // out unset before the first load lands.
    Effect::new(move |_| {
        let list = filtered.get();
        let still_visible = selected
            .get_untracked()
            .is_some_and(|k| list.iter().any(|f| file_key(f) == k));
        if !still_visible {
            selected.set(list.first().map(file_key));
        }
    });

    let move_selection = move |delta: i32| {
        let list = filtered.get_untracked();
        if list.is_empty() {
            return;
        }
        let cur = selected
            .get_untracked()
            .and_then(|k| list.iter().position(|f| file_key(f) == k))
            .unwrap_or(0) as i32;
        let len = list.len() as i32;
        let next = ((cur + delta) % len + len) % len;
        selected.set(list.get(next as usize).map(file_key));
    };

    let handle_keydown = move |ev: leptos::ev::KeyboardEvent| match ev.key().as_str() {
        "ArrowDown" => {
            ev.prevent_default();
            move_selection(1);
        }
        "ArrowUp" => {
            ev.prevent_default();
            move_selection(-1);
        }
        "Enter" => {
            ev.prevent_default();
            let list = filtered.get_untracked();
            let entry = selected
                .get_untracked()
                .and_then(|k| list.iter().find(|f| file_key(f) == k).cloned());
            if let Some(f) = entry {
                on_open.with_value(|cb| cb(preview_state_for(&f)));
            }
        }
        "Escape" => {
            ev.prevent_default();
            on_close.with_value(|f| f());
        }
        _ => {}
    };

    let backdrop_mousedown = RwSignal::new(false);
    let esc_id = crate::modal_esc::use_escape_close(move || on_close.with_value(|f| f()));

    view! {
        <div
            class="dialog-backdrop"
            data-esc-id=esc_id.to_string()
            on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
            on:click=move |_| if backdrop_mousedown.get() { on_close.with_value(|f| f()) }
        >
            <div class="dialog md-finder-dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                <div class="dialog-header">
                    <h2 class="dialog-title">"Find file"</h2>
                    <button class="dialog-close" on:click=move |_| on_close.with_value(|f| f())>"✕"</button>
                </div>

                <div class="md-finder-list">
                    {move || {
                        if files_res.get().is_none() {
                            view! { <p class="dialog-hint">"Loading…"</p> }.into_any()
                        } else if filtered.get().is_empty() {
                            view! { <p class="dialog-hint">"No matching files"</p> }.into_any()
                        } else {
                            view! {
                                <For
                                    each=move || filtered.get()
                                    key=|f| file_key(f)
                                    let:f
                                >
                                    {
                                        let f_for_selected = f.clone();
                                        let f_for_hover = f.clone();
                                        let f_for_click = f.clone();
                                        let is_selected = move || {
                                            selected.get().as_ref() == Some(&file_key(&f_for_selected))
                                        };
                                        let row_class = move || {
                                            if is_selected() {
                                                "md-finder-row md-finder-row--selected"
                                            } else {
                                                "md-finder-row"
                                            }
                                        };
                                        let pill_style = source_pill_style(&f.source_color);
                                        let dot_style = format!("background:{};", f.source_color);
                                        view! {
                                            <div
                                                class=row_class
                                                on:mouseenter=move |_| selected.set(Some(file_key(&f_for_hover)))
                                                on:click=move |_| on_open.with_value(|cb| cb(preview_state_for(&f_for_click)))
                                            >
                                                <span class="md-finder-path">{f.rel_path.clone()}</span>
                                                <span class="pill pill-source md-finder-source-tag" style=pill_style>
                                                    <span class="pill-dot" style=dot_style></span>
                                                    {f.source_name.clone()}
                                                </span>
                                            </div>
                                        }
                                    }
                                </For>
                            }.into_any()
                        }
                    }}
                </div>

                <div class="md-finder-searchbox">
                    <input
                        type="text"
                        class="form-input"
                        placeholder="Filter by path…"
                        node_ref=search_ref
                        autofocus=true
                        prop:value=move || query.get()
                        on:input=move |ev| query.set(event_target_value(&ev))
                        on:keydown=handle_keydown
                    />
                </div>
            </div>
        </div>
    }
}
