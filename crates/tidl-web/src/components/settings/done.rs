//! Done tab in Settings (backlog #124, #125) — how long completed items are
//! kept before being archived from their source, and how long archived items are
//! retained as history. Saving applies the rules immediately: any completed
//! item already older than a source's threshold is stripped from its origin and
//! archived. Thereafter a background sweep (`tidl-server`'s `done_sweep` loop,
//! backlog #135) keeps applying each source's age rule on a cadence.

use leptos::prelude::*;

use crate::api::{
    clear_archived_done_items, get_done_settings, list_done_scopes, purge_done_items,
    set_done_scope, set_done_settings, DoneFilter, DoneScope, DoneSettings, Retention,
};

#[component]
pub(super) fn DonePanel(on_close: impl Fn() + Send + Sync + 'static) -> impl IntoView {
    let on_close =
        StoredValue::new(std::sync::Arc::new(on_close) as std::sync::Arc<dyn Fn() + Send + Sync>);

    let refresh = RwSignal::new(0u32);
    let done_refresh = crate::app::use_done_refresh();
    let scopes_res = LocalResource::new(move || {
        let _ = refresh.get();
        async move { list_done_scopes().await.unwrap_or_default() }
    });

    // Per-source "<older-than days>" inputs, built once when the scopes load.
    // The scopes and their editable text live in two component-owned signals
    // (parallel by index) rather than a per-row `RwSignal` created inside the
    // `Effect`: a signal created in a reactive scope is disposed when that scope
    // re-runs, so after any refresh — e.g. "Clear archived items now" bumping
    // `refresh` and re-running `scopes_res` — the old per-row signals would be
    // dead and `on_save` would panic reading them, leaving the button stuck on
    // "Saving…".
    let scopes_sig: RwSignal<Vec<DoneScope>> = RwSignal::new(Vec::new());
    let values: RwSignal<Vec<String>> = RwSignal::new(Vec::new());
    let archive_days = RwSignal::new(String::new());
    let loaded = RwSignal::new(false);
    let saving = RwSignal::new(false);

    Effect::new(move |_| {
        if let Some(scopes) = scopes_res.get() {
            if !loaded.get_untracked() {
                loaded.set(true);
                values.set(
                    scopes
                        .iter()
                        .map(|s| s.older_than_days.map(|v| v.to_string()).unwrap_or_default())
                        .collect(),
                );
                scopes_sig.set(scopes);
                leptos::task::spawn_local(async move {
                    if let Ok(s) = get_done_settings().await {
                        archive_days.set(
                            s.archive_retention_days
                                .map(|d| d.to_string())
                                .unwrap_or_default(),
                        );
                    }
                });
            }
        }
    });

    let on_save = move |_: leptos::ev::MouseEvent| {
        if saving.get_untracked() {
            return;
        }
        saving.set(true);
        let current = scopes_sig.get_untracked();
        let current_values = values.get_untracked();
        let archive = archive_days.get_untracked().trim().parse::<u32>().ok();
        leptos::task::spawn_local(async move {
            let mut applies: Vec<(String, String, u32)> = Vec::new();
            for (i, scope) in current.into_iter().enumerate() {
                let older = current_values
                    .get(i)
                    .map(|s| s.trim())
                    .and_then(|s| s.parse::<u32>().ok());
                let origin = scope.origin.clone();
                let scope_id = scope.scope_id.clone();
                let _ = set_done_scope(DoneScope {
                    auto_archive: scope.auto_archive,
                    keep_newest: None,
                    older_than_days: older,
                    ..scope
                })
                .await;
                if let Some(d) = older {
                    applies.push((origin, scope_id, d));
                }
            }
            let _ = set_done_settings(DoneSettings {
                archive_retention_days: archive,
            })
            .await;
            for (origin, scope_id, d) in applies {
                let f = DoneFilter {
                    origin: Some(origin),
                    scope_id: Some(scope_id),
                    ..Default::default()
                };
                let _ = purge_done_items(f, Retention::OlderThan(d)).await;
            }
            saving.set(false);
            refresh.update(|n| *n += 1);
            done_refresh.update(|n| *n += 1);
        });
    };

    let clear_archived = move |_: leptos::ev::MouseEvent| {
        leptos::task::spawn_local(async move {
            let _ = clear_archived_done_items().await;
            refresh.update(|n| *n += 1);
            done_refresh.update(|n| *n += 1);
        });
    };

    view! {
        <p class="dialog-hint">
            "Archiving a completed item strips it from its source (markdown line, note token, or calendar task) and keeps it as read-only history on the Done tab."
        </p>

        <div class="form-group">
            <label class="form-label">"Archive completed items older than (days)"</label>
            <p class="form-hint">
                "Per source. Leave blank to keep a source's completed items indefinitely."
            </p>
            <ul class="done-list">
                {move || scopes_sig.get().into_iter().enumerate().map(|(i, scope)| {
                    let label = if scope.label.is_empty() {
                        format!("{}: {}", scope.origin, scope.scope_id)
                    } else {
                        scope.label.clone()
                    };
                    view! {
                        <li class="done-row">
                            <span class="done-title">{label}</span>
                            <input
                                class="form-input done-rule"
                                type="number"
                                min="1"
                                placeholder="never"
                                prop:value=move || values.get().get(i).cloned().unwrap_or_default()
                                on:input=move |ev| {
                                    let v = event_target_value(&ev);
                                    values.update(|vals| {
                                        if let Some(slot) = vals.get_mut(i) {
                                            *slot = v;
                                        }
                                    });
                                }
                            />
                        </li>
                    }
                }).collect_view()}
            </ul>
        </div>

        <div class="form-group">
            <label class="form-label">"Keep archived items as history for (days)"</label>
            <input
                class="form-input"
                type="number"
                min="0"
                placeholder="forever"
                prop:value=move || archive_days.get()
                on:input=move |ev| archive_days.set(event_target_value(&ev))
            />
            <p class="form-hint">
                "Archived items older than this are deleted permanently. 0 = keep no archived history. Blank = keep forever."
            </p>
            <button class="btn-danger" on:click=clear_archived>"Clear archived items now"</button>
        </div>

        <div class="dialog-actions">
            <button class="btn-cancel" on:click=move |_| on_close.with_value(|f| f())>"Close"</button>
            <button class="btn-primary" on:click=on_save disabled=move || saving.get()>
                {move || if saving.get() { "Saving…" } else { "Save & apply" }}
            </button>
        </div>
    }
}
