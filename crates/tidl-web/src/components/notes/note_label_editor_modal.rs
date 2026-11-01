//! Label create/edit/delete modal for the Notes sidebar (backlog #114, #128).
//!
//! Compact counterpart to `CollectionEditorModal` — a label has a name and a
//! colour (no sharing). Structurally mirrors it (dialog CSS, palette, delete
//! confirm), so the two stay recognisably the same interaction.

use leptos::prelude::*;
use std::sync::Arc;

use crate::api::{create_note_label, delete_note_label, update_note_label, NoteLabelInfo};
use crate::util::{format_server_fn_error, mouse_event_targets_self};

/// Label colour palette (backlog #127) — the base hue set rotated +45°,
/// disjoint from calendars, markdown sources, and collections. Used both by
/// the editor's colour picker and (via `resolve_label`) as the hash-assigned
/// colour for a label created implicitly from the note editor. Values are CSS
/// variable references (light/dark aware).
pub const LABEL_PALETTE: &[&str] = &[
    "var(--pal-label-1)",
    "var(--pal-label-2)",
    "var(--pal-label-3)",
    "var(--pal-label-4)",
    "var(--pal-label-5)",
    "var(--pal-label-6)",
];

#[component]
pub fn NoteLabelEditorModal(
    /// `None` to create; `Some(label)` to edit an existing one.
    label: Option<NoteLabelInfo>,
    on_close: impl Fn() + Send + Sync + 'static,
    on_refresh: Callback<()>,
) -> impl IntoView {
    let on_close = StoredValue::new(Arc::new(on_close) as Arc<dyn Fn() + Send + Sync>);
    let is_edit = label.is_some();
    let label_id = StoredValue::new(label.as_ref().map(|l| l.id));

    let name_val = RwSignal::new(label.as_ref().map(|l| l.name.clone()).unwrap_or_default());
    let color_val = RwSignal::new(
        label
            .as_ref()
            .map(|l| l.color.clone())
            .unwrap_or_else(|| LABEL_PALETTE[0].to_string()),
    );
    let saving = RwSignal::new(false);
    let error_msg = RwSignal::<Option<String>>::new(None);
    let delete_open = RwSignal::new(false);

    let save = move |_: leptos::ev::MouseEvent| {
        let name = name_val.get_untracked();
        let color = color_val.get_untracked();
        if name.trim().is_empty() {
            error_msg.set(Some("Name is required.".into()));
            return;
        }
        saving.set(true);
        error_msg.set(None);
        let on_close = on_close;
        let on_refresh = on_refresh;
        leptos::task::spawn_local(async move {
            let result = match label_id.get_value() {
                Some(id) => update_note_label(id, name.trim().to_string(), color).await,
                None => create_note_label(name.trim().to_string(), color)
                    .await
                    .map(|_| ()),
            };
            match result {
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

    let do_delete = move |_: leptos::ev::MouseEvent| {
        saving.set(true);
        error_msg.set(None);
        let on_close = on_close;
        let on_refresh = on_refresh;
        leptos::task::spawn_local(async move {
            let Some(id) = label_id.get_value() else {
                return;
            };
            match delete_note_label(id).await {
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

    let title = if is_edit { "Edit label" } else { "New label" };

    let backdrop_mousedown = RwSignal::new(false);
    let esc_id = crate::modal_esc::use_escape_close(move || {
        // Escape dismisses the nested delete-confirm before the label editor.
        if delete_open.get_untracked() {
            delete_open.set(false);
        } else {
            on_close.get_value()();
        }
    });
    view! {
        <div class="dialog-backdrop"
            data-esc-id=esc_id.to_string()
            on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
            on:click=move |_| if backdrop_mousedown.get() { on_close.get_value()() }
        >
            <div class="dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                <div class="dialog-header">
                    <h2 class="dialog-title">{title}</h2>
                    <button class="dialog-close" on:click=move |_| on_close.get_value()()>"✕"</button>
                </div>

                <div class="dialog-form">
                    <div class="form-group">
                        <label class="form-label">"Name"</label>
                        <input
                            class="form-input"
                            type="text"
                            placeholder="Label name"
                            prop:value=move || name_val.get()
                            on:input=move |ev| name_val.set(event_target_value(&ev))
                            disabled=move || saving.get()
                        />
                    </div>

                    <div class="form-group">
                        <label class="form-label">"Color"</label>
                        <div class="note-palette">
                            {LABEL_PALETTE.iter().map(|&c| {
                                let color = c.to_string();
                                let color2 = c.to_string();
                                view! {
                                    <button
                                        type="button"
                                        class="note-palette-swatch"
                                        style=move || format!(
                                            "background:{};{}",
                                            color,
                                            if color_val.get() == color {
                                                "outline:2px solid var(--text,#111);outline-offset:2px;"
                                            } else { "" }
                                        )
                                        on:click=move |ev| {
                                            ev.prevent_default();
                                            color_val.set(color2.clone());
                                        }
                                    ></button>
                                }
                            }).collect_view()}
                        </div>
                    </div>

                    {move || error_msg.get().map(|e| view! { <p class="dialog-error">{e}</p> })}
                </div>

                <div class="dialog-actions">
                    <button class="btn-primary" disabled=move || saving.get() on:click=save>
                        {move || if saving.get() {
                            if is_edit { "Saving…" } else { "Creating…" }
                        } else if is_edit { "Save" } else { "Create" }}
                    </button>
                    {if is_edit {
                        view! {
                            <button
                                class="btn-danger"
                                disabled=move || saving.get()
                                on:click=move |_| delete_open.set(true)
                            >"Delete"</button>
                        }.into_any()
                    } else {
                        ().into_any()
                    }}
                    <button class="btn-secondary" disabled=move || saving.get() on:click=move |_| on_close.get_value()()>
                        "Cancel"
                    </button>
                </div>
            </div>
        </div>

        {move || delete_open.get().then(|| {
            let backdrop_mousedown = RwSignal::new(false);
            view! {
                <div class="dialog-backdrop"
                    on:mousedown=move |ev: leptos::web_sys::MouseEvent| backdrop_mousedown.set(mouse_event_targets_self(&ev))
                    on:click=move |_| if backdrop_mousedown.get() { delete_open.set(false) }
                >
                    <div class="dialog" on:click=|ev: leptos::web_sys::MouseEvent| ev.stop_propagation()>
                        <div class="dialog-header">
                            <h2 class="dialog-title">"Delete label?"</h2>
                        </div>
                        <div class="dialog-form">
                            <p>"This removes the label from every note that carries it. The notes themselves are kept. This cannot be undone."</p>
                        </div>
                        <div class="dialog-actions">
                            <button class="btn-danger" disabled=move || saving.get() on:click=do_delete>
                                {move || if saving.get() { "Deleting…" } else { "Yes, delete" }}
                            </button>
                            <button class="btn-secondary" on:click=move |_| delete_open.set(false)>"Cancel"</button>
                        </div>
                    </div>
                </div>
            }
        })}
    }
}
