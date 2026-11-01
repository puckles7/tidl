//! Server-side directory picker modal.
//!
//! The browser cannot natively open a folder picker for a path *on the
//! server*, so this walks the server filesystem via
//! [`list_server_directories`](crate::api::list_server_directories): the user
//! can drill into subdirectories, jump back to the parent, type a path
//! directly, and confirm the current folder. Only directories are ever shown.

use leptos::prelude::*;
use std::sync::Arc;

use crate::api::{list_server_directories, list_source_directories};
use crate::types::DirListing;
use crate::util::{format_server_fn_error, mouse_event_targets_self};
/// A modal folder browser rooted at `start` (defaults to the server's current
/// working directory when blank). Calls `on_pick` with the chosen absolute
/// path.
#[component]
pub fn DirPickerModal(
    /// Directory to open first; blank/`None` starts at the server cwd.
    start: Option<String>,
    on_close: impl Fn() + Send + Sync + 'static,
    on_pick: impl Fn(String) + Send + Sync + 'static,
) -> impl IntoView {
    let on_close = StoredValue::new(Arc::new(on_close) as Arc<dyn Fn() + Send + Sync>);
    let on_pick = StoredValue::new(Arc::new(on_pick) as Arc<dyn Fn(String) + Send + Sync>);

    let listing = RwSignal::<Option<DirListing>>::new(None);
    let error_msg = RwSignal::<Option<String>>::new(None);
    let path_input = RwSignal::new(start.clone().unwrap_or_default());

    // Loads a directory into `listing`. A `Callback` (rather than a plain
    // closure) so every row/button handler can share it — closures aren't
    // `Copy` and can't be moved into more than one `on:click`.
    let load = Callback::new(move |path: Option<String>| {
        error_msg.set(None);
        listing.set(None);
        leptos::task::spawn_local(async move {
            match list_server_directories(path).await {
                Ok(l) => {
                    path_input.set(l.path.clone());
                    listing.set(Some(l));
                }
                Err(e) => error_msg.set(Some(format_server_fn_error(&e.to_string()))),
            }
        });
    });

    load.run(start);

    let backdrop_mousedown = RwSignal::new(false);
    let esc_id = crate::modal_esc::use_escape_close(move || on_close.get_value()());

    view! {
        <div
            class="dialog-backdrop"
            data-esc-id=esc_id.to_string()
            on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
            on:click=move |_| if backdrop_mousedown.get() { on_close.get_value()() }
        >
            <div class="dialog dir-picker-dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                <div class="dialog-header">
                    <h2 class="dialog-title">"Select folder"</h2>
                    <button class="dialog-close" on:click=move |_| on_close.get_value()()>"✕"</button>
                </div>

                <div class="dir-picker-bar">
                    <input
                        type="text"
                        class="form-input dir-picker-input"
                        placeholder="/home/user/notes"
                        prop:value=move || path_input.get()
                        on:input=move |ev| path_input.set(event_target_value(&ev))
                        on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                            if ev.key() == "Enter" {
                                load.run(Some(path_input.get_untracked()));
                            }
                        }
                    />
                    <button
                        class="btn-secondary"
                        on:click=move |_| load.run(Some(path_input.get_untracked()))
                    >"Go"</button>
                </div>

                <div class="dir-picker-list">
                    {move || {
                        if let Some(e) = error_msg.get() {
                            return view! { <p class="dialog-error">{e}</p> }.into_any();
                        }
                        let Some(l) = listing.get() else {
                            return view! { <p class="dialog-hint">"Loading…"</p> }.into_any();
                        };
                        view! {
                            <>
                                {l.parent.clone().map(|parent| {
                                    view! {
                                        <div
                                            class="md-finder-row dir-picker-row"
                                            on:click=move |_| load.run(Some(parent.clone()))
                                        >
                                            <span class="md-finder-path">".."</span>
                                        </div>
                                    }
                                })}
                                {if l.dirs.is_empty() {
                                    view! { <p class="dialog-hint">"No subdirectories"</p> }.into_any()
                                } else {
                                    l.dirs.iter().map(|d| {
                                        let path = d.path.clone();
                                        let name = d.name.clone();
                                        view! {
                                            <div
                                                class="md-finder-row dir-picker-row"
                                                on:click=move |_| load.run(Some(path.clone()))
                                            >
                                                <span class="md-finder-path">{name}</span>
                                            </div>
                                        }
                                    }).collect_view().into_any()
                                }}
                            </>
                        }.into_any()
                    }}
                </div>

                <div class="dialog-actions">
                    <button
                        class="btn-primary"
                        disabled=move || listing.get().is_none()
                        on:click=move |_| {
                            if let Some(l) = listing.get_untracked() {
                                on_pick.get_value()(l.path);
                            }
                        }
                    >"Select this folder"</button>
                    <button
                        class="btn-secondary"
                        on:click=move |_| on_close.get_value()()
                    >"Cancel"</button>
                </div>
            </div>
        </div>
    }
}

/// Folder browser scoped to one markdown source (backlog #122) — the same
/// drill/parent/type-a-path interaction as [`DirPickerModal`], but every path
/// shown and returned is relative to the source root, and only that source's
/// own tree is reachable. Called with the chosen relative folder (empty string
/// = source root).
#[component]
pub fn SourceDirPickerModal(
    source_id: i64,
    /// Relative subfolder to open first; blank/`None` starts at the root.
    start: Option<String>,
    on_close: impl Fn() + Send + Sync + 'static,
    on_pick: impl Fn(String) + Send + Sync + 'static,
) -> impl IntoView {
    let on_close = StoredValue::new(Arc::new(on_close) as Arc<dyn Fn() + Send + Sync>);
    let on_pick = StoredValue::new(Arc::new(on_pick) as Arc<dyn Fn(String) + Send + Sync>);

    let listing = RwSignal::<Option<DirListing>>::new(None);
    let error_msg = RwSignal::<Option<String>>::new(None);
    let path_input = RwSignal::new(start.clone().unwrap_or_default());

    let load = Callback::new(move |path: Option<String>| {
        error_msg.set(None);
        listing.set(None);
        leptos::task::spawn_local(async move {
            match list_source_directories(source_id, path).await {
                Ok(l) => {
                    path_input.set(l.path.clone());
                    listing.set(Some(l));
                }
                Err(e) => error_msg.set(Some(format_server_fn_error(&e.to_string()))),
            }
        });
    });

    load.run(start);

    let backdrop_mousedown = RwSignal::new(false);

    view! {
        <div
            class="dialog-backdrop"
            on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
            on:click=move |_| if backdrop_mousedown.get() { on_close.get_value()() }
        >
            <div class="dialog dir-picker-dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                <div class="dialog-header">
                    <h2 class="dialog-title">"Select folder in source"</h2>
                    <button class="dialog-close" on:click=move |_| on_close.get_value()()>"✕"</button>
                </div>

                <div class="dir-picker-bar">
                    <input
                        type="text"
                        class="form-input dir-picker-input"
                        placeholder="folder/subfolder"
                        prop:value=move || path_input.get()
                        on:input=move |ev| path_input.set(event_target_value(&ev))
                        on:keydown=move |ev: leptos::ev::KeyboardEvent| {
                            if ev.key() == "Enter" {
                                load.run(Some(path_input.get_untracked()));
                            }
                        }
                    />
                    <button
                        class="btn-secondary"
                        on:click=move |_| load.run(Some(path_input.get_untracked()))
                    >"Go"</button>
                </div>

                <div class="dir-picker-list">
                    {move || {
                        if let Some(e) = error_msg.get() {
                            return view! { <p class="dialog-error">{e}</p> }.into_any();
                        }
                        let Some(l) = listing.get() else {
                            return view! { <p class="dialog-hint">"Loading…"</p> }.into_any();
                        };
                        view! {
                            <>
                                {l.parent.clone().map(|parent| {
                                    view! {
                                        <div
                                            class="md-finder-row dir-picker-row"
                                            on:click=move |_| load.run(Some(parent.clone()))
                                        >
                                            <span class="md-finder-path">".."</span>
                                        </div>
                                    }
                                })}
                                {if l.dirs.is_empty() {
                                    view! { <p class="dialog-hint">"No subdirectories"</p> }.into_any()
                                } else {
                                    l.dirs.iter().map(|d| {
                                        let path = d.path.clone();
                                        let name = d.name.clone();
                                        view! {
                                            <div
                                                class="md-finder-row dir-picker-row"
                                                on:click=move |_| load.run(Some(path.clone()))
                                            >
                                                <span class="md-finder-path">{name}</span>
                                            </div>
                                        }
                                    }).collect_view().into_any()
                                }}
                            </>
                        }.into_any()
                    }}
                </div>

                <div class="dialog-actions">
                    <button
                        class="btn-primary"
                        on:click=move |_| on_pick.get_value()(path_input.get_untracked().trim().to_string())
                    >"Use this folder"</button>
                    <button
                        class="btn-secondary"
                        on:click=move |_| on_close.get_value()()
                    >"Cancel"</button>
                </div>
            </div>
        </div>
    }
}
